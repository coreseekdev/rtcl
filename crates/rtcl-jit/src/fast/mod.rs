//! fast-tier 子系统（Lane A P1）：shared（引擎无关逻辑层）+ wasmi/wt
//! 两引擎驱动。语义修复单点落地于 shared（p05/dirty/sync/kind5 教训）。

pub(crate) mod shared;
pub(crate) mod wasmi;
pub(crate) mod wt;

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
pub(crate) use wt::fast_try_wt;
