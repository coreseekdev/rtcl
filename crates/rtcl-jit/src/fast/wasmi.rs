//! fast-tier 驱动（wasmi）：共享 Store + funcref 表 + call_indirect
//! 直呼 + per-depth 参数切片 + (i64 val, i32 rc) 多值返回。宿主逻辑
//! 全在 fast::shared（sh_*），本文件 = 引擎胶水。

#![allow(unsafe_code)]

use crate::abi::{self, code};
use crate::classic::compile_cached;
use crate::exec::Host;
use crate::exec;
use crate::emit;
use crate::fast::shared::{
    sh_binary_op, sh_bool_test, sh_call_dispatch, sh_fail, sh_get_var,
    sh_incr_var, sh_proc_exit_frame, sh_rc_to_i64, sh_set_var, sh_sync_var,
};
use crate::classic::PoolKey;
use rtcl_core::value::Value;
use rtcl_core::{Error, Interp};
use rtcl_ir::ByteCode;
use std::rc::Rc;

pub(crate) struct FastHost {
    /// 引擎无关共享层（逻辑真值——docs/JIT-ENGINE-MERGE.md P3-lite）。
    pub sh: crate::fast::shared::FastShared,
    /// slot → proc_main 句柄（resolve 冷融合代打）。
    pub runs: Vec<wasmi::TypedFunc<(i32, i32, i32), (i64, i32)>>,
    // 共享句柄（二阶段初始化：实体须在 Store 存在后创建——先 None 后
    // 填充；使用方经 *_g()/mem()/tbl() 取，init 后恒 Some）。
    pub engine: wasmi::Engine,
    pub memory: Option<wasmi::Memory>,
    pub table: Option<wasmi::Table>,
    pub o_fuel: Option<wasmi::Global>,
    pub o_epoch: Option<wasmi::Global>,
    pub o_caching: Option<wasmi::Global>,
    pub o_cur: Option<wasmi::Global>,
    pub o_depth: Option<wasmi::Global>,
    pub o_dirty: Option<wasmi::Global>,
}

impl FastHost {
    fn mem(&self) -> wasmi::Memory {
        self.memory.clone().expect("fast store init")
    }
    fn tbl(&self) -> wasmi::Table {
        self.table.clone().expect("fast store init")
    }
    fn fuel_g(&self) -> wasmi::Global {
        self.o_fuel.clone().expect("fast store init")
    }
    fn epoch_g(&self) -> wasmi::Global {
        self.o_epoch.clone().expect("fast store init")
    }
}

thread_local! {
    static FAST_STORE: std::cell::RefCell<Option<wasmi::Store<FastHost>>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    static FAST_MODULES: std::cell::RefCell<
        std::collections::HashMap<PoolKey, wasmi::Module>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
    static DM_SEQ: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Evict fast module entries for a retired proc body.
pub(crate) fn retire_bytecode(ptr: usize) {
    FAST_MODULES.with(|m| m.borrow_mut().retain(|k, _| k.1 != ptr));
}

/// Drop the thread-local fast store/modules when their interpreter dies.
pub(crate) fn teardown_interp(ptr: usize) {
    FAST_STORE.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell
            .as_ref()
            .is_some_and(|store| store.data().sh.interp == ptr as *mut Interp)
        {
            *cell = None;
        }
    });
    FAST_MODULES.with(|m| m.borrow_mut().retain(|k, _| k.0 != ptr));
}

/// Caller 上读 cur_proc global（每宿主调用 ~few ns——全部冷路径）。
fn read_cur(c: &mut wasmi::Caller<'_, FastHost>) -> usize {
    if let Some(wasmi::Extern::Global(g)) = c.get_export("cur_proc") {
        if let wasmi::Val::I32(v) = g.get(&mut *c) {
            return v.max(0) as usize;
        }
    }
    0
}

