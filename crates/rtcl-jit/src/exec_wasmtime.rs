//! wasmtime 引擎路径（feature `jit-native`，设计文档 §3「jit-native
//! 同一 ABI，host 侧用 Caller 实现」）。宿主逻辑共享 `exec::Host`
//! 的引擎无关方法；此处只做 import 注册与实例化。
//!
//! `register_wasmtime_abi` 是顶层脚本路径与 proc 池路径（proc_exec.rs）
//! 共用的注册体：全套 `rtcl::*` 宿主路由 + fuel/epoch/caching 三个
//! 可变 global，返回 global 句柄供池命中重置。

use crate::emit;
use crate::exec::{Host, JitRun};
use rtcl_core::Interp;
use rtcl_parser::ByteCode;

use wasmtime::{Linker, Module, Store, TypedFunc};

thread_local! {
    /// wasmtime Engine 单例（顶层与 proc 池路径共用；Engine 构造贵）。
    /// opt_level none：编译时间优先——顶层大单元（gen_set 445 行）在
    /// 默认 speed 档一次 Cranelift 编译 ~19s，撞 judge 的 15s 超时；
    /// none 档编译快数倍，运行档位损失由性能 bench 单独量（差分门禁
    /// 只看值正确性）。
    static WT_ENGINE: wasmtime::Engine = {
        let mut cfg = wasmtime::Config::new();
        cfg.cranelift_opt_level(wasmtime::OptLevel::None);
        wasmtime::Engine::new(&cfg).expect("wasmtime engine")
    };
}

/// 共享 Engine 句柄（proc 池路径的 Store/Module 同源）。
pub(crate) fn wt_engine() -> wasmtime::Engine {
    WT_ENGINE.with(|e| e.clone())
}


pub fn execute(
    interp: Interp,
    bc: &ByteCode,
    module: &emit::Module,
    file: &str,
) -> (Interp, JitRun) {
    // arena 预驻留与 wasmi 顶层路径同源（arena_of：UNSET/哨兵/EMPTY
    // + 常量池 + **locals 名区**——slot→name 重写的伪常量索引越过
    // 常量池后指向 locals 名；漏掉会让全部变量读写错位，lappend/
    // upvar/uplevel 全挂——proc_scope/list_basic 差分实证）。
    let epoch0 = interp.tier1_epoch() as i64;
    let mut interp_cell = interp;
    let host = Host::new(&mut interp_cell as *mut rtcl_core::Interp, crate::exec::arena_of(bc), bc, module);
    let watermark = host.arena_len();

    let engine = wt_engine();
    let wmodule = Module::new(&engine, &module.bytes[..]).expect("wasmtime: module");
    let mut store = Store::new(&engine, host);

    // 全套 ABI（与 proc 池路径同一注册体；global 句柄一次性路径不用）。
    let (instance, _globals) = register_wasmtime_abi(&mut store, epoch0, &wmodule);
    let run: TypedFunc<(), i32> = instance
        .get_typed_func(&mut store, "run")
        .expect("run export");
    let rc = run.call(&mut store, ()).expect("wasmtime: run");
    store.data_mut().arena_truncate(watermark);

    let host = store.into_data();
    let run = crate::exec::finish(host, rc, file);
    (interp_cell, run)
}

