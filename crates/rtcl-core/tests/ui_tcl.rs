//! ui.tcl 词表层的原生集成测试：`_ui` 桥命令换成记录器，
//! 校验词表命令产生的 op JSON、参数校验、批次与事件回路。
//! 契约见 docs/UI-DSL.md。

use rtcl_core::{Interp, Value};

/// `_ui` 记录器：把每次过桥的一批 op（一个 JSON 数组字符串）追加到
/// 全局变量 `ui_test_log`，一批一行。
fn ui_recorder(interp: &mut Interp, args: &[Value]) -> rtcl_core::error::Result<Value> {
    let cur = interp
        .get_var("::ui_test_log")
        .ok()
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    let new = if cur.is_empty() {
        args[1].as_str().to_string()
    } else {
        format!("{cur}\n{}", args[1].as_str())
    };
    interp.set_var("::ui_test_log", Value::from_str(&new))?;
    Ok(Value::empty())
}

fn setup() -> Interp {
    let mut interp = Interp::new();
    interp.register_command("_ui", ui_recorder);
    interp.load_ui().expect("load_ui");
    interp.eval("set ::ui_test_log {}").unwrap();
    interp
}

/// 已发出的 op 批次（每行一个 JSON 数组）。
fn op_log(interp: &mut Interp) -> String {
    interp
        .eval("set ::ui_test_log")
        .unwrap()
        .as_str()
        .to_string()
}

