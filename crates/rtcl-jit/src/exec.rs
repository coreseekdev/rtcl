//! JIT 执行器（feature `jit-exec`，wasmi 引擎）：ABI v2 宿主实现 + 顶层
//! 单元驱动。宿主全部经 rtcl-core/rtcl-vm 公共 API 路由——语义对齐的
//! 三条锚：
//!
//! - **命令**（`rtcl.call`）：`interp.eval("\n"*(L-1) + site_text)`——
//!   原文经解释器全链路执行（dispatch/错误帧/步数/trace 全在 interp），
//!   换行前缀使 harness 帧的行号与 vm_exec 的 `entry_offset+site.line`
//!   相同。字节级等价由构造保证。
//! - **变量/算术兜底**（`get_var`/`set_var`/`incr_var`/`binary_op`/
//!   `bool_test`）：`read_var`/`set_var`/`incr_var` 与
//!   `types::eval_expr`（带 canonical 引用的原文嵌入）——vm_exec 同款
//!   入口。守卫失败 = 调宿主 full-semantics import（「编译仍全量、
//!   运行时快慢分流」，§2）。
//! - **错误协议**：宿主 eval 错误（case a）→ `interp.error_report` 出
//!   报告；快车道宿主错误（case b）→ 驱动机械复刻 `err_harness_frame`
//!   /`unwind_subs`/`error_report` 的字符串协议（fail 表的静态
//!   cur_site/bodies 上下文）。
//!
//! 变量缓存（wasm 内存 + dirty 位）：**仅宿主 `rtcl.call` 之前允许**
//! （新进程不变量：首次命令调用之前不可能存在 trace/upvar/链接——
//! 首次 `rtcl.call` 时宿主清零 `caching` global，此后全部走宿主）。

// 本文件 Host 持 *mut Interp（M2 seam 的所有权交换）；unsafe 收敛于 irp()。
#![allow(unsafe_code)]

use crate::abi::{self, code, BinOp};
use crate::emit;
use crate::ir::FailInfo;
use crate::pass::EmitConfig;
use rtcl_core::value::Value;
use rtcl_core::Error;
use rtcl_core::Interp;
use rtcl_parser::{ByteCode, Compiler, ScriptUnit};
use rtcl_vm::VmContext;
use std::rc::Rc;

pub(crate) use crate::host::Host;
pub(crate) use crate::host::{Pending, ForeachHost, STALE_MARK, arena_of, classify_control};

/// 顶层执行结果。
pub enum Outcome {
    /// 单元未被接受（编译拒绝）——调用方走 `interp.eval`（保证字节等价）。
    Fallback,
    /// `run()` 的 Tcl result code + 挂起错误的处置提示。
    Completed { code: i32, exit_code: Option<i32> },
}

/// 宿主侧挂起错误：`from_eval` = 来自 `rtcl.call` 的 eval（解释器已
/// 累积 errorInfo——case a）；否则快车道宿主原语错误（case b）。
/// 执行引擎选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineKind {
    /// wasmi（in-process 纯 Rust；judge/bench 默认）。
    #[default]
    Wasmi,
    /// wasmtime（native JIT；feature `jit-native`）。
    Wasmtime,
}

/// 顶层驱动：接受则 wasm 执行，否则 Fallback。
pub struct JitRun {
    pub outcome: Outcome,
    /// case b 的终局报告（jitsh 打印）；case a 由调用方走 error_report。
    pub driver_report: Option<String>,
    /// case a 的错误（moved out 供 error_report）。
    pub eval_error: Option<Error>,
    /// run() 的 result 值（set_result）。
    pub result: Option<Value>,
}

pub fn run_top(interp: Interp, script: &str, file: &str) -> (Interp, JitRun) {
    run_top_with(interp, script, file, engine_from_env())
}

/// `RTCL_JIT_ENGINE=wasmtime|wasmi`（默认 wasmi）。
pub fn engine_from_env() -> EngineKind {
    match crate::env_value("RTCL_JIT_ENGINE").as_deref() {
        Some("wasmtime") if cfg!(feature = "jit-native") => EngineKind::Wasmtime,
        Some("wasmi") => EngineKind::Wasmi,
        // 默认引擎：jit-native 在位时选 wasmtime——Cranelift 原生编译
        // vs wasmi 指令解释（fib 5.0×/loop 1.7× vs wasmi 路径，实测
        // 2026-11-01）；纯 jit-exec 构建（rtcl-cli）无 wasmtime 依赖，
        // 保持 wasmi。RTCL_JIT_ENGINE 显式指定恒优先。
        _ => {
            #[cfg(feature = "jit-native")]
            {
                EngineKind::Wasmtime
            }
            #[cfg(not(feature = "jit-native"))]
            {
                EngineKind::Wasmi
            }
        }
    }
}

