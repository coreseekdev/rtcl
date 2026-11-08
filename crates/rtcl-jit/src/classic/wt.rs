//! classic proc 池（wasmtime）：Cranelift 模块缓存 + Store/Instance
//! 池 + 池命中完整重置。

/// (fuel, epoch, caching) global 句柄三元组（池命中重置用）。
#[cfg(feature = "jit-native")]
type WtGlobals = (wasmtime::Global, wasmtime::Global, wasmtime::Global);
/// wasmtime 池条目：key + Store + Instance + global 句柄 + run 句柄 +
/// cache 区 extent（512+vars）。
#[cfg(feature = "jit-native")]
type WtPoolEntry = (
    PoolKey,
    wasmtime::Store<Host>,
    wasmtime::Instance,
    WtGlobals,
    wasmtime::TypedFunc<(), i32>,
);

use crate::abi::{self, code};
use crate::classic::PoolKey;
use crate::classic::wasmi::code_epoch_key;
use crate::fast::shared::{
    sh_binary_op, sh_bool_test, sh_call_dispatch, sh_fail, sh_get_var,
    sh_incr_var, sh_proc_exit_frame, sh_rc_to_i64, sh_set_var, sh_sync_var,
};
use crate::emit;
use crate::exec::Host;
use crate::pass::EmitConfig;
use rtcl_core::value::Value;
use rtcl_core::{Error, Interp};
use rtcl_ir::ByteCode;
use rtcl_vm::interp::set_proc_body_executor;
use std::rc::Rc;
use wasmi::Caller;

thread_local! {
    /// Cranelift Module 缓存（key 三元组同 wasmi 池）。
    #[cfg(feature = "jit-native")]
    static WT_MODULES: std::cell::RefCell<
        std::collections::HashMap<(usize, usize, u64), wasmtime::Module>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    /// wasmtime Store+Instance+globals 池（key 三元组；上限 64）。globals
    /// 句柄随池条目保存——池命中必须**完整重置**（fuel=0 / epoch=本代 /
    /// caching=1 + 清 memory 缓存区 + reset_for_reuse），与 wasmi 路径
    /// 同款；只做 reset_for_reuse 会把上一调用的 epoch/caching/缓存值
    /// 带进来（catch_error 差分实证：第二调用读到空变量值）。
    #[cfg(feature = "jit-native")]
    static WT_POOL: std::cell::RefCell<Vec<WtPoolEntry>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Drop cached wasmtime modules/pool entries belonging to a dead Interp.
#[cfg(feature = "jit-native")]
pub(crate) fn teardown_interp(ptr: usize) {
    WT_MODULES.with(|m| m.borrow_mut().retain(|k, _| k.0 != ptr));
    WT_POOL.with(|p| p.borrow_mut().retain(|(k, ..)| k.0 != ptr));
}

#[cfg(not(feature = "jit-native"))]
pub(crate) fn teardown_interp(_ptr: usize) {}

/// 池快路径（wasmtime）：仅命中时执行——免模块查找（同 wasmi 池优先）。
#[cfg(feature = "jit-native")]
pub(crate) fn execute_proc_wasmtime_pooled(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
) -> Option<rtcl_core::Result<Value>> {
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        code_epoch_key(bc),
    );
    let pooled = WT_POOL.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|(k, ..)| *k == pool_key).map(|pos| p.remove(pos))
    })?;
    let (_, mut store, instance, globals, run) = pooled;
    pool_reset_wt(&mut store);
    Some(run_and_decode_wt(interp, bc, store, instance, globals, run, pool_key))
}

/// 池命中重置（wasmtime；wasm 序言承担 cache 清零 + caching=1）。
#[cfg(feature = "jit-native")]
fn pool_reset_wt(store: &mut wasmtime::Store<Host>) {
    store.data_mut().reset_for_reuse();
}

