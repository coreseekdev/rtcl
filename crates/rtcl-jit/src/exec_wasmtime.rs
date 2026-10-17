//! wasmtime 引擎路径（feature `jit-native`，设计文档 §3「jit-native
//! 同一 ABI，host 侧用 Caller 实现」）。宿主逻辑共享 `exec::Host`
//! 的引擎无关方法；此处只做 import 注册与实例化。

use crate::abi;
use crate::emit;
use crate::exec::{Host, JitRun, Outcome, Pending};
use crate::pass::EmitConfig;
use rtcl_core::value::Value;
use rtcl_core::{Error, Interp};
use rtcl_parser::ByteCode;
use rtcl_vm::VmContext;
use std::rc::Rc;
use wasmtime::{Caller, Engine, Extern, Linker, Module, Store, TypedFunc, Val};

pub fn execute(
    interp: Interp,
    bc: &ByteCode,
    module: &emit::Module,
    file: &str,
) -> (Interp, JitRun) {
    let mut arena = vec![Value::unset(), Value::unset(), Value::empty()];
    for c in bc.constants() {
        arena.push(Value::from_str(c));
    }
    let host = Host::new(interp, arena, bc, module);
    let watermark = host.arena_len();

    let engine = Engine::default();
    let wmodule = Module::new(&engine, &module.bytes[..]).expect("wasmtime: module");
    let mut store = Store::new(&engine, host);
    let mut linker = <Linker<Host>>::new(&engine);

    let mut_g =
        |ty: wasmtime::ValType| wasmtime::GlobalType::new(ty, wasmtime::Mutability::Var);
    let g_fuel =
        wasmtime::Global::new(&mut store, mut_g(wasmtime::ValType::I64), Val::I64(0))
            .expect("fuel global");
    let g_epoch =
        wasmtime::Global::new(&mut store, mut_g(wasmtime::ValType::I64), Val::I64(0))
            .expect("epoch global");
    let g_caching =
        wasmtime::Global::new(&mut store, mut_g(wasmtime::ValType::I32), Val::I32(1))
            .expect("caching global");
    linker.define(&mut store, "rtcl", "fuel", g_fuel).unwrap();
    linker.define(&mut store, "rtcl", "epoch", g_epoch).unwrap();
    linker.define(&mut store, "rtcl", "caching", g_caching).unwrap();

    linker
        .func_wrap("rtcl", "unbox_int", |c: Caller<'_, Host>, h: i32| -> (i32, i64) {
            c.data().unbox_int_raw(h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "unbox_float", |c: Caller<'_, Host>, h: i32| -> (i32, f64) {
            c.data().unbox_float_raw(h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "box", |mut c: Caller<'_, Host>, v: i64| -> i32 {
            c.data_mut().box_raw(v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "box_float", |mut c: Caller<'_, Host>, f: f64| -> i32 {
            c.data_mut().box_float_raw(f)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "binary_op", |mut c: Caller<'_, Host>, op: i32, a: i64, b: i64| -> i64 {
            c.data_mut().binary_op_raw(op, a, b)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "get_var", |mut c: Caller<'_, Host>, name_h: i32| -> i64 {
            c.data_mut().get_var_raw(name_h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "set_var", |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i32 {
            c.data_mut().set_var_raw(name_h, v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "incr_var", |mut c: Caller<'_, Host>, name_h: i32, n: i64| -> i64 {
            c.data_mut().incr_var_raw(name_h, n)
        })
        .unwrap();
    let caching_g = g_caching.clone();
    linker
        .func_wrap(
            "rtcl",
            "call",
            move |mut c: Caller<'_, Host>, cmd_id: i32, site: i32, argc: i32| -> i64 {
                // 新进程不变量：首次命令调用关闭变量缓存。
                let _ = caching_g.set(&mut c, Val::I32(0));
                c.data_mut().call_raw(cmd_id, site, argc)
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "set_result", |mut c: Caller<'_, Host>, v: i64| {
            c.data_mut().set_result_raw(v);
        })
        .unwrap();
    let fuel_g = g_fuel.clone();
    linker
        .func_wrap("rtcl", "refuel", move |mut c: Caller<'_, Host>| -> i64 {
            let chunk = c.data().refuel_chunk();
            for _ in 0..chunk {
                if let Err(e) = VmContext::charge_step(c.data_mut().interp_mut()) {
                    c.data_mut().set_pending(e, false);
                    return abi::ERR_SENTINEL;
                }
            }
            let _ = fuel_g.set(&mut c, Val::I64(chunk));
            0
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "fail", |mut c: Caller<'_, Host>, fail_id: i32| -> i32 {
            c.data_mut().fail_raw(fail_id)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "bool_test", |mut c: Caller<'_, Host>, v: i64| -> i64 {
            c.data_mut().bool_test_raw(v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "push_const", |_c: Caller<'_, Host>, idx: i32| -> i64 {
            abi::const_handle(idx as u32)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "exit", |mut c: Caller<'_, Host>, code: i32| {
            c.data_mut().exit_raw(code);
        })
        .unwrap();

    let instance = linker.instantiate(&mut store, &wmodule).expect("wasmtime: instantiate");
    let run: TypedFunc<(), i32> = instance
        .get_typed_func(&mut store, "run")
        .expect("run export");
    let rc = run.call(&mut store, ()).expect("wasmtime: run");
    store.data_mut().arena_truncate(watermark);

    crate::exec::finish(store.into_data(), rc, file)
}
