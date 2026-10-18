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
pub(crate) struct Pending {
    err: Error,
    from_eval: bool,
}

pub(crate) struct Host {
    pub(crate) interp: Interp,
    pub(crate) arena: Vec<Value>,
    pub(crate) consts: Vec<String>,
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
    /// foreach/lmap 宿主态栈（lists/collected）。
    pub(crate) foreaches: Vec<ForeachHost>,
    /// ByteCode 的 foreach 形状表（start/bind 目标用）。
    pub(crate) infos: Vec<rtcl_parser::bytecode::ForeachInfo>,
    /// 最近 rtcl.fail 的表索引（case-a 帧构建用）。
    pub(crate) last_fail: Option<u32>,
    /// 自首次任意宿主命令求值后置位——此后 var_traces/dead_flat 可能
    /// 非空（take_var_fast 的守卫集），LappendVar 等保守路径据此分支。
    pub(crate) commands_ran: bool,
}

/// 一个活动 foreach/lmap 的宿主态。
pub(crate) struct ForeachHost {
    pub lists: Vec<Vec<Value>>,
    pub collected: Option<Vec<Value>>,
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

    /// err_exit_frame 复刻：`\n    (tag line N)`（累积器为空则无操作）。
    fn exit_frame(&mut self, tag: &str) {
        if let Some(info) = &mut self.info {
            info.push_str(&format!("\n    ({} line {})", tag, self.err_line));
        }
    }

