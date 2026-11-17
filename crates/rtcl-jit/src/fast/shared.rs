//! fast-tier 引擎无关共享层（Lane A 合并——docs/JIT-ENGINE-MERGE.md P3-lite）。
//!
//! wasmi（proc_exec.rs）与 wasmtime（exec_fast_wt.rs）的 fast 驱动是
//! 逐函数镜像——所有宿主闭包的**逻辑体**（路由 per-proc Host + 哨兵/
//! pending 协议 + 分派 + 跨帧传播）完全一致，只有引擎胶水（Caller/
//! Global/Memory 形态）不同。本模块把逻辑体收成单一真值：
//!
//! - `FastShared`：Store 数据的引擎无关字段（两引擎结构体嵌入 `sh`）；
//! - `sh_*`：闭包逻辑体（两引擎闭包瘦身为「引擎胶水 + sh_ 调用」）。
//!
//! 纪律：任何 fast 宿主语义修改（本会话 p05/dirty/sync 系列的教训——
//! 镜像时代每个 bug 都被迫双处修复）只改这里；引擎文件只许动胶水。

#![allow(unsafe_code)]

use crate::abi;
use crate::exec::{Host, Pending};
use rtcl_core::{Error, Interp, Value};
use rtcl_parser::ByteCode;
use std::rc::Rc;

/// fast 共享 Store 数据的引擎无关字段（引擎句柄留在各引擎结构体）。
pub(crate) struct FastShared {
    pub interp: *mut Interp,
    /// per-proc 表（arena/fails/lines/consts——Host 全方法复用）。
    pub hosts: Vec<Host>,
    /// slot → proc 体（名反查/jit_redispatch_after 用）。
    pub codes: Vec<Rc<ByteCode>>,
    pub slots: std::collections::HashMap<crate::classic::PoolKey, usize>,
    /// slot → nparams（fast_try 槽命中快臂免编译查找时取 FastMeta 的
    /// 装箱参数数；实例化时登记——与 codes 平行）。
    pub nparams_of: Vec<u32>,
    /// 一命中直查缓存（fib/oo 方法面 = 单 proc 反复调用，1 项即命中
    /// 率 ~100%）：免 slots HashMap 的 4 元组哈希探测/调用。
    pub last: Option<((usize, usize, u64, u64), usize)>,
    // 共享运行态
    pub pending: Option<Pending>,
    pub pending_exit: Option<i32>,
    pub stale_site: Option<u32>,
    pub epoch_at_entry: u64,
    pub last_fail: Option<u32>,
    /// slot → 本实例 site_cache_base 值（import 非 export，Caller 拿
    /// 不到——实例化时登记）。
    pub scb_of: Vec<i32>,
}

impl FastShared {
    pub fn new(interp: *mut Interp) -> Self {
        Self {
            interp,
            hosts: Vec::new(),
            codes: Vec::new(),
            slots: std::collections::HashMap::new(),
            nparams_of: Vec::new(),
            last: None,
            pending: None,
            pending_exit: None,
            stale_site: None,
            epoch_at_entry: 0,
            last_fail: None,
            scb_of: Vec::new(),
        }
    }

    /// 槽直查（1 项缓存优先，未命中落 HashMap）。
    pub(crate) fn slot_of(&self, key: (usize, usize, u64, u64)) -> Option<usize> {
        if let Some((k, slot)) = &self.last {
            if *k == key {
                return Some(*slot);
            }
        }
        self.slots.get(&key).copied()
    }

    /// 登记槽（slots 表 + 1 项缓存同步更新）。
    pub(crate) fn slot_insert(&mut self, key: (usize, usize, u64, u64), slot: usize) {
        self.slots.insert(key, slot);
        self.last = Some((key, slot));
    }

