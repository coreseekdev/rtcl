//! 跨引擎产物回环门禁：同一份 .wasm 字节在 wasmi 与 wasmtime 上执行
//! 必须与解释器等值（产物即产品——towasm 线的核心主张）。

use rtcl_core::Interp;
use rtcl_towasm::compile_script;

fn probe(interp: &Interp) -> String {
    match interp.get_var("__tw_probe") {
        Ok(v) => v.as_str().to_string(),
        Err(_) => "<unset>".into(),
    }
}

fn roundtrip(src: &str) {
    let art = compile_script(src).expect("compile");
    // 解释器基线
    let mut ip = Interp::new();
    ip.eval(src).expect("interp eval");
    let base = probe(&ip);
    // wasmi 产物
    let (iw, rw) = rtcl_jit::exec::execute_artifact_wasmi(
        Interp::new(),
        &art.bc,
        &art.module,
        "roundtrip",
    );
    assert!(
        !matches!(rw.outcome, rtcl_jit::exec::Outcome::Fallback),
        "wasmi fell back"
    );
    // wasmtime 产物
    let (it, _rt) = rtcl_jit::exec_wasmtime::execute(
        Interp::new(),
        &art.bc,
        &art.module,
        "roundtrip",
    );
    assert_eq!(base, probe(&iw), "wasmi probe mismatch");
    assert_eq!(base, probe(&it), "wasmtime probe mismatch");
}

#[test]
fn arith() {
    roundtrip(
        "set __tw_probe [expr {6 * 7}]\n",
    );
}

#[test]
fn loop_sum() {
    roundtrip(
        "set t 0\nfor {set i 0} {$i < 100} {incr i} { set t [expr {$t + $i}] }\nset __tw_probe $t\n",
    );
}

#[test]
fn strings() {
    roundtrip(
        "set s \"\"\nfor {set i 0} {$i < 5} {incr i} { append s \"x$i,\" }\nset __tw_probe $s\n",
    );
}

#[test]
fn proc_call() {
    roundtrip(
        "proc add {a b} { expr {$a + $b} }\nset __tw_probe [add 20 22]\n",
    );
}

#[test]
fn control_flow() {
    roundtrip(
        "set out {}\nforeach x {1 2 3 4} { if {$x == 2} continue; if {$x > 3} break; lappend out $x }\nset __tw_probe $out\n",
    );
}

#[test]
fn manifest_lists_abi() {
    // classic 顶层：内嵌 memory（无共享 memory/table 导入），27 宿主
    // func + 3 global 导入，run/cur_site/memory 导出。
    let art = compile_script("set __tw_probe 1\n").expect("compile");
    assert!(art.manifest.contains("func rtcl.call"));
    assert!(art.manifest.contains("global rtcl.fuel"));
    assert!(art.manifest.contains("func run"));
    assert!(art.manifest.contains("global cur_site"));
    assert!(art.manifest.contains("memory: min="));
    assert!(!art.manifest.contains("reference-types"));
    // 产物只含顶层单元（jiterpreter 模式）——proc 体运行时编译，
    // 清单如实标注（AOT 模块集 = towasm 路线图）。
    let art2 = compile_script("proc f {n} { expr {$n * 2} }\nset __tw_probe [f 21]\n")
        .expect("compile");
    assert!(art2.manifest.contains("proc-bodies: runtime-compiled"));
    assert!(art2.manifest.contains("func run"));
}
