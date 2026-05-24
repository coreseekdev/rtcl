use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use rtcl_core::Value;

thread_local! {
    pub static JS_COMMANDS: RefCell<std::collections::HashMap<String, js_sys::Function>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Native command `_js_call <name> <args...>` — dispatches to registered JS callback.
pub fn js_call_cmd(_interp: &mut rtcl_core::Interp, args: &[Value]) -> rtcl_core::error::Result<Value> {
    if args.len() < 2 {
        return Err(rtcl_core::error::Error::wrong_args("_js_call", 2, args.len()));
    }

    let cmd_name = args[1].to_string();

    let callback = JS_COMMANDS.with(|cmds| {
        cmds.borrow().get(&cmd_name).cloned()
    });

    match callback {
        Some(func) => {
            let js_args = js_sys::Array::new();
            for arg in &args[2..] {
                js_args.push(&JsValue::from_str(&arg.to_string()));
            }

            let result = func.apply(&JsValue::NULL, &js_args).map_err(|e| {
                rtcl_core::error::Error::runtime(
                    format!("JS command '{}' error: {:?}", cmd_name, e),
                    rtcl_core::error::ErrorCode::Generic
                )
            })?;

            match result.as_string() {
                Some(s) => Ok(Value::from(s)),
                None => {
                    let to_string = js_sys::Reflect::get(&result, &JsValue::from_str("toString"))
                        .map_err(|e| rtcl_core::error::Error::runtime(
                            format!("Failed to get toString: {:?}", e),
                            rtcl_core::error::ErrorCode::Generic
                        ))?;

                    if to_string.is_function() {
                        let func = js_sys::Function::from(to_string);
                        let s = func.call0(&result).map_err(|e| rtcl_core::error::Error::runtime(
                            format!("toString() failed: {:?}", e),
                            rtcl_core::error::ErrorCode::Generic
                        ))?;
                        Ok(Value::from(s.as_string().unwrap_or_default()))
                    } else {
                        Ok(Value::empty())
                    }
                }
            }
        }
        None => Err(rtcl_core::error::Error::runtime(
            format!("No JS callback registered for '{}'", cmd_name),
            rtcl_core::error::ErrorCode::NotFound
        ))
    }
}

/// Register a JS callback and create a Tcl proc wrapper.
pub fn register_js_command(interp: &mut rtcl_core::Interp, name: &str, func: js_sys::Function) {
    JS_COMMANDS.with(|cmds| {
        cmds.borrow_mut().insert(name.to_string(), func);
    });

    let wrapper = format!(
        "proc {} {{args}} {{ _js_call {} {{*}}$args }}",
        name, name
    );
    let _ = interp.eval(&wrapper);
}
