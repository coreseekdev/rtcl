//! Proc-body JIT executor（M2 seam 的 rtcl-jit 侧）。
//!
//! rtcl-vm 的进程级钩子（`ProcBodyExecutor`，vm_exec.rs）把 Site A 命中
//! 的编译体交到这里：epoch 门（M3 口径）→ `.unfused()` 展开 PR#2 融合
//! 超指令 → nanopass 管线发射 → wasmi 实例化执行 → Tcl result code
//! 映射回 `Result<Value, Error>`。任何一步不接受/不确定（编译拒绝、
//! epoch 不等）都返回 `None` 退回 `exec_bytecode`——宁可少 JIT 不可错。
//!
//! 错误协议：**不拼报告、不加 `(file …)` 帧**。proc 语境的错误帧由
//! call_proc 边界负责——宿主 eval 路由（命令/expr）的错误已在
//! `interp` 的累积器上（case a，call_proc 会追加 `(procedure …)`）；
//! 宿主原语错误（case b）的驱动侧补帧（harness/unwind）在 proc 语境
//! 的对齐由差分电池裁决（见 `bench/scripts/jit_proc_probes/`）。

/// 池 key 三元组（interp 指针, proc 体 Rc 指针, epoch）。
type PoolKey = (usize, usize, u64);
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
use crate::emit;
use crate::exec::Host;
use crate::pass::EmitConfig;
use rtcl_core::value::Value;
use rtcl_core::{Error, Interp};
use rtcl_ir::ByteCode;
use rtcl_vm::interp::set_proc_body_executor;
use std::rc::Rc;
use wasmi::Caller;

/// 注册 proc-body 钩子（进程级一次性；`jitsh` main 调用）。
pub fn install() {
    set_proc_body_executor(Some(exec_proc_body));
}

thread_local! {
    /// proc 体的发射产物缓存（M4「定义即编译」的雏形）：key =
    /// (ByteCode 的 Rc 指针, epoch)——指针稳定性由 ProcDef.compiled 的
    /// 长期 Rc 保证；redefine 换新 Rc 且 epoch 变化，双重防陈旧命中。
    /// 上限 64 条，超出全清（进程级模块数有限；简单粗暴即可）。
    static PROC_MODULES: std::cell::RefCell<
        std::collections::HashMap<(usize, u64), Rc<emit::Module>>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    /// wasmi **编译产物**缓存 + **Engine 单例**（Module/Linker 绑
    /// Engine——每次 default() 让缓存跨引擎失效）。Store/Host/Instance
    /// **每次新建**：wasmi Store 复用有状态残留（同 Store 第二次 run
    /// 提前 OK 退出——defarg 双调用实证），正确性优先。
    static WASMI_MODULES: std::cell::RefCell<
        std::collections::HashMap<(usize, usize, u64), wasmi::Module>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    static WASMI_ENGINE: std::cell::OnceCell<wasmi::Engine> = const { std::cell::OnceCell::new() };
    /// Store/Instance **池**（key 三元组）：复用免 instantiate——状态
    /// 残留由 **完整重置**（globals + memory cache 区 + host 状态）修复。
    static PROC_POOL: std::cell::RefCell<
        Vec<(
            PoolKey,
            wasmi::Store<Host>,
            wasmi::Instance,
            wasmi::TypedFunc<(), i32>,
        )>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

/// 池 key 的 epoch 分量（redefine 换 Rc + epoch 变化——双重防陈旧）。
fn code_epoch_key(bc: &ByteCode) -> u64 {
    bc.epoch
}

/// 钩子体：接受则 JIT 执行，`None` 退回 vm_exec。
fn exec_proc_body(interp: &mut Interp, code: &Rc<ByteCode>) -> Option<rtcl_core::Result<Value>> {
    // epoch 门（M3 口径——Site A 的 guard 已查过，此处防御性重查：
    // 钩子与门之间的窗口内可能发生 Tier1 bump）。
    if code.epoch != interp.tier1_epoch() {
        return None;
    }
    // 引擎分派（RTCL_JIT_ENGINE；默认 wasmi）。wasmtime 走 Cranelift
    // 池化路径（Engine 单例 + Module 缓存 + Store/Instance 池）。
    #[cfg(feature = "jit-native")]
    if crate::exec::engine_from_env() == crate::exec::EngineKind::Wasmtime {
        // 池优先：命中 = 该单元此前已被接受（池条目只在编译成功后
        // 存在）——免 PROC_MODULES/WASMI_MODULES 两次哈希查找
        //（fib 差分剖析：每调用 ~80ns 的纯开销）。
        if let Some(r) = execute_proc_wasmtime_pooled(interp, code) {
            return Some(r);
        }
        let key = (Rc::as_ptr(code) as usize, code.epoch);
        let module = compile_cached(code, key)?;
        return Some(execute_proc_wasmtime(interp, code, &module));
    }
    // wasmi 池优先（同上——命中路径零模块查找）。
    if let Some(r) = execute_proc_pooled(interp, code) {
        return Some(r);
    }
    let key = (Rc::as_ptr(code) as usize, code.epoch);
    let module = compile_cached(code, key)?;
    // proc 语境的 case-b 帧协议已落地（rtcl-vm jit_err_* 公共入口 +
    // Host.proc_ctx 路由）——不再按 Cold 退回（f002196/f004860 的
    // (procedure …) 帧链由差分验证）。
    Some(execute_proc(interp, code, &module))
}

/// emit 编译缓存（key 二元组；未命中现编译，拒绝不缓存负项）。
fn compile_cached(code: &Rc<ByteCode>, key: (usize, u64)) -> Option<Rc<emit::Module>> {
    PROC_MODULES.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(m) = c.get(&key) {
            return Some(Rc::clone(m));
        }
        // PR#2 融合超指令展开（BEGIN_CMD_PUSH_CONST / FOREACH_NEXT_CLOSE
        // / CMP_VI_JF …）——JIT 消费 unfused 规范形状（run_top_with
        // 同款）。
        let bc = code.unfused();
        match emit::compile(&bc, &EmitConfig::default()) {
            Ok(mm) => {
                if c.len() >= 64 {
                    c.clear();
                }
                let m = Rc::new(mm);
                c.insert(key, Rc::clone(&m));
                Some(m)
            }
            Err(_) => None,
        }
    })
}

