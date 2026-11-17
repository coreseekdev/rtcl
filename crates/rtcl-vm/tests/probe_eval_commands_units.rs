//! 树走查直呼单元（`Interp::eval_commands` / `eval_commands_from` 裸调）
//! 的单元语境回归：proc 体经 `call_proc` 的 `_` 臂（体编译旁落/陈旧
//! epoch/fallback）走 `eval(def.body)` 时，嵌套 eval 的编译门（C1
//! `eval_depth == 1` 顶层 slot 编译）此前把**体单元**误编译成顶层
//! slot 单元——top_slots 变量路由与 proc 帧槽表（params-first 序）
//! 错位，体内循环/分支读实参读到邻槽值（"参数错位"）。宿主暴露面：
//! 设计器 capture 的 ui.tcl `eval_commands` 挂载形（rtcl-designer
//! capture.rs 模块注移交；VM 路径 `eval()` 正常——外层已占深度位）。
//!
//! 修复：eval_commands/eval_commands_from 占住深度第 1 位（嵌套
//! eval ≥2 → compile_unit，语义与 eval() 收敛）。本文件固化：
//! ①最小错位形（dict for + if 读实参）；②机制对照（has_foreach
//! 绕过 C1 门即愈）；③多行 braced 实参 20 形矩阵（tclsh 8.6.17
//! oracle 逐形钉值）；④构体内联语境；⑤ui.tcl 双挂载形。

use rtcl_vm::interp::Interp;
use rtcl_vm::Value;

fn run_tree(script: &str) -> String {
    let mut interp = Interp::new();
    let unit = rtcl_parser::ScriptUnit::parse(script).unwrap();
    match interp.eval_commands(&unit) {
        Ok(v) => v.as_str().to_string(),
        Err(e) => format!("<ERR: {}>", e.message_text()),
    }
}

fn run_vm(script: &str) -> String {
    let mut interp = Interp::new();
    match interp.eval(script) {
        Ok(v) => v.as_str().to_string(),
        Err(e) => format!("<ERR: {}>", e.message_text()),
    }
}

fn run_incremental(script: &str) -> String {
    let mut interp = Interp::new();
    match interp.eval_incremental(script) {
        Ok(v) => v.as_str().to_string(),
        Err(e) => format!("<ERR: {}>", e.message_text()),
    }
}

fn case(name: &str, script: &str, expect: &str) {
    let vm = run_vm(script);
    let tree = run_tree(script);
    assert_eq!(vm, expect, "[{name}] VM path diverges from tclsh oracle");
    assert_eq!(tree, expect, "[{name}] tree path (eval_commands) diverges");
}

const P1: &str = "proc pp {a b} { return \"a=<$a> b=<$b>\" }";

// ── ① 最小错位形：树分派 proc 体里 dict for 循环 + if 臂读实参 ──

const A1: &str = r#"namespace eval ::nsa1 {
    proc define {type spec} {
        set bad ""
        dict for {opt entry} $spec {
            if {[llength $entry] != 3} {
                set bad "A1 type=<$type> opt=<$opt> entry=<$entry>"
                break
            }
        }
        return "A1 type=<$type> bad=<$bad>"
    }
}
::nsa1::define Surface {
    -title {string {} {}}
}
"#;

