//! Headless acceptance for the Tk → nsh pipeline: Tcl builds the tree, nsh
//! renders it, native input runs Tcl callbacks, and the whole tree is
//! rebuilt from the shadow model.

use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[test]
fn tcl_widgets_render_and_react_to_native_input() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<rtcl_tk_demo::DemoApp>()))
        .expect("headless engine starts");

    // Initial tree: label, entry, buttons, checkbutton, scale and listbox.
    app.wait_for("Hello from Tcl/Tk on GTK4")
        .expect("textvariable label renders");
    app.wait_for("World")
        .expect("entry with -textvariable renders");
    app.wait_for("I agree").expect("checkbutton renders");
    app.wait_for("Red").expect("listbox item renders");

    // Button -command changes both the button text and a -textvariable.
    app.press("Click me").expect("press the Tk button");
    app.wait_for("Clicked 1")
        .expect("Tcl -command changed the button text");
    app.wait_for("Clicks: 1")
        .expect("the label follows its -textvariable");
    app.press("Clicked 1").expect("press again");
    app.wait_for("Clicked 2")
        .expect("Tcl command runs on every press");

    // Entry edits write the Tcl variable back; the button reads it.
    app.type_text("text:World", "!")
        .expect("type into the Tk entry");
    app.press("Greet").expect("press Greet");
    app.wait_for("Hello, World!!")
        .expect("entry text reached Tcl");

    // Checkbutton: native press → -variable → -command → label.
    app.press("text:I agree").expect("toggle the checkbutton");
    app.wait_for("Agreed").expect("checkbutton command ran");
    app.press("text:I agree").expect("toggle it back");
    app.wait_for("Not agreed")
        .expect("offvalue reached the command");

    // Listbox selection is rendered from the shadow model.
    app.press("Red").expect("press a listbox row");
    app.wait_for("▶ ").expect("selected row marker rendered");

    // `after` timers are pumped by the renderer without `vwait`.
    app.wait_for("Later fired")
        .expect("after timer fired through the event pump");
}
