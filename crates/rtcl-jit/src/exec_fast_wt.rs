//! fast-tier 的 wasmtime 驱动（feature `jit-native`）：wasmi 侧
//! proc_exec.rs 的 FastHost/fast_linker/execute_fast 全量移植。
//!
//! 架构同源：共享 Store（FastHostWt）+ per-slot Host（exec::Host 引擎
//! 无关方法全复用）+ funcref 表 call_indirect 直呼 + per-depth 参数
//! 切片 + (i64 val, i32 rc) 多值返回。差异只在引擎类型（Global/Memory/
//! Table/TypedFunc/Caller）与 Trap 语义（wasmi Err = 栈耗尽 →
//! StackOverflow 映射，wasmtime 同）。
//!
//! 移植对照（wasmi → 本文件）：FastHost→FastHostWt / init_fast_store→
//! init_fast_store_wt / fast_linker→fast_linker_wt / fast_ensure_slot→
//! fast_ensure_slot_wt / execute_fast→execute_fast_wt / fast_try→
//! fast_try_wt / fast_take_fail、fast_rc_to_i64、read_cur、refresh_epoch、
//! fast_read_words 各有 _wt 版（Caller 类型不同）。

#![allow(unsafe_code)]

use crate::abi;
use crate::exec::{Host, Pending};
use rtcl_core::{Error, Interp, Value};
use rtcl_parser::ByteCode;
use std::rc::Rc;

use wasmtime::{AsContext, AsContextMut, Caller, Linker, Val};

type PoolKey = (usize, usize, u64);

pub(crate) struct FastHostWt {
    pub interp: *mut Interp,
    /// per-proc 表（arena/fails/lines/consts——Host 全方法复用）。
    pub hosts: Vec<Host>,
    /// slot → proc 体（名反查/jit_redispatch_after 用）。
    pub codes: Vec<Rc<ByteCode>>,
    /// slot → proc_main 句柄（resolve 冷融合代打）。
    pub runs: Vec<wasmtime::TypedFunc<(i32, i32, i32), (i64, i32)>>,
    pub slots: std::collections::HashMap<PoolKey, usize>,
    // 共享运行态（语义与 wasmi FastHost 逐字段一致）
    pub pending: Option<Pending>,
    pub pending_exit: Option<i32>,
    pub stale_site: Option<u32>,
    pub epoch_at_entry: u64,
    pub last_fail: Option<u32>,
    /// slot → 本实例 site_cache_base 值。
    pub scb_of: Vec<i32>,
    pub engine: wasmtime::Engine,
    pub memory: Option<wasmtime::Memory>,
    pub table: Option<wasmtime::Table>,
    pub o_fuel: Option<wasmtime::Global>,
    pub o_epoch: Option<wasmtime::Global>,
    pub o_caching: Option<wasmtime::Global>,
    pub o_cur: Option<wasmtime::Global>,
    pub o_depth: Option<wasmtime::Global>,
    pub o_dirty: Option<wasmtime::Global>,
}

impl FastHostWt {
    fn mem(&self) -> wasmtime::Memory {
        self.memory.clone().expect("fast store init")
    }
    fn tbl(&self) -> wasmtime::Table {
        self.table.clone().expect("fast store init")
    }
    fn fuel_g(&self) -> wasmtime::Global {
        self.o_fuel.clone().expect("fast store init")
    }
    fn epoch_g(&self) -> wasmtime::Global {
        self.o_epoch.clone().expect("fast store init")
    }
    fn dirty_g(&self) -> wasmtime::Global {
        self.o_dirty.clone().expect("fast store init")
    }
}

