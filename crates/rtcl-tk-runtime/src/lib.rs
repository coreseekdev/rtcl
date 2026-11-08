//! Tcl/Tk-compatible native UI runtime.
//!
//! This crate is the bridge between the Tcl `tk` command surface and the
//! backend-independent widget model in [`rtcl_tk_core`].  It is enabled by
//! the native-only `tk` feature; the web/AI `ui::*` DSL is a separate track
//! (`docs/UI-DSL.md`) and never depends on this crate.
//!
//! The nsh renderer lives in a backend crate; this runtime owns only the
//! shadow tree and the interpreter-facing command layer.

#![cfg_attr(target_arch = "wasm32", allow(unused))]

#[cfg(target_arch = "wasm32")]
compile_error!(
    "rtcl-tk-runtime (native Tk/nsh) is native-only; it is mutually \
     exclusive with the wasm target. Use ui::* for web/WebView UI instead."
);

pub use rtcl_tk_core::{TkError, TkTree, WidgetKind};

/// One interpreter's Tk state.  The Tcl command layer (M1) mutates
/// [`TkRuntime::tree`]; a native backend renders snapshots of it.
#[derive(Debug, Default)]
pub struct TkRuntime {
    tree: TkTree,
}

impl TkRuntime {
    pub fn new() -> Self {
        Self { tree: TkTree::new() }
    }

    pub fn tree(&self) -> &TkTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut TkTree {
        &mut self.tree
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_exists_and_children_are_tk_paths() {
        let mut rt = TkRuntime::new();
        rt.tree_mut()
            .create(".", "b", WidgetKind::Button)
            .unwrap();
        assert_eq!(rt.tree().children(".").unwrap(), [".b"]);
    }
}
