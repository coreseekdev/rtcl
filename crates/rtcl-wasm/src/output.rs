//! JS 实现的宿主控制台：把 `puts` 输出转发给 JavaScript 回调。

use wasm_bindgen::prelude::*;
use rtcl_core::host::HostConsole;
use crate::js_command::js_error_message;

/// 以 JS 回调为落点的 [`HostConsole`] 实现。
///
/// 每次 `puts` 触发一次回调（含行尾换行，-nonewline 时不带）；回调抛
/// 异常则转成 `Err`，`puts` 报 `error writing "stdout": <消息>`。
///
/// # Safety
/// `Send` 由 `HostConsole` 要求，而 `js_sys::Function` 不是 `Send`。
/// WASM 单线程运行，句柄只从 JS 线程使用，跨 `Send` 界共享回调在实践中
/// 是可靠的。
pub struct JsConsole {
    callback: js_sys::Function,
}

// SAFETY: 单线程 WASM —— 见上方 doc 注释。
unsafe impl Send for JsConsole {}

impl JsConsole {
    pub fn new(callback: js_sys::Function) -> Self {
        JsConsole { callback }
    }

    fn write(&self, s: &str) -> Result<(), String> {
        self.callback
            .call1(&JsValue::NULL, &JsValue::from_str(s))
            .map(|_| ())
            .map_err(|e| js_error_message(&e))
    }
}

impl HostConsole for JsConsole {
    fn write_stdout(&self, s: &str) -> Result<(), String> {
        self.write(s)
    }

    // 与既有 wasm 行为一致：stderr 保持静默（原生 wasm 的 stdio 落点
    // 本身也是无声丢弃）。需要 stderr 分流时给 HostConsole 增补第二个
    // 回调即可。
    fn write_stderr(&self, _s: &str) -> Result<(), String> {
        Ok(())
    }
}