pub fn run_top_with(
    interp: Interp,
    script: &str,
    file: &str,
    engine: EngineKind,
) -> (Interp, JitRun) {
    let unit = match ScriptUnit::parse(script) {
        Ok(u) => u,
        Err(_) => {
            return (interp, JitRun {
                outcome: Outcome::Fallback,
                driver_report: None,
                eval_error: None,
                result: None,
            })
        }
    };
    // 解释器原生超指令（rtcl-ir 的 fuse_pairs）先展开回规范窗口：JIT
    // 接受集/降级路径无需跟进修补（口径与替代方案见
    // docs/JIT-OPCODE-DELTA.md；将来做原生 lower 时删掉本展开即可）。
    let bc = Compiler::compile_unit(Rc::clone(&unit.source), &unit.commands).unfused();
    // M3 入口失效 gate —— 与 rtcl-vm eval.rs 的 bytecode_cache gate
    // 逐条对齐（!fallback 由 emit::compile 的拒绝路径等价承担）：
    // 单元编译时代的 `epoch` != 解释器当前 tier1_epoch ⇒ Tier1 折叠
    // 已被中途的命令注册/改名/删除（proc set、rename、宿主注册……）
    // 作废，整单元回解释器（Fallback ⇒ interp.eval ⇒ 同一款 gate ⇒
    // 树遍历）。vm_exec 的语义是仅单元入口检查（同单元内 bump 不
    // 重 gate——e1/e2 探针口径），本 gate 同样只在该入口比较一次。
    // 现场编译的单元 epoch 恒 0（ByteCode 默认；proc body 由
    // compile_proc_body 盖章），故 bump 后本 gate 恒失败——与
    // vm_exec 的「bump 后顶层 eval 全走树遍历」行为严格一致。
    if bc.epoch != interp.tier1_epoch() {
        return (interp, JitRun {
            outcome: Outcome::Fallback,
            driver_report: None,
            eval_error: None,
            result: None,
        });
    }
    let module = match emit::compile(&bc, &EmitConfig::default()) {
        Ok(m) => m,
        Err(_) => {
            return (interp, JitRun {
                outcome: Outcome::Fallback,
                driver_report: None,
                eval_error: None,
                result: None,
            })
        }
    };
    // v5 顶层同款：循环内命令分派（DynCall/Call）的经典 wasm 每迭代
    // 宿主协议（var 缓存失效 + 分派往返）远慢于解释器原生分派——
    // dict_ops 23×/list_ops 9× 实测亏；arith/var_incr 纯算术循环
    // classic 1.4-1.9× 赢。回落 interp.eval（jitsh main 的 Fallback
    // 分支）。aggressive（wasm32）：解释器基线 wasm 化，保留 classic。
    if !crate::aggressive_tier() && crate::pass::assemble::callsite_in_loop(&module.l3) {
        return (interp, JitRun {
            outcome: Outcome::Fallback,
            driver_report: None,
            eval_error: None,
            result: None,
        });
    }
    match engine {
        #[cfg(feature = "jit-native")]
        EngineKind::Wasmtime => crate::exec_wasmtime::execute(interp, &bc, &module, file),
        EngineKind::Wasmi => execute(interp, &bc, &module, file),
        #[cfg(not(feature = "jit-native"))]
        EngineKind::Wasmtime => execute(interp, &bc, &module, file),
    }
}

