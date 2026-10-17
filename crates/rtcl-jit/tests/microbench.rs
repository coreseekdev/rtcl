//! 开工前置 micro-bench (b) — TASK-JIT-WASM.md §2 / 设计文档 §8 假设 C:
//! per-proc 同步编译延迟 <1ms 预算。
//!
//! `cargo test --release -p rtcl-jit --test microbench -- --ignored --nocapture`
//!
//! 阶段拆分: Tcl→ByteCode(parse+compile) / wasm 发射 / 引擎 Module::new /
//! Linker 实例化。  `--ignored` 因为它是基准（数字随机器浮动），不是断言。

use rtcl_parser::{Compiler, ScriptUnit};
use wasm_encoder::{
    CodeSection, Function, FunctionSection, Instruction, Module, TypeSection, ValType,
};

fn bench<T>(label: &str, reps: usize, mut f: impl FnMut() -> T) {
    f(); // warmup
    let t0 = std::time::Instant::now();
    for _ in 0..reps {
        std::hint::black_box(f());
    }
    let us = t0.elapsed().as_nanos() as f64 / reps as f64 / 1000.0;
    println!("{:<34} {:>9.1} µs", label, us);
}

/// 一个 ~600 指令的循环体模块（wasm-encoder 直构，形状≈ arith_loop 的
/// lower 产物：循环内 i64 局部算术 + 回边），用于量引擎侧的
/// compile+instantiate 成本——发射器的成本由其自己的阶段行量。
fn fat_loop_module(n_ops: usize) -> Vec<u8> {
    let mut body = Function::new(vec![]);
    body.instruction(&Instruction::I64Const(0));
    let mut i = 0;
    while i < n_ops {
        body.instruction(&Instruction::I64Const(1));
        body.instruction(&Instruction::I64Add);
        i += 2;
    }
    body.instruction(&Instruction::End);
    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![ValType::I64]);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    let mut code = CodeSection::new();
    code.function(&body);
    let mut m = Module::new();
    m.section(&types).section(&funcs).section(&code);
    m.finish()
}

#[test]
#[ignore = "benchmark, not an assertion"]
fn per_proc_compile_latency() {
    println!("== 假设 C: per-proc 同步编译延迟（预算 <1000 µs） ==");

    let bodies: &[(&str, &str)] = &[
        ("trivial  (return 42)", "return 42"),
        ("arith    (sum loop)", "set s 0\nfor {set i 0} {$i < 100000} {incr i} {set s [expr {$s + $i * 2}]}\nputs $s"),
        ("fib      (proc body)", "if {$n < 2} {return $n}\nexpr {[fib [expr {$n - 1}]] + [fib [expr {$n - 2}]]}"),
        ("stringy  (concat/lindex)", "set l [list a b c]\nset r {}\nforeach x $l {append r $x-x}\nreturn $r"),
    ];
    for (name, src) in bodies {
        let src: &'static str = Box::leak(src.to_string().into_boxed_str());
        bench(&format!("bytecode: {name}"), 200, || {
            let unit = ScriptUnit::parse(src).expect("parse");
            Compiler::compile_unit(unit.source, &unit.commands)
        });
    }

    // 发射（M0 子集作为下界；管线落地后同口径重测）
    bench("emit wasm (M0 subset, lower bd)", 500, || {
        let unit = ScriptUnit::parse("return 42").expect("parse");
        let code = Compiler::compile_unit(unit.source, &unit.commands);
        rtcl_jit::emit::compile_module(&code).expect("emit")
    });

    // 引擎侧：wasmi 编译 + 实例化（~600 指令模块 ≈ arith_loop lower 产物）
    for (label, size) in [("wasmi compile  600-op mod", 600), ("wasmi compile 2000-op mod", 2000)] {
        let bytes = fat_loop_module(size);
        bench(label, 200, || wasmi::Module::new(&wasmi::Engine::default(), &bytes).expect("w"));
    }
    let bytes = fat_loop_module(600);
    bench("wasmi Module+instantiate 600-op", 200, || {
        let engine = wasmi::Engine::default();
        let module = wasmi::Module::new(&engine, &bytes).expect("w");
        let mut store = wasmi::Store::new(&engine, ());
        let linker = <wasmi::Linker<()>>::new(&engine);
        linker.instantiate_and_start(&mut store, &module).expect("inst")
    });
    println!("(all reps release-mode, best-effort single-thread)");
}
