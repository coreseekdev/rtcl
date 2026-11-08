use nsh_core::element::Root;
use nsh_core::prelude::*;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[component]
fn Hello() -> impl IntoElement {
    text("hello")
}

#[test]
fn minimal_headless() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<Hello>())).expect("start");
    app.wait_for("hello").expect("hello renders");
}