/// 实例化并执行 proc 体，映射 `run()` 返回码。
/// 池快路径：仅当池命中时执行（命中 = 单元此前被接受——免模块查找）。
/// 未命中返回 None，由冷路径（execute_proc）建 Store 入池。
fn execute_proc_pooled(interp: &mut Interp, bc: &Rc<ByteCode>) -> Option<rtcl_core::Result<Value>> {
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        code_epoch_key(bc),
    );
    let pooled = PROC_POOL.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|(k, ..)| *k == pool_key).map(|pos| p.remove(pos))
    })?;
    let (_, mut store, instance, run) = pooled;
    pool_reset(&mut store);
    Some(run_and_decode(interp, bc, store, instance, run, pool_key))
}

/// 池命中重置（裁剪版）。语义依据：
/// - **fuel 免 set**：fuel 只在回边扣减、耗尽 refuel——剩余预算让下一
///   调用更晚触发 refuel，语义无害（charge_step 的步数预算才是防线）。
/// - **epoch 免 set**：池 key 含 epoch 且入口门刚比对过 code.epoch ==
///   interp.tier1_epoch()——global 里就是同值。
/// - **caching 必设 1**：cache 区（VALUES/STATE）清零后重新武装。
/// - **memory 清零裁剪到实际 extent**：只清 [0, 512+vars)（VALUES
///   槽 + STATE 字节；vars 数 emit 期随池条目存档）——SCRATCH 区
///   先写后读无跨调用语义。1024B→~513B（fib 1 变量）。
/// 池命中重置（wasm 序言承担 cache 清零 + caching=1——每 var 2 条
/// store ~2ns vs 宿主 memory.write ~80ns；fuel/epoch 免 set 依据见
/// run_and_decode 注释）。宿主侧只剩 host 状态清理。
fn pool_reset(store: &mut wasmi::Store<Host>) {
    store.data_mut().reset_for_reuse();
}

