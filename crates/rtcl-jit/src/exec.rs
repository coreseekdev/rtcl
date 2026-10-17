//! JIT 执行器（feature `jit-exec`，wasmi 引擎）：ABI v2 宿主实现 + 顶层
//! 单元驱动。宿主全部经 rtcl-core/rtcl-vm 公共 API 路由——语义对齐的
//! 三条锚：
//!
//! - **命令**（`rtcl.call`）：`interp.eval("\n"*(L-1) + site_text)`——
//!   原文经解释器全链路执行（dispatch/错误帧/步数/trace 全在 interp），
//!   换行前缀使 harness 帧的行号与 vm_exec 的 `entry_offset+site.line`
//!   相同。字节级等价由构造保证。
//! - **变量/算术兜底**（`get_var`/`set_var`/`incr_var`/`binary_op`/
//!   `bool_test`）：`read_var`/`set_var`/`incr_var` 与
//!   `types::eval_expr`（带 canonical 引用的原文嵌入）——vm_exec 同款
//!   入口。守卫失败 = 调宿主 full-semantics import（「编译仍全量、
//!   运行时快慢分流」，§2）。
//! - **错误协议**：宿主 eval 错误（case a）→ `interp.error_report` 出
//!   报告；快车道宿主错误（case b）→ 驱动机械复刻 `err_harness_frame`
//!   /`unwind_subs`/`error_report` 的字符串协议（fail 表的静态
//!   cur_site/bodies 上下文）。
//!
//! 变量缓存（wasm 内存 + dirty 位）：**仅宿主 `rtcl.call` 之前允许**
//! （新进程不变量：首次命令调用之前不可能存在 trace/upvar/链接——
//! 首次 `rtcl.call` 时宿主清零 `caching` global，此后全部走宿主）。

use crate::abi::{self, code, BinOp};
use crate::emit;
use crate::ir::FailInfo;
use crate::pass::EmitConfig;
use rtcl_core::value::Value;
use rtcl_core::Error;
use rtcl_core::Interp;
use rtcl_parser::{ByteCode, Compiler, ScriptUnit};
use rtcl_vm::VmContext;
use std::rc::Rc;

/// 顶层执行结果。
pub enum Outcome {
    /// 单元未被接受（编译拒绝）——调用方走 `interp.eval`（保证字节等价）。
    Fallback,
    /// `run()` 的 Tcl result code + 挂起错误的处置提示。
    Completed { code: i32, exit_code: Option<i32> },
}

/// 宿主侧挂起错误：`from_eval` = 来自 `rtcl.call` 的 eval（解释器已
/// 累积 errorInfo——case a）；否则快车道宿主原语错误（case b）。
struct Pending {
    err: Error,
    from_eval: bool,
}

struct Host {
    interp: Interp,
    arena: Vec<Value>,
    consts: Vec<String>,
    fails: Vec<FailInfo>,
    sites: Vec<rtcl_parser::SrcSpan>,
    lines: Vec<u32>,
    source: Rc<str>,
    result: Option<Value>,
    pending: Option<Pending>,
    pending_exit: Option<i32>,
    /// case b 的驱动侧 errorInfo 累积器（机械复刻 err_harness_frame）。
    info: Option<String>,
    err_line: usize,
    pending_top: Option<String>,
    /// 步数预算的 charge 粒度（REFUEL_CHUNK 次 backedge 计 1 步/次）。
    refuel_chunk: i64,
}

/// err_is_error 复刻（控制流完成不参与帧累积）。
fn is_real_error(e: &Error) -> bool {
    !(e.is_return() || e.is_break() || e.is_continue() || e.is_exit() || e.is_tail_call())
}

impl Host {
    fn value_of(&self, tagged: i64) -> Value {
        if tagged & 1 == 1 {
            return Value::from_imm(abi::untag(tagged));
        }
        let idx = (tagged >> 1) as usize;
        self.arena.get(idx).cloned().unwrap_or_else(Value::empty)
    }

    fn tag_value(&mut self, v: &Value) -> i64 {
        if v.is_imm() {
            return abi::tag_imm(v.imm());
        }
        // arena[0..3] 是保留槽；值从 watermark 起。
        let idx = self.arena.len();
        self.arena.push(v.clone());
        abi::handle(idx as u32)
    }

    fn site_text(&self, site: u32) -> &str {
        self.sites
            .get(site as usize)
            .map(|s| s.slice(&self.source))
            .unwrap_or("")
    }
}

const HARNESS_MAX: usize = 150;