/// epoch 刷新：bump 只能发生在宿主 impure 路由（call_argv/resolve 的
/// 分派）里——顺带同步共享 epoch global，CallSite 的 wasm 内 epoch 门
/// 即时可见。
fn refresh_epoch(c: &mut wasmi::Caller<'_, FastHost>) {
    let h = c.data();
    let now = unsafe { (*h.sh.interp).tier1_epoch() } as i64;
    let g = h.epoch_g();
    if let wasmi::Val::I64(cur) = g.get(&mut *c) {
        if cur != now {
            let _ = g.set(&mut *c, wasmi::Val::I64(now));
        }
    }
}

/// 共享 memory 读 n 个 u64（SCRATCH/参数区协议）。
fn fast_read_words(c: &mut wasmi::Caller<'_, FastHost>, base: i32, n: i32) -> Vec<u64> {
    let n_words = n.max(0) as usize;
    let mut bytes = vec![0u8; n_words * 8];
    let mut words = vec![0u64; n_words];
    if let Some(wasmi::Extern::Memory(m)) = c.get_export("memory") {
        let _ = m.read(&mut *c, base as usize, &mut bytes);
        for (i, w) in words.iter_mut().enumerate() {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
            *w = u64::from_le_bytes(b);
        }
    }
    words
}

/// fast 版 take_fail（Host::take_fail 的 proc_ctx 臂复刻；pending 共享）。

