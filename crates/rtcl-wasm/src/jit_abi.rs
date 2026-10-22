//! M1'-5 wasm→wasm 直通接线（TASK-JIT-WASM.md §2.5 / 设计文档 §3）。
//!
//! 宿主以**裸 `extern "C"` 导出**（纯数值签名，不经 wasm-bindgen 包装）
//! 供 JIT 模块的 import object 直塞——JIT→宿主即 wasm→wasm 调用，省 JS
//! trampoline 与 i64↔BigInt 往返（开工前置 bench (a)，V8：直通 1.46
//! ns/call vs trampoline 11.41）。JS trampoline 对照路径并存（§2.5）。
//!
//! JS 侧组装（直通）：
//! ```js
//! const spec = handle.jit_import_spec();          // 导出名/签名清单
//! const globals = { fuel: new WebAssembly.Global({value:'i64',mutable:true}, 1024n),
//!                   epoch: new WebAssembly.Global({value:'i64',mutable:true}, 0n),
//!                   caching: new WebAssembly.Global({value:'i32',mutable:true}, 1) };
//! const direct = {};   // 直通：rtcl-wasm 自身导出直塞
//! for (const f of spec.funcs) direct[f] = wasmExports[f];
//! const tramp = {};    // 对照：JS 闭包转发（BigInt 往返）
//! for (const f of spec.funcs) tramp[f] = (...a) => wasmExports[f](...a);
//! const inst = new WebAssembly.Instance(
//!   new WebAssembly.Module(bytes),
//!   { rtcl: { ...globals, ...(direct_or_tramp) } });
//! ```
//! TODO(M2 wasm 侧): proc-body 钩子（rtcl_jit::proc_exec::install）评估
//! 结论（2026-10-19 合并任务阶段 5，time-boxed）：**暂不接线**。*mut
//! Interp 的安全性论证（单线程同步作用域内调用方借用有效）在 native
//! 侧成立，但 wasm32 的 RtclHandle 经 wasm-bindgen 暴露——JS 可在钩子
//! 执行期间重入任意 &mut RtclHandle 方法（JS 回调命令 `_js_dispatch`），
//! 别名 &mut 的前提被破坏；需要先给执行器加「宿主回调重入哨兵」（如
//! Host 持标志位 + js_command 入口检查）才能安全接线。工作量独立
//! （半个任务包），不影响 native 侧 judge/bench 路径。
//! 导出函数操作「当前 JIT 会话」（`rtcl_jit_session_begin/end` 括起，
//! 单线程、作用域内有效）；`jit_compile`（feature `jit`）提供纯发射。

use crate::RtclHandle;
use rtcl_core::value::Value;
use rtcl_core::{Error, Interp};
use wasm_bindgen::prelude::*;
use std::cell::RefCell;

/// ABI v2 常量（与 `rtcl_jit::abi` 同值；此处独立声明避免引擎依赖）。
const ERR_SENTINEL: i64 = 2;
const CONST_ARENA_BASE: u32 = 3;

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// 当前 JIT 会话：arena + 常量池 + 站点表 + pending 错误 + 解释器指针
/// （scope 内由持柄者保证存活；单线程 wasm 模型）。
struct Session {
    interp: *mut Interp,
    arena: Vec<Value>,
    consts: Vec<String>,
    sites: Vec<(u32, u32)>, // (start, end) into source
    lines: Vec<u32>,
    source: String,
    pending: Option<Error>,
    fuel_chunk: i64,
}

// 单线程作用域指针（Session 只在 begin/end 之间存活）。
unsafe impl Send for Session {}

impl Session {
    fn value_of(&self, tagged: i64) -> Value {
        if tagged & 1 == 1 {
            return Value::from_imm(tagged >> 1);
        }
        self.arena
            .get((tagged >> 1) as usize)
            .cloned()
            .unwrap_or_else(Value::empty)
    }

    fn tag(&mut self, v: &Value) -> i64 {
        if v.is_imm() {
            return ((v.imm() as u64) << 1) as i64 | 1;
        }
        let idx = self.arena.len();
        self.arena.push(v.clone());
        ((idx as u64) << 1) as i64
    }

    fn set_pending(&mut self, e: Error) {
        self.pending = Some(e);
    }
}

fn with_session<R>(f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    SESSION.with(|s| {
        let mut s = s.borrow_mut();
        s.as_mut().map(f)
    })
}