thread_local! {
    /// fast 模块专用 Engine（OptLevel::Speed）：fast 单元小（~1.5-4KB），
    /// Cranelift speed 档编译毫秒级——顶层大单元的 19s 编译超时是
    /// classic 引擎 None 档的缘由，fast 侧不受约束。perf：fib/fl 再
    /// 提一档（None→Speed 典型 1.5-3×）。
    static WT_FAST_ENGINE: wasmtime::Engine = {
        let mut cfg = wasmtime::Config::new();
        cfg.cranelift_opt_level(wasmtime::OptLevel::Speed);
        wasmtime::Engine::new(&cfg).expect("wasmtime fast engine")
    };
    static WT_FAST_STORE: std::cell::RefCell<Option<wasmtime::Store<FastHostWt>>> =
        const { std::cell::RefCell::new(None) };
    static WT_FAST_MODULES: std::cell::RefCell<
        std::collections::HashMap<PoolKey, wasmtime::Module>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Caller 上读 cur_proc global（fast 模块 re-export，get_export 可见）。
fn read_cur_wt(c: &mut Caller<'_, FastHostWt>) -> usize {
    if let Some(wasmtime::Extern::Global(g)) = c.get_export("cur_proc") {
        if let Val::I32(v) = g.get(&mut *c) {
            return v.max(0) as usize;
        }
    }
    0
}

/// epoch 刷新（wasmi refresh_epoch 同语义）：共享 epoch global 与
/// interp tier1_epoch 对齐——CallSite wasm 内 epoch 门即时可见。
fn refresh_epoch_wt(c: &mut Caller<'_, FastHostWt>) {
    let now = unsafe { (*c.data().interp).tier1_epoch() } as i64;
    let g = c.data().epoch_g();
    if let Val::I64(cur) = g.get(&mut *c) {
        if cur != now {
            let _ = g.set(&mut *c, Val::I64(now));
        }
    }
}

/// 共享 memory 读 n 个 u64（SCRATCH/参数区协议）。
fn fast_read_words_wt(c: &mut Caller<'_, FastHostWt>, base: i32, n: i32) -> Vec<u64> {
    let n_words = n.max(0) as usize;
    let mut bytes = vec![0u8; n_words * 8];
    let mut words = vec![0u64; n_words];
    if let Some(wasmtime::Extern::Memory(m)) = c.get_export("memory") {
        if m.read(&mut *c, base.max(0) as usize, &mut bytes).is_ok() {
            for (i, w) in words.iter_mut().enumerate() {
                let mut b = [0u8; 8];
                b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
                *w = u64::from_le_bytes(b);
            }
        }
    }
    words
}

/// fast 版 take_fail（wasmi fast_take_fail 逐行复刻；pending 共享）。
fn fast_take_fail_wt(h: &mut FastHostWt, cur: usize, fail_id: u32) -> i32 {
    if std::env::var_os("JIT_VM_TRACE").is_some() {
        eprintln!("[fast-wt] take_fail cur={cur} fid={fail_id}");
    }
    use crate::ir::BodyKind;
    let Some(p) = h.pending.take() else { return 0 };
    let e = p.err;
    if e.message_text() == crate::exec::STALE_MARK {
        return 6;
    }
    let kind = crate::exec::classify_control(&e);
    if kind != 0 {
        // kind 5：Next 区域 continue 逃逸（经典 take_fail 同口径）。
        if kind == 2
            && h.hosts[cur]
                .fails
                .get(fail_id as usize)
                .is_some_and(|f| f.esc.is_some())
        {
            return 5;
        }
        return kind;
    }
    let Some(fi) = h.hosts[cur].fails.get(fail_id as usize).cloned() else {
        h.pending = Some(Pending { err: e, from_eval: false });
        return 0;
    };
    let irp = unsafe { &mut *h.interp };
    let frameless = fi
        .bodies
        .last()
        .is_some_and(|(_, s)| *s == fi.site);
    if fi.site != u32::MAX && !frameless {
        let text = h.hosts[cur].site_text_of(fi.site);
        let line = h.hosts[cur].lines_of(fi.site);
        let msg = e.message_text();
        irp.jit_err_set_line(line);
        irp.jit_err_harness_frame(&msg, &text, line);
    }
    for (bkind, site) in fi.bodies.iter().rev() {
        let text = h.hosts[cur].site_text_of(*site);
        let line = h.hosts[cur].lines_of(*site);
        match bkind {
            BodyKind::Sub => irp.jit_err_set_pending(&text, line),
            BodyKind::Expr => {
                let msg = e.message_text();
                irp.jit_err_harness_frame(&msg, &text, line);
            }
            BodyKind::BodyFramed(lmap) => {
                let tag = if *lmap { "\"lmap\" body" } else { "\"foreach\" body" };
                irp.jit_err_exit_frame(tag);
                let msg = e.message_text();
                irp.jit_err_harness_frame(&msg, &text, line);
            }
            BodyKind::Body | BodyKind::Next => {}
        }
    }
    h.pending = Some(Pending { err: e, from_eval: false });
    0
}

/// 多值 (val, rc) → 单 i64（resolve 代打返回契约；wasmi fast_rc_to_i64
/// 同语义）。
fn fast_rc_to_i64_wt(c: &mut Caller<'_, FastHostWt>, val: i64, rc: i32) -> i64 {
    match rc {
        0 | 2 => val,
        3 | 4 => {
            let e = if rc == 3 { Error::brk() } else { Error::cont() };
            c.data_mut().pending = Some(Pending { err: e, from_eval: false });
            abi::ERR_SENTINEL
        }
        5 => {
            c.data_mut().pending = Some(Pending {
                err: Error::Msg(crate::exec::STALE_MARK.to_string()),
                from_eval: false,
            });
            abi::ERR_SENTINEL
        }
        6 => abi::ERR_SENTINEL, // EXIT：pending_exit 已置
        _ => abi::ERR_SENTINEL, // ERROR：内层 fail 已置 pending+帧
    }
}

/// fast 共享 Store 的 wasmtime linker：与 wasmi fast_linker 逐闭包对照
/// （cur_proc 路由 per-proc Host；gated 家族 unreachable；命令分派双臂
/// 均消费 cross-frame 哨兵 → dirty）。
fn fast_linker_wt<C: wasmtime::AsContextMut<Data = FastHostWt>>(
    store: &mut C,
    site_cache_base: i32,
) -> Linker<FastHostWt> {
    let (memory, table, g_fuel, g_epoch, g_caching, g_cur, g_depth, g_dirty) = {
        let h = store.as_context().data();
        (
            h.mem(),
            h.tbl(),
            h.fuel_g(),
            h.epoch_g(),
            h.o_caching.clone().expect("fast store init"),
            h.o_cur.clone().expect("fast store init"),
            h.o_depth.clone().expect("fast store init"),
            h.dirty_g(),
        )
    };
    let mut linker = <Linker<FastHostWt>>::new(&store.as_context().data().engine);
    linker.define(&mut *store, "rtcl", "memory", wasmtime::Extern::Memory(memory)).unwrap();
    linker.define(&mut *store, "rtcl", "funs", wasmtime::Extern::Table(table)).unwrap();
    linker.define(&mut *store, "rtcl", "fuel", wasmtime::Extern::Global(g_fuel.clone())).unwrap();
    linker.define(&mut *store, "rtcl", "epoch", wasmtime::Extern::Global(g_epoch.clone())).unwrap();
    linker.define(&mut *store, "rtcl", "caching", wasmtime::Extern::Global(g_caching)).unwrap();
    linker.define(&mut *store, "rtcl", "cur_proc", wasmtime::Extern::Global(g_cur)).unwrap();
    linker.define(&mut *store, "rtcl", "fast_depth", wasmtime::Extern::Global(g_depth)).unwrap();
    linker.define(&mut *store, "rtcl", "dirty", wasmtime::Extern::Global(g_dirty)).unwrap();
    let g_scb = wasmtime::Global::new(
        &mut *store,
        wasmtime::GlobalType::new(wasmtime::ValType::I32, wasmtime::Mutability::Var),
        Val::I32(site_cache_base),
    )
    .expect("scb global");
    linker.define(&mut *store, "rtcl", "site_cache_base", wasmtime::Extern::Global(g_scb)).unwrap();

    // ── 宿主路由（冷路径经 cur_proc 路由到 per-proc Host）──
    linker.func_wrap("rtcl", "unbox_int", |mut c: Caller<'_, FastHostWt>, h: i32| -> (i32, i64) {
        let cur = read_cur_wt(&mut c);
        c.data_mut().hosts[cur].unbox_int_raw(h)
    }).unwrap();
    linker.func_wrap("rtcl", "unbox_float", |mut c: Caller<'_, FastHostWt>, h: i32| -> (i32, f64) {
        let cur = read_cur_wt(&mut c);
        c.data_mut().hosts[cur].unbox_float_raw(h)
    }).unwrap();
    linker.func_wrap("rtcl", "box", |mut c: Caller<'_, FastHostWt>, v: i64| -> i32 {
        let cur = read_cur_wt(&mut c);
        c.data_mut().hosts[cur].box_raw(v)
    }).unwrap();
    linker.func_wrap("rtcl", "box_float", |mut c: Caller<'_, FastHostWt>, f: f64| -> i32 {
        let cur = read_cur_wt(&mut c);
        c.data_mut().hosts[cur].box_float_raw(f)
    }).unwrap();
    linker.func_wrap("rtcl", "binary_op", |mut c: Caller<'_, FastHostWt>, op: i32, a: i64, b: i64| -> i64 {
        let cur = read_cur_wt(&mut c);
        refresh_epoch_wt(&mut c);
        let h = &mut c.data_mut().hosts[cur];
        let r = h.binary_op_raw(op, a, b);
        if r == abi::ERR_SENTINEL {
            let e = h.pending.take().map(|p| p.err).unwrap_or_else(|| Error::Msg("fast binary_op failed".into()));
            c.data_mut().pending = Some(Pending { err: e, from_eval: false });
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "get_var", |mut c: Caller<'_, FastHostWt>, name_h: i32| -> i64 {
        let cur = read_cur_wt(&mut c);
        let name = c.data_mut().hosts[cur].const_name(name_h);
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[ft-wt] get_var cur={cur} name={name:?}");
        }
        let r = unsafe { &mut *c.data().interp }.jit_read_var_ref(&name);
        let h = &mut c.data_mut().hosts[cur];
        match r {
            Ok(v) => h.tag_value(&v),
            Err(e) => {
                c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }).unwrap();
    linker.func_wrap("rtcl", "set_var", |mut c: Caller<'_, FastHostWt>, name_h: i32, v: i64| -> i32 {
        let cur = read_cur_wt(&mut c);
        let (name, val) = {
            let h = &mut c.data_mut().hosts[cur];
            (h.const_name(name_h), h.value_of(v))
        };
        match unsafe { &mut *c.data().interp }.set_var(&name, val) {
            Ok(_) => 0,
            Err(e) => {
                c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                1
            }
        }
    }).unwrap();
    linker.func_wrap("rtcl", "call", |_c: Caller<'_, FastHostWt>, _a: i32, _b: i32, _d: i32| -> i64 {
        unreachable!("call import in fast store")
    }).unwrap();
    linker.func_wrap("rtcl", "set_result", |_c: Caller<'_, FastHostWt>, _v: i64| {}).unwrap();
    let fuel_g = store.as_context().data().fuel_g();
    linker.func_wrap("rtcl", "refuel", move |mut c: Caller<'_, FastHostWt>| -> i64 {
        let chunk = abi::REFUEL_CHUNK;
        for _ in 0..chunk {
            if let Err(e) = rtcl_vm::VmContext::charge_step(unsafe { &mut *c.data().interp }) {
                c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                return abi::ERR_SENTINEL;
            }
        }
        let _ = fuel_g.set(&mut c, Val::I64(chunk));
        0
    }).unwrap();
    linker.func_wrap("rtcl", "fail", |mut c: Caller<'_, FastHostWt>, fail_id: i32| -> i32 {
        let cur = read_cur_wt(&mut c);
        let fid = fail_id.max(0) as u32;
        c.data_mut().last_fail = Some(fid);
        let h = &mut c.data_mut();
        let r = fast_take_fail_wt(h, cur, fid);
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[ft-wt] fail cur={cur} fid={fid} -> {r}");
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "bool_test", |mut c: Caller<'_, FastHostWt>, v: i64| -> i64 {
        let cur = read_cur_wt(&mut c);
        let r = c.data_mut().hosts[cur].bool_test_raw(v);
        if r == abi::ERR_SENTINEL {
            let e = c.data_mut().hosts[cur].pending.take().map(|p| p.err)
                .unwrap_or_else(|| Error::Msg("fast bool_test failed".into()));
            c.data_mut().pending = Some(Pending { err: e, from_eval: false });
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "incr_var", |mut c: Caller<'_, FastHostWt>, name_h: i32, n: i64| -> i64 {
        let cur = read_cur_wt(&mut c);
        let r = c.data_mut().hosts[cur].incr_var_raw(name_h, n);
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[ft-wt] incr_var cur={cur} n={n} -> {r}");
        }
        if r == abi::ERR_SENTINEL {
            let e = c.data_mut().hosts[cur].pending.take().map(|p| p.err)
                .unwrap_or_else(|| Error::Msg("fast incr failed".into()));
            c.data_mut().pending = Some(Pending { err: e, from_eval: false });
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "push_const", |_c: Caller<'_, FastHostWt>, idx: i32| -> i64 {
        abi::const_handle(idx as u32)
    }).unwrap();
    linker.func_wrap("rtcl", "exit", |mut c: Caller<'_, FastHostWt>, code: i32| {
        c.data_mut().pending_exit = Some(code);
    }).unwrap();
    linker.func_wrap("rtcl", "concat_n", |_c: Caller<'_, FastHostWt>, _b: i32, _n: i32| -> i64 {
        unreachable!("concat in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_start", |_c: Caller<'_, FastHostWt>, _i: i32, _b: i32, _n: i32| -> i64 {
        unreachable!("foreach in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_bind", |_c: Caller<'_, FastHostWt>, _i: i32, _x: i32| -> i64 {
        unreachable!("foreach in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_collect", |_c: Caller<'_, FastHostWt>, _v: i64| -> () {
        unreachable!("foreach in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_end", |_c: Caller<'_, FastHostWt>| -> i64 {
        unreachable!("foreach in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "eval_script", |_c: Caller<'_, FastHostWt>, _v: i64| -> i64 {
        unreachable!("eval_script in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "eval_expr", |_c: Caller<'_, FastHostWt>, _v: i64| -> i64 {
        unreachable!("eval_expr in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "lappend_var", |_c: Caller<'_, FastHostWt>, _n: i32, _v: i64| -> i64 {
        unreachable!("lappend in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "sync_var", |mut c: Caller<'_, FastHostWt>, name_h: i32| -> i64 {
        let cur = read_cur_wt(&mut c);
        let name = c.data().hosts[cur].const_name(name_h);
        let interp = unsafe { &mut *c.data().interp };
        match interp.get_var(&name).cloned() {
            Ok(v) => {
                let h = &mut c.data_mut().hosts[cur];
                h.tag_value(&v)
            }
            Err(_) => 0,
        }
    }).unwrap();

    // call_argv：resolve 未命中后的常驻回落——jit_call_proc_direct 优先，
    // dispatch_values 兜底；epoch 门 + cross-frame 哨兵传播（wasmi 同）。
    linker.func_wrap("rtcl", "call_argv", |mut c: Caller<'_, FastHostWt>, base: i32, argc: i32, site: i32| -> i64 {
        let cur = read_cur_wt(&mut c);
        refresh_epoch_wt(&mut c);
        let words = fast_read_words_wt(&mut c, base, argc);
        let mut args: Vec<Value> = Vec::with_capacity(words.len());
        {
            let h = &c.data().hosts[cur];
            for w in words {
                args.push(h.value_of(w as i64));
            }
        }
        let interp = unsafe { &mut *c.data().interp };
        let epoch0 = c.data().epoch_at_entry;
        let r = match interp.jit_call_proc_direct(args[0].as_str(), &args) {
            Some(r) => r,
            None => interp.dispatch_values(&args),
        };
        // 跨帧写哨兵传播：被调方执行了 upvar/global → dirty global 置位。
        if rtcl_vm::interp::jit_take_cross_frame() {
            if std::env::var_os("JIT_FT").is_some() {
                eprintln!("[ft-wt] call_argv cross-frame → dirty=1 (cur={cur})");
            }
            let g = c.data().dirty_g();
            let _ = g.set(&mut c, Val::I32(1));
        }
        match r {
            Ok(v) => {
                if interp.tier1_epoch() != epoch0 {
                    c.data_mut().stale_site = Some(site.max(0) as u32);
                    c.data_mut().pending = Some(Pending {
                        err: Error::Msg(crate::exec::STALE_MARK.to_string()),
                        from_eval: false,
                    });
                    return abi::ERR_SENTINEL;
                }
                let h = &mut c.data_mut().hosts[cur];
                h.tag_value(&v)
            }
            Err(e) => {
                c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                abi::ERR_SENTINEL
            }
        }
    }).unwrap();

    // resolve_proc：冷融合——解析目标 → fast 化则实例化+填 cache+代打；
    // 否则 cache=-1 + 分派代打。首调后纯 wasm call_indirect。
    linker.func_wrap("rtcl", "resolve_proc", |mut c: Caller<'_, FastHostWt>, site: i32, argc: i32, base: i32| -> i64 {
        let cur = read_cur_wt(&mut c);
        refresh_epoch_wt(&mut c);
        let words = fast_read_words_wt(&mut c, base, argc);
        let name = c.data().hosts[cur].value_of(words[0] as i64).as_str().to_string();
        let interp = unsafe { &mut *c.data().interp };
        let epoch0 = c.data().epoch_at_entry;
        let target = interp.jit_proc_code(&name);
        let write_cache = |c: &mut Caller<'_, FastHostWt>, slot: i32| {
            let cur0 = read_cur_wt(c);
            let base = c.data().scb_of.get(cur0).copied().unwrap_or(0);
            if let Some(wasmtime::Extern::Memory(m)) = c.get_export("memory") {
                let addr = (base + site * 4).max(0) as usize;
                let _ = m.write(&mut *c, addr, &slot.to_le_bytes());
            }
        };
        let slot = target.as_ref().and_then(|code| fast_ensure_slot_wt(&mut c, interp, code));
        match slot {
            Some(slot) => {
                write_cache(&mut c, slot as i32);
                let tf = c.data().runs[slot].clone();
                match tf.call(&mut c, (slot as i32, argc, base)) {
                    Ok((val, rc)) => fast_rc_to_i64_wt(&mut c, val, rc),
                    Err(_) => {
                        c.data_mut().pending = Some(Pending {
                            err: Error::runtime("maximum recursion depth exceeded", rtcl_core::error::ErrorCode::StackOverflow),
                            from_eval: false,
                        });
                        abi::ERR_SENTINEL
                    }
                }
            }
            None => {
                write_cache(&mut c, -1);
                let mut args: Vec<Value> = Vec::with_capacity(words.len());
                {
                    let h = &c.data().hosts[cur];
                    for w in &words {
                        args.push(h.value_of(*w as i64));
                    }
                }
                match interp.dispatch_values(&args) {
                    Ok(v) => {
                        // 无条件 dirty：非 proc 命令的宿主分派可能改任意
                        // 帧变量（lappend/eval 型 catch…）——仅跨帧哨兵
                        // 会漏（judge catch_error 实证）。proc 直呼走
                        // Some 臂不经此处，热路径零成本。
                        if std::env::var_os("JIT_FT").is_some() {
                            eprintln!("[ft-wt] resolve dispatch → dirty=1 (cur={cur})");
                        }
                        {
                            let g = c.data().dirty_g();
                            let _ = g.set(&mut c, Val::I32(1));
                        }
                        if interp.tier1_epoch() != epoch0 {
                            c.data_mut().stale_site = Some(site.max(0) as u32);
                            c.data_mut().pending = Some(Pending {
                                err: Error::Msg(crate::exec::STALE_MARK.to_string()),
                                from_eval: false,
                            });
                            return abi::ERR_SENTINEL;
                        }
                        let h = &mut c.data_mut().hosts[cur];
                        h.tag_value(&v)
                    }
                    Err(e) => {
                        c.data_mut().pending = Some(Pending { err: e, from_eval: false });
                        abi::ERR_SENTINEL
                    }
                }
            }
        }
    }).unwrap();

    // proc_exit_frame：fast 错误冷路径——本层补 (procedure "name" line N)。
    linker.func_wrap("rtcl", "proc_exit_frame", |mut c: Caller<'_, FastHostWt>, slot: i32| {
        let slot = slot.max(0) as usize;
        let code = c.data().codes.get(slot).cloned();
        if let Some(code) = code {
            let ip = unsafe { &mut *c.data().interp };
            if let Some(name) = ip.jit_proc_name_of(&code) {
                let tag = format!("procedure \"{}\"", name);
                ip.jit_err_exit_frame(&tag);
            }
        }
    }).unwrap();

    linker
}

/// 在共享 Store 内确保 slot（解析目标 → emit 缓存 → fast gate → 实例化
/// → 表槽/句柄/登记）。不可 fast 化或实例化失败 → None。
fn fast_ensure_slot_wt(
    c: &mut Caller<'_, FastHostWt>,
    interp: &mut Interp,
    code: &Rc<ByteCode>,
) -> Option<usize> {
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(code) as usize,
        code.epoch,
    );
    if let Some(s) = c.data().slots.get(&pool_key) {
        return Some(*s);
    }
    let module = crate::proc_exec::compile_cached(code, (Rc::as_ptr(code) as usize, code.epoch))?;
    let _meta = module.fast?; // fast gate 不命中 → None（分派回落）
    let engine = c.data().engine.clone();
    let wmodule = WT_FAST_MODULES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(w) = m.get(&pool_key) {
            return Some(w.clone());
        }
        match wasmtime::Module::new(&engine, &module.bytes[..]) {
            Ok(w) => {
                if m.len() >= 64 {
                    m.clear();
                }
                m.insert(pool_key, w.clone());
                if std::env::var_os("JIT_DM").is_some() {
                    let _ = std::fs::write("/tmp/jitdbg/fm.wt.wasm", &module.bytes[..]);
                }
                Some(w)
            }
            Err(_) => None,
        }
    })?;
    let slot = c.data().hosts.len();
    if slot >= crate::abi::cache::MAX_FAST_SLOTS {
        // 槽位上限（内存预算/容量防线）：超出不 fast 化。
        return None;
    }
    let scb = crate::abi::cache::SITE_CACHE_BASE + (slot as u32) * crate::abi::cache::SITE_STRIDE;
    let mut linker = fast_linker_wt(c, scb as i32);
    let host = Host::new(interp as *mut Interp, crate::exec::arena_of(code), code, &module);
    let inst = linker.instantiate(&mut *c, &wmodule).ok()?;
    let f = inst.get_typed_func::<(i32, i32, i32), (i64, i32)>(&mut *c, "proc_main").ok()?;
    let tbl = c.data().tbl();
    tbl.set(&mut *c, slot as u64, wasmtime::Ref::Func(Some(*f.func())))
        .ok()?;
    // site-cache 段预失效（0xFF = -1）：新鲜内存是 0——slot 0 恒「命中」
    // 表[0]（第一个实例自己）→ 自调用死循环。
    {
        let mem = c.data().mem();
        let fill = vec![0xFFu8; crate::abi::cache::SITE_STRIDE as usize];
        let _ = mem.write(&mut *c, scb as usize, &fill);
    }
    let h = c.data_mut();
    h.hosts.push(host);
    h.scb_of.push(scb as i32);
    h.codes.push(Rc::clone(code));
    h.runs.push(f);
    h.slots.insert(pool_key, slot);
    Some(slot)
}