/// 从实例 memory 读 n 个 u64（暂存区协议）。
fn read_words(c: &wasmi::Caller<'_, Host>, base: i32, n: i32) -> Vec<u64> {
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

/// towasm 产物回环入口：以既有字节在 wasmi 顶层执行（execute 的 pub
/// 包装——产物即产品：任意引擎拿到同一份 .wasm 都应等价执行，见
/// crates/rtcl-towasm 的跨引擎 roundtrip 门禁）。
pub fn execute_artifact_wasmi(
    interp: Interp,
    bc: &ByteCode,
    module: &emit::Module,
    file: &str,
) -> (Interp, JitRun) {
    execute(interp, bc, module, file)
}

fn execute(
    interp: Interp,
    bc: &ByteCode,
    module: &emit::Module,
    file: &str,
) -> (Interp, JitRun) {
    use wasmi::{Caller, Engine, Extern, Linker, Module, Store, TypedFunc};

    // arena 预驻留：0=UNSET 1=哨兵保留 2=EMPTY 3..=常量池。
    // M3：epoch global 以解释器当前代初始化（替换 M1' 的硬编码 0）。
    // 运行中不刷新——vm_exec 的失效语义是仅单元入口检查（入口 gate
    // 在 run_top_with），回边比对（EpochCheck{at_entry:false} →
    // STALE）为 M4 的模块缓存/proc-body 钩子预留：届时按调用入口
    // 拒绝陈旧模块即可，无需中断运行。
    let epoch0 = interp.tier1_epoch() as i64;
    // M2 seam：Host 持指针（单线程作用域借用）——Interp 本体留在本
    // 栈帧，Store drop 后取回返回。
    let mut interp_cell = interp;
    let host = Host::new(&mut interp_cell as *mut Interp, arena_of(bc), bc, module);
    let watermark = host.arena_len();

    let engine = Engine::default();
    let wmodule = match Module::new(&engine, &module.bytes[..]) {
        Ok(m) => m,
        Err(e) => panic!("jit: emitted module invalid: {e}"),
    };
    let mut store = Store::new(&engine, host);
    let mut linker = <Linker<Host>>::new(&engine);

    // globals（fuel=0 触发首步充值；epoch 恒 0——驱动 M1' 不失效；
    // caching=1 直到首次 rtcl.call）。句柄克隆进宿主闭包（wasm→宿主
    // 直写 global，§3「宿主充值」）。
    let g_fuel = wasmi::Global::new(&mut store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_epoch = wasmi::Global::new(&mut store, wasmi::Val::I64(epoch0), wasmi::Mutability::Var);
    let g_caching = wasmi::Global::new(&mut store, wasmi::Val::I32(1), wasmi::Mutability::Var);
    linker.define("rtcl", "fuel", Extern::Global(g_fuel)).unwrap();
    linker.define("rtcl", "epoch", Extern::Global(g_epoch)).unwrap();
    linker.define("rtcl", "caching", Extern::Global(g_caching)).unwrap();
    // fast 模块的附加导入 stub（fast 门的模块经常规池运行时：proc_main
    // 不被调用，这些定义只为实例化完整——共享 Store 驱动提供真实现）。
    {
        let mem = wasmi::Memory::new(&mut store, wasmi::MemoryType::new(1, None)).unwrap();
        linker.define("rtcl", "memory", Extern::Memory(mem)).unwrap();
        let tbl = wasmi::Table::new(
            &mut store,
            wasmi::TableType::new(wasmi::RefType::Func, 256, None),
            wasmi::Ref::Func(wasmi::Nullable::Null),
        ).unwrap();
        linker.define("rtcl", "funs", Extern::Table(tbl)).unwrap();
        let g_cur = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "cur_proc", Extern::Global(g_cur)).unwrap();
        let g_scb = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "site_cache_base", Extern::Global(g_scb)).unwrap();
        let g_fd = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "fast_depth", Extern::Global(g_fd)).unwrap();
        let g_dty = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        linker.define("rtcl", "dirty", Extern::Global(g_dty)).unwrap();
    }
    linker.func_wrap("rtcl", "resolve_proc", |_cmd: i32, _argc: i32, _base: i32| -> i64 {
        unreachable!("resolve_proc stub (regular store)")
    }).unwrap();
    linker.func_wrap("rtcl", "proc_exit_frame", |_slot: i32| {}).unwrap();
    let g_fuel_call = g_fuel;
    let g_caching_call = g_caching;

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
    let g_caching_bin = g_caching;
    linker
        .func_wrap("rtcl", "binary_op", move |mut c: Caller<'_, Host>, op: i32, a: i64, b: i64| -> i64 {
            // 表达式可含 [cmd] 替换——缓存窗口关闭。
            let _ = g_caching_bin.set(&mut c, wasmi::Val::I32(0));
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
    linker
        .func_wrap(
            "rtcl",
            "call",
            move |mut c: Caller<'_, Host>, cmd_id: i32, site: i32, argc: i32| -> i64 {
                // 新进程不变量落地：首次命令调用即关闭变量缓存。
                let _ = g_caching_call.set(&mut c, wasmi::Val::I32(0));
                c.data_mut().call_raw(cmd_id, site, argc)
            },
        )
        .unwrap();
    linker
        .func_wrap("rtcl", "set_result", |mut c: Caller<'_, Host>, v: i64| {
            c.data_mut().set_result_raw(v);
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "sync_var", |mut c: Caller<'_, Host>, name_h: i32| -> i64 {
            c.data_mut().sync_var_raw(name_h)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "refuel", move |mut c: Caller<'_, Host>| -> i64 {
            let chunk = c.data().refuel_chunk();
            for _ in 0..chunk {
                if let Err(e) = VmContext::charge_step(c.data_mut().interp_mut()) {
                    c.data_mut().set_pending(e, false);
                    return abi::ERR_SENTINEL;
                }
            }
            let _ = g_fuel_call.set(&mut c, wasmi::Val::I64(chunk));
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
        .func_wrap("rtcl", "concat_n", |mut c: Caller<'_, Host>, base: i32, n: i32| -> i64 {
            let mem = c.get_export("memory").and_then(|e| e.into_memory());
            let n_words = n.max(0) as usize;
            let mut bytes = vec![0u8; n_words * 8];
            let mut words = vec![0u64; n_words];
            if let Some(m) = mem {
                let _ = m.read(&c, base as usize, &mut bytes);
                for (i, w) in words.iter_mut().enumerate() {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
                    *w = u64::from_le_bytes(b);
                }
            }
            c.data_mut().concat_raw(&words)
        })
        .unwrap();
    let g_caching_fe = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "foreach_start",
            move |mut c: Caller<'_, Host>, info: i32, base: i32, ngroups: i32| -> i64 {
                let words = read_words(&c, base, ngroups);
                let _ = g_caching_fe.set(&mut c, wasmi::Val::I32(0));
                c.data_mut().foreach_start_raw(info, &words)
            },
        )
        .unwrap();
    let g_caching_bind = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "foreach_bind",
            move |mut c: Caller<'_, Host>, info: i32, idx: i32| -> i64 {
                let _ = g_caching_bind.set(&mut c, wasmi::Val::I32(0));
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
    let g_caching_lap = g_caching;
    linker
        .func_wrap("rtcl", "lappend_var", move |mut c: Caller<'_, Host>, name_h: i32, v: i64| -> i64 {
            let _ = g_caching_lap.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().lappend_var_raw(name_h, v)
        })
        .unwrap();
    let g_caching_cav = g_caching;
    linker
        .func_wrap(
            "rtcl",
            "call_argv",
            move |mut c: Caller<'_, Host>, base: i32, argc: i32, site: i32| -> i64 {
                // 命令可写任意变量——缓存窗口关闭（与 call 同纪律）。
                let _ = g_caching_cav.set(&mut c, wasmi::Val::I32(0));
                let words = read_words(&c, base, argc);
                c.data_mut().call_argv_raw(&words, site)
            },
        )
        .unwrap();
    let g_caching_es = g_caching;
    linker
        .func_wrap("rtcl", "eval_script", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = g_caching_es.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_script_raw(v)
        })
        .unwrap();
    let g_caching_ee = g_caching;
    linker
        .func_wrap("rtcl", "eval_expr", move |mut c: Caller<'_, Host>, v: i64| -> i64 {
            let _ = g_caching_ee.set(&mut c, wasmi::Val::I32(0));
            c.data_mut().eval_expr_raw(v)
        })
        .unwrap();
    linker
        .func_wrap("rtcl", "exit", |mut c: Caller<'_, Host>, code: i32| {
            c.data_mut().exit_raw(code);
        })
        .unwrap();

    let instance = linker.instantiate_and_start(&mut store, &wmodule).unwrap_or_else(|e| {
        panic!("jit: instantiate failed: {e}")
    });
    let run: TypedFunc<(), i32> = instance.get_typed_func(&store, "run").unwrap();


    rtcl_vm::interp::set_jit_call_in_flight(true);
    let rc = run.call(&mut store, ()).unwrap_or_else(|trap| {
        rtcl_vm::interp::set_jit_call_in_flight(false);
        // wasm trap = 内部 bug（越界/除零未兜底等）——显式失败。
        panic!("jit: wasm trap: {trap:?}")
    });
    // run 结束缓存回写：caching=1 期间 StoreVar 只写 wasm 缓存（值栈
    // 区），缓存随 Store 丢弃——interp 帧从未回写。单单元 judge 全经
    // set_result/puts（宿主出口）不可见；跨单元读帧（REPL 多次 eval、
    // towasm 产物消费方、rtcl-wasm 嵌入）读到陈旧值 = 正确性缺口。
    // 逐 var 查 STATE：2=脏 → set_var_raw 回写（O(nvars) 内存读 +
    // 脏项宿主 set）。
    {
        let nvars = module.l2.vars.len().min(crate::abi::cache::MAX_VARS);
        if nvars > 0 {
            if let Some(wasmi::Extern::Memory(m)) = instance.get_export(&mut store, "memory") {
                let mut states = vec![0u8; nvars];
                if m.read(&mut store, crate::abi::cache::STATE as usize, &mut states).is_ok() {
                    let mut vals = vec![0u8; nvars * 8];
                    let _ = m.read(&mut store, crate::abi::cache::VALUES as usize, &mut vals);
                    for (k, v) in module.l2.vars.iter().enumerate() {
                        if states[k] == 2 {
                            let mut b = [0u8; 8];
                            b.copy_from_slice(&vals[k * 8..k * 8 + 8]);
                            let tagged = i64::from_le_bytes(b);
                            let name_h = crate::abi::const_handle(v.const_idx) as i32;
                            store.data_mut().set_var_raw(name_h, tagged);
                        }
                    }
                }
            }
        }
    }

    rtcl_vm::interp::set_jit_call_in_flight(false);

    // ④ 中途 bump：整单元 STALE → 从 bump 命令的下一条顶层命令树走查
    //（上下文保持；非边界 bump 维持错误报告现状）。重分派后按正常
    // 完成收束（结果 = 重分派结果，或 bump 恰在末条时的暂存结果）。
    let rc = if rc == code::STALE {
        let stale = { store.data().stale_site };
        match stale.and_then(|site| {
            store.data_mut().interp_mut().jit_redispatch_after(&bc, site)
        }) {
            Some((true, Ok(v))) => {
                store.data_mut().result = Some(v);
                code::OK
            }
            // 错误帧已由树走查 harness 追加到 interp 累积器——以 case-a
            // 进 pending，finish 的 eval_error 路径交 jitsh error_report。
            Some((true, Err(e))) => {
                store.data_mut().pending = Some(Pending { err: e, from_eval: true });
                code::ERROR
            }
            Some((false, _)) => code::OK, // 末条 bump：结果已暂存 host.result
            None => code::STALE,          // 非边界：维持 finish 现状路径
        }
    } else {
        rc
    };

    // arena 回收到水位线（§3 纪律）。
    store.data_mut().arena.truncate(watermark);
    let host = store.into_data();
    let run = crate::exec::finish(host, rc, file);
    (interp_cell, run)
}

