//! Event binding dispatch through the native backend: default Tk sequences
//! (`<Key>`, `<Return>`) and the `%` substitutions they receive.

use nsh_core::element::Root;
use nsh_core::geometry::{Points, Size};
use nsh_core::prelude::*;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

const APP: &str = r#"
set ::keys 0
set ::returns 0
set ::chars ""
set ::value x
set ::report "start"
entry .e -textvariable ::value
pack .e -fill x
label .l -textvariable ::report
pack .l
bind .e <Key> {
    incr ::keys
    append ::chars "%K"
    set ::report "keys=$::keys chars=$::chars"
}
bind .e <Return> {
    incr ::returns
    set ::report "ret=$::returns keys=$::keys"
}
focus .e
"#;

#[component]
fn BindApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn key_and_return_bindings_fire_once() {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<BindApp>())).expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(300.0)))
        .expect("resize");

    app.wait_for("start").expect("label renders");
    app.type_text("text:x", "a").expect("type into the entry");
    app.wait_for("keys=1 chars=a")
        .expect("<Key> fired with %K substitutions");
    app.press_key("b").expect("type a second key");
    app.wait_for("keys=2 chars=ab")
        .expect("<Key> fired for both keys");

    app.press_key("Return").expect("press Return");
    app.wait_for("ret=1 keys=2")
        .expect("Return binding runs once and <Key> does not shadow it");
}

const BUTTON_APP: &str = r#"
set ::n 0
set ::report "n=0"
button .b -text Go -command {incr ::n; set ::report "n=$::n"}
label .l -textvariable ::report
pack .b
pack .l
focus .b
"#;

#[component]
fn ButtonApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(BUTTON_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn button_space_activation() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<ButtonApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(300.0)))
        .expect("resize");
    app.wait_for("Go").expect("button renders");
    app.press("Go").expect("click the button");
    app.wait_for("n=1").expect("click invokes");
    app.press_key("space").expect("press space");
    // Tk: space on the focused button invokes it again.
    app.wait_for("n=2")
        .expect("space on the focused button invokes it");
}

const SPACE_ENTRY_APP: &str = r#"
set ::n 0
set ::value ""
set ::report "n=0"
button .b -text Go -command {incr ::n; set ::report "n=$::n"}
label .l -textvariable ::report
entry .e -textvariable ::value
pack .b
pack .l
pack .e
focus .e
"#;

#[component]
fn SpaceEntryApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(SPACE_ENTRY_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn space_in_entry_does_not_invoke_a_button() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<SpaceEntryApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(300.0)))
        .expect("resize");
    app.wait_for("Go").expect("button renders");
    app.press("Go").expect("click the button");
    app.wait_for("n=1").expect("click invokes");
    app.press_key("space").expect("space in the entry");
    let _ = app.settle();
    assert!(
        !app.shows("n=2"),
        "space belongs to the focused entry: {:?}",
        app.screen().texts()
    );
}

const SPACE_OVERRIDE_APP: &str = r#"
set ::n 0
set ::report "n=0"
button .b -text Go -command {incr ::n; set ::report "n=$::n"}
bind Button <Key-space> {set ::report custom}
label .l -textvariable ::report
pack .b
pack .l
focus .b
"#;

#[component]
fn SpaceOverrideApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(SPACE_OVERRIDE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn user_class_binding_replaces_default_space_invoke() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<SpaceOverrideApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(300.0)))
        .expect("resize");
    app.wait_for("Go").expect("button renders");
    app.press("Go").expect("click the button");
    app.wait_for("n=1").expect("click invokes");
    app.press_key("space").expect("press space");
    app.wait_for("custom")
        .expect("the user class binding replaced the default");
    assert!(
        !app.shows("n=2"),
        "the default invoke did not also run: {:?}",
        app.screen().texts()
    );
}

