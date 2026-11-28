//! Tcl interpreter — executes parsed commands.
//!
//! The interpreter is split into:
//! - This file: [`Interp`] struct definition and constructor
//! - [`registry`]: built-in command registration and CmdId dispatch
//! - [`vars`]: variable access methods
//! - [`eval`]: script evaluation and word expansion
//! - [`call`]: procedure calls and tail-call optimisation
//! - [`vm_bridge`]: [`VmContext`](crate::VmContext) implementation
//! - [`util`]: shared helpers (`split_array_ref`, `glob_match`)
//! - [`commands`] submodules: individual command implementations

pub mod commands;
// unicode 表已抽独立 crate（rtcl-regexp-spencer 双端共用同一份 Tcl
// 分类/大小写表）——re-export 保持 crate::interp::unicode 路径不变。
pub use rtcl_unicode as unicode;
mod registry;
mod vars;
mod eval;
mod call;
mod vm_bridge;
mod vm_exec;
pub(crate) mod util;
mod varmap;
// D1 变量表重写步骤 1（specs/ARCH-DESIGN-20261126.md D1）：基础设施先
// 落（本模块自带测试）；读写面接线在步骤 1 迁移提交接入——接线后该
// 模块顶部的 dead_code 暂免收窄到未接线句柄 API。
pub(crate) mod vararena;
pub(crate) mod tclhash;
pub(crate) mod dbgtrace;

// Re-export utilities so command modules can reach them via `super::super::glob_match`
pub(crate) use util::{split_array_ref, glob_match};
pub(crate) use varmap::{VarMap, VarSet};

pub use vm_exec::{add_interp_teardown_hook, set_bytecode_disabled, set_bytecode_retire_hook, set_interp_teardown_hook, set_proc_body_executor, set_proc_precompile_hook, ByteCodeRetireHook, InterpTeardownHook, ProcBodyExecutor, ProcPrecompileHook};

use crate::command::{CommandFunc, CommandCategory, CommandMeta};
use crate::value::Value;
use rtcl_parser::ByteCode;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeSet as HashSet;

#[cfg(not(feature = "embedded"))]
use std::cell::Cell;
use std::rc::Rc;

#[cfg(feature = "embedded")]
use alloc::rc::Rc;

/// A procedure definition.
#[derive(Debug, Clone)]
pub(crate) struct ProcDef {
    /// Shared behind `Rc<ProcDef>` and cloned per call site, so the heavy
    /// fields are `Rc` themselves — a call then bumps pointers instead of
    /// deep-copying the parameter list, the body text and the statics map
    /// (tclsh's proc record is likewise shared until redefined).
    pub params: Rc<Vec<(String, Option<String>)>>,
    pub body: Rc<str>,
    /// Static variables: persist across calls. Key = var name, value = current value.
    pub statics: Rc<HashMap<String, Value>>,
    /// Body compiled once at definition time (named `proc` only; apply
    /// lambdas and OO synthetics stay on the tree-walk, which their
    /// per-construction lifetime would otherwise recompile per call).
    /// `None` = the body does not parse (the tree-walk reproduces the
    /// parse error per call, exactly like an uncached parse).
    pub compiled: Option<Rc<ByteCode>>,
}

/// A link from a local variable name to a variable in another scope.
#[derive(Debug, Clone)]
pub(crate) enum UpvarLink {
    /// Link to `globals[name]`.  `Rc<str>`: the canonical key is memoised
    /// in `var_link_cache` and handed to the frame's upvar map, slot
    /// `link_keys` and `globals` probes by refcount — a `variable` link
    /// installed on every method call allocated four strings here.
    Global(Rc<str>),
    /// Link to `frames[frame_index].locals[name]`.
    Frame { frame_index: usize, var_name: String },
    /// The link's target was destroyed by a `namespace delete` (or the
    /// containing array was unset) while the alias was live.  Reads keep
    /// working against the alias's seeded local copy; writes error
    /// (tclsh 8.6.17).  `array` distinguishes the two error messages.
    /// `was_global`: the target was a GLOBAL key (`::ns::v`/`g(e)`) —
    /// the read fallback may consult `globals[target]`; a frame-local
    /// dead target must NOT (a same-named global element would leak
    /// through an alias that never pointed at it — trv_p11 upvar-elem
    /// 的帧局部占位 `arr(1)` vs 全局 `arr(1)`).
    Dead { array: bool, was_global: bool, target: String },
}

/// Frame upvar links.  `Rc<str>` keys: a prelinked method's link names
/// are memoised (`ProcMemo`) and re-inserted on every call, so the key is
/// a refcount bump rather than a fresh `String`.
#[cfg(not(feature = "embedded"))]
pub(crate) type UpvarMap = varmap::FxHashedMap<Rc<str>, UpvarLink>;

/// One OO-method `variable` prelink: the declared name plus the
/// once-ness flag of its namespace-set registration (the registering
/// namespace is the object's own, whose lifetime is the memo's — a
/// re-created namespace implies a retired chain memo).
#[derive(Debug)]
pub(crate) struct PrelinkEntry {
    pub(crate) name: Rc<str>,
    /// `resolve_var_link`'s deterministic result — pure string math
    /// memoised by that call; precomputed with the prelink list so the
    /// per-call install skips the probe entirely.
    pub(crate) qualified: Rc<str>,
    pub(crate) ns_part: Rc<str>,
    pub(crate) registered: core::cell::Cell<bool>,
    /// The frame slot holding `name` in the linked body's compiled locals
    /// (precomputed with the list; the installing frame IS that body's, so
    /// the per-call link install skips the table scan).  `None` when the
    /// body has no slot for it (name-keyed frames fall back).
    pub(crate) slot: Option<u32>,
}
#[cfg(feature = "embedded")]
pub(crate) type UpvarMap = alloc::collections::BTreeMap<Rc<str>, UpvarLink>;

/// A live `namespace ensemble` — the ensemble command's dispatch table
/// (tclsh 8.6.17).  Keyed in `Interp::ensembles` by the ensemble's
/// registered command name (canonical, `::`-prefixed).
#[derive(Debug, Clone)]
pub(crate) struct EnsembleDef {
    /// Backing namespace the ensemble was created in (configure
    /// -namespace; subcommand implementations resolve here).
    pub namespace: String,
    /// `-map`: subcommand → implementation command prefix (the prefix's
    /// first word is qualified against the backing namespace at set time).
    pub map: Vec<(String, Vec<String>)>,
    /// `-prefixes`: unique-prefix subcommand matching (default on).
    pub prefixes: bool,
    /// `-subcommands`: explicit candidate list overriding the namespace's
    /// exports (None = use the exports).
    pub subcommands: Option<Vec<String>>,
    /// `-unknown`: handler command prefix, stored as typed.
    pub unknown: Option<Vec<String>>,
    /// `-parameters` names — that many leading words are always consumed
    /// as parameter values and prepended to the impl's args.
    pub parameters: Vec<String>,
}

/// The outcome of one command-name resolution (`dispatch_values`'s
/// procs/ensembles/builtins chain), memoised in [`Interp::cmd_cache`].
/// tclsh caches resolved `Command *` tokens at its call sites; rtcl
/// re-resolved per call, walking the full chain — including two
/// `format!("::{}", name)` probe keys — for every dispatched command
/// (stdlib's `::tcl::tm` ensemble means the engines' fast-path guards on
/// "no ensembles registered" never fired in a real session).  Holding the
/// fn pointer / `Rc<ProcDef>` is safe because every command-table mutation
/// bumps [`Interp::cmd_generation`], a mismatch being a miss.  Only the
/// proc-hit and builtin-hit outcomes are cached; ensemble/unknown/
/// unresolved names re-run the chain every time (they sit outside hot
/// loops, and their outcomes depend on maps worth keeping out of the
/// staleness audit).
#[derive(Debug, Clone)]
pub(crate) enum ResolvedCmd {
    /// A built-in won.  Which arm of the fallback chain found it doesn't
    /// matter — the cached outcome is "call this fn".
    Builtin(CommandFunc),
    /// A proc won, registered under `key` — tclsh's command token.  The
    /// def is NOT cached here (unlike the call-site token, whose
    /// generation check now covers the statics write-back bump): the
    /// resolution cache outlives many generations of nothing in
    /// particular and `call_proc`'s per-hit re-fetch of `procs[key]` is
    /// one probe.  The name doubles as what `call_proc` reports in
    /// frames (`(procedure "key")`).
    Proc(Rc<str>),
}

/// One [`Interp::cmd_cache`] entry: the target plus the command-table
/// generation it was resolved under.
#[derive(Debug, Clone)]
pub(crate) struct CachedCmd {
    pub gen: u64,
    pub target: ResolvedCmd,
}

/// A procedure call frame.
#[derive(Debug, Clone)]
pub(crate) struct CallFrame {
    /// 帧变量表（D1 步骤 2，specs/ARCH-DESIGN-20261126.md D1）：名键面
    /// 的权威存储（`locals` 平表 + `array_locals` 位集 + `elem_counts`
    /// 计数三合一——镜像步骤 5 退役），形状 = 步骤 1 的
    /// [`vararena::VarTable`]（TableId::Frame(帧栈下标) 身份）。读路径
    /// 经 `vars.rs` 的 frame_local_* 咽喉。生命周期随帧：池化归还/
    /// 尾调重绑/trace 清场走 [`CallFrame::reset_var_state`]（表清空 +
    /// 代数递增——池化复用不冲突）；协程排水随帧整体进出 record（表
    /// 随帧）。插入序向量与 `locals_order` 日志同语义（日志为 T3 直
    /// 列的 release 重放源——步骤 5 记账保留）。
    pub vt: vararena::VarTable,
    /// Tcl hash-order log for this frame's var table (`info locals` /
    /// `info vars` in proc scope replay it; reset with the table on
    /// frame recycle so every call rebuilds it from param order).
    /// release 特性——T3 直列仍读它重放（链接条目只入日志
    /// frame_log_link 不入 vt；vt 插入序与日志序的等价由步骤 2-4 迁
    /// 移期对拍逐字节验证，重放源保留见 BASELINE 记账）。
    pub locals_order: Vec<String>,
    /// `const`-marked variable names in this frame (tclsh 9.1's
    /// VAR_CONSTANT flag, TIP 508-era).  `None` while the frame declares
    /// no constants — the write guards in `vars.rs` probe this first so
    /// const-free frames pay one branch.  Cleared in
    /// [`CallFrame::reset_var_state`].
    pub consts: Option<Vec<String>>,
    /// 帧内链接登记表（别名名 → 目标）。D1 步骤 5 决策：**保留**，不
    /// 收编 Var.link——链接名可无 vt 条目（link-only，T3 直列经
    /// locals_order 日志 + in_table 过滤列举）；三链接形含 Dead（惰性
    /// 目标探测即语义）；Frame 链接的跨帧句柄在池化重置下不稳定
    /// （代数守卫换不来 Dead 判定）。全部热径已有 is_empty 单分支守
    /// 卫，收编无可得收益、语义风险在字节级红线面。
    pub upvars: UpvarMap,
    /// Namespace the running proc was defined in (tclsh: `namespace current`
    /// inside the proc resolves here).  Variable *reads* do NOT fall back to
    /// this namespace — only locals, upvar/`variable` links, and `::`-qualified
    /// names are visible in a proc frame.  `Rc<str>` — written per call.
    pub ns: Option<Rc<str>>,
    /// Namespace active at CALL time — the context of the caller's script
    /// (e.g. the body of a `namespace eval` this proc was invoked from).
    /// `uplevel` relative levels target this context when the caller is
    /// not a proc frame (tclsh 8.6.17).
    pub call_ns: Option<Rc<str>>,
    /// Commands created by `local` — deleted when this frame exits.
    pub local_procs: Vec<String>,
    /// Scripts registered by `defer` — executed in reverse order on frame exit.
    pub deferred_scripts: Vec<String>,
    /// E2 slot-compiled locals: the frame's variable values by slot index
    /// (`None` = currently unset — a `set` target whose write was skipped
    /// by control flow).  Empty when the frame is not slot-compiled, or
    /// after it degraded back to the name-keyed store: slots and `locals`
    /// never hold the same name at once, they alias (every name-keyed
    /// read/write consults [`CallFrame::slot_index_of`] first, so
    /// uncompiled writers — foreach vars, `lappend`, `catch` results —
    /// observe and mutate the same variable the compiled ops touch).
    /// 8-byte elements: an unset slot is the `Value::unset()` sentinel
    /// (bits 0 — unreachable for real values), not an Option wrapper.
    pub slots: Vec<Value>,
    /// Parallel to `slots`: a slot whose storage moved to `locals` because
    /// an exceptional structure was installed on its NAME (an upvar /
    /// `global` / `variable` link — see
    /// [`Interp::degrade_frame_link`]).  Slot ops consult this one bool
    /// (an indexed read, no hashing) and defer to the name path, which
    /// resolves the link; every other slot keeps its compiled fast path,
    /// so a linked method variable no longer degrades the WHOLE frame to
    /// name-keyed lookups.
    pub slot_aliased: Vec<bool>,
    /// Frame-level short-circuit for the `slot_aliased` scan: true once
    /// ANY slot of this frame has been link-aliased (`upvar` / `global` /
    /// `variable`).  The hot slot ops test this one bool first — a frame
    /// that never saw a link (the overwhelmingly common case) skips the
    /// per-slot Vec<bool> probe entirely.  Cleared with `slot_aliased`.
    pub has_aliased: bool,
    /// Parallel to `slots`: for an aliased slot holding a GLOBAL link
    /// (`variable`/`global`/upvar-to-global), the canonical target key —
    /// slot ops resolve through ONE hash (`globals.get(key)`) instead of
    /// the name path's two (upvars probe + globals probe).  `None` for
    /// non-linked slots and for link kinds the direct path can't serve
    /// (frame-to-frame upvar, dead targets).
    pub link_keys: Vec<Option<std::rc::Rc<str>>>,
    /// The compiled unit the slots belong to; its `locals()` table names
    /// slot i.  `Rc` — one bump per call, taken on degrade.
    pub slot_table: Option<Rc<ByteCode>>,
    /// The proc's compiled-locals table, in slot mode and out of it
    /// alike (it stays when the frame degrades).  It names the locals
    /// tclsh's compiler gave a slot in its compiledLocals array: their
    /// entry exists before the first `set` and after `unset`, and they
    /// are listed before the locals made at run time.  `Rc` — one bump
    /// per call.
    pub compiled: Option<Rc<ByteCode>>,
    /// Deferred `tailcall` command, armed by the `tailcall` builtin and
    /// fired at frame exit — kept on the frame (not just the completion
    /// error) so a `catch` consuming the completion still lets it fire,
    /// and a second `tailcall` overwrites it (tclsh tailcall-12.3).
    pub tailcall: Option<Vec<String>>,
    /// `info level 0` for this frame: the invocation words as dispatched
    /// (as-typed command name + evaluated arguments).  Stored raw —
    /// rendering the list form cost an allocation + quoting walk per call
    /// that the ~never-asked `info level 0` doesn't justify (tclsh renders
    /// from the live argv on demand).  [`frame_level0`] renders it.
    /// A plain reused buffer: the frame pool carries the allocation across
    /// calls (cleared, not dropped — a proc call pays no Vec malloc for
    /// this), and emptiness doubles as "not a bound invocation" (a
    /// dispatched command always has at least its name word).
    pub level0: Vec<Value>,
    /// How many `namespace eval`s were open when this frame was created —
    /// reconstructs the tclsh varFrame chain (proc frames and ns-eval
    /// scopes interleave) for `uplevel` level arithmetic.
    pub ns_depth: usize,
}

impl CallFrame {
    /// 帧名键面清空（D1 步骤 2）：`vt` 整表重置（代数逐槽递增——池化
    /// 复用旧句柄全灭）+ 插入日志清空。帧池归还、尾调重绑、trace 清
    /// 场三处共用。
    pub(crate) fn reset_var_state(&mut self) {
        self.vt.reset();
        self.locals_order.clear();
    }