/// tcl_log_excerpt 的逐字节复刻（eval.rs:14）。
fn log_excerpt(text: &str) -> String {
    let len = text.chars().count();
    let cap = len.clamp(100, 150);
    if len > cap {
        let mut s: String = text.chars().take(cap).collect();
        s.push_str("...");
        s
    } else {
        text.to_string()
    }
}

impl Host {
    /// err_harness_frame 复刻（case b；fresh 恒 false——驱动语境）。
    fn harness_frame(&mut self, msg: &str, text: &str, line: usize) {
        match &mut self.info {
            Some(info) => {
                info.push_str(&format!(
                    "\n    invoked from within\n\"{}\"",
                    log_excerpt(text)
                ));
                self.err_line = line;
            }
            None => {
                self.info = Some(format!(
                    "{}\n    while executing\n\"{}\"",
                    msg,
                    log_excerpt(text)
                ));
                self.err_line = line;
            }
        }
        self.pending_top = None;
    }

    /// unwind_subs 复刻（Sub → defer 最外层；Expr → append；
    /// Body/Next → 无）。
    fn unwind(&mut self, bodies: &[(crate::ir::BodyKind, u32)], e: &Error) {
        for (kind, site) in bodies {
            let text = self.site_text(*site).to_string();
            let line = self.lines.get(*site as usize).copied().unwrap_or(0) as usize;
            match kind {
                crate::ir::BodyKind::Sub => {
                    self.pending_top = Some(text);
                    self.err_line = line;
                }
                crate::ir::BodyKind::Expr => {
                    let msg = e.message_text();
                    self.harness_frame(&msg, &text, line);
                }
                crate::ir::BodyKind::Body | crate::ir::BodyKind::Next => {}
            }
        }
    }

    /// case b 的 fail 处置：分类 + （错误时）帧追加。返回 kind。
    fn take_fail(&mut self, fail_id: u32) -> i32 {
        let Some(p) = self.pending.take() else { return 0 };
        let e = p.err;
        if p.from_eval {
            // case a：解释器已按协议累积；只分类。pending 保留给终局报告。
            self.pending = Some(Pending { err: e, from_eval: true });
            return 0;
        }
        let e = match classify_control(&e) {
            0 => e,
            k => return k,
        };
        let fi = self.fails.get(fail_id as usize).cloned();
        if let Some(fi) = fi {
            let frameless = fi
                .bodies
                .last()
                .is_some_and(|(_, s)| *s == fi.site);
            if fi.site != u32::MAX && !frameless {
                let text = self.site_text(fi.site).to_string();
                let line = self.lines.get(fi.site as usize).copied().unwrap_or(0) as usize;
                let msg = e.message_text();
                self.harness_frame(&msg, &text, line);
            }
            self.unwind(&fi.bodies, &e);
        }
        self.pending = Some(Pending { err: e, from_eval: false });
        0
    }

    /// error_report 复刻（case b 终局）。
    fn driver_report(&mut self, file: &str) -> String {
        let e = self.pending.as_ref().map(|p| p.err.clone());
        match e {
            Some(e) if is_real_error(&e) => {
                if let Some(text) = self.pending_top.take() {
                    if let Some(info) = &mut self.info {
                        info.push_str(&format!(
                            "\n    invoked from within\n\"{}\"",
                            log_excerpt(&text)
                        ));
                    }
                }
                let tag = format!("file \"{}\"", file);
                if let Some(info) = &mut self.info {
                    info.push_str(&format!("\n    ({} line {})", tag, self.err_line));
                }
                self.info.take().unwrap_or_else(|| e.to_string())
            }
            Some(e) => e.to_string(),
            None => String::new(),
        }
    }
}

/// 控制流分类（fail 的返回 kind）：0=错误、1/3=break（内层/外层）、
/// 2/4=continue。level≤1 = 属于本单元的内层循环。
fn classify_control(e: &Error) -> i32 {
    if e.is_break() {
        if e.loop_level() > 1 { 3 } else { 1 }
    } else if e.is_continue() {
        if e.loop_level() > 1 { 4 } else { 2 }
    } else {
        0
    }
}

