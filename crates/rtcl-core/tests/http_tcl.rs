//! http.tcl 词表层的原生集成测试：`__rtcl_http` 桥换成记录器，
//! 校验请求参数过桥、完成回灌（::http::_done）的回调/变量/复位、
//! 参数与桥错误、JSON Lines 助手。契约见 docs/ROADMAP.md §3。

use rtcl_core::{Interp, Value};

/// `__rtcl_http id method url body opts` 记录器：一行一次调用，
/// 字段用 `|` 连接存入 `::http_test_log`。
fn bridge_recorder(interp: &mut Interp, args: &[Value]) -> rtcl_core::error::Result<Value> {
    let cur = interp
        .get_var("::http_test_log")
        .ok()
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    let line = args[1..]
        .iter()
        .map(|a| a.as_str().to_string())
        .collect::<Vec<_>>()
        .join("|");
    let new = if cur.is_empty() { line } else { format!("{cur}\n{line}") };
    interp.set_var("::http_test_log", Value::from_str(&new))?;
    Ok(Value::empty())
}

fn setup() -> Interp {
    let mut interp = Interp::new();
    interp.register_command("__rtcl_http", bridge_recorder);
    interp.load_http().expect("load_http");
    interp.eval("set ::http_test_log {}").unwrap();
    interp
}

fn log(interp: &mut Interp) -> String {
    interp
        .eval("set ::http_test_log")
        .unwrap()
        .as_str()
        .to_string()
}

#[test]
fn get_request_reaches_bridge_and_returns_id() {
    let mut interp = setup();
    let id = interp
        .eval(r#"http::get "http://127.0.0.1:9/lines" -command {set ::got 1}"#)
        .unwrap();
    assert_eq!(id.as_str(), "1");
    assert_eq!(
        log(&mut interp),
        "1|GET|http://127.0.0.1:9/lines||{\"timeout\":0,\"ctype\":\"application\\/json\"}"
    );
    // 脚本没被阻塞：回调尚未跑（完成还没回灌）
    assert_eq!(interp.eval("info exists ::got").unwrap().as_str(), "0");
}

#[test]
fn done_runs_callback_with_http_vars_then_resets() {
    let mut interp = setup();
    interp
        .eval(r#"http::get http://x -command {set ::r [list $http_status $http_error $http_body]}"#)
        .unwrap();
    interp
        .eval(r#"::http::_done 1 200 {} {hello world}"#)
        .unwrap();
    assert_eq!(interp.eval("set ::r").unwrap().as_str(), "200 {} {hello world}");
    // 变量跑完复位（$ui_row 同款）
    for v in ["::http_status", "::http_error", "::http_body"] {
        assert_eq!(interp.eval(&format!("info exists {v}")).unwrap().as_str(), "0", "{v} must reset");
    }
    // 迟到/未知 id 静默
    interp.eval(r#"::http::_done 99 200 {} x"#).unwrap();
}

#[test]
fn post_carries_body_and_content_type() {
    let mut interp = setup();
    interp
        .eval(r#"http::post http://x/echo {{"a":1}} -content-type text/plain -command {set ::got 1}"#)
        .unwrap();
    assert_eq!(
        log(&mut interp),
        "1|POST|http://x/echo|{\"a\":1}|{\"timeout\":0,\"ctype\":\"text\\/plain\"}"
    );
}

#[test]
fn argument_errors_are_tcl_errors() {
    let mut interp = setup();
    let e = interp.eval("http::get http://x").unwrap_err();
    assert!(e.to_string().contains("-command is required"), "{e}");
    let e = interp.eval("http::get http://x -command {} -oops 1").unwrap_err();
    assert!(e.to_string().contains("unknown option"), "{e}");
    let e = interp.eval("http::get http://x -command").unwrap_err();
    assert!(e.to_string().contains("-name value pairs"), "{e}");
    let e = interp.eval("http::get {} -command {}").unwrap_err();
    assert!(e.to_string().contains("URL must not be empty"), "{e}");

    // 桥未注册 → 清晰错误（wasm 提示）
    let mut bare = Interp::new();
    bare.load_http().unwrap();
    let e = bare.eval("http::get http://x -command {puts x}").unwrap_err();
    assert!(e.to_string().contains("__rtcl_http is not registered"), "{e}");

    // 桥抛错 → 包装错误，calls 表不留悬挂（重试仍能用同 id 序列）
    let mut failing = Interp::new();
    failing.register_command("__rtcl_http", |_i, _a| {
        Err(rtcl_core::error::Error::runtime("no route", rtcl_core::error::ErrorCode::Generic))
    });
    failing.load_http().unwrap();
    let e = failing.eval("http::get http://x -command {puts x}").unwrap_err();
    assert!(e.to_string().contains("host bridge failed"), "{e}");
}

#[test]
fn callback_error_propagates_after_state_reset() {
    let mut interp = setup();
    interp
        .eval(r#"http::get http://x -command {error "boom ${::http_status}"}"#)
        .unwrap();
    let e = interp.eval("::http::_done 1 500 oops body").unwrap_err();
    assert!(e.to_string().contains("boom 500"), "{e}");
    assert_eq!(interp.eval("info exists ::http_body").unwrap().as_str(), "0");
}

#[test]
fn jsonl_splits_nonempty_trimmed_lines() {
    let mut interp = setup();
    interp
        .eval(r#"set lines [http::jsonl "{\"a\":1}\n\n  {\"b\":2}  \n"]"#)
        .unwrap();
    assert_eq!(interp.eval("llength $lines").unwrap().as_str(), "2");
    assert_eq!(interp.eval("lindex $lines 0").unwrap().as_str(), r#"{"a":1}"#);
    assert_eq!(interp.eval("lindex $lines 1").unwrap().as_str(), r#"{"b":2}"#);
}