    /// 刷新帧表的表身份（推栈点——池化帧可换帧栈深度复用；协程 resume
    /// 重推同款）。句柄产出据此携带当次栈位。
    pub(crate) fn bind_var_table(&mut self, frame_idx: usize) {
        self.vt
            .set_owner(vararena::TableId::Frame(frame_idx as u32));
    }

    /// Slot index of `name` in this frame's compiled locals table, when
    /// the frame is slot-compiled (`None` for name-keyed/degraded frames
    /// or table misses — the caller then takes its ordinary path).  A
    /// linear scan, like tclsh's compiledLocals lookup; tables are small
    /// (params + `set`/`incr` targets).
    pub(crate) fn slot_index_of(&self, name: &str) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        self.slot_table
            .as_ref()?
            .locals()
            .iter()
            .position(|n| n == name)
    }

    /// Is `name` one of the proc's compiled locals (see
    /// [`Self::compiled`])?  A linear scan, for the cold paths only.
    pub(crate) fn is_compiled_local(&self, name: &str) -> bool {
        self.compiled
            .as_ref()
            .is_some_and(|t| t.locals().iter().any(|l| l == name))
    }

    /// Current value of the named slot (`None` when unset or absent).
    pub(crate) fn slot_value(&self, name: &str) -> Option<&Value> {
        let i = self.slot_index_of(name)?;
        if self.has_aliased && self.slot_aliased.get(i).copied().unwrap_or(false) {
            return None;
        }
        let v = &self.slots[i];
        if v.is_unset() {
            return None;
        }
        Some(v)
    }
}

/// `info level 0` string for a frame: the invocation words, list-rendered
/// on demand from [`CallFrame::level0`].
pub(crate) fn frame_level0(frame: &CallFrame) -> String {
    if frame.level0.is_empty() {
        return String::new();
    }
    Value::from_list(&frame.level0).as_str().to_string()
}

/// Identity hasher for the const pool's pointer keys: ByteCode addresses
/// are already unique and well-spread, so "hashing" is a move, not a mix
/// (std's keyed SipHash on the per-`exec_bytecode` probe was ~1.5% of the
/// fib profile).  Only `write_usize` is exercised (`Hash for usize`).
#[cfg(not(feature = "embedded"))]
#[derive(Default)]
pub(crate) struct PtrHasher(u64);

#[cfg(not(feature = "embedded"))]
impl core::hash::Hasher for PtrHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0 = n as u64;
    }
    fn write(&mut self, bytes: &[u8]) {
        // Never reached through usize keys; kept total for safety.
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ b as u64;
        }
    }
}

/// One unit's per-call-site resolved-command token table (keyed by the
/// op index of the `Call`/`DynCall` op, sorted): shared through the pool
/// entry so one pool probe per `exec_bytecode` reaches every depth
/// executing the unit.
pub(crate) type CmdSites = Rc<core::cell::RefCell<Vec<(u32, CmdSite)>>>;

/// One loaded unit's pooled constants plus its call-site token table —
/// the pair [`Interp::const_pool_get`] hands the executor (a single pool
/// probe serves both halves, so they travel together).
pub(crate) type LoadedUnitParts = (Rc<[Value]>, CmdSites);

/// One [`Interp::const_pool`] entry: a compiled unit's string constants
/// pre-materialised as `Value`s (tclsh's literal table).  Holds the
/// `Rc<ByteCode>` keepalive so the address key stays valid for the
/// entry's lifetime.
pub(crate) struct ConstPoolEntry {
    /// The compiled unit the values belong to.  NOT dead code despite
    /// never being read: the pool is keyed by `Rc::as_ptr(code)`, so the
    /// entry must hold the unit alive or a freed unit's address could be
    /// reused by a new unit and served this stale entry's constants.
    #[allow(dead_code)]
    pub code: Rc<ByteCode>,
    /// `code.constants()` as `Value`s, in table order — one slice serves
    /// both `PushConst` (u16) and `PushConstWide` (u32), which index the
    /// same table.
    pub values: Rc<[Value]>,
    /// Per-call-site resolved-command tokens (tclsh caches a resolved
    /// `Command *` on its invoke instructions the same way): keyed by the
    /// op index of the `Call`/`DynCall` op, sorted.  Shared through the
    /// entry so one pool probe per `exec_bytecode` reaches both the
    /// constants and these — the executor pays no per-call hashing.
    pub sites: CmdSites,
}

/// One call-site resolution token: the dispatch outcome a specific
/// compiled call op resolved to, valid while the command-table generation,
/// the invocation name, and the resolving namespace all hold (a
/// substituted command word or a namespace switch re-resolves).
#[derive(Debug, Clone)]
pub(crate) struct CmdSite {
    pub gen: u64,
    /// Invocation word this resolution answered (the op may see a
    /// different substituted name each execution).
    pub name: Rc<str>,
    /// Namespace the resolution ran under (`cmd_cache`'s outer key).
    pub ns: Rc<str>,
    pub target: ResolvedCmd,
    /// The proc def when `target` is `Proc` — the token hands the def
    /// straight to `call_proc`, skipping the per-call `procs[key]`
    /// probe (fib profile: hash + key compare + find ≈ 14% of the
    /// whole bench).  Validity: every `procs` mutation bumps
    /// `cmd_generation` — INCLUDING the statics write-backs, which
    /// replace the map entry (bumped as of this token's introduction;
    /// that was the one unbumped mutation and the reason the def
    /// couldn't ride the token before).
    pub def: Option<Rc<ProcDef>>,
}

/// One live `array startsearch` iteration over an array's element names.
/// A snapshot of the element set plus the array's mutation stamp: any
/// element added or removed (tclsh semantics) invalidates the search.
pub(crate) struct ArraySearch {
    /// Rendered identifier, e.g. `s-1-a` — includes the array name as given.
    pub rendered: String,
    /// Remaining element names, in snapshot order.
    pub elements: Vec<String>,
    /// Snapshot of the array's stamp when the search started.
    pub stamp: u64,
}

/// Per-array search table: tclsh's monotonic search counter (reset when the
/// last search goes away) plus the live searches.
pub(crate) struct ArraySearchList {
    pub ctr: i64,
    pub active: Vec<ArraySearch>,
}

/// One registered `trace add variable` callback: canonical ops (sorted
/// array/read/unset/write) plus the script invoked with `name1 name2 op`
/// appended.  D1 步骤 4 起数据本体驻 [`vararena::TraceSet`]（挂 Var），
/// 本类型是记录形状。
#[derive(Clone, Debug)]
pub(crate) struct VarTrace {
    pub ops: Vec<String>,
    pub script: String,
}

/// One registered `trace add execution` callback: identity (tclsh's
/// TraceObservation record — `trace remove` matches the *registration*, so a
/// remove+re-add is a different record even if byte-identical), canonical ops
/// (sorted enter/leave/enterstep/leavestep) plus the script invoked with the
/// command text and op appended.
#[derive(Clone)]
pub(crate) struct ExecTrace {
    pub id: u64,
    pub ops: Vec<String>,
    pub script: String,
}

/// Enterstep bookkeeping for one dispatching command (see
/// [`Interp::exec_step_begin`], implemented in `commands::trace`).
pub(crate) struct ExecStepCtx {
    /// Resolved keys of EVERY traced proc on the stack (tclsh dynamic
    /// range, D-39④) with their enterstep-time record ids.
    pub steps: Vec<(String, Vec<u64>)>,
    /// The command's words, list-rendered (tclsh's substituted text).
    pub cmdtext: String,
}

/// Tcl interpreter.
/// 顶层 slot 单元的槽缓存（C2'）：values 仅归**单元槽 op**读写；
/// 名路径（分派中的 set_var/get_var/…）只触全局。同步 = 边界
///（非纯 op 前）flush（set 槽 → 全局）+ invalidate（全槽 → unset，
/// 读回落全局拉新）。纯 op 循环零全局触达 = 提速来源。
pub(crate) struct TopSlots {
    pub names: Vec<String>,
    pub values: Vec<Value>,
    /// 名已数组化/链接（槽 op 回落名路径，is-array 错误归属全链）。
    pub aliased: Vec<bool>,
}

/// The recursion/nesting limit a fresh interpreter starts with, sized
/// for the stack the platform gives it. Exceeding it must be Tcl's
/// catchable error, never a stack overflow (abort; on wasm a trap that
/// leaves the instance unusable).
///
/// - **native: 1000** (tclsh's `recursionlimit`). 999 levels need about
///   4MB whatever the shape — a plain call, an `apply` inside `catch`,
///   recursion inside `switch`, all the heavy constructs nested — so the
///   default 8MB main-thread stack carries a 2x margin. A thread with a
///   smaller stack needs [`Interp::set_max_call_depth`] (about 5000 bytes
///   a level).
/// - **wasm32: 150.** The engine's stack is about 1MiB (V8 carries wasm
///   frames on the JS stack) and a level costs 1.4KB (a plain call, which
///   overflowed at 736) to 5.2KB (`apply` inside `catch`, which overflowed
///   at 200; measured in Node 22). With 1000 every recursion overflowed
///   the stack before the limit was reached.
///
/// A depth count cannot see how heavy each level is; a host that knows
/// its stack can set the limit to match.
fn default_max_call_depth() -> usize {
    if cfg!(target_arch = "wasm32") {
        150
    } else {
        1000
    }
}

