//! The classic grid/pack form renders and its Tk callbacks run through the
//! native backend: entry `-textvariable`, button `-command`, `<Return>`
//! binding, listbox and ttk widgets.

use nsh_core::element::Root;
use nsh_core::geometry::{Points, Size};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[test]
fn classic_form_round_trip() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<rtcl_tk_demo::ClassicApp>()))
        .expect("headless engine starts");
    // The form is taller than a phone: give the dropdown room to show.
    app.set_viewport(Size::new(Points(900.0), Points(1400.0)))
        .expect("resize the headless viewport");

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

    // Notebook pages switch from the tab bar.
    app.wait_for("General page").expect("first page renders");
    app.press("Advanced").expect("select the Advanced tab");
    app.wait_for("Advanced page")
        .expect("the notebook shows the selected page");

    // Combobox: value list drops down and writes the variable.
    app.wait_for("Medium")
        .expect("combobox shows -textvariable");
    app.press("▾").expect("open the combobox list");
    app.wait_for("Large").expect("value list renders");
    app.press("Large").expect("pick a value");
    app.wait_for("Large")
        .expect("the picked value is written to the entry");
    app.press("▾").expect("open the combobox list again");
    app.wait_for("Small").expect("the value list renders again");
    app.press("Small").expect("pick another value");
    app.wait_for("Small")
        .expect("the second picked value is in the entry");
}