/// fast 共享 Store 的 linker：per-instance 重建（闭包注册一次/proc，
/// 冷）；共享 memory/table/fuel/epoch/caching/cur_proc/fast_depth 的同
/// 一句柄 + 每实例独立 site_cache_base（初值 = 本 slot 的缓存段基）。
fn fast_linker<C: wasmi::AsContextMut<Data = FastHost>>(store: &mut C, site_cache_base: i32) -> wasmi::Linker<FastHost> {
    use wasmi::{Caller, Extern, Linker, Val};
    let (engine, memory, table, g_fuel, g_epoch, g_caching, g_cur, g_depth) = {
        let ctx = store.as_context();
        let h = ctx.data();
        (
            h.engine.clone(),
            h.mem(),
            h.tbl(),
            h.fuel_g(),
            h.epoch_g(),
            h.o_caching.clone().expect("fast store init"),
            h.o_cur.clone().expect("fast store init"),
            h.o_depth.clone().expect("fast store init"),
        )
    };
    let mut linker = <Linker<FastHost>>::new(&engine);
    linker.define("rtcl", "memory", Extern::Memory(memory)).unwrap();
    linker.define("rtcl", "funs", Extern::Table(table)).unwrap();
    linker.define("rtcl", "fuel", Extern::Global(g_fuel.clone())).unwrap();
    linker.define("rtcl", "epoch", Extern::Global(g_epoch.clone())).unwrap();
    linker.define("rtcl", "caching", Extern::Global(g_caching)).unwrap();
    linker.define("rtcl", "cur_proc", Extern::Global(g_cur)).unwrap();
    linker.define("rtcl", "fast_depth", Extern::Global(g_depth)).unwrap();
    let g_dirty = {
        let ctx = store.as_context();
        ctx.data().o_dirty.clone().expect("fast store init")
    };
    linker.define("rtcl", "dirty", Extern::Global(g_dirty)).unwrap();
    let g_scb = wasmi::Global::new(&mut *store, Val::I32(site_cache_base), wasmi::Mutability::Var);
    linker.define("rtcl", "site_cache_base", Extern::Global(g_scb)).unwrap();

    // ── 宿主路由（热路径不触达；冷路径经 cur_proc 路由到 per-proc Host ──
    linker.func_wrap("rtcl", "unbox_int", |mut c: Caller<'_, FastHost>, h: i32| -> (i32, i64) {
        let cur = read_cur(&mut c);
        c.data_mut().sh.hosts[cur].unbox_int_raw(h)
    }).unwrap();
    linker.func_wrap("rtcl", "unbox_float", |mut c: Caller<'_, FastHost>, h: i32| -> (i32, f64) {
        let cur = read_cur(&mut c);
        c.data_mut().sh.hosts[cur].unbox_float_raw(h)
    }).unwrap();
    linker.func_wrap("rtcl", "box", |mut c: Caller<'_, FastHost>, v: i64| -> i32 {
        let cur = read_cur(&mut c);
        c.data_mut().sh.hosts[cur].box_raw(v)
    }).unwrap();
    linker.func_wrap("rtcl", "box_float", |mut c: Caller<'_, FastHost>, f: f64| -> i32 {
        let cur = read_cur(&mut c);
        c.data_mut().sh.hosts[cur].box_float_raw(f)
    }).unwrap();
    linker.func_wrap("rtcl", "binary_op", |mut c: Caller<'_, FastHost>, op: i32, a: i64, b: i64| -> i64 {
        let cur = read_cur(&mut c);
        refresh_epoch(&mut c);
        sh_binary_op(&mut c.data_mut().sh, cur, op, a, b)
    }).unwrap();
    linker.func_wrap("rtcl", "get_var", |mut c: Caller<'_, FastHost>, name_h: i32| -> i64 {
        let cur = read_cur(&mut c);
        sh_get_var(&mut c.data_mut().sh, cur, name_h)
    }).unwrap();
    linker.func_wrap("rtcl", "set_var", |mut c: Caller<'_, FastHost>, name_h: i32, v: i64| -> i32 {
        let cur = read_cur(&mut c);
        sh_set_var(&mut c.data_mut().sh, cur, name_h, v)
    }).unwrap();
    linker.func_wrap("rtcl", "call", |_c: Caller<'_, FastHost>, _a: i32, _b: i32, _d: i32| -> i64 {
        // fast 体从不发射 site-eval 路由（RTCL_JIT_SITE_EVAL 仅经典体）
        unreachable!("call import in fast store")
    }).unwrap();
    linker.func_wrap("rtcl", "set_result", |_c: Caller<'_, FastHost>, _v: i64| {}).unwrap();
    let fuel_g = {
        let ctx = store.as_context();
        let h = ctx.data();
        h.fuel_g()
    };
    linker.func_wrap("rtcl", "refuel", move |mut c: Caller<'_, FastHost>| -> i64 {
        let chunk = abi::REFUEL_CHUNK;
        for _ in 0..chunk {
            if let Err(e) = rtcl_vm::VmContext::charge_step(unsafe { &mut *c.data().sh.interp }) {
                c.data_mut().sh.pending = Some(crate::exec::Pending { err: e, from_eval: false });
                return abi::ERR_SENTINEL;
            }
        }
        let _ = fuel_g.set(&mut c, Val::I64(chunk));
        0
    }).unwrap();
    linker.func_wrap("rtcl", "fail", |mut c: Caller<'_, FastHost>, fail_id: i32| -> i32 {
        let cur = read_cur(&mut c);
        sh_fail(&mut c.data_mut().sh, cur, fail_id)
    }).unwrap();
    linker.func_wrap("rtcl", "bool_test", |mut c: Caller<'_, FastHost>, v: i64| -> i64 {
        let cur = read_cur(&mut c);
        sh_bool_test(&mut c.data_mut().sh, cur, v)
    }).unwrap();
    linker.func_wrap("rtcl", "incr_var", |mut c: Caller<'_, FastHost>, name_h: i32, n: i64| -> i64 {
        let cur = read_cur(&mut c);
        sh_incr_var(&mut c.data_mut().sh, cur, name_h, n)
    }).unwrap();
    linker.func_wrap("rtcl", "push_const", |_c: Caller<'_, FastHost>, idx: i32| -> i64 {
        abi::const_handle(idx as u32)
    }).unwrap();
    linker.func_wrap("rtcl", "exit", |mut c: Caller<'_, FastHost>, code: i32| {
        c.data_mut().sh.pending_exit = Some(code);
    }).unwrap();
    linker.func_wrap("rtcl", "concat_n", |_c: Caller<'_, FastHost>, _b: i32, _n: i32| -> i64 {
        unreachable!("concat in fast store (gated)")
    }).unwrap();
    // v4 foreach 真路由（gate 放行后 fast 体可达）：与经典闭包同语义，
    // 经 cur_proc 路由到 per-proc Host。bind 写帧变量——调用方（wasm
    // ForeachBind 臂）随后 emit_resync_slots 拉回槽。
    linker.func_wrap("rtcl", "foreach_start", |mut c: Caller<'_, FastHost>, info: i32, base: i32, ngroups: i32| -> i64 {
        let cur = read_cur(&mut c);
        let words = fast_read_words(&mut c, base, ngroups);
        let h = &mut c.data_mut().sh.hosts[cur];
        let r = h.foreach_start_raw(info, &words);
        if r == abi::ERR_SENTINEL {
            let e = h.pending.take().map(|p| p.err)
                .unwrap_or_else(|| Error::Msg("fast foreach_start failed".into()));
            c.data_mut().sh.pending = Some(crate::exec::Pending { err: e, from_eval: false });
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_collect", |mut c: Caller<'_, FastHost>, v: i64| -> () {
        let cur = read_cur(&mut c);
        c.data_mut().sh.hosts[cur].foreach_collect_raw(v);
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_end", |mut c: Caller<'_, FastHost>| -> i64 {
        let cur = read_cur(&mut c);
        let h = &mut c.data_mut().sh.hosts[cur];
        let r = h.foreach_end_raw();
        if r == abi::ERR_SENTINEL {
            let e = h.pending.take().map(|p| p.err)
                .unwrap_or_else(|| Error::Msg("fast foreach_end failed".into()));
            c.data_mut().sh.pending = Some(crate::exec::Pending { err: e, from_eval: false });
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "eval_script", |_c: Caller<'_, FastHost>, _v: i64| -> i64 {
        unreachable!("eval_script in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "eval_expr", |_c: Caller<'_, FastHost>, _v: i64| -> i64 {
        unreachable!("eval_expr in fast store (gated)")
    }).unwrap();
    linker.func_wrap("rtcl", "lappend_var", |_c: Caller<'_, FastHost>, _n: i32, _v: i64| -> i64 {
        unreachable!("lappend in fast store (gated)")
    }).unwrap();

    // call_argv：resolve 未命中（cache=-1）后的常驻回落——jit_call_proc_direct
    // 优先，dispatch_values 兜底；④ epoch 门同 call_argv_raw。
    linker.func_wrap("rtcl", "sync_var", |mut c: Caller<'_, FastHost>, name_h: i32| -> i64 {
        let cur = read_cur(&mut c);
        sh_sync_var(&mut c.data_mut().sh, cur, name_h)
    }).unwrap();
    linker.func_wrap("rtcl", "foreach_bind", |mut c: Caller<'_, FastHost>, info: i32, idx: i32| -> i64 {
        let cur = read_cur(&mut c);
        if std::env::var_os("JIT_FT").is_some() {
            eprintln!("[ft] foreach_bind info={info} idx={idx}");
        }
        let h = &mut c.data_mut().sh.hosts[cur];
        let r = h.foreach_bind_raw(info, idx);
        if r == abi::ERR_SENTINEL {
            let e = h.pending.take().map(|p| p.err)
                .unwrap_or_else(|| Error::Msg("fast foreach_bind failed".into()));
            c.data_mut().sh.pending = Some(crate::exec::Pending { err: e, from_eval: false });
        }
        r
    }).unwrap();
    linker.func_wrap("rtcl", "call_argv", |mut c: Caller<'_, FastHost>, base: i32, argc: i32, site: i32| -> i64 {
        let cur = read_cur(&mut c);
        refresh_epoch(&mut c);
        let words = fast_read_words(&mut c, base, argc);
        let mut args: Vec<Value> = Vec::with_capacity(words.len());
        {
            let h = &c.data().sh.hosts[cur];
            for w in words {
                args.push(h.value_of(w as i64));
            }
        }
        let epoch0 = c.data().sh.epoch_at_entry;
        let (r, cross) = sh_call_dispatch(&mut c.data_mut().sh, cur, args, site, epoch0, true);
        if cross {
            if let Some(g) = c.data().o_dirty.clone() {
                let _ = g.set(&mut c, wasmi::Val::I32(1));
            }
        }
        r
    }).unwrap();

    // resolve_proc：冷融合——解析目标 → fast 化则实例化+填 cache+代打；
    // 否则 cache=-1 + 分派代打。首调后纯 wasm call_indirect。
    linker.func_wrap("rtcl", "resolve_proc", |mut c: Caller<'_, FastHost>, site: i32, argc: i32, base: i32| -> i64 {
        let cur = read_cur(&mut c);
        refresh_epoch(&mut c);
        let words = fast_read_words(&mut c, base, argc);
        let name = c.data().sh.hosts[cur].value_of(words[0] as i64).as_str().to_string();
        let interp = unsafe { &mut *c.data().sh.interp };
        let epoch0 = c.data().sh.epoch_at_entry;
        let target = interp.jit_proc_code(&name);
        // cache 写辅助：本实例 site_cache_base global + 共享 memory。
        let write_cache = |c: &mut Caller<'_, FastHost>, slot: i32| {
            let cur0 = read_cur(c);
            let base = c.data().sh.scb_of.get(cur0).copied().unwrap_or(0);
            if let Some(wasmi::Extern::Memory(m)) = c.get_export("memory") {
                let addr = (base + site * 4).max(0) as usize;
                let _ = m.write(&mut *c, addr, &slot.to_le_bytes());
            }
        };
        let slot = target.as_ref().and_then(|code| fast_ensure_slot(&mut c, interp, code));
        match slot {
            Some(slot) => {
                write_cache(&mut c, slot as i32);
                let tf = c.data().runs[slot];
                match tf.call(&mut c, (slot as i32, argc, base)) {
                    Ok((val, rc)) => sh_rc_to_i64(&mut c.data_mut().sh, val, rc),
                    Err(_) => {
                        c.data_mut().sh.pending = Some(crate::exec::Pending {
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
                    let h = &c.data().sh.hosts[cur];
                    for w in &words {
                        args.push(h.value_of(*w as i64));
                    }
                }
                // 无条件 dirty 在 sh_call_dispatch 内（非 proc 命令可能
                // 改任意帧变量——catch_error 实证）。
                let (r, cross) = sh_call_dispatch(&mut c.data_mut().sh, cur, args, site, epoch0, false);
                if cross {
                    if let Some(g) = c.data().o_dirty.clone() {
                        let _ = g.set(&mut c, wasmi::Val::I32(1));
                    }
                }
                r
            }
        }
    }).unwrap();

    // proc_exit_frame：fast 错误冷路径——本层补 (procedure "name" line N)。
    linker.func_wrap("rtcl", "proc_exit_frame", |mut c: Caller<'_, FastHost>, slot: i32| {
        sh_proc_exit_frame(&mut c.data_mut().sh, slot);
    }).unwrap();

    linker
}

