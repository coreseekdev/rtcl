//! `wm attributes -alpha` reaches the renderer's opacity channel: the Tcl
//! value flows rtcl → nsh's `DesktopCommand::SetOpacity`, which the headless
//! screen keeps (the GTK backend applies the same command to its window,
//! `gtk widget set_opacity`, whose visual effect depends on the platform
//! compositor).  Wish itself only reads back the endpoints reliably on X11
//! without a compositor — intermediate values round to 1.0 there — so the
//! exact value comparison runs against nsh's recorded channel, not wish.
//!
//! The alpha is applied through `-command`s, not the initial script: the
//! window-attribute hook installs with the backend (after the script's
//! first eval), so attributes set while it evaluates reach no renderer —
//! the same holds for `-fullscreen`/`-topmost`.

use nsh_core::element::Root;
use nsh_core::prelude::*;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

const APP: &str = r#"
set ::report ""
label .l -textvariable ::report
pack .l
button .half -text "half" -command {wm attributes . -alpha 0.5; set ::report "a=[wm attributes . -alpha]"}
pack .half
button .clear -text "clear" -command {wm attributes . -alpha 1.0; set ::report "a=[wm attributes . -alpha]"}
pack .clear
button .ghost -text "ghost" -command {wm attributes . -alpha 0.0; set ::report "a=[wm attributes . -alpha]"}
pack .ghost
"#;

#[component]
fn AlphaApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn window_alpha_reaches_the_opacity_channel() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<AlphaApp>()))
        .expect("headless engine starts");
    app.wait_for("half").expect("buttons render");
    assert_eq!(app.screen().window_opacity(), 1.0, "the window starts opaque");

    // 0.5 flows to the renderer and the model read-back agrees.
    app.press("text:half").expect("apply 0.5");
    app.wait_for("a=0.5").expect("the model read the alpha back");
    assert_eq!(app.screen().window_opacity(), 0.5);

    // 1.0 restores full opacity, 0.0 makes the window invisible.
    app.press("text:clear").expect("restore 1.0");
    app.wait_for("a=1.0").expect("the model read 1.0 back");
    assert_eq!(app.screen().window_opacity(), 1.0);

    app.press("text:ghost").expect("apply 0.0");
    app.wait_for("a=0.0").expect("the model read 0.0 back");
    assert_eq!(app.screen().window_opacity(), 0.0);
}
