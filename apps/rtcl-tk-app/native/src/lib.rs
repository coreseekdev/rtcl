//! The rtcl Tk demo as a Rust app: the Tcl script drives the widget tree,
//! the renderer mounts it through nsh (`root` in `nsh.json`).

use nsh_core::prelude::*;

use rtcl_tk_demo as demo;

/// The app's UI set: registers the root so the hosts serve it.
#[native_module("rtcl-tk-app/native")]
fn native(registry: &mut Registry) {
    registry.register::<TclApp>();
}

/// The demo script (the classic form): the widget tree's source of truth.
pub const SCRIPT: &str = include_str!("../../script.tcl");

/// The root component: installs the Tcl interpreter and renders the tree.
#[component]
pub fn TclApp() -> impl IntoElement {
    demo::ensure_tcl_script(SCRIPT);
    rtcl_tk_nsh::tk_view()
}
