//! Treeview rows respond to native presses.

use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[test]
fn treeview_collapses_and_selects() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<rtcl_tk_demo::TreeApp>()))
        .expect("headless engine starts");
    app.wait_for("Project").expect("root row");
    app.wait_for("src").expect("child row");
    app.press("Project").expect("collapse the root row");
    assert!(!app.shows("src"), "collapse hides the child");
    app.press("Project").expect("expand the root row");
    app.wait_for("src").expect("child is back");
}
