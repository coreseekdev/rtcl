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
fn layout_options_pass_through_and_validate() {
    let mut interp = setup();
    interp.eval(r#"ui::surface .s; ui::row .s.r -gap 8"#).unwrap();
    interp
        .eval(r#"ui::text .s.r.t -text hi -weight 2 -fill y -align center -width 30%"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(log.contains(r#""gap":"8""#), "size 过线为 string: {log}");
    assert!(log.contains(r#""weight":2"#), "number 过线为 JSON number: {log}");
    assert!(log.contains(r#""fill":"y""#), "{log}");
    assert!(log.contains(r#""align":"center""#), "{log}");
    assert!(log.contains(r#""width":"30%""#), "{log}");

    // 布局选项的校验同样严格
    let e = interp.eval(r#"ui::text .s.r.t2 -fill everywhere"#).unwrap_err();
    assert!(e.to_string().contains("must be one of"), "{e}");
    let e = interp.eval(r#"ui::text .s.r.t2 -width abc"#).unwrap_err();
    assert!(e.to_string().contains("expected size"), "{e}");
    let e = interp.eval(r#"ui::text .s.r.t2 -weight x"#).unwrap_err();
    assert!(e.to_string().contains("expected number"), "{e}");

    // catalog 内省含公共布局选项（LLM 词表来源）
    let c = interp.eval("ui::catalog Text").unwrap().as_str().to_string();
    assert!(c.contains("-weight") && c.contains("-fill") && c.contains("-gap"), "{c}");
}

#[test]
fn snapshot_replays_to_equivalent_state() {
    let mut interp = setup();
    interp
        .eval(r#"ui::surface .s -title T; ui::text .s.t -text hello -weight 2"#)
        .unwrap();
    interp
        .eval(r#"ui::button .s.b -text Go -command {puts hi}"#)
        .unwrap();
    interp.eval(r#"ui::configure .s.t -text world"#).unwrap();
    interp.eval(r#"ui::model set /name alice"#).unwrap();

    let snap = interp
        .eval(r#"ui::snapshot .s -model /name"#)
        .unwrap()
        .as_str()
        .to_string();
    assert!(snap.contains("ui::surface .s -title T"), "{snap}");
    assert!(snap.contains("-text world"), "configure 已合并: {snap}");
    assert!(snap.contains("-command {puts hi}"), "{snap}");
    assert!(snap.contains("-weight 2"), "{snap}");
    assert!(snap.contains("ui::model set /name alice"), "{snap}");

    // destroy 后墓碑从快照消失
    interp.eval("ui::destroy .s.b").unwrap();
    let snap2 = interp.eval("ui::snapshot .s").unwrap().as_str().to_string();
    assert!(!snap2.contains(".s.b"), "{snap2}");

    // 重放等价：新 interp 执行快照，注册表与 op 流一致
    let mut interp2 = setup();
    interp2.eval(&snap).unwrap();
    let log2 = op_log(&mut interp2);
    assert!(log2.contains(r#""path":".s.t"#) && log2.contains(r#""text":"world""#), "{log2}");
    assert!(log2.contains(r#""weight":2"#), "{log2}");
    assert!(log2.contains(r#""events":["click"]"#), "{log2}");
    assert_eq!(
        interp2.eval("ui::model get /name").unwrap().as_str(),
        "alice"
    );
    // 快照里的 -command 重放后仍可触发
    interp2.post_event("::ui::fire .s.b click");
    assert!(interp2.pump_events().is_empty());
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

#[test]
fn table_create_emits_shape_and_scripts_stay_tcl_side() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"
            ui::table .s.t -path /pods -columns {name status restarts age} \
                -rowactions {open {puts open} del {puts del}} \
                -weight 2 -fill y
        "#)
        .unwrap();
    let log = op_log(&mut interp);
    // op 形状：bind 类 -path 过线为 props.bind；-columns 过线为 JSON
    // 数组；布局公共选项混入 props；events 数组含 row<动作> 名。
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.t","type":"Table","#,
            r#""props":{"bind":"\/pods","columns":["name","status","restarts","age"],"#,
            r#""weight":2,"fill":"y"},"events":["rowopen","rowdel"]}"#
        )),
        "table create op: {log}"
    );
    // -rowactions 的脚本本身不过桥（只有事件名过线）
    assert!(!log.contains("puts open"), "rowaction script must not cross: {log}");
    assert!(!log.contains("puts del"), "rowaction script must not cross: {log}");
    assert!(log.contains(r#""events":["rowopen","rowdel"]"#), "{log}");
}

#[test]
fn table_rowactions_validation_errors() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();

    // 奇数个元素 → 事件脚本不成对
    let e = interp
        .eval(r#"ui::table .s.t -path /pods -columns name -rowactions {open {puts o} foo}"#)
        .unwrap_err();
    assert!(e.to_string().contains("must be event script pairs"), "{e}");

    // 动作名非法（事件名要能成为 Tcl 词与 JS 字符串）
    let e = interp
        .eval(r#"ui::table .s.t -path /pods -columns name -rowactions {open! {puts o}}"#)
        .unwrap_err();
    assert!(e.to_string().contains("bad rowaction name"), "{e}");

    // 校验失败不发 op（.s 的 create 之外无多余批次）
    assert_eq!(op_log(&mut interp).lines().count(), 1, "errors must not emit ops");
}

#[test]
fn table_row_event_passes_row_index_via_ui_row() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"
            ui::table .s.t -path /pods -columns name -rowactions {open {set ::got $ui_row}}
        "#)
        .unwrap();
    // JS 回灌 "::ui::fire .s.t rowopen 42"，行号进回调脚本作用域
    interp.post_event("::ui::fire .s.t rowopen 42");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(interp.eval("set ::got").unwrap().as_str(), "42");
    assert!(
        interp.eval("info exists ::ui_row").unwrap().as_str() == "0",
        "ui_row must not leak out of the callback"
    );

    // rowclick 未绑 handler：静默无错（Tk 语义——未注册事件不是错误）
    interp.post_event("::ui::fire .s.t rowclick 42");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn table_model_incremental_subpath_ops() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp.eval(r#"ui::table .s.t -path /pods -columns name"#).unwrap();
    // 游标喂数：不预置 /pods（dict set 自动建父——与 tclsh 一致，
    // 预置标量再下钻会报 "missing value to go with key"）。
    // 行值以运行时字符串构造（词法 {…} 会被去外壳，json decode 的
    // 根检查看到的是 {"name":…} 原文）——真实喂法见测试与 demo。
    interp
        .eval(r#"set row [format {{"name":"pod-%d","status":"Running"}} 42]; ui::model set /pods/42 $row"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#""op":"model","sub":"set","path":"\/pods\/42","value":{"name":"pod-42","status":"Running"}"#),
        "row-increment op: {log}"
    );

    // 行存为原始 JSON 串（Tcl 模型树 dict 语义：标量不能再下钻），
    // v1 只支持整行增量；cell 级 /pods/42/status 需行值为 dict，
    // 留作模型通道演进（docs/UI-DSL.md 变更建议 §8）。
    // 直接传字面量行 JSON 要双大括号（Tcl 引词规则：一层 {…} 即去壳）。
    interp
        .eval(r#"ui::model set /pods/0 {{"name":"pod-0"}}"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#""op":"model","sub":"set","path":"\/pods\/0","value":{"name":"pod-0"}"#),
        "cursor ops stay one-row-per-op: {log}"
    );
}

#[test]
fn table_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Table").unwrap().as_str().to_string();
    // 组件自身选项 + 公共布局选项（词表层单一事实源，喂 LLM 的 prompt）
    for opt in ["-path", "-columns", "-rowactions", "-fill", "-weight", "-gap"] {
        assert!(c.contains(opt), "catalog Table missing {opt}: {c}");
    }
    assert!(c.contains("bind"), "{c}");
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Table"), "{all}");
}

// ── Log：词表层（追加式行数组；与 Table 同款 bind/游标语义）────

#[test]
fn log_create_emits_shape_and_cursor_appends() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::log .s.l -path /deploylog -weight 1 -fill x"#)
        .unwrap();
    let log = op_log(&mut interp);
    // -path bind 过线为 props.bind；布局公共选项混入；无 events（无
    // lineactions，lineclick 走 ui::on 动态注册）。
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.l","type":"Log","#,
            r#""props":{"bind":"\/deploylog","weight":1,"fill":"x"},"events":[]"#
        )),
        "log create op: {log}"
    );

    // 追加喂数：子路径一次一行（同 table 游标；行值对象带 level）
    interp
        .eval(r#"set l0 [format {{"ts":"00:00:01","level":"info","text":"pipeline ok"}}]; ui::model set /deploylog/0 $l0"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#""op":"model","sub":"set","path":"\/deploylog\/0","value":{"ts":"00:00:01","level":"info","text":"pipeline ok"}"#),
        "append op one-row-per-op: {log}"
    );
    // 标量行值也过线（渲染器 fallback info）
    interp.eval(r#"ui::model set /deploylog/1 {{"level":"warn","text":"slow"}}"#).unwrap();
    interp.eval(r#"ui::model set /deploylog/2 plain-text"#).unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#""path":"\/deploylog\/1","value":{"level":"warn","text":"slow"}"#),
        "{log}"
    );
    assert!(
        log.contains(r#""path":"\/deploylog\/2","value":"plain-text""#),
        "scalar line value passes as string: {log}"
    );
}

#[test]
fn log_lineclick_event_passes_line_index_via_ui_row() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp.eval(r#"ui::log .s.l -path /deploylog"#).unwrap();
    interp
        .eval(r#"ui::on .s.l lineclick {set ::got $ui_row}"#)
        .unwrap();
    let log = op_log(&mut interp);
    // ui::on 动态注册 → set op 携带更新后的 events（转发标记过桥）
    assert!(
        log.contains(r#""op":"set","path":".s.l","props":{},"events":["lineclick"]"#),
        "ui::on emits set op with events: {log}"
    );

    // JS 回灌 "::ui::fire .s.l lineclick 42"，行号进回调作用域
    interp.post_event("::ui::fire .s.l lineclick 42");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(interp.eval("set ::got").unwrap().as_str(), "42");
    assert!(
        interp.eval("info exists ::ui_row").unwrap().as_str() == "0",
        "ui_row must not leak out of the callback"
    );

    // 未绑事件名静默（Tk 语义）
    interp.post_event("::ui::fire .s.l linehover 3");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn log_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Log").unwrap().as_str().to_string();
    for opt in ["-path", "-fill", "-weight", "-gap"] {
        assert!(c.contains(opt), "catalog Log missing {opt}: {c}");
    }
    assert!(c.contains("bind"), "{c}");
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Log"), "{all}");
}

// ── Kv / Tree：详情双件套词表层 ────────────────────────────────

#[test]
fn kv_create_emits_shape_and_keys_list() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::kv .s.kv -path /info -keys {name status uptime} -weight 1"#)
        .unwrap();
    let log = op_log(&mut interp);
    // -path bind 过线；-keys list kind → JSON 数组；布局公共选项混入；
    // 无事件（kv 是只读详情，无行事件）。
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.kv","type":"Kv","#,
            r#""props":{"bind":"\/info","keys":["name","status","uptime"],"weight":1},"events":[]}"#
        )),
        "kv create op: {log}"
    );

    // 子路径增量：/info/status 单键更新一次一个 op
    interp
        .eval(r#"ui::model set /info/status Running"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#""op":"model","sub":"set","path":"\/info\/status","value":"Running""#),
        "kv per-key model op: {log}"
    );
}

#[test]
fn tree_create_emits_shape() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::tree .s.tree -path /obj -fill x"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.tree","type":"Tree","#,
            r#""props":{"bind":"\/obj","fill":"x"},"events":[]}"#
        )),
        "tree create op: {log}"
    );
}

#[test]
fn kv_tree_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Kv").unwrap().as_str().to_string();
    for opt in ["-path", "-keys", "-fill", "-weight"] {
        assert!(c.contains(opt), "catalog Kv missing {opt}: {c}");
    }
    assert!(c.contains("bind"), "{c}");

    let t = interp.eval("ui::catalog Tree").unwrap().as_str().to_string();
    for opt in ["-path", "-fill", "-weight"] {
        assert!(t.contains(opt), "catalog Tree missing {opt}: {t}");
    }
    assert!(t.contains("bind"), "{t}");

    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Kv"), "{all}");
    assert!(all.contains("Tree"), "{all}");
}

// ── Chart：Vega-Lite spec（JSON kind）过桥 ─────────────────────

#[test]
fn chart_create_passes_vegalite_spec_raw() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::chart .s.c -vegalite {{"data":{"name":"/cpu"},"mark":"line","encoding":{"x":{"field":"t"},"y":{"field":"v","type":"quantitative"}}}} -weight 1"#)
        .unwrap();
    let log = op_log(&mut interp);
    // spec 原样过线（json kind 不重编码）：数据路径在 spec 里
    // （data.name），图表定义与数据分离。
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.c","type":"Chart","#,
            r#""props":{"vegalite":{"data":{"name":"/cpu"},"mark":"line","encoding":{"x":{"field":"t"},"y":{"field":"v","type":"quantitative"}}},"weight":1},"events":[]}"#
        )),
        "chart create op: {log}"
    );
}

#[test]
fn chart_invalid_json_is_tcl_error() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    let e = interp.eval(r#"ui::chart .s.c -vegalite {not json}"#).unwrap_err();
    assert!(e.to_string().contains("expected JSON"), "{e}");
    // 坏 spec 不产生 op
    assert_eq!(op_log(&mut interp).lines().count(), 1, "no op for bad spec");
}

#[test]
fn chart_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Chart").unwrap().as_str().to_string();
    for opt in ["-vegalite", "-fill", "-weight", "-gap"] {
        assert!(c.contains(opt), "catalog Chart missing {opt}: {c}");
    }
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Chart"), "{all}");
}

// ── Trace：span 树瀑布词表层 ─────────────────────────────────

#[test]
fn trace_create_emits_shape() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::trace .s.tr -path /trace -weight 1 -fill x"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.tr","type":"Trace","#,
            r#""props":{"bind":"\/trace","weight":1,"fill":"x"},"events":[]}"#
        )),
        "trace create op: {log}"
    );
}

#[test]
fn trace_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Trace").unwrap().as_str().to_string();
    for opt in ["-path", "-fill", "-weight"] {
        assert!(c.contains(opt), "catalog Trace missing {opt}: {c}");
    }
    assert!(c.contains("bind"), "{c}");
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Trace"), "{all}");
}

// ── Confirm：模态决定的异步回灌（禁 vwait）─────────────────────

#[test]
fn confirm_create_binds_command_to_ok_event() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::confirm .s.c -danger 1 -preview {{"issue":"SENTRY-42","affected":3}} -command {set ::ran 1}"#)
        .unwrap();
    let log = op_log(&mut interp);
    // -command 绑 ok（不是 click）；脚本不过桥；preview 原样过线。
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.c","type":"Confirm","#,
            r#""props":{"danger":true,"preview":{"issue":"SENTRY-42","affected":3}},"events":["ok"]}"#
        )),
        "confirm create op: {log}"
    );
    assert!(!log.contains("set ::ran"), "command script must not cross: {log}");
}