/// 初始化共享 fast Store（二阶段：句柄在 Store 存在后创建；wasmi
/// init_fast_store 同构——16 页内存，global 七件套）。
fn init_fast_store_wt(interp: &mut Interp) -> wasmtime::Store<FastHostWt> {
    let engine = WT_FAST_ENGINE.with(|e| e.clone());
    let mut store = wasmtime::Store::new(
        &engine,
        FastHostWt {
            interp: interp as *mut Interp,
            hosts: Vec::new(),
            codes: Vec::new(),
            runs: Vec::new(),
            slots: std::collections::HashMap::new(),
            pending: None,
            pending_exit: None,
            stale_site: None,
            epoch_at_entry: 0,
            last_fail: None,
            scb_of: Vec::new(),
            engine: engine.clone(),
            memory: None,
            table: None,
            o_fuel: None,
            o_epoch: None,
            o_caching: None,
            o_cur: None,
            o_depth: None,
            o_dirty: None,
        },
    );
    // 16 页（1MB）：site-cache 68KB + 参数切片（128B×depth）。
    let memory = wasmtime::Memory::new(
        &mut store,
        wasmtime::MemoryType::new(16, None),
    )
    .expect("fast memory");
    let table = wasmtime::Table::new(
        &mut store,
        wasmtime::TableType::new(wasmtime::RefType::FUNCREF, 256, None),
        wasmtime::Ref::Func(None),
    )
    .expect("fast table");
    let mk_g_i32 = |store: &mut wasmtime::Store<FastHostWt>, v: i32| {
        wasmtime::Global::new(
            store,
            wasmtime::GlobalType::new(wasmtime::ValType::I32, wasmtime::Mutability::Var),
            Val::I32(v),
        )
        .expect("fast global")
    };
    let g_fuel = wasmtime::Global::new(
        &mut store,
        wasmtime::GlobalType::new(wasmtime::ValType::I64, wasmtime::Mutability::Var),
        Val::I64(0),
    )
    .expect("fast fuel global");
    let g_epoch = wasmtime::Global::new(
        &mut store,
        wasmtime::GlobalType::new(wasmtime::ValType::I64, wasmtime::Mutability::Var),
        Val::I64(0),
    )
    .expect("fast epoch global");
    let g_caching = mk_g_i32(&mut store, 1);
    let g_cur = mk_g_i32(&mut store, 0);
    let g_depth = mk_g_i32(&mut store, 0);
    let g_dirty = mk_g_i32(&mut store, 0);
    {
        let h = store.data_mut();
        h.memory = Some(memory);
        h.table = Some(table);
        h.o_fuel = Some(g_fuel);
        h.o_epoch = Some(g_epoch);
        h.o_caching = Some(g_caching);
        h.o_cur = Some(g_cur);
        h.o_depth = Some(g_depth);
        h.o_dirty = Some(g_dirty);
    }
    store
}

