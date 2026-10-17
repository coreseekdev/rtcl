//! JS 实现的宿主时钟：给 `after`/`vwait`/`time` 接上 `Date.now`。
//!
//! 浏览器 Worker 不能同步睡眠（阻塞会饿死 postMessage 事件投递），
//! 所以 [`HostClock::sleep_ms`] 恒返回 `Err` —— `after ms`（无脚本）
//! 与 `vwait` 的阻塞等待分支在浏览器里报 Tcl 错误，脚本必须走
//! `after ms script` 的事件驱动形式（docs/UI-DSL.md §4）。

use rtcl_core::host::HostClock;

/// 以 `Date.now` 为落点的 [`HostClock`] 实现。
pub struct JsClock;

impl HostClock for JsClock {
    fn now_ms(&self) -> f64 {
        js_sys::Date::now()
    }

    fn sleep_ms(&self, _ms: u64) -> Result<(), String> {
        Err("synchronous sleep is not available in the browser host; \
             use `after ms script` and the event loop instead"
            .to_string())
    }
}