#[test]
fn confirm_decision_relay_is_async() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::confirm .s.c -command {set ::ran 1}"#)
        .unwrap();
    // 创建后脚本立即跑到底（没有等待）——ran 未置位
    assert_eq!(interp.eval("info exists ::ran").unwrap().as_str(), "0");

    // JS 点 Confirm → fire ok → 命令跑
    interp.post_event("::ui::fire .s.c ok");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(interp.eval("set ::ran").unwrap().as_str(), "1");

    // cancel 默认无 handler：静默
    interp.post_event("::ui::fire .s.c cancel");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");

    // ui::on 绑 cancel → set op 带新事件；再点 Cancel 生效
    interp
        .eval(r#"ui::on .s.c cancel {set ::cancelled 1}"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(r#""op":"set","path":".s.c","props":{},"events":["ok","cancel"]"#),
        "cancel binding crosses as events: {log}"
    );
    interp.post_event("::ui::fire .s.c cancel");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(interp.eval("set ::cancelled").unwrap().as_str(), "1");

    // ok 重复回灌：命令再跑一次（JS 侧按钮已禁用，协议层不拦）
    interp.post_event("::ui::fire .s.c ok");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn confirm_invalid_preview_is_tcl_error() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    let e = interp.eval(r#"ui::confirm .s.c -preview {not json} -command {}"#).unwrap_err();
    assert!(e.to_string().contains("expected JSON"), "{e}");
}