/// expr 操作数 canonical 引用（与 rtcl-jit exec 同款协议）。
fn expr_quote(s: &str) -> String {
    let braced_ok = {
        let mut depth = 0i32;
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

/// strict_bool 复刻（数值非零真；布尔词及无歧义前缀）。
fn strict_bool(v: &Value) -> Result<bool, Error> {
    if let Some(i) = v.as_int() {
        return Ok(i != 0);
    }
    if let Some(f) = v.as_float() {
        return Ok(f != 0.0);
    }
    let lower = v.as_str().trim().to_ascii_lowercase();
    if !lower.is_empty() {
        let words = ["true", "false", "yes", "no", "on", "off"];
        let hits: Vec<&str> = words.iter().copied().filter(|w| w.starts_with(&lower)).collect();
        if hits.len() == 1 {
            return Ok(matches!(hits[0], "true" | "yes" | "on"));
        }
    }
    Err(Error::Msg(format!(
        "expected boolean value but got \"{}\"",
        v.as_str()
    )))
}

fn binary_expr(op: i32, sa: &str, sb: &str) -> String {
    match op {
        0 => format!("{sa} + {sb}"),
        1 => format!("{sa} - {sb}"),
        2 => format!("{sa} * {sb}"),
        3 => format!("{sa} / {sb}"),
        4 => format!("{sa} % {sb}"),
        5 => format!("{sa} ** {sb}"),
        6 => format!("-{sa}"),
        7 => format!("!{sa}"),
        8 => format!("{sa} & {sb}"),
        9 => format!("{sa} | {sb}"),
        10 => format!("{sa} ^ {sb}"),
        11 => format!("~{sa}"),
        12 => format!("{sa} << {sb}"),
        13 => format!("{sa} >> {sb}"),
        14 => format!("{sa} == {sb}"),
        15 => format!("{sa} != {sb}"),
        16 => format!("{sa} < {sb}"),
        17 => format!("{sa} > {sb}"),
        18 => format!("{sa} <= {sb}"),
        19 => format!("{sa} >= {sb}"),
        20 => format!("{sa} eq {sb}"),
        21 => format!("{sa} ne {sb}"),
        22 => format!("{sa} && {sb}"),
        _ => format!("{sa} || {sb}"),
    }
}

// ── 裸 extern "C" 导出（ABI v2；名字 = rtcl_jit_*，js 侧按 spec 直塞）──

#[no_mangle]
pub extern "C" fn rtcl_jit_binary_op(op: i32, a: i64, b: i64) -> i64 {
    with_session(|s| {
        let va = s.value_of(a);
        let vb = s.value_of(b);
        let expr = binary_expr(op, &expr_quote(va.as_str()), &expr_quote(vb.as_str()));
        let r = unsafe { &mut *s.interp }.eval_expr(&expr);
        match r {
            Ok(v) => s.tag(&v),
            Err(e) => {
                s.set_pending(e);
                ERR_SENTINEL
            }
        }
    })
    .unwrap_or(ERR_SENTINEL)
}

#[no_mangle]
pub extern "C" fn rtcl_jit_get_var(name_h: i32) -> i64 {
    with_session(|s| {
        let name = s
            .consts
            .get(((name_h >> 1) as usize).saturating_sub(CONST_ARENA_BASE as usize))
            .cloned()
            .unwrap_or_default();
        match unsafe { &mut *s.interp }.read_var(&name) {
            Ok(v) => s.tag(&v),
            Err(e) => {
                s.set_pending(e);
                ERR_SENTINEL
            }
        }
    })
    .unwrap_or(ERR_SENTINEL)
}

#[no_mangle]
pub extern "C" fn rtcl_jit_set_var(name_h: i32, v: i64) -> i32 {
    with_session(|s| {
        let name = s
            .consts
            .get(((name_h >> 1) as usize).saturating_sub(CONST_ARENA_BASE as usize))
            .cloned()
            .unwrap_or_default();
        let val = s.value_of(v);
        match unsafe { &mut *s.interp }.set_var(&name, val) {
            Ok(_) => 0,
            Err(e) => {
                s.set_pending(e);
                1
            }
        }
    })
    .unwrap_or(1)
}

#[no_mangle]
pub extern "C" fn rtcl_jit_incr_var(name_h: i32, n: i64) -> i64 {
    with_session(|s| {
        let name = s
            .consts
            .get(((name_h >> 1) as usize).saturating_sub(CONST_ARENA_BASE as usize))
            .cloned()
            .unwrap_or_default();
        match unsafe { &mut *s.interp }.incr_var(&name, n >> 1) {
            Ok(v) => s.tag(&v),
            Err(e) => {
                s.set_pending(e);
                ERR_SENTINEL
            }
        }
    })
    .unwrap_or(ERR_SENTINEL)
}

#[no_mangle]
pub extern "C" fn rtcl_jit_bool_test(v: i64) -> i64 {
    with_session(|s| {
        let val = s.value_of(v);
        match strict_bool(&val) {
            Ok(b) => ((b as i64 as u64) << 1) as i64 | 1,
            Err(e) => {
                s.set_pending(e);
                ERR_SENTINEL
            }
        }
    })
    .unwrap_or(ERR_SENTINEL)
}

#[no_mangle]
pub extern "C" fn rtcl_jit_push_const(idx: i32) -> i64 {
    ((CONST_ARENA_BASE as u64 + idx as u64) << 1) as i64
}

#[no_mangle]
pub extern "C" fn rtcl_jit_refuel() -> i64 {
    with_session(|s| {
        let chunk = s.fuel_chunk;
        for _ in 0..chunk {
            if let Err(e) =
                rtcl_vm::VmContext::charge_step(unsafe { &mut *s.interp })
            {
                s.set_pending(e);
                return ERR_SENTINEL;
            }
        }
        0
    })
    .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn rtcl_jit_fail(_fail_id: i32) -> i32 {
    // 错误帧协议由驱动侧（exec::Host 同款逻辑）复刻；裸导出层只报告
    // 「纯错误」（kind 0）——控制流穿越在 wasm32 内嵌场景按宿主策略。
    0
}

#[no_mangle]
pub extern "C" fn rtcl_jit_exit(code: i32) {
    // wasm32 内嵌无进程退出：记录并由驱动解释。
    with_session(|s| {
        let _ = code;
        s.pending = Some(Error::exit(Some(code)));
    });
}

#[no_mangle]
pub extern "C" fn rtcl_jit_session_begin() -> i32 {
    // 会话由持柄者经 bind_jit_session 建立；裸导出层只做存在性探测。
    SESSION.with(|s| s.borrow().is_some()) as i32
}

#[no_mangle]
pub extern "C" fn rtcl_jit_session_end() {
    SESSION.with(|s| {
        *s.borrow_mut() = None;
    });
}

// ── wasm-bindgen 侧 API ───────────────────────────────────────────────

#[wasm_bindgen]
impl RtclHandle {
    /// ABI v2 导出清单（JS 组装直通/对照 import object 用）。
    /// funcs = 裸导出名；globals = 宿主持有的可变 global 形状。
    pub fn jit_import_spec(&self) -> wasm_bindgen::JsValue {
        use wasm_bindgen::JsCast;
        let o = js_sys::Object::new();
        let funcs = js_sys::Array::new();
        for f in [
            "rtcl_jit_binary_op",
            "rtcl_jit_get_var",
            "rtcl_jit_set_var",
            "rtcl_jit_incr_var",
            "rtcl_jit_bool_test",
            "rtcl_jit_push_const",
            "rtcl_jit_refuel",
            "rtcl_jit_fail",
            "rtcl_jit_exit",
        ] {
            funcs.push(&js_sys::JsString::from(f).into());
        }
        js_sys::Reflect::set(&o, &"funcs".into(), &funcs.into()).unwrap();
        let globals = js_sys::Object::new();
        for (n, ty) in [("fuel", "i64"), ("epoch", "i64"), ("caching", "i32")] {
            js_sys::Reflect::set(&globals, &n.into(), &ty.into()).unwrap();
        }
        js_sys::Reflect::set(&o, &"globals".into(), &globals.into()).unwrap();
        o.unchecked_into()
    }

    /// 建立 JIT 会话：arena 预驻留（常量池）+ 站点表 + 解释器绑定。
    /// 供 JS 在实例化 JIT 模块（直通或对照）前调用；配套
    /// `rtcl_jit_session_end`（或下次 begin 覆盖）结束。
    pub fn jit_session_bind(
        &mut self,
        consts: js_sys::Array,
        fuel_chunk: i64,
    ) -> Result<(), wasm_bindgen::JsValue> {
        use wasm_bindgen::JsCast;
        let mut pool: Vec<String> = Vec::new();
        for i in 0..consts.length() {
            let s = consts
                .get(i)
                .as_string()
                .ok_or_else(|| wasm_bindgen::JsValue::from_str("const not a string"))?;
            pool.push(s);
        }
        let mut arena = vec![Value::unset(), Value::unset(), Value::empty()];
        for c in &pool {
            arena.push(Value::from_str(c));
        }
        SESSION.with(|s| {
            *s.borrow_mut() = Some(Session {
                interp: &mut self.interp as *mut Interp,
                arena,
                consts: pool,
                sites: Vec::new(),
                lines: Vec::new(),
                source: String::new(),
                pending: None,
                fuel_chunk,
            });
        });
        Ok(())
    }
}