/// run + 结果解码 + 还池（wasmtime；池快路径与冷路径共用）。
#[cfg(feature = "jit-native")]
#[allow(clippy::too_many_arguments)]
fn run_and_decode_wt(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
    mut store: wasmtime::Store<Host>,
    instance: wasmtime::Instance,
    globals: WtGlobals,
    run: wasmtime::TypedFunc<(), i32>,
    pool_key: PoolKey,
) -> rtcl_core::Result<Value> {


    let watermark = store.data().arena_len();

    rtcl_vm::interp::set_jit_call_in_flight(true);
    let rc = run.call(&mut store, ()).expect("jit proc wt: run");
    rtcl_vm::interp::set_jit_call_in_flight(false);
    store.data_mut().arena_truncate(watermark);

    let mut result = store.data_mut().result.take();
    let pending_exit = store.data_mut().pending_exit.take();
    let pending = store.data_mut().pending.take();
    let stale_site = store.data_mut().stale_site;
    WT_POOL.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() >= 64 {
            p.clear();
        }
        p.push((pool_key, store, instance, globals, run));
    });

    match rc {
        code::OK => Ok(result.unwrap_or_else(Value::empty)),
        code::RETURN => Err(Error::ret(result)),
        code::BREAK => Err(Error::brk()),
        code::CONTINUE => Err(Error::cont()),
        // ④ 中途 bump 重分派（wasmi proc 路径同款）。
        code::STALE => {
            let r = stale_site
                .and_then(|site| interp.jit_redispatch_after(bc, site));
            match r {
                Some((true, res)) => res,
                Some((false, _)) => Ok(result.take().unwrap_or_else(Value::empty)),
                None => Err(Error::Msg("jit proc wt: stale epoch at non-boundary".into())),
            }
        }
        code::EXIT => Err(Error::exit(Some(pending_exit.unwrap_or(0)))),
        _ => match pending {
            Some(p) => Err(p.err),
            None => Err(Error::Msg("jit proc wt: failed without pending".into())),
        },
    }
}

/// 冷路径（wasmtime）：建 Store/Host/Instance 入池后执行。
#[cfg(feature = "jit-native")]
pub(crate) fn execute_proc_wasmtime(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
    module: &emit::Module,
) -> rtcl_core::Result<Value> {
    use wasmtime::Store;

    let epoch0 = interp.tier1_epoch() as i64;
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        code_epoch_key(bc),
    );
    let mut host = Host::new(
        interp as *mut Interp,
        crate::exec::arena_of(bc),
        bc,
        module,
    );
    host.proc_ctx = true;
    let wmodule = WT_MODULES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(w) = m.get(&pool_key) {
            return w.clone();
        }
        let engine = store_engine();
        let w = wasmtime::Module::new(&engine, &module.bytes[..])
            .expect("jit proc wt: module");
        if m.len() >= 64 {
            m.clear();
        }
        m.insert(pool_key, w.clone());
        w
    });
    let engine = store_engine();
    let mut store = Store::new(&engine, host);
    let (instance, globals) = register_wasmtime_proc(&mut store, epoch0, &wmodule);
    let run = instance
        .get_typed_func(&mut store, "run")
        .expect("run export");
    pool_reset_wt(&mut store);
    run_and_decode_wt(interp, bc, store, instance, globals, run, pool_key)
}

#[cfg(feature = "jit-native")]
fn store_engine() -> wasmtime::Engine {
    crate::exec_wasmtime::wt_engine()
}

/// wasmtime 的 proc 语境 ABI 注册（与 exec_wasmtime 顶层路径共用同一
/// 注册体）。
#[cfg(feature = "jit-native")]
fn register_wasmtime_proc(
    store: &mut wasmtime::Store<Host>,
    epoch0: i64,
    wmodule: &wasmtime::Module,
) -> (
    wasmtime::Instance,
    (wasmtime::Global, wasmtime::Global, wasmtime::Global),
) {
    crate::exec_wasmtime::register_wasmtime_abi(store, epoch0, wmodule)
}


// ── fast-tier 驱动：共享 Store + funcref 表 + call_indirect 直呼 ──────────
//
// MoonBit 式热路径零宿主过渡（adb032c 的 emit 侧契约）：
// - 全部 fast 实例驻一个共享 Store（funcref 不跨 Store）；共享
//   memory/table/fuel/epoch/caching/cur_proc/fast_depth，per-instance 仅
//   site_cache_base（每实例一段 site 槽缓存）
// - FastHost = per-proc Host 表（复用全部 Host 方法）+ **共享** pending/
//   stale_site（callee 置 pending 于 cur=callee 时刻，caller 的 fail 在
//   cur=caller 读——per-host 存储即丢）
// - epoch 失效协议：impure 闭包（call_argv/resolve）顺带刷新共享 epoch
//   global；CallSite 的 epoch 门在 wasm 内比对编译期值 → 失配走回落
//   （内含 ④ STALE 机制）
