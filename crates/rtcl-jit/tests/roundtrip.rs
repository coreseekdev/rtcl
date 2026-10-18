//! 需要 jit-exec（wasmi 执行器）；默认构建跳过。
#![cfg(feature = "jit-exec")]

//! ABI v2 round-trip：parse → ByteCode → nanopass 发射 → wasmi 实例化 →
//! 宿主执行（`exec::run_top` 全链路），与解释器语义对齐。
//! M0 的三函数 handle ABI 测试已随 ABI v2 重写（设计文档 §3、风险 5）。

use rtcl_core::Interp;
use rtcl_jit::exec::{self, Outcome};
use rtcl_parser::{Compiler, ScriptUnit};
use std::rc::Rc;

/// 编译断言：`script` 的单元是否被 JIT 接受。
fn accepted(script: &str) -> bool {
    let unit = ScriptUnit::parse(script).expect("parse");
    let bc = Compiler::compile_unit(unit.source, &unit.commands).unfused();
    rtcl_jit::emit::compile(&bc, &Default::default()).is_ok()
}

/// 与 jitsh 文件模式同构的输出（stdout 拼接 + 退出码）——差分用。
fn jit_file_mode(script: &str, file: &str) -> (String, i32) {
    let interp = Interp::new();
    let (mut interp, run) = exec::run_top(interp, script, file);
    match run.outcome {
        Outcome::Fallback => match interp.eval(script) {
            Ok(_) => (String::new(), 0),
            Err(e) => (interp.error_report(&e, file), 1),
        },
        Outcome::Completed { code, exit_code } => match code {
            0 | 5 => (String::new(), 0),
            6 => (rtcl_core::Error::exit(Some(exit_code.unwrap_or(0))).to_string(), 1),
            _ => {
                if let Some(e) = run.eval_error {
                    (interp.error_report(&e, file), 1)
                } else {
                    (run.driver_report.clone().unwrap_or_default(), 1)
                }
            }
        },
    }
}

fn interp_file_mode(script: &str, file: &str) -> (String, i32) {
    let mut interp = Interp::new();
    match interp.eval(script) {
        Ok(_) => (String::new(), 0),
        Err(e) => (interp.error_report(&e, file), 1),
    }
}

fn assert_same(script: &str) {
    let file = "t.tcl";
    let a = interp_file_mode(script, file);
    let b = jit_file_mode(script, file);
    assert_eq!(a, b, "divergence for {script:?}:\n  interp={a:?}\n  jit   ={b:?}");
}

#[test]
fn wasm_magic_prefix_and_size() {
    let unit = ScriptUnit::parse("set s 0\nfor {set i 0} {$i < 100} {incr i} {set s [expr {$s + $i}]}")
        .expect("parse");
    let bc = Compiler::compile_unit(unit.source, &unit.commands).unfused();
    let m = rtcl_jit::emit::compile(&bc, &Default::default()).expect("compiles");
    assert_eq!(&m.bytes[..4], b"\0asm");
    // per-proc/unit 模块 <4KB 预算（HANDOFF §6）——含完整 ABI 导入段。
    assert!(m.bytes.len() < 4096, "module too large: {}", m.bytes.len());
}

#[test]
fn acceptance_subset() {
    // 接受：Tier1 折叠的数值/控制流 + Call + M2 原语/foreach/eval。
    for src in [
        "set x 1\nputs $x",
        "set i 0\nwhile {$i < 3} {incr i}",
        "for {set i 0} {$i < 9} {incr i 2} {if {$i == 4} {continue}}",
        "return 5",
        "exit 3",
        "foreach x {1 2} {puts $x}",
        "lappend l x",
        "set x 5\nputs x${x}y",
        "set s {puts hi}\nputs a[eval $s]b",
        "while {1} {break}",
        "set cmd puts\n$cmd hello",
        "puts [expr {min(2, 3)}]",    // mathfunc → EVAL_EXPR 宿主路由
    ] {
        assert!(accepted(src), "must accept: {src:?}");
    }
    // 拒绝：M1' 之外的 op（foreach/lappend/EvalScript/数组元素引用…）。
    // 顶层 `eval {...}`/`string ...` 本身是 Call（接受）；拒绝的是这些
    // op 出现在单元内的形状（经实证 disassemble 校准）。
    for src in [
        "set x [eval {puts 1}]",      // Call 嵌 Sub region（错误协议二分）
        "set a(1) 2\nputs $a($x)",   // 数组元素引用（下标替换语义）
    ] {
        assert!(!accepted(src), "must reject: {src:?}");
    }
}