    /// JIT_FT 诊断开关（fast tier 逐 op 跟踪打印）。曾以
    /// `std::env::var_os` 直查——热路径每次宿主变量操作一次 getenv
    /// （oo_bench2 画像 2.25% cycles）；环境进程内不变 → 一次读缓存。
    pub(crate) fn ft_trace_on() -> bool {
        static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *V.get_or_init(|| std::env::var_os("JIT_FT").is_some())
    }

    /// JIT_VM_TRACE 诊断开关（take_fail 跟踪）——同上一次读缓存。
    pub(crate) fn vm_trace_on() -> bool {
        static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *V.get_or_init(|| std::env::var_os("JIT_VM_TRACE").is_some())
    }

    /// execute_fast 入口态重置（防御性——下次入口也重置）。
    pub fn reset_entry(&mut self) {
        self.pending = None;
        self.pending_exit = None;
        self.stale_site = None;
        self.last_fail = None;
    }

    fn irp(&self) -> &mut Interp {
        unsafe { &mut *self.interp }
    }
}

// ── 宿主闭包逻辑体（两引擎共用；cur 路由到 per-proc Host）────────

pub(crate) fn sh_binary_op(s: &mut FastShared, cur: usize, op: i32, a: i64, b: i64) -> i64 {
    let h = &mut s.hosts[cur];
    let r = h.binary_op_raw(op, a, b);
    if r == abi::ERR_SENTINEL {
        let e = h
            .pending
            .take()
            .map(|p| p.err)
            .unwrap_or_else(|| Error::Msg("fast binary_op failed".into()));
        s.pending = Some(Pending { err: e, from_eval: false });
    }
    r
}

pub(crate) fn sh_get_var(s: &mut FastShared, cur: usize, name_h: i32) -> i64 {
    let name = s.hosts[cur].const_name(name_h);
    let r = s.irp().jit_read_var_ref(&name);
    if FastShared::ft_trace_on() {
        eprintln!(
            "[ft] get_var cur={cur} name={name:?} -> {:?}",
            r.as_ref()
                .map(|v| v.as_str().to_string())
                .map_err(|e| e.message_text().to_string())
        );
    }
    match r {
        Ok(v) => s.hosts[cur].tag_value(&v),
        Err(e) => {
            s.pending = Some(Pending { err: e, from_eval: false });
            abi::ERR_SENTINEL
        }
    }
}

pub(crate) fn sh_set_var(s: &mut FastShared, cur: usize, name_h: i32, v: i64) -> i32 {
    let (name, val) = {
        let h = &mut s.hosts[cur];
        (h.const_name(name_h), h.value_of(v))
    };
    match s.irp().set_var(&name, val) {
        Ok(_) => 0,
        Err(e) => {
            s.pending = Some(Pending { err: e, from_eval: false });
            1
        }
    }
}

pub(crate) fn sh_fail(s: &mut FastShared, cur: usize, fail_id: i32) -> i32 {
    let fid = fail_id.max(0) as u32;
    s.last_fail = Some(fid);
    let r = sh_take_fail(s, cur, fid);
    if FastShared::ft_trace_on() {
        eprintln!("[ft] fail cur={cur} fid={fid} -> {r}");
    }
    r
}

pub(crate) fn sh_bool_test(s: &mut FastShared, cur: usize, v: i64) -> i64 {
    let r = s.hosts[cur].bool_test_raw(v);
    if r == abi::ERR_SENTINEL {
        let e = s.hosts[cur]
            .pending
            .take()
            .map(|p| p.err)
            .unwrap_or_else(|| Error::Msg("fast bool_test failed".into()));
        s.pending = Some(Pending { err: e, from_eval: false });
    }
    r
}

pub(crate) fn sh_incr_var(s: &mut FastShared, cur: usize, name_h: i32, n: i64) -> i64 {
    let r = s.hosts[cur].incr_var_raw(name_h, n);
    if FastShared::ft_trace_on() {
        eprintln!("[ft] incr_var cur={cur} n={n} -> {r}");
    }
    if r == abi::ERR_SENTINEL {
        let e = s.hosts[cur]
            .pending
            .take()
            .map(|p| p.err)
            .unwrap_or_else(|| Error::Msg("fast incr failed".into()));
        s.pending = Some(Pending { err: e, from_eval: false });
    }
    r
}