/// 多值 (val, rc) → 单 i64（resolve 代打的返回契约）：OK/RETURN=值；
/// 其余置共享 pending（控制流错误显式构造；错误沿用内层已置的）。

/// 在共享 Store 内确保 slot（解析目标 → emit 缓存 → fast gate → 实例化
/// → 表槽/句柄/登记）。不可 fast 化或实例化失败 → None。
fn fast_ensure_slot(
    c: &mut wasmi::Caller<'_, FastHost>,
    interp: &mut Interp,
    code: &Rc<ByteCode>,
) -> Option<usize> {
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(code) as usize,
        code.epoch,
    );
    if let Some(s) = c.data().sh.slots.get(&pool_key) {
        return Some(*s);
    }
    let module = compile_cached(code, (Rc::as_ptr(code) as usize, code.epoch))?;
    let _meta = module.fast?; // fast gate 不命中 → None（分派回落）
    let wmodule = FAST_MODULES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(w) = m.get(&pool_key) {
            return Some(w.clone());
        }
        let engine = c.data().engine.clone();
        match wasmi::Module::new(&engine, &module.bytes[..]) {
            Ok(w) => {
                if m.len() >= 64 {
                    m.clear();
                }
                m.insert(pool_key, w.clone());
                if std::env::var_os("JIT_DM").is_some() {
                    let n = DM_SEQ.with(|c| {
                        let n = c.get();
                        c.set(n + 1);
                        n
                    });
                    let _ = std::fs::write(format!("/tmp/jitdbg/fm{n}.wasm"), &module.bytes[..]);
                }
                Some(w)
            }
            Err(_) => None,
        }
    })?;
    let slot = c.data().sh.hosts.len();
    if slot >= crate::abi::cache::MAX_FAST_SLOTS {
        // 槽位上限（内存预算/容量防线）：超出不 fast 化。
        return None;
    }
    let scb = crate::abi::cache::SITE_CACHE_BASE + (slot as u32) * crate::abi::cache::SITE_STRIDE;
    let mut linker = fast_linker(c, scb as i32);
    let host = Host::new(interp as *mut Interp, crate::exec::arena_of(code), code, &module);
    let inst = linker.instantiate_and_start(&mut *c, &wmodule).ok()?;
    let f = inst.get_export(&mut *c, "proc_main").and_then(wasmi::Extern::into_func)?;
    let tf = f.typed::<(i32, i32, i32), (i64, i32)>(&mut *c).ok()?;
    let tbl = c.data().tbl();
    tbl.set(&mut *c, slot as u64, wasmi::Ref::Func(f.into())).ok()?;
    // site-cache 段预失效（0xFF = -1）：新鲜内存是 0——slot 0 恒「命中」
    // 表[0]（第一个实例自己）→ 自调用死循环（t5 栈耗尽/judge 25 fail
    // 的根因）。
    {
        let mem = c.data().mem();
        let fill = vec![0xFFu8; crate::abi::cache::SITE_STRIDE as usize];
        let _ = mem.write(&mut *c, scb as usize, &fill);
    }
    let h = c.data_mut();
    h.sh.hosts.push(host);
    h.sh.scb_of.push(scb as i32);
    h.sh.codes.push(Rc::clone(code));
    h.runs.push(tf);
    h.sh.slots.insert(pool_key, slot);
    Some(slot)
}