#[test]
fn minimal_misbind_dict_for_if_reads() {
    // 修复前（裸调 eval_commands）：bad 读到 "A1 type=<spec 文本>
    // opt=<> entry=<Surface>"（体单元被 C1 误编译成顶层 slot 单元，
    // 读落邻槽）；VM 路径一直正确。
    case("A1-minimal", A1, "A1 type=<Surface> bad=<>");

    // 机制对照：体里塞一条无害 foreach → eval_impl 的 has_foreach 门
    // 关闭 toplevel 编译 → 与修复同效（修复前的活体证明）。
    const H1: &str = r#"namespace eval ::nsh1 {
        proc define {type spec} {
            set bad ""
            dict for {opt entry} $spec {
                if {[llength $entry] != 3} {
                    set bad "H1 type=<$type> opt=<$opt> entry=<$entry>"
                    break
                }
            }
            foreach _unused {0 1} {break}
            return "H1 type=<$type> bad=<$bad>"
        }
    }
    ::nsh1::define Surface {
        -title {string {} {}}
    }
    "#;
    case("H1-has-foreach-control", H1, "H1 type=<Surface> bad=<>");

    // 变体：无 variable 链接 / 顶层限定名 proc / 单行 braced 实参 /
    // 无前导短横——同一错位家族全修复。
    const B: &str = r#"namespace eval ::ns2 {}
    proc ::ns2::define {type spec} {
        set normspec [dict create]
        dict for {opt entry} $spec {
            if {[llength $entry] != 3} {
                error "B type=<$type> opt=<$opt> entry=<$entry>"
            }
        }
        return "B OK type=<$type>"
    }
    ::ns2::define Surface {
        -title {string {} {}}
    }
    "#;
    case("B-top-level-proc", B, "B OK type=<Surface>");

    const C: &str = r#"namespace eval ::ns3 {
        proc define {type spec} {
            set normspec [dict create]
            dict for {opt entry} $spec {
                if {[llength $entry] != 3} {
                    error "C type=<$type> opt=<$opt> entry=<$entry>"
                }
            }
            return "C OK type=<$type>"
        }
    }
    ::ns3::define Surface {-title {string {} {}}}
    "#;
    case("C-single-line-spec", C, "C OK type=<Surface>");

    // 循环构体对照：foreach/while 内联形本来就正确（修复前后皆绿）
    const F1: &str = r#"namespace eval ::nsf1 {
        proc define {type spec} {
            set bad ""
            foreach {opt entry} $spec {
                if {[llength $entry] != 3} {
                    set bad "F1 type=<$type> opt=<$opt> entry=<$entry>"
                    break
                }
            }
            return "F1 type=<$type> bad=<$bad>"
        }
    }
    ::nsf1::define Surface {
        -title {string {} {}}
    }
    "#;
    case("F1-foreach-control", F1, "F1 type=<Surface> bad=<>");

    // error 直落循环体（无 if 臂）也正确
    const A2: &str = r#"namespace eval ::nsa2 {
        proc define {type spec} {
            dict for {opt entry} $spec {
                error "A2 type=<$type> opt=<$opt> entry=<$entry>"
            }
            return unreachable
        }
    }
    set st [catch {::nsa2::define Surface {
        -title {string {} {}}
    }} msg]
    set out "st=$st msg=$msg"
    set out
    "#;
    case(
        "A2-error-in-loop",
        A2,
        "st=1 msg=A2 type=<Surface> opt=<-title> entry=<string {} {}>",
    );

    // eval_incremental（REPL/source 面）与直呼面同语义
    assert_eq!(run_incremental(A1), "A1 type=<Surface> bad=<>");
}

// ── ② ui.tcl 的 define 生成器形：proc 定义用双引号多行模板体 ──

#[test]
fn generated_proc_template_shape() {
    const GEN: &str = r#"
proc mkgen {type spec} {
    set ::cat $spec
    set lname [string tolower $type]
    proc ::$lname {path args} "
        ::do_it [list $type] \$path \$args
    "
}
proc ::do_it {t path arglist} {
    return "t=<$t> path=<$path> args=<$arglist>"
}
mkgen Surface {
    title {string {} {}}
    fill {enum {none x y both} {}}
}
surface .s -title Demo
"#;
    case("generated-proc", GEN, "t=<Surface> path=<.s> args=<-title Demo>");

    // 定义/调用分单元（挂载形：先挂定义单元，后直呼调用单元）
    let def_unit = r#"
proc mkgen {type spec} {
    proc ::surface {path args} "
        ::do_it [list $type] \$path \$args
    "
}
proc ::do_it {t path arglist} {
    return "t=<$t> path=<$path> args=<$arglist>"
}
"#;
    let call_unit = "mkgen Surface {\n title {string {} {}}\n}\nsurface .s -title Demo";
    let mut interp = Interp::new();
    let u1 = rtcl_parser::ScriptUnit::parse(def_unit).unwrap();
    interp.eval_commands(&u1).unwrap();
    let u2 = rtcl_parser::ScriptUnit::parse(call_unit).unwrap();
    let r = match interp.eval_commands(&u2) {
        Ok(v) => v.as_str().to_string(),
        Err(e) => format!("<ERR: {}>", e.message_text()),
    };
    assert_eq!(r, "t=<Surface> path=<.s> args=<-title Demo>");
}

// ── ③ 多行 braced 实参矩阵（tclsh 8.6.17 oracle 逐形钉值） ──

