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
