//! Instantiation engines (feature-gated).
//!
//! - `jit-wasm`: the JS WebAssembly API via `js-sys` — the primary
//!   production path (`wasm32-unknown-unknown` in browsers / node).
//!   The import object is built by the embedder; the three host functions
//!   the M0 ABI requires are `rtcl.push_int`, `rtcl.set_result` and
//!   `rtcl.push_empty` (see [`crate::emit`]).
//! - `jit-native`: wasmtime embedding — lands with JIT M1, when native
//!   execution becomes a bench target.  The feature exists now so both
//!   feature-matrix states compile.
//!
//! wasip1 and embedded no_std builds never JIT and never touch this
//! module.

#[cfg(feature = "jit-wasm")]
pub mod js {
    use js_sys::{Object, WebAssembly};
    use wasm_bindgen::{JsCast, JsValue};

    /// A compiled + instantiated jit module.
    pub struct JitInstance {
        instance: WebAssembly::Instance,
    }

    /// Compile `bytes` and instantiate it against the embedder-supplied
    /// import object.  Mirrors `new WebAssembly.Instance(new
    /// WebAssembly.Module(bytes), imports)` — synchronous, so the module
    /// must stay small (per-proc modules are <4KB, HANDOFF §5).
    pub fn instantiate(bytes: &[u8], imports: &Object) -> Result<JitInstance, JsValue> {
        // Uint8Array::from copies; the module section of the resulting
        // wasm keeps its own copy of everything it needs, so the caller's
        // buffer lifetime is not tied to the instance.
        let array = js_sys::Uint8Array::from(bytes);
        let module = WebAssembly::Module::new(&array)?;
        let instance = WebAssembly::Instance::new(&module, imports)?;
        Ok(JitInstance { instance })
    }

    impl JitInstance {
        /// Call the exported `run() -> i32` (Tcl result code).
        pub fn run(&self) -> Result<i32, JsValue> {
            let f = js_sys::Reflect::get(self.instance.exports().as_ref(), &"run".into())?;
            let f: js_sys::Function = f.dyn_into()?;
            let n = f.call0(&JsValue::NULL)?;
            Ok(n.dyn_into::<js_sys::Number>()?.value_of() as i32)
        }

        /// The instance's `WebAssembly.Instance::exports` object, for
        /// embedders that want to grow the ABI (M1+: typed value
        /// constructors, epoch guard).
        pub fn exports(&self) -> Object {
            self.instance.exports()
        }
    }
}

#[cfg(feature = "jit-native")]
pub mod native {
    //! wasmtime-backed native instantiation — lands with JIT M1
    //! (HANDOFF §5), together with the first native bench target.  The
    //! emitter in [`crate::emit`] is engine-agnostic; only this module
    //! changes.
}