const NAV_APP: &str = r#"
listbox .l -height 3
pack .l
foreach item {L1 L2 L3} {.l insert end $item}
ttk::treeview .tv -height 3
pack .tv
foreach item {T1 T2 T3} {.tv insert {} end -id $item -text $item}
set ::report ""
label .r -textvariable ::report
pack .r
bind .tv <KeyRelease> {set ::report [.tv selection]}
"#;

#[component]
fn NavApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(NAV_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn arrow_keys_move_listbox_and_treeview_selection() {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<NavApp>())).expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(500.0)))
        .expect("resize");
    app.wait_for("L1").expect("listbox renders");

    let selected = |app: &Headless, item: &str| {
        let texts = app.screen().texts();
        texts
            .iter()
            .position(|text| *text == item)
            .and_then(|index| index.checked_sub(1))
            .is_some_and(|index| texts[index].trim() == "▶")
    };

    app.press("L1").expect("select the first row");
    app.wait_for("▶").expect("a row is selected");
    assert!(selected(&app, "L1"), "the clicked row is selected");
    app.press_key("down")
        .expect("move the listbox selection down");
    app.wait_for("L2").expect("the next row renders");
    assert!(selected(&app, "L2"), "Down selects the next listbox row");

    app.press("T1").expect("select the first tree row");
    app.press_key("down")
        .expect("move the treeview selection down");
    app.wait_for("T2").expect("the treeview reports T2");
}

const FOCUS_DESYNC_APP: &str = r#"
set ::n 0
set ::value x
set ::report "n=0"
button .b -text Go -command {incr ::n; set ::report "n=$::n"}
label .l -textvariable ::report
entry .e -textvariable ::value
pack .b
pack .l
pack .e
focus .b
"#;

#[component]
fn FocusDesyncApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(FOCUS_DESYNC_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn typing_in_an_entry_does_not_trigger_a_focused_button() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<FocusDesyncApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(300.0)))
        .expect("resize");
    app.wait_for("Go").expect("button renders");
    // The script focused the button, then the user clicks into the entry.
    app.type_text("text:x", "").expect("focus the entry");
    app.press_key("space").expect("space in the entry");
    let _ = app.settle();
    assert!(
        !app.shows("n=1"),
        "space went to the entry, not the Tk-focused button: {:?}",
        app.screen().texts()
    );
}

const WHEEL_APP: &str = r#"
listbox .l -height 3
pack .l
foreach item {I1 I2 I3 I4 I5 I6 I7 I8 I9 I10} {.l insert end $item}
ttk::treeview .tv -height 3
pack .tv
foreach item {T1 T2 T3 T4 T5 T6 T7 T8 T9 T10} {.tv insert {} end -id $item -text $item}
"#;

#[component]
fn WheelApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(WHEEL_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn wheel_scrolls_listbox_and_treeview() {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<WheelApp>())).expect("headless engine starts");
    app.set_viewport(Size::new(Points(600.0), Points(600.0)))
        .expect("resize");
    app.wait_for("I1").expect("listbox renders");
    app.wait_for("T1").expect("treeview renders");

    app.scroll("I1", Points(100.0))
        .expect("wheel over the listbox");
    app.wait_for("I5")
        .expect("the listbox slice follows the wheel");
    assert!(
        !app.shows("I1"),
        "the first row scrolled out: {:?}",
        app.screen().texts()
    );

    app.scroll("T1", Points(100.0))
        .expect("wheel over the treeview");
    app.wait_for("T6")
        .expect("the treeview slice follows the wheel");
    assert!(
        !app.shows("T1"),
        "the first tree row scrolled out: {:?}",
        app.screen().texts()
    );
}

const WIDGET_BIND_APP: &str = r#"
set ::report ""
label .lab -text ClickMe
pack .lab
label .out -textvariable ::report
pack .out
bind .lab <Button-1> {set ::report clicked}
"#;

