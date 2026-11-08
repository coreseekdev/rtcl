//! Headless M1b acceptance: Tcl builds the tree, nsh renders it, a native
//! press evaluates the Tk `-command`, and the re-render shows the new text.

use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[test]
fn button_press_runs_tcl_command_and_updates_view() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<rtcl_tk_demo::DemoApp>()))
        .expect("headless engine starts");
    app.wait_for("Click me").expect("initial Tk button renders");
    app.wait_for("Hello from Tcl/Tk on GTK4")
        .expect("initial Tk label renders");
    app.press("Click me").expect("press the Tk button");
    app.wait_for("Clicked 1")
        .expect("Tcl -command changed the button text");
    app.press("Clicked 1").expect("press again");
    app.wait_for("Clicked 2")
        .expect("Tcl command runs on every press");
}
