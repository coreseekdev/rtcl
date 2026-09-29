use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use rtcl_core::Value;

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    /// JS callbacks bucketed per interpreter id — instances never see each other's commands.
    static JS_COMMANDS: RefCell<HashMap<u64, HashMap<String, js_sys::Function>>> =
        RefCell::new(HashMap::new());
}

/// Allocate a unique id for a new interpreter instance.
pub fn next_interp_id() -> u64 {
    NEXT_ID.with(|id| {
        let v = id.get();
        id.set(v + 1);
        v
    })
}

/// Native command `_js_dispatch <interpId> <name> <args...>` — dispatches to the
/// JS callback registered for `interpId`. The id is baked into the generated
/// proc wrapper, so each interpreter only ever reaches its own bucket.
pub fn js_dispatch_cmd(_interp: &mut rtcl_core::Interp, args: &[Value]) -> rtcl_core::error::Result<Value> {
    if args.len() < 3 {
        return Err(rtcl_core::error::Error::wrong_args("_js_dispatch", 3, args.len()));
    }

    let interp_id: u64 = args[1].to_string().parse().unwrap_or(u64::MAX);
    let cmd_name = args[2].to_string();

    let callback = JS_COMMANDS.with(|cmds| {
        cmds.borrow().get(&interp_id).and_then(|b| b.get(&cmd_name)).cloned()
    });

    match callback {
        Some(func) => {
            let js_args = js_sys::Array::new();
            for arg in &args[3..] {
                js_args.push(&JsValue::from_str(&arg.to_string()));
            }

            let result = func.apply(&JsValue::NULL, &js_args).map_err(|e| {
                let msg = js_error_message(&e);
                rtcl_core::error::Error::runtime(
                    format!("JS command '{}' error: {}", cmd_name, msg),
                    rtcl_core::error::ErrorCode::Generic
                )
            })?;

            js_value_to_result(cmd_name, result)
        }
        None => Err(rtcl_core::error::Error::runtime(
            format!("No JS callback registered for '{}'", cmd_name),
            rtcl_core::error::ErrorCode::NotFound
        ))
    }
}

/// Convert a JS return value to a Tcl value via `String()` (JS ToString semantics).
fn js_value_to_result(cmd_name: String, result: JsValue) -> rtcl_core::error::Result<Value> {
    match result.as_string() {
        Some(s) => Ok(Value::from(s)),
        None => {
            let string_fn = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("String"))
                .map_err(|e| rtcl_core::error::Error::runtime(
                    format!("global String() not available: {:?}", e),
                    rtcl_core::error::ErrorCode::Generic
                ))?;
            let s = js_sys::Function::from(string_fn)
                .call1(&JsValue::NULL, &result)
                .map_err(|e| rtcl_core::error::Error::runtime(
                    format!("JS command '{}' returned an unstringifiable value: {}",
                            cmd_name, js_error_message(&e)),
                    rtcl_core::error::ErrorCode::Generic
                ))?;
            Ok(Value::from(s.as_string().unwrap_or_default()))
        }
    }
}

/// Extract a human-readable message from a thrown JS value.
pub fn js_error_message(e: &JsValue) -> String {
    if let Some(s) = e.as_string() {
        return s;
    }
    if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        if let Some(m) = err.message().as_string() {
            return m;
        }
    }
    // Fall back to JS String() conversion.
    if let Ok(sf) = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("String")) {
        if let Ok(s) = js_sys::Function::from(sf).call1(&JsValue::NULL, e) {
            if let Some(s) = s.as_string() {
                return s;
            }
        }
    }
    "unknown JS error".to_string()
}

/// Register a JS callback under `interpId` and create a Tcl proc wrapper that
/// dispatches back to this interpreter's bucket.
pub fn register_js_command(interp: &mut rtcl_core::Interp, interp_id: u64, name: &str, func: js_sys::Function) {
    JS_COMMANDS.with(|cmds| {
        cmds.borrow_mut()
            .entry(interp_id)
            .or_default()
            .insert(name.to_string(), func);
    });

    let wrapper = format!(
        "proc {name} {{args}} {{ _js_dispatch {id} {name} {{*}}$args }}",
        name = name,
        id = interp_id,
    );
    let _ = interp.eval(&wrapper);
}

/// Remove a JS callback and its Tcl proc wrapper. Returns true if it existed.
pub fn unregister_js_command(interp: &mut rtcl_core::Interp, interp_id: u64, name: &str) -> bool {
    let existed = JS_COMMANDS.with(|cmds| {
        cmds.borrow_mut().get_mut(&interp_id)
            .map(|b| b.remove(name).is_some())
            .unwrap_or(false)
    });
    // `rename <name> ""` deletes a proc in Tcl semantics.
    let _ = interp.eval(&format!("rename {name} \"\""));
    existed
}