/// 初始化共享 fast Store（二阶段：实体句柄须在 Store 存在后创建）。
fn init_fast_store(interp: &mut Interp) -> wasmi::Store<FastHost> {
    let engine = crate::classic::wasmi::WASMI_ENGINE.with(|e| e.get_or_init(wasmi::Engine::default).clone());
    let mut store = wasmi::Store::new(
        &engine,
        FastHost {
            sh: crate::fast::shared::FastShared::new(interp as *mut Interp),
            runs: Vec::new(),
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
    // 16 页（1MB）：site-cache 68KB + 参数切片（128B×depth——深度
    // ~5000 的余量；超出深度由 wasmi 栈限先行拦截）。
    let memory =
        wasmi::Memory::new(&mut store, wasmi::MemoryType::new(16, None)).expect("fast memory");
    let table = wasmi::Table::new(
        &mut store,
        wasmi::TableType::new(wasmi::RefType::Func, 256, None),
        wasmi::Ref::Func(wasmi::Nullable::Null),
    )
    .expect("fast table");
    let g_fuel = wasmi::Global::new(&mut store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_epoch = wasmi::Global::new(&mut store, wasmi::Val::I64(0), wasmi::Mutability::Var);
    let g_caching = wasmi::Global::new(&mut store, wasmi::Val::I32(1), wasmi::Mutability::Var);
    let g_cur = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
    let g_depth = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
    {
        let h = store.data_mut();
        h.memory = Some(memory);
        h.table = Some(table);
        h.o_fuel = Some(g_fuel);
        h.o_epoch = Some(g_epoch);
        h.o_caching = Some(g_caching);
        h.o_cur = Some(g_cur);
        h.o_depth = Some(g_depth);
    }
    {
        let g_dirty = wasmi::Global::new(&mut store, wasmi::Val::I32(0), wasmi::Mutability::Var);
        store.data_mut().o_dirty = Some(g_dirty);
    }
    store
}

/// fast-tier 尝试：槽命中直接驱动（零编译查找）；未命中编译判定
///（module.fast 存在才 fast 化——非 fast 体 None 回落常规路径，编译
/// 产物已被 PROC_MODULES 缓存，常规冷路径零额外成本）。
/// 顶层单元的 fast 执行（C4）：run_top_with 直调——JitRun 映射（rc
/// 语义 = finish() 同款：0=ok 2=return 6=exit 1=error）。
pub(crate) fn execute_fast_top_wasmi(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
    module: &emit::Module,
    meta: crate::emit::FastMeta,
) -> Option<crate::exec::JitRun> {
    let r = execute_fast(interp, bc, module, meta)?;
    Some(crate::exec::map_fast_top_run(r))
}

pub(crate) fn fast_try(interp: &mut Interp, code: &Rc<ByteCode>) -> Option<rtcl_core::Result<Value>> {
    if crate::fast::fast_in_call() {
        return None;
    }
    let key = (Rc::as_ptr(code) as usize, code.epoch);
    let module = compile_cached(code, key)?;
    let meta = module.fast?;
    execute_fast(interp, code, &module, meta)
}

/// fast-tier 外层驱动：exec_proc_body 的 fast 分支。None = 回落常规池。
fn execute_fast(
    interp: &mut Interp,
    bc: &Rc<ByteCode>,
    module: &emit::Module,
    meta: crate::emit::FastMeta,
) -> Option<rtcl_core::Result<Value>> {
    let epoch = interp.tier1_epoch();
    let pool_key = (
        interp as *mut Interp as usize,
        Rc::as_ptr(bc) as usize,
        bc.epoch,
    );
    FAST_STORE.with(|f| {
        let mut cell = f.borrow_mut();
        if cell.is_none() {
            *cell = Some(init_fast_store(interp));
        }
        {
            let store = cell.as_mut().unwrap();
            if store.data().sh.interp != interp as *mut Interp {
                *store = init_fast_store(interp);
                FAST_MODULES.with(|m| m.borrow_mut().clear());
            }
        }
        let store = cell.as_mut().unwrap();
        // 入口态
        {
            let h = store.data_mut();
            h.sh.pending = None;
            h.sh.pending_exit = None;
            h.sh.stale_site = None;
            h.sh.last_fail = None;
            h.sh.epoch_at_entry = epoch;
        }
        if let Err(e) = rtcl_vm::VmContext::charge_step(interp) {
            return Some(Err(e));
        }
        // slot 确保（驱动侧——resolve 闭包内另有 fast_ensure_slot）
        let slot = match store.data().sh.slots.get(&pool_key) {
            Some(s) => *s,
            None
                if store.data().sh.hosts.len() < crate::abi::cache::MAX_FAST_SLOTS =>
            {
                let engine0 = store.data().engine.clone();
                let wmodule = match FAST_MODULES.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(w) = m.get(&pool_key) {
                        return Ok(w.clone());
                    }
                    match wasmi::Module::new(&engine0, &module.bytes[..]) {
                        Ok(w) => {
                            if m.len() >= 64 {
                                m.clear();
                            }
                            m.insert(pool_key, w.clone());
                            if std::env::var_os("JIT_DM").is_some() {
                                let n = DM_SEQ.with(|c| {
                                    let n = c.get();
                                    c.set(n + 1);
                                    n
                                });
                                let _ = std::fs::write(format!("/tmp/jitdbg/fm{n}.wasm"), &module.bytes[..]);
                            }
                            Ok(w)
                        }
                        Err(e) => Err(e),
                    }
                }) {
                    Ok(w) => w,
                    Err(_) => return None,
                };
                let slot = store.data().sh.hosts.len();
                let scb = crate::abi::cache::SITE_CACHE_BASE
                    + (slot as u32) * crate::abi::cache::SITE_STRIDE;
                let mut linker = fast_linker(store, scb as i32);
                let host = Host::new(
                    interp as *mut Interp,
                    crate::exec::arena_of(bc),
                    bc,
                    module,
                );
                let inst = linker.instantiate_and_start(&mut *store, &wmodule).ok()?;
                let f = inst
                    .get_export(&mut *store, "proc_main")
                    .and_then(wasmi::Extern::into_func)?;
                let tf = f.typed::<(i32, i32, i32), (i64, i32)>(&mut *store).ok()?;
                let tbl = store.data().tbl();
                tbl.set(&mut *store, slot as u64, wasmi::Ref::Func(f.into())).ok()?;
                {
                    let mem = store.data().mem();
                    let fill = vec![0xFFu8; crate::abi::cache::SITE_STRIDE as usize];
                    let _ = mem.write(&mut *store, scb as usize, &fill);
                }
                let h = store.data_mut();
                h.sh.hosts.push(host);
                h.sh.scb_of.push(scb as i32);
                h.sh.codes.push(Rc::clone(bc));
                h.runs.push(tf);
                h.sh.slots.insert(pool_key, slot);
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
            let _ = ge.set(&mut *store, wasmi::Val::I64(epoch as i64));
            let _ = gf.set(&mut *store, wasmi::Val::I64(abi::REFUEL_CHUNK));
            let _ = gd.set(&mut *store, wasmi::Val::I32(0));
            let _ = gc.set(&mut *store, wasmi::Val::I32(1));
        }
        // 参数 → per-depth 切片（wrapper depth++ 后 = 1 → base = PARAMS+STRIDE）
        let arg_base =
            (crate::abi::cache::PARAMS + crate::abi::cache::PARAM_STRIDE) as usize;
        {
            let mut buf: Vec<u8> = Vec::with_capacity(meta.nparams * 8);
            for i in 0..meta.nparams {
                let name = bc.locals().get(i).cloned().unwrap_or_default();
                let v = interp.get_var(&name).cloned().unwrap_or(Value::empty());
                let tag = store.data_mut().sh.hosts[slot].tag_value(&v);
                buf.extend_from_slice(&tag.to_le_bytes());
            }
            // wrapper 拷贝语义 = dispatch_values 形状（argv[0] 是命令名，
            // 参数在 argv+8 起）——外层驱动按同形暂存：参数写 arg_base+8。
            let mem = store.data().mem();
            mem.write(&mut *store, arg_base + 8, &buf).ok()?;
        }
        let tf = store.data().runs[slot];
        crate::fast::set_fast_in_call(true);
        let r = tf.call(
            &mut *store,
            (slot as i32, meta.nparams as i32, arg_base as i32),
        );
        crate::fast::set_fast_in_call(false);
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
            0 => Ok(store.data().sh.hosts[slot].value_of(val)),
            2 => Err(Error::ret(Some(store.data().sh.hosts[slot].value_of(val)))),
            3 => Err(Error::brk()),
            4 => Err(Error::cont()),
            5 => {
                let site = store.data().sh.stale_site;
                match site.and_then(|s| interp.jit_redispatch_after(bc, s)) {
                    Some((true, res)) => res,
                    Some((false, _)) => Ok(Value::empty()),
                    None => Err(Error::Msg("jit fast: stale at non-boundary".into())),
                }
            }
            6 => {
                let code = store.data_mut().sh.pending_exit.take().unwrap_or(0);
                Err(Error::exit(Some(code)))
            }
            _ => match store.data_mut().sh.pending.take() {
                Some(p) => Err(p.err),
                None => Err(Error::Msg("jit fast: failed without pending".into())),
            },
        };
        // 出口清理（下次入口也重置——此处防御性）
        let h = store.data_mut();
        h.sh.pending = None;
        h.sh.stale_site = None;
        h.sh.last_fail = None;
        // Fast Store 是 per-thread 常驻的：把本次调用 tag 进 arena 的
        // Value 与 foreach 宿主态放掉，否则字符串/列表型 proc 会随调用
        // 次数无界增长。返回的 Value 已 clone，截断安全。
        let base = h.sh.hosts[slot].arena_base;
        h.sh.hosts[slot].arena_truncate(base);
        h.sh.hosts[slot].foreaches.clear();
        Some(out)
    })
}
