//! Tcl interpreter — executes parsed commands.
//!
//! The interpreter is split into:
//! - This file: [`Interp`] struct definition and constructor
//! - [`registry`]: built-in command registration and CmdId dispatch
//! - [`vars`]: variable access methods
//! - [`eval`]: script evaluation and word expansion
//! - [`call`]: procedure calls and tail-call optimisation
//! - [`vm_bridge`]: [`VmContext`](rtcl_vm::VmContext) implementation
//! - [`util`]: shared helpers (`split_array_ref`, `glob_match`)
//! - [`commands`] submodules: individual command implementations

pub mod commands;
pub mod unicode;
mod registry;
mod vars;
mod eval;
mod call;
mod vm_bridge;
mod vm_exec;
mod util;
mod varmap;

// Re-export utilities so command modules can reach them via `super::super::glob_match`
pub(crate) use util::{split_array_ref, glob_match};
pub(crate) use varmap::{VarMap, VarSet};

pub use vm_exec::set_bytecode_disabled;

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
    /// Link to `globals[name]`.
    Global(String),
    /// Link to `frames[frame_index].locals[name]`.
    Frame { frame_index: usize, var_name: String },
    /// The link's target was destroyed by a `namespace delete` (or the
    /// containing array was unset) while the alias was live.  Reads keep
    /// working against the alias's seeded local copy; writes error
    /// (tclsh 8.6.17).  `array` distinguishes the two error messages.
    Dead { array: bool, target: String },
}

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
    pub locals: VarMap<Value>,
    /// Names in `locals` that are arrays (scalar/array distinction).
    pub array_locals: VarSet,
    pub upvars: VarMap<UpvarLink>,
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
    pub slots: Vec<Option<Value>>,
    /// Parallel to `slots`: a slot whose storage moved to `locals` because
    /// an exceptional structure was installed on its NAME (an upvar /
    /// `global` / `variable` link — see
    /// [`Interp::degrade_frame_link`]).  Slot ops consult this one bool
    /// (an indexed read, no hashing) and defer to the name path, which
    /// resolves the link; every other slot keeps its compiled fast path,
    /// so a linked method variable no longer degrades the WHOLE frame to
    /// name-keyed lookups.
    pub slot_aliased: Vec<bool>,
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

    /// Current value of the named slot (`None` when unset or absent).
    pub(crate) fn slot_value(&self, name: &str) -> Option<&Value> {
        let i = self.slot_index_of(name)?;
        if self.slot_aliased.get(i).copied().unwrap_or(false) {
            return None;
        }
        self.slots[i].as_ref()
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

/// One [`Interp::const_pool`] entry: a compiled unit's string constants
/// pre-materialised as `Value`s (tclsh's literal table).  Holds the
/// `Rc<ByteCode>` keepalive so the address key stays valid for the
/// entry's lifetime.
pub(crate) struct ConstPoolEntry {
    /// The compiled unit the values belong to (address-keyed keepalive).
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
    pub sites: Rc<core::cell::RefCell<Vec<(u32, CmdSite)>>>,
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
    /// Allocation counter value (smallest free at startsearch time).
    pub id: i64,
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
/// appended.
#[derive(Clone)]
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
    /// Resolved key of the enclosing proc.
    pub key: String,
    /// The command's words, list-rendered (tclsh's substituted text).
    pub cmdtext: String,
    /// Ids of the caller's step records at enterstep time.
    pub snapshot: Vec<u64>,
}

/// Tcl interpreter.
pub struct Interp {
    /// Variables (global scope).
    pub(crate) globals: VarMap<Value>,
    /// Names in `globals` that are arrays (scalar/array distinction).
    pub(crate) array_globals: VarSet,
    /// Procedure call frames (empty at global level).  Boxed: a `CallFrame`
    /// is ~264 bytes and pool-pop/frame-push/frame-pop/pool-push moved the
    /// whole struct four times per call (a memcpy each — the fib profile's
    /// memmove bucket); boxing makes those moves 8-byte pointer copies while
    /// the box itself rides the pool.
    pub(crate) frames: Vec<Box<CallFrame>>,
    /// Recycled call frames: proc return pushes its (emptied) frame here
    /// and the next call pops it, keeping the locals-map allocation and
    /// its capacity across calls (tclsh keeps one frame arena per
    /// interpreter; recursion depth bounds the pool naturally).
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
    pub(crate) var_link_cache: VarMap<VarMap<(String, String)>>,
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
    pub(crate) bytecode_cache: HashMap<Rc<str>, Rc<rtcl_parser::ByteCode>>,
    /// Loop-body memo: recent body `Value`s handed to
    /// [`Interp::eval_body_value`] plus their compiled forms — a pointer
    /// compare replaces `eval`'s two full-text cache hashes on every hit
    /// (while/for/foreach bodies, `time` scripts).  A few slots, not one:
    /// `for` alternates body and next scripts every iteration, and one
    /// loop nested inside another needs both its own body and the outer
    /// body/next resident (tclsh instead compiles loop bodies inline into
    /// the surrounding bytecode and pays nothing per iteration).
    pub(crate) body_memo: Vec<(Value, Rc<rtcl_parser::ByteCode>)>,
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
    pub(crate) expr_check_cache: HashMap<String, Result<(), String>>,
    /// Call stack depth (for recursion limit).
    pub(crate) call_depth: usize,
    /// Maximum call depth.
    pub(crate) max_call_depth: usize,
    /// Last result.
    pub(crate) result: Value,
    /// Bytecode cache — keyed by script source.
    pub(crate) code_cache: HashMap<String, ByteCode>,
    /// Package registry: name → version string.
    #[cfg(feature = "package")]
    pub(crate) packages: HashMap<String, String>,
    /// Current namespace ("::") at the global level.  `Rc<str>` — a proc
    /// call swaps this to the definition namespace and back, and the swap
    /// is a refcount bump rather than a heap copy; same for the frame's
    /// `ns`/`call_ns` records.  Names built fresh (qualify, ns eval) move
    /// their String in via `Rc::from`, which reuses the allocation.
    pub(crate) current_namespace: Rc<str>,
    /// The root namespace as a shared `Rc` — the namespace of every
    /// global-scope proc, handed out per call without allocating.
    pub(crate) ns_root: Rc<str>,
    /// Known namespaces ("::" always present).
    pub(crate) namespaces: HashMap<String, commands::namespace::NamespaceInfo>,
    /// `namespace import` aliases: fully-qualified alias name → the fully
    /// qualified name of the original command.  Aliases stay separate from
    /// `procs` so the origin's *current* body always dispatches (tclsh
    /// semantics: redefining the source proc is visible through the alias).
    pub(crate) import_aliases: HashMap<String, String>,
    /// Per-namespace `namespace unknown` handler scripts (raw, as given).
    pub(crate) ns_unknown: HashMap<String, String>,
    /// Live `namespace ensemble`s keyed by registered command name.
    pub(crate) ensembles: HashMap<String, EnsembleDef>,
    /// Live array searches keyed by the owning scope's stamp key.
    pub(crate) array_searches: HashMap<String, ArraySearchList>,
    /// Mutation counters per array (element-set changes invalidate searches).
    pub(crate) array_stamps: HashMap<String, u64>,
    /// Container-generation counters per array, bumped on array-level
    /// create/destroy only (`array get` uses them to spot a callback that
    /// destroyed and recreated the array — snapshot elements then skip).
    pub(crate) array_generations: HashMap<String, u64>,
    /// Whole-variable traces keyed by the owning scope's stamp key.
    pub(crate) var_traces: HashMap<String, Vec<VarTrace>>,
    /// Element-specific traces: stamp key → element name → traces.
    pub(crate) elem_traces: HashMap<String, HashMap<String, Vec<VarTrace>>>,
    /// Elements that exist only to carry a trace (traced but never set).
    pub(crate) trace_phantoms: HashMap<String, std::collections::HashSet<String>>,
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
    /// > 0 while an execution-trace callback itself is running (its inner
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
    pub(crate) flat_aliases: Vec<(String, String)>,
    /// Eval-level aliases whose target was deleted (namespace removal):
    /// reads keep the redirect, writes error.
    pub(crate) dead_flat: Vec<String>,
    /// Flat keys of variables a `variable` command declared at eval level
    /// (tclsh: the ns-eval varFrame holds a reference, so a namespace that
    /// deletes ITSELF leaves the declared variable readable for the rest
    /// of the body — var-1.16/1.17's `set result` after the self-delete).
    pub(crate) ns_variable_links: Vec<String>,
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
    /// Newlines between the enclosing script's first line and the script
    /// currently being evaluated by a construct that re-evals a word of
    /// its own source (`while`/`for`/`foreach` bodies): a body command's
    /// parse-relative line + this offset is the line within the enclosing
    /// script, which is what `(procedure ... line N)` must report (tclsh
    /// keeps one absolute line table).  0 at script level.
    pub(crate) line_offset: usize,
    /// Line of the command currently dispatching (parse-relative).
    pub(crate) cur_cmd_line: usize,
    /// Raw source of each word of the command currently dispatching,
    /// aligned with its arguments — braced/quoted script words carry their
    /// delimiters, letting constructs recover their body's line offset.
    /// Spans resolve through [`Interp::cur_source`]; `Rc` (shared with the
    /// cached parse tree) so save/restore per dispatch is a refcount bump.
    pub(crate) cur_cmd_word_srcs: Rc<Vec<rtcl_parser::SrcSpan>>,
    /// Shared empty word-source vector — `exec_bytecode` swaps the
    /// per-command word sources out for this singleton on entry (one Rc
    /// bump; the previous code allocated a fresh `Rc<Vec<..>>` per proc
    /// call).
    pub(crate) word_srcs_nil: Rc<Vec<rtcl_parser::SrcSpan>>,
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
    /// Pending "the next `eval` is part of the current proc body unit"
    /// request (see [`Interp::lexical_body`]).
    pub(crate) next_eval_lexical: bool,
    /// Source of the outermost command whose word substitution errored,
    /// waiting to see if the error escapes uncaught — the top-level
    /// report then adds `invoked from within "<text>"` (tclsh logs the
    /// enclosing command only when nothing else consumed the log).
    pub(crate) err_pending_top: Option<Rc<str>>,
    /// Current script name (for info script).
    #[cfg(feature = "std")]
    pub(crate) script_name: String,
    /// Channel table (stdin/stdout/stderr + opened files/pipes).
    #[cfg(feature = "std")]
    pub channels: crate::channel::ChannelTable,
    /// 宿主控制台服务：无 `io` 特性构建里 `puts` 的 stdout/stderr 落点
    /// （原生 stdio，或 wasm 宿主注入的 JS 实现）。
    pub(crate) console: Box<dyn crate::host::HostConsole>,
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
    /// Child interpreter table for `interp` command.
    #[cfg(feature = "std")]
    pub(crate) child_interps: HashMap<String, Box<Interp>>,
    /// Next child interpreter ID counter.
    #[cfg(feature = "std")]
    pub(crate) next_interp_id: u64,
    /// One-shot variable-link list for the next `call_proc` frame: the OO
    /// method dispatcher hands its owner's `variable` declarations over
    /// and `call_proc` installs the links at frame setup — the same
    /// triple the `variable` command performs (alias entry, per-name slot
    /// aliasing, seed-if-exists), minus a dispatched command per variable
    /// per call.  Prefix errors are unreachable: declaration names are
    /// validated by the `variable` define word and object namespaces
    /// always exist.
    pub(crate) frame_prelink: Option<Vec<String>>,
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
            globals: VarMap::default(),
            array_globals: VarSet::default(),
            array_searches: HashMap::new(),
            array_stamps: HashMap::new(),
            array_generations: HashMap::new(),
            var_traces: HashMap::new(),
            elem_traces: HashMap::new(),
            trace_phantoms: HashMap::new(),
            cmd_traces: HashMap::new(),
            exec_traces: HashMap::new(),
            exec_trace_ids: 0,
            exec_step_stack: Vec::new(),
            exec_step_running: 0,
            tier1_epoch: 0,
            flat_aliases: Vec::new(),
            dead_flat: Vec::new(),
            ns_variable_links: Vec::new(),
            ns_eval_keep: Vec::new(),
            err_info: None,
            err_fresh: false,
            err_code_raised: false,
            err_line: 1,
            line_offset: 0,
            cur_cmd_line: 0,
            cur_cmd_word_srcs: Rc::new(Vec::new()),
            word_srcs_nil: Rc::new(Vec::new()),
            cur_cmd_text: rtcl_parser::SrcSpan { start: 0, end: 0 },
            cur_source: Rc::from(""),
            err_from_subst: false,
            lexical_body: false,
            next_eval_lexical: false,
            err_pending_top: None,
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
            bytecode_cache: HashMap::new(),
            body_memo: Vec::new(),
            lambda_code_cache: HashMap::new(),
            const_pool: HashMap::default(),
            in_locals_unit: false,
            expr_check_cache: HashMap::new(),
            call_depth: 0,
            max_call_depth: 1000,
            result: Value::empty(),
            code_cache: HashMap::new(),
            #[cfg(feature = "package")]
            packages: HashMap::new(),
            current_namespace: Rc::from("::"),
            ns_root: Rc::from("::"),
            namespaces: {
                let mut ns = HashMap::new();
                ns.insert("::".to_string(), commands::namespace::NamespaceInfo::default());
                ns
            },
            import_aliases: HashMap::new(),
            ns_unknown: HashMap::new(),
            ensembles: HashMap::new(),
            #[cfg(feature = "std")]
            script_name: String::new(),
            #[cfg(feature = "std")]
            channels: crate::channel::ChannelTable::new(),
            console: {
                #[cfg(feature = "std")]
                let c: Box<dyn crate::host::HostConsole> = Box::new(crate::host::NativeConsole);
                #[cfg(not(feature = "std"))]
                let c: Box<dyn crate::host::HostConsole> = Box::new(crate::host::NullConsole);
                c
            },
            aliases: HashMap::new(),
            references: HashMap::new(),
            next_ref_id: 0,
            saved_commands: HashMap::new(),
            tainted_vars: HashMap::new(),
            xtrace_callback: String::new(),
            #[cfg(feature = "std")]
            event_queue: Vec::new(),
            #[cfg(feature = "std")]
            next_event_id: 1,
            #[cfg(feature = "std")]
            child_interps: HashMap::new(),
            #[cfg(feature = "std")]
            next_interp_id: 1,
            frame_level0_args: None,
            frame_prelink: None,
            oo: commands::oo::OoState::default(),
            #[cfg(feature = "io")]
            reflected: HashMap::new(),
            #[cfg(feature = "io")]
            transforms: HashMap::new(),
        };
        commands::oo::init(&mut interp);
        interp.register_builtins();
        interp.init_special_vars();
        interp.load_stdlib();
        interp
    }

    /// Load the Tcl-level standard library (embedded at compile time).
    fn load_stdlib(&mut self) {
        const STDLIB_TCL: &str = include_str!("../stdlib.tcl");
        if let Err(e) = self.eval(STDLIB_TCL) {
            panic!("stdlib.tcl failed to load: {e}");
        }
    }

    /// 注入宿主控制台实现（wasm 宿主由此把 `puts` 的输出接到 JS 回调；
    /// 原生构建默认已是 [`crate::host::NativeConsole`]，无需调用）。
    pub fn set_console(&mut self, console: Box<dyn crate::host::HostConsole>) {
        self.console = console;
    }

    /// Populate special global variables (`$env`, `$tcl_platform`, etc.).
    fn init_special_vars(&mut self) {
        // --- $env array (mirror OS environment) ---
        #[cfg(feature = "env")]
        for (key, val) in std::env::vars() {
            let full = format!("env({})", key);
            self.globals.insert(full, Value::from_str(&val));
        }
        #[cfg(feature = "env")]
        if !self.globals.contains_key("env") {
            self.globals.insert("env".to_string(), Value::empty());
        }

        // --- $tcl_platform array ---
        self.globals.insert("tcl_platform(engine)".to_string(), Value::from_str("rtcl"));
        self.globals.insert(
            "tcl_platform(os)".to_string(),
            Value::from_str(std::env::consts::OS),
        );
        self.globals.insert(
            "tcl_platform(platform)".to_string(),
            Value::from_str(if cfg!(unix) { "unix" } else if cfg!(windows) { "windows" } else { "unknown" }),
        );
        self.globals.insert(
            "tcl_platform(machine)".to_string(),
            Value::from_str(std::env::consts::ARCH),
        );
        self.globals.insert(
            "tcl_platform(osVersion)".to_string(),
            Value::from_str(""),
        );
        self.globals.insert(
            "tcl_platform(byteOrder)".to_string(),
            Value::from_str(if cfg!(target_endian = "little") { "littleEndian" } else { "bigEndian" }),
        );
        self.globals.insert(
            "tcl_platform(wordSize)".to_string(),
            Value::from_int(std::mem::size_of::<usize>() as i64),
        );
        self.globals.insert(
            "tcl_platform(pointerSize)".to_string(),
            Value::from_int(std::mem::size_of::<*const ()>() as i64),
        );
        self.globals.insert("tcl_platform".to_string(), Value::empty());
        self.array_globals.insert("tcl_platform".to_string());
        self.array_globals.insert("env".to_string());

        // --- $argv0, $argv, $argc (empty defaults — cli layer overrides) ---
        self.globals.insert("argv0".to_string(), Value::from_str(""));
        self.globals.insert("argv".to_string(), Value::from_str(""));
        self.globals.insert("argc".to_string(), Value::from_int(0));

        // --- $tcl_interactive ---
        self.globals.insert("tcl_interactive".to_string(), Value::from_int(0));

        // --- $errorInfo (empty until an error occurs); $errorCode does
        // not exist until an error sets it (tclsh: info exists → 0) ---
        self.globals.insert("errorInfo".to_string(), Value::from_str(""));

        // --- $auto_path (empty list — package system can populate later) ---
        self.globals.insert("auto_path".to_string(), Value::from_str(""));

        // --- $tcl_version, $tcl_patchLevel ---
        self.globals.insert("tcl_version".to_string(), Value::from_str("8.6"));
        self.globals.insert("tcl_patchLevel".to_string(), Value::from_str("8.6.0-rtcl"));
        self.globals.insert("tcl_library".to_string(), Value::from_str("/usr/share/tcltk/tcl8.6"));
    }

    /// Whether we are inside a procedure scope.
    #[allow(dead_code)]
    pub(crate) fn in_proc(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Reference to variable storage for the current scope (for iteration).
    /// Does NOT follow upvar links.
    pub(crate) fn scope_vars(&self) -> &VarMap<Value> {
        if let Some(frame) = self.frames.last() {
            &frame.locals
        } else {
            &self.globals
        }
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
        if self.const_pool.len() >= CONST_POOL_MAX {
            self.const_pool.clear();
        }
        let values: Rc<[Value]> = code
            .constants()
            .iter()
            .map(|s| Value::from_str(s))
            .collect();
        self.const_pool.insert(
            Rc::as_ptr(code) as usize,
            ConstPoolEntry {
                code: Rc::clone(code),
                values,
                sites: Rc::default(),
            },
        );
    }

    /// Pool probe for a unit being executed (one hash per `exec_bytecode`
    /// call, amortised over every PushConst and call-site token it uses):
    /// the literal slice plus the call-site token table.
    pub(crate) fn const_pool_get(
        &self,
        code: &ByteCode,
    ) -> Option<(
        Rc<[Value]>,
        Rc<core::cell::RefCell<Vec<(u32, CmdSite)>>>,
    )> {
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
    fn test_stdlib_throw_ok() {
        let mut interp = Interp::new();
        let result = interp.eval("throw ok hello").unwrap();
        assert_eq!(result.as_str(), "hello");
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
    fn test_stdlib_dict_getdef() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set d [dict create a 1 b 2]
            dict getdef $d c 99
        "#).unwrap();
        assert_eq!(result.as_str(), "99");
    }

    #[test]
    fn test_stdlib_dict_getdef_exists() {
        let mut interp = Interp::new();
        let result = interp.eval(r#"
            set d [dict create a 1 b 2]
            dict getdef $d a 99
        "#).unwrap();
        assert_eq!(result.as_str(), "1");
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
    fn test_stdlib_fileevent_shim() {
        let mut interp = Interp::new();
        // fileevent is a shim that just tailcalls its args
        // We just verify it doesn't error when called with a known command
        let result = interp.eval("fileevent set x 42").unwrap();
        assert_eq!(result.as_str(), "42");
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