#[component]
fn WidgetBindApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(WIDGET_BIND_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn label_click_bindings_fire() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<WidgetBindApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    app.wait_for("ClickMe").expect("label renders");
    app.press("ClickMe").expect("click the label");
    app.wait_for("clicked")
        .expect("<Button-1> on a plain label runs");
}

const PASTE_APP: &str = r#"
set ::value x
set ::report ""
entry .e -textvariable ::value
pack .e
label .out -textvariable ::report
pack .out
bind .e <<Paste>> {set ::report pasted}
"#;

#[component]
fn PasteApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(PASTE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn paste_fires_virtual_event() {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<PasteApp>())).expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    app.wait_for("x").expect("entry renders");
    let mut paste = PasteEvent::default();
    paste.text = "hi".to_string();
    app.paste("text:x", &paste).expect("paste");
    let _ = app.settle();
    app.wait_for("pasted").expect("<<Paste>> binding fired");
}

const ENTRY_POINTER_APP: &str = r#"
set ::value x
set ::report ""
entry .e -textvariable ::value
pack .e
label .out -textvariable ::report
pack .out
bind .e <Button-1> {set ::report click}
bind .e <FocusIn> {append ::report " in"}
"#;

#[component]
fn EntryPointerApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(ENTRY_POINTER_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn entry_pointer_and_focus_bindings() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<EntryPointerApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    app.wait_for("x").expect("entry renders");
    app.press("text:x").expect("click into the entry");
    app.wait_for("click in")
        .expect("<Button-1> and <FocusIn> fired for the entry");
}

const RICH_KEY_APP: &str = r##"
set ::report ""
set ::wheel ""
text .t -height 3
.t insert end "hi"
.t tag configure hot -foreground "#ef4444"
.t tag add hot 1.0 1.2
pack .t
label .out -textvariable ::report
pack .out
label .wo -textvariable ::wheel
pack .wo
bind .t <Key> {append ::report "key=%A"}
bind .t <MouseWheel> {set ::wheel %D}
bind .t <Motion> {append ::report M}
"##;

#[component]
fn RichKeyApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(RICH_KEY_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn rich_text_key_bindings_fire() {
    let mut config = EngineConfig::new(Root::of::<RichKeyApp>());
    config.environment.os = nsh_core::platform::Os::Linux;
    let mut app = Headless::start(config).expect("headless engine starts");
    app.set_viewport(Size::new(Points(500.0), Points(300.0)))
        .expect("resize");
    app.wait_for("hi").expect("rich editor renders");
    app.press("hi").expect("focus the editor");
    app.press_key("x").expect("type in the editor");
    app.wait_for("key=x")
        .expect("the text widget's <Key> binding fired");
    assert!(
        app.screen().texts().iter().any(|text| *text == "key=x"),
        "the binding ran once: {:?}",
        app.screen().texts()
    );

    app.wheel("hxi", Points(0.0), Points(100.0))
        .expect("wheel over the rich editor");
    app.wait_for("-120")
        .expect("the text widget's <MouseWheel> binding fired");

    app.hover("hxi").expect("hover the rich editor");
    app.wait_for("key=xM")
        .expect("the text widget's <Motion> binding fired");
}

const RICH_UNDO_APP: &str = r##"
text .t -height 3 -undo 1
.t insert end "hi"
.t tag configure hot -foreground "#ef4444"
.t tag add hot 1.0 1.2
.t edit reset
pack .t
button .undo -text "undo" -command {.t edit undo}
pack .undo
"##;

#[component]
fn RichUndoApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(RICH_UNDO_APP);
    rtcl_tk_nsh::tk_view()
}

const TAG_BIND_APP: &str = r##"
set ::hit 0
set ::enter 0
set ::release 0
text .t -height 3
.t insert end "hi"
.t tag configure hot -foreground "#ef4444"
.t tag add hot 1.0 1.2
.t tag bind hot <Button-1> {set ::hit clicked}
.t tag bind hot <Enter> {set ::enter entered}
.t tag bind hot <ButtonRelease-1> {set ::release released}
pack .t
label .out -textvariable ::hit
pack .out
label .en -textvariable ::enter
pack .en
label .rel -textvariable ::release
pack .rel
"##;