/// expr 操作数的 canonical 引用（braced 优先，双引号转义兜底）。
fn expr_quote(s: &str) -> String {
    let braced_ok = {
        let mut depth: i32 = 0;
        let mut ok = true;
        for c in s.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        ok = false;
                        break;
                    }
                }
                '\\' | '[' | '$' | '"' | ';' => {}
                _ => {}
            }
        }
        ok && depth == 0 && !s.ends_with('\\')
    };
    if braced_ok {
        format!("{{{}}}", s)
    } else {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '\\' | '"' => {
                    out.push('\\');
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

impl Host {
    /// binary_op：宿主完整 Tcl 语义（§3）——`types::eval_expr` 原文路由。
    fn binary_op(&mut self, op: i32, a: i64, b: i64) -> Result<Value, Error> {
        let kind = op;
        let va = self.value_of(a);
        let vb = self.value_of(b);
        let sa = expr_quote(va.as_str());
        let sb = expr_quote(vb.as_str());
        let expr = match kind {
            x if x == BinOp::Add as i32 => format!("{sa} + {sb}"),
            x if x == BinOp::Sub as i32 => format!("{sa} - {sb}"),
            x if x == BinOp::Mul as i32 => format!("{sa} * {sb}"),
            x if x == BinOp::Div as i32 => format!("{sa} / {sb}"),
            x if x == BinOp::Mod as i32 => format!("{sa} % {sb}"),
            x if x == BinOp::Pow as i32 => format!("{sa} ** {sb}"),
            x if x == BinOp::Neg as i32 => format!("-{sa}"),
            x if x == BinOp::Not as i32 => format!("!{sa}"),
            x if x == BinOp::BitNot as i32 => format!("~{sa}"),
            x if x == BinOp::BitAnd as i32 => format!("{sa} & {sb}"),
            x if x == BinOp::BitOr as i32 => format!("{sa} | {sb}"),
            x if x == BinOp::BitXor as i32 => format!("{sa} ^ {sb}"),
            x if x == BinOp::Shl as i32 => format!("{sa} << {sb}"),
            x if x == BinOp::Shr as i32 => format!("{sa} >> {sb}"),
            x if x == BinOp::Eq as i32 => format!("{sa} == {sb}"),
            x if x == BinOp::Ne as i32 => format!("{sa} != {sb}"),
            x if x == BinOp::Lt as i32 => format!("{sa} < {sb}"),
            x if x == BinOp::Gt as i32 => format!("{sa} > {sb}"),
            x if x == BinOp::Le as i32 => format!("{sa} <= {sb}"),
            x if x == BinOp::Ge as i32 => format!("{sa} >= {sb}"),
            x if x == BinOp::StrEq as i32 => format!("{sa} eq {sb}"),
            x if x == BinOp::StrNe as i32 => format!("{sa} ne {sb}"),
            x if x == BinOp::And as i32 => format!("{sa} && {sb}"),
            x if x == BinOp::Or as i32 => format!("{sa} || {sb}"),
            _ => return Err(Error::Msg("jit: bad binary_op".into())),
        };
        let _ = HARNESS_MAX;
        self.interp.eval_expr(&expr)
    }

    /// strict_bool 复刻（expr_funcs::strict_bool 的公共 API 重实现：
    /// 数值非零真、布尔词及其无歧义前缀，否则 exact 错误文本）。
    fn bool_test(&self, v: i64) -> Result<bool, Error> {
        let val = self.value_of(v);
        if let Some(i) = val.as_int() {
            return Ok(i != 0);
        }
        if let Some(f) = val.as_float() {
            return Ok(f != 0.0);
        }
        let lower = val.as_str().trim().to_ascii_lowercase();
        if !lower.is_empty() {
            let candidates =
                ["true", "false", "yes", "no", "on", "off"].into_iter().filter(|w| w.starts_with(&lower));
            let mut it = candidates;
            let first = it.next();
            if let Some(first) = first {
                if it.next().is_none() {
                    return Ok(matches!(first, "true" | "yes" | "on"));
                }
            }
        }
        Err(Error::Msg(format!(
            "expected boolean value but got \"{}\"",
            val.as_str()
        )))
    }
}

/// 顶层驱动：接受则 wasm 执行，否则 Fallback。
pub struct JitRun {
    pub outcome: Outcome,
    /// case b 的终局报告（jitsh 打印）；case a 由调用方走 error_report。
    pub driver_report: Option<String>,
    /// case a 的错误（moved out 供 error_report）。
    pub eval_error: Option<Error>,
    /// run() 的 result 值（set_result）。
    pub result: Option<Value>,
}

pub fn run_top(interp: Interp, script: &str, file: &str) -> (Interp, JitRun) {
    let unit = match ScriptUnit::parse(script) {
        Ok(u) => u,
        Err(_) => {
            return (interp, JitRun {
                outcome: Outcome::Fallback,
                driver_report: None,
                eval_error: None,
                result: None,
            })
        }
    };
    let bc = Compiler::compile_unit(Rc::clone(&unit.source), &unit.commands);
    let module = match emit::compile(&bc, &EmitConfig::default()) {
        Ok(m) => m,
        Err(_) => {
            return (interp, JitRun {
                outcome: Outcome::Fallback,
                driver_report: None,
                eval_error: None,
                result: None,
            })
        }
    };
    execute(interp, &bc, &module, file)
}

fn execute(
    interp: Interp,
    bc: &ByteCode,
    module: &emit::Module,
    file: &str,
) -> (Interp, JitRun) {
    use wasmi::{Caller, Engine, Extern, Linker, Module, Store, TypedFunc};

    // arena 预驻留：0=UNSET 1=哨兵保留 2=EMPTY 3..=常量池。
    let mut arena = vec![Value::unset(), Value::unset(), Value::empty()];
    for c in bc.constants() {
        arena.push(Value::from_str(c));
    }
    let host = Host {
        interp,
        arena,
        consts: bc.constants().to_vec(),
        fails: module.l2.fails.clone(),
        sites: bc.sites.iter().map(|s| s.text).collect(),
        lines: bc.sites.iter().map(|s| s.line).collect(),
        source: Rc::clone(&bc.source),
        result: None,
        pending: None,
        pending_exit: None,
        info: None,
        err_line: 0,
        pending_top: None,
        refuel_chunk: abi::REFUEL_CHUNK,
    };
    let watermark = host.arena.len();

    let engine = Engine::default();
    let wmodule = match Module::new(&engine, &module.bytes[..]) {
        Ok(m) => m,
        Err(e) => panic!("jit: emitted module invalid: {e}"),
    };
    let mut store = Store::new(&engine, host);
    let mut linker = <Linker<Host>>::new(&engine);

    // globals（fuel=0 触发首步充值；epoch 恒 0——驱动 M1' 不失效；
    // caching=1 直到首次 rtcl.call）。句柄克隆进宿主闭包（wasm→宿主
    // 直写 global，§3「宿主充值」）。
    let g_fuel = wasmi::Global::new(&mut store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_epoch = wasmi::Global::new(&mut store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_caching = wasmi::Global::new(&mut store, wasmi::Val::I32(1), wasmi::Mutability::Var);
    linker.define("rtcl", "fuel", Extern::Global(g_fuel.clone())).unwrap();
    linker.define("rtcl", "epoch", Extern::Global(g_epoch.clone())).unwrap();
    linker.define("rtcl", "caching", Extern::Global(g_caching.clone())).unwrap();
    let g_fuel_call = g_fuel.clone();
    let g_caching_call = g_caching.clone();

    linker
        .func_wrap("rtcl", "unbox_int", |mut c: Caller<'_, Host>, h: i32| -> (i32, i64) {
            let v = c.data().arena.get((h >> 1) as usize).cloned().unwrap_or_else(Value::empty);
            match v.as_int() {
                Some(n) => (1, n),
                None => (0, 0),
            }
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "unbox_float", |mut c: Caller<'_, Host>, h: i32| -> (i32, f64) {
            let v = c.data().arena.get((h >> 1) as usize).cloned().unwrap_or_else(Value::empty);
            match v.as_float() {
                Some(f) => (1, f),
                None => (0, 0.0),
            }
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "box", |mut c: Caller<'_, Host>, v: i64| -> i32 {
            let val = c.data().value_of(v);
            let idx = c.data().arena.len();
            c.data_mut().arena.push(val);
            ((idx as u64) << 1) as i32
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "box_float", |mut c: Caller<'_, Host>, f: f64| -> i32 {
            let val = Value::from_float(f);
            let idx = c.data().arena.len();
            c.data_mut().arena.push(val);
            ((idx as u64) << 1) as i32
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "binary_op", |mut c: Caller<'_, Host>, op: i32, a: i64, b: i64| -> i64 {
            match c.data_mut().binary_op(op, a, b) {
                Ok(v) => {
                    let t = c.data_mut().tag_value(&v);
                    t
                }
                Err(e) => {
                    c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "get_var", |mut c: Caller<'_, Host>, name_h: i32| -> i64 {
            let name = c.data().consts[((name_h >> 1) as usize).saturating_sub(abi::CONST_ARENA_BASE as usize)].clone();
            match c.data_mut().interp.read_var(&name) {
                Ok(v) => {
                    let t = c.data_mut().tag_value(&v);
                    t
                }
                Err(e) => {
                    c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "set_var", |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i32 {
            let name = c.data().consts[((name_h >> 1) as usize).saturating_sub(abi::CONST_ARENA_BASE as usize)].clone();
            let val = c.data().value_of(v);
            match c.data_mut().interp.set_var(&name, val) {
                Ok(_) => 0,
                Err(e) => {
                    c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                    1
                }
            }
        })
        .unwrap();
    linker
        .func_wrap(
            "rtcl",
            "incr_var",
            |mut c: Caller<'_, Host>, name_h: i32, n: i64| -> i64 {
                let name =
                    c.data().consts[((name_h >> 1) as usize).saturating_sub(abi::CONST_ARENA_BASE as usize)].clone();
                let amount = abi::untag(n);
                match c.data_mut().interp.incr_var(&name, amount) {
                    Ok(v) => c.data_mut().tag_value(&v),
                    Err(e) => {
                        c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                        abi::ERR_SENTINEL
                    }
                }
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "rtcl",
            "call",
            move |mut c: Caller<'_, Host>, _cmd_id: i32, site: i32, _argc: i32| -> i64 {
                // 宿主 eval 路由：站点原文 + 换行前缀（行号对齐）。
                // 新进程不变量落地：首次命令调用即关闭变量缓存。
                let _ = g_caching_call.set(&mut c, wasmi::Val::I32(0));
                let site = site.max(0) as u32;
                let text = c.data().site_text(site).to_string();
                let line = c.data().lines.get(site as usize).copied().unwrap_or(1).max(1) as usize;
                let script = format!("{}{}", "\n".repeat(line - 1), text);
                match c.data_mut().interp.eval(&script) {
                    Ok(v) => c.data_mut().tag_value(&v),
                    Err(e) => {
                        c.data_mut().pending = Some(Pending { err: e, from_eval: true });
                        abi::ERR_SENTINEL
                    }
                }
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "set_result", |mut c: Caller<'_, Host>, v: i64| {
            let val = c.data().value_of(v);
            c.data_mut().result = Some(val);
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "refuel", move |mut c: Caller<'_, Host>| -> i64 {
            let chunk = c.data().refuel_chunk;
            for _ in 0..chunk {
                if let Err(e) = VmContext::charge_step(&mut c.data_mut().interp) {
                    c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                    return abi::ERR_SENTINEL;
                }
            }
            let _ = g_fuel_call.set(&mut c, wasmi::Val::I64(chunk));
            0
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "fail", |mut c: Caller<'_, Host>, fail_id: i32| -> i32 {
            c.data_mut().take_fail(fail_id as u32)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "bool_test", |mut c: Caller<'_, Host>, v: i64| -> i64 {
            match c.data().bool_test(v) {
                Ok(b) => abi::tag_imm(b as i64),
                Err(e) => {
                    c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "push_const", |mut c: Caller<'_, Host>, idx: i32| -> i64 {
            let h = abi::const_handle(idx as u32);
            h
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "exit", |mut c: Caller<'_, Host>, code: i32| {
            c.data_mut().pending_exit = Some(code);
        })
        .unwrap();

    let instance = linker.instantiate_and_start(&mut store, &wmodule).unwrap_or_else(|e| {
        panic!("jit: instantiate failed: {e}")
    });
    let run: TypedFunc<(), i32> = instance.get_typed_func(&store, "run").unwrap();


    let rc = run.call(&mut store, ()).unwrap_or_else(|trap| {
        // wasm trap = 内部 bug（越界/除零未兜底等）——显式失败。
        panic!("jit: wasm trap: {trap:?}")
    });

    // arena 回收到水位线（§3 纪律）。
    store.data_mut().arena.truncate(watermark);

    let mut host = store.into_data();
    // 首次 call 关缓存：在 call import 内部做（读取 cur_site 与
    // caching global 的通道在实例上）；上面 instantiate 后统一处理。
    let outcome_code = rc;
    let pending_exit = host.pending_exit.take();
    let (driver_report, eval_error) = match outcome_code {
        code::ERROR => {
            let is_case_a = host.pending.as_ref().is_some_and(|p| p.from_eval);
            if is_case_a {
                let e = host.pending.take().unwrap().err;
                (None, Some(e))
            } else {
                let r = host.driver_report(file);
                (Some(r), None)
            }
        }
        code::RETURN | code::BREAK | code::CONTINUE => {
            let e = match outcome_code {
                code::RETURN => Error::ret(host.result.clone()),
                code::BREAK => Error::brk(),
                _ => Error::cont(),
            };
            host.pending = Some(Pending { err: e, from_eval: false });
            let r = host.driver_report(file);
            (Some(r), None)
        }
        _ => (None, None),
    };
    let result = host.result.take();
    let interp = host.interp;
    (
        interp,
        JitRun {
            outcome: Outcome::Completed { code: outcome_code, exit_code: pending_exit },
            driver_report,
            eval_error,
            result,
        },
    )
}
