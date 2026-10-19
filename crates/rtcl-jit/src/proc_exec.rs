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
    /// wasmi 实例池：key = (interp 指针, 模块 key)。fib 类深递归每次
    /// 调用 instantiate 是 .24×→.47× 后的剩余大头（vm_exec 有 VmPool
    /// 复用而 JIT 没有）——Store+Instance 成对缓存（fuel/epoch/caching
    /// global 状态在 Store 内持续，语义一致；Host 状态 run 前 reset）。
    /// 单线程顺序执行；池上限 = 递归深度（64），超出全清。
    static WASMI_POOL: std::cell::RefCell<
        Vec<((usize, usize, u64), wasmi::Store<Host>, wasmi::Instance)>,
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
    let key = (Rc::as_ptr(code) as usize, code.epoch);
    let module = PROC_MODULES.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(m) = c.get(&key) {
            return Some(Rc::clone(m));
        }
        // PR#2 融合超指令展开（BEGIN_CMD_PUSH_CONST / FOREACH_NEXT_CLOSE
        // / CMP_VI_JF …）——JIT 消费 unfused 规范形状（run_top_with
        // 同款）。编译拒绝不缓存负项（退回 vm_exec，重试成本低）。
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
    })?;
    // proc 语境的 case-b 帧协议已落地（rtcl-vm jit_err_* 公共入口 +
    // Host.proc_ctx 路由）——不再按 Cold 退回（f002196/f004860 的
    // (procedure …) 帧链由差分验证）。
    CURRENT_MODULE_BYTES.with(|b| *b.borrow_mut() = module.bytes.clone());
    Some(execute_proc(interp, code, &module))
}

/// 实例化并执行 proc 体，映射 `run()` 返回码。
fn execute_proc(interp: &mut Interp, bc: &Rc<ByteCode>, module: &emit::Module) -> rtcl_core::Result<Value> {
    use wasmi::{Caller, Engine, Extern, Linker, Module, Store, TypedFunc};

    let epoch0 = interp.tier1_epoch() as i64;
    // key 三元组：interp 指针 + **proc 体 Rc 指针** + epoch——只有
    // epoch 会让不同 proc（同 interp 同 epoch）撞 key（B 的调用命中
    // A 的 Store/Instance——judge gen_oo/gen_proc 大面积回归的根因）。
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        code_epoch_key(bc),
    );

    // 池命中：reset Host 状态 + 复用 Store/Instance（免 instantiate——
    // vm_exec 有 VmPool 复用而 JIT 每次重建是 .47× 的剩余大头）。
    let mut reused = false;
    WASMI_POOL.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(pos) = p.iter().position(|(k, ..)| *k == pool_key) {
            let (_, mut store, inst) = p.remove(pos);
            store.data_mut().reset_for_reuse();
            // 把 Store/Instance 经 thread_local 传给本函数继续用：
            // 单线程——借用 thread_local 槽位代替跨作用域移动。
            POOL_TAKE.with(|t| *t.borrow_mut() = Some((store, inst)));
            reused = true;
        }
    });
    let (mut store, instance) = if reused {
        POOL_TAKE.with(|t| t.borrow_mut().take().expect("pool take"))
    } else {
        let mut host = Host::new(
            interp as *mut Interp,
            crate::exec::arena_of(bc),
            bc,
            module,
        );
        host.proc_ctx = true;
        let engine = Engine::default();
        let wmodule = Module::new(&engine, &module.bytes[..]).expect("jit proc: module");
        let mut store = Store::new(&engine, host);
        // 注册 imports + instantiate（与 exec.rs 同款宿主闭包）。
        let instance = register_and_instantiate(&mut store, epoch0);
        (store, instance)
    };
    let watermark = store.data().arena_len();

    let run: TypedFunc<(), i32> = instance
        .get_typed_func(&store, "run")
        .expect("run export");
    let rc = run.call(&mut store, ()).unwrap_or_else(|trap| {
        panic!("jit proc: wasm trap: {trap:?}")
    });
    store.data_mut().arena_truncate(watermark);

    // 读取结果（借用）→ Store/Instance 还池 → 组装返回。
    let result = store.data_mut().result.take();
    let pending_exit = store.data_mut().pending_exit.take();
    let pending = store.data_mut().pending.take();
    WASMI_POOL.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() >= 64 {
            p.clear();
        }
        p.push((pool_key, store, instance));
    });

    match rc {
        code::OK => Ok(result.unwrap_or_else(Value::empty)),
        code::RETURN => Err(Error::ret(result)),
        code::BREAK => Err(Error::brk()),
        code::CONTINUE => Err(Error::cont()),
        code::STALE => Err(Error::Msg("jit proc: stale epoch (unexpected)".into())),
        code::EXIT => Err(Error::exit(Some(pending_exit.unwrap_or(0)))),
        _ => match pending {
            Some(p) => Err(p.err),
            None => Err(Error::Msg("jit proc: failed without pending error".into())),
        },
    }
}

thread_local! {
    /// register_and_instantiate 的模块字节交接（execute_proc 设置）。
    static CURRENT_MODULE_BYTES: std::cell::RefCell<Vec<u8>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// execute_proc 内部的 Store 交接槽（单线程；池命中路径把
    /// Store/Instance 从 WASMI_POOL 移交进作用域）。
    static POOL_TAKE: std::cell::RefCell<
        Option<(wasmi::Store<Host>, wasmi::Instance)>,
    > = const { std::cell::RefCell::new(None) };
}

/// imports 注册 + 实例化（proc 语境；与 exec.rs 同款宿主闭包）。
fn register_and_instantiate(store: &mut wasmi::Store<Host>, epoch0: i64) -> wasmi::Instance {
    use wasmi::{Caller, Engine, Extern, Linker, Module, TypedFunc};

    let wmodule = {
        // Module::new 需要 Engine——从 store 取。
        let engine = store.engine().clone();
        let bytes = CURRENT_MODULE_BYTES.with(|b| b.borrow().clone());
        Module::new(&engine, &bytes).expect("jit proc: module")
    };
    let mut linker = <Linker<Host>>::new(store.engine());

    let g_fuel = wasmi::Global::new(&mut *store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_epoch = wasmi::Global::new(&mut *store, wasmi::Val::I64(epoch0), wasmi::Mutability::Var);
    let g_caching = wasmi::Global::new(&mut *store, wasmi::Val::I32(1), wasmi::Mutability::Var);
    linker.define("rtcl", "fuel", Extern::Global(g_fuel.clone())).unwrap();
    linker.define("rtcl", "epoch", Extern::Global(g_epoch.clone())).unwrap();
    linker.define("rtcl", "caching", Extern::Global(g_caching.clone())).unwrap();

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
    let caching_bin = g_caching.clone();
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
    let caching_call = g_caching.clone();
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
    linker
        .func_wrap(
            "rtcl",
            "call_argv",
            |mut c: Caller<'_, Host>, base: i32, argc: i32, site: i32| -> i64 {
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
    let fuel_g = g_fuel.clone();
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
    let caching_fs = g_caching.clone();
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
    let caching_fb = g_caching.clone();
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
    let caching_es = g_caching.clone();
    linker
        .func_wrap("rtcl", "eval_script", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = caching_es.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_script_raw(v)
        })
        .unwrap();
    let caching_ee = g_caching.clone();
    linker
        .func_wrap("rtcl", "eval_expr", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = caching_ee.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_expr_raw(v)
        })
        .unwrap();
    let caching_lap = g_caching.clone();
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
        .instantiate_and_start(store, &wmodule)
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