pub(crate) fn sh_sync_var(s: &mut FastShared, cur: usize, name_h: i32) -> i64 {
    let name = s.hosts[cur].const_name(name_h);
    let r = match s.irp().get_var(&name).cloned() {
        Ok(v) => s.hosts[cur].tag_value(&v),
        Err(_) => 0,
    };
    if FastShared::ft_trace_on() {
        let pv = if r == 0 {
            "<none>".into()
        } else {
            s.hosts[cur].value_of(r).as_str().to_string()
        };
        eprintln!("[ft] sync {name} = {pv}");
    }
    r
}

/// call_argv / resolve_proc 分派臂共用：jit_call_proc_direct（direct）
/// 或 dispatch_values 直连分派 → 跨帧哨兵消费 → epoch 门 → tag/哨兵。
/// 返回 (tagged|ERR_SENTINEL, cross_frame)——dirty 置位是引擎胶水。
pub(crate) fn sh_call_dispatch(
    s: &mut FastShared,
    cur: usize,
    args: Vec<Value>,
    site: i32,
    epoch0: u64,
    direct: bool,
) -> (i64, bool) {
    let r = if direct {
        match s.irp().jit_call_proc_direct(args[0].as_str(), &args) {
            Some(r) => r,
            None => s.irp().dispatch_values(&args),
        }
    } else {
        s.irp().dispatch_values(&args)
    };
    // 无条件 dirty：非 proc 命令的宿主分派可能改任意帧变量
    //（lappend/eval 型 catch…）；proc 直呼的 Some 臂不经此处。
    // 仅靠跨帧哨兵会漏（judge catch_error 实证）。跨帧哨兵仍在
    // dispatch 后取走（upvar/global 语义独立于命令类）。
    let cross = rtcl_vm::interp::jit_take_cross_frame();
    let _ = cross;
    match r {
        Ok(v) => {
            if s.irp().tier1_epoch() != epoch0 {
                s.stale_site = Some(site.max(0) as u32);
                s.pending = Some(Pending {
                    err: Error::Msg(crate::exec::STALE_MARK.to_string()),
                    from_eval: false,
                });
                return (abi::ERR_SENTINEL, true);
            }
            let tagged = s.hosts[cur].tag_value(&v);
            (tagged, true)
        }
        Err(e) => {
            s.pending = Some(Pending { err: e, from_eval: false });
            (abi::ERR_SENTINEL, true)
        }
    }
}

pub(crate) fn sh_proc_exit_frame(s: &mut FastShared, slot: i32) {
    let slot = slot.max(0) as usize;
    let code = s.codes.get(slot).cloned();
    if let Some(code) = code {
        let ip = s.irp();
        if let Some(name) = ip.jit_proc_name_of(&code) {
            let tag = format!("procedure \"{}\"", name);
            ip.jit_err_exit_frame(&tag);
        }
    }
}