    /// unwind_subs 复刻（Sub → defer 最外层；Expr → append；
    /// Body/Next → 无）。
    fn unwind(&mut self, bodies: &[(crate::ir::BodyKind, u32)], e: &Error) {
        for (kind, site) in bodies.iter().map(|(k, s)| (*k, *s)) {
            let text = self.site_text(site).to_string();
            let line = self.lines.get(site as usize).copied().unwrap_or(0) as usize;
            match kind {
                crate::ir::BodyKind::Sub => {
                    self.pending_top = Some(text);
                    self.err_line = line;
                }
                crate::ir::BodyKind::Expr => {
                    let msg = e.message_text();
                    self.harness_frame(&msg, &text, line);
                }
                crate::ir::BodyKind::BodyFramed(lmap) => {
                    // vm_exec unwind_subs 的 BodyFramed 复刻：
                    // ("foreach"/"lmap" body line N) + 站点 harness 帧。
                    let tag = if lmap { "\"lmap\" body" } else { "\"foreach\" body" };
                    self.exit_frame(tag);
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
        // 宿主原语错误也可能已在 interp 累积帧（如 trace 脚本失败）——
        // 哨兵取出作为基底（无累积器时 = 消息文本，两种情形同构）。
        {
            const SENT: &str = "\u{0}j";
            let raw = self.interp.error_report(&e, SENT);
            let tail = format!("\n    (file \"{}\" line ", SENT);
            let base = match raw.rfind(&tail) {
                Some(i) => Some(raw[..i].to_string()),
                None if raw != e.to_string() => Some(raw),
                None => None,
            };
            if let Some(b) = base {
                self.info = Some(b);
            }
        }
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

    /// case-a 帧构建：interp 累积器（经哨兵标签 error_report 取出——
    /// 空 file 名不可能出现在真实路径，剥尾确定）+ 驱动补帧（EvalExpr
    /// 站点 harness / EvalScript pending / unwind 的 Sub/Expr/BodyFramed）
    /// + 文件帧（err_line 取最后一次 append 的行）。
    pub(crate) fn build_case_a_report(&mut self, fail_id: Option<u32>, e: &Error, file: &str) -> String {
        const SENT: &str = "\u{0}j";
        let raw = self.interp.error_report(e, SENT);
        let tail = format!("\n    (file \"{}\" line ", SENT);
        let base = match raw.rfind(&tail) {
            Some(i) => raw[..i].to_string(),
            None => raw,
        };
        self.info = if base.is_empty() { None } else { Some(base) };
        let mut pending_frame: Option<(String, usize)> = None;
        if let Some(fid) = fail_id {
            if let Some(fi) = self.fails.get(fid as usize).cloned() {
                let text = self.site_text(fi.site).to_string();
                let line = self.lines.get(fi.site as usize).copied().unwrap_or(0) as usize;
                // err_line 初始化 = 最内层失败帧的行（vm_exec 的 err_line
                // 由嵌套 eval 的最后一次 append 设置；此处以 fail 站点行
                // 近似——exit_frame 在 harness append 之前读取它）。
                if self.info.is_some() {
                    self.err_line = line;
                }
                match fi.kind {
                    crate::ir::FailKind::EvalExpr => {
                        let msg = e.message_text();
                        self.harness_frame(&msg, &text, line);
                    }
                    crate::ir::FailKind::EvalScript => {
                        pending_frame = Some((text, line));
                    }
                    _ => {}
                }
                self.unwind(&fi.bodies, e);
            }
        }
        // pending 帧最终追加（vm_exec error_report 的 drain 语义）：
        // unwind 的 Sub 覆盖 step-1 的 word pending（外层胜出）。
        // pending_top 只带文本（无行——unwind 已设置 err_line）。
        let mut pending_final = pending_frame;
        if let Some(t) = self.pending_top.take() {
            let l = self.err_line;
            pending_final = Some((t, l));
        }
        if let Some((t, l)) = pending_final {
            if let Some(info) = &mut self.info {
                info.push_str(&format!(
                    "\n    invoked from within\n\"{}\"",
                    log_excerpt(&t)
                ));
                self.err_line = l;
            }
        }
        if let Some(info) = &mut self.info {
            info.push_str(&format!("\n    (file \"{}\" line {})", file, self.err_line));
        }
        self.info.take().unwrap_or_else(|| e.to_string())
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

/// arena 预驻留：0=UNSET 1=哨兵保留 2=EMPTY 3..=常量池。
fn arena_of(bc: &ByteCode) -> Vec<Value> {
    let mut arena = vec![Value::unset(), Value::unset(), Value::empty()];
    for c in bc.constants() {
        arena.push(Value::from_str(c));
    }
    arena
}

impl Host {
    // ── 构造与引擎协作 ──────────────────────────────────────────────

    pub fn new(interp: Interp, arena: Vec<Value>, bc: &ByteCode, module: &emit::Module) -> Host {
        Host {
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
            foreaches: Vec::new(),
            infos: {
                let mut v = Vec::new();
                let mut i = 0u32;
                while let Some(f) = bc.foreach_info(i) {
                    v.push(f.clone());
                    i += 1;
                }
                v
            },
            last_fail: None,
            commands_ran: false,
        }
    }

    pub fn arena_len(&self) -> usize {
        self.arena.len()
    }

    pub fn arena_truncate(&mut self, n: usize) {
        self.arena.truncate(n);
    }

    pub fn refuel_chunk(&self) -> i64 {
        self.refuel_chunk
    }

    pub fn interp_mut(&mut self) -> &mut Interp {
        &mut self.interp
    }

    pub fn set_pending(&mut self, err: Error, from_eval: bool) {
        self.pending = Some(Pending { err, from_eval });
    }

    // ── 宿主逻辑（引擎无关；各引擎的 import 包装只做参数搬运）──────

    pub fn binary_op_raw(&mut self, op: i32, a: i64, b: i64) -> i64 {
        let r = self.do_binary_op(op, a, b);
        r
    }

    fn do_binary_op(&mut self, op: i32, a: i64, b: i64) -> i64 {
        let r = self.binary_op_impl(op, a, b);
        // 布尔值域 op（比较/逻辑）归一为 imm 0/1——vm_exec 的 op_eq/
        // op_rel 直接 from_bool（imm）；eval_expr 路由可能产出字符串
        // 形态（"0"/"1"），下游 BoolOfBool/BoolTest 的 tagged-0/1 假设
        // 依赖 imm 形状。
        const BOOL_OPS: &[i32] = &[
            14, 15, 16, 17, 18, 19, 20, 21, 22, 23, // Eq Ne Lt Gt Le Ge StrEq StrNe And Or
        ];
        if BOOL_OPS.contains(&op) {
            if let Ok(v) = self.value_of_result(r) {
                if let Some(i) = v.as_int() {
                    return abi::tag_imm(i.clamp(0, 1));
                }
            }
        }
        r
    }

    fn value_of_result(&self, tagged: i64) -> Result<Value, ()> {
        if tagged & 1 == 1 {
            return Ok(Value::from_imm(abi::untag(tagged)));
        }
        self.arena
            .get((tagged >> 1) as usize)
            .cloned()
            .ok_or(())
    }

    fn binary_op_impl(&mut self, op: i32, a: i64, b: i64) -> i64 {
        match self.binary_op(op, a, b) {
            Ok(v) => self.tag_value(&v),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn get_var_raw(&mut self, name_h: i32) -> i64 {
        let name = self.const_name(name_h);
        match self.interp.read_var(&name) {
            Ok(v) => {
                let t = self.tag_value(&v);
                t
            }
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn set_var_raw(&mut self, name_h: i32, v: i64) -> i32 {
        let name = self.const_name(name_h);
        let val = self.value_of(v);
        match self.interp.set_var(&name, val) {
            Ok(_) => 0,
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                1
            }
        }
    }

    pub fn incr_var_raw(&mut self, name_h: i32, n: i64) -> i64 {
        let name = self.const_name(name_h);
        match self.interp.incr_var(&name, abi::untag(n)) {
            Ok(v) => self.tag_value(&v),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    /// 命令分派：站点原文 + 换行前缀 eval 路由。
    pub fn call_raw(&mut self, _cmd_id: i32, site: i32, _argc: i32) -> i64 {
        self.commands_ran = true;
        let site = site.max(0) as u32;
        let text = self.site_text(site).to_string();
        let line = self.lines.get(site as usize).copied().unwrap_or(1).max(1) as usize;
        let script = format!("{}{}", "\n".repeat(line - 1), text);
        match self.interp.eval(&script) {
            Ok(v) => self.tag_value(&v),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: true });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn set_result_raw(&mut self, v: i64) {
        let val = self.value_of(v);
        self.result = Some(val);
    }

    pub fn bool_test_raw(&mut self, v: i64) -> i64 {
        match self.bool_test(v) {
            Ok(b) => abi::tag_imm(b as i64),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn unbox_int_raw(&self, h: i32) -> (i32, i64) {
        let v = self.arena.get((h >> 1) as usize).cloned().unwrap_or_else(Value::empty);
        match v.as_int() {
            Some(n) => (1, n),
            None => (0, 0),
        }
    }

    pub fn unbox_float_raw(&self, h: i32) -> (i32, f64) {
        let v = self.arena.get((h >> 1) as usize).cloned().unwrap_or_else(Value::empty);
        match v.as_float() {
            Some(f) => (1, f),
            None => (0, 0.0),
        }
    }

    pub fn box_raw(&mut self, v: i64) -> i32 {
        let val = self.value_of(v);
        let idx = self.arena.len();
        self.arena.push(val);
        ((idx as u64) << 1) as i32
    }

    pub fn box_float_raw(&mut self, f: f64) -> i32 {
        let val = Value::from_float(f);
        let idx = self.arena.len();
        self.arena.push(val);
        ((idx as u64) << 1) as i32
    }

    /// `rtcl.concat_n(base_off, n)`：从模块 memory 读 n 个 tagged word，
    /// as_str 连接（vm_exec Concat 语义——永不失败）。
    pub fn concat_raw(&mut self, words: &[u64]) -> i64 {
        let mut joined = String::new();
        for w in words {
            let v = self.value_of(*w as i64);
            joined.push_str(v.as_str());
        }
        let out = Value::from_str(&joined);
        self.tag_value(&out)
    }

    pub fn fail_raw(&mut self, fail_id: i32) -> i32 {
        self.last_fail = Some(fail_id.max(0) as u32);
        self.take_fail(fail_id.max(0) as u32)
    }

    /// `foreach_start(info, words)`：strict 解析 + bind(0)；返回 tagged
    /// iters 或哨兵。
    pub fn foreach_start_raw(&mut self, info: i32, words: &[u64]) -> i64 {
        let info = info.max(0) as usize;
        let Some(fi) = self.infos.get(info).cloned() else {
            return abi::tag_imm(0);
        };
        // vm_exec 弹组序 = 逆序；暂存区升序——逆读。
        let mut lists = Vec::with_capacity(words.len());
        for w in words.iter().rev() {
            let v = self.value_of(*w as i64);
            match v.as_list_ref() {
                Some(items) => lists.push(items.to_vec()),
                None => match v.as_list_strict() {
                    Ok(items) => lists.push(items),
                    Err(e) => {
                        let _ = self.interp.set_var("errorCode", Value::from_str(e.code));
                        self.pending = Some(Pending {
                            err: Error::Msg(e.message),
                            from_eval: false,
                        });
                        return abi::ERR_SENTINEL;
                    }
                },
            }
        }
        let max = fi
            .groups
            .iter()
            .zip(&lists)
            .map(|(g, l)| l.len().div_ceil(g.len().max(1)))
            .max()
            .unwrap_or(0);
        self.foreaches.push(ForeachHost {
            lists,
            collected: fi.lmap.then(Vec::new),
        });
        if max > 0 && self.foreach_bind_idx(&fi, 0).is_err() {
            return abi::ERR_SENTINEL;
        }
        abi::tag_imm(max as i64)
    }

    /// bind 第 idx 次迭代的循环变量（foreach_bind 复刻：空值兜底、
    /// framed 装饰、TCL WRITE VARNAME）。
    fn foreach_bind_idx(
        &mut self,
        fi: &rtcl_parser::bytecode::ForeachInfo,
        idx: usize,
    ) -> Result<(), ()> {
        let empty = Value::empty();
        let groups = fi.groups.clone();
        for (g_i, g) in groups.iter().enumerate() {
            let n = g.len();
            for (vi, t) in g.iter().enumerate() {
                let v = self
                    .foreaches
                    .last()
                    .and_then(|f| f.lists.get(g_i))
                    .and_then(|l| l.get(idx * n + vi))
                    .cloned()
                    .unwrap_or_else(|| empty.clone());
                let name = self.consts.get(t.name_idx as usize).cloned().unwrap_or_default();
                if let Err(e) = self.interp.set_var(&name, v) {
                    if is_real_error(&e) {
                        let _ = self
                            .interp
                            .set_var("errorCode", Value::from_str("TCL WRITE VARNAME"));
                        if fi.framed {
                            let what = if fi.lmap { "lmap" } else { "foreach" };
                            if self.info.is_none() {
                                self.info = Some(e.message_text());
                            }
                            let deco = format!("\n    (setting {} loop variable \"{}\")", what, name);
                            if let Some(is) = &mut self.info {
                                is.push_str(&deco);
                            }
                        }
                    }
                    self.pending = Some(Pending { err: e, from_eval: false });
                    return Err(());
                }
            }
        }
        Ok(())
    }

    pub fn foreach_bind_raw(&mut self, info: i32, idx: i32) -> i64 {
        let fi = match self.infos.get(info.max(0) as usize) {
            Some(f) => f.clone(),
            None => return 0,
        };
        match self.foreach_bind_idx(&fi, idx.max(0) as usize) {
            Ok(()) => 0,
            Err(()) => abi::ERR_SENTINEL,
        }
    }

    pub fn foreach_collect_raw(&mut self, v: i64) {
        let val = self.value_of(v);
        if let Some(f) = self.foreaches.last_mut() {
            if let Some(c) = &mut f.collected {
                c.push(val);
            }
        }
    }

    pub fn foreach_end_raw(&mut self) -> i64 {
        let f = self.foreaches.pop();
        let v = match f.and_then(|f| f.collected) {
            Some(c) => Value::from_list(&c),
            None => Value::empty(),
        };
        self.tag_value(&v)
    }

    /// LappendVar 宿主全语义（vm_exec LappendVar 臂的公共 API 复刻）：
    /// 存在且 list rep → 追加；不存在 → 创建 [value]；非 list rep →
    /// strict 错误（cmd_lappend 的形状）。
    pub fn lappend_var_raw(&mut self, name_h: i32, v: i64) -> i64 {
        let name = self.const_name(name_h);
        let val = self.value_of(v);
        if self.interp.var_exists(&name) {
            match self.interp.get_var(&name) {
                Ok(cur) => match cur.as_list() {
                    Some(mut items) => {
                        items.push(val);
                        let nv = Value::from_list(&items);
                        match self.interp.set_var(&name, nv.clone()) {
                            Ok(_) => self.tag_value(&nv),
                            Err(e) => {
                                self.pending = Some(Pending { err: e, from_eval: false });
                                abi::ERR_SENTINEL
                            }
                        }
                    }
                    None => {
                        // 非 list rep：cmd_lappend 的 strict 错误。
                        let s = cur.as_str().to_string();
                        let probe = Value::from_str(&s);
                        match probe.as_list_strict() {
                            Ok(items) => {
                                // as_list 宽松失败但 strict 成功（罕见形状）
                                let mut items = items;
                                items.push(val);
                                let nv = Value::from_list(&items);
                                match self.interp.set_var(&name, nv.clone()) {
                                    Ok(_) => self.tag_value(&nv),
                                    Err(e) => {
                                        self.pending =
                                            Some(Pending { err: e, from_eval: false });
                                        abi::ERR_SENTINEL
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = self
                                    .interp
                                    .set_var("errorCode", Value::from_str(e.code));
                                self.pending = Some(Pending {
                                    err: Error::Msg(e.message),
                                    from_eval: false,
                                });
                                abi::ERR_SENTINEL
                            }
                        }
                    }
                },
                Err(e) => {
                    self.pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        } else {
            let nv = Value::from_list(&[val]);
            match self.interp.set_var(&name, nv.clone()) {
                Ok(_) => self.tag_value(&nv),
                Err(e) => {
                    self.pending = Some(Pending { err: e, from_eval: false });
                    abi::ERR_SENTINEL
                }
            }
        }
    }

    /// EvalScript 路由：嵌套 eval；错误 from_eval=true（站点 pending 帧
    /// 由 case-a 构建器补）。
    pub fn eval_script_raw(&mut self, v: i64) -> i64 {
        self.commands_ran = true;
        let val = self.value_of(v);
        let script = val.as_str().to_string();
        match self.interp.eval(&script) {
            Ok(r) => self.tag_value(&r),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: true });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn eval_expr_raw(&mut self, v: i64) -> i64 {
        let val = self.value_of(v);
        let expr = val.as_str().to_string();
        match self.interp.eval_expr(&expr) {
            Ok(r) => self.tag_value(&r),
            Err(e) => {
                self.pending = Some(Pending { err: e, from_eval: true });
                abi::ERR_SENTINEL
            }
        }
    }

    pub fn exit_raw(&mut self, code: i32) {
        self.pending_exit = Some(code);
    }

    fn const_name(&self, name_h: i32) -> String {
        self.consts
            [((name_h >> 1) as usize)
                .saturating_sub(abi::CONST_ARENA_BASE as usize)]
        .clone()
    }

    /// binary_op：宿主完整 Tcl 语义（§3）——`types::eval_expr` 原文路由。
    fn binary_op(&mut self, op: i32, a: i64, b: i64) -> Result<Value, Error> {
        // 表达式可含命令替换（eval_expr 求值 [..]）——置位。
        self.commands_ran = true;
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

/// 执行引擎选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineKind {
    /// wasmi（in-process 纯 Rust；judge/bench 默认）。
    #[default]
    Wasmi,
    /// wasmtime（native JIT；feature `jit-native`）。
    Wasmtime,
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
    run_top_with(interp, script, file, engine_from_env())
}

/// `RTCL_JIT_ENGINE=wasmtime|wasmi`（默认 wasmi）。
pub fn engine_from_env() -> EngineKind {
    match std::env::var("RTCL_JIT_ENGINE").as_deref() {
        Ok("wasmtime") if cfg!(feature = "jit-native") => EngineKind::Wasmtime,
        _ => EngineKind::Wasmi,
    }
}

pub fn run_top_with(
    interp: Interp,
    script: &str,
    file: &str,
    engine: EngineKind,
) -> (Interp, JitRun) {
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
    match engine {
        #[cfg(feature = "jit-native")]
        EngineKind::Wasmtime => crate::exec_wasmtime::execute(interp, &bc, &module, file),
        EngineKind::Wasmi => execute(interp, &bc, &module, file),
        #[cfg(not(feature = "jit-native"))]
        EngineKind::Wasmtime => execute(interp, &bc, &module, file),
    }
}

/// 从实例 memory 读 n 个 u64（暂存区协议）。
fn read_words(c: &wasmi::Caller<'_, Host>, base: i32, n: i32) -> Vec<u64> {
    let n_words = n.max(0) as usize;
    let mut bytes = vec![0u8; n_words * 8];
    let mut words = vec![0u64; n_words];
    if let Some(wasmi::Extern::Memory(m)) = c.get_export("memory") {
        let _ = m.read(c, base as usize, &mut bytes);
        for (i, w) in words.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
            *w = u64::from_le_bytes(b);
        }
    }
    words
}

fn execute(
    interp: Interp,
    bc: &ByteCode,
    module: &emit::Module,
    file: &str,
) -> (Interp, JitRun) {
    use wasmi::{Caller, Engine, Extern, Linker, Module, Store, TypedFunc};

    // arena 预驻留：0=UNSET 1=哨兵保留 2=EMPTY 3..=常量池。
    let host = Host::new(interp, arena_of(bc), bc, module);
    let watermark = host.arena_len();

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
    linker.define("rtcl", "fuel", Extern::Global(g_fuel)).unwrap();
    linker.define("rtcl", "epoch", Extern::Global(g_epoch)).unwrap();
    linker.define("rtcl", "caching", Extern::Global(g_caching)).unwrap();
    let g_fuel_call = g_fuel;
    let g_caching_call = g_caching;

    linker
        .func_wrap("rtcl", "unbox_int", |c: Caller<'_, Host>, h: i32| -> (i32, i64) {
            c.data().unbox_int_raw(h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "unbox_float", |c: Caller<'_, Host>, h: i32| -> (i32, f64) {
            c.data().unbox_float_raw(h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "box", |mut c: Caller<'_, Host>, v: i64| -> i32 {
            c.data_mut().box_raw(v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "box_float", |mut c: Caller<'_, Host>, f: f64| -> i32 {
            c.data_mut().box_float_raw(f)
        })
        .unwrap();
    let g_caching_bin = g_caching.clone();
    linker
        .func_wrap("rtcl", "binary_op", move |mut c: Caller<'_, Host>, op: i32, a: i64, b: i64| -> i64 {
            // 表达式可含 [cmd] 替换——缓存窗口关闭。
            let _ = g_caching_bin.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().binary_op_raw(op, a, b)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "get_var", |mut c: Caller<'_, Host>, name_h: i32| -> i64 {
            c.data_mut().get_var_raw(name_h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "set_var", |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i32 {
            c.data_mut().set_var_raw(name_h, v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "incr_var", |mut c: Caller<'_, Host>, name_h: i32, n: i64| -> i64 {
            c.data_mut().incr_var_raw(name_h, n)
        })
        .unwrap();
    linker
        .func_wrap(
            "rtcl",
            "call",
            move |mut c: Caller<'_, Host>, cmd_id: i32, site: i32, argc: i32| -> i64 {
                // 新进程不变量落地：首次命令调用即关闭变量缓存。
                let _ = g_caching_call.set(&mut c, wasmi::Val::I32(0));
                c.data_mut().call_raw(cmd_id, site, argc)
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "set_result", |mut c: Caller<'_, Host>, v: i64| {
            c.data_mut().set_result_raw(v);
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "refuel", move |mut c: Caller<'_, Host>| -> i64 {
            let chunk = c.data().refuel_chunk();
            for _ in 0..chunk {
                if let Err(e) = VmContext::charge_step(c.data_mut().interp_mut()) {
                    c.data_mut().set_pending(e, false);
                    return abi::ERR_SENTINEL;
                }
            }
            let _ = g_fuel_call.set(&mut c, wasmi::Val::I64(chunk));
            0
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "fail", |mut c: Caller<'_, Host>, fail_id: i32| -> i32 {
            c.data_mut().fail_raw(fail_id)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "bool_test", |mut c: Caller<'_, Host>, v: i64| -> i64 {
            c.data_mut().bool_test_raw(v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "push_const", |_c: Caller<'_, Host>, idx: i32| -> i64 {
            
            abi::const_handle(idx as u32)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "concat_n", |mut c: Caller<'_, Host>, base: i32, n: i32| -> i64 {
            let mem = c.get_export("memory").and_then(|e| e.into_memory());
            let n_words = n.max(0) as usize;
            let mut bytes = vec![0u8; n_words * 8];
            let mut words = vec![0u64; n_words];
            if let Some(m) = mem {
                let _ = m.read(&c, base as usize, &mut bytes);
                for (i, w) in words.iter_mut().enumerate() {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
                    *w = u64::from_le_bytes(b);
                }
            }
            c.data_mut().concat_raw(&words)
        })
        .unwrap();
    let g_caching_fe = g_caching.clone();
    linker
        .func_wrap(
            "rtcl",
            "foreach_start",
            move |mut c: Caller<'_, Host>, info: i32, base: i32, ngroups: i32| -> i64 {
                let words = read_words(&c, base, ngroups);
                let _ = g_caching_fe.set(&mut c, wasmi::Val::I32(0));
                c.data_mut().foreach_start_raw(info, &words)
            },
        )
        .unwrap();
    let g_caching_bind = g_caching.clone();
    linker
        .func_wrap(
            "rtcl",
            "foreach_bind",
            move |mut c: Caller<'_, Host>, info: i32, idx: i32| -> i64 {
                let _ = g_caching_bind.set(&mut c, wasmi::Val::I32(0));
                c.data_mut().foreach_bind_raw(info, idx)
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "foreach_collect", |mut c: Caller<'_, Host>, v: i64| {
            c.data_mut().foreach_collect_raw(v);
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "foreach_end", |mut c: Caller<'_, Host>| -> i64 {
            c.data_mut().foreach_end_raw()
        })
        .unwrap();
    let g_caching_lap = g_caching.clone();
    linker
        .func_wrap("rtcl", "lappend_var", move |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i64 {
            let _ = g_caching_lap.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().lappend_var_raw(name_h, v)
        })
        .unwrap();
    let g_caching_es = g_caching.clone();
    linker
        .func_wrap("rtcl", "eval_script", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = g_caching_es.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_script_raw(v)
        })
        .unwrap();
    let g_caching_ee = g_caching.clone();
    linker
        .func_wrap("rtcl", "eval_expr", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = g_caching_ee.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_expr_raw(v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "exit", |mut c: Caller<'_, Host>, code: i32| {
            c.data_mut().exit_raw(code);
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
    crate::exec::finish(store.into_data(), rc, file)
}

/// run() 返回后的共享终局：错误二分（case a/b）与报告构建。
pub(crate) fn finish(mut host: Host, outcome_code: i32, file: &str) -> (Interp, JitRun) {
    let pending_exit = host.pending_exit.take();
    let (driver_report, eval_error) = match outcome_code {
        code::ERROR => {
            let is_case_a = host.pending.as_ref().is_some_and(|p| p.from_eval);
            if is_case_a {
                let e = host.pending.take().unwrap().err;
                // 需要驱动补帧的形态（EvalExpr 站点帧 / EvalScript
                // pending / unwind 有 Sub/Expr/BodyFramed）；纯 Call 的
                // 站点帧已由嵌套 eval 追加——走 error_report 原路。
                let needs_extras = host.last_fail.is_some_and(|f| {
                    host.fails.get(f as usize).is_some_and(|fi| {
                        !matches!(fi.kind, crate::ir::FailKind::Call)
                            || fi
                                .bodies
                                .iter()
                                .any(|(k, _)| !matches!(
                                    k,
                                    crate::ir::BodyKind::Body | crate::ir::BodyKind::Next
                                ))
                    })
                });
                if needs_extras {
                    let r = host.build_case_a_report(host.last_fail, &e, file);
                    (Some(r), None)
                } else {
                    (None, Some(e))
                }
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