/// run + 结果解码 + 还池（池快路径与冷路径共用）。
fn run_and_decode(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
    mut store: wasmi::Store<Host>,
    instance: wasmi::Instance,
    run: wasmi::TypedFunc<(), i32>,
    pool_key: PoolKey,
) -> rtcl_core::Result<Value> {
    let watermark = store.data().arena_len();

    // run 句柄随池条目缓存——免每调用的字符串导出查找（fib 差分
    // 实证 ~0.1µs/调用）。
    // ⑧ 重入哨兵：run 窗口内 wasm32 宿主的重入求值被明确拒绝。
    rtcl_vm::interp::set_jit_call_in_flight(true);
    let rc = run.call(&mut store, ()).unwrap_or_else(|trap| {
        rtcl_vm::interp::set_jit_call_in_flight(false);
        panic!("jit proc: wasm trap: {trap:?}")
    });
    rtcl_vm::interp::set_jit_call_in_flight(false);
    store.data_mut().arena_truncate(watermark);

    // 读取结果 → Store/Instance 还池。
    let mut result = store.data_mut().result.take();
    let pending_exit = store.data_mut().pending_exit.take();
    let pending = store.data_mut().pending.take();
    let stale_site = store.data_mut().stale_site;
    PROC_POOL.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() >= 64 {
            p.clear();
        }
        p.push((pool_key, store, instance, run));
    });

    match rc {
        code::OK => Ok(result.unwrap_or_else(Value::empty)),
        code::RETURN => Err(Error::ret(result)),
        code::BREAK => Err(Error::brk()),
        code::CONTINUE => Err(Error::cont()),
        // ④ 中途 bump：从 bump 命令的下一条顶层命令树走查重分派
        //(proc 语境的帧/命名空间/行号已由 call_proc 就位——上下文
        // 保持由 eval_commands_from 保证）。非边界 bump 维持错误现状。
        code::STALE => {
            let r = stale_site
                .and_then(|site| interp.jit_redispatch_after(bc, site));
            match r {
                Some((true, res)) => res,
                Some((false, _)) => Ok(result.take().unwrap_or_else(Value::empty)),
                None => Err(Error::Msg("jit proc: stale epoch at non-boundary".into())),
            }
        }
        code::EXIT => Err(Error::exit(Some(pending_exit.unwrap_or(0)))),
        _ => match pending {
            Some(p) => Err(p.err),
            None => Err(Error::Msg("jit proc: failed without pending error".into())),
        },
    }
}

/// 冷路径：建 Store/Host/Instance 入池后执行。
fn execute_proc(interp: &mut Interp, bc: &Rc<ByteCode>, module: &emit::Module) -> rtcl_core::Result<Value> {
    use wasmi::{Engine, Module, Store};

    let epoch0 = interp.tier1_epoch() as i64;
    // key 三元组：interp 指针 + **proc 体 Rc 指针** + epoch——只有
    // epoch 会让不同 proc（同 interp 同 epoch）撞 key（B 的调用命中
    // A 的 Store/Instance——judge gen_oo/gen_proc 大面积回归的根因）。
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        code_epoch_key(bc),
    );

    // Engine 单例 + Module 编译缓存（key 三元组）。
    let engine = WASMI_ENGINE.with(|e| e.get_or_init(Engine::default).clone());
    let wmodule = WASMI_MODULES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(w) = m.get(&pool_key) {
            return w.clone();
        }
        let w = match Module::new(&engine, &module.bytes[..]) {
            Ok(m) => m,
            Err(e) => {
                if std::env::var_os("JIT_DUMP_MOD").is_some() {
                    std::fs::write("/tmp/jitdbg/failmod.wasm", &module.bytes[..]).unwrap();
                }
                panic!("jit proc: module: {e:?}");
            }
        };
        if m.len() >= 64 {
            m.clear();
        }
        m.insert(pool_key, w.clone());
        w
    });
    // 池条目只在冷路径创建（池快路径未命中）——取走即可（position
    // 恒 miss，remove 兜底防御重复）。
    let pooled = PROC_POOL.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|(k, ..)| *k == pool_key).map(|pos| p.remove(pos))
    });
    let (_, mut store, instance, run) = match pooled {
        Some(entry) => entry,
        None => {
            let mut host = Host::new(
                interp as *mut Interp,
                crate::exec::arena_of(bc),
                bc,
                module,
            );
            host.proc_ctx = true;
            let mut store = Store::new(&engine, host);
            let inst = register_and_instantiate(&mut store, epoch0, &wmodule);
            let runf = inst
                .get_typed_func::<(), i32>(&store, "run")
                .expect("run export");
            (pool_key, store, inst, runf)
        }
    };
    pool_reset(&mut store);
    run_and_decode(interp, bc, store, instance, run, pool_key)
}

thread_local! {
    /// execute_proc 内部的 Store 交接槽（单线程）。
    static POOL_TAKE: std::cell::RefCell<
        Option<(wasmi::Store<Host>, wasmi::Instance)>,
    > = const { std::cell::RefCell::new(None) };
}