#[test]
fn create_surface_and_button_emit_create_ops() {
    let mut interp = setup();
    interp.eval(r#"ui::surface .s -title Counter"#).unwrap();
    interp.eval(r#"ui::column .s.main"#).unwrap();
    interp.eval(r#"ui::button .s.main.inc -text +1"#).unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#"[{"op":"create","path":".s","type":"Surface","props":{"title":"Counter"},"events":[]}]"#),
        "surface op: {log}"
    );
    assert!(
        log.contains(r#"{"op":"create","path":".s.main.inc","type":"Button","props":{"text":"+1"},"events":[]}"#),
        "button op: {log}"
    );
}

#[test]
fn validation_errors_are_tcl_errors() {
    let mut interp = setup();
    interp.eval(r#"ui::surface .s"#).unwrap();

    // 未知选项
    let e = interp.eval(r#"ui::button .s.b -nope 1"#).unwrap_err();
    assert!(e.to_string().contains("unknown option \"-nope\""), "{e}");

    // enum 越界
    let e = interp.eval(r#"ui::button .s.b -variant danger"#).unwrap_err();
    assert!(e.to_string().contains("must be one of"), "{e}");

    // 父路径不存在
    let e = interp.eval(r#"ui::button .nope.b"#).unwrap_err();
    assert!(e.to_string().contains("parent \".nope\" does not exist"), "{e}");

    // 路径非法
    let e = interp.eval(r#"ui::button s"#).unwrap_err();
    assert!(e.to_string().contains("bad widget path"), "{e}");

    // 重复路径
    interp.eval(r#"ui::text .s.t -text a"#).unwrap();
    let e = interp.eval(r#"ui::text .s.t -text b"#).unwrap_err();
    assert!(e.to_string().contains("already exists"), "{e}");

    // 三次合法命令产生且只产生三批 op（surface/text/button 失败的不算）
    let log = op_log(&mut interp);
    assert_eq!(log.lines().count(), 2, "errors must not emit ops: {log}");
}

#[test]
fn batch_accumulates_into_one_crossing_and_discards_on_error() {
    let mut interp = setup();
    interp
        .eval(r#"ui::batch { ui::surface .s -title T; ui::text .s.a -text A; ui::text .s.b -text B }"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert_eq!(log.lines().count(), 1, "batch = one crossing: {log}");
    assert!(log.contains(r#""path":".s.a"#) && log.contains(r#""path":".s.b"#), "{log}");

    // 错误：半个批次不过桥，且批内已登记的 widget 要回滚
    interp.eval("set ::ui_test_log {}").unwrap();
    let e = interp
        .eval(r#"ui::batch { ui::text .s.x -text X; ui::button .s.y -nope 1 }"#)
        .unwrap_err();
    assert!(e.to_string().contains("unknown option"), "{e}");
    assert_eq!(op_log(&mut interp), "", "failed batch must not emit");
    // .s.x 在批内登记过，作废后必须回到不存在
    let e = interp.eval(r#"ui::configure .s.x -text Y"#).unwrap_err();
    assert!(e.to_string().contains("unknown widget"), "{e}");
}

#[test]
fn command_handler_stays_tcl_side_and_fires_via_events() {
    let mut interp = setup();
    interp
        .eval(r#"
            ui::surface .s
            ui::text .s.count -text 0
            ui::button .s.inc -text +1 -command {ui::configure .s.count -text [incr ::n]}
        "#)
        .unwrap();
    // create op 里带 events:["click"]，但脚本不过桥
    let log = op_log(&mut interp);
    assert!(log.contains(r#""events":["click"]"#), "{log}");
    assert!(!log.contains("incr"), "callback script must not cross the bridge: {log}");

    // JS 侧点击 → dispatch_event 回灌 → 回调执行 → configure op 发出
    interp.post_event("::ui::fire .s.inc click");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(interp.eval("set ::n").unwrap().as_str(), "1");
    let log = op_log(&mut interp);
    assert!(log.contains(r#"{"op":"set","path":".s.count","props":{"text":"1"}"#), "{log}");
}

#[test]
fn model_domain_roundtrip() {
    let mut interp = setup();
    interp.eval(r#"ui::surface .s"#).unwrap();
    interp.eval(r#"ui::model set /pods {[{"name":"a"}]}"#).unwrap();
    interp.eval(r#"ui::model set /title Hello"#).unwrap();

    // 模型在 Tcl 侧可读
    assert_eq!(interp.eval("ui::model get /title").unwrap().as_str(), "Hello");

    // JSON 文本原样过线，纯文本按 string 编码（json encode 把 / 转义为 \/）
    let log = op_log(&mut interp);
    assert!(log.contains(r#""path":"\/pods","value":[{"name":"a"}]"#), "{log}");
    assert!(log.contains(r#""path":"\/title","value":"Hello""#), "{log}");

    // unset
    interp.eval(r#"ui::model unset /title"#).unwrap();
    let e = interp.eval(r#"ui::model get /title"#).unwrap_err();
    assert!(e.to_string().contains("no such path"), "{e}");
    let log = op_log(&mut interp);
    assert!(log.contains(r#"{"op":"model","sub":"unset","path":"\/title"}"#), "{log}");

    // 裸标量字面量原样过线（rtcl json decode 根须 object/array，见 ui.tcl 注释）
    interp.eval(r#"ui::model set /flag false"#).unwrap();
    interp.eval(r#"ui::model set /count 42"#).unwrap();
    let log = op_log(&mut interp);
    assert!(log.contains(r#""path":"\/flag","value":false"#), "{log}");
    assert!(log.contains(r#""path":"\/count","value":42"#), "{log}");
}

#[test]
fn configure_and_destroy() {
    let mut interp = setup();
    interp.eval(r#"ui::surface .s; ui::text .s.t -text A"#).unwrap();
    interp.eval(r#"ui::configure .s.t -text B"#).unwrap();
    let log = op_log(&mut interp);
    assert!(log.contains(r#"{"op":"set","path":".s.t","props":{"text":"B"}"#), "{log}");

    // destroy 清理子孙注册表
    interp.eval(r#"ui::text .s.t.sub -text C"#).unwrap(); // 父 .s.t 存在即可
    interp.eval(r#"ui::destroy .s.t"#).unwrap();
    let e = interp.eval(r#"ui::configure .s.t -text C"#).unwrap_err();
    assert!(e.to_string().contains("unknown widget"), "{e}");
    // 子孙一并注销
    let e = interp.eval(r#"ui::configure .s.t.sub -text C"#).unwrap_err();
    assert!(e.to_string().contains("unknown widget"), "{e}");
}

#[test]
fn catalog_is_introspectable() {
    let mut interp = setup();
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Button") && all.contains("-command"), "{all}");
    let btn = interp.eval("ui::catalog Button").unwrap().as_str().to_string();
    assert!(btn.contains("enum"), "{btn}");
    let e = interp.eval("ui::catalog Nope").unwrap_err();
    assert!(e.to_string().contains("unknown component type"), "{e}");
}