pub struct Interp {
    /// D1 步骤 1（specs/ARCH-DESIGN-20261126.md D1）：全局变量表
    /// arena——`globals` 平表 + `phantom_globals` + `array_globals`
    /// （连带全局域 `elem_counts`）三合一的权威存储（步骤 1-4 的镜像
    /// 与 debug 对拍于步骤 5 退役——单一权威）。读写经 vars.rs 的
    /// global_* 咽喉；每命名空间一张的拆分随步骤 2/3。
    pub(crate) var_arena: vararena::VarArena,
    /// 顶层 slot 单元执行中（Some = 激活；exec_bytecode 进出管理）。
    pub(crate) top_slots: Option<TopSlots>,
    /// True while the :: table's ARRAY entries hold ONLY the startup
    /// arrays (`env`, `tcl_platform`): the per-op array guards in get_var/
    /// set_var/read_var then skip their `contains` probe entirely —
    /// the startup arrays make the guard permanently hot, and the
    /// probe was ~10% of var-heavy loops (dloop/oo profiles).  Every
    /// mutation site maintains the flag (inserts of any other name
    /// clear it; removals recompute — both are cold paths).
    pub(crate) arrays_env_only: bool,
    /// `array default` values (TIP 508, tclsh 9.1): keyed by the same
    /// stamp key as the array-search state (`G:name` / `F<idx>:name`).
    /// Consulted on element-miss reads (the default is NOT materialized:
    /// `info exists` stays 0, `array size` uncounted) and cleared at
    /// array removal (`loc_remove_array`) and frame death.
    pub(crate) array_defaults: VarMap<Value>,
    /// `const`-marked variables at the global level (tclsh 9.1's
    /// VAR_CONSTANT).  Frame-scoped marks ride [`CallFrame::consts`].
    pub(crate) const_globals: Vec<String>,
    /// Procedure call frames (empty at global level).  Boxed: a `CallFrame`
    /// is ~264 bytes and pool-pop/frame-push/frame-pop/pool-push moved the
    /// whole struct four times per call (a memcpy each — the fib profile's
    /// memmove bucket); boxing makes those moves 8-byte pointer copies while
    /// the box itself rides the pool.
    // Deliberate Box: see above — frame moves must stay pointer copies
    // (the pool rides them per call); clippy's size heuristic disagrees.
    #[allow(clippy::vec_box)]
    pub(crate) frames: Vec<Box<CallFrame>>,
    /// Recycled call frames: proc return pushes its (emptied) frame here
    /// and the next call pops it, keeping the locals-map allocation and
    /// its capacity across calls (tclsh keeps one frame arena per
    /// interpreter; recursion depth bounds the pool naturally).
    // Deliberate Box: same as `frames` — the recycled frame must not be
    // memcpy'd on pool pop/push.
    #[allow(clippy::vec_box)]
    pub(crate) frame_pool: Vec<Box<CallFrame>>,
    /// Recycled bytecode-executor states (vm_exec): every exec_bytecode
    /// call borrows one and returns it emptied, so proc calls pay no
    /// fresh stack/loops/bodies/scratch allocations.
    pub(crate) vm_pool: vm_exec::VmPool,
    /// Commands (built-in and registered).  Fx-hashed like the var tables:
    // dispatch probes this per command, and SipHash was ~20ns of every call.
    pub(crate) commands: VarMap<CommandFunc>,
    /// Command category metadata.
    pub(crate) command_categories: HashMap<String, CommandCategory>,
    /// Command metadata (usage + help) for built-in and registered commands.
    pub(crate) command_meta: HashMap<String, CommandMeta>,
    /// User-defined procedures.  `Rc` so per-call dispatch clones a
    /// reference (params + body + statics are NOT copied per call);
    /// statics write-back goes through `Rc::make_mut`.
    pub(crate) procs: VarMap<Rc<ProcDef>>,
    /// Tcl hash-table iteration order for the command tables: every
    /// command creation appends its key here (insert-when-new; a
    /// redefine of a live command keeps its position, a recreate after
    /// delete appends anew — Tcl hash-entry semantics).  `info commands`
    /// / `info procs` replay the per-namespace subsequence through
    /// [`tclhash::tcl_order`]; stale entries (deleted commands) are
    /// skipped by liveness, duplicates resolve to the LAST occurrence.
    /// Seeded at init with tclsh 8.6.17's registration sequence.
    pub(crate) command_order: Vec<String>,
    /// 运行时预置层加载中（内嵌 stdlib.tcl / ui.tcl 的 eval 期间置位）。
    /// M4「定义即编译」的排除面：预置层是 rtcl 的 rumble 层对应物
    /// （Racket CS 的 runtime 层构建期编译、不随进程重编）——每进程
    /// `Interp::new` 都重放同一批 proc 定义，预发射只产生纯启动税
    /// （实测 ~2-3ms/进程，29 个 stdlib proc × emit），且 stdlib proc
    /// 极少是用户负载的热点；保持它们的调用时编译（既有路径）。用户
    /// 脚本（含 source 的用户库）不受影响——定义即编译照常。
    pub(crate) in_rt_prelude: bool,
    /// Namespace creation order (`namespace children` replay).
    pub(crate) ns_order: Vec<String>,
    /// rtcl-superset commands hidden from `info commands`: tclsh 8.6.17
    /// exposes exactly 101 globals; rtcl's extra commands stay invokable
    /// but invisible, matching the oracle's listing surface.
    pub(crate) hidden_commands: VarSet,
    /// Live coroutines (`coroutine name cmd` → record).  A suspended
    /// record owns the coroutine's frames and the frozen bytecode-level
    /// chain (innermost last); a running record is empty (state lives in
    /// [`Interp::coro_ctx`]).
    pub(crate) coroutines: VarMap<crate::interp::commands::coro::Coroutine>,
    /// Context of the coroutine currently RUNNING on this interpreter:
    /// its record key, the frames-base at entry (frames above belong to
    /// the coroutine) and the pending suspend value.
    pub(crate) coro_ctx: Option<crate::interp::commands::coro::CoroCtx>,
    /// Suspend chain: bytecode levels that unwound on
    /// [`Error::Suspend`] freeze their executor state here, innermost
    /// first; the collector moves it into the coroutine record.
    pub(crate) coro_chain:
        Vec<(Rc<rtcl_parser::ByteCode>, vm_exec::SuspendedState)>,
    /// Host-declared extension modules (`dynmod`): pre-instantiated by
    /// the host (wasm side modules via rtcl-wasm's `register_module`),
    /// attached to the command table by `load` / detached by `unload`.
    pub(crate) dyn_modules:
        varmap::FxHashedMap<String, commands::dynmod::DynModule>,
    /// 文件系统数据面服务（HostFs）：file/open/source/glob/cd/pwd 的
    /// std::fs 直连点经此路由（§10quinter——通道协议面留原生，数据面
    /// 走宿主）。默认 NativeFs（std::fs，行为与路由前逐字一致）；
    /// wasm 宿主经 set_host_fs 注入 JS 实现。
    #[cfg(any(feature = "file", feature = "io"))]
    pub(crate) host_fs: Box<dyn crate::host::HostFs>,
    /// Command-name resolution cache (tclsh's resolved-command tokens):
    /// outer key = the namespace the resolution ran in, inner key = the
    /// invocation name as written.  Both levels probe with `&str`, so a
    /// hit costs two Fx hashes and zero allocations.  Entries carry the
    /// command-table generation they resolved under; every mutation of
    /// `commands`/`procs`/`ensembles`/`import_aliases`/`aliases` bumps
    /// [`Interp::cmd_generation`], ageing every entry out at once.
    /// Bounded (`cmd_cache_len` cap, clear on overflow) — wasm32 is a
    /// target.
    pub(crate) cmd_cache: VarMap<VarMap<CachedCmd>>,
    /// Root-namespace (`::`) half of the resolution cache: the common
    /// global-scope case resolves with ONE probe instead of the two-level
    /// map's two (the outer probe re-hashed `"::"` for every dispatched
    /// command — ~3% of the fib profile).  Shares `cmd_cache_len` and the
    /// generation ageing with the two-level half.
    pub(crate) cmd_cache_root: VarMap<CachedCmd>,
    /// Number of inner entries across `cmd_cache` (cheap overflow check;
    /// the cap keeps the clear-on-overflow amortised).
    pub(crate) cmd_cache_len: usize,
    /// `variable`-link resolution memo: current namespace → declared name →
    /// (canonical variable key, parent-namespace part).  The resolution is
    /// a pure string function of (ns, name) — a hot loop's `variable acc;`
    /// re-ran `qualify`/`normalise`/`parent_of` (three allocations) on
    /// every call, ~25% of an OO method-call benchmark whose bodies carry
    /// the link prefix.  The LIVE checks around it (parent exists, the
    /// owning namespace's variable-set insert, the per-frame upvar link)
    /// stay per-call.  No invalidation is needed; bounded like the other
    /// caches (wasm32 is a target).
    pub(crate) var_link_cache: VarMap<VarMap<(Rc<str>, Rc<str>)>>,
    /// Command-table generation: bumped at every command-table mutation.
    pub(crate) cmd_generation: u64,
    /// Parse-tree cache: script text → AST (with the source text the
    /// commands' spans point into; the key `Rc<str>` shares the unit's
    /// source allocation).  Parsing is a pure function of the text
    /// (substitution happens after parse), so a cached tree is
    /// interchangeable with a fresh parse; bodies re-evaluated per
    /// call/iteration (procs, loops) skip re-tokenization.  Entries are
    /// bounded (see `eval`) — wasm32 is a target.
    pub(crate) parse_cache: HashMap<Rc<str>, Rc<rtcl_parser::ScriptUnit>>,
    /// Compiled-form cache: script text → bytecode (keyed by the same
    /// shared source allocation as [`Interp::parse_cache`]).  Compiled
    /// once per cached text; the executor reproduces the tree-walk's
    /// frames exactly (see `vm_exec`), so a cached unit running op-by-op
    /// is interchangeable with the AST walk.  Bound and cleared together
    /// with the parse cache (see `eval`).
    pub(crate) bytecode_cache: varmap::FxHashedMap<Rc<str>, Rc<rtcl_parser::ByteCode>>,
    /// Approximate byte budgets for the compile caches (embedding
    /// high-water guard): source/AST, bytecode, materialised literal
    /// values, lambda bodies and expression-verdict entries.
    pub(crate) parse_cache_bytes: usize,
    pub(crate) bytecode_cache_bytes: usize,
    pub(crate) const_pool_bytes: usize,
    pub(crate) lambda_code_cache_bytes: usize,
    pub(crate) expr_check_cache_bytes: usize,
    /// Loop-body memo: recent body `Value`s handed to
    /// [`Interp::eval_body_value`] plus their compiled forms — a pointer
    /// compare replaces `eval`'s two full-text cache hashes on every hit
    /// (while/for/foreach bodies, `time` scripts).  A few slots, not one:
    /// `for` alternates body and next scripts every iteration, and one
    /// loop nested inside another needs both its own body and the outer
    /// body/next resident (tclsh instead compiles loop bodies inline into
    /// the surrounding bytecode and pays nothing per iteration).
    pub(crate) body_memo: Vec<(Value, Rc<rtcl_parser::ByteCode>)>,
    /// F3：epoch 失配体的重编译 memo（键 = 失配单元的 `Rc` 身份——
    /// stale 副本被条目钉住所以键不悬垂；体重定义后旧键随 FIFO 有界
    /// 驻留，上限 [`crate::interp::vm_exec`] 侧 `STALE_RECOMPILE_MAX`）。
    /// 值 = 当前世代的重编译产物（epoch 戳对齐消费门，跨代即废）。
    pub(crate) stale_recompiles:
        Vec<(Rc<rtcl_parser::ByteCode>, Rc<rtcl_parser::ByteCode>)>,
    /// Compiled lambda bodies keyed by the apply term string — the term
    /// IS the lambda's identity, so params/body/bytecode are pure
    /// functions of it (tclsh caches the compiled form on the lambda
    /// obj).  Bounded like the parse cache (wasm32 is a target).
    pub(crate) lambda_code_cache: HashMap<String, Rc<rtcl_parser::ByteCode>>,
    /// Literal value pool: compiled unit (by `Rc` allocation address) →
    /// its constants as ready `Value`s.  `PushConst` then pushes an
    /// Rc bump instead of re-materialising the literal string on every
    /// execution (tclsh keeps literal objects in a table exactly like
    /// this).  The entry keeps the `Rc<ByteCode>` alive, so the address
    /// is unique and stable for the entry's lifetime; bounded with
    /// clear-on-overflow (wasm32 is a target).  A miss simply falls back
    /// to `Value::from_str` — correctness never depends on residency.
    #[cfg(not(feature = "embedded"))]
    pub(crate) const_pool:
        HashMap<usize, ConstPoolEntry, core::hash::BuildHasherDefault<PtrHasher>>,
    #[cfg(feature = "embedded")]
    pub(crate) const_pool: HashMap<usize, ConstPoolEntry>,
    /// `check_expr` verdict memo: expr text → `Err(msg)` on syntax error,
    /// `Ok(())` when clean (see `types::expr::eval_expr`).  Pure function
    /// of the text; loop conditions re-check every iteration.
    pub(crate) expr_check_cache: varmap::FxHashedMap<String, Result<(), String>>,
    /// Call stack depth (for recursion limit).
    pub(crate) call_depth: usize,
    /// Maximum call depth.
    pub(crate) max_call_depth: usize,
    /// Script-driven re-entry that nests native frames *without* a proc
    /// call: an `unknown`-handler re-dispatch (namespace or ensemble), an
    /// alias chain, a trace callback's evaluation. `call_depth` counts
    /// proc calls and traced builtin dispatches only, so none of these
    /// were bounded — a handler naming itself, `alias a a`, or a ring of
    /// write traces each setting the next variable ran the native stack
    /// out (abort; wasm: a trap that kills the instance). One shared
    /// budget, sized like the recursion limit.
    pub(crate) nest_depth: usize,
    /// Eval step budget (A1): commands dispatched since last arm; every
    /// command dispatch charges one step (interpreter `eval_command` and
    /// the VM funnels `invoke_command`/`call` alike — pure control flow
    /// like `while {1} {}` is caught too).
    /// `Rc`: a child interpreter shares its parent's counter.  The budget
    /// then bounds the work of the whole family — with a counter of its
    /// own a child ran forever inside a budget the host had armed (one
    /// created before arming inherited "unlimited", and nested
    /// `interp eval` multiplied the budget by the family's size).
    pub(crate) steps: Rc<Cell<u64>>,
    /// `None` = unlimited (default — zero behavior change); `Some(n)` =
    /// trip past n steps with `ErrorCode::Timeout`. Arming re-zeroes the
    /// counter (nesting-proof: the host arms before each top-level eval).
    pub(crate) max_steps: Option<u64>,
    /// Last result.
    pub(crate) result: Value,
    /// Package registry: name → version string.
    #[cfg(feature = "package")]
    pub(crate) packages: HashMap<String, String>,
    /// Current namespace ("::") at the global level.  `Rc<str>` — a proc
    /// call swaps this to the definition namespace and back, and the swap
    /// is a refcount bump rather than a heap copy; same for the frame's
    /// `ns`/`call_ns` records.  Names built fresh (qualify, ns eval) move
    /// their String in via `Rc::from`, which reuses the allocation.
    pub(crate) current_namespace: Rc<str>,
    /// tclsh's `lookupNsPtr`: the namespace the *next* command dispatch
    /// resolves its command name in, instead of the current one.  Set by a
    /// `tailcall` to anything but a proc (TclNRTailcallEval): the command
    /// is looked up in the namespace of the frame the tailcall ran in and
    /// runs in the caller's.  One dispatch consumes it.
    pub(crate) cmd_lookup_ns: Option<Rc<str>>,
    /// The namespace the command now running was looked up in, while that
    /// is not the current one — i.e. during a command dispatched under
    /// [`Self::cmd_lookup_ns`], after the swap ended.  A command that
    /// resolves its own word again (`my`, the object commands) resolves it
    /// here.
    pub(crate) cmd_lookup_in: Option<Rc<str>>,
    /// Which host instance this interpreter is, in the tables its host
    /// keeps *outside* it (the wasm JS callback buckets).  Only the host
    /// sets it; no script can — a dispatch bridge looks the bucket up by
    /// this, never by an argument the script passed.
    pub(crate) host_instance_id: u64,
    /// The root namespace as a shared `Rc` — the namespace of every
    /// global-scope proc, handed out per call without allocating.
    pub(crate) ns_root: Rc<str>,
    /// Known namespaces ("::" always present).
    pub(crate) namespaces: varmap::FxHashedMap<String, commands::namespace::NamespaceInfo>,
    /// Namespaces in the visibility-removal phase of a two-phase teardown
    /// (`namespace delete`, tclsh NS_DYING): `namespace exists`/require_ns
    /// report them gone, but the entity stays in [`Self::namespaces`] until
    /// the var/command delete traces have all fired — resolution walks the
    /// parent's child table without a flag check (TclGetNamespaceForQualName),
    /// so vars/commands of a dying tree stay reachable from trace callbacks.
    pub(crate) ns_dying: std::collections::HashSet<String>,
    /// 自删语境（`namespace delete [namespace current]` 后、eval 体尚
    /// 未退出）的**可复活命令名**（ns 全名 → 体帧窗口开始时在该 ns 的
    /// 命令全名集）：窗口内同名重建任一名单上的命令即复活整个 ns
    /// （tclsh 基本语义：basic-15.3——同名 `proc p` 重建后 ns 以新空
    /// 表归位，exists=1；不同名新建命令只在体帧内可解析、随体消亡）。
    pub(crate) ns_resurrect_cmds: HashMap<String, std::collections::HashSet<String>>,
    /// Var flat keys inside the pending-unset window of a teardown (tclsh
    /// UnsetVarStruct's "in-table but undefined" step): reads and `info vars`
    /// miss them, `namespace which -variable` still resolves them (the Var
    /// struct stays in the table until the trace returns).
    pub(crate) var_nuke_pending: std::collections::HashSet<String>,
    /// `namespace import` aliases: fully-qualified alias name → the fully
    /// qualified name of the original command.  Aliases stay separate from
    /// `procs` so the origin's *current* body always dispatches (tclsh
    /// semantics: redefining the source proc is visible through the alias).
    pub(crate) import_aliases: HashMap<String, String>,
    /// Per-namespace `namespace unknown` handler scripts (raw, as given).
    pub(crate) ns_unknown: HashMap<String, String>,
    /// Live `namespace ensemble`s keyed by registered command name.
    pub(crate) ensembles: HashMap<String, EnsembleDef>,
    /// Live array searches keyed by the owning array's host handle
    /// (D1 步骤 3：搜索键 = 宿主 VarRef——TableId::Array(宿主句柄)，
    /// 代数守卫使旧数组亡后的残留键不可达；帧宿主随帧出清).
    pub(crate) array_searches: HashMap<vararena::VarRef, ArraySearchList>,
    /// Mutation counters per array (element-set changes invalidate
    /// searches); keyed by the host handle, same as the searches.
    pub(crate) array_stamps: HashMap<vararena::VarRef, u64>,
    /// Container-generation counters per array, bumped on array-level
    /// create/destroy only (`array get` uses them to spot a callback that
    /// destroyed and recreated the array — snapshot elements then skip).
    /// Monotonic stamp bumped on every array container create/delete.
    /// A single interpreter-wide counter replaces a per-array map (which
    /// grew with array-name churn and had no removal path): `array get`
    /// snapshots are conservatively invalidated by any array mutation.
    pub(crate) array_generation: u64,
    /// 持有 trace 集（`Var.traces: Option<Box<TraceSet>>` 在席）的宿主
    /// Var 数——「任意变量 trace 在册」的 O(1) 真源（D1 步骤 4）。快径
    /// 守卫（ts 单元、scalar_top 读写、incr/lappend/append 元素快径、
    /// read_var 免火臂、槽 fast path……）据此让路：>0 走全语义链。
    /// 计数形与旧字符串表的「键 linger」守卫语义逐字同：注册建集 +1；
    /// `trace remove` 只摘记录（空集**留存**——lingering 键形，基线钉
    /// 形：摘净后 upvar-8.7 仍拦）；宿主整体消亡（快照 take/unset 摘
    /// 除/帧清场/ns 清扫）时 −1。方向偏差只可能偏保守（守卫多让路 =
    /// 走全链 = 语义等价，仅慢）。
    pub(crate) n_trace_hosts: usize,
    /// Currently-firing trace targets (tclsh's VAR_TRACE_ACTIVE, D-36):
    /// `W\0{stamp}` for whole-variable fires, `E\0{stamp}\0{elem}` for
    /// element fires.  A fire on an active target is skipped, so a
    /// callback reading/writing its own traced variable cannot recurse.
    pub(crate) trace_active: std::collections::HashSet<String>,
    /// `trace add command` registrations (stored; rename/delete traces).
    pub(crate) cmd_traces: HashMap<String, Vec<(Vec<String>, String)>>,
    /// `trace add execution` registrations (stored; enter/leave/step traces).
    pub(crate) exec_traces: HashMap<String, Vec<ExecTrace>>,
    /// Identity counter for [`ExecTrace`] records.
    pub(crate) exec_trace_ids: u64,
    /// Resolved command keys of the procs currently executing (innermost
    /// last) — execution `enterstep` traces resolve the *caller* command
    /// through this stack.
    pub(crate) exec_step_stack: Vec<String>,
    /// `> 0` while an execution-trace callback itself is running (its inner
    /// commands do not re-trigger step traces — tclsh suppresses them).
    pub(crate) exec_step_running: u32,
    /// Shadow epoch for compiled proc bodies: bumped whenever a command
    /// whose *leaf* name matches one of the inline-folded Tier1 commands
    /// (`set`, `if`, `while`, `for`, `expr`, `incr`, `return`, `exit`,
    /// `break`, `continue`) gains, loses, or changes its binding.  Those
    /// folds bypass dispatch, so code compiled under an older epoch must
    /// fall back to the dynamically-resolving tree-walk (namespace-41.1:
    /// `test` compiled `set ::g 0` inline, then `proc set` shadowed it —
    /// tclsh re-resolves and so must we).
    pub(crate) tier1_epoch: u64,
    /// True while executing a compiled PROC-CONTEXT unit (`locals_mode`:
    /// proc bodies, compiled lambda bodies) — the runtime mirror of
    /// tclsh's compiledLocals context.  DynCall'd commands consult it to
    /// reproduce semantics the compiler bakes in at inline sites
    /// (cmd_catch: a braced body runs lexically — its constructs inline,
    /// loop-variable writes stay plain — instead of as a fresh
    /// dispatched unit).  Cleared at every fresh-unit boundary (`eval`
    /// of a new text), like `lexical_body`.
    pub(crate) in_locals_unit: bool,
    /// Eval-level (no call frame) variable aliases created by `upvar` /
    /// `variable`: canonical local flat key → canonical target flat key.
    /// At the global level tclsh's `upvar`/`variable` link two variables;
    /// rtcl's flat global table models that with this redirect.  A Vec so
    /// `namespace eval` can truncate to its entry mark on exit.
    /// D1 步骤 5 决策：**保留**（解改名/前置重写层，非 per-Var 态——
    /// 别名名可无表条目；收编需物化别名 Var 并动列举序红线）。
    pub(crate) flat_aliases: Vec<(String, String)>,
    /// Eval-level aliases whose target was deleted (namespace removal):
    /// reads keep the redirect, writes error.
    /// D1 步骤 5 决策：**保留**（flat_aliases 的亡目标伴随面——同前
    /// 理由：前置重写层，非 per-Var 态）。
    pub(crate) dead_flat: Vec<String>,
    /// Flat keys of variables a `variable` command declared at eval level
    /// (tclsh: the ns-eval varFrame holds a reference, so a namespace that
    /// deletes ITSELF leaves the declared variable readable for the rest
    /// of the body — var-1.16/1.17's `set result` after the self-delete).
    pub(crate) ns_variable_links: Vec<String>,
    /// `variable` 命令在 eval 级装的**瞬态**链接（裸名 → ns 变量键）：
    /// 体帧作用域（var-3.10 与 DIFF-8 的分野——upvar 的 eval 级别名链
    /// 接两个持久 ns 变量、随 flat_aliases 持久；variable 链接随声明
    /// 体帧生灭、不漏入嵌套体）。ns eval 体入口 take 清空、退出恢复。
    /// D1 步骤 5 决策：**保留**（体作用域瞬态别名——take/恢复的
    /// 生命周期挂在 ns-eval 体进出，Var.link 无此作用域语义）。
    pub(crate) ns_transient: Vec<(String, String)>,
    /// Namespace variables kept alive past `namespace delete` because a
    /// live variable link still references them; dropped when the
    /// declaring `namespace eval` exits.
    pub(crate) ns_eval_keep: Vec<String>,
    /// Accumulated `errorInfo` of the error currently propagating
    /// (message + `while executing` / `invoked from within` frames).
    /// `None` when no error is in flight; consumed by `catch`.
    pub(crate) err_info: Option<String>,
    /// Suppress the next script-harness frame append — the erroring
    /// command already wrote errorInfo itself (`error msg info`) or
    /// propagates framelessly (`while`/`if` bodies, tclsh's inlined loop
    /// instructions).
    pub(crate) err_fresh: bool,
    /// Set when the raise site of the in-flight error installed
    /// ::errorCode itself (scan formats, exec CHILDSTATUS, `error`,
    /// `return -errorcode`); a `catch` must then preserve it instead of
    /// deriving the code from the error variant.
    pub(crate) err_code_raised: bool,
    /// Line of the last erroring command recorded by a script harness;
    /// consumed by `(procedure ... line N)`, `(in namespace eval ...
    /// script line N)`, `("uplevel" body line N)`, `(file ... line N)`.
    pub(crate) err_line: usize,
    /// C1（2026-11-16）：本次错误传播中**最后一条失败命令**在其单元内
    /// 的行（dispatch_values 出错时捕获，传播由内向外逐层覆写 → catch
    /// 读到的是 catch 体单元内最外层失败命令的行——tclsh 的 errorLine
    /// 在每个日志事件处即时更新，catch 读到的是外围命令日志事件的行）。
    pub(crate) err_boundary_line: Option<usize>,
    /// Newlines between the enclosing script's first line and the script
    /// currently being evaluated by a construct that re-evals a word of
    /// its own source (`while`/`for`/`foreach` bodies): a body command's
    /// parse-relative line + this offset is the line within the enclosing
    /// script, which is what `(procedure ... line N)` must report (tclsh
    /// keeps one absolute line table).  0 at script level.
    pub(crate) line_offset: usize,
    /// Line of the command currently dispatching (parse-relative).
    pub(crate) cur_cmd_line: usize,
    /// Line of the command currently dispatching **absolute in the
    /// current unit**（树走 = cmd.line + line_offset；VM = entry_offset +
    /// site.line）——运行时 expr 求值器的括号行重定基以它为基
    /// （tclsh 编译单元的行表 = 单元绝对行；site.line_rel 是括号内容
    /// 相对行，单 Cmd token 表达式留在运行时求值器时以此为准）。
    pub(crate) cur_cmd_line_abs: usize,
    /// Raw source of each word of the command currently dispatching,
    /// aligned with its arguments — braced/quoted script words carry their
    /// delimiters, letting constructs recover their body's line offset.
    /// Spans resolve through [`Interp::cur_source`]; borrowed from the
    /// owning unit/AST (see [`WordSrcs`]) so per-command save/restore is
    /// a pointer copy, not a refcount bump.
    pub(crate) cur_cmd_word_srcs: util::WordSrcs,
    /// Source text of the command currently dispatching, for constructs
    /// that need the raw invocation (`info level 0` inside
    /// `namespace eval`).  Span into [`Interp::cur_source`].
    pub(crate) cur_cmd_text: rtcl_parser::SrcSpan,
    /// Source text of the script unit the currently dispatching command
    /// was parsed from — [`Interp::cur_cmd_text`] and
    /// [`Interp::cur_cmd_word_srcs`] are spans into it.  Shared with the
    /// parse cache: swapping it per dispatch is a refcount bump.
    pub(crate) cur_source: Rc<str>,
    /// `info level 0` inside `namespace eval`: the ns-eval command's
    /// source text, one entry per live `namespace eval` (tclsh's
    /// namespace-eval varFrame is visible to `info level 0`).
    pub(crate) ns_level0: Vec<String>,
    /// The qualified namespace of each live `namespace eval`, parallel to
    /// [`Interp::ns_level0`] — uplevel's scope chain needs the target's
    /// namespace, not just its level-0 text.
    pub(crate) ns_stack: Vec<String>,
    /// Set when a word's error came from a command substitution (the
    /// nested eval already logged its frames; the enclosing command logs
    /// none and marks [`Interp::err_pending_top`] instead).
    pub(crate) err_from_subst: bool,
    /// The script unit currently tree-walking is part of a proc body —
    /// the context tclsh compiles foreach/lmap in (compiledLocals), so
    /// dispatched `foreach` takes the compiled semantics (frameless body
    /// errors, absolute lines, plain var-write errors).  Arming a
    /// construct's body for "same unit" (while/for/if-arm bodies,
    /// `[...]` words, expr-operand brackets — the seams the compiler
    /// inlines into the surrounding unit) sets
    /// [`Interp::next_eval_lexical`]; `eval` consumes it at entry and
    /// restores this field on exit.  `exec_bytecode` runs its unit with
    /// this cleared: a DynCall'd command inside compiled unit is tclsh's
    /// INST_CALL fallback — dispatched semantics.
    pub(crate) lexical_body: bool,
    /// eval 递归深度（0 = 最外层——文件/REPL 顶层；≥1 = 嵌套 eval）。
    /// 顶层 slot 编译只在深度 0 生效（嵌套体 eval 的变量链接语义
    /// ——dict update/with 的别名——依赖非 slot 编译）。
    pub(crate) eval_depth: usize,
    /// Pending "the next `eval` is part of the current proc body unit"
    /// request (see [`Interp::lexical_body`]).
    pub(crate) next_eval_lexical: bool,
    /// Source of the outermost command whose word substitution errored,
    /// waiting to see if the error escapes uncaught — the top-level
    /// report then adds `invoked from within "<text>"` (tclsh logs the
    /// enclosing command only when nothing else consumed the log).
    /// Payload = (命令文本, 单元内绝对行, 单元相对行)。第三项 Some 时
    /// 构造出口帧（ns eval 体）可还原体相对行——defer 形态下 err_line
    /// 停在最内层日志命令（可能在嵌套 proc 单元）的行，属主命令的行
    /// 显式携带（we5 钉形；JIT 路径的挂起帧无相对行，置 None 走
    /// err_line 旧口径）。
    pub(crate) err_pending_top: Option<(Rc<str>, usize, Option<usize>)>,
    /// [`Interp::err_pending_top`] 被更外层 defer **顶替**时，原单元边界
    /// 帧（内层替换单元的命令帧）的**链式载体**（e8/e9）：嵌套替换词
    /// `set x [[ [[a]b]c]d]` 的每层单元出口帧按顶替序入链，顶层报告
    /// （`error_report`）先链后顶逐帧落账（tclsh 文件级退出的逐单元
    /// 日志——五帧序）；其余边界与 `pending_top` 同生死：catch 消费
    /// 清空、harness 直落帧清空（ERR_ALREADY_LOGGED 单次消费的同款
    /// 纪律）、eval_isolated/trace 回调 save-restore 同步、flow 转换
    /// 丢弃（tclsh 的 `set x [break]` 不落替换帧）。
    pub(crate) err_pending_chain: Vec<(Rc<str>, usize)>,
    /// [`Interp::err_pending_top`] 的来源标记：true = 替换**单元边界**
    /// 写入（bytecode 的 Sub 区弹出 / 词级 defer、tree-walk 的词级
    /// defer）——该 pending 是内层单元的出口帧，可在下一个单元边界
    /// 提交为 `invoked from within`（e8/e9 三帧 errorInfo 的中间帧）。
    /// false = expr 抑制臂写入（字节码 Expr 区——tclsh 对替换源错误
    /// 不落 `expr {...}` 帧，提交必须跳过）或 pending 已消费/为空。
    pub(crate) err_pending_unit_frame: bool,
    /// 本次错误源自**运行时 expr 求值器的括号操作数**（sticky——穿过
    /// 嵌套单元机器直达词级 defer 消费点）：其 EvalScript defer 的
    /// pending 是被 tclsh 抑制的 `expr {...}` 帧，不可提交（e8/e9 与
    /// expr 帧抑制的正交性；expr.rs 两处括号臂置位、wordop-defer/
    /// 新错误复位消费）。
    pub(crate) err_expr_origin: bool,
    /// D-3 -errorstack：当前错误的 INNER 条目体（"returnImm x {}" /
    /// "invokeStk1 <cmdtext>"）；新错误在 harness 首帧处重建。
    pub(crate) err_inner: Option<String>,
    /// cmd_error 预装的 returnImm 形（一次性，harness 首帧消费——否则
    /// 会被自身 dispatch 的 harness 帧覆写成 invokeStk 形）。
    pub(crate) err_pending_inner: Option<String>,
    /// 错误传播途中离开的 proc 帧（CALL 条目，内层先）——call_proc 错
    /// 误出口 push，fresh 错误清空。
    pub(crate) err_calls: Vec<String>,
    /// 最近一次 call_proc 的错误是否来自**体执行**（true）而非入口
    /// 绑定/分派失败（false）。`next` 用它区分"目标方法体出错"
    /// （正常 INNER）与"next 自身分派失败"（INNER=tclooNext）。
    pub(crate) err_call_body: bool,
    /// 活动 `uplevel` 的跨层数（栈；每次错误日志事件追加
    /// `UP <delta>`，tclsh Tcl_LogCommandInfo 的 varFramePtr 判定）。
    pub(crate) err_uplevel: Vec<(usize, usize)>,
    /// 每个活动 `namespace eval` 的调用词重建文本（CALL 条目载荷；
    /// 与 [`Interp::ns_stack`] 平行），不是 ns_level0 的源文本。
    pub(crate) ns_call_text: Vec<String>,
    /// 下一次 call_proc 是否由 `next` 调起：tclsh 的下一实现帧 usage
    /// 词是 `next`（`wrong # args: should be "next x"`），且合成的方法
    /// 名参数不进 usage。
    pub(crate) frame_next_call: bool,
    /// 跳过**下一次**错误日志事件（不追加 errorInfo 帧、不记 INNER/CALL）
    /// ——`return -level 0 -code error -errorinfo <非空>` 的物化点
    /// （tclsh：return 命令自身不产生日志事件）。
    pub(crate) err_skip_log: bool,
    /// 下一次日志事件（errorInfo 已有帧）仍要**设置 INNER 并清栈**——
    /// 上述 -errorinfo 物化后的首个调用方站点（tclsh resetErrorStack
    /// 在物化后的首个 Tcl_LogCommandInfo 生效）。
    pub(crate) err_force_inner: bool,
    /// 当前脚本单元是否处于**文件执行语境**（tclsh 的 TclEvalEx 逐命令
    /// 路径：`rtcl -f` 的文件、`source` 的文件）。tclsh 的文件脚本经
    /// TclNREvalFile → TclEvalEx 逐命令执行：每条顶层命令经
    /// Tcl_EvalObjv 分派，出错时由该命令所在脚本层的错误路径记帧
    /// （tclBasic.c:5613）——文件级 if/while/for 字面体的体错误因此多记
    /// `("while"/"for" body line N)` + 命令 invoker 帧；catch/proc 体等
    /// 编译语境则全部内联、只记最内层命令。arm 由文件入口设置，
    /// eval_impl 逐单元消费（嵌套 eval/catch 体 = 编译语境 = false）。
    pub(crate) in_file_script: bool,
    /// [`Interp::in_file_script`] 的入口武装位（文件入口 set，首个
    /// eval_impl 消费）。
    pub(crate) file_script_armed: bool,
    /// 下一次 errorInfo 帧用 `while executing`（而非 invoked from
    /// within）：帧边界物化的 `return -code error`（无显式 -errorinfo）
    /// 由 msg 作基、errorInfo 内部为 NULL（tclsh 实测 B/I/J 形）。
    pub(crate) err_while_exec: bool,
    /// 下一次 call_proc 建 **objc=0 特殊帧**（tclsh 析构器帧）：
    /// `info level 0` 为空、错误日志事件不追加 CALL。
    pub(crate) frame_level0_empty: bool,
    /// Current script name (for info script).
    #[cfg(feature = "std")]
    pub(crate) script_name: String,
    /// Channel table (stdin/stdout/stderr + opened files/pipes).
    #[cfg(feature = "std")]
    pub channels: crate::channel::ChannelTable,
    /// Background `exec ... &` children: retained so their handles are not
    /// leaked and so `try_wait` can reap zombies (Tcl's detached-process
    /// reaper model, on the event-loop tick / before the next spawn).
    #[cfg(feature = "exec")]
    pub(crate) background_children: Vec<std::process::Child>,
    /// Exit status (raw) of background children already reaped before a
    /// `wait` asked for them (exec/update/Drop reap finished children):
    /// pid → raw status. Read by `wait`, cleared past a cap.
    pub(crate) child_status: std::collections::HashMap<u32, i32>,
    /// 宿主控制台服务：无 `io` 特性构建里 `puts` 的 stdout/stderr 落点
    /// （原生 stdio，或 wasm 宿主注入的 JS 实现）。
    /// `Rc`: a child interpreter shares its parent's (a child with the
    /// default one dropped every `puts`).
    pub(crate) console: Rc<dyn crate::host::HostConsole>,
    /// 宿主 HTTP 服务：`http::*` 词表层的 native 落点（缺省
    /// [`crate::host::NullHttp`]——请求即报"网络服务不可用"；rtcl-cli 的
    /// `net` feature 注入 reqwest 实现；wasm 走 JS 桥，不经此处）。
    pub(crate) http: Box<dyn crate::host::HostHttp>,
    /// 进程执行服务（wasm 外部命令）：`None` → 该宿主不提供 exec。
    pub(crate) exec: Option<Box<dyn crate::host::HostExec>>,
    /// 宿主文件 fd 源（[`crate::host::HostFdFs`]）：exec 重定向把路径
    /// 变成 `HostFd` 放进子进程 fd 表（`None` → 该宿主不提供文件 fd）。
    /// 与上方 `host_fs`（数据面通道服务）是平行的兄弟接缝，见 host.rs。
    pub(crate) fd_fs: Option<Box<dyn crate::host::HostFdFs>>,
    /// 宿主作业表（`exec &` / `job *`）；`None` → 不支持后台作业。
    pub(crate) jobs: Option<Box<dyn crate::host::HostJobs>>,
    /// 时钟服务：`after`/`vwait`/`time` 的 now/sleep 落点
    /// （原生 `Instant`，或 wasm 宿主注入的 JS 实现）。
    /// `Rc`: a child interpreter shares its parent's (a child with the
    /// native one trapped on every `after`/`vwait`/`time` on a host that
    /// has no native clock — wasm).
    #[cfg(feature = "std")]
    pub(crate) clock: Rc<dyn crate::host::HostClock>,
    /// Alias definitions (name → target + prefix args).
    pub(crate) aliases: HashMap<String, commands::introspect::AliasInfo>,
    /// Reference table for ref/getref/setref.
    pub(crate) references: HashMap<String, commands::introspect::RefInfo>,
    /// Next reference ID counter.
    pub(crate) next_ref_id: u64,
    /// Saved command definitions for upcall support.
    pub(crate) saved_commands: HashMap<String, commands::introspect::SavedCommand>,
    /// Set of tainted variable names.
    pub(crate) tainted_vars: HashMap<String, bool>,
    /// Execution trace callback script (xtrace). Empty = disabled.
    pub(crate) xtrace_callback: String,
    /// Event queue for `after`/`vwait`/`update`.
    #[cfg(feature = "std")]
    pub(crate) event_queue: Vec<TimedEvent>,
    /// Next event ID counter for `after` scheduling.
    #[cfg(feature = "std")]
    pub(crate) next_event_id: u64,
    /// 后台 `chan copy -command` 任务（tclsh TclCopyChannel 模型）：
    /// 调度即挂起、事件循环（update/vwait）泵送、回调收累计数。
    #[cfg(feature = "std")]
    pub(crate) copy_jobs: Vec<commands::chan_io::ChanCopyJob>,
    /// Child interpreter table for `interp` command, keyed by the
    /// interpreter's name.  `None` while the interpreter is
    /// being evaluated (`interp eval` takes it out so that it and its
    /// parent are two separate values; the evaluating frame owns it) —
    /// the name still exists meanwhile.
    #[cfg(feature = "std")]
    pub(crate) child_interps: HashMap<String, Option<Box<Interp>>>,
    /// Commands `interp hide` took away.  Hidden means gone from every
    /// table an invocation resolves through — a script in this
    /// interpreter gets `invalid command name` — until `interp expose`
    /// puts them back; only the master's `interp invokehidden` reaches
    /// them meanwhile.  ([`Interp::hidden_commands`] is another thing:
    /// a listing filter for host bridges, invokable as ever.)
    pub(crate) hidden_cmds: HashMap<String, commands::interp_cmd::HiddenCmd>,
    /// `interp create -safe`: the commands that could reach outside
    /// this interpreter are hidden.
    pub(crate) is_safe: bool,
    /// Aliases whose words run in another interpreter (`interp alias
    /// this name {} target …`): the command name and where the words run.
    pub(crate) cross_aliases: HashMap<String, commands::interp_cmd::CrossAlias>,
    /// The interpreter whose table this one is in — null for the one no
    /// other holds.  A child lives inside its parent, so the pointer is
    /// good for as long as this interpreter is.
    #[cfg(feature = "std")]
    pub(crate) owner: *mut Interp,
    /// An alias renamed in this interpreter: the name it was made under
    /// (which still names it, tclsh's alias token) → the key it is under.
    pub(crate) alias_tokens: HashMap<String, String>,
    /// The command of each child interpreter, by the name it goes by —
    /// it follows a `rename` — to the interpreter's name in the table.
    #[cfg(feature = "std")]
    pub(crate) interp_path_names: HashMap<String, String>,
    /// One-shot variable-link list for the next `call_proc` frame: the OO
    /// method dispatcher hands its owner's `variable` declarations over
    /// and `call_proc` installs the links at frame setup — the same
    /// triple the `variable` command performs (alias entry, per-name slot
    /// aliasing, seed-if-exists), minus a dispatched command per variable
    /// per call.  Prefix errors are unreachable: declaration names are
    /// validated by the `variable` define word and object namespaces
    /// always exist.
    pub(crate) frame_prelink: Option<Rc<Vec<PrelinkEntry>>>,
    /// One-shot level-0 word override for the next `call_proc` frame
    /// (`apply` renders `info level 0` as `apply {<term>} <args...>`,
    /// which the plain arg-list form can't express).
    pub(crate) frame_level0_args: Option<Vec<Value>>,
    /// TclOO object-system state (`oo::` commands, objects, classes).
    pub(crate) oo: commands::oo::OoState,
    /// Reflected channels created by `chan create`, keyed by handle
    /// (`rc0`, `rc1`, ...). Reads/writes route through the handler script.
    #[cfg(feature = "io")]
    pub(crate) reflected: HashMap<String, commands::chan_io::ReflectedChannel>,
    /// Transforms pushed with `chan push`, keyed by the channel handle they
    /// wrap.
    #[cfg(feature = "io")]
    pub(crate) transforms: HashMap<String, commands::chan_io::ChannelTransform>,
}