/// imports 注册 + 实例化（proc 语境；与 exec.rs 同款宿主闭包）。
fn register_and_instantiate(store: &mut wasmi::Store<Host>, epoch0: i64, wmodule: &wasmi::Module) -> wasmi::Instance {
    use wasmi::{Caller, Extern, Linker};

    let mut linker = <Linker<Host>>::new(store.engine());

    let g_fuel = wasmi::Global::new(&mut *store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_epoch = wasmi::Global::new(&mut *store, wasmi::Val::I64(epoch0), wasmi::Mutability::Var);
    let g_caching = wasmi::Global::new(&mut *store, wasmi::Val::I32(1), wasmi::Mutability::Var);
    linker.define("rtcl", "fuel", Extern::Global(g_fuel)).unwrap();
    linker.define("rtcl", "epoch", Extern::Global(g_epoch)).unwrap();
    linker.define("rtcl", "caching", Extern::Global(g_caching)).unwrap();
    // fast 模块附加导入 stub（经常规池运行时 proc_main 不被调用）。
    {
        let mem = wasmi::Memory::new(&mut *store, wasmi::MemoryType::new(1, None)).unwrap();
        linker.define("rtcl", "memory", Extern::Memory(mem)).unwrap();
        let tbl = wasmi::Table::new(
            &mut *store,
            wasmi::TableType::new(wasmi::RefType::Func, 256, None),
            wasmi::Ref::Func(wasmi::Nullable::Null),
        ).unwrap();
        linker.define("rtcl", "funs", Extern::Table(tbl)).unwrap();
        let g_cur = wasmi::Global::new(&mut *store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "cur_proc", Extern::Global(g_cur)).unwrap();
        let g_scb = wasmi::Global::new(&mut *store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "site_cache_base", Extern::Global(g_scb)).unwrap();
        let g_fd = wasmi::Global::new(&mut *store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "fast_depth", Extern::Global(g_fd)).unwrap();
    }
    linker.func_wrap("rtcl", "resolve_proc", |_s: i32, _a: i32, _b: i32| -> i64 {
        unreachable!("resolve_proc stub (regular store)")
    }).unwrap();
    linker.func_wrap("rtcl", "proc_exit_frame", |_slot: i32| {}).unwrap();

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
            let _ = caching_bin.set(&mut c, wasmi::Val::I32(0));
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
                let _ = caching_call.set(&mut c, wasmi::Val::I32(0));
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
                let _ = caching_cav.set(&mut c, wasmi::Val::I32(0));
                let words = read_words(&mut c, base, argc);
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
                    return abi::ERR_SENTINEL;
                }
            }
            let _ = fuel_g.set(&mut c, wasmi::Val::I64(chunk));
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
    let caching_fs = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "foreach_start",
            move |mut c: Caller<'_, Host>, info: i32, base: i32, ngroups: i32| -> i64 {
                let _ = caching_fs.set(&mut c, wasmi::Val::I32(0));
                let words = read_words(&mut c, base, ngroups);
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
                let _ = caching_fb.set(&mut c, wasmi::Val::I32(0));
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
            let _ = caching_es.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_script_raw(v)
        })
        .unwrap();
    let caching_ee = g_caching;
    linker
        .func_wrap("rtcl", "eval_expr", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = caching_ee.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_expr_raw(v)
        })
        .unwrap();
    let caching_lap = g_caching;
    linker
        .func_wrap("rtcl", "lappend_var", move |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i64 {
            let _ = caching_lap.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().lappend_var_raw(name_h, v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "concat_n", |mut c: Caller<'_, Host>, base: i32, n: i32| -> i64 {
            let words = read_words(&mut c, base, n);
            c.data_mut().concat_raw(&words)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "exit", |mut c: Caller<'_, Host>, code: i32| {
            c.data_mut().exit_raw(code);
        })
        .unwrap();

    linker
        .instantiate_and_start(store, wmodule)
        .unwrap_or_else(|e| panic!("jit proc: instantiate failed: {e}"))
}


/// 从实例 memory 读 n 个 u64（暂存区协议；与 exec.rs 同款）。
fn read_words(c: &mut Caller<'_, Host>, base: i32, n: i32) -> Vec<u64> {
    let n_words = n.max(0) as usize;
    let mut bytes = vec![0u8; n_words * 8];
    let mut words = vec![0u64; n_words];
    if let Some(wasmi::Extern::Memory(m)) = c.get_export("memory") {
        let _ = m.read(c, base as usize, &mut bytes);
        for (i, w) in words.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
            *w = u64::from_le_bytes(b);
        }
    }
    words
}

// ── wasmtime proc 路径（阶段 3：Cranelift + 池化冲 1×）────────────────

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

/// 池快路径（wasmtime）：仅命中时执行——免模块查找（同 wasmi 池优先）。
#[cfg(feature = "jit-native")]
fn execute_proc_wasmtime_pooled(
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
    use wasmtime::TypedFunc;

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
fn execute_proc_wasmtime(
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