/// fast-tier 尝试（wasmtime 侧；fast_try 同构）。
pub(crate) fn fast_try_wt(interp: &mut Interp, code: &Rc<ByteCode>) -> Option<rtcl_core::Result<Value>> {
    if crate::proc_exec::fast_in_call() {
        return None;
    }
    let key = (Rc::as_ptr(code) as usize, code.epoch);
    let module = crate::proc_exec::compile_cached(code, key)?;
    let meta = module.fast?;
    execute_fast_wt(interp, code, &module, meta)
}

/// fast-tier 外层驱动（wasmtime；execute_fast 同构）。
fn execute_fast_wt(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
    module: &crate::emit::Module,
    meta: crate::emit::FastMeta,
) -> Option<rtcl_core::Result<Value>> {
    let epoch = interp.tier1_epoch();
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        bc.epoch,
    );
    WT_FAST_STORE.with(|f| {
        let mut cell = f.borrow_mut();
        if cell.is_none() {
            *cell = Some(init_fast_store_wt(interp));
        }
        let store = cell.as_mut().unwrap();
        // 入口态
        {
            let h = store.data_mut();
            h.pending = None;
            h.pending_exit = None;
            h.stale_site = None;
            h.last_fail = None;
            h.epoch_at_entry = epoch;
        }
        if let Err(e) = rtcl_vm::VmContext::charge_step(interp) {
            return Some(Err(e));
        }
        // slot 确保（驱动侧——resolve 闭包内另有 fast_ensure_slot_wt）
        let slot = match store.data().slots.get(&pool_key) {
            Some(s) => *s,
            None
                if store.data().hosts.len() < crate::abi::cache::MAX_FAST_SLOTS =>
            {
                let engine0 = store.data().engine.clone();
                let wmodule = match WT_FAST_MODULES.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(w) = m.get(&pool_key) {
                        return Ok(w.clone());
                    }
                    match wasmtime::Module::new(&engine0, &module.bytes[..]) {
                        Ok(w) => {
                            if m.len() >= 64 {
                                m.clear();
                            }
                            m.insert(pool_key, w.clone());
                            if std::env::var_os("JIT_DM").is_some() {
                                let _ = std::fs::write("/tmp/jitdbg/fm.wt.wasm", &module.bytes[..]);
                            }
                            Ok(w)
                        }
                        Err(e) => Err(e),
                    }
                }) {
                    Ok(w) => w,
                    Err(_) => return None,
                };
                let slot = store.data().hosts.len();
                let scb = crate::abi::cache::SITE_CACHE_BASE
                    + (slot as u32) * crate::abi::cache::SITE_STRIDE;
                let mut linker = fast_linker_wt(store, scb as i32);
                let host = Host::new(
                    interp as *mut Interp,
                    crate::exec::arena_of(bc),
                    bc,
                    module,
                );
                let inst = linker.instantiate(&mut *store, &wmodule).ok()?;
                let tf = inst
                    .get_typed_func::<(i32, i32, i32), (i64, i32)>(&mut *store, "proc_main")
                    .ok()?;
                let tbl = store.data().tbl();
                tbl.set(&mut *store, slot as u64, wasmtime::Ref::Func(Some(*tf.func())))
                    .ok()?;
                {
                    let mem = store.data().mem();
                    let fill = vec![0xFFu8; crate::abi::cache::SITE_STRIDE as usize];
                    let _ = mem.write(&mut *store, scb as usize, &fill);
                }
                let h = store.data_mut();
                h.hosts.push(host);
                h.scb_of.push(scb as i32);
                h.codes.push(Rc::clone(bc));
                h.runs.push(tf);
                h.slots.insert(pool_key, slot);
                slot
            }
            // 槽位上限：超出不 fast 化（回解释器）。
            None => return None,
        };
        // 入口 globals：epoch/fuel/depth（句柄先克隆，避免借用冲突）
        {
            let (ge, gf, gd, gc) = {
                let h = store.data();
                (h.epoch_g(), h.fuel_g(), h.o_depth.clone().unwrap(), h.o_caching.clone().unwrap())
            };
            let _ = ge.set(&mut *store, Val::I64(epoch as i64));
            let _ = gf.set(&mut *store, Val::I64(abi::REFUEL_CHUNK));
            let _ = gd.set(&mut *store, Val::I32(0));
            let _ = gc.set(&mut *store, Val::I32(1));
        }
        // 参数 → per-depth 切片（wrapper depth++ 后 = 1 → base = PARAMS+STRIDE）
        let arg_base =
            (crate::abi::cache::PARAMS + crate::abi::cache::PARAM_STRIDE) as usize;
        {
            let mut buf: Vec<u8> = Vec::with_capacity(meta.nparams * 8);
            for i in 0..meta.nparams {
                let name = bc.locals().get(i).cloned().unwrap_or_default();
                let v = interp.get_var(&name).cloned().unwrap_or(Value::empty());
                let tag = store.data_mut().hosts[slot].tag_value(&v);
                buf.extend_from_slice(&tag.to_le_bytes());
            }
            // wrapper 拷贝语义 = dispatch_values 形状（argv[0] 是命令名，
            // 参数在 argv+8 起）——外层驱动按同形暂存：参数写 arg_base+8。
            let mem = store.data().mem();
            mem.write(&mut *store, arg_base + 8, &buf).ok()?;
        }
        let tf = store.data().runs[slot].clone();
        crate::proc_exec::set_fast_in_call(true);
        let r = tf.call(&mut *store, (slot as i32, meta.nparams as i32, arg_base as i32));
        crate::proc_exec::set_fast_in_call(false);
        let (val, rc) = match r {
            Ok(pair) => pair,
            Err(_trap) => {
                // wasm 栈溢出（深递归）——优雅映射
                return Some(Err(Error::runtime(
                    "maximum recursion depth exceeded",
                    rtcl_core::error::ErrorCode::StackOverflow,
                )));
            }
        };
        let out = match rc {
            0 => Ok(store.data().hosts[slot].value_of(val)),
            2 => Err(Error::ret(Some(store.data().hosts[slot].value_of(val)))),
            3 => Err(Error::brk()),
            4 => Err(Error::cont()),
            5 => {
                let site = store.data().stale_site;
                match site.and_then(|s| interp.jit_redispatch_after(bc, s)) {
                    Some((true, res)) => res,
                    Some((false, _)) => Ok(Value::empty()),
                    None => Err(Error::Msg("jit fast: stale at non-boundary".into())),
                }
            }
            6 => {
                let code = store.data_mut().pending_exit.take().unwrap_or(0);
                Err(Error::exit(Some(code)))
            }
            _ => match store.data_mut().pending.take() {
                Some(p) => Err(p.err),
                None => Err(Error::Msg("jit fast: failed without pending".into())),
            },
        };
        // 出口清理（下次入口也重置——此处防御性）
        let h = store.data_mut();
        h.pending = None;
        h.stale_site = None;
        h.last_fail = None;
        Some(out)
    })
}