#[component]
fn TagBindApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(TAG_BIND_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn rich_text_tag_bindings_fire_on_press() {
    let mut config = EngineConfig::new(Root::of::<TagBindApp>());
    config.environment.os = nsh_core::platform::Os::Linux;
    let mut app = Headless::start(config).expect("headless engine starts");
    app.set_viewport(Size::new(Points(500.0), Points(300.0)))
        .expect("resize");
    app.wait_for("hi").expect("rich editor renders");
    app.press("hi").expect("press the tagged text");
    app.wait_for("clicked")
        .expect("the tag's <Button-1> binding fired");
    app.wait_for("released")
        .expect("the tag's <ButtonRelease-1> binding fired");
    app.hover("hi").expect("hover the tagged text");
    app.wait_for("entered")
        .expect("the tag's <Enter> binding fired");
}

#[test]
fn rich_edits_enter_the_text_undo_stack() {
    let mut config = EngineConfig::new(Root::of::<RichUndoApp>());
    config.environment.os = nsh_core::platform::Os::Linux;
    let mut app = Headless::start(config).expect("headless engine starts");
    app.set_viewport(Size::new(Points(500.0), Points(300.0)))
        .expect("resize");
    app.wait_for("hi").expect("rich editor renders");
    app.press("hi").expect("focus the editor");
    app.press_key("x").expect("type in the editor");
    app.wait_for("hxi").expect("the edit reaches the model");
    app.press("undo").expect("run .t edit undo");
    app.wait_for("hi").expect("undo restores the previous text");
    assert!(
        app.screen().texts().iter().any(|text| *text == "hi"),
        "undo restored the text: {:?}",
        app.screen().texts()
    );
}

const CONFIGURE_APP: &str = r#"
set ::report ""
frame .f
pack .f -fill both -expand 1
label .out -textvariable ::report
pack .out
bind .f <Configure> {if {%w > 500} {set ::report big}}
"#;

#[component]
fn ConfigureApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(CONFIGURE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn configure_fires_on_resize() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<ConfigureApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    let _ = app.settle();
    app.set_viewport(Size::new(Points(700.0), Points(400.0)))
        .expect("grow");
    app.wait_for("big")
        .expect("<Configure> reported the new width");
}

const SCALE_APP: &str = r#"
set ::report ""
scale .s -from 0 -to 100 -variable ::v
pack .s -fill x -padx 20 -pady 20
label .out -textvariable ::report
pack .out
bind .s <Button-1> {set ::report pressed}
"#;

#[component]
fn ScaleApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(SCALE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn scale_press_binding_fires() {
    let mut app =
        Headless::start(EngineConfig::new(Root::of::<ScaleApp>())).expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    let _ = app.settle();
    app.press("role:adjustable").expect("press the scale");
    app.wait_for("pressed")
        .expect("<Button-1> fired for the scale");
}

const BUTTON_CONFIGURE_APP: &str = r#"
set ::report ""
button .b -text Hi
pack .b -fill x
label .out -textvariable ::report
pack .out
bind .b <Configure> {if {%w > 500} {set ::report big}}
"#;

#[component]
fn ButtonConfigureApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(BUTTON_CONFIGURE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn button_configure_fires_on_resize() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<ButtonConfigureApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    let _ = app.settle();
    app.set_viewport(Size::new(Points(700.0), Points(400.0)))
        .expect("grow");
    app.wait_for("big")
        .expect("the button's own <Configure> reported the filled width");
}

const NB_CB_CONFIGURE_APP: &str = r#"
set ::nb ""
set ::cb ""
ttk::notebook .nb
frame .nb.a -text A
.nb add .nb.a
pack .nb -fill x
ttk::combobox .cb -values {A B}
pack .cb -fill x
label .nbo -textvariable ::nb
pack .nbo
label .cbo -textvariable ::cb
pack .cbo
bind .nb <Configure> {set ::nb nbcfg}
bind .cb <Configure> {set ::cb cbcfg}
"#;

#[component]
fn NbCbConfigureApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(NB_CB_CONFIGURE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn notebook_and_combobox_configure_fire() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<NbCbConfigureApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(500.0), Points(400.0)))
        .expect("resize");
    app.wait_for("A").expect("notebook/combobox render");
    let _ = app.settle();
    app.wait_for("nbcfg")
        .expect("the notebook reported <Configure>");
    app.wait_for("cbcfg")
        .expect("the combobox reported <Configure>");
}

