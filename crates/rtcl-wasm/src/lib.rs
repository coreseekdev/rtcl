mod js_command;
mod output;

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
        Ok(handle)
    }

    /// Execute a Tcl script and return the result as a string.
    pub fn exec(&mut self, code: &str) -> Result<String, JsValue> {
        self.interp
            .eval(code)
            .map(|v| v.to_string())
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Register a JavaScript function as a Tcl command.
    pub fn register_command(&mut self, name: &str, callback: js_sys::Function) -> Result<(), JsValue> {
        js_command::register_js_command(&mut self.interp, self.id, name, callback);
        Ok(())
    }

    /// Remove a JS-registered command.
    pub fn unregister_command(&mut self, name: &str) -> Result<bool, JsValue> {
        Ok(js_command::unregister_js_command(&mut self.interp, self.id, name))
    }

    /// Route `puts` (stdout) output to a JavaScript callback.
    pub fn set_output_handler(&mut self, callback: js_sys::Function) -> Result<(), JsValue> {
        self.interp
            .channels
            .set_stdout(Box::new(output::JsOutputChannel::new(callback)));
        Ok(())
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
