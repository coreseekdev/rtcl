//! Engine-thread Tcl/Tk setup for the GTK4 demo and its headless tests.
//!
//! Tcl is single-threaded and must live on the nsh engine thread (the UI
//! thread); `DemoApp` creates it on first render.  The native entry point is
//! `main.rs`; tests mount the same component with `nsh-testing`.

use nsh_core::prelude::*;
use rtcl_core::Interp;
use std::cell::RefCell;

pub const DEMO: &str = r#"
button .b -text "Click me" -command {
    if {![info exists ::n]} { set ::n 0 }
    incr ::n
    .b configure -text "Clicked $::n"
}
pack .b -padx 12 -pady 6
label .l -text "Hello from Tcl/Tk on GTK4"
pack .l -padx 12 -pady 6
"#;

thread_local! {
    /// The engine-thread interpreter.  Created on the first render so the
    /// GTK runner and headless nsh-testing share exactly one setup.
    pub static INTERP: RefCell<Option<Interp>> = const { RefCell::new(None) };
}

pub fn ensure_tcl() {
    let fresh = INTERP.with(|cell| {
        if cell.borrow().is_some() {
            return false;
        }
        let mut slot = cell.borrow_mut();
        // Store first: tk-runtime records the Interp address as the active
        // engine-thread interpreter, so the value must already live in its
        // stable TLS slot (a stack Interp moved afterwards would leave a
        // dangling active pointer).
        let interp = slot.insert(Interp::new());
        rtcl_tk_runtime::install(interp);
        if let Err(e) = interp.eval(DEMO) {
            eprintln!("demo script: {e}");
        }
        drop(slot);
        true
    });
    if fresh {
        rtcl_tk_nsh::install_backend();
    }
}

#[component]
pub fn DemoApp() -> impl IntoElement {
    ensure_tcl();
    rtcl_tk_nsh::tk_view()
}