const WHEEL_BIND_APP: &str = r#"
set ::value x
set ::wheel ""
set ::button ""
entry .e -textvariable ::value
pack .e
label .wo -textvariable ::wheel
pack .wo
label .bo -textvariable ::button
pack .bo
bind .e <MouseWheel> {set ::wheel %D}
bind .e <Button-5> {set ::button down}
"#;

#[component]
fn WheelBindApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(WHEEL_BIND_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn wheel_bindings_fire_on_entry() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<WheelBindApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(500.0), Points(300.0)))
        .expect("resize");
    app.wait_for("x").expect("entry renders");
    app.wheel("text:x", Points(0.0), Points(100.0))
        .expect("wheel over the entry");
    app.wait_for("-120")
        .expect("<MouseWheel> got Tk's %D for a down notch");
    app.wait_for("down")
        .expect("<Button-5> fired for the wheel");
}

const MENU_CONFIGURE_APP: &str = r#"
set ::report ""
menu .m -tearoff 0
menu .file -tearoff 0
.file add command -label Reset -command {}
.m add cascade -label File -menu .file
. configure -menu .m
label .out -textvariable ::report
pack .out
bind .file <Configure> {set ::report cfg}
"#;

#[component]
fn MenuConfigureApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(MENU_CONFIGURE_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn menu_dropdown_configure_fires() {
    let mut app = Headless::start(EngineConfig::new(Root::of::<MenuConfigureApp>()))
        .expect("headless engine starts");
    app.set_viewport(Size::new(Points(500.0), Points(400.0)))
        .expect("resize");
    app.wait_for("File").expect("menubar renders");
    app.press("File").expect("open the File menu");
    app.wait_for("cfg")
        .expect("the menu window's <Configure> fired");
}

const HOVER_APP: &str = r#"
set ::log ""
label .a -text HoverMe
pack .a
label .b -text Other
pack .b
label .out -textvariable ::log
pack .out
bind .a <Enter> {append ::log E}
bind .a <Leave> {append ::log L}
bind .a <Motion> {append ::log M}
"#;

#[component]
fn HoverApp() -> impl IntoElement {
    rtcl_tk_demo::ensure_tcl_script(HOVER_APP);
    rtcl_tk_nsh::tk_view()
}

#[test]
fn hover_and_motion_bindings_fire() {
    let mut config = EngineConfig::new(Root::of::<HoverApp>());
    config.environment.os = nsh_core::platform::Os::Linux;
    let mut app = Headless::start(config).expect("headless engine starts");
    app.set_viewport(Size::new(Points(400.0), Points(300.0)))
        .expect("resize");
    app.wait_for("HoverMe").expect("label renders");
    app.hover("HoverMe").expect("hover the label");
    // A render after the Enter handler can replace the node the first
    // queued Motion targeted; a second move (as a real pointer sends) reaches
    // the new one.
    app.hover("HoverMe").expect("move over the label again");
    app.wait_for("EM")
        .expect("<Enter> and <Motion> fired on hover");
    app.hover("Other").expect("hover the other label");
    app.wait_for("EML")
        .expect("<Leave> fired when the pointer left");
}
