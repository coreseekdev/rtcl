//! wasmtime 引擎路径（feature `jit-native`，设计文档 §3「jit-native
//! 同一 ABI，host 侧用 Caller 实现」）。宿主逻辑共享 `exec::Host`
//! 的引擎无关方法；此处只做 import 注册与实例化。

use crate::emit;
use crate::exec::{Host, JitRun};
use rtcl_core::value::Value;
use rtcl_core::Interp;
use rtcl_parser::ByteCode;

use wasmtime::{Engine, Linker, Module, Store, TypedFunc};


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
    // M3：epoch global 以解释器当前代初始化（与 exec.rs 同一语义——
    // 入口-only，运行中不刷新；回边比对为 M4 模块缓存预留）。
    let _epoch0 = interp.tier1_epoch() as i64;
    let mut interp_cell = interp;
    let host = Host::new(&mut interp_cell as *mut rtcl_core::Interp, arena, bc, module);
    let watermark = host.arena_len();

    let engine = Engine::default();
    let wmodule = Module::new(&engine, &module.bytes[..]).expect("wasmtime: module");
    let mut store = Store::new(&engine, host);
    let linker = <Linker<Host>>::new(&engine);


    let instance = linker.instantiate(&mut store, &wmodule).expect("wasmtime: instantiate");
    let run: TypedFunc<(), i32> = instance
        .get_typed_func(&mut store, "run")
        .expect("run export");
    let rc = run.call(&mut store, ()).expect("wasmtime: run");
    store.data_mut().arena_truncate(watermark);

    let host = store.into_data();
    let run = crate::exec::finish(host, rc, file);
    (interp_cell, run)
}
