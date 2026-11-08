//! The classic grid/pack form renders and its Tk callbacks run through the
//! native backend: entry `-textvariable`, button `-command`, `<Return>`
//! binding, listbox and ttk widgets.

use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[test]
fn classic_form_round_trip() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<rtcl_tk_demo::ClassicApp>()))
        .expect("headless engine starts");

    app.wait_for("Name:").expect("label renders");
    app.wait_for("World").expect("entry shows -textvariable");
    app.wait_for("Red").expect("listbox renders");
    app.wait_for("OK").expect("button renders");

    // Editing the entry writes ::name back and focuses it; Return then
    // runs the bind script, which invokes OK.
    app.type_text("text:World", "!")
        .expect("type into the entry");
    app.press_key("Return").expect("press Return in the entry");
    app.wait_for("Hello, World!")
        .expect("bind <Return> invoked the button with the entry value");

    app.press("OK").expect("press OK");
    app.wait_for("Hello, World!")
        .expect("the button command reads the same variable");
}
