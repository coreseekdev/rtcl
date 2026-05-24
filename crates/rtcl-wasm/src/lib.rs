mod js_command;

use wasm_bindgen::prelude::*;
use rtcl_core::Interp;

#[wasm_bindgen]
pub struct RtclHandle {
    interp: Interp,
}

#[wasm_bindgen]
impl RtclHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<RtclHandle, JsValue> {
        let mut handle = RtclHandle {
            interp: Interp::new(),
        };
        handle.interp.register_command("_js_call", js_command::js_call_cmd);
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
        js_command::register_js_command(&mut self.interp, name, callback);
        Ok(())
    }
}