/// run() 返回后的共享终局：错误二分（case a/b）与报告构建。
pub(crate) fn finish(mut host: Host, outcome_code: i32, file: &str) -> JitRun {
    let pending_exit = host.pending_exit.take();
    let (driver_report, eval_error) = match outcome_code {
        code::ERROR => {
            let is_case_a = host.pending.as_ref().is_some_and(|p| p.from_eval);
            if is_case_a {
                let e = host.pending.take().unwrap().err;
                // 需要驱动补帧的形态（EvalExpr 站点帧 / EvalScript
                // pending / unwind 有 Sub/Expr/BodyFramed）；纯 Call 的
                // 站点帧已由嵌套 eval 追加——走 error_report 原路。
                let needs_extras = host.last_fail.is_some_and(|f| {
                    host.fails.get(f as usize).is_some_and(|fi| {
                        !matches!(fi.kind, crate::ir::FailKind::Call)
                            || fi
                                .bodies
                                .iter()
                                .any(|(k, _)| !matches!(
                                    k,
                                    crate::ir::BodyKind::Body | crate::ir::BodyKind::Next
                                ))
                    })
                });
                if needs_extras {
                    let r = host.build_case_a_report(host.last_fail, &e, file);
                    (Some(r), None)
                } else {
                    (None, Some(e))
                }
            } else {
                let r = host.driver_report(file);
                (Some(r), None)
            }
        }
        code::RETURN | code::BREAK | code::CONTINUE => {
            let e = match outcome_code {
                code::RETURN => Error::ret(host.result.clone()),
                code::BREAK => Error::brk(),
                _ => Error::cont(),
            };
            host.pending = Some(Pending { err: e, from_eval: false });
            let r = host.driver_report(file);
            (Some(r), None)
        }
        _ => (None, None),
    };
    JitRun {
        outcome: Outcome::Completed { code: outcome_code, exit_code: pending_exit },
        driver_report,
        eval_error,
        result: host.result.take(),
    }
}