#[test]
fn matrix_multiline_braced_args() {
    case(
        "trivial",
        &format!("{}\nset r [pp {{\nfirst\n}} {{\nsecond\n}}]\nset r", P1),
        "a=<\nfirst\n> b=<\nsecond\n>",
    );
    case(
        "ml-body-ml-args",
        "proc pp {a b} {\n    return \"a=<$a> b=<$b>\"\n}\nset r [pp {\nfirst\n} {\nsecond\n}]\nset r",
        "a=<\nfirst\n> b=<\nsecond\n>",
    );
    case(
        "direct-call",
        &format!("{}\npp {{\nfirst\n}} {{\nsecond\n}}", P1),
        "a=<\nfirst\n> b=<\nsecond\n>",
    );
    case(
        "same-line-open",
        &format!("{}\npp {{first\nsecond}} {{b}}", P1),
        "a=<first\nsecond> b=<b>",
    );
    case(
        "three-args",
        "proc p3 {x y z} { return \"<$x|$y|$z>\" }\np3 head {\nmiddle\n} tail",
        "<head|\nmiddle\n|tail>",
    );
    case(
        "inner-content",
        "proc pp {a b} { return \"a=<$a> b=<$b>\" }\npp {\nfoo {bar} baz; qux \"q\"\n} {\nend\n}",
        "a=<\nfoo {bar} baz; qux \"q\"\n> b=<\nend\n>",
    );
    // 反斜杠换行在 braced 内折叠为单空格（tclsh oracle len 19）
    case(
        "backslash-nl",
        "proc pp {a} { return \"a=<$a>\" }\npp {\nfirst \\\nsecond\n}",
        "a=<\nfirst  second\n>",
    );
    case(
        "inner-hash",
        "proc pp {a} { return \"a=<$a>\" }\npp {\n# not a comment\nvalue\n}",
        "a=<\n# not a comment\nvalue\n>",
    );
    case(
        "followed-by-cmds",
        "proc pp {a b} { return \"a=<$a> b=<$b>\" }\nset r [pp {\nfirst\n} {\nsecond\n}]\nset t 1\nincr t\nstring length $r",
        "24",
    );
    case(
        "ns-qualified",
        "namespace eval n1 {}\nproc n1::pp {a b} { return \"a=<$a> b=<$b>\" }\nn1::pp {\nfirst\n} {\nsecond\n}",
        "a=<\nfirst\n> b=<\nsecond\n>",
    );
    // {*} 展开：list 解析剥外沿空白（oracle "a=<first> b=<second>"）
    case(
        "expand",
        "proc pp {a b} { return \"a=<$a> b=<$b>\" }\npp {*}{\nfirst\n second\n}",
        "a=<first> b=<second>",
    );
    case(
        "args-param",
        "proc pa {args} { return \"n=[llength $args] 0=[lindex $args 0]\" }\npa {\nfirst\n} {\nsecond\n}",
        "n=2 0=\nfirst\n",
    );
    case(
        "default-param",
        "proc dp {a {b bb} c} { return \"<$a|$b|$c>\" }\ndp {\nfirst\n} xx {\nthird\n}",
        "<\nfirst\n|xx|\nthird\n>",
    );
    case(
        "nested-call",
        &format!("{}\nproc outer {{}} {{\n  pp {{\ninner-first\n}} {{\ninner-second\n}}\n}}\nouter", P1),
        "a=<\ninner-first\n> b=<\ninner-second\n>",
    );
    case(
        "tabs-blank",
        "proc pp {a} { return \"a=<$a>\" }\npp {\n\tindented\n\n\ttail\n}",
        "a=<\n\tindented\n\n\ttail\n>",
    );
    case(
        "verbatim-var",
        "proc pp {a} { return \"a=<$a>\" }\nset v AAA\npp {\n$v stays\n}",
        "a=<\n$v stays\n>",
    );
    case(
        "verbatim-cmd",
        "proc pp {a} { return \"a=<$a>\" }\npp {\n[set zz] stays\n}",
        "a=<\n[set zz] stays\n>",
    );
    case(
        "via-var",
        "proc pp {a} { return \"a=<$a>\" }\nset x {\nfirst\n}\npp $x",
        "a=<\nfirst\n>",
    );
    case(
        "apply",
        "apply {{a b} { return \"a=<$a> b=<$b>\" }} {\nfirst\n} {\nsecond\n}",
        "a=<\nfirst\n> b=<\nsecond\n>",
    );
    case(
        "body-lines",
        "proc pp {a b} {\n    set s $a\n    append s \"|\"\n    append s $b\n    return $s\n}\npp {\nfirst\n} {\nsecond\n}",
        "\nfirst\n|\nsecond\n",
    );
}

// ── ④ 构体内联语境（体单元词法语境里的 proc 调用） ──