#[test]
fn end_to_end_matches_interpreter() {
    // 覆盖：常量/变量/算术/比较/循环/break/continue/return/exit/错误帧链/
    // Sub 区域 defer/守卫兜底（非数值字符串/浮点/bignum）/exit 码。
    for src in [
        "set x 5\nputs $x\nputs [expr {$x + 3 * 2}]",
        "set s 0\nfor {set i 0} {$i < 100} {incr i} {set s [expr {$s + $i * 2}]}\nputs $s",
        "set i 0\nwhile {$i < 5} { incr i; puts $i }",
        "set s 0\nfor {set i 0} {$i < 10} {incr i 2} {\n  if {$i == 4} { continue }\n  if {$i == 8} { break }\n  set s [expr {$s + $i}]\n}\nputs $s",
        "set x abc\nputs [expr {$x + 1}]",
        "set x 1\nerror boom\nputs unreachable",
        "puts [expr {7 / 2}]\nputs [expr {-7 / 2}]\nputs [expr {-7 % 2}]\nputs [expr {1 << 100}]\nputs [expr {2 ** 10}]\nputs [expr {1 + \" 42 \"}]",
        "return 42\nputs no",
        "exit 3\nputs no",
        "if {yes} { puts yes } else { puts no }\nif no { puts a } elseif maybe { puts b } else { puts c }",
        "set x 1.5\nputs $x\nputs [expr {1.5 + 1}]",
        "set a(1) x\nputs $a(1)",
        "break",
        "continue",
        "set i 5\nset j 3\nputs [expr {$i & $j}]\nputs [expr {$i | 8}]\nputs [expr {$i ^ 1}]\nputs [expr {$i << 60}]\nputs [expr {$i >> 2}]\nputs [expr {!$i}]\nputs [expr {-$i}]\nputs [expr {~$i}]",
        "set x 1\nwhile {$x} { set x [expr {$x + 1}]; if {$x > 3} { break } }\nputs $x",
        "puts [expr {2 == 2}]\nputs [expr {\"42\" == 42}]\nputs [expr {abc eq abc}]\nputs [expr {abc neabd}]",
        "set big [expr {2 ** 62}]\nputs [expr {$big + $big}]",
    ] {
        assert_same(src);
    }
}

#[test]
fn tier1_bump_falls_back_to_interpreter() {
    // M3 入口失效 gate：Tier1 bump（proc set 重定义）之后，含折叠 set
    // 的顶层单元必须 Fallback，且终局行为与纯 interp.eval 逐字节一致
    // （tclsh 语义：重定义的 set 不赋值 → can't read "x"）。
    // 跨单元口径 = vm_exec eval.rs 的 bytecode gate（单元内中途 bump
    // 不重 gate——e1/e2 探针口径，JIT 不比 vm_exec 严）。
    let script = "set x 5\nputs $x";

    // 纯解释器基线：bump 后 eval 走树遍历（set 被重定义 proc 拦截）。
    let mut base = Interp::new();
    base.eval("proc set {args} {return 99}").expect("bump eval");
    let base_report = match base.eval(script) {
        Ok(v) => (v.to_string(), 0),
        Err(e) => (base.error_report(&e, "t.tcl"), 1),
    };
    assert!(
        base_report.0.contains("can't read \"x\""),
        "tree-walk baseline must shadow set: {base_report:?}"
    );

    // JIT 驱动：同一 Interp 上 bump 后 run_top_with ⇒ 必须 Fallback
    // （bc.epoch(0) != tier1_epoch(1)），终局与基线一致。
    let mut bumped = Interp::new();
    bumped.eval("proc set {args} {return 99}").expect("bump eval");
    assert_ne!(bumped.tier1_epoch(), 0, "bump must have fired");
    let (mut interp, run) = exec::run_top_with(bumped, script, "t.tcl", exec::EngineKind::Wasmi);
    match run.outcome {
        Outcome::Fallback => {}
        Outcome::Completed { code, .. } => {
            panic!("stale unit must fall back, got completed code={code}")
        }
    }
    let jit_report = match interp.eval(script) {
        Ok(v) => (v.to_string(), 0),
        Err(e) => (interp.error_report(&e, "t.tcl"), 1),
    };
    assert_eq!(base_report, jit_report, "fallback must match interpreter");

    // 对照：无 bump 时同脚本被接受（gate 不误伤）。
    assert!(accepted(script), "fresh-epoch unit must be accepted");
}

#[test]
fn budget_stops_jit_infinite_loop() {
    // `while {$x} {}` 形态被 JIT 接受（无条件回边）——fuel 必须可停机。
    let script = "set x 1\nwhile {$x} {}";
    let file = "t.tcl";
    // interp 侧
    let mut ip = Interp::new();
    ip.set_max_steps(Some(10_000));
    let a = match ip.eval(script) {
        Err(e) => (ip.error_report(&e, file), 1),
        Ok(_) => (String::new(), 0),
    };
    // jit 侧（同预算）
    let (mut ip2, run) = {
        let ip = Interp::new();
        let mut ip = ip;
        ip.set_max_steps(Some(10_000));
        exec::run_top(ip, script, file)
    };
    let b = match run.outcome {
        Outcome::Fallback => match ip2.eval(script) {
            Err(e) => (ip2.error_report(&e, file), 1),
            Ok(_) => (String::new(), 0),
        },
        Outcome::Completed { code, .. } => {
            assert_ne!(code, 0, "infinite loop must not complete");
            if let Some(e) = run.eval_error {
                (ip2.error_report(&e, file), 1)
            } else {
                (run.driver_report.clone().unwrap_or_default(), 1)
            }
        }
    };
    assert!(a.0.contains("step budget"), "interp: {a:?}");
    assert_eq!(a, b, "budget report must be byte-identical");
    // 可恢复：同单元无预算时正常完成（用退出码观察——文件模式不回显
    // 顶层结果；循环若未正常结束则 error）。
    let c = jit_file_mode(
        "set x 1\nset n 0\nwhile {$x} { incr n; if {$n > 4} { set x 0 } }\nif {$n != 5} { error \"n=$n\" }",
        file,
    );
    assert_eq!(c, (String::new(), 0), "resumable run: {c:?}");
}
