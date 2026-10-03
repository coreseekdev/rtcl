//! M0 acceptance: the full round-trip — parse Tcl → compile ByteCode →
//! emit a wasm module → instantiate → call → retrieve the result — runs
//! under a real wasm engine (wasmi, test-only) and matches the
//! interpreter's semantics for the compiled subset.

use rtcl_ir::bytecode::ByteCode;
use rtcl_parser::{Compiler, ScriptUnit};
use wasmi::{Caller, Engine, Linker, Module, Store, TypedFunc};

/// Host-side view of a Tcl value crossing the jit boundary (the arena,
/// per HANDOFF §5: u32 handles + runtime-side value storage).
#[derive(Debug, Clone, PartialEq)]
enum HostVal {
    Int(i64),
    Empty,
}

#[derive(Default)]
struct Host {
    arena: Vec<HostVal>,
    result: Option<HostVal>,
}

/// Compile `script` the same way `rtcl-core`'s vm_exec does, then emit a
/// wasm module from the ByteCode.
fn jit_compile(script: &str) -> Result<Vec<u8>, rtcl_jit::Unsupported> {
    let unit = ScriptUnit::parse(script).expect("parse");
    let code = Compiler::compile_unit(unit.source, &unit.commands);
    rtcl_jit::emit::compile_module(&code)
}

/// Instantiate `bytes` against a fresh host and run it; returns the Tcl
/// result code plus the recorded result value.
fn jit_run(bytes: &[u8]) -> Result<(i32, HostVal), wasmi::Error> {
    let engine = Engine::default();
    let module = Module::new(&engine, &mut &bytes[..])?;
    let mut store = Store::new(&engine, Host::default());
    let mut linker = <Linker<Host>>::new(&engine);
    linker.func_wrap(
        "rtcl",
        "push_int",
        |mut caller: Caller<'_, Host>, v: i64| -> i32 {
            let h = caller.data().arena.len() as i32;
            caller.data_mut().arena.push(HostVal::Int(v));
            h
        },
    )?;
    linker.func_wrap("rtcl", "push_empty", |mut caller: Caller<'_, Host>| -> i32 {
        let h = caller.data().arena.len() as i32;
        caller.data_mut().arena.push(HostVal::Empty);
        h
    })?;
    linker.func_wrap("rtcl", "set_result", |mut caller: Caller<'_, Host>, h: i32| {
        let v = caller.data().arena[h as usize].clone();
        caller.data_mut().result = Some(v);
    })?;
    let instance = linker.instantiate_and_start(&mut store, &module)?;
    let run: TypedFunc<(), i32> = instance.get_typed_func(&store, "run")?;
    let code = run.call(&mut store, ())?;
    let result = store.data().result.clone().expect("set_result ran");
    Ok((code, result))
}

fn roundtrip(script: &str) -> (i32, HostVal) {
    let bytes = jit_compile(script).expect("emitter accepts");
    jit_run(&bytes).expect("wasm runs")
}

#[test]
fn wasm_magic_prefix() {
    let bytes = jit_compile("return 42").expect("compiles");
    assert_eq!(&bytes[..4], b"\0asm");
    // A 3-instruction program is a few dozen bytes — the per-proc module
    // size assumption (<4KB, HANDOFF §5) has huge headroom.
    assert!(bytes.len() < 256, "module too large: {}", bytes.len());
}

#[test]
fn empty_body_roundtrip() {
    // `proc q {} {}` — an empty unit must yield the empty value.
    assert_eq!(roundtrip(""), (0, HostVal::Empty));
}

#[test]
fn return_int_roundtrip() {
    assert_eq!(roundtrip("return 42"), (0, HostVal::Int(42)));
    assert_eq!(roundtrip("return 0"), (0, HostVal::Int(0)));
}

#[test]
fn stream_end_takes_last_value() {
    // The interpreter's stream-end discipline: the result is the last
    // command's value (depth 1 at stream end — the leftover handle
    // becomes the result).  `expr {2 + 3}` also exercises the peephole
    // fold: the jit consumes the already-folded PUSH_INT 5.
    assert_eq!(roundtrip("expr {2 + 3}"), (0, HostVal::Int(5)));
}

#[test]
fn unsupported_ops_are_rejected_not_miscompiled() {
    // Anything outside the M0 subset must be rejected cleanly so the
    // caller keeps the interpreter path — never partially compiled.
    // (`return -7` dispatches by design — compile_return keeps
    // option-lookalike values away from the Return op.)
    for src in ["set x 1", "if {1} {} {}", "expr {1 + x}", "puts hi", "return -7"] {
        let err = jit_compile(src).expect_err("must reject");
        assert!(!err.0.is_empty());
    }
}