/// wasmtime 的 ABI 注册（顶层与 proc 池路径共用）：三个可变 global +
/// 全套宿主路由（call_argv 直连、fail、foreach 四件套、eval 路由等）。
/// 返回 (Instance, (fuel, epoch, caching) 句柄)——global 句柄供池命中
/// 重置（define 进 linker 后克隆仍有效）。
pub(crate) fn register_wasmtime_abi(
    store: &mut wasmtime::Store<Host>,
    epoch0: i64,
    wmodule: &wasmtime::Module,
) -> (
    wasmtime::Instance,
    (wasmtime::Global, wasmtime::Global, wasmtime::Global),
) {
    use wasmtime::{Caller, Val};
    let mut linker = <Linker<Host>>::new(store.engine());

    let mut_g =
        |ty: wasmtime::ValType| wasmtime::GlobalType::new(ty, wasmtime::Mutability::Var);
    let g_fuel =
        wasmtime::Global::new(&mut *store, mut_g(wasmtime::ValType::I64), Val::I64(0))
            .expect("fuel global");
    let g_epoch =
        wasmtime::Global::new(&mut *store, mut_g(wasmtime::ValType::I64), Val::I64(epoch0))
            .expect("epoch global");
    let g_caching =
        wasmtime::Global::new(&mut *store, mut_g(wasmtime::ValType::I32), Val::I32(1))
            .expect("caching global");
    linker.define(&mut *store, "rtcl", "fuel", g_fuel).unwrap();
    linker.define(&mut *store, "rtcl", "epoch", g_epoch).unwrap();
    linker.define(&mut *store, "rtcl", "caching", g_caching).unwrap();

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
    let caching_bin = g_caching;
    linker
        .func_wrap("rtcl", "binary_op", move |mut c: Caller<'_, Host>, op: i32, a: i64, b: i64| -> i64 {
            let _ = caching_bin.set(&mut c, Val::I32(0));
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
    let caching_call = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "call",
            move |mut c: Caller<'_, Host>, cmd_id: i32, site: i32, argc: i32| -> i64 {
                let _ = caching_call.set(&mut c, Val::I32(0));
                c.data_mut().call_raw(cmd_id, site, argc)
            },
        )
        .unwrap();
    let caching_cav = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "call_argv",
            move |mut c: Caller<'_, Host>, base: i32, argc: i32, site: i32| -> i64 {
                // 直连分派可能改动任意变量（lappend/upvar/自写 proc）——
                // 先废缓存（wasmi 版同款；wasmtime 注册起初漏了此项，
                // LoadVar 读回陈旧缓存值，lappend/upvar 差分实证）。
                let _ = caching_cav.set(&mut c, Val::I32(0));
                let words = read_words_wt(&mut c, base, argc);
                c.data_mut().call_argv_raw(&words, site)
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "set_result", |mut c: Caller<'_, Host>, v: i64| {
            c.data_mut().set_result_raw(v);
        })
        .unwrap();
    let fuel_g = g_fuel;
    linker
        .func_wrap("rtcl", "refuel", move |mut c: Caller<'_, Host>| -> i64 {
            let chunk = c.data().refuel_chunk();
            for _ in 0..chunk {
                if let Err(e) = rtcl_vm::VmContext::charge_step(c.data_mut().interp_mut()) {
                    c.data_mut().set_pending(e, false);
                    return crate::abi::ERR_SENTINEL;
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
            crate::abi::const_handle(idx as u32)
        })
        .unwrap();
    let caching_fs = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "foreach_start",
            move |mut c: Caller<'_, Host>, info: i32, base: i32, ngroups: i32| -> i64 {
                let _ = caching_fs.set(&mut c, Val::I32(0));
                let words = read_words_wt(&mut c, base, ngroups);
                c.data_mut().foreach_start_raw(info, &words)
            },
        )
        .unwrap();
    let caching_fb = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "foreach_bind",
            move |mut c: Caller<'_, Host>, info: i32, idx: i32| -> i64 {
                let _ = caching_fb.set(&mut c, Val::I32(0));
                c.data_mut().foreach_bind_raw(info, idx)
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "foreach_collect", |mut c: Caller<'_, Host>, v: i64| {
            c.data_mut().foreach_collect_raw(v);
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "foreach_end", |mut c: Caller<'_, Host>| -> i64 {
            c.data_mut().foreach_end_raw()
        })
        .unwrap();
    let caching_es = g_caching;
    linker
        .func_wrap("rtcl", "eval_script", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = caching_es.set(&mut c, Val::I32(0));
            c.data_mut().eval_script_raw(v)
        })
        .unwrap();
    let caching_ee = g_caching;
    linker
        .func_wrap("rtcl", "eval_expr", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = caching_ee.set(&mut c, Val::I32(0));
            c.data_mut().eval_expr_raw(v)
        })
        .unwrap();
    let caching_lap = g_caching;
    linker
        .func_wrap("rtcl", "lappend_var", move |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i64 {
            let _ = caching_lap.set(&mut c, Val::I32(0));
            c.data_mut().lappend_var_raw(name_h, v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "concat_n", |mut c: Caller<'_, Host>, base: i32, n: i32| -> i64 {
            let words = read_words_wt(&mut c, base, n);
            c.data_mut().concat_raw(&words)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "exit", |mut c: Caller<'_, Host>, code: i32| {
            c.data_mut().exit_raw(code);
        })
        .unwrap();

    let instance = linker
        .instantiate(&mut *store, wmodule)
        .expect("wasmtime: instantiate");
    (instance, (g_fuel, g_epoch, g_caching))
}

/// 从 wasmtime 实例 memory 读 n 个 u64（SCRATCH 协议；wasmi 版同款）。
pub(crate) fn read_words_wt(c: &mut wasmtime::Caller<'_, Host>, base: i32, n: i32) -> Vec<u64> {
    let n_words = n.max(0) as usize;
    let mut bytes = vec![0u8; n_words * 8];
    let mut words = vec![0u64; n_words];
    if let Some(wasmtime::Extern::Memory(m)) = c.get_export("memory") {
        let _ = m.read(&mut *c, base as usize, &mut bytes);
        for (i, w) in words.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
            *w = u64::from_le_bytes(b);
        }
    }
    words
}