#[test]
fn confirm_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Confirm").unwrap().as_str().to_string();
    for opt in ["-danger", "-preview", "-command", "-fill", "-weight"] {
        assert!(c.contains(opt), "catalog Confirm missing {opt}: {c}");
    }
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Confirm"), "{all}");
}

#[test]
fn json_kind_accepts_scalars_rejects_empty() {
    // json kind 放行标量 JSON（-confirm 的字符串消息预览依赖它；
    // rtcl 的 json decode 只收对象/数组为根，校验包了一层数组）；
    // 空串会破坏 ops payload，必须拒绝。
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp.eval(r#"ui::confirm .s.a -preview {"executed — re-armed"}"#).unwrap();
    interp.eval(r#"ui::confirm .s.b -preview 42"#).unwrap();
    interp.eval(r#"ui::confirm .s.c -preview true"#).unwrap();
    interp.eval(r#"ui::confirm .s.d -preview null"#).unwrap();
    let log = op_log(&mut interp);
    assert!(log.contains(r#""preview":"executed — re-armed""#), "{log}");
    assert!(log.contains(r#""preview":42"#), "{log}");
    assert!(log.contains(r#""preview":true"#), "{log}");
    assert!(log.contains(r#""preview":null"#), "{log}");

    let e = interp.eval(r#"ui::confirm .s.e -preview {}"#).unwrap_err();
    assert!(e.to_string().contains("expected JSON"), "{e}");
}

// ── Form：schema 驱动表单（-command 绑 commit）────────────────

#[test]
fn form_create_emits_shape_and_binds_commit() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::form .s.f -path /form -schema {{"type":"object","properties":{"email":{"type":"string","title":"Email"}},"required":["email"]}} -command {set ::submitted 1}"#)
        .unwrap();
    let log = op_log(&mut interp);
    assert!(
        log.contains(concat!(
            r#"{"op":"create","path":".s.f","type":"Form","#,
            r#""props":{"bind":"\/form","schema":{"type":"object","properties":{"email":{"type":"string","title":"Email"}},"required":["email"]}},"events":["commit"]}"#
        )),
        "form create op: {log}"
    );
    assert!(!log.contains("set ::submitted"), "command script must not cross: {log}");
}

#[test]
fn form_commit_relay_and_error_path() {
    let mut interp = setup();
    interp.eval("ui::surface .s").unwrap();
    interp
        .eval(r#"ui::form .s.f -path /form -schema {{"type":"object","properties":{}}} -command {set ::submitted 1}"#)
        .unwrap();
    // 创建后脚本跑到底（不阻塞）
    assert_eq!(interp.eval("info exists ::submitted").unwrap().as_str(), "0");
    interp.post_event("::ui::fire .s.f commit");
    let errors = interp.pump_events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(interp.eval("set ::submitted").unwrap().as_str(), "1");

    // 非法 schema JSON → Tcl 错误
    let e = interp.eval(r#"ui::form .s.bad -schema {nope}"#).unwrap_err();
    assert!(e.to_string().contains("expected JSON"), "{e}");
}

#[test]
fn form_catalog_shows_options_and_layout() {
    let mut interp = setup();
    let c = interp.eval("ui::catalog Form").unwrap().as_str().to_string();
    for opt in ["-path", "-schema", "-command", "-fill", "-weight"] {
        assert!(c.contains(opt), "catalog Form missing {opt}: {c}");
    }
    assert!(c.contains("bind"), "{c}");
    let all = interp.eval("ui::catalog").unwrap().as_str().to_string();
    assert!(all.contains("Form"), "{all}");
}