/// A scheduled time event (for `after ms script`).
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub(crate) struct TimedEvent {
    /// Unique event ID.
    pub id: u64,
    /// Absolute fire time (milliseconds since process start, from `Instant`).
    pub fire_at_ms: u64,
    /// Script to evaluate when the event fires.
    pub script: String,
    /// Whether this is an idle event (`after idle`).
    pub is_idle: bool,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    /// Create a new interpreter.
    pub fn new() -> Self {
        let mut interp = Interp {
            var_arena: vararena::VarArena::new(),
            arrays_env_only: true,
            array_defaults: VarMap::default(),
            const_globals: Vec::new(),
            array_searches: HashMap::new(),
            array_stamps: HashMap::new(),
            array_generation: 0,
            n_trace_hosts: 0,
            trace_active: std::collections::HashSet::new(),
            cmd_traces: HashMap::new(),
            exec_traces: HashMap::new(),
            exec_trace_ids: 0,
            exec_step_stack: Vec::new(),
            exec_step_running: 0,
            tier1_epoch: 0,
            flat_aliases: Vec::new(),
            dead_flat: Vec::new(),
            ns_variable_links: Vec::new(),
            ns_transient: Vec::new(),
            ns_eval_keep: Vec::new(),
            err_info: None,
            err_fresh: false,
            err_code_raised: false,
            err_line: 1,
            err_boundary_line: None,
            line_offset: 0,
            cur_cmd_line: 0,
            cur_cmd_word_srcs: util::WordSrcs::empty(),
            cur_cmd_text: rtcl_parser::SrcSpan { start: 0, end: 0 },
            cur_source: Rc::from(""),
            err_from_subst: false,
            err_pending_unit_frame: false,
            err_expr_origin: false,
            lexical_body: false,
            in_file_script: false,
            file_script_armed: false,
            cur_cmd_line_abs: 0,
            eval_depth: 0,
            top_slots: None,
            next_eval_lexical: false,
            err_pending_top: None,
            err_pending_chain: Vec::new(),
            err_inner: None,
            err_pending_inner: None,
            err_calls: Vec::new(),
            err_call_body: false,
            err_uplevel: Vec::new(),
            ns_call_text: Vec::new(),
            frame_next_call: false,
            frame_level0_empty: false,
            err_skip_log: false,
            err_force_inner: false,
            err_while_exec: false,
            ns_level0: Vec::new(),
            ns_stack: Vec::new(),
            frames: Vec::new(),
            frame_pool: Vec::new(),
            vm_pool: vm_exec::VmPool::new(),
            commands: VarMap::default(),
            command_categories: HashMap::new(),
            command_meta: HashMap::new(),
            procs: VarMap::default(),
            cmd_cache: VarMap::default(),
            cmd_cache_root: VarMap::default(),
            cmd_cache_len: 0,
            var_link_cache: VarMap::default(),
            cmd_generation: 0,
            parse_cache: HashMap::new(),
            bytecode_cache: varmap::FxHashedMap::default(),
            parse_cache_bytes: 0,
            bytecode_cache_bytes: 0,
            const_pool_bytes: 0,
            lambda_code_cache_bytes: 0,
            expr_check_cache_bytes: 0,
            body_memo: Vec::new(),
            stale_recompiles: Vec::new(),
            lambda_code_cache: HashMap::new(),
            const_pool: HashMap::default(),
            in_locals_unit: false,
            expr_check_cache: varmap::FxHashedMap::default(),
            call_depth: 0,
            max_call_depth: default_max_call_depth(),
            nest_depth: 0,
            steps: Rc::new(Cell::new(0)),
            max_steps: None,
            result: Value::empty(),
            #[cfg(feature = "package")]
            packages: HashMap::new(),
            current_namespace: Rc::from("::"),
            cmd_lookup_ns: None,
            cmd_lookup_in: None,
            hidden_cmds: HashMap::new(),
            is_safe: false,
            cross_aliases: HashMap::new(),
            owner: std::ptr::null_mut(),
            alias_tokens: HashMap::new(),
            interp_path_names: HashMap::new(),
            // No instance: a value no host hands out.
            host_instance_id: u64::MAX,
            ns_root: Rc::from("::"),
            namespaces: {
                let mut ns = varmap::FxHashedMap::default();
                ns.insert("::".to_string(), commands::namespace::NamespaceInfo::default());
                ns
            },
            ns_dying: std::collections::HashSet::new(),
            ns_resurrect_cmds: HashMap::new(),
            var_nuke_pending: std::collections::HashSet::new(),
            import_aliases: HashMap::new(),
            ns_unknown: HashMap::new(),
            command_order: Vec::new(),
            ns_order: vec!["::".to_string()],
            hidden_commands: VarSet::default(),
            coroutines: VarMap::default(),
            coro_ctx: None,
            coro_chain: Vec::new(),
            dyn_modules: varmap::FxHashedMap::default(),
            #[cfg(any(feature = "file", feature = "io"))]
            host_fs: Box::new(crate::host::NativeFs),
            ensembles: HashMap::new(),
            #[cfg(feature = "std")]
            script_name: String::new(),
            #[cfg(feature = "std")]
            channels: crate::channel::ChannelTable::new(),
            #[cfg(feature = "exec")]
            background_children: Vec::new(),
            child_status: std::collections::HashMap::new(),
            console: {
                #[cfg(feature = "std")]
                let c: Rc<dyn crate::host::HostConsole> = Rc::new(crate::host::NativeConsole);
                #[cfg(not(feature = "std"))]
                let c: Rc<dyn crate::host::HostConsole> = Rc::new(crate::host::NullConsole);
                c
            },
            http: Box::new(crate::host::NullHttp),
            exec: None,
            fd_fs: None,
            jobs: None,
            aliases: HashMap::new(),
            references: HashMap::new(),
            next_ref_id: 0,
            saved_commands: HashMap::new(),
            tainted_vars: HashMap::new(),
            xtrace_callback: String::new(),
            #[cfg(feature = "std")]
            event_queue: Vec::new(),
            copy_jobs: Vec::new(),
            #[cfg(feature = "std")]
            next_event_id: 0,  // tclsh 9.x: after/timer ids start at after#0 (8.6 was 1-based)
            #[cfg(feature = "std")]
            clock: Rc::new(crate::host::NativeClock),
            #[cfg(feature = "std")]
            child_interps: HashMap::new(),
            #[cfg(feature = "std")]
            frame_level0_args: None,
            frame_prelink: None,
            oo: commands::oo::OoState::default(),
            #[cfg(feature = "io")]
            reflected: HashMap::new(),
            #[cfg(feature = "io")]
            transforms: HashMap::new(),
            in_rt_prelude: false,
        };
        commands::oo::init(&mut interp);
        interp.ensure_ns_path("::zlib");
        interp.register_builtins();
        interp.init_special_vars();
        interp.load_stdlib();
        // Seed the Tcl iteration-order logs AFTER stdlib: the canonical
        // tclsh surface (101 visible commands, startup globals) must
        // include the stdlib-defined init.tcl ports (unknown, auto_load,
        // history, …) at their tclsh registration positions.
        interp.seed_command_order();
        interp
    }

    /// Load the Tcl-level standard library (embedded at compile time).
    /// M4：加载期置 [`Interp::in_rt_prelude`]——预置层 proc 定义不触发
    /// JIT 定义点预编译（rumble 层口径，见字段注）。
    fn load_stdlib(&mut self) {
        const STDLIB_TCL: &str = include_str!("../stdlib.tcl");
        self.in_rt_prelude = true;
        let r = self.eval(STDLIB_TCL);
        self.in_rt_prelude = false;
        if let Err(e) = r {
            panic!("stdlib.tcl failed to load: {e}");
        }
    }

    /// 加载 ui::* 词表层（契约见 docs/UI-DSL.md）。宿主须先注册 `_ui`
    /// 桥命令（wasm：JS op 汇聚点；原生测试：记录器）。词表命令在
    /// 调用时才解析 `_ui`，所以注册可以晚于加载。
    pub fn load_ui(&mut self) -> crate::error::Result<()> {
        const UI_TCL: &str = include_str!("../ui.tcl");
        // ui.tcl 同属预置层（见 in_rt_prelude 字段注）——同口径排除。
        self.in_rt_prelude = true;
        let r = self.eval(UI_TCL);
        self.in_rt_prelude = false;
        r.map(|_| ())
    }

    /// 加载 http::* 词表层（数据通道的 wasm 客户端，docs/ROADMAP.md
    /// §3）。宿主须注册 `__rtcl_http` 桥（wasm：www/http-bridge.js 的
    /// fetch 实现；原生：暂无，本方法仅供 wasm 宿主显式调用）。同样
    /// 调用时才解析桥命令，注册可晚于加载。
    pub fn load_http(&mut self) -> crate::error::Result<()> {
        const HTTP_TCL: &str = include_str!("../http.tcl");
        self.eval(HTTP_TCL).map(|_| ())
    }

    /// 加载 log::* 词表层（内存 ring 事件存储 + 查询管道，
    /// docs/ROADMAP.md §3 起步版）。纯 Tcl，原生与 wasm 都可用。
    pub fn load_log(&mut self) -> crate::error::Result<()> {
        const LOG_TCL: &str = include_str!("../log.tcl");
        self.eval(LOG_TCL).map(|_| ())
    }

    /// 加载 rules::* 词表层（规则/告警引擎，docs/ROADMAP.md §3）。
    /// after 轮询驱动；wasm 主线程形态需要宿主心跳把到点的 after
    /// 事件泵出来（www/event-loop.js）。
    pub fn load_rules(&mut self) -> crate::error::Result<()> {
        const RULES_TCL: &str = include_str!("../rules.tcl");
        self.eval(RULES_TCL).map(|_| ())
    }

    /// 注入宿主控制台实现（wasm 宿主由此把 `puts` 的输出接到 JS 回调；
    /// 原生构建默认已是 [`crate::host::NativeConsole`]，无需调用）。
    pub fn set_console(&mut self, console: Box<dyn crate::host::HostConsole>) {
        self.console = Rc::from(console);
    }

    /// 注入宿主 HTTP 实现（native `net` feature 由 rtcl-cli 注入 reqwest
    /// 实现；wasm 宿主不经此路径——走 JS `__rtcl_http` 桥 + 事件回灌）。
    pub fn set_http(&mut self, http: Box<dyn crate::host::HostHttp>) {
        self.http = http;
    }

    /// 注入进程执行服务（宿主实现 `HostExec::spawn`；不注入 = exec 不可用）。
    pub fn set_exec(&mut self, exec: Box<dyn crate::host::HostExec>) {
        self.exec = Some(exec);
    }

    /// 只读访问进程执行服务（未注入 → `None`）。
    pub fn host_exec(&self) -> Option<&dyn crate::host::HostExec> {
        self.exec.as_deref()
    }

    /// 注入宿主文件 fd 源（exec 重定向/heredoc 的 fd 来源；数据面
    /// `file`/`open` 仍走 `set_host_fs`，两层不同构）。
    pub fn set_fd_fs(&mut self, fd_fs: Box<dyn crate::host::HostFdFs>) {
        self.fd_fs = Some(fd_fs);
    }

    /// 只读访问宿主文件 fd 源（未注入 → `None`）。
    pub fn fd_fs(&self) -> Option<&dyn crate::host::HostFdFs> {
        self.fd_fs.as_deref()
    }

    /// 注入宿主作业表（`exec &` / `job *`）。
    pub fn set_jobs(&mut self, jobs: Box<dyn crate::host::HostJobs>) {
        self.jobs = Some(jobs);
    }

    /// 只读访问宿主作业表（未注入 → `None`）。
    pub fn host_jobs(&self) -> Option<&dyn crate::host::HostJobs> {
        self.jobs.as_deref()
    }

    /// 只读访问宿主 HTTP 服务。桥命令用：`request` 返回独立拥有的响应体，
    /// 借用不跨越逐块读取循环，读取期间可 re-entrant `eval` 回调。
    pub fn host_http(&self) -> &dyn crate::host::HostHttp {
        &*self.http
    }

    /// 注入宿主时钟实现（wasm 宿主由此给 `after`/`vwait`/`time` 接上
    /// JS 时钟；原生构建默认已是 [`crate::host::NativeClock`]，无需调用）。
    #[cfg(feature = "std")]
    pub fn set_clock(&mut self, clock: Box<dyn crate::host::HostClock>) {
        self.clock = Rc::from(clock);
    }

    /// Give a freshly built child interpreter what it starts with from
    /// its parent: the host services (the console, the clock) and the
    /// limits.  A child used to get the native ones — on a host that has
    /// no native clock (wasm) every `after`/`vwait`/`time` in it trapped
    /// and took the instance with it; with a host console its `puts`
    /// went nowhere — and no step budget, so `interp eval child {while
    /// {1} {}}` ran past the budget the host had armed.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub(crate) fn inherit_from(&mut self, parent: &Interp) {
        self.console = Rc::clone(&parent.console);
        #[cfg(feature = "std")]
        {
            self.clock = Rc::clone(&parent.clock);
        }
        self.steps = Rc::clone(&parent.steps);
        self.max_steps = parent.max_steps;
        self.max_call_depth = parent.max_call_depth;
    }


    /// A1 eval 步数预算：武装 `Some(n)` = 每次 eval 的命令步数上限，
    /// 超限抛 `ErrorCode::Timeout`；`None` = 不限（缺省）。武装即清零
    /// 计数——宿主每次顶层 eval 前武装一次，嵌套 eval/命令替换不重置
    ///（预算跨嵌套累计，防 `while {1} {eval {...}}` 绕过）。
    pub fn set_max_steps(&mut self, max: Option<u64>) {
        self.steps.set(0);
        self.max_steps = max;
        // The children share the counter; they get the new bound too.
        for child in self.child_interps.values_mut().flatten() {
            child.max_steps = max;
        }
    }

    /// 当前步数预算（None = 不限）。
    pub fn max_steps(&self) -> Option<u64> {
        self.max_steps
    }

    /// 递归/嵌套上限。同时约束 proc 递归（`call_depth`）与不经 proc
    /// 调用的脚本重入（[`Interp::nest_depth`]）。缺省值见
    /// [`default_max_call_depth`]；栈比缺省假设小的宿主（非主线程、
    /// 小栈的嵌入式线程）按栈预算下调，如
    /// `set_max_call_depth(stack_bytes / 5000)`。
    pub fn set_max_call_depth(&mut self, max: usize) {
        self.max_call_depth = max.max(1);
        // Children never get to recurse deeper than their parent.
        for child in self.child_interps.values_mut().flatten() {
            child.max_call_depth = self.max_call_depth;
        }
    }

    /// 当前递归/嵌套上限。
    pub fn max_call_depth(&self) -> usize {
        self.max_call_depth
    }

    /// 进入一层脚本重入（见 [`Interp::nest_depth`]）。每个
    /// `enter_nested` 与同一路径上的 `leave_nested` 成对——中间的调用
    /// 以 `Result` 返回（含 Suspend/exit flow），计数随栈展开恢复。
    pub(crate) fn enter_nested(&mut self) -> std::result::Result<(), crate::error::Error> {
        if self.nest_depth >= self.max_call_depth {
            return Err(crate::error::Error::recursion_limit());
        }
        self.nest_depth += 1;
        Ok(())
    }

    /// 离开一层脚本重入。
    pub(crate) fn leave_nested(&mut self) {
        self.nest_depth = self.nest_depth.saturating_sub(1);
    }

    /// Tier1 折叠代（只读）。Tier1 命令（`set`/`if`/`while`/…，PR#2 起
    /// 含 `lappend` 与宿主注册命令）被编译单元内联折叠，任何同名命令
    /// 的注册/改名/删除都会 bump 此计数；编译单元记录其编译时代的
    /// `epoch`，消费方（字节码 VM 的单元入口门、JIT 驱动的执行入口门）
    /// 比对两者判定折叠是否仍有效——不等则退回动态解析路径
    /// （namespace-41.1）。仅供嵌入式宿主/JIT 失效检查读取；bump 只
    /// 发生在解释器内部的注册/改名路径上，无 setter。
    pub fn tier1_epoch(&self) -> u64 {
        self.tier1_epoch
    }

    /// 执行 trace 是否在册（只读）。与 [`Self::tier1_epoch`] 同族的
    /// 嵌入式宿主/JIT 失效检查读取点：执行 trace 只在分派/树走查面
    /// 触发，任何在册 trace 都使「折叠 op 世界」（字节码 VM/JIT 的
    /// 内联 set/incr/lappend…）不可观察——与内部 `bytecode_applicable`
    /// 的首个门同口径。JIT 顶层驱动在单元入口以此回退解释器
    /// （M3：trace 先于单元注册的嵌入式窗口，h2/h5 探针面）。
    pub fn has_exec_traces(&self) -> bool {
        !self.exec_traces.is_empty()
    }

    /// JIT/嵌入式执行器的错误帧写入口（case-b：宿主原语错误不在嵌套
    /// eval 内发生，帧须由外部执行器按 vm_exec exec_inner 的形状补进
    /// 本累积器——proc 语境的 (procedure …) 边界帧依赖它）。与内部
    /// err_harness_frame/err_exit_frame/pending_top/err_line 同实现，
    /// 语义以 judge 差分为准。
    pub fn jit_err_harness_frame(&mut self, msg: &str, text: &str, line: usize) {
        self.err_harness_frame(msg, text, line);
    }

    /// 同上：`(tag line N)` 构造出口帧。
    pub fn jit_err_exit_frame(&mut self, tag: &str) {
        self.err_exit_frame(tag);
    }

    /// 同上：词括号 defer（pending 帧 + 行号——行号随帧走，defer 时
    /// 不动 err_line：构造出口帧读的是最内层日志命令的行，tclsh 的
    /// errorLine 只在日志事件处更新）。
    pub fn jit_err_set_pending(&mut self, text: &str, line: usize) {
        self.err_pending_top = Some((Rc::from(text), line, None));
        self.err_pending_unit_frame = true;
    }

    /// 同上：行号直设（exit_frame 前置读取的口径）。
    pub fn jit_err_set_line(&mut self, line: usize) {
        self.err_line = line;
    }

    /// 同上：当前行号（体帧的 body-relative 行计算——vm unwind_subs
    /// 的 n = err_line - (体首行 - 1) 同口径）。
    pub fn jit_err_line(&self) -> usize {
        self.err_line
    }

    /// 同上：defer 帧清除（err_harness_frame 的 superseded 规则暴露给
    /// JIT 驱动——本层 harness 已 append 时，Sub defer 的外围命令帧
    /// 被顶替）。
    pub fn jit_err_clear_pending(&mut self) {
        self.err_pending_top = None;
        self.err_pending_unit_frame = false;
        self.err_pending_chain.clear();
    }

    /// 同上：defer 帧在位查询——顶层 D-11 解码（decode_toplevel_flow）
    /// 的「外层胜」纪律：宿主分派路径的 defer 已种帧时，JIT 驱动不得
    /// 以自己的 cur_site 快照顶替（i2：`if 1 {return -code break}` 的
    /// 外围 if 帧由嵌套 eval 的 unwind_subs 种）。
    pub fn jit_err_has_pending(&self) -> bool {
        self.err_pending_top.is_some()
    }

    /// 同上：文件执行语境查询（unwind_subs 的体帧分支开关——rtcl-cli
    /// run_file 经 arm_file_script 武装；JIT 驱动同口径）。
    pub fn jit_err_in_file_script(&self) -> bool {
        self.in_file_script
    }

    /// 同上：文件语境直设。JIT 顶层单元不经 eval_impl（arm 的消费点
    /// 在那里）——wasm 单元须与「rtcl-cli 消费 arm 后的顶层单元」同
    /// 状态起步（宿主分派的嵌套 eval 各自 save/false/restore，不受
    /// 影响）。
    pub fn jit_err_set_file_script(&mut self, v: bool) {
        self.in_file_script = v;
    }

    /// 同上：无行号出口帧（vm 的 `("for" loop-end command)` 钉形）。
    pub fn jit_err_exit_frame_raw(&mut self, text: &str) {
        self.err_exit_frame_raw(text);
    }

    /// 同上：errorInfo 累积器在位查询（体帧 append 的 vm 语义 =
    /// `if let Some(info)`——累积器为空时跳过，不造首帧）。
    pub fn jit_err_has_info(&self) -> bool {
        self.err_info.is_some()
    }

    /// 同上：累积器 raw append（体帧退出行 `("while" body line N)` 的
    /// vm 形 = `if let Some(info) { info.push_str }`——空时跳过）。
    pub fn jit_err_append_info_raw(&mut self, text: &str) {
        if let Some(info) = &mut self.err_info {
            info.push_str(text);
        }
    }

    /// 每条命令分派计一步；超限抛错。解释器（`eval_command`）与 VM
    ///（`invoke_command`/`call`）两条分派路都过这里。
    /// 每条命令分派计一步；超限抛错。解释器（`eval_command`）与 VM
    ///（`invoke_command`/`call`）两条分派路都过这里。
    pub(crate) fn charge_step(&mut self) -> crate::error::Result<()> {
        if let Some(max) = self.max_steps {
            let steps = self.steps.get().saturating_add(1);
            self.steps.set(steps);
            if steps > max {
                return Err(crate::error::Error::runtime(
                    format!("eval step budget exceeded: {max}"),
                    crate::error::ErrorCode::Timeout,
                ));
            }
        }
        Ok(())
    }

    /// Command-table insert hook: append the key to the iteration log.
    /// Call right after a `commands`/`procs`/`ensembles`/`import_aliases`/
    /// `aliases` insert that returned `None` (i.e. the key was NEW):
    /// redefining a live command keeps its position, recreating a deleted
    /// one appends anew (Tcl hash-entry semantics).
    ///
    /// 内存审计 2026-11-20：注册序日志是 push-only——删除路径（rename
    /// away / 协程 destroy / namespace delete）不清条目，`info commands`
    /// 端按表活性过滤（行为不可见、内存滞留）。命令 churn 工作负载
    /// （协程 resume 命令、动态 proc 定义+rename away，oracle tclsh
    /// 同型零增长）此前线性增长（~100B/命令）。护栏：条目数超 4096
    /// 且超活命令数 4 倍时整表重建——保留活条目与注册次序（replay 序
    /// 不变），孤儿批量剪枝，滞留上界 = max(4096, 4×live)。
    pub(crate) fn note_new_command(&mut self, key: &str) {
        self.command_order.push(key.to_string());
        let live = self.commands.len()
            + self.procs.len()
            + self.ensembles.len()
            + self.import_aliases.len();
        if self.command_order.len() > 4096 && self.command_order.len() > 4 * live {
            self.rebuild_command_order_live();
        }
    }

    /// Rebuild [`Interp::command_order`] to live commands only (order
    /// preserved); same for the hidden-command set's dead entries.
    fn rebuild_command_order_live(&mut self) {
        let Self {
            command_order,
            hidden_commands,
            commands,
            procs,
            ensembles,
            import_aliases,
            ..
        } = self;
        let is_live = |k: &str| -> bool {
            let alt = format!("::{}", k);
            commands.contains_key(k)
                || procs.contains_key(k)
                || ensembles.contains_key(k)
                || import_aliases.contains_key(k)
                || commands.contains_key(&alt)
                || procs.contains_key(&alt)
                || ensembles.contains_key(&alt)
                || import_aliases.contains_key(&alt)
        };
        command_order.retain(|k| is_live(k));
        hidden_commands.retain(|k| is_live(k));
    }

    /// 注入文件系统数据面服务（wasm 宿主由此把 open/file/source 的
    /// 数据面接到 JS——内存 fs / OPFS worker）。默认 NativeFs。
    #[cfg(any(feature = "file", feature = "io"))]
    pub fn set_host_fs(&mut self, fs: Box<dyn crate::host::HostFs>) {
        self.host_fs = fs;
    }

    /// Hide a command from `info commands` (invokable but invisible —
    /// the mechanism behind host bridges and internal dispatch wrappers;
    /// probe with `namespace which`, not `info commands`).
    pub fn hide_command(&mut self, name: &str) {
        self.hidden_commands.insert(name.trim_start_matches("::").to_string());
    }

    /// Un-hide a command from `info commands` (the inverse of
    /// `hide_command`): wish lists its widget constructors and paths
    /// (`button`, `.b`, `.m` clone children) among the visible surface,
    /// so a Tk host bridge opts its registrations back in.
    pub fn show_command(&mut self, name: &str) {
        self.hidden_commands.remove(name.trim_start_matches("::"));
    }

    /// Whether `name` is currently hidden from listings.
    pub fn is_command_hidden(&self, name: &str) -> bool {
        self.hidden_commands.contains(name.trim_start_matches("::"))
    }

    /// Ensure a namespace and its ancestors exist, logging creation order
    /// for the `namespace children` replay.
    pub(crate) fn ensure_ns_path(&mut self, qualified: &str) {
        #[cfg(feature = "std")]
        if std::env::var_os("RTCL_DEBUG_NS").is_some() {
            eprintln!("ensure_ns_path: {qualified:?}");
        }
        if !self.namespaces.contains_key("::") {
            self.namespaces
                .insert("::".to_string(), Default::default());
            self.ns_order.push("::".to_string());
        }
        if qualified == "::" {
            return;
        }
        let parts: Vec<&str> =
            qualified.split("::").filter(|s| !s.is_empty()).collect();
        let mut path = String::new();
        for part in parts {
            path.push_str("::");
            path.push_str(part);
            if !self.namespaces.contains_key(&path) {
                self.namespaces
                    .insert(path.clone(), Default::default());
                self.ns_order.push(path.clone());
            }
        }
    }

    /// Populate special global variables (`$env`, `$tcl_platform`, etc.).
    ///
    /// The :: var table must iterate exactly like tclsh 8.6.17's startup
    /// (`info globals`: tcl_rcFileName tcl_version argv0 argv
    /// tcl_interactive auto_path auto_index env tcl_pkgPath
    /// tcl_patchLevel argc tcl_library tcl_platform; `array names
    /// tcl_platform` likewise) — the insertion sequence below replays to
    /// both orders (solved/validated in specs/tclhash/fit_model.py).
    /// Element keys (`arr(k)`) ride the same log and are filtered from
    /// the :: replay; their relative order drives per-array replay.
    fn init_special_vars(&mut self) {
        // --- 启动数组容器先行（D1 步骤 3）：元素表 = 宿主 Var 的子表
        // ——元素写入前宿主必须已是数组（标记建无值 Var + 子表；基键
        // 的值随后照旧落表——插入序红线不动：无值标记被下方基键
        // global_insert 按「占位上值 = 新建序位」搬到落表位）。
        self.global_mark_array("tcl_platform", Some(0));
        #[cfg(feature = "env")]
        self.global_mark_array("env", Some(0));
        #[cfg(not(feature = "env"))]
        self.global_mark_array("env", None);

        // --- $env array (mirror OS environment; environ order) ---
        // `vars_os`: `std::env::vars()` panics on a variable that is not
        // valid Unicode, which made `Interp::new()` abort the host (one
        // stray RTCL_BAD=\xff\xfe in the environment was enough); tclsh
        // passes the bytes through. Lossy here (the string is Tcl text).
        #[cfg(feature = "env")]
        let mut env_elems = 0usize;
        #[cfg(feature = "env")]
        for (key, val) in std::env::vars_os() {
            let full = format!("env({})", key.to_string_lossy());
            self.global_insert(full, Value::from_str(&val.to_string_lossy()));
            env_elems += 1;
        }

        // --- $tcl_platform array (element order replays to tclsh's) ---
        self.global_insert("tcl_platform(byteOrder)".to_string(),
            Value::from_str(if cfg!(target_endian = "little") { "littleEndian" } else { "bigEndian" }));
        self.global_insert("tcl_platform(pointerSize)".to_string(),
            Value::from_int(std::mem::size_of::<*const ()>() as i64));
        self.global_insert("tcl_platform(osVersion)".to_string(), Value::from_str(""));
        self.global_insert(
            "tcl_platform(platform)".to_string(),
            Value::from_str(if cfg!(unix) { "unix" } else if cfg!(windows) { "windows" } else { "unknown" }),
        );
        self.global_insert(
            "tcl_platform(machine)".to_string(),
            Value::from_str(std::env::consts::ARCH),
        );
        self.global_insert("tcl_platform(threaded)".to_string(), Value::from_int(1));
        self.global_insert("tcl_platform(engine)".to_string(), Value::from_str("rtcl"));
        self.global_insert("tcl_platform(os)".to_string(),
            Value::from_str(std::env::consts::OS));
        self.global_insert("tcl_platform(pathSeparator)".to_string(), Value::from_str(":"));
        self.global_insert("tcl_platform(wordSize)".to_string(),
            Value::from_int(std::mem::size_of::<usize>() as i64));
        #[cfg(feature = "std")]
        self.global_insert(
            "tcl_platform(user)".to_string(),
            Value::from_str(&std::env::var("USER").unwrap_or_default()),
        );
        // D1 步骤 1/3：两个启动数组的位/计数入 arena（global_mark_array
        // ——对已标记宿主只更新计数）。元素已在上方入子表（D1 步骤 3
        // ——元素形键经 global_insert 落宿主子表，平键域不再持有元素
        // 键；`array names` 序 = 子表插入序的重放，与旧平键序逐字同）。
        // env 无 env 特性时无计数（与旧平表同形）。
        {
            let n = self
                .var_arena
                .root()
                .get("tcl_platform")
                .and_then(|v| v.elements())
                .map(|t| t.len())
                .unwrap_or(0);
            self.global_mark_array("tcl_platform", Some(n as u32));
        }
        #[cfg(feature = "env")]
        {
            self.global_mark_array("env", Some(env_elems as u32));
        }
        self.arrays_env_only = true;

        // --- :: var table in tclsh's iteration order ---
        // ::auto_index 惰性首建（2026-11-16 var-order 轮）：tclsh 文件
        // 模式启动不含 auto_index——unknown→auto_load 首次运行时由
        // auto_load_index 创建（oo2 r4 / ai* 差分）。rtcl 的 stdlib
        // auto_load_index 移植体在触发时同样 set auto_index(...)。
        // tcl_rcFileName 是 Tcl_Main 在 init.tcl 批**之前**设的应用
        // 变量(IVQ 钉形:`info globals`/`info vars` 全局序以它为首,
        // 同桶链头位)——先插,再进 init.tcl 批。
        self.global_insert(
            "tcl_rcFileName".to_string(),
            Value::from_str("~/.tclshrc"),
        );
        self.global_insert("tcl_version".to_string(), Value::from_str("8.6"));
        self.global_insert("argv0".to_string(), Value::from_str(""));
        self.global_insert("argv".to_string(), Value::from_str(""));
        self.global_insert("tcl_interactive".to_string(), Value::from_int(0));
        self.global_insert("argc".to_string(), Value::from_int(0));
        // `env` base sits between argc and tcl_pkgPath in tclsh's
        // insertion sequence (replay-validated; see fn docs).
        self.global_insert("env".to_string(), Value::empty());
        self.global_insert(
            "tcl_pkgPath".to_string(),
            Value::from_str(
                "/usr/local/lib/tcltk /usr/local/share/tcltk /usr/lib/tcltk/x86_64-linux-gnu /usr/lib/tcltk /usr/share/tcltk /usr/lib/tcltk/tcl8.6 /usr/lib",
            ),
        );
        self.global_insert("auto_path".to_string(), Value::from_str(""));
        self.global_insert("tcl_patchLevel".to_string(), Value::from_str("8.6.17"));
        self.global_insert("tcl_library".to_string(), Value::from_str("/usr/share/tcltk/tcl8.6"));
        self.global_insert("tcl_platform".to_string(), Value::empty());
    }

    /// Drop the `history` command wrapper (tclsh registers it only for
    /// stdin/interactive runs — tclMain's RecordAndEvalObj `history add`
    /// auto-load; a **file-argument or `-c` run has no `history` command**,
    /// 2026-11-15 实测)。Removes the `::history` proc (the `::tcl::history`
    /// implementation stays, as in tclsh) and its command-order entries so
    /// the `info commands` replay matches tclsh's file-mode table.
    pub fn drop_stdin_history(&mut self) {
        // ::tcl::history 实体同随 stdin 语境消失（tclsh 文件模式
        // `info commands ::tcl::*` = Bgerror/CopyDirectory/pkgconfig/
        // prefix 四项——history proc 由 history add 触发 auto-load，
        // probe1 面计数对齐）。
        for key in ["::history", "history", "::tcl::history"] {
            self.procs.remove(key);
            self.commands.remove(key);
            self.import_aliases.remove(key);
            self.command_order.retain(|k| k != key);
        }
        self.note_cmd_mutation();
    }

    /// Whether we are inside a procedure scope.
    #[allow(dead_code)]
    pub(crate) fn in_proc(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Resolution-cache probe: the memoised winner for invoking `name`
    /// in the current namespace, if one was cached under the live
    /// command-table generation.  Two Fx hashes, zero allocations.
    pub(crate) fn cmd_cache_get(&self, name: &str) -> Option<ResolvedCmd> {
        if self.current_namespace.as_ref() == "::" {
            let cached = self.cmd_cache_root.get(name)?;
            return (cached.gen == self.cmd_generation).then(|| cached.target.clone());
        }
        let inner = self.cmd_cache.get(self.current_namespace.as_ref())?;
        let cached = inner.get(name)?;
        (cached.gen == self.cmd_generation).then(|| cached.target.clone())
    }

    /// Resolution-cache insert (miss path only — the full chain just ran).
    /// `note_cmd_mutation` bumps make stale entries miss, so an insert
    /// here always overwrites with a fresh verdict for the same key.
    pub(crate) fn cmd_cache_put(&mut self, name: &str, target: ResolvedCmd) {
        const CMD_CACHE_MAX: usize = 8192;
        if self.cmd_cache_len >= CMD_CACHE_MAX {
            self.cmd_cache.clear();
            self.cmd_cache_root.clear();
            self.cmd_cache_len = 0;
        }
        let entry = CachedCmd { gen: self.cmd_generation, target };
        if self.current_namespace.as_ref() == "::" {
            if self.cmd_cache_root.insert(name.to_string(), entry).is_none() {
                self.cmd_cache_len += 1;
            }
            return;
        }
        match self.cmd_cache.get_mut(self.current_namespace.as_ref()) {
            Some(inner) => {
                if inner.insert(name.to_string(), entry).is_none() {
                    self.cmd_cache_len += 1;
                }
            }
            None => {
                let mut inner: VarMap<CachedCmd> = VarMap::default();
                inner.insert(name.to_string(), entry);
                self.cmd_cache
                    .insert(self.current_namespace.to_string(), inner);
                self.cmd_cache_len += 1;
            }
        }
    }

    /// Clear every compile cache together and reset its byte counter.
    /// Used by the count/byte high-water guards so a single eviction
    /// cannot leave a half-evicted unit family behind.
    pub(crate) fn clear_compile_caches(&mut self) {
        self.parse_cache.clear();
        self.bytecode_cache.clear();
        self.const_pool.clear();
        self.parse_cache_bytes = 0;
        self.bytecode_cache_bytes = 0;
        self.const_pool_bytes = 0;
    }

    /// Materialise `code`'s constants into the literal pool (idempotent
    /// overwrite; units without constants skip the pool entirely).
    /// Called once at each compile seam — a cached unit then serves every
    /// later execution.
    pub(crate) fn const_pool_insert(&mut self, code: &Rc<ByteCode>) {
        // 2048: a stdlib load inserts several hundred proc bodies; 512
        // overflowed mid-session and the clear-on-overflow evicted the
        // caller's units with no way back (pre-G8g they stayed evicted).
        const CONST_POOL_MAX: usize = 2048;
        if code.constants().is_empty() {
            return;
        }
        let values: Rc<[Value]> = code
            .constants()
            .iter()
            .map(|s| Value::from_str(s))
            .collect();
        let value_bytes: usize = values
            .iter()
            .map(|v| v.as_str().len() + 32)
            .sum();
        if self.const_pool.len() >= CONST_POOL_MAX
            || self.const_pool_bytes + value_bytes > CONST_POOL_BYTE_BUDGET
        {
            self.const_pool.clear();
            self.const_pool_bytes = 0;
        }
        self.const_pool.insert(
            Rc::as_ptr(code) as usize,
            ConstPoolEntry {
                code: Rc::clone(code),
                values,
                sites: Rc::default(),
            },
        );
        self.const_pool_bytes += value_bytes;
    }

    /// Drop a dead proc's compiled unit from the literal/call-site pool and
    /// clear any CmdSite token that still points at its `ProcDef`.  Called
    /// on redefinition/deletion so old bodies/bytecode are not pinned until
    /// the 2048-entry pool clear.
    pub(crate) fn purge_proc_artifacts(&mut self, def: &Rc<ProcDef>) {
        if let Some(code) = &def.compiled {
            self.const_pool.remove(&(Rc::as_ptr(code) as usize));
            crate::interp::vm_exec::run_bytecode_retire_hook(Rc::as_ptr(code));
        }
        for entry in self.const_pool.values() {
            let mut sites = entry.sites.borrow_mut();
            for (_, site) in sites.iter_mut() {
                if site
                    .def
                    .as_ref()
                    .is_some_and(|d| Rc::ptr_eq(d, def))
                {
                    site.def = None;
                }
            }
        }
    }

    /// Pool probe for a unit being executed (one hash per `exec_bytecode`
    /// call, amortised over every PushConst and call-site token it uses):
    /// the literal slice plus the call-site token table.
    pub(crate) fn const_pool_get(&self, code: &ByteCode) -> Option<LoadedUnitParts> {
        self.const_pool
            .get(&(code as *const ByteCode as usize))
            .map(|e| (Rc::clone(&e.values), Rc::clone(&e.sites)))
    }

    /// A command table (`commands` / `procs` / `ensembles` /
    /// `import_aliases`) is about to change / has just changed: age every
    /// resolution-cache entry out at once.  Cheap by construction — call
    /// it from every mutation site; resolution-affecting mutations never
    /// sit in hot loops.
    pub(crate) fn note_cmd_mutation(&mut self) {
        self.cmd_generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- A1 eval 步数预算（huijuan B7 前置：agent 脚本防跑飞） ----

    #[test]
    fn step_budget_trips_on_runaway_while() {
        let mut interp = Interp::new();
        interp.set_max_steps(Some(100));
        let r = interp.eval("set n 0; while {1} {incr n}");
        let msg = format!("{}", r.expect_err("runaway while must trip budget"));
        assert!(msg.contains("step budget"), "{msg}");
    }

    #[test]
    fn step_budget_trips_on_empty_body_while() {
        // TASKS A1 的原始案例：空体 + 字面条件——编译形里零命令分派，
        // 只能靠 VM 回边计步。
        let mut interp = Interp::new();
        interp.set_max_steps(Some(100));
        let r = interp.eval("while {1} {}");
        let msg = format!("{}", r.expect_err("empty-body while must trip budget"));
        assert!(msg.contains("step budget"), "{msg}");
        // 字面条件 + continue 体同样拦下。
        let mut interp = Interp::new();
        interp.set_max_steps(Some(100));
        assert!(interp.eval("while {1} {continue}").is_err());
    }

    #[test]
    fn step_budget_default_unlimited_and_rearm_resets() {
        let mut interp = Interp::new();
        // 缺省不限：大循环照常完成（conformance 零行为变化）。
        assert!(interp.max_steps().is_none());
        interp.eval("set n 0; while {$n < 2000} {incr n}").unwrap();
        assert_eq!(interp.eval("set n").unwrap().as_str(), "2000");
        // 武装即清零：上一段 2000+ 步不累计进下一段预算。
        interp.set_max_steps(Some(50));
        interp.eval("set m 0; while {$m < 10} {incr m}").unwrap();
        assert_eq!(interp.eval("set m").unwrap().as_str(), "10");
    }

    #[test]
    fn step_budget_covers_loops_that_dispatch_nothing() {
        // `timerate {} <ms>` iterates an empty script and an assemble
        // `jump` loop runs instructions only: neither dispatches a
        // command, so both used to outrun an armed budget.
        let mut interp = Interp::new();
        interp.set_max_steps(Some(1000));
        let r = interp.eval("timerate {} 3600000");
        let msg = format!("{}", r.expect_err("timerate iterations must charge steps"));
        assert!(msg.contains("step budget"), "{msg}");
        let mut interp = Interp::new();
        interp.set_max_steps(Some(1000));
        let r = interp.eval("::tcl::unsupported::assemble {label L; jump L}");
        let msg = format!("{}", r.expect_err("assemble instructions must charge steps"));
        assert!(msg.contains("step budget"), "{msg}");
    }

    #[test]
    fn timerate_rejects_negative_counts() {
        // `as u64` used to wrap a negative duration into a deadline days
        // away.
        let mut interp = Interp::new();
        let r = interp.eval("timerate {} -1");
        let msg = format!("{}", r.expect_err("negative duration"));
        assert!(msg.contains("bad duration: must be an integer >= 0"), "{msg}");
        let r = interp.eval("timerate {} 10 -5");
        let msg = format!("{}", r.expect_err("negative maxcount"));
        assert!(msg.contains("bad maxcount: must be an integer >= 0"), "{msg}");
    }

    #[test]
    fn recursion_limit_is_configurable() {
        let mut interp = Interp::new();
        assert_eq!(interp.max_call_depth(), default_max_call_depth());
        interp.set_max_call_depth(20);
        let r = interp.eval("proc f {n} {f $n}; f 1");
        let msg = format!("{}", r.expect_err("must hit the lowered limit"));
        // tclsh's wording for its own limit (Tcl_LimitCheck) — and the
        // errorCode a `catch` reports with it.
        assert_eq!(msg, "too many nested evaluations (infinite loop?)");
        let code = interp
            .eval("catch {f 1} m; set ::errorCode")
            .expect("the catch returns");
        assert_eq!(code.as_str(), "TCL LIMIT STACK");
        interp.set_max_call_depth(0);
        assert_eq!(interp.max_call_depth(), 1, "a limit is never zero");
    }

    #[test]
    fn step_budget_catches_proc_bodies_and_recursion() {
        let mut interp = Interp::new();
        // 预算 45（≈15 层递归处拦截）：master 的 VM 帧比旧线胖，debug
        // 深递归会先于预算撞上 2MB 测试线程栈——浅拦截同样证明 proc
        // 体与递归都被计步。
        interp.set_max_steps(Some(45));
        let r = interp.eval(
            "proc f {n} { if {$n <= 0} { return 0 }; expr {[f [expr {$n - 1}]] + 1} }\nf 500",
        );
        assert!(r.is_err(), "runaway recursion must trip budget");
    }

    #[test]
    fn test_set_var() {
        let mut interp = Interp::new();
        interp.eval("set x 42").unwrap();
        assert_eq!(interp.get_var("x").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_expr() {
        let mut interp = Interp::new();
        let result = interp.eval("expr 1 + 2").unwrap();
        assert_eq!(result.as_int(), Some(3));
    }

    #[test]
    fn test_while() {
        let mut interp = Interp::new();
        interp.eval("set i 0").unwrap();
        interp.eval("while {$i < 5} { incr i }").unwrap();
        assert_eq!(interp.get_var("i").unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_tailcall_factorial() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc fact {n acc} {
                    if {$n <= 1} { return $acc }
                    tailcall fact [expr {$n - 1}] [expr {$n * $acc}]
                }",
            )
            .unwrap();
        let result = interp.eval("fact 10 1").unwrap();
        assert_eq!(result.as_str(), "3628800");
    }

    #[test]
    fn test_tailcall_deep_no_overflow() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc countdown {n} {
                    if {$n <= 0} { return done }
                    tailcall countdown [expr {$n - 1}]
                }",
            )
            .unwrap();
        // Without TCO this would overflow the 1000-deep call stack
        let result = interp.eval("countdown 5000").unwrap();
        assert_eq!(result.as_str(), "done");
    }

    #[test]
    fn test_tailcall_mutual_recursion() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc tc_even {n} {
                    if {$n == 0} { return 1 }
                    tailcall tc_odd [expr {$n - 1}]
                }
                proc tc_odd {n} {
                    if {$n == 0} { return 0 }
                    tailcall tc_even [expr {$n - 1}]
                }",
            )
            .unwrap();
        let result = interp.eval("tc_even 100").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    // --- stdlib.tcl tests ---

    #[test]
    fn test_stdlib_throw_error() {
        let mut interp = Interp::new();
        let err = interp.eval("throw error {something broke}").unwrap_err();
        assert!(err.to_string().contains("something broke"));
    }

    #[test]
    fn test_stdlib_throw_list_errorcode() {
        // dict-deep：throw {E C} msg —— code 是列表形 errorCode。
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {throw {E C} msg} m; list $m $::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "msg {E C}");
    }

    #[test]
    fn test_stdlib_throw_ok() {
        // tclsh 8.6 throw 语义：code 是 errorCode（任意串），一律报错。
        let mut interp = Interp::new();
        let code = interp.eval("catch {throw ok hello} m; list $m $::errorCode").unwrap();
        assert_eq!(code.as_str(), "hello ok");
    }

    #[test]
    fn test_stdlib_throw_catch() {
        let mut interp = Interp::new();
        let result = interp
            .eval("catch {throw error oops} msg; set msg")
            .unwrap();
        assert_eq!(result.as_str(), "oops");
    }

    #[test]
    fn test_stdlib_parray() {
        let mut interp = Interp::new();
        // parray writes to stdout; just verify it doesn't error
        interp.eval("array set x {a 1 b 2 c 3}").unwrap();
        interp.eval("parray x").unwrap();
    }

    // --- auto_qualify (stdlib.tcl, tclsh 8.6 init.tcl parity / gen_init) ---

    #[test]
    fn test_auto_qualify_manpage_examples() {
        let mut interp = Interp::new();
        let cases = [
            ("::foo::bar ::blue", "::foo::bar"),
            ("::global ::sub", "global"),
            ("nocolons ::", "nocolons"),
            ("nocolons ::sub", "::sub::nocolons nocolons"),
            ("foo::bar ::", "::foo::bar"),
            ("foo::bar ::sub", "::sub::foo::bar ::foo::bar"),
            (":::foo::::bar ::blue", "::foo::bar"),
            (":::foo ::bar", "foo"),
        ];
        for (call, want) in cases {
            let got = interp
                .eval(&format!("auto_qualify {call}"))
                .unwrap()
                .as_str()
                .to_string();
            assert_eq!(got, want, "auto_qualify {call}");
        }
    }

    // --- ::tcl::pkgconfig (misc.rs, tclPkgConfig.c parity / gen_config) ---

    #[test]
    fn test_pkgconfig_surface() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("llength [::tcl::pkgconfig list]")
                .unwrap()
                .as_str(),
            "18"
        );
        assert_eq!(
            interp
                .eval("::tcl::pkgconfig get bindir,install")
                .unwrap()
                .as_str(),
            "/usr/bin"
        );
        assert_eq!(
            interp
                .eval("catch {::tcl::pkgconfig get foo} m; set ::errorCode")
                .unwrap()
                .as_str(),
            "TCL LOOKUP CONFIG foo"
        );
    }

    #[test]
    fn test_pkgconfig_arity_and_subcommand_errors() {
        let mut interp = Interp::new();
        let cases = [
            (
                "catch {::tcl::pkgconfig} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig subcommand ?arg?\"",
            ),
            (
                "catch {::tcl::pkgconfig foo} m; set m",
                "bad subcommand \"foo\": must be get or list",
            ),
            (
                "catch {::tcl::pkgconfig foo} m; set ::errorCode",
                "TCL LOOKUP INDEX subcommand foo",
            ),
            (
                "catch {::tcl::pkgconfig list foo} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig list\"",
            ),
            (
                "catch {::tcl::pkgconfig get} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig get key\"",
            ),
            (
                "catch {::tcl::pkgconfig get foo bar} m; set m",
                "wrong # args: should be \"::tcl::pkgconfig subcommand ?arg?\"",
            ),
            // Prefix resolution + ambiguity (Tcl_GetIndexFromObj semantics).
            ("llength [::tcl::pkgconfig l]", "18"),
            (
                "catch {::tcl::pkgconfig \"\"} m; set m",
                "ambiguous subcommand \"\": must be get or list",
            ),
            (
                "catch {::tcl::pkgconfig \"\"} m; set ::errorCode",
                "TCL LOOKUP INDEX subcommand {}",
            ),
        ];
        for (script, want) in cases {
            let got = interp.eval(script).unwrap().as_str().to_string();
            assert_eq!(got, want, "script: {script}");
        }
    }

    // --- Phase 5B stdlib tests ---

    #[test]
    fn test_stdlib_function() {
        let mut interp = Interp::new();
        let result = interp.eval("function hello").unwrap();
        assert_eq!(result.as_str(), "hello");
    }

    #[test]
    fn test_stdlib_lambda() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set f [lambda {x} { expr {$x * 2} }]
            $f 21
        "#).unwrap();
        assert_eq!(result.as_str(), "42");
    }

    #[test]
    fn test_stdlib_curry() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set add5 [curry expr 5 +]
            $add5 10
        "#).unwrap();
        assert_eq!(result.as_str(), "15");
    }

    #[test]
    fn test_stdlib_loop() {
        let mut interp = Interp::new();
        interp.eval(r#"
            set sum 0
            loop i 1 6 {
                incr sum $i
            }
        "#).unwrap();
        let result = interp.eval("set sum").unwrap();
        assert_eq!(result.as_str(), "15");
    }

    #[test]
    fn test_stdlib_loop_with_step() {
        let mut interp = Interp::new();
        interp.eval(r#"
            set vals {}
            loop i 0 10 2 {
                lappend vals $i
            }
        "#).unwrap();
        let result = interp.eval("set vals").unwrap();
        assert_eq!(result.as_str(), "0 2 4 6 8");
    }

    
    #[test]
    fn test_stdlib_ensemble() {
        let mut interp = Interp::new();
        interp.eval(r#"
            proc {myns add} {a b} { expr {$a + $b} }
            proc {myns mul} {a b} { expr {$a * $b} }
            ensemble myns
        "#).unwrap();
        let result = interp.eval("myns add 3 4").unwrap();
        assert_eq!(result.as_str(), "7");
        let result = interp.eval("myns mul 3 4").unwrap();
        assert_eq!(result.as_str(), "12");
    }

    #[test]
    fn test_fileevent_native_surface() {
        let mut interp = Interp::new();
        // 查询/设置/查询 + 方向与未知通道错误（原生命令，不再是 shim）。
        assert_eq!(interp.eval("fileevent stdin readable").unwrap().as_str(), "");
        assert_eq!(
            interp.eval("fileevent stdin readable {puts hi}").unwrap().as_str(),
            ""
        );
        assert_eq!(interp.eval("fileevent stdin readable").unwrap().as_str(), "puts hi");
        assert_eq!(interp.eval("fileevent stdout writable").unwrap().as_str(), "");
        assert!(interp.eval("fileevent nosuch readable {}").is_err());
        assert!(interp.eval("fileevent stdout readable {}").is_err());
        assert!(interp.eval("fileevent stdin writable {}").is_err());
    }

    #[test]
    fn test_stdlib_json_encode_string() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"json::encode "hello world""#).unwrap();
        assert_eq!(result.as_str(), "\"hello world\"");
    }

    #[test]
    fn test_stdlib_json_encode_num() {
        let mut interp = Interp::new();
        let result = interp.eval("json::encode 42 num").unwrap();
        assert_eq!(result.as_str(), "42");
    }

    #[test]
    fn test_stdlib_error_info() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"errorInfo "something failed""#).unwrap();
        assert!(result.as_str().contains("something failed"));
    }

    #[test]
    fn test_stdlib_namespace_inscope() {
        let mut interp = Interp::new();
        interp.eval(r#"
            namespace eval foo {
                proc bar {} { return "in foo" }
            }
        "#).unwrap();
        let result = interp.eval("namespace inscope foo bar").unwrap();
        assert_eq!(result.as_str(), "in foo");
    }

    // --- Variable names with special characters ---

    #[test]
    fn test_var_braced_path_slash() {
        let mut interp = Interp::new();
        interp.eval(r#"set {path/file.exe} "hello""#).unwrap();
        let result = interp.eval("set {path/file.exe}").unwrap();
        assert_eq!(result.as_str(), "hello");
        // Also verify ${} deref syntax in a proc
        interp.eval("proc getit {} { global {path/file.exe}; set x ${path/file.exe} }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "hello");
    }

    #[test]
    fn test_var_braced_absolute_path() {
        let mut interp = Interp::new();
        interp.eval(r#"set {/usr/local/bin/prog} "world""#).unwrap();
        interp.eval("proc getit {} { global {/usr/local/bin/prog}; set x ${/usr/local/bin/prog} }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "world");
    }

    #[test]
    fn test_var_braced_dots() {
        let mut interp = Interp::new();
        interp.eval(r#"set {config.server.host} "localhost""#).unwrap();
        interp.eval("proc getit {} { global {config.server.host}; set x ${config.server.host} }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "localhost");
    }

    #[test]
    fn test_var_bare_dot_boundary() {
        let mut interp = Interp::new();
        // $foo.bar should be $foo + ".bar", not variable "foo.bar"
        interp.eval("set foo test").unwrap();
        interp.eval("proc getit {} { global foo; set x $foo.bar }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "test.bar");
    }

    #[test]
    fn test_var_braced_with_suffix() {
        let mut interp = Interp::new();
        interp.eval(r#"set {a.b} "test""#).unwrap();
        interp.eval("proc getit {} { global {a.b}; set x ${a.b}.x }").unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "test.x");
    }

    #[test]
    fn test_var_braced_in_string() {
        let mut interp = Interp::new();
        interp.eval(r#"set {path/file.exe} "/bin/ls""#).unwrap();
        interp.eval(r#"proc getit {} { global {path/file.exe}; set x "exe=${path/file.exe}" }"#).unwrap();
        let result = interp.eval("getit").unwrap();
        assert_eq!(result.as_str(), "exe=/bin/ls");
    }

    #[test]
    fn test_var_array_dot_slash_key() {
        let mut interp = Interp::new();
        interp.eval(r#"set arr(a.b/c) "value""#).unwrap();
        let result = interp.eval("set arr(a.b/c)").unwrap();
        assert_eq!(result.as_str(), "value");
    }

    #[test]
    fn test_stdlib_defer() {
        let mut interp = Interp::new();
        interp.eval(r#"
            set log {}
            proc cleanup {} {
                global log
                defer {global log; lappend log "deferred1"}
                defer {global log; lappend log "deferred2"}
                lappend log "body"
            }
            cleanup
        "#).unwrap();
        let result = interp.eval("set log").unwrap();
        // defer runs in reverse order on proc exit
        assert_eq!(result.as_str(), "body deferred2 deferred1");
    }
}

// ── ⑧ wasm32 重入哨兵 ─────────────────────────────────────────────────────
//
// JIT 的 wasm 执行期间（rtcl-jit 的 run() 括起的窗口），JS 侧对同一解释
// 器的重入（`RtclHandle::exec`/`dispatch_event`/`register_command` 等
// wasm-bindgen 导出）会在外层 wasm 帧仍持有解释器裸指针时重新进入求值
// ——单线程顺序执行下无数据竞争，但嵌套求值会经由 proc-body 钩子递归
// JIT 化同一 proc（池条目正被外层占用），栈纪律与错误帧的契约都未定义。
// 哨兵放在 rtcl-vm（rtcl-jit 与 rtcl-wasm 共同可见的层）：rtcl-jit 在
// run() 前后武装/解除，rtcl-wasm 的重入入口检测并以明确错误拒绝。
thread_local! {
    static JIT_CALL_IN_FLIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 武装/解除「JIT wasm 调用进行中」哨兵（rtcl-jit 专用；RAII 成对调用）。
pub fn set_jit_call_in_flight(in_flight: bool) {
    JIT_CALL_IN_FLIGHT.with(|f| f.set(in_flight));
}

/// JIT wasm 调用是否进行中（wasm32 宿主重入入口的检测点）。
pub fn jit_call_in_flight() -> bool {
    JIT_CALL_IN_FLIGHT.with(|f| f.get())
}

// ── fast-tier 跨帧写哨兵：upvar/global 命令链接到非当前帧时置位；
// rtcl-jit 的 fast linker 在每次分派后读取并写入 wasm dirty global
//（调用后槽 resync 的运行时门——常见路径零宿主读）。
thread_local! {
    static JIT_CROSS_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 置位跨帧写哨兵（upvar/global 命令实现调用；帧深度>0 时）。
pub fn jit_note_cross_frame() {
    JIT_CROSS_FRAME.with(|f| f.set(true));
}

/// 读取并清除跨帧写哨兵（fast linker 分派后调用）。
pub fn jit_take_cross_frame() -> bool {
    JIT_CROSS_FRAME.with(|f| f.replace(false))
}

/// Embedding high-water budgets for the count-bounded caches.  They are
/// deliberately conservative totals (not per-entry limits): a script can
/// fill a count cap with maximum-size entries and otherwise reach hundreds
/// of MB.  Clearing is whole-cache (no LRU), matching the existing policy.
pub(crate) const PARSE_CACHE_BYTE_BUDGET: usize = 32 * 1024 * 1024;
pub(crate) const BYTECODE_CACHE_BYTE_BUDGET: usize = 64 * 1024 * 1024;
pub(crate) const CONST_POOL_BYTE_BUDGET: usize = 32 * 1024 * 1024;
pub(crate) const LAMBDA_CACHE_BYTE_BUDGET: usize = 16 * 1024 * 1024;
pub(crate) const EXPR_CACHE_BYTE_BUDGET: usize = 16 * 1024 * 1024;

impl Interp {
    /// Tell the interpreter which instance it is in its host's own
    /// tables (see [`Interp::host_instance_id`]).
    pub fn set_host_instance_id(&mut self, id: u64) {
        self.host_instance_id = id;
    }

    /// The instance id the host gave this interpreter
    /// ([`Interp::set_host_instance_id`]); `u64::MAX` when none.
    pub fn host_instance_id(&self) -> u64 {
        self.host_instance_id
    }

    /// Close every channel the way leaving the process should: without
    /// waiting for a pipe child or a pending connection.  A host about to
    /// exit calls this before dropping the interpreter; the plain `Drop`
    /// still closes, and waits — an interpreter dropped inside a live
    /// process must not leave children unreaped.
    pub fn abandon_channels(&mut self) {
        self.channels.abandon_all();
    }
}

/// Dropping the last interpreter handle releases OS resources via field
/// drops (ChannelTable's Drop closes files/pipes) and gives the optional
/// JIT engine a chance to drop thread-local pools/modules keyed by this
/// interpreter pointer.  No Tcl code runs here.
impl Drop for Interp {
    fn drop(&mut self) {
        #[cfg(feature = "exec")]
        self.reap_background_children();
        crate::interp::vm_exec::run_interp_teardown_hook(self as *mut Interp);
    }
}
