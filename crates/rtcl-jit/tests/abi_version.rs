//! 需要 jit-exec（wasmi 执行器）；默认构建跳过。
#![cfg(feature = "jit-exec")]

//! ABI 版本 export（docs/ABI-OPT-NOTES.md §3a）：发射产物携带
//! `rtcl_abi_version` export、宿主实例化后立即比对、不匹配明确
//! 拒绝。三层覆盖：比对函数矩阵（纯函数）/ 发射产物 export 存在性
//! （wasm 字节面）/ run_top 端到端（宿主全链路——检查在实例化与
//! run 之间，panic 即测试失败）。

use rtcl_core::Interp;
use rtcl_jit::abi::{abi_version_compatible, ABI_VERSION};
use rtcl_jit::exec::{self, Outcome};
use rtcl_parser::{Compiler, ScriptUnit};

#[test]
fn version_constant_encoding() {
    // v2.0 = 高 16 位 major 2 / 低 16 位 minor 0（abi.rs 注释的约定）。
    assert_eq!(ABI_VERSION, 0x0002_0000);
    assert_eq!(ABI_VERSION >> 16, 2);
    assert_eq!(ABI_VERSION & 0xffff, 0);
}

#[test]
fn compatibility_matrix() {
    // 同版本 → 放行。
    assert!(abi_version_compatible(ABI_VERSION).is_ok());
    // 模块 minor 更小 → 放行（minor = 加法性扩展，宿主向后兼容）。
    assert!(abi_version_compatible((2 << 16) | 0).is_ok());
    // 模块 minor 超前 → 拒绝（宿主不认识的新面）。
    let e = abi_version_compatible((2 << 16) | 1).unwrap_err();
    assert!(e.contains("minor ahead"), "{e}");
    // major 不等（旧 v1 模块 / 未知的 v3 模块）→ 拒绝。
    let e = abi_version_compatible((1 << 16) | 0).unwrap_err();
    assert!(e.contains("major mismatch"), "{e}");
    let e = abi_version_compatible((3 << 16) | 0).unwrap_err();
    assert!(e.contains("major mismatch"), "{e}");
}

/// 发射产物字节面：exports 段含 `rtcl_abi_version`（fast 与常规两种
/// 形态——fast 形态的函数索引不同，追加序必须两种都对）。
#[test]
fn emitted_module_exports_version() {
    for (name, script) in [
        ("classic", "set x 0\nset y [expr {$x + 1}]"),
        // 循环内 callsite 走 v5 回落，不落 fast；fib 形态是 fast 门
        // 经典命中（proc_main 导出的 fast 模块形状）。
        ("fast", "proc fib n { if {$n < 2} { return $n }; return [expr {[fib [expr {$n-1}] + [fib [expr {$n-2}]]]}] }"),
    ] {
        let unit = ScriptUnit::parse(script).expect("parse");
        let bc = Compiler::compile_unit(unit.source, &unit.commands).unfused();
        let m = rtcl_jit::emit::compile(&bc, &Default::default()).expect("emit");
        let engine = wasmi::Engine::default();
        let wm = wasmi::Module::new(&engine, &m.bytes[..]).expect("valid wasm");
        let names: Vec<&str> = wm.exports().map(|e| e.name()).collect();
        assert!(
            names.contains(&"rtcl_abi_version"),
            "{name}: no rtcl_abi_version export; exports = {names:?}"
        );
    }
}

/// 端到端：run_top 全链路（宿主实例化后比对再 run）——版本检查
/// panic 会击穿本测试；正常完成即检查通过。
#[test]
fn run_top_passes_version_check() {
    let script = "set acc 0\nfor {set i 0} {$i < 10} {incr i} { set acc [expr {$acc + $i}] }\nset acc";
    let interp = Interp::new();
    let (mut interp, run) = exec::run_top(interp, script, "abi_version.tcl");
    match run.outcome {
        Outcome::Fallback => {
            let v = interp.eval(script).expect("interp eval");
            assert_eq!(v.as_int(), Some(45));
        }
        Outcome::Completed { code, .. } => {
            assert_eq!(code, 0);
            assert_eq!(run.result.as_ref().and_then(|v| v.as_int()), Some(45));
        }
    }
}