#[test]
fn matrix_multiline_braced_inside_constructs() {
    let shapes: Vec<(&str, String, &str)> = vec![
        ("proc-body", format!("{}\nproc outer {{}} {{\n  pp {{\ninner-first\n}} {{\ninner-second\n}}\n}}\nouter", P1),
         "a=<\ninner-first\n> b=<\ninner-second\n>"),
        ("namespace-eval", "namespace eval n2 {\n    proc pp {a b} { return \"a=<$a> b=<$b>\" }\n    set r [pp {\nfirst\n} {\nsecond\n}]\n}\nset ::out $n2::r\nset ::out".to_string(),
         "a=<\nfirst\n> b=<\nsecond\n>"),
        // catch 体 + list 串形化（oracle 引号形）
        ("catch-body", format!("{}\nset r [catch {{\n  pp {{\nfirst\n}} {{\nsecond\n}}\n}} msg]\nlist $r $msg", P1),
         "0 {a=<\nfirst\n> b=<\nsecond\n>}"),
        ("while-body", format!("{}\nset i 0\nset acc {{}}\nwhile {{$i < 2}} {{\n  lappend acc [pp v$i {{\narg$i\n}}]\n  incr i\n}}\nllength $acc", P1),
         "2"),
        ("foreach-body", format!("{}\nset acc {{}}\nforeach k {{x y}} {{\n  lappend acc [pp $k {{\nv-$k\n}}]\n}}\nllength $acc", P1),
         "2"),
        ("if-branch", format!("{}\nif {{1}} {{\n  pp {{\nfirst\n}} {{\nsecond\n}}\n}} else {{\n  nope\n}}", P1),
         "a=<\nfirst\n> b=<\nsecond\n>"),
        ("switch-branch", format!("{}\nswitch -- t {{\n  t {{\n    pp {{\nfirst\n}} {{\nsecond\n}}\n  }}\n  f {{nope}}\n}}", P1),
         "a=<\nfirst\n> b=<\nsecond\n>"),
        ("try-body", format!("{}\ntry {{\n  pp {{\nfirst\n}} {{\nsecond\n}}\n}} on error {{e}} {{\n  nope\n}}", P1),
         "a=<\nfirst\n> b=<\nsecond\n>"),
        ("def-in-namespace", "namespace eval n3 {\n    proc pp {a b} {\n        return \"a=<$a> b=<$b>\"\n    }\n}\nn3::pp {\nfirst\n} {\nsecond\n}".to_string(),
         "a=<\nfirst\n> b=<\nsecond\n>"),
    ];
    for (name, script, expect) in &shapes {
        let vm = run_vm(script);
        let tree = run_tree(script);
        assert_eq!(vm, *expect, "[{name}] VM path diverges from oracle");
        assert_eq!(tree, *expect, "[{name}] tree path (eval_commands) diverges");
    }
}

// ── ⑤ ui.tcl 双挂载形（rtcl-designer capture 的 ui.tcl 挂载面） ──

fn recorder(interp: &mut Interp, args: &[Value]) -> rtcl_vm::Result<Value> {
    let cur = interp
        .get_var("::ui_test_log")
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

fn tree_unit(interp: &mut Interp, script: &str) -> String {
    let unit = rtcl_parser::ScriptUnit::parse(script).unwrap();
    match interp.eval_commands(&unit) {
        Ok(v) => v.as_str().to_string(),
        Err(e) => format!("<ERR: {}>", e.message_text()),
    }
}

#[test]
fn ui_tcl_mounted_via_eval_commands() {
    let call_script = concat!(
        "ui::surface .s -title Demo\n",
        "set props {\n -text \"multi\"\n -width 3\n}\n",
        "ui::button .s.b {*}$props\n",
        "ui::text .s.t -text {\n hello\n}\n",
    );
    for (name, tree_mount) in [("vm-mount", false), ("tree-mount", true)] {
        let mut interp = Interp::new();
        interp.register_command("_ui", recorder);
        let _ = interp.eval("set ::ui_test_log {}");
        let ui_tcl = include_str!("../src/ui.tcl");
        let mount = if tree_mount {
            let unit = rtcl_parser::ScriptUnit::parse(ui_tcl).expect("ui.tcl parses");
            match interp.eval_commands(&unit) {
                Ok(_) => "OK".to_string(),
                Err(e) => format!("<ERR: {}>", e.message_text()),
            }
        } else {
            match interp.eval(ui_tcl) {
                Ok(_) => "OK".to_string(),
                Err(e) => format!("<ERR: {}>", e.message_text()),
            }
        };
        assert_eq!(mount, "OK", "[{name}] ui.tcl mount failed");
        // 调用一律树走查分派（裸调面）
        let r = tree_unit(&mut interp, call_script);
        assert!(
            !r.starts_with("<ERR"),
            "[{name}] multi-line braced ui calls failed under tree dispatch: {r}"
        );
        let log = tree_unit(&mut interp, "set ::ui_test_log");
        assert!(log.contains("hello"), "[{name}] text prop lost: {log}");
        assert!(log.contains("\"title\":\"Demo\""), "[{name}] surface props lost: {log}");
    }
}