/// fast 版 take_fail（Host::take_fail 的 proc_ctx 臂复刻；pending 共享）。
pub(crate) fn sh_take_fail(s: &mut FastShared, cur: usize, fail_id: u32) -> i32 {
    if FastShared::vm_trace_on() {
        eprintln!("[fast] take_fail cur={cur} fid={fail_id}");
    }
    use crate::ir::BodyKind;
    let Some(p) = s.pending.take() else { return 0 };
    let e = p.err;
    if e.message_text() == crate::exec::STALE_MARK {
        return 6;
    }
    let kind = crate::exec::classify_control(&e);
    if kind != 0 {
        // kind 5：Next 区域 continue 逃逸（fail 表 esc 已在快照期解析
        // ——经典 take_fail 同口径；缺此转换时 eval 型 continue 落
        // kind 2 → 单元 CONTINUE → proc 边界转错，gen_for 实证）。
        if kind == 2
            && s.hosts[cur]
                .fails
                .get(fail_id as usize)
                .is_some_and(|f| f.esc.is_some())
        {
            return 5;
        }
        return kind;
    }
    let Some(fi) = s.hosts[cur].fails.get(fail_id as usize).cloned() else {
        s.pending = Some(Pending { err: e, from_eval: false });
        return 0;
    };
    let irp = s.irp();
    let frameless = fi.bodies.last().is_some_and(|(_, st)| *st == fi.site);
    if fi.site != u32::MAX && !frameless {
        let text = s.hosts[cur].site_text_of(fi.site);
        let line = s.hosts[cur].lines_of(fi.site);
        let msg = e.message_text();
        irp.jit_err_set_line(line);
        irp.jit_err_harness_frame(&msg, &text, line);
    }
    for (i, (bkind, site)) in fi.bodies.iter().enumerate().rev() {
        let text = s.hosts[cur].site_text_of(*site);
        let line = s.hosts[cur].lines_of(*site);
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
            BodyKind::Body | BodyKind::Next => {
                // 文件语境体帧（vm unwind_subs 同臂；仅最外层体区。proc
                // 体 = 编译语境——call_proc 已强制 in_file_script=false，
                // 此臂自然不触发）。
                let is_next = matches!(bkind, BodyKind::Next);
                let outermost = !fi.bodies[..i].iter().any(|(k, _)| {
                    matches!(
                        k,
                        BodyKind::Body | BodyKind::Next | BodyKind::BodyFramed(_)
                    )
                });
                if !outermost || !irp.jit_err_in_file_script() {
                    continue;
                }
                let Some(first_word) = s.hosts[cur].site_first_word_of(*site) else {
                    continue;
                };
                let msg = e.message_text();
                match first_word.as_str() {
                    "while" if !is_next => {
                        let n = irp
                            .jit_err_line()
                            .saturating_sub(s.hosts[cur].body_open_line_of(*site).saturating_sub(1));
                        irp.jit_err_append_info_raw(&format!("\n    (\"while\" body line {n})"));
                        irp.jit_err_harness_frame(&msg, &text, line);
                    }
                    "for" => {
                        if is_next {
                            irp.jit_err_exit_frame_raw("\"for\" loop-end command");
                        } else {
                            let n = irp
                                .jit_err_line()
                                .saturating_sub(s.hosts[cur].body_open_line_of(*site).saturating_sub(1));
                            irp.jit_err_append_info_raw(&format!("\n    (\"for\" body line {n})"));
                        }
                        irp.jit_err_harness_frame(&msg, &text, line);
                    }
                    "if" => {
                        irp.jit_err_harness_frame(&msg, &text, line);
                    }
                    _ => {}
                }
            }
        }
    }
    s.pending = Some(Pending { err: e, from_eval: false });
    0
}

/// 多值 (val, rc) → 单 i64（resolve 代打的返回契约）：OK/RETURN=值；
/// 其余置共享 pending（控制流错误显式构造；错误沿用内层已置的）。
pub(crate) fn sh_rc_to_i64(s: &mut FastShared, val: i64, rc: i32) -> i64 {
    match rc {
        0 | 2 => val,
        3 | 4 => {
            let e = if rc == 3 { Error::brk() } else { Error::cont() };
            s.pending = Some(Pending { err: e, from_eval: false });
            abi::ERR_SENTINEL
        }
        5 => {
            s.pending = Some(Pending {
                err: Error::Msg(crate::exec::STALE_MARK.to_string()),
                from_eval: false,
            });
            abi::ERR_SENTINEL
        }
        6 => abi::ERR_SENTINEL, // EXIT：pending_exit 已置
        _ => abi::ERR_SENTINEL, // ERROR：内层 fail 已置 pending+帧
    }
}
