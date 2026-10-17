//! log.tcl 词表层的原生集成测试：内存 ring（容量/淘汰/来源）、查询
//! 输出的形状与排序（search/stats/top/hist/lines）。契约见
//! docs/ROADMAP.md §3。

use rtcl_core::Interp;

fn setup() -> Interp {
    let mut interp = Interp::new();
    interp.load_log().expect("load_log");
    interp
}

fn ev(interp: &mut Interp, s: &str) -> String {
    interp.eval(s).unwrap().as_str().to_string()
}

#[test]
fn ingest_validates_json_object_text() {
    let mut interp = setup();
    assert_eq!(ev(&mut interp, r#"log::ingest {{"a":1}}"#), "1");
    assert_eq!(ev(&mut interp, r#"log::ingest {{"a":2}}"#), "2");
    assert_eq!(ev(&mut interp, "log::count"), "2");
    let e = interp.eval(r#"log::ingest {plain text}"#).unwrap_err();
    assert!(e.to_string().contains("JSON object"), "{e}");
    let e = interp.eval(r#"log::ingest {[1,2]}"#).unwrap_err();
    assert!(e.to_string().contains("JSON object"), "{e}");
}

#[test]
fn ring_capacity_evicts_oldest_and_retrims_on_shrink() {
    let mut interp = setup();
    assert_eq!(ev(&mut interp, "log::capacity 3"), "3");
    for i in 0..5 {
        ev(&mut interp, &format!(r#"log::ingest {{{{"i":{i}}}}}"#));
    }
    assert_eq!(ev(&mut interp, "log::count"), "3");
    assert_eq!(ev(&mut interp, r#"join [log::tail 10] |"#), r#"{"i":2}|{"i":3}|{"i":4}"#);
    assert_eq!(ev(&mut interp, r#"join [log::search {"i":0}] |"#), "");
    // 缩小容量立即重裁（保留最新）
    assert_eq!(ev(&mut interp, "log::capacity 2"), "2");
    assert_eq!(ev(&mut interp, "log::count"), "2");
    assert_eq!(ev(&mut interp, r#"join [log::tail 10] |"#), r#"{"i":3}|{"i":4}"#);
}

#[test]
fn source_filter_applies_to_count_tail_search() {
    let mut interp = setup();
    ev(&mut interp, r#"log::ingest {{"m":"a1"}} -source api"#);
    ev(&mut interp, r#"log::ingest {{"m":"w1"}} -source web"#);
    ev(&mut interp, r#"log::ingest {{"m":"a2"}} -source api"#);
    assert_eq!(ev(&mut interp, "log::count -source api"), "2");
    assert_eq!(ev(&mut interp, "log::count -source web"), "1");
    assert_eq!(ev(&mut interp, "log::count"), "3");
    assert_eq!(ev(&mut interp, r#"join [log::tail 10 -source api] |"#), r#"{"m":"a1"}|{"m":"a2"}"#);
    assert_eq!(ev(&mut interp, r#"lindex [log::search a1 -source api] 0"#), r#"{"m":"a1"}"#);
    assert_eq!(ev(&mut interp, r#"join [log::search a1 -source web] |"#), "");
}

#[test]
fn search_newest_first_with_limit() {
    let mut interp = setup();
    for i in 0..4 {
        ev(&mut interp, &format!(r#"log::ingest {{{{"kind":"hit","i":{i}}}}}"#));
    }
    ev(&mut interp, r#"log::ingest {{"kind":"miss"}}"#);
    let r = ev(&mut interp, r#"join [log::search hit] |"#);
    assert_eq!(r, r#"{"kind":"hit","i":3}|{"kind":"hit","i":2}|{"kind":"hit","i":1}|{"kind":"hit","i":0}"#);
    assert_eq!(ev(&mut interp, r#"join [log::search hit -limit 2] |"#), r#"{"kind":"hit","i":3}|{"kind":"hit","i":2}"#);
}

#[test]
fn stats_order_limit_and_missing_field() {
    let mut interp = setup();
    for _ in 0..3 { ev(&mut interp, r#"log::ingest {{"level":"info","n":1}}"#); }
    for _ in 0..2 { ev(&mut interp, r#"log::ingest {{"level":"warn","n":2}}"#); }
    ev(&mut interp, r#"log::ingest {{"level":"error","n":3}}"#);
    ev(&mut interp, r#"log::ingest {{"level":"zzz","n":4}}"#);
    ev(&mut interp, r#"log::ingest {{"other":1}}"#);   // 缺 level：跳过
    // 次数降序，并列值升序
    assert_eq!(ev(&mut interp, "log::stats level"), "info 3 warn 2 error 1 zzz 1");
    assert_eq!(ev(&mut interp, "log::stats level -limit 2"), "info 3 warn 2");
    assert_eq!(ev(&mut interp, "log::top level 3"), "info 3 warn 2 error 1");
    assert_eq!(ev(&mut interp, "log::stats other"), "1 1");
    assert_eq!(ev(&mut interp, "log::stats nope"), "");
}

#[test]
fn hist_buckets_and_edge_cases() {
    let mut interp = setup();
    for v in [1, 2, 3, 10] {
        ev(&mut interp, &format!(r#"log::ingest {{{{"n":{v}}}}}"#));
    }
    ev(&mut interp, r#"log::ingest {{"n":"x"}}"#);   // 非数值：跳过
    // 1..10 三等分：桶宽 3 → [1,4)=3 [4,7)=0 [7,10]=1
    assert_eq!(ev(&mut interp, "log::hist n 3"), "1 4 3 4 7 0 7 10 1");

    // 全等值：桶宽 1，全落桶 0
    let mut flat = setup();
    for _ in 0..2 { ev(&mut flat, r#"log::ingest {{"n":5}}"#); }
    assert_eq!(ev(&mut flat, "log::hist n 2"), "5 6 2 6 7 0");

    // 无数据：全零桶
    assert_eq!(ev(&mut interp, "log::hist missing 2"), "0 0 0 0 0 0");
}

#[test]
fn ingest_lines_batch_counts_and_trims() {
    let mut interp = setup();
    assert_eq!(ev(&mut interp, r#"log::ingest-lines "{\"a\":1}\n\n{\"a\":2}\n" -source batch"#), "2");
    assert_eq!(ev(&mut interp, "log::count -source batch"), "2");
    assert_eq!(ev(&mut interp, "log::stats a -source batch"), "1 1 2 1");
}
