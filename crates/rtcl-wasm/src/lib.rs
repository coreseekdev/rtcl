mod js_command;
mod clock;
mod output;
pub mod jit_abi;

use wasm_bindgen::prelude::*;
use rtcl_core::Interp;

#[wasm_bindgen]
pub struct RtclHandle {
    interp: Interp,
    id: u64,
}

#[wasm_bindgen]
impl RtclHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<RtclHandle, JsValue> {
        let mut handle = RtclHandle {
            interp: Interp::new(),
            id: js_command::next_interp_id(),
        };
        handle.interp.register_command("_js_dispatch", js_command::js_dispatch_cmd);
        handle.interp.set_clock(Box::new(clock::JsClock));
        handle
            .interp
            .load_ui()
            .map_err(|e| JsValue::from_str(&format!("ui.tcl failed to load: {e}")))?;
        handle
            .interp
            .load_http()
            .map_err(|e| JsValue::from_str(&format!("http.tcl failed to load: {e}")))?;
        handle
            .interp
            .load_log()
            .map_err(|e| JsValue::from_str(&format!("log.tcl failed to load: {e}")))?;
        handle
            .interp
            .load_rules()
            .map_err(|e| JsValue::from_str(&format!("rules.tcl failed to load: {e}")))?;
        Ok(handle)
    }

    /// Execute a Tcl script and return the result as a string.
    pub fn exec(&mut self, code: &str) -> Result<String, JsValue> {
        // ⑧ 重入哨兵：JIT wasm 调用进行中（JS 命令回调里再调 exec），
        // 外层 wasm 帧仍持有解释器——明确拒绝而非未定义行为。
        if rtcl_vm::interp::jit_call_in_flight() {
            return Err(JsValue::from_str(
                "reentrant execution during JIT call is not supported",
            ));
        }
        self.interp
            .eval(code)
            .map(|v| v.to_string())
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Register a JavaScript function as a Tcl command.
    pub fn register_command(&mut self, name: &str, callback: js_sys::Function) -> Result<(), JsValue> {
        if rtcl_vm::interp::jit_call_in_flight() {
            return Err(JsValue::from_str(
                "reentrant registration during JIT call is not supported",
            ));
        }
        js_command::register_js_command(&mut self.interp, self.id, name, callback);
        Ok(())
    }

    /// Remove a JS-registered command.
    pub fn unregister_command(&mut self, name: &str) -> Result<bool, JsValue> {
        if rtcl_vm::interp::jit_call_in_flight() {
            return Err(JsValue::from_str(
                "reentrant unregistration during JIT call is not supported",
            ));
        }
        Ok(js_command::unregister_js_command(&mut self.interp, self.id, name))
    }

    /// Tcl reader check: is `code` a complete script (braces, quotes and
    /// brackets balanced)?  Web REPLs call this to decide whether to keep
    /// reading continuation lines — the same oracle `info complete` and the
    /// native CLI REPL use (`rtcl_parser::is_complete`).
    pub fn is_complete(&self, code: &str) -> bool {
        rtcl_parser::is_complete(code)
    }

    /// Route `puts` (stdout) output to a JavaScript callback.
    ///
    /// 注入 rtcl-core 的 `HostConsole` 宿主控制台服务：无 `io` 特性构建
    /// 下 `puts` 的 stdout/stderr 都经该 trait 落到 JS。
    pub fn set_output_handler(&mut self, callback: js_sys::Function) -> Result<(), JsValue> {
        self.interp
            .set_console(Box::new(output::JsConsole::new(callback)));
        Ok(())
    }

    /// Inject an external event (UI action, host callback): queue the
    /// script and immediately pump all ready events. 事件回调脚本的错误
    /// 不静默——收集后以 `Err` 抛给 JS，供 shell 呈现（对应
    /// docs/UI-DSL.md 的 bgerror 回路）。
    pub fn dispatch_event(&mut self, script: &str) -> Result<(), JsValue> {
        if rtcl_vm::interp::jit_call_in_flight() {
            return Err(JsValue::from_str(
                "reentrant dispatch during JIT call is not supported",
            ));
        }
        self.interp.post_event(script);
        let errors = self.interp.pump_events();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(JsValue::from_str(&errors.join("\n")))
        }
    }
}

/// Native-side API (not exported to JS — this impl block deliberately carries
/// no `#[wasm_bindgen]`). Gives Rust hosts the same power the crate uses
/// internally in `RtclHandle::new()` — registering native `CommandFunc`s —
/// plus full interpreter configuration, so a host embedding rtcl does not
/// need to maintain a parallel binding stack.
impl RtclHandle {
    /// Direct access to the underlying interpreter, e.g.
    /// `handle.interp().register_command("my.cmd", my_cmd_func)`.
    pub fn interp(&mut self) -> &mut Interp {
        &mut self.interp
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use rtcl_core::Value;

    #[test]
    fn native_host_can_register_rust_commands() {
        let mut handle = RtclHandle::new().expect("handle");
        handle
            .interp()
            .register_command("hj_double", |_interp, args| {
                let n = args[1].as_int().unwrap_or(0);
                Ok(Value::from_int(n * 2))
            });
        assert_eq!(handle.exec("hj_double 21").expect("exec"), "42");
    }

    #[test]
    fn native_registration_is_isolated_from_js_bridge() {
        let mut handle = RtclHandle::new().expect("handle");
        handle
            .interp()
            .register_command("mk", |_interp, args| {
                Ok(Value::from_str(&format!("native<{}>", args[1].as_str())))
            });
        assert_eq!(handle.exec("mk x").expect("exec"), "native<x>");
        // _js_dispatch (registered internally) still present.
        assert!(handle.exec("info commands _js_dispatch").expect("exec").contains("_js_dispatch"));
    }
}
