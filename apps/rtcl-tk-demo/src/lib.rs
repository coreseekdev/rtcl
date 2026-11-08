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

menu .menubar -tearoff 0
menu .file -tearoff 0
.file add command -label "Reset" -command {
    set ::clicks 0
    .b configure -text "Click me"
    set ::greeting "Reset from menu"
}
.menubar add cascade -label "File" -menu .file
. configure -menu .menubar

label .later -text "waiting for after"
pack .later -padx 12 -pady 4
after 80 {.later configure -text "Later fired"}
"#;

/// A classic grid/pack form, close to what a Tk tutorial app looks like.
pub const CLASSIC: &str = r##"
font create Heading -family {DejaVu Sans} -size 18 -weight bold
label .title -text "Classic Tk Demo" -font Heading
pack .title -padx 8 -pady 4

set ::name "World"
proc greeting {name} {
    if {$name eq ""} { return "Hello, stranger" }
    return "Hello, $name"
}

frame .form -borderwidth 1 -relief groove
label .form.nameLabel -text "Name:"
entry .form.nameEntry -textvariable ::name -width 24
button .form.ok -text OK -command {
    set ::message [greeting $::name]
}
grid .form.nameLabel -row 0 -column 0 -sticky w
grid .form.nameEntry -row 0 -column 1 -sticky ew -padx 4
grid .form.ok -row 1 -column 1 -sticky e
grid columnconfigure .form 1 -weight 1
pack .form -fill x -padx 8 -pady 8

message .msg -textvariable ::message -width 320
pack .msg -padx 8

frame .listframe
pack .listframe -fill x -padx 8
listbox .listframe.lb -height 3
pack .listframe.lb -side left -fill both -expand 1
scrollbar .listframe.sb -orient vertical -command {.listframe.lb yview}
pack .listframe.sb -side right -fill y
.listframe.lb configure -yscrollcommand {.listframe.sb set}
foreach color {Red Green Blue Yellow Purple Orange Black White} {
    .listframe.lb insert end $color
}

ttk::notebook .nb
pack .nb -fill x -padx 8 -pady 6
frame .nb.general -text "General"
frame .nb.advanced -text "Advanced"
label .nb.general.note -text "General page"
pack .nb.general.note -padx 8 -pady 4
label .nb.advanced.note -text "Advanced page"
pack .nb.advanced.note -padx 8 -pady 4
.nb add .nb.general
.nb add .nb.advanced

set ::size "Medium"
ttk::combobox .cb -values {Small Medium Large} -textvariable ::size
pack .cb -padx 8 -pady 4

ttk::treeview .tv -columns {size} -height 3 -show {tree headings}
.tv heading #0 -text Folder
.tv heading size -text Size
.tv insert {} end -id root -text Project -open 1 -values {0 files}
.tv insert root end -id src -text src -values {3 files}
.tv insert root end -text README -values {1 file}
pack .tv -fill x -padx 8 -pady 4

ttk::separator .sep
pack .sep -fill x -pady 4

ttk::progressbar .pb -value 40 -maximum 100
pack .pb -fill x -padx 8

ttk::labelframe .lf -text "Options"
pack .lf -fill x -padx 8 -pady 4
label .lf.hint -text "labelframe body"
pack .lf.hint -padx 6 -pady 2
label .themed -text "Themed label" -background "#ffe08a" -foreground "#7a1f1f"
pack .themed -padx 8 -pady 4

::rtcl::chart .chart -title "Sales" -values {3 7 5} -labels {A B C} -width 320 -height 140
pack .chart -padx 8 -pady 4

canvas .cv -width 240 -height 90 -background "#f8fafc"
.cv create rectangle 10 10 110 70 -fill "#93c5fd" -outline "#1d4ed8"
.cv create oval 130 10 220 70 -fill "#fca5a5"
.cv create line 20 80 220 80 -fill "#111827" -width 2
.cv create text 120 40 -text "Canvas"
pack .cv -padx 8 -pady 4

focus .form.nameEntry
bind .form.nameEntry <Return> {.form.ok invoke}
set ::message ""
"##;

thread_local! {
    /// The engine-thread interpreter.  Created on the first render so the
    /// GTK runner and headless nsh-testing share exactly one setup.
    pub static INTERP: RefCell<Option<Interp>> = const { RefCell::new(None) };
}

pub fn ensure_tcl() {
    ensure_tcl_script(DEMO);
}

pub fn ensure_tcl_script(script: &str) {
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
        if let Err(e) = interp.eval(script) {
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

/// The [`CLASSIC`] form, for the regression test.

/// Minimal treeview probe/regression app.
pub const TREE: &str = r#"
ttk::treeview .tv -columns {size} -height 3 -show {tree headings}
.tv heading #0 -text Folder
.tv heading size -text Size
.tv insert {} end -id root -text Project -open 1 -values {0 files}
.tv insert root end -id src -text src -values {3 files}
pack .tv -fill x -padx 8 -pady 8
"#;

#[component]
pub fn TreeApp() -> impl IntoElement {
    ensure_tcl_script(TREE);
    rtcl_tk_nsh::tk_view()
}

#[component]
pub fn ClassicApp() -> impl IntoElement {
    ensure_tcl_script(CLASSIC);
    rtcl_tk_nsh::tk_view()
}
