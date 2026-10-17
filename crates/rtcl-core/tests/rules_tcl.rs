//! rules.tcl 词表层的原生集成测试：after 轮询驱动、false→true 沿
//! 触发与去抖、错误计数与循环存活、stop/start/remove/list。契约见
//! docs/ROADMAP.md §3。

use rtcl_core::Interp;
use std::time::Duration;

fn setup() -> Interp {
    let mut interp = Interp::new();
    interp.load_rules().expect("load_rules");
    interp
}

/// 睡过定时器再泵一拍（与 wasm 心跳同语义：一次泵冲出到点事件，
/// _tick 续期的新事件在本次泵之后）。
fn tick(interp: &mut Interp, ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
    let errs = interp.pump_events();
    assert!(errs.is_empty(), "pump errors: {errs:?}");
}

#[test]
fn add_validates_options() {
    let mut interp = setup();
    let e = interp.eval("rules::add r -every 0 -do {set x 1}").unwrap_err();
    assert!(e.to_string().contains("positive integer"), "{e}");
    let e = interp.eval("rules::add r -when {1}").unwrap_err();
    assert!(e.to_string().contains("-do"), "{e}");
    let e = interp.eval("rules::add r -every 1000 -oops 1 -do {set x 1}").unwrap_err();
    assert!(e.to_string().contains("unknown option"), "{e}");
    interp.eval("rules::add r -every 1000 -do {set x 1}").unwrap();
    let e = interp.eval("rules::add r -every 1000 -do {set x 1}").unwrap_err();
    assert!(e.to_string().contains("already exists"), "{e}");
    interp.eval("rules::remove r").unwrap();
}

#[test]
fn edge_trigger_fires_once_per_transition() {
    let mut interp = setup();
    interp.eval("set ::level 10; set ::trips 0").unwrap();
    interp
        .eval(r#"rules::add alarm -every 1 -when {expr {$::level > 80}} -do {incr ::trips}"#)
        .unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires alarm").unwrap().as_str(), "0");
    interp.eval("set ::level 90").unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires alarm").unwrap().as_str(), "1");
    // 保持为真：沿已消耗，不再触发（去抖）
    tick(&mut interp, 10);
    tick(&mut interp, 10);
    assert_eq!(interp.eval("set ::trips").unwrap().as_str(), "1");
    // 回落重新武装，再掀第二次沿
    interp.eval("set ::level 20").unwrap();
    tick(&mut interp, 10);
    interp.eval("set ::level 99").unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires alarm").unwrap().as_str(), "2");
    interp.eval("rules::remove alarm").unwrap();
}

#[test]
fn errors_count_and_loop_survives() {
    let mut interp = setup();
    interp
        .eval(r#"rules::add bad -every 1 -when {error "boom"} -do {set x 1}"#)
        .unwrap();
    interp
        .eval(r#"rules::add good -every 1 -when {expr {$::flag eq "on"}} -do {incr ::hits}"#)
        .unwrap();
    interp.eval("set ::flag off; set ::hits 0").unwrap();
    tick(&mut interp, 10);
    let errs: u32 = interp.eval("rules::errors").unwrap().as_str().parse().unwrap();
    assert!(errs >= 1, "expected condition errors, got {errs}");
    // 坏规则不杀好规则
    interp.eval("set ::flag on").unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("set ::hits").unwrap().as_str(), "1");
    interp.eval("rules::remove bad; rules::remove good").unwrap();
}

#[test]
fn action_error_consumes_the_edge_and_counts() {
    let mut interp = setup();
    interp.eval("set ::go 0").unwrap();
    interp
        .eval(r#"rules::add a -every 1 -when {expr {$::go}} -do {error "action fail"}"#)
        .unwrap();
    interp.eval("set ::go 1").unwrap();
    tick(&mut interp, 10);
    let errs: u32 = interp.eval("rules::errors").unwrap().as_str().parse().unwrap();
    assert!(errs >= 1, "expected action error, got {errs}");
    assert_eq!(interp.eval("rules::fires a").unwrap().as_str(), "0", "failed action does not count as fire");
    // 消耗沿：仍为真不再重试；回落-再掀才再来一次
    tick(&mut interp, 10);
    let errs2: u32 = interp.eval("rules::errors").unwrap().as_str().parse().unwrap();
    assert_eq!(errs2, errs, "edge consumed, no retry storm");
    interp.eval("rules::remove a").unwrap();
}

#[test]
fn stop_start_remove_and_list() {
    let mut interp = setup();
    interp.eval("set ::c 1; set ::n 0").unwrap();
    interp
        .eval(r#"rules::add r -every 1 -when {expr {$::c}} -do {incr ::n}"#)
        .unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires r").unwrap().as_str(), "1");
    // 列表内省：{名称 active armed fires errors}
    let l = interp.eval("rules::list").unwrap().as_str().to_string();
    assert_eq!(l, "r 1 0 1 0", "{l}");
    // stop：续期链断，不再涨
    interp.eval("rules::stop r").unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires r").unwrap().as_str(), "1");
    // start：不重置沿（armed 仍 0）——条件恒真不会再触发（告警语义）
    interp.eval("rules::start r").unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires r").unwrap().as_str(), "1");
    // 回落再掀 → 第二次
    interp.eval("set ::c 0").unwrap();
    tick(&mut interp, 10);
    interp.eval("set ::c 1").unwrap();
    tick(&mut interp, 10);
    assert_eq!(interp.eval("rules::fires r").unwrap().as_str(), "2");
    // remove：取消 + 未知报错
    interp.eval("rules::remove r").unwrap();
    let e = interp.eval("rules::fires r").unwrap_err();
    assert!(e.to_string().contains("unknown rule"), "{e}");
    tick(&mut interp, 10);   // 已取消：无事件可泵、无错
}
