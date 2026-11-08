//! Engine-thread Tcl/Tk setup for the GTK4 demo and its headless tests.
//!
//! Tcl is single-threaded and must live on the nsh engine thread (the UI
//! thread); `DemoApp` creates it on first render.  The native entry point is
//! `main.rs`; tests mount the same component with `nsh-testing`.

use nsh_core::prelude::*;
use rtcl_core::Interp;
use std::cell::RefCell;

/// A small form exercising the M1b surface: `-textvariable`, entry
/// editing, checkbutton, scale, listbox, `bind` and `after`.
pub const DEMO: &str = r#"
set ::greeting "Hello from Tcl/Tk on GTK4"
set ::name "World"
set ::clicks 0
set ::agreed 0
set ::volume 30

label .greeting -textvariable ::greeting
pack .greeting -padx 12 -pady 6

frame .form
pack .form -fill x -padx 12
label .form.name_label -text "Name:"
pack .form.name_label -side left
entry .form.name -textvariable ::name -width 18
pack .form.name -side left -padx 6
button .form.greet -text "Greet" -command {
    set ::greeting "Hello, $::name!"
}
pack .form.greet -side left

button .b -text "Click me" -command {
    incr ::clicks
    .b configure -text "Clicked $::clicks"
    set ::greeting "Clicks: $::clicks"
}
pack .b -padx 12 -pady 4

checkbutton .agree -text "I agree" -variable ::agreed -onvalue 1 -offvalue 0 -command {
    set ::greeting [expr {$::agreed ? "Agreed" : "Not agreed"}]
}
pack .agree -padx 12 -anchor w

label .volume_label -text "Volume:"
pack .volume_label -padx 12 -pady 2
scale .volume -from 0 -to 100 -variable ::volume -command {
    set ::greeting "Volume $::volume"
}
pack .volume -fill x -padx 12 -pady 4

listbox .colors -height 3
pack .colors -fill x -padx 12 -pady 4
.colors insert end Red Green Blue

label .later -text "waiting for after"
pack .later -padx 12 -pady 4
after 80 {.later configure -text "Later fired"}

# `shot` runs the app with RTCL_TK_AUTO=1 to capture an interacted state.
if {[info exists ::env(RTCL_TK_AUTO)]} {
    after 200 {
        .b invoke
        .agree invoke
        .colors selection set 1
        .volume set 65
    }
}
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
