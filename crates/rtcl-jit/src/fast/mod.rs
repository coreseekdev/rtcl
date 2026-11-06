//! fast-tier 子系统（Lane A P1）：shared（引擎无关逻辑层）+ wasmi/wt
//! 两引擎驱动。语义修复单点落地于 shared（p05/dirty/sync/kind5 教训）。

pub(crate) mod shared;
pub(crate) mod wasmi;
// wasmtime 驱动（feature jit-native）：jit-exec-only 构建（jitsh/wasmi）
// 不编入，否则 use wasmtime 在无该依赖时失败。
#[cfg(feature = "jit-native")]
pub(crate) mod wt;

use rtcl_core::Interp;
use rtcl_ir::ByteCode;
use std::rc::Rc;

thread_local! {
    /// fast 调用窗口标志：execute_fast 的 tf.call 期间为真——此时
    /// resolve 分派支的嵌套 exec_proc_body **不得**再进 execute_fast
    ///（FAST_STORE 的 RefCell 已被外层持有，双重借用 panic；u1/t5 冷
    /// callee 实证）。嵌套走常规池（冷路径，一次性——cmd_cache 变热
    /// 后 resolve 走 fast_ensure+exec 支，不经 hook）。
    static FAST_IN_CALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 当前是否处于 fast 调用窗口。
pub(crate) fn fast_in_call() -> bool {
    FAST_IN_CALL.with(|f| f.get())
}

/// fast 调用窗口标志设置（wasmi/wasmtime 驱动共用同一线程本地）。
pub(crate) fn set_fast_in_call(v: bool) {
    FAST_IN_CALL.with(|f| f.set(v))
}

pub(crate) use wasmi::fast_try;
#[cfg(feature = "jit-native")]
pub(crate) use wt::fast_try_wt;

/// 顶层单元的 fast 执行（C4）：引擎分派 + JitRun 映射。None → classic
/// 回落（fast 初始化失败/引擎不可用）。
#[cfg(feature = "jit-native")]
pub(crate) fn execute_fast_top(
    interp: &mut Interp,
    bc: &ByteCode,
    module: &crate::emit::Module,
    meta: crate::emit::FastMeta,
    engine: crate::exec::EngineKind,
) -> Option<crate::exec::JitRun> {
    let bc_rc = Rc::new(bc.clone());
    if engine == crate::exec::EngineKind::Wasmtime {
        return wt::execute_fast_top_wt(interp, &bc_rc, module, meta);
    }
    wasmi::execute_fast_top_wasmi(interp, &bc_rc, module, meta)
}

#[cfg(not(feature = "jit-native"))]
pub(crate) fn execute_fast_top(
    interp: &mut Interp,
    bc: &ByteCode,
    module: &crate::emit::Module,
    meta: crate::emit::FastMeta,
) -> Option<crate::exec::JitRun> {
    let bc_rc = Rc::new(bc.clone());
    wasmi::execute_fast_top_wasmi(interp, &bc_rc, module, meta)
}
