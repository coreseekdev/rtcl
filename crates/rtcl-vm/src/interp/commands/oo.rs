//! TclOO — the object system (`oo::object`, `oo::class`, `oo::define`,
//! `oo::objdefine`, `oo::copy`, `info object`, `info class`).
//!
//! Semantics pinned against tclsh 8.6.17 (see `judge/probes/oo_probe*.tcl`):
//!
//! * Objects are commands backed by a per-object namespace `::oo::ObjN`
//!   (auto-generated for BOTH `create` and `new` — `oo::object create
//!   ::one::two::three` still gets `::oo::ObjN` as its namespace).
//! * Method resolution order: the object's own methods, then the object's
//!   mixins (C3), then the class chain (C3) with each chain class's mixins
//!   ahead of the class itself.  A class-level mixin beats a superclass.
//! * Only methods whose name starts with a lowercase letter are exported;
//!   `unknown method "x": must be ...` lists the *exported* chain methods
//!   plus `destroy` (sorted, `a, b or c`), with `new` suppressed on
//!   `::oo::class` itself.
//! * Method bodies run through the ordinary proc machinery with a synthetic
//!   first parameter carrying the method name, so `wrong # args: should be
//!   "inst1 m a b"` and `info level 0` = `inst1 m x` both come out right;
//!   the frame namespace is the object's namespace.
//! * `variable` declarations link by *defining* owner only (an inherited
//!   class's variables are not visible in a subclass method).
//! * Definition scripts (`oo::define C { ... }`) evaluate in the
//!   `::oo::define` namespace; errors gain an
//!   `(in definition script for class "::C" line N)` frame.  The multi-word
//!   form (`oo::define C method nm {a} {b}`) dispatches the definition word
//!   directly and adds no such frame.
//! * Destroying a class destroys every object whose chain contains it plus
//!   every class (subclass or class-mixin user) whose chain contains it.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::commands::misc;
use crate::interp::commands::namespace::{normalise, parent_of, qualify};
use crate::interp::{Interp, ProcDef, Rc};
use crate::value::Value;
use core::cell::RefCell;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;
type FxMapK<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<crate::interp::varmap::VarHasher>>;
#[cfg(feature = "embedded")]
type FxMapK<K, V> = alloc::collections::BTreeMap<K, V>;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

#[cfg(not(feature = "embedded"))]
use std::collections::HashSet;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeSet as HashSet;

// ── data model ─────────────────────────────────────────────────────────

/// How a defined method executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MethodKind {
    /// Ordinary Tcl body.
    Tcl,
    /// `forward name cmd ?arg...?` — dispatches `cmd prefix ?callargs...?`.
    Forward { cmd: String, prefix: Vec<String> },
    /// Core behaviour reached through the chain (`create`, `new`, `destroy`).
    Builtin(&'static str),
}

/// 方法 proc 体 memo 的一条构建：`(owner vars, def namespace, method
/// name, 定义, prelink entries)`。
///
/// The namespace is part of the identity: one `MethodDef` serves every
/// instance of a class, and each instance's `variable` links resolve in
/// its OWN namespace (`::oo::ObjN`) — the shared class `variables` list
/// alone would let one instance's prelink resolution leak to another.
/// The method NAME rides the key too: `renamemethod` moves the same
/// `MethodDef` under a new name, and the arity text's `params[0]` must
/// name the invoked method.
#[derive(Debug, Clone)]
pub(crate) struct ProcMemoEntry {
    vars: Option<Rc<Vec<String>>>,
    ns: Rc<str>,
    method: Rc<str>,
    def: Rc<ProcDef>,
    links: Rc<Vec<crate::interp::PrelinkEntry>>,
}

/// Capped evict-oldest: alternating instances of one class (and a
/// `renamemethod`) each keep their own build instead of ping-ponging a
/// single slot — a miss paid a full ProcDef rebuild + recompile.
type ProcMemo = Vec<ProcMemoEntry>;
const PROC_MEMO_MAX: usize = 4;

/// The [`MethodDef::head_cache`] cell: (typed name's `Value`, method
/// name's `Rc`, the two head Values [typed, method]).
type HeadCache = (Value, Rc<str>, Value, Value);

/// A method definition (own methods, forwards, constructors, destructors).
#[derive(Debug, Clone)]
pub(crate) struct MethodDef {
    /// Shared (immutable once defined): chain building / `chain.to_vec()`
    /// / constructor clones bump references instead of copying every
    /// parameter name and the whole body string per method call.
    pub params: Rc<[(String, Option<String>)]>,
    pub body: Rc<str>,
    pub exported: bool,
    pub kind: MethodKind,
    /// Compiled form of the assembled method [`ProcDef`], memoised (see
    /// `memoised_proc_def`): entries keyed `(owner `variable` snapshot,
    /// instance ns, method name)`.  `Rc<RefCell<..>>` so chain
    /// clones share one cell; a redefinition installs a fresh `MethodDef`
    /// (fresh cell), retiring the memo with its inputs.  The vars snapshot
    /// is the `link_prefix` input — a `variable` declaration change
    /// rebuilds (the `define` word also bumps `mutation_ctr`, retiring the
    /// chain memo with it).  The link list rides the same key: a hit hands
    /// the frame-prelink list back as an `Rc`, so a method call allocates
    /// nothing for its `variable` links.
    pub proc_memo: Rc<RefCell<ProcMemo>>,
    /// Cached argument head for the invoked method: (the typed name's
    /// `Value`, the method name's `Rc`, the two head Values [typed,
    /// method]).  The typed name varies per CALL SITE (`c` vs `::c`) and
    /// the method name varies per `renamemethod` (the moved `MethodDef`
    /// keeps this cell); the hit test is `Value::same_allocation` plus
    /// `Rc::ptr_eq` on the method — pointer compares, since values are
    /// immutable and the cache keeps its own handles alive (no string
    /// compare, no `to_string` on the miss path).  Cold on misses.
    pub head_cache: RefCell<Option<HeadCache>>,
}

/// A fresh [`MethodDef::proc_memo`] cell.
fn fresh_memo() -> Rc<RefCell<ProcMemo>> {
    Rc::new(RefCell::new(Vec::new()))
}

/// Whose definition a chain entry comes from.  `variable` linking and
/// `self class` follow this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The object's own per-object table.
    Object,
    /// A class in the resolution order (its `variable`s, not its
    /// superclass's, are linked).
    Class(String),
}

/// D-43（errorInfo 族）：OO 方法/构造器/析构器帧的错误出口帧——
/// tclsh 形状 `(class "::K" method "m" line N)` / `(class "::A"
/// destructor line N)`（method 带引号，destructor/constructor 不带）。
/// 类 = 当前 ActiveMethod 链条目的 owner（定义类）。非 OO 帧（或
/// 对象自有方法）返回 None 回落 procedure 帧。
pub(crate) fn oo_method_exit_tag(interp: &Interp) -> Option<String> {
    if !oo_method_frame(interp) {
        return None;
    }
    let a = interp.oo.active.last()?;
    match &a.chain.get(a.index)?.owner {
        Owner::Class(c) => Some(match a.method.as_ref() {
            "destructor" => format!("class \"{}\" destructor", c),
            "constructor" => format!("class \"{}\" constructor", c),
            m => format!("class \"{}\" method \"{}\"", c, m),
        }),
        Owner::Object => None,
    }
}

/// 当前出口帧是否为 OO 方法实现帧（类或对象自有方法，含 `next` 目标
/// 帧）。CALL 条目抑制与出口帧 tag 共用同一判据：调用帧仍在栈上
/// （len = depth + 1，与 cleanup_active 的 depth >= len 弹出判据互补）。
pub(crate) fn oo_method_frame(interp: &Interp) -> bool {
    interp
        .oo
        .active
        .last()
        .map(|a| a.frame_depth + 1 == interp.frames.len())
        .unwrap_or(false)
}

/// One resolvable link in a method chain.
#[derive(Debug, Clone)]
pub(crate) struct ChainEntry {
    pub owner: Owner,
    pub def: MethodDef,
    /// The owner's `variable` list (identity) as of chain construction,
    /// captured while the table entry was already at hand.  Sound to reuse
    /// while `mutation_ctr` is unchanged: the field's only writer
    /// (`oo::define variable`) swaps the Rc and bumps the counter, and
    /// chains handed out are stamped by it.  `None` (chains not from the
    /// stamp-checked memo) falls back to the per-call probe.
    pub vars: Option<Rc<Vec<String>>>,
}

/// 链解析 memo：类键 → (成员/父类键 → (stamp, 条目))。
type ChainMemo = FxMapK<Rc<str>, (u64, Rc<Vec<ChainEntry>>)>;

/// 对象/类键解析 memo：ns 键 → (typed 名 → (stamp, 解析结果))。
type KeyMemo = FxMapK<Rc<str>, (u64, Option<Rc<str>>)>;

/// A plain object.
#[derive(Debug, Default, Clone)]
pub(crate) struct OoObject {
    /// Per-object namespace (`::oo::ObjN`).  `Rc<str>`: the dispatcher
    /// hands it to `call_proc` (and `self namespace`) on every call.
    pub ns: Rc<str>,
    /// Canonical class key.
    pub class: String,
    /// Per-object mixins (canonical keys).
    pub mixins: Vec<String>,
    /// Per-object methods in definition order.
    pub methods: Vec<(String, MethodDef)>,
    /// Object-level filters（oo::objdefine o {filter ...}，D-41②——
    /// 原静默吞掉）。
    pub filters: Vec<String>,
    /// Object-level hidden（oo::objdefine o {unexport m}——隐藏继承
    /// 方法，D-41⑤；原 no-op）。
    pub hidden: Vec<String>,
    /// `variable` declarations (definition order).  `Rc`: the method-call
    /// fast path memoises on this list's identity (a declaration replaces
    /// the whole vector, so a pointer compare detects it).
    pub variables: Rc<Vec<String>>,
}

/// A class (both the behaviour of its instances and, via `obj_*`, the
/// class-as-object's own per-object definitions).
#[derive(Debug, Default, Clone)]
pub(crate) struct OoClass {
    pub ns: Rc<str>,
    pub superclasses: Vec<String>,
    /// Class-level mixins.
    pub mixins: Vec<String>,
    /// Methods of *instances* of this class.
    pub methods: Vec<(String, MethodDef)>,
    pub variables: Rc<Vec<String>>,
    /// Names hidden by `unexport` of a method this class does not itself
    /// define (hides the inherited implementation).
    pub hidden: Vec<String>,
    pub constructor: Option<MethodDef>,
    pub destructor: Option<MethodDef>,
    /// `filter` names (stored; not executed).
    pub filters: Vec<String>,
    // ── the class object's own per-object definitions (oo::objdefine) ──
    pub obj_methods: Vec<(String, MethodDef)>,
    pub obj_variables: Rc<Vec<String>>,
    pub obj_mixins: Vec<String>,
}

/// A method execution in progress (`self`/`my`/`next` context).  The
/// chain is an `Rc` snapshot shared with the memo it came from: `next`
/// follows the chain as it was at INVOCATION time even across later
/// redefinitions (the old `to_vec` copy's semantics, without the copy).
#[derive(Debug, Clone)]
pub(crate) struct ActiveMethod {
    /// `Rc<str>`: the hot dispatcher shares the resolved key and the
    /// chain-memo's method key instead of allocating two Strings per
    /// method call (profile: ~1.5% malloc + ~1.5% free under
    /// `invoke_method`/`exec_chain_entry`, plus their drops).
    pub obj: Rc<str>,
    pub method: Rc<str>,
    pub chain: Rc<Vec<ChainEntry>>,
    pub index: usize,
    /// `frames.len()` before the method's frame was pushed; the entry is
    /// live while `frames.len() > frame_depth`.
    pub frame_depth: usize,
}

/// Target of an open definition script / definition word.
#[derive(Debug, Clone)]
pub(crate) enum DefineTarget {
    /// `oo::define C` — the definition words describe C's instances.
    Class(String),
    /// `oo::objdefine X` — the words describe X as an object.
    Object(String),
}

/// Dispatcher-cache entry for the object dispatcher and `my` (see their
/// fast paths).  Validation is identity-and-stamp: the typed word and the
/// method word must be the *same allocations* (a const-pool literal, a
/// slot value re-cloned, or the `my` word — all stable across calls), the
/// resolution namespace the same `Rc`, and the mutation stamp live.
/// Those together imply `resolve_object_key`/`qualify` would recompute
/// the identical key and `chain_for` would hit its memo — so the hash
/// probes are skipped and the cached chain feeds `exec_chain_entry`.
/// Each dispatcher holds a two-entry set (two call sites, or two method
/// names alternating — the one-entry form thrashed, Round AB).
#[derive(Debug)]
pub(crate) struct OoCallCache {
    pub ns: Rc<str>,
    pub stamp: u64,
    pub typed: Value,
    pub method: Value,
    pub key: Rc<str>,
    /// The object's namespace, captured at fill time — the `my` slot's
    /// `frame_in_object_ns` guard needs it without re-probing the tables.
    pub obj_ns: Option<Rc<str>>,
    pub method_rc: Rc<str>,
    pub chain: Rc<Vec<ChainEntry>>,
}

/// All OO state, hanging off [`Interp`].
#[derive(Debug, Default)]
pub(crate) struct OoState {
    pub next_obj_id: usize,
    // Fx-hashed: these are probed by canonical object/class key on every
    // OO dispatch (`resolve_object_key`, `owner_variables`,
    // `object_ns_of`); std's SipHash was ~5% of the oo_bench2_vars
    // profile in `find<(String, OoClass)>`.  Never iterated for output
    // (`creation_order` drives listings), so the hash choice is
    // order-invisible.
    pub objects: FxMapK<String, OoObject>,
    pub classes: FxMapK<String, OoClass>,
    /// Objects and classes in creation order (listing/cascade order).
    pub creation_order: Vec<String>,
    pub active: Vec<ActiveMethod>,
    /// Pooled argument vectors for method invocation: taken at dispatch,
    /// returned after the call — steady-state method calls allocate
    /// nothing for the argument vector.
    pub args_pool: Vec<Vec<Value>>,
    pub define_stack: Vec<DefineTarget>,
    /// `my`/`self`/`next` command keys → owning object key.  Entries whose
    /// key no longer holds such a command were renamed away and are swept
    /// when their owner is destroyed (oo-1.20).
    pub my_commands: FxMapK<String, String>,
    /// Structural mutation stamp: bumped at EVERY mutation of the class /
    /// object tables (create, destroy, every define word — see the bump
    /// sites in this module).  [`OoState::chain_memo`] entries validate
    /// against it, the same pattern as `Interp::cmd_generation` ageing the
    /// resolution cache.  Every write to `oo.classes`/`oo.objects`
    /// structure lives in this file; a missed bump site would serve a
    /// stale chain, so the grep audit is `classes.(insert|remove|get_mut)`
    /// + `objects.(insert|remove|get_mut)`.
    pub mutation_ctr: u64,
    /// Resolution-chain memo (tclsh caches the call context the same way):
    /// object/class key → method → the chain as built, valid under the
    /// `mutation_ctr` stamp it was built at (nested maps so a hit probes
    /// by `&str` twice and allocates nothing — the flat
    /// `(String, String)` key allocated two Strings per probe, visible in
    /// the OO call profile).  `chain_memo` serves plain dispatch,
    /// `chain_memo_priv` the `include_private` (`my`) one.  A mutation
    /// anywhere retires every entry at once — conservative but exactly
    /// tclsh's re-resolution semantics.  Bounded with clear-on-overflow
    /// (wasm32 is a target).  The `Rc` chain is shared with the
    /// `ActiveMethod` snapshots, so `next` also stops paying a
    /// `to_vec` per invocation.
    pub chain_memo: FxMapK<Rc<str>, ChainMemo>,
    /// The `include_private` half of [`Self::chain_memo`].
    pub chain_memo_priv: FxMapK<Rc<str>, ChainMemo>,
    /// The object dispatcher's cache entry — a single slot: its hot loops
    /// repeat one method, and a second probe costs a few instructions per
    /// call (measured).  See [`OoCallCache`].
    pub obj_cache: Option<OoCallCache>,
    /// The `my` dispatcher's two-entry set: method bodies alternate
    /// `my a` / `my b` recursion, which a single slot thrashed (Round AB).
    pub my_cache: [Option<OoCallCache>; 2],
    /// Alternating fill cursor for [`Self::my_cache`].
    pub my_cache_next: u8,
    /// Typed-name → canonical object/class key memo (see
    /// `resolve_object_key`): current namespace → typed name →
    /// (stamp, resolution).  `RefCell` so `&Interp` read paths can fill
    /// it, the `proc_memo` precedent.
    pub key_memo: RefCell<FxMapK<Rc<str>, KeyMemo>>,
}

// ── init ───────────────────────────────────────────────────────────────

fn builtin_method(name: &'static str) -> MethodDef {
    MethodDef { params: Vec::new().into(), body: Rc::from(""), exported: true, kind: MethodKind::Builtin(name), proc_memo: fresh_memo(), head_cache: RefCell::new(None) }
}

fn register_object_command(interp: &mut Interp, key: &str, func: crate::command::CommandFunc) {
    interp.commands.insert(key.to_string(), func);
    interp
        .command_categories
        .insert(key.to_string(), crate::command::CommandCategory::Extension);
    interp.command_meta.insert(
        key.to_string(),
        crate::command::CommandMeta { usage: "methodName ?arg ...?", help: "Object command" },
    );
    interp.note_cmd_mutation();
}

fn register_oo_command(interp: &mut Interp, key: &str, usage: &'static str, func: crate::command::CommandFunc) {
    interp.commands.insert(key.to_string(), func);
    interp
        .command_categories
        .insert(key.to_string(), crate::command::CommandCategory::Extension);
    interp.command_meta.insert(
        key.to_string(),
        crate::command::CommandMeta { usage, help: "TclOO command" },
    );
    interp.note_cmd_mutation();
}

/// The definition words, registered as procs in `::oo::define` so that a
/// definition script's `variable` shadows the builtin `variable` command
/// (dispatch tries procs before builtins).
const DEF_WORDS: &[&str] = &[
    "method", "forward", "mixin", "superclass", "constructor", "destructor",
    "export", "unexport", "variable", "filter", "deletemethod", "renamemethod", "self",
];

/// Create the core classes and commands.  Called once from `Interp::new`.
pub(crate) fn init(interp: &mut Interp) {
    ensure_ns(&mut interp.namespaces, "::oo");
    ensure_ns(&mut interp.namespaces, "::oo::define");

    // ::oo::object — the root class.  `destroy` is its own (core) method so
    // that `info class methods oo::object` shows it and every instance's
    // must-be list ends with `destroy`.
    let id = interp.oo.next_obj_id + 1;
    interp.oo.next_obj_id = id;
    let mut root = OoClass { ns: Rc::from(format!("::oo::Obj{}", id)), ..Default::default() };
    root.methods.push(("destroy".to_string(), builtin_method("destroy")));
    // D-43⑤：oo::object 私有核心方法（tclsh must-be 清单含它们）。
    // D-43⑤：这四个是**私有**核心方法（tclsh：外部 o varname 报
    // unknown；must-be 清单仅在 my 私有调用路径列出）。
    for core in ["variable", "varname", "eval", "<cloned>"] {
        let mut md = builtin_method(core);
        md.exported = false;
        root.methods.push((core.to_string(), md));
    }
    interp.oo.classes.insert("::oo::object".to_string(), root);
    interp.oo.creation_order.push("::oo::object".to_string());

    // ::oo::class — the metaclass.
    let id = interp.oo.next_obj_id + 1;
    interp.oo.next_obj_id = id;
    let mut metaclass =
        OoClass { ns: Rc::from(format!("::oo::Obj{}", id)), superclasses: vec!["::oo::object".to_string()], ..Default::default() };
    metaclass.methods.push(("create".to_string(), builtin_method("create")));
    metaclass.methods.push(("new".to_string(), builtin_method("new")));
    interp.oo.classes.insert("::oo::class".to_string(), metaclass);
    interp.oo.creation_order.push("::oo::class".to_string());

    register_object_command(interp, "::oo::object", cmd_oo_object);
    register_object_command(interp, "::oo::class", cmd_oo_object);
    register_oo_command(interp, "::oo::copy", "sourceName ?targetName? ?targetNamespace?", cmd_oo_copy);
    register_oo_command(interp, "::oo::define", "className arg ?arg ...?", cmd_oo_define);
    register_oo_command(interp, "::oo::objdefine", "objectName arg ?arg ...?", cmd_oo_objdefine);
    register_oo_command(interp, "::tcl::oo::def", "word ?arg ...?", cmd_oo_def);

    // Definition words as procs in ::oo::define.
    for word in DEF_WORDS {
        let key = format!("::oo::define::{}", word);
        let body = format!("::tcl::oo::def {} {{*}}$args", word);
        interp.procs.insert(
            key,
            super::super::Rc::new(ProcDef {
                params: super::super::Rc::new(vec![("args".to_string(), None)]),
                body: super::super::Rc::from(body),
                statics: super::super::Rc::new(HashMap::new()), compiled: None,
            }),
        );
    }
    interp.note_cmd_mutation();

    // tclsh 的 ObjN 计数器在用户代码前已消耗到 10（核心内部对象
    // Obj3..Obj10；可观察的核心名仍是 oo::object=Obj1 / oo::class=
    // Obj2），首个用户对象是 ::oo::Obj11（实测 tclsh 8.6.17）。基线
    // 对齐让对象名、info object namespace、errorstack 文本逐字一致。
    interp.oo.next_obj_id = 10;
}

// ── namespace helpers ──────────────────────────────────────────────────

/// Ensure a namespace and its ancestors exist (local copy of the namespace
/// module's `ensure_namespace`, which is private).
fn ensure_ns(namespaces: &mut super::super::varmap::FxHashedMap<String, super::namespace::NamespaceInfo>, qualified: &str) {
    namespaces.entry("::".to_string()).or_default();
    if qualified == "::" {
        return;
    }
    let parts: Vec<&str> = qualified.split("::").filter(|s| !s.is_empty()).collect();
    let mut path = String::new();
    for part in parts {
        if path.is_empty() {
            path = format!("::{}", part);
        } else {
            path = format!("{}::{}", path, part);
        }
        namespaces.entry(path.clone()).or_default();
    }
}

/// Canonical candidates for an as-typed object name.
fn object_key_candidates(interp: &Interp, typed: &str) -> Vec<String> {
    if typed.starts_with("::") {
        vec![normalise(typed)]
    } else {
        vec![qualify(&interp.current_namespace, typed), format!("::{}", typed)]
    }
}

/// Resolve an as-typed name to an object/class key.
fn resolve_object_key(interp: &Interp, typed: &str) -> Option<Rc<str>> {
    // The resolution is a pure function of (current namespace, typed
    // name, the object/class tables) — and every table mutation bumps
    // `mutation_ctr` (the chain_memo audit), so entries validate against
    // the stamp they resolved under.  The dispatcher runs this per method
    // call; the naive form generated candidate Strings (`qualify` +
    // `format!("::{}")`) and probed the tables twice each time — ~13% of
    // the OO call bench.  Unknown names memoize too (the error path then
    // skips the walk).  Bounded: a new outer entry clears the table at
    // 64 namespaces, an inner one at 256 names.
    {
        let memo = interp.oo.key_memo.borrow();
        if let Some(inner) = memo.get(interp.current_namespace.as_ref()) {
            if let Some((stamp, k)) = inner.get(typed) {
                if *stamp == interp.oo.mutation_ctr {
                    return k.clone();
                }
            }
        }
    }
    let resolved: Option<Rc<str>> = (|| {
        for cand in object_key_candidates(interp, typed) {
            if interp.oo.objects.contains_key(&cand) || interp.oo.classes.contains_key(&cand) {
                return Some(Rc::from(cand.as_str()));
            }
        }
        None
    })();
    {
        let mut memo = interp.oo.key_memo.borrow_mut();
        const KEY_MEMO_NS_MAX: usize = 64;
        const KEY_MEMO_NAME_MAX: usize = 256;
        if memo.len() >= KEY_MEMO_NS_MAX && !memo.contains_key(interp.current_namespace.as_ref())
        {
            memo.clear();
        }
        let inner = memo
            .entry(Rc::from(interp.current_namespace.as_ref()))
            .or_default();
        if inner.len() >= KEY_MEMO_NAME_MAX {
            inner.clear();
        }
        inner.insert(Rc::from(typed), (interp.oo.mutation_ctr, resolved.clone()));
    }
    resolved
}

/// Does a command (builtin, proc, ensemble or alias) exist under this
/// canonical key?
fn command_exists_any(interp: &Interp, canonical: &str) -> bool {
    if interp.procs.contains_key(canonical)
        || interp.commands.contains_key(canonical)
        || interp.ensembles.contains_key(canonical)
        || interp.import_aliases.contains_key(canonical)
        || interp.aliases.contains_key(canonical)
    {
        return true;
    }
    let bare = canonical.trim_start_matches(':');
    if !bare.contains("::") && bare != canonical {
        return interp.commands.contains_key(bare) || interp.procs.contains_key(bare);
    }
    false
}

// ── C3 linearisation ───────────────────────────────────────────────────

fn linearize(interp: &Interp, class: &str) -> Vec<String> {
    let mut cache: HashMap<String, Vec<String>> = HashMap::new();
    lin_rec(interp, class, &mut cache, 0)
}

fn lin_rec(
    interp: &Interp,
    class: &str,
    cache: &mut HashMap<String, Vec<String>>,
    depth: usize,
) -> Vec<String> {
    if depth > 64 {
        return vec![class.to_string()];
    }
    if let Some(cached) = cache.get(class) {
        return cached.clone();
    }
    // Superclasses only: TclOO prepends each class's mixins per class in
    // build_chain; folding mixins into the C3 merge can deadlock it.
    let parents: Vec<String> = match interp.oo.classes.get(class) {
        Some(cls) => cls.superclasses.clone(),
        None => vec![],
    };
    let mut lists: Vec<Vec<String>> =
        parents.iter().map(|p| lin_rec(interp, p, cache, depth + 1)).collect();
    lists.push(parents.clone());

    let mut out = vec![class.to_string()];
    loop {
        // Pick the first head that appears in no tail.
        let mut picked: Option<String> = None;
        'outer: for list in &lists {
            if let Some(head) = list.first() {
                let in_tail = lists.iter().any(|l| l.iter().skip(1).any(|x| x == head));
                if !in_tail {
                    picked = Some(head.clone());
                    break 'outer;
                }
            }
        }
        match picked {
            Some(p) => {
                for list in lists.iter_mut() {
                    list.retain(|x| *x != p);
                }
                out.push(p);
            }
            None => break,
        }
    }
    cache.insert(class.to_string(), out.clone());
    out
}

// ── method chains ──────────────────────────────────────────────────────

/// Push one class's method-table match, honouring the `hidden` set.
fn push_class_entry(
    interp: &Interp,
    class: &str,
    method: &str,
    include_private: bool,
    hidden: &mut HashSet<String>,
    out: &mut Vec<ChainEntry>,
) {
    let cls = match interp.oo.classes.get(class) {
        Some(c) => c,
        None => return,
    };
    if cls.hidden.iter().any(|h| h == method) {
        hidden.insert(method.to_string());
    }
    if hidden.contains(method) {
        return;
    }
    if let Some((_, def)) = cls.methods.iter().find(|(n, _)| n == method) {
        if (def.exported || include_private)
            && !out.iter().any(|e| e.owner == Owner::Class(class.to_string()))
        {
            out.push(ChainEntry {
                vars: Some(Rc::clone(&cls.variables)),
                owner: Owner::Class(class.to_string()),
                def: def.clone(),
            });
            hidden.remove(method);
        }
    }
}

/// Build the resolution chain for `method` on object/class `key`.
/// First match wins; the whole chain is kept for `next`.
fn build_chain(interp: &Interp, key: &str, method: &str, include_private: bool) -> Vec<ChainEntry> {
    let mut out: Vec<ChainEntry> = Vec::new();
    let mut hidden: HashSet<String> = HashSet::new();

    if let Some(obj) = interp.oo.objects.get(key) {
        if let Some((_, def)) = obj.methods.iter().find(|(n, _)| n == method) {
            if def.exported || include_private {
                out.push(ChainEntry {
                    vars: Some(Rc::clone(&obj.variables)),
                    owner: Owner::Object,
                    def: def.clone(),
                });
            }
        }
        for m in &obj.mixins {
            for c in linearize(interp, m) {
                push_class_entry(interp, &c, method, include_private, &mut hidden, &mut out);
            }
        }
        for c in linearize(interp, &obj.class) {
            let mixins = interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default();
            for mx in &mixins {
                for c2 in linearize(interp, mx) {
                    push_class_entry(interp, &c2, method, include_private, &mut hidden, &mut out);
                }
            }
            push_class_entry(interp, &c, method, include_private, &mut hidden, &mut out);
        }
        return out;
    }

    if let Some(cls) = interp.oo.classes.get(key) {
        // The class as an object: its own per-object table, per-object
        // mixins, then the metaclass chain.
        if let Some((_, def)) = cls.obj_methods.iter().find(|(n, _)| n == method) {
            if def.exported || include_private {
                out.push(ChainEntry {
                    vars: Some(Rc::clone(&cls.obj_variables)),
                    owner: Owner::Object,
                    def: def.clone(),
                });
            }
        }
        for m in &cls.obj_mixins {
            for c in linearize(interp, m) {
                push_class_entry(interp, &c, method, include_private, &mut hidden, &mut out);
            }
        }
        for c in linearize(interp, "::oo::class") {
            let mixins = interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default();
            for mx in &mixins {
                for c2 in linearize(interp, mx) {
                    push_class_entry(interp, &c2, method, include_private, &mut hidden, &mut out);
                }
            }
            push_class_entry(interp, &c, method, include_private, &mut hidden, &mut out);
        }
    }
    out
}

/// All (name, entry) pairs reachable from `key`, in resolution order.
fn full_walk(interp: &Interp, key: &str) -> Vec<(String, ChainEntry)> {
    let mut out: Vec<(String, ChainEntry)> = Vec::new();
    let mut hidden: HashSet<String> = HashSet::new();

    macro_rules! take_class {
        ($class:expr) => {
            if let Some(cls) = interp.oo.classes.get(&$class) {
                for h in &cls.hidden {
                    if !out.iter().any(|(n, _)| n == h) {
                        hidden.insert(h.clone());
                    }
                }
                for (name, def) in &cls.methods {
                    if !out.iter().any(|(n, _)| n == name) && !hidden.contains(name) {
                        out.push((name.clone(), ChainEntry {
                            vars: Some(Rc::clone(&cls.variables)),
                            owner: Owner::Class($class.clone()),
                            def: def.clone(),
                        }));
                        hidden.remove(name);
                    }
                }
            }
        };
    }

    if let Some(obj) = interp.oo.objects.get(key) {
        for (name, def) in &obj.methods {
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.clone(), ChainEntry {
                    vars: Some(Rc::clone(&obj.variables)),
                    owner: Owner::Object,
                    def: def.clone(),
                }));
            }
        }
        for m in &obj.mixins {
            for c in linearize(interp, m) {
                take_class!(c);
            }
        }
        for c in linearize(interp, &obj.class) {
            let mixins = interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default();
            for mx in &mixins {
                for c2 in linearize(interp, mx) {
                    take_class!(c2);
                }
            }
            take_class!(c);
        }
        return out;
    }

    if let Some(cls) = interp.oo.classes.get(key) {
        for (name, def) in &cls.obj_methods {
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.clone(), ChainEntry {
                    vars: Some(Rc::clone(&cls.obj_variables)),
                    owner: Owner::Object,
                    def: def.clone(),
                }));
            }
        }
        for m in &cls.obj_mixins {
            for c in linearize(interp, m) {
                take_class!(c);
            }
        }
        for c in linearize(interp, "::oo::class") {
            let mixins = interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default();
            for mx in &mixins {
                for c2 in linearize(interp, mx) {
                    take_class!(c2);
                }
            }
            take_class!(c);
        }
    }
    out
}

/// Owner-specific variable declarations for a chain entry.
fn owner_variables<'a>(
    interp: &'a Interp,
    owner: &'a Owner,
    key: &str,
) -> Option<&'a Rc<Vec<String>>> {
    match owner {
        Owner::Object => match interp.oo.objects.get(key) {
            Some(o) => Some(&o.variables),
            None => interp.oo.classes.get(key).map(|c| &c.obj_variables),
        },
        Owner::Class(c) => interp.oo.classes.get(c).map(|cls| &cls.variables),
    }
}

// ── active-method context ──────────────────────────────────────────────

/// Topmost live active-method entry (dropping stale ones).
fn top_active(interp: &mut Interp) -> Option<ActiveMethod> {
    loop {
        let a = interp.oo.active.last()?;
        if interp.frames.len() > a.frame_depth {
            return Some(a.clone());
        }
        interp.oo.active.pop();
    }
}

fn cleanup_active(interp: &mut Interp) {
    let len = interp.frames.len();
    while let Some(a) = interp.oo.active.last() {
        if a.frame_depth >= len {
            interp.oo.active.pop();
        } else {
            break;
        }
    }
}

/// Is the innermost frame inside the object's namespace subtree?
fn frame_in_object_ns(interp: &Interp, obj_ns: &str) -> bool {
    let ns = interp
        .frames
        .last()
        .and_then(|f| f.ns.clone())
        .unwrap_or_else(|| interp.current_namespace.clone());
    ns.as_ref() == obj_ns || ns.starts_with(&format!("{}::", obj_ns))
}

// ── messages ───────────────────────────────────────────────────────────

fn render_or_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} or {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// `unknown method "x": must be ...` — exported chain names + `destroy`,
/// with `new` suppressed on `::oo::class` itself.
fn unknown_method_error(interp: &Interp, key: &str, typed: &str, method: &str) -> Error {
    let hidden: Vec<String> = interp
        .oo
        .objects
        .get(key)
        .map(|o| o.hidden.clone())
        .unwrap_or_default();
    let walk = full_walk(interp, key);
    let fw: Vec<String> = walk.iter().map(|(n, _)| n.clone()).collect();
    let mut names: Vec<String> = Vec::new();
    for (name, entry) in &walk {
        if entry.def.exported
            && !hidden.contains(name)
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    }
    // destroy 的可见性与 full_walk 一致——类链 hidden（unexport
    // destroy）已在 walk 里排除，这里不能凭对象 hidden 无条件重推
    // （否则全隐藏判定永不触发——实测回归点）。
    if fw.iter().any(|n| n == "destroy") {
        names.push("destroy".to_string());
    }
    if key == "::oo::class" {
        names.retain(|n| n != "new");
    }
    names.sort();
    names.dedup();
    // D-41⑥：全部可见方法被隐藏（如 unexport destroy）时的消息。
    // 键用 canonical（tclsh ::o）。
    if names.is_empty() {
        return Error::Msg(format!(
            "object \"{}\" has no visible methods",
            key
        ));
    }
    let _ = typed;
    Error::Msg(format!(
        "unknown method \"{}\": must be {}",
        method,
        render_or_list(&names)
    ))
}

fn does_not_refer(kind_obj: bool, typed: &str) -> Error {
    Error::Msg(format!(
        "{} does not refer to an {}",
        typed,
        if kind_obj { "object" } else { "class" }
    ))
}

// ── method execution ───────────────────────────────────────────────────

/// Execute chain entry `index` for object `key` (`typed` = as-typed object
/// name, used in messages and `info level 0`).
#[allow(clippy::too_many_arguments)]
fn exec_chain_entry(
    interp: &mut Interp,
    key: &Rc<str>,
    method: &Rc<str>,
    index: usize,
    chain: &Rc<Vec<ChainEntry>>,
    call_args: &[Value],
    typed: &str,
    typed_val: &Value,
    fresh_chain: bool,
) -> Result<Value> {
    let entry = &chain[index];
    match entry.def.kind.clone() {
        MethodKind::Builtin(name) => match name {
            "destroy" => {
                if !call_args.is_empty() {
                    return Err(Error::wrong_args_with_usage(typed, 2, 2 + call_args.len(), "destroy"));
                }
                destroy_object(interp, key, false)?;
                Ok(Value::empty())
            }
            "create" => oo_create(interp, key, call_args, typed),
            "new" => oo_new(interp, key, call_args, typed),
            // D-43⑤：my variable / my varname / my eval——对象 ns 语义
            // （variable 链接对象 ns 变量进当前帧；varname 出全名；eval
            // 在对象 ns 求值）。<cloned> 为复制钩子占位（oo::copy 另走
            // build_chain 探测）。
            "variable" => {
                let ns = object_ns_of(interp, key);
                let prev = std::mem::replace(&mut interp.current_namespace, Rc::from(ns));
                let mut a: Vec<Value> = vec![Value::from_str("variable")];
                a.extend_from_slice(call_args);
                let r = interp.dispatch_values(&a);
                interp.current_namespace = prev;
                r
            }
            "varname" => {
                let ns = object_ns_of(interp, key);
                let name = call_args.first().map(|v| v.as_str()).unwrap_or("");
                Ok(Value::from_str(&format!("{}::{}", ns, name)))
            }
            "eval" => {
                let ns = object_ns_of(interp, key);
                let prev = std::mem::replace(&mut interp.current_namespace, Rc::from(ns));
                let script = call_args
                    .iter()
                    .map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let r = interp.eval(&script);
                interp.current_namespace = prev;
                r
            }
            "<cloned>" => Ok(Value::empty()),
            _ => Ok(Value::empty()),
        },
        MethodKind::Forward { cmd, prefix } => {
            let mut args: Vec<Value> = vec![Value::from_str(&cmd)];
            for p in &prefix {
                args.push(Value::from_str(p));
            }
            args.extend_from_slice(call_args);
            interp.dispatch_values(&args)
        }
        MethodKind::Tcl => {
            // tclsh compiles a method body once and caches the compiled
            // form on the method record; rtcl rebuilt this ProcDef per
            // call — two full-body String copies plus the bytecode
            // cache's full-text probe inside call_proc's eval (and no
            // compiled form at all, so the body ran tree-walked).  The
            // body is `def.body` verbatim; the owner's `variable`
            // declarations ride frame_prelink into call_proc's frame
            // setup (the same link, without dispatching a `variable`
            // command per name per call).  Memoised on the MethodDef;
            // a `variable` change rebuilds via the snapshot compare.
            // The def namespace rides the same value the frame install
            // would use (the object's namespace) — computed before the
            // memo build so the prelink list can pre-resolve each name.
            let ns = object_ns_of(interp, key);
            let (proc_def, links) = {
                // Hint only for chains straight from the stamp-checked
                // memo; `next`/cold paths keep the live probe so a
                // mid-body `variable` declaration is observed there.
                let vars_hint = if fresh_chain { entry.vars.as_ref() } else { None };
                memoised_proc_def(interp, &entry.owner, key, &entry.def, method, &ns, vars_hint)
            };
            if !links.is_empty() {
                interp.frame_prelink = Some(links);
            }

            // Argument head (typed, method Values) from the MethodDef's
            // cache when the typed name matches the last call site; the
            // vector itself comes from the pool — steady-state method
            // calls allocate nothing for the dispatch arguments.
            let mut args = interp.oo.args_pool.pop().unwrap_or_default();
            args.clear();
            {
                let mut hc = entry.def.head_cache.borrow_mut();
                match hc.as_mut() {
                    // Identity, not text: the cached handle stays alive in
                    // the cell, so pointer equality implies equal text — a
                    // different allocation with look-alike text misses and
                    // rebuilds (correct, just cold).  The method word rides
                    // along: `renamemethod` moves this def (and its cell)
                    // under a new name, and the old head would replay the
                    // stale name in `info level 0`.
                    Some((t, m, h0, h1))
                        if t.same_allocation(typed_val) && Rc::ptr_eq(m, method) =>
                    {
                        args.push(h0.clone());
                        args.push(h1.clone());
                    }
                    _ => {
                        let h0 = Value::from_str(typed);
                        let h1 = Value::from_str(method);
                        args.push(h0.clone());
                        args.push(h1.clone());
                        *hc = Some((typed_val.clone(), Rc::clone(method), h0, h1));
                    }
                }
            }
            args.extend_from_slice(call_args);

            interp.oo.active.push(ActiveMethod {
                obj: Rc::clone(key),
                method: Rc::clone(method),
                chain: Rc::clone(chain),
                index,
                frame_depth: interp.frames.len(),
            });
            let r = interp.call_proc(&proc_def, &args, typed, Some(ns));
            cleanup_active(interp);
            args.clear();
            if interp.oo.args_pool.len() < 8 {
                interp.oo.args_pool.push(args);
            }
            r
        }
    }
}

/// `variable a b;` prefix that auto-links the defining class's declared
/// variables into a method/constructor/destructor body (TclOO links them
/// without an explicit `variable` statement).  Names bound as parameters
/// are skipped.
fn link_prefix(vars: &[String], params: &[(String, Option<String>)]) -> String {
    let names: Vec<&str> = params.iter().map(|(p, _)| p.as_str()).collect();
    let link: Vec<&str> = vars.iter().map(|s| s.as_str()).filter(|v| !names.contains(v)).collect();
    // ONE `variable` command per name: `variable a b` at the Tcl level is
    // the PAIRED declare-with-initial-value form (b is a's initial value),
    // not a two-name declaration — a multi-name prefix used to clobber the
    // first variable with the literal second name (probed tclsh 8.6.17:
    // native TclOO links each name separately).
    link
        .iter()
        .map(|n| format!("variable {};", n))
        .collect()
}

/// Build the synthetic method [`ProcDef`] for `body` (already carrying
/// any `variable` link prefix).  The first parameter carries the invoked
/// method's name so arity errors read `wrong # args: should be
/// "obj m a b"` and `info level 0` is `obj m x`.
fn method_proc_def(
    body: &str,
    def_params: &[(String, Option<String>)],
    method: &str,
) -> ProcDef {
    let mut params: Vec<(String, Option<String>)> = vec![(method.to_string(), None)];
    params.extend(def_params.iter().cloned());
    ProcDef {
        params: Rc::new(params),
        body: Rc::from(body),
        statics: Rc::new(HashMap::new()),
        compiled: None,
    }
}

/// Memoised [`ProcDef`] for a method body: params/body/bytecode are pure
/// functions of (def, invoked method name, the owner's `variable` list —
/// the `link_prefix` input), so the assembled ProcDef is built and
/// compiled once and shared by every invocation (tclsh caches the
/// compiled body on its method record the same way).  Keyed by the
/// typed method name (params[0] feeds the arity-usage text and
/// `info level 0`) plus a snapshot of the owner's `variable` list; a
/// mismatch on either just rebuilds.  Method (re)definition installs a
/// fresh `MethodDef` (fresh cell), retiring the memo with its inputs; a
/// `variable` declaration change diverges the live list from the
/// snapshot (and bumps `mutation_ctr`, retiring the chain memo too).
/// `export` toggling is the only other in-place mutation and does not
/// enter the memo.  The shared `Rc<ProcDef>` is never written back: OO
/// statics are always empty, so `call_proc`'s `Rc::make_mut` write-back
/// cannot fire, and its per-call epoch/applicability gate keeps a stale
/// compiled form on the tree-walk exactly like a named proc's.
fn memoised_proc_def(
    interp: &mut Interp,
    owner: &Owner,
    key: &str,
    def: &MethodDef,
    method: &str,
    ns: &Rc<str>,
    vars_hint: Option<&Rc<Vec<String>>>,
) -> (Rc<ProcDef>, Rc<Vec<crate::interp::PrelinkEntry>>) {
    // The live `variable` list is the memo key — one probe of the owner's
    // table, no allocation; the link list (owner's declared variables
    // minus parameter-bound names — the old `link_prefix` filter, the
    // prefix text itself is gone) is built only on a rebuild and cached
    // beside the ProcDef.
    // The owner's live `variable` list, by identity: the memo keys on the
    // `Rc`'s pointer (a declaration replaces the vector wholesale), so the
    // per-call check is one pointer compare — no table-content walk.
    let vars: Option<Rc<Vec<String>>> = match vars_hint {
        Some(v) => Some(Rc::clone(v)),
        None => owner_variables(interp, owner, key).cloned(),
    };
    {
        let memo = def.proc_memo.borrow();
        if let Some(e) = memo.iter().find(|e| {
            Rc::ptr_eq(&e.ns, ns)
                && e.method.as_ref() == method
                && match (&e.vars, &vars) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                    _ => false,
                }
        }) {
            return (Rc::clone(&e.def), Rc::clone(&e.links));
        }
    }
    let names: Vec<&str> = def.params.iter().map(|(p, _)| p.as_str()).collect();
    let empty: Vec<String> = Vec::new();
    let vars_slice: &[String] = vars.as_deref().unwrap_or(&empty);
    let mut pd = method_proc_def(&def.body, &def.params, method);
    // Compile at the same seam named `proc`s use; the compiled form also
    // unlocks call_proc's slot-locals binding (empty statics, params
    // seeded in order — the synthetic name first, by construction).
    let compiled =
        super::super::vm_exec::compile_proc_body(&pd.params, &def.body, interp.tier1_epoch);
    // The prelink list lands here — after the compiled body exists, so
    // each entry can carry the frame slot its name occupies (the install
    // then skips the locals-table scan).
    let links: Rc<Vec<crate::interp::PrelinkEntry>> = Rc::new(
        vars_slice
            .iter()
            .filter(|v| !names.contains(&v.as_str()))
            .map(|v| {
                // Pure string math (see `resolve_var_link`) — precomputed
                // here so the per-call install never probes the memo.
                let (qualified, ns_part) =
                    crate::interp::commands::namespace::resolve_var_link(interp, ns, v);
                let slot = compiled.as_ref().and_then(|c| {
                    c.locals()
                        .iter()
                        .position(|l| l == v.as_str())
                        .map(|i| i as u16)
                });
                crate::interp::PrelinkEntry {
                    name: Rc::from(v.as_str()),
                    qualified,
                    ns_part,
                    registered: core::cell::Cell::new(false),
                    slot,
                }
            })
            .collect(),
    );
    if let Some(code) = &compiled {
        interp.const_pool_insert(code);
    }
    pd.compiled = compiled;
    let proc_def = Rc::new(pd);
    let mut memo = def.proc_memo.borrow_mut();
    if memo.len() >= PROC_MEMO_MAX {
        memo.remove(0);
    }
    memo.push(ProcMemoEntry {
        vars: vars.clone(),
        ns: Rc::clone(ns),
        method: Rc::from(method),
        def: Rc::clone(&proc_def),
        links: Rc::clone(&links),
    });
    (proc_def, links)
}

fn object_ns_of(interp: &Interp, key: &str) -> Rc<str> {
    if let Some(o) = interp.oo.objects.get(key) {
        return Rc::clone(&o.ns);
    }
    if let Some(c) = interp.oo.classes.get(key) {
        return Rc::clone(&c.ns);
    }
    Rc::from("::")
}

/// Is the entry still the resolution this invocation would recompute?
fn cache_entry_live(interp: &Interp, c: &OoCallCache, typed_val: &Value, method_val: &Value) -> bool {
    c.stamp == interp.oo.mutation_ctr
        && Rc::ptr_eq(&c.ns, &interp.current_namespace)
        && c.typed.same_allocation(typed_val)
        && c.method.same_allocation(method_val)
}

/// Two-entry probe for the `my` dispatcher's set.
fn my_cache_hit(interp: &Interp, typed_val: &Value, method_val: &Value) -> Option<usize> {
    if let Some(c) = &interp.oo.my_cache[0] {
        if cache_entry_live(interp, c, typed_val, method_val) {
            return Some(0);
        }
    }
    if let Some(c) = &interp.oo.my_cache[1] {
        if cache_entry_live(interp, c, typed_val, method_val) {
            return Some(1);
        }
    }
    None
}

/// Build an entry for the current resolution (the caller only fills after
/// a miss, so no stale-match handling is needed).
fn cache_entry(
    interp: &Interp,
    typed_val: &Value,
    method_val: &Value,
    obj_ns: Option<Rc<str>>,
    key: &Rc<str>,
    method_rc: &Rc<str>,
    chain: &Rc<Vec<ChainEntry>>,
) -> OoCallCache {
    OoCallCache {
        ns: Rc::clone(&interp.current_namespace),
        stamp: interp.oo.mutation_ctr,
        typed: typed_val.clone(),
        method: method_val.clone(),
        key: Rc::clone(key),
        obj_ns,
        method_rc: Rc::clone(method_rc),
        chain: Rc::clone(chain),
    }
}

/// Invoke `method` on `key`, or produce the unknown-method error.
///
/// The resolution chain comes from [`OoState::chain_memo`] — building it
/// walked every linearised class (C3 merge + per-class mixin lists) per
/// call, a third of the OO call bench.  A miss builds and backfills under
/// the live mutation stamp; empty chains memo too (the unknown-method
/// error path then skips the walk as well).
#[allow(clippy::too_many_arguments)]
fn invoke_method(
    interp: &mut Interp,
    key: &Rc<str>,
    method: &str,
    method_val: &Value,
    call_args: &[Value],
    typed: &str,
    typed_val: &Value,
    include_private: bool,
) -> Result<Value> {
    // D-41⑤：对象级 hidden 的方法对外部调用视同未定义（my 私有调用
    // 不受限——tclsh unexport 只对外）。
    if !include_private {
        if let Some(o) = interp.oo.objects.get(key.as_ref()) {
            if o.hidden.iter().any(|h| h == method) {
                return Err(unknown_method_error(interp, key, typed, method));
            }
        }
    }
    let (chain0, method_rc) = chain_for(interp, key, method, include_private);
    // D-41②：filter 前置进链——对象 filters 最先、类链 filter 按
    // linearize 序（最具体先），每个 filter 名在对象全链上解析取其
    // 最具体实现。filter 体内 `next` 顺着 (filter…→真实方法) 走，
    // 无 next 则其返回值即调用结果（tclsh probe: F + 空结果）。filter
    // 拦截**未知方法**调用（tclsh: filter 存在时 o whatever 也进
    // filter——D-41①② 交互）；链空且无 filter 才走 unknown 转调。
    // create/new/destroy 与私有调用不过滤。
    let chain = if !include_private
        && !matches!(method, "create" | "new" | "destroy" | "destructor" | "constructor")
    {
        let mut flist: Vec<String> = interp
            .oo
            .objects
            .get(key.as_ref())
            .map(|o| o.filters.clone())
            .unwrap_or_default();
        // filter 随**类链**收（对象键先取其类；linearize 不吃对象键）。
        let chain_root = interp
            .oo
            .objects
            .get(key.as_ref())
            .map(|o| o.class.clone())
            .unwrap_or_else(|| key.to_string());
        for c in linearize(interp, &chain_root) {
            if let Some(cls) = interp.oo.classes.get(&c) {
                for f in &cls.filters {
                    if !flist.contains(f) {
                        flist.push(f.clone());
                    }
                }
            }
        }
        if flist.is_empty() {
            chain0
        } else {
            let mut prepended: Vec<ChainEntry> = Vec::new();
            for f in flist.iter().rev() {
                let (fchain, _) = chain_for(interp, key, f, include_private);
                if let Some(entry) = fchain.first() {
                    prepended.push(entry.clone());
                }
            }
            if prepended.is_empty() {
                chain0
            } else {
                prepended.extend(chain0.iter().cloned());
                Rc::new(prepended)
            }
        }
    } else {
        chain0
    };
    if chain.is_empty() {
        // D-41①：未命中转调类的 `unknown` 方法（tclsh 语义——unknown
        // 收到 {原方法名 实参...}）；unknown 自身未命中才报
        // unknown-method 错误（此处不再二次转调，防递归）。
        let (uchain, umethod) = chain_for(interp, key, "unknown", include_private);
        if !uchain.is_empty() {
            let mut uargs: Vec<Value> = vec![Value::from_str(method)];
            uargs.extend_from_slice(call_args);
            return exec_chain_entry(
                interp,
                key,
                &umethod,
                0,
                &uchain,
                &uargs,
                typed,
                typed_val,
                false,
            );
        }
        return Err(unknown_method_error(interp, key, typed, method));
    }
    if include_private {
        let idx = (interp.oo.my_cache_next & 1) as usize;
        interp.oo.my_cache_next = (idx as u8 + 1) & 1;
        let entry = cache_entry(
            interp,
            typed_val,
            method_val,
            Some(object_ns_of(interp, key)),
            key,
            &method_rc,
            &chain,
        );
        interp.oo.my_cache[idx] = Some(entry);
    } else {
        let entry = cache_entry(interp, typed_val, method_val, None, key, &method_rc, &chain);
        interp.oo.obj_cache = Some(entry);
    }
    exec_chain_entry(interp, key, &method_rc, 0, &chain, call_args, typed, typed_val, true)
}

/// The memoised [`build_chain`] for this invocation context.  Nested maps
/// probed by `&str`: a hit allocates nothing (the flat `(String, String)`
/// key allocated two Strings per probe — see [`OoState::chain_memo`]).
fn chain_for(
    interp: &mut Interp,
    key: &str,
    method: &str,
    include_private: bool,
) -> (Rc<Vec<ChainEntry>>, Rc<str>) {
    let memo = if include_private {
        &interp.oo.chain_memo_priv
    } else {
        &interp.oo.chain_memo
    };
    // Hit: hand back the memo's own method-name Rc — the dispatcher's
    // ActiveMethod shares it, so a hit allocates nothing.
    if let Some((k, (_, chain))) = memo
        .get(key)
        .and_then(|inner| inner.get_key_value(method))
        .filter(|(_, (s, _))| *s == interp.oo.mutation_ctr)
    {
        return (Rc::clone(chain), Rc::clone(k));
    }
    let chain = Rc::new(build_chain(interp, key, method, include_private));
    let method_rc: Rc<str> = Rc::from(method);
    const CHAIN_MEMO_MAX: usize = 512;
    {
        let memo = if include_private {
            &mut interp.oo.chain_memo_priv
        } else {
            &mut interp.oo.chain_memo
        };
        if memo.len() >= CHAIN_MEMO_MAX {
            memo.clear();
        }
        memo.entry(Rc::from(key))
            .or_insert_with(FxMapK::default)
            .insert(Rc::clone(&method_rc), (interp.oo.mutation_ctr, Rc::clone(&chain)));
    }
    (chain, method_rc)
}

// ── the object dispatcher ──────────────────────────────────────────────

extern "Rust" fn cmd_oo_object(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Borrowed throughout: the typed name, the resolved key and the
    // method word never need to outlive this dispatch (the ActiveMethod
    // snapshot owns its copies).  The old form String-allocated the
    // typed name, the method name AND a Vec clone of the call args per
    // invocation — the dispatcher's entry tax.
    let typed = args[0].as_str();
    // Dispatcher cache (see [`OoCallCache`]): a stable typed word + method
    // word under the same namespace and live mutation stamp mean the
    // resolve and the chain memo would both hit — run the cached
    // resolution straight through.  `args.len() < 2` keeps falling into
    // the arity error below.
    if args.len() >= 2 {
        if let Some(c) = interp.oo.obj_cache.as_ref() {
            if cache_entry_live(interp, c, &args[0], &args[1]) {
                let (key, method_rc, chain) = (
                    Rc::clone(&c.key),
                    Rc::clone(&c.method_rc),
                    Rc::clone(&c.chain),
                );
                let r = exec_chain_entry(
                    interp,
                    &key,
                    &method_rc,
                    0,
                    &chain,
                    &args[2..],
                    typed,
                    &args[0],
                    true,
                );
                return r;
            }
        }
    }
    let key = match resolve_object_key(interp, typed) {
        Some(k) => k,
        None => return Err(Error::invalid_command(typed)),
    };
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            typed,
            2,
            args.len(),
            "method ?arg ...?",
        ));
    }
    let method = args[1].as_str();
    invoke_method(interp, &key, method, &args[1], &args[2..], typed, &args[0], false)
}

// ── my / self / next ───────────────────────────────────────────────────

extern "Rust" fn cmd_oo_my(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Borrowed throughout (see cmd_oo_object).
    let typed = args[0].as_str();
    // Dispatcher cache, `my` slot: the typed word ("my"), the method word,
    // the namespace and the stamp pin the resolution (`qualify` + the
    // `my_commands` probe) and the chain.  The frame guard still runs per
    // call — it depends on the live frame stack.
    if args.len() >= 2 {
        if let Some(i) = my_cache_hit(interp, &args[0], &args[1]) {
            let (key, method_rc, chain, obj_ns) = {
                let c = interp.oo.my_cache[i].as_ref().unwrap();
                (
                    Rc::clone(&c.key),
                    Rc::clone(&c.method_rc),
                    Rc::clone(&c.chain),
                    c.obj_ns.clone(),
                )
            };
            let obj_ns = obj_ns.unwrap_or_else(|| object_ns_of(interp, &key));
            if !frame_in_object_ns(interp, &obj_ns) {
                return Err(Error::invalid_command(typed));
            }
            return exec_chain_entry(
                interp,
                &key,
                &method_rc,
                0,
                &chain,
                &args[2..],
                typed,
                &args[0],
                true,
            );
        }
    }
    let cmd_key = qualify(&interp.current_namespace, typed);
    let key: Rc<str> = match interp.oo.my_commands.get(&cmd_key) {
        Some(k) => Rc::from(k.as_str()),
        None => return Err(Error::invalid_command(typed)),
    };
    let ns = object_ns_of(interp, &key);
    if !frame_in_object_ns(interp, &ns) {
        return Err(Error::invalid_command(typed));
    }
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("my", 2, args.len(), "method ?arg ...?"));
    }
    let method = args[1].as_str();
    invoke_method(interp, &key, method, &args[1], &args[2..], typed, &args[0], true)
}

const SELF_SUBCMDS: &[&str] = &[
    "call", "caller", "class", "filter", "method", "namespace", "next", "object", "target",
];

fn resolve_sub<'a>(word: &str, list: &[&'a str]) -> Option<&'a str> {
    for item in list {
        if *item == word {
            return Some(item);
        }
    }
    let mut hits: Vec<&'a str> = list.iter().filter(|i| i.starts_with(word)).copied().collect();
    if hits.len() == 1 {
        return hits.pop();
    }
    None
}

fn chain_desc(entry: &ChainEntry, method: &str) -> Value {
    let owner = match &entry.owner {
        Owner::Object => String::new(),
        Owner::Class(c) => c.clone(),
    };
    Value::from_list(&[
        Value::from_str("method"),
        Value::from_str(method),
        Value::from_str(&owner),
        Value::from_str("method"),
    ])
}

extern "Rust" fn cmd_oo_self(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        // `self` with no arguments is the object's fully-qualified name —
        // the same value `self object` yields (tclsh 8.6.17).
        return match top_active(interp) {
            Some(a) => Ok(Value::from_str(&a.obj)),
            None => Err(Error::invalid_command("self")),
        };
    }
    let word = args[1].as_str();
    let sub = match resolve_sub(word, SELF_SUBCMDS) {
        Some(s) => s,
        None => {
            return Err(Error::runtime(
                format!(
                    "bad subcommand \"{}\": must be {}",
                    word,
                    render_or_list(&SELF_SUBCMDS.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                ),
                ErrorCode::Generic,
            ))
        }
    };
    let act = match top_active(interp) {
        Some(a) => a,
        None => return Err(Error::invalid_command("self")),
    };
    match sub {
        "object" => Ok(Value::from_str(&act.obj)),
        "class" => Ok(Value::from_str(&object_class_of(interp, &act.obj))),
        "namespace" => Ok(Value::from_str(&object_ns_of(interp, &act.obj))),
        "method" => Ok(Value::from_str(&act.method)),
        "call" => {
            // tclsh 形状（D-43⑥ 深链实测）：{全链描述符列表 当前索引}
            // ——描述符覆盖整条调用链（B 的链含 A 的条目）。
            let descs: Vec<Value> = act
                .chain
                .iter()
                .map(|e| chain_desc(e, act.method.as_ref()))
                .collect();
            Ok(Value::from_list(&[
                Value::from_list(&descs),
                Value::from_int(act.index as i64),
            ]))
        }
        "next" => {
            // tclsh 形状（实测）：仅**紧邻下一实现**一对 {定义类 方法}——
            // 平铺两元素（Cc→B→A 链上只报 ::B m，不含更远）。
            let mut items: Vec<Value> = Vec::new();
            if let Some(e) = act.chain.get(act.index + 1) {
                let owner = match &e.owner {
                    Owner::Object => String::new(),
                    Owner::Class(c) => c.clone(),
                };
                items.push(Value::from_str(&owner));
                items.push(Value::from_str(act.method.as_ref()));
            }
            Ok(Value::from_list(&items))
        }
        "filter" | "target" => Err(Error::Msg("not inside a filtering context".to_string())),
        "caller" => Err(Error::Msg("caller is not an object".to_string())),
        _ => Err(Error::invalid_command("self")),
    }
}

fn object_class_of(interp: &Interp, key: &str) -> String {
    if let Some(o) = interp.oo.objects.get(key) {
        return o.class.clone();
    }
    if interp.oo.classes.contains_key(key) {
        return "::oo::class".to_string();
    }
    String::new()
}

extern "Rust" fn cmd_oo_next(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let act = match top_active(interp) {
        Some(a) => a,
        None => {
            set_tcloo_next_inner(interp);
            return Err(Error::Msg("no next method implementation".to_string()));
        }
    };
    let next_index = act.index + 1;
    if next_index >= act.chain.len() {
        // D-40⑤：用尽消息按方法名（tclsh：析构器/构造器各有其文）。
        // D-3：next 自身的分派失败 → INNER 记为 tclooNext（tclsh 实测）。
        set_tcloo_next_inner(interp);
        return Err(Error::Msg(
            match act.method.as_ref() {
                "destructor" => "no next destructor implementation",
                "constructor" => "no next constructor implementation",
                _ => "no next method implementation",
            }
            .to_string(),
        ));
    }
    // D-40④：`next` 无参 = 以无参调下一实现（tclsh 8.6 不重转发当前
    // 实参）；显式实参照转。
    let call_args: Vec<Value> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        Vec::new()
    };
    // D-40⑤ 精化：下一实现的 level0 = `next <args>`（tclsh 形状）——
    // 借 frame_level0_args 一次性覆写通道（apply 同款）。
    let mut l0 = vec![Value::from_str("next")];
    l0.extend(args[1..].iter().cloned());
    interp.frame_level0_args = Some(l0);
    // D-3：下一实现帧的 usage 词是 `next`，且合成方法名参数不进
    // usage（tclsh：`wrong # args: should be "next x"`）——不再从
    // level0 首词取对象名（next 帧的 level0 首词就是 "next"）。
    interp.frame_next_call = true;
    let typed = "next";
    let r = exec_chain_entry(
        interp,
        &act.obj,
        &act.method,
        next_index,
        &act.chain,
        &call_args,
        typed,
        &Value::from_str(typed),
        false,
    );
    // D-3：next 目标的**分派/绑定失败**（无下一实现、目标 arity 错）
    // 只留 INNER=tclooNext（CALL 条目由外层日志事件按帧追加）；
    // 目标方法体出错则 INNER 已由体内事件给出，不动。
    if let Err(e) = &r {
        if interp.err_is_error(e) && !interp.err_call_body {
            set_tcloo_next_inner(interp);
        }
    }
    r
}

/// next 自身分派失败时的 INNER（tclsh 的内部命令名，实测原样单词）。
fn set_tcloo_next_inner(interp: &mut Interp) {
    interp.err_pending_inner = Some("tclooNext".to_string());
    interp.err_call_body = false;
}

// ── create / new / destroy builtins ────────────────────────────────────

fn next_ns(interp: &mut Interp) -> String {
    interp.oo.next_obj_id += 1;
    format!("::oo::Obj{}", interp.oo.next_obj_id)
}

fn attach_ns_commands(interp: &mut Interp, key: &str, ns: &str) {
    for role in ["my", "self", "next"] {
        let cmd_key = format!("{}::{}", ns, role);
        let func: crate::command::CommandFunc = match role {
            "my" => cmd_oo_my,
            "self" => cmd_oo_self,
            _ => cmd_oo_next,
        };
        interp.commands.insert(cmd_key.clone(), func);
        interp
            .command_categories
            .insert(cmd_key.clone(), crate::command::CommandCategory::Extension);
        interp.oo.my_commands.insert(cmd_key, key.to_string());
    }
    interp.note_cmd_mutation();
}

/// Remove everything a create/new (or destroy) attached for `key`.
fn detach_object(interp: &mut Interp, key: &str, ns: &str, keep_ns: bool) {
    super::super::vm_exec::note_tier1_mutation(interp, key);
    interp.note_cmd_mutation();
    interp.commands.remove(key);
    interp.command_categories.remove(key);
    interp.command_meta.remove(key);
    if parent_of(key) == "::" {
        let bare = key.trim_start_matches(':');
        interp.commands.remove(bare);
        interp.command_categories.remove(bare);
        interp.command_meta.remove(bare);
    }
    for role in ["my", "self", "next"] {
        let cmd_key = format!("{}::{}", ns, role);
        interp.commands.remove(&cmd_key);
        interp.command_categories.remove(&cmd_key);
        interp.command_meta.remove(&cmd_key);
        interp.oo.my_commands.remove(&cmd_key);
    }
    // Renamed-away my/self/next commands (same dispatcher, key no longer
    // registered) are swept with their owner.
    let orphans: Vec<String> = interp
        .commands
        .iter()
        .filter(|(k, f)| {
            let is_oo = **f as usize == cmd_oo_my as *const () as usize
                || **f as usize == cmd_oo_self as *const () as usize
                || **f as usize == cmd_oo_next as *const () as usize;
            is_oo && !interp.oo.my_commands.contains_key(*k)
        })
        .map(|(k, _)| k.clone())
        .collect();
    for k in orphans {
        interp.commands.remove(&k);
        interp.command_categories.remove(&k);
        interp.command_meta.remove(&k);
    }
    interp.oo.objects.remove(key);
    interp.oo.classes.remove(key);
    interp.oo.mutation_ctr += 1;
    interp.oo.creation_order.retain(|k| k != key);
    // Namespace-scoped storage.  A running method's `variable` links into
    // this namespace keep their value as a seeded local copy (tclsh: the
    // linked Var is refcounted — the destroy purge must not strand the
    // frame's reads).
    let var_prefix = format!("{}::", ns.trim_start_matches(':'));
    interp.materialize_frame_links(&var_prefix);
    let globals: Vec<String> = interp
        .globals
        .keys()
        .filter(|k| k.starts_with(&var_prefix))
        .cloned()
        .collect();
    for g in globals {
        interp.globals.remove(&g);
        interp.array_globals.remove(&g);
        interp.arrays_env_only = interp
            .array_globals
            .iter()
            .all(|k| k == "env" || k == "tcl_platform");
    }
    if !keep_ns {
        interp.namespaces.remove(ns);
    }
}

/// Register a freshly created object/class in every table.  A global-scope
/// object also gets its bare key (`::foo` is callable as `foo` from the
/// global namespace — tclsh command lookup reaches `::name` unqualified).
fn attach_object(interp: &mut Interp, key: &str, ns: &str) {
    super::super::vm_exec::note_tier1_mutation(interp, key);
    interp.note_cmd_mutation();
    register_object_command(interp, key, cmd_oo_object);
    if parent_of(key) == "::" {
        let bare = key.trim_start_matches(':');
        interp.commands.insert(bare.to_string(), cmd_oo_object);
        interp
            .command_categories
            .insert(bare.to_string(), crate::command::CommandCategory::Extension);
        interp.command_meta.insert(
            bare.to_string(),
            crate::command::CommandMeta { usage: "methodName ?arg ...?", help: "Object command" },
        );
    }
    ensure_ns(&mut interp.namespaces, ns);
    attach_ns_commands(interp, key, ns);
    interp.oo.creation_order.push(key.to_string());
}

fn oo_create(interp: &mut Interp, class_key: &str, rest: &[Value], typed: &str) -> Result<Value> {
    // D-41④：元类判定 = 类链含 ::oo::class（用户元类 K{superclass
    // oo::class} 的 create 定义脚本照常求值；::oo::class 自身线性化
    // 含自身，天然为真）。
    let is_meta = class_key == "::oo::class"
        || chain_contains(interp, class_key, "::oo::class");
    if rest.is_empty() {
        let usage = if is_meta {
            "create objectName ?definitionScript?"
        } else {
            "create objectName ?arg ...?"
        };
        return Err(Error::wrong_args_with_usage(typed, 2, 2, usage));
    }
    let name_typed = rest[0].as_str().to_string();
    let canonical = qualify(&interp.current_namespace, &name_typed);
    if command_exists_any(interp, &canonical) {
        return Err(Error::Msg(format!(
            "can't create object \"{}\": command already exists with that name",
            name_typed
        )));
    }
    let ns = Rc::from(next_ns(interp));
    if is_meta {
        interp.oo.classes.insert(
            canonical.clone(),
            OoClass {
                ns: Rc::clone(&ns),
                superclasses: vec!["::oo::object".to_string()],
                ..Default::default()
            },
        );
    } else {
        interp.oo.objects.insert(
            canonical.clone(),
            OoObject { ns: Rc::clone(&ns), class: class_key.to_string(), ..Default::default() },
        );
    }
    interp.oo.mutation_ctr += 1;
    attach_object(interp, &canonical, &ns);

    // Definition script (metaclass create only).
    if is_meta && rest.len() > 1 {
        if rest.len() > 2 {
            let e = Error::wrong_args_with_usage(
                typed,
                2,
                2 + rest.len(),
                format!("create {} ?definitionScript?", name_typed),
            );
            detach_object(interp, &canonical, &ns, false);
            return Err(e);
        }
        let script = rest[1].as_str().to_string();
        if let Err(e) = run_define_script(interp, DefineTarget::Class(canonical.clone()), &script, true) {
            detach_object(interp, &canonical, &ns, false);
            return Err(e);
        }
    }

    // Constructor.  D-40②③：沿链找最具体的构造器（子类无 ctor 时继承
    // 链上的——create 参数不再静默丢弃），其 variable 链接到**定义该
    // 构造器的类**（memoised_proc_def 的 owner 路径），`next` 与普通
    // 方法同机制。
    if !is_meta {
        let chain = Rc::new(ctor_chain(interp, class_key));
        if !chain.is_empty() {
            let method: Rc<str> = Rc::from("constructor");
            let canonical_rc: Rc<str> = Rc::from(canonical.as_str());
            let typed_val = Value::from_str(&name_typed);
            let call_args: Vec<Value> = rest[1..].to_vec();
            // tclsh 构造器帧的 level0 = **类命令调用词**（实测
            // `CC create xyz 5`），CALL 条目同源。
            {
                let mut l0 = vec![Value::from_str(typed), Value::from_str("create")];
                l0.extend(rest.iter().cloned());
                interp.frame_level0_args = Some(l0);
            }
            let r = exec_chain_entry(
                interp,
                &canonical_rc,
                &method,
                0,
                &chain,
                &call_args,
                &name_typed,
                &typed_val,
                false,
            );
            if let Err(e) = r {
                detach_object(interp, &canonical, &ns, false);
                return Err(e);
            }
        }
    }
    Ok(Value::from_str(&canonical))
}

fn oo_new(interp: &mut Interp, class_key: &str, rest: &[Value], typed: &str) -> Result<Value> {
    // D-41③：oo::class 自身只有 create/destroy——`oo::class new` 会
    // 造出非类对象（info object isa class = 0 的坏实例），tclsh 拒绝。
    if class_key == "::oo::class" {
        return Err(Error::Msg(
            "unknown method \"new\": must be create or destroy".to_string(),
        ));
    }
    let name = next_ns(interp);
    let ns: Rc<str> = Rc::from(name.as_str());
    interp.oo.objects.insert(
        name.clone(),
        OoObject { ns: Rc::clone(&ns), class: class_key.to_string(), ..Default::default() },
    );
    interp.oo.mutation_ctr += 1;
    attach_object(interp, &name, &ns);

    // D-40②③：与 oo_create 同——链上最具体构造器 + owner 变量链接。
    let chain = Rc::new(ctor_chain(interp, class_key));
    if !chain.is_empty() {
        let method: Rc<str> = Rc::from("constructor");
        let name_rc: Rc<str> = Rc::from(name.as_str());
        let typed_val = Value::from_str(&name);
        let call_args: Vec<Value> = rest.to_vec();
        // tclsh 构造器帧的 level0 = 类命令调用词（`FF new 5`）。
        {
            let mut l0 = vec![Value::from_str(typed), Value::from_str("new")];
            l0.extend(rest.iter().cloned());
            interp.frame_level0_args = Some(l0);
        }
        let r = exec_chain_entry(
            interp,
            &name_rc,
            &method,
            0,
            &chain,
            &call_args,
            &name,
            &typed_val,
            false,
        );
        if let Err(e) = r {
            detach_object(interp, &name, &ns, false);
            return Err(e);
        }
    }
    Ok(Value::from_str(&name))
}

/// Constructor chain: classes in linearisation order that define one.
fn ctor_chain(interp: &Interp, class_key: &str) -> Vec<ChainEntry> {
    let mut out = Vec::new();
    for c in linearize(interp, class_key) {
        if let Some(cls) = interp.oo.classes.get(&c) {
            if let Some(def) = &cls.constructor {
                out.push(ChainEntry {
                    vars: Some(Rc::clone(&cls.variables)),
                    owner: Owner::Class(c.clone()),
                    def: def.clone(),
                });
            }
        }
    }
    out
}

fn dtor_chain(interp: &Interp, class_key: &str) -> Vec<ChainEntry> {
    let mut out = Vec::new();
    for c in linearize(interp, class_key) {
        if let Some(cls) = interp.oo.classes.get(&c) {
            if let Some(def) = &cls.destructor {
                out.push(ChainEntry {
                    vars: Some(Rc::clone(&cls.variables)),
                    owner: Owner::Class(c.clone()),
                    def: def.clone(),
                });
            }
        }
    }
    out
}

/// Does `key`'s resolution chain reference `class_key` — via the
/// superclass linearisation or via any chain class's mixins?
fn chain_contains(interp: &Interp, key: &str, class_key: &str) -> bool {
    fn class_refs(interp: &Interp, class: &str, target: &str) -> bool {
        for c in linearize(interp, class) {
            if c == target {
                return true;
            }
            if let Some(cls) = interp.oo.classes.get(&c) {
                if cls.mixins.iter().any(|m| m == target) {
                    return true;
                }
            }
        }
        false
    }
    if let Some(obj) = interp.oo.objects.get(key) {
        if class_refs(interp, &obj.class, class_key) {
            return true;
        }
        for m in &obj.mixins {
            if linearize(interp, m).iter().any(|c| c == class_key) {
                return true;
            }
        }
        return false;
    }
    if interp.oo.classes.contains_key(key) {
        return class_refs(interp, key, class_key);
    }
    false
}

/// Destroy an object or class.  Classes cascade onto every object whose
/// chain contains them and every class (subclass or mixin user) whose
/// chain contains them.  Destructor errors are raised *after* removal
/// (tclsh: the object is gone either way).
fn destroy_object(interp: &mut Interp, key: &str, keep_ns: bool) -> Result<Value> {
    let is_class = interp.oo.classes.contains_key(key);
    let is_object = interp.oo.objects.contains_key(key);
    if !is_class && !is_object {
        return Ok(Value::empty());
    }

    let mut dtor_error: Option<Error> = None;
    if is_class {
        // Objects first (creation order), then classes.
        let victims: Vec<String> = interp
            .oo
            .creation_order
            .iter()
            .filter(|k| **k != key && interp.oo.objects.contains_key(*k))
            .filter(|k| chain_contains(interp, k, key))
            .cloned()
            .collect();
        for v in victims {
            destroy_object(interp, &v, false)?;
        }
        let subvictims: Vec<String> = interp
            .oo
            .creation_order
            .iter()
            .filter(|k| **k != key && interp.oo.classes.contains_key(*k))
            .filter(|k| chain_contains(interp, k, key))
            .cloned()
            .collect();
        for v in subvictims {
            destroy_object(interp, &v, false)?;
        }
    } else {
        // D-40①：只跑最具体一个析构器，`next` 沿链继续（tclsh 语义；
        // rtcl 原无条件跑全链，析构器内调 next 时链上析构器双跑）。走
        // exec_chain_entry——ActiveMethod/index、链上每个析构器自己的
        // variable 链接（memoised_proc_def 的 owner 路径）与 next 全部
        // 与普通方法同机制。析构错误在对象移除后抛出（不变）。
        let class_key = interp.oo.objects.get(key).map(|o| o.class.clone()).unwrap_or_default();
        let chain = Rc::new(dtor_chain(interp, &class_key));
        if !chain.is_empty() {
            let method: Rc<str> = Rc::from("destructor");
            let key_rc: Rc<str> = Rc::from(key);
            let typed_val = Value::from_str(key);
            // tclsh 析构器帧是 **objc=0 特殊帧**（TclLogCommandInfo 的
            // "Special frame, nothing to report"）：info level 0 为空、
            // 错误日志事件不追加 CALL。
            interp.frame_level0_empty = true;
            if let Err(e) = exec_chain_entry(
                interp,
                &key_rc,
                &method,
                0,
                &chain,
                &[],
                key,
                &typed_val,
                false,
            ) {
                dtor_error = Some(e);
            }
        }
    }

    let ns = object_ns_of(interp, key);
    detach_object(interp, key, &ns, keep_ns);
    match dtor_error {
        Some(e) => Err(e),
        None => Ok(Value::empty()),
    }
}

// ── oo::define / oo::objdefine ─────────────────────────────────────────

fn run_define_script(
    interp: &mut Interp,
    target: DefineTarget,
    script: &str,
    class_form: bool,
) -> Result<Value> {
    let name = match &target {
        DefineTarget::Class(c) | DefineTarget::Object(c) => c.clone(),
    };
    interp.oo.define_stack.push(target);
    let prev = core::mem::replace(&mut interp.current_namespace, Rc::from("::oo::define"));
    let r = interp.eval(script);
    interp.current_namespace = prev;
    interp.oo.define_stack.pop();
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            let tag = if class_form {
                format!("in definition script for class \"{}\"", name)
            } else {
                format!("in definition script for object \"{}\"", name)
            };
            interp.err_exit_frame(&tag);
        }
    }
    r
}

extern "Rust" fn cmd_oo_define(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "oo::define",
            3,
            args.len(),
            "className arg ?arg ...?",
        ));
    }
    let typed = args[1].as_str().to_string();
    let canonical = match resolve_object_key(interp, &typed) {
        Some(k) if interp.oo.classes.contains_key(k.as_ref()) => k,
        Some(_) => return Err(does_not_refer(false, &typed)),
        None => return Err(does_not_refer(true, &typed)),
    };
    if args.len() == 3 {
        let script = args[2].as_str().to_string();
        run_define_script(interp, DefineTarget::Class(canonical.to_string()), &script, true)
    } else {
        // Multi-word form: the words form a single command evaluated in the
        // definition context, with no definition-script error frame
        // (tclsh: `oo::define C error foo` reports only the outer frame).
        interp.oo.define_stack.push(DefineTarget::Class(canonical.to_string()));
        let prev = core::mem::replace(&mut interp.current_namespace, Rc::from("::oo::define"));
        let r = interp.dispatch_values(&args[2..]);
        interp.current_namespace = prev;
        interp.oo.define_stack.pop();
        r
    }
}

extern "Rust" fn cmd_oo_objdefine(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "oo::objdefine",
            3,
            args.len(),
            "objectName arg ?arg ...?",
        ));
    }
    let typed = args[1].as_str().to_string();
    let canonical = match resolve_object_key(interp, &typed) {
        Some(k) => k,
        None => return Err(does_not_refer(true, &typed)),
    };
    if args.len() == 3 {
        let script = args[2].as_str().to_string();
        run_define_script(interp, DefineTarget::Object(canonical.to_string()), &script, false)
    } else {
        // Multi-word form — see cmd_oo_define.
        interp.oo.define_stack.push(DefineTarget::Object(canonical.to_string()));
        let prev = core::mem::replace(&mut interp.current_namespace, Rc::from("::oo::define"));
        let r = interp.dispatch_values(&args[2..]);
        interp.current_namespace = prev;
        interp.oo.define_stack.pop();
        r
    }
}

/// `::tcl::oo::def word ?arg...?` — hidden dispatcher behind the
/// definition-word procs.
extern "Rust" fn cmd_oo_def(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("::tcl::oo::def", 2, args.len(), "word ?arg ...?"));
    }
    let word = args[1].as_str().to_string();
    def_dispatch(interp, &word, &args[2..])
}

// ── definition words ───────────────────────────────────────────────────

fn parse_param_list(text: &str) -> Result<Vec<(String, Option<String>)>> {
    let items = Value::from_str(text).as_list().unwrap_or_else(|| vec![Value::from_str(text)]);
    let mut specs: Vec<(String, Option<String>)> = Vec::new();
    for item in items {
        let parts = item.as_list().unwrap_or_else(|| vec![item.clone()]);
        if parts.is_empty() {
            return Err(Error::Msg("argument with no name".to_string()));
        }
        if parts.len() > 2 {
            return Err(Error::Msg(format!(
                "too many fields in argument specifier \"{}\"",
                item.as_str()
            )));
        }
        if parts.len() == 2 {
            specs.push((parts[0].as_str().to_string(), Some(parts[1].as_str().to_string())));
        } else {
            specs.push((parts[0].as_str().to_string(), None));
        }
    }
    Ok(specs)
}

fn name_exports(name: &str) -> bool {
    matches!(name.as_bytes().first(), Some(c) if c.is_ascii_lowercase())
}

/// Apply a definition word to the open definition target.
fn def_dispatch(interp: &mut Interp, word: &str, rest: &[Value]) -> Result<Value> {
    let target = match interp.oo.define_stack.last() {
        Some(t) => t.clone(),
        None => return Err(Error::invalid_command(word)),
    };
    match word {
        "method" => {
            if rest.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    word, 4, rest.len() + 1, "name args body",
                ));
            }
            let name = rest[0].as_str().to_string();
            let params = parse_param_list(rest[1].as_str())?;
            let def = MethodDef {
                params: params.into(),
                body: Rc::from(rest[2].as_str()),
                exported: name_exports(&name),
                kind: MethodKind::Tcl,
                proc_memo: fresh_memo(), head_cache: RefCell::new(None),
            };
            set_method(interp, &target, &name, def)
        }
        "forward" => {
            if rest.len() < 2 {
                return Err(Error::wrong_args_with_usage(
                    word, 3, rest.len() + 1, "name cmdName ?arg ...?",
                ));
            }
            let name = rest[0].as_str().to_string();
            let def = MethodDef {
                params: Vec::new().into(),
                body: Rc::from(""),
                exported: name_exports(&name),
                kind: MethodKind::Forward {
                    cmd: rest[1].as_str().to_string(),
                    prefix: rest[2..].iter().map(|v| v.as_str().to_string()).collect(),
                },
                proc_memo: fresh_memo(), head_cache: RefCell::new(None),
            };
            set_method(interp, &target, &name, def)
        }
        "mixin" => {
            let mut mixins: Vec<String> = Vec::new();
            for arg in rest {
                let typed = arg.as_str();
                let canonical = match resolve_object_key(interp, typed) {
                    Some(k) => k,
                    None => return Err(does_not_refer(true, typed)),
                };
                if !interp.oo.classes.contains_key(canonical.as_ref()) {
                    if canonical.as_ref() == def_target_name(&target) {
                        return Err(Error::Msg("may not mix a class into itself".to_string()));
                    }
                    return Err(Error::Msg("may only mix in classes".to_string()));
                }
                if canonical.as_ref() == def_target_name(&target) {
                    return Err(Error::Msg("may not mix a class into itself".to_string()));
                }
                mixins.push(canonical.to_string());
            }
            // D-41⑦：间接环（M mixin A、A 的 superclass 是 M）在定义时
            // 即报——沿 mixin+superclass 边从新 mixin 名单出发，回到
            // 目标类即环（linearize 的深度 64 截断不再静默吞）。
            if let DefineTarget::Class(cname) = &target {
                // seen 不预置目标——目标由 c == cname 判定（预置会把
                // 回边挡掉，环永远查不出来）。
                let mut seen: Vec<String> = Vec::new();
                let mut frontier = mixins.clone();
                let mut cyc = false;
                let mut depth = 0;
                while !frontier.is_empty() && depth < 64 {
                    depth += 1;
                    let mut next: Vec<String> = Vec::new();
                    for c in &frontier {
                        if c == cname {
                            cyc = true;
                            break;
                        }
                        if let Some(cls) = interp.oo.classes.get(c) {
                            for m in cls.mixins.iter().chain(cls.superclasses.iter()) {
                                if !seen.contains(m) {
                                    seen.push(m.clone());
                                    next.push(m.clone());
                                }
                            }
                        }
                    }
                    if cyc {
                        break;
                    }
                    frontier = next;
                }
                if cyc {
                    return Err(Error::Msg(
                        "may not mix a class into itself".to_string(),
                    ));
                }
            }
            match target {
                DefineTarget::Object(name) => {
                    if let Some(o) = interp.oo.objects.get_mut(&name) {
                        o.mixins = mixins;
                    } else if let Some(c) = interp.oo.classes.get_mut(&name) {
                        c.obj_mixins = mixins;
                    }
                }
                DefineTarget::Class(name) => {
                    if let Some(c) = interp.oo.classes.get_mut(&name) {
                        c.mixins = mixins;
                    }
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "superclass" => {
            if let DefineTarget::Object(_) = target {
                return Err(Error::invalid_command(word));
            }
            let target_name = def_target_name(&target);
            let mut supers: Vec<String> = Vec::new();
            for arg in rest {
                let typed = arg.as_str();
                let canonical = match resolve_object_key(interp, typed) {
                    Some(k) => k,
                    None => return Err(does_not_refer(true, typed)),
                };
                if !interp.oo.classes.contains_key(canonical.as_ref()) {
                    return Err(Error::Msg("only a class can be a superclass".to_string()));
                }
                if canonical.as_ref() == target_name
                    || linearize(interp, &canonical).contains(&target_name)
                {
                    return Err(Error::Msg(
                        "attempt to form circular dependency graph".to_string(),
                    ));
                }
                supers.push(canonical.to_string());
            }
            if let DefineTarget::Class(name) = target {
                if let Some(c) = interp.oo.classes.get_mut(&name) {
                    // D-40⑥：`oo::define C {superclass}`（空名单）重置为
                    // ::oo::object（tclsh 语义——实例从此保有 destroy）。
                    if supers.is_empty() {
                        supers.push("::oo::object".to_string());
                    }
                    c.superclasses = supers;
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "constructor" => {
            if let DefineTarget::Object(_) = target {
                return Err(Error::invalid_command(word));
            }
            if rest.len() != 2 {
                return Err(Error::wrong_args_with_usage(
                    word, 3, rest.len() + 1, "arguments body",
                ));
            }
            let params = parse_param_list(rest[0].as_str())?;
            if let DefineTarget::Class(name) = target {
                if let Some(c) = interp.oo.classes.get_mut(&name) {
                    c.constructor = Some(MethodDef {
                        params: params.into(),
                        body: Rc::from(rest[1].as_str()),
                        exported: false,
                        kind: MethodKind::Tcl,
                        proc_memo: fresh_memo(), head_cache: RefCell::new(None),
                    });
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "destructor" => {
            if let DefineTarget::Object(_) = target {
                return Err(Error::invalid_command(word));
            }
            if rest.len() != 1 {
                return Err(Error::wrong_args_with_usage(word, 2, rest.len() + 1, "body"));
            }
            if let DefineTarget::Class(name) = target {
                if let Some(c) = interp.oo.classes.get_mut(&name) {
                    c.destructor = Some(MethodDef {
                        params: Vec::new().into(),
                        body: Rc::from(rest[0].as_str()),
                        exported: false,
                        kind: MethodKind::Tcl,
                        proc_memo: fresh_memo(), head_cache: RefCell::new(None),
                    });
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "export" => {
            if rest.is_empty() {
                return Err(Error::wrong_args_with_usage(word, 2, 1, "name ?name ...?"));
            }
            for arg in rest {
                set_export(interp, &target, arg.as_str(), true);
            }
            Ok(Value::empty())
        }
        "unexport" => {
            for arg in rest {
                if !set_export(interp, &target, arg.as_str(), false) {
                    // Hiding a method this class does not define shadows the
                    // inherited implementation.
                    if let DefineTarget::Class(name) = &target {
                        if let Some(c) = interp.oo.classes.get_mut(name) {
                            if !c.hidden.iter().any(|h| h == arg.as_str()) {
                                c.hidden.push(arg.as_str().to_string());
                            }
                        }
                    }
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "deletemethod" => {
            for arg in rest {
                delete_method(interp, &target, arg.as_str());
            }
            Ok(Value::empty())
        }
        "variable" => {
            // Validate every name first (tclsh adds none on error).
            for arg in rest {
                let name = arg.as_str();
                if name.contains("::") {
                    return Err(Error::Msg(format!(
                        "invalid declared variable name \"{}\": must not contain namespace separators",
                        name
                    )));
                }
                if name.contains('(') {
                    return Err(Error::Msg(format!(
                        "invalid declared variable name \"{}\": must not refer to an array element",
                        name
                    )));
                }
            }
            for arg in rest {
                let name = arg.as_str().to_string();
                match &target {
                    DefineTarget::Class(name2) => {
                        if let Some(c) = interp.oo.classes.get_mut(name2) {
                            if !c.variables.contains(&name) {
                                let mut v = (*c.variables).clone();
                                v.push(name);
                                c.variables = Rc::new(v);
                            }
                        }
                    }
                    DefineTarget::Object(name2) => {
                        if let Some(o) = interp.oo.objects.get_mut(name2) {
                            if !o.variables.contains(&name) {
                                let mut v = (*o.variables).clone();
                                v.push(name);
                                o.variables = Rc::new(v);
                            }
                        } else if let Some(c) = interp.oo.classes.get_mut(name2) {
                            if !c.obj_variables.contains(&name) {
                                let mut v = (*c.obj_variables).clone();
                                v.push(name);
                                c.obj_variables = Rc::new(v);
                            }
                        }
                    }
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "filter" => {
            match target {
                DefineTarget::Class(name) => {
                    if let Some(c) = interp.oo.classes.get_mut(&name) {
                        c.filters = rest.iter().map(|v| v.as_str().to_string()).collect();
                    }
                }
                // D-41②：对象级 filter 也生效（原静默吞掉）。
                DefineTarget::Object(name) => {
                    if let Some(o) = interp.oo.objects.get_mut(name.as_str()) {
                        o.filters = rest.iter().map(|v| v.as_str().to_string()).collect();
                    }
                }
            }
            interp.oo.mutation_ctr += 1;
            Ok(Value::empty())
        }
        "renamemethod" => {
            if rest.len() != 2 {
                return Err(Error::wrong_args_with_usage(word, 3, rest.len() + 1, "fromName toName"));
            }
            let from = rest[0].as_str().to_string();
            let to = rest[1].as_str().to_string();
            let def = take_method(interp, &target, &from);
            if let Some(def) = def {
                let mut def = def;
                def.exported = name_exports(&to);
                set_method(interp, &target, &to, def)?;
            }
            Ok(Value::empty())
        }
        "self" => {
            if rest.is_empty() {
                return Err(Error::wrong_args_with_usage(word, 2, 1, "arg ?arg ...?"));
            }
            interp.dispatch_values(rest)
        }
        _ => Err(Error::invalid_command(word)),
    }
}

fn def_target_name(target: &DefineTarget) -> String {
    match target {
        DefineTarget::Class(c) | DefineTarget::Object(c) => c.clone(),
    }
}

/// Insert or replace a method in the target's own table (redefinition
/// resets the export state to the name-derived default).
fn set_method(interp: &mut Interp, target: &DefineTarget, name: &str, def: MethodDef) -> Result<Value> {
    match target {
        DefineTarget::Class(cn) => {
            if let Some(c) = interp.oo.classes.get_mut(cn) {
                if let Some(slot) = c.methods.iter_mut().find(|(n, _)| n == name) {
                    slot.1 = def;
                } else {
                    c.methods.push((name.to_string(), def));
                }
            }
        }
        DefineTarget::Object(on) => {
            if let Some(o) = interp.oo.objects.get_mut(on) {
                if let Some(slot) = o.methods.iter_mut().find(|(n, _)| n == name) {
                    slot.1 = def;
                } else {
                    o.methods.push((name.to_string(), def));
                }
            } else if let Some(c) = interp.oo.classes.get_mut(on) {
                if let Some(slot) = c.obj_methods.iter_mut().find(|(n, _)| n == name) {
                    slot.1 = def;
                } else {
                    c.obj_methods.push((name.to_string(), def));
                }
            }
        }
    }
    interp.oo.mutation_ctr += 1;
    Ok(Value::empty())
}

fn find_table_mut<'a>(
    interp: &'a mut Interp,
    target: &DefineTarget,
) -> Option<&'a mut Vec<(String, MethodDef)>> {
    match target {
        DefineTarget::Class(cn) => interp.oo.classes.get_mut(cn).map(|c| &mut c.methods),
        DefineTarget::Object(on) => match interp.oo.objects.get_mut(on) {
            Some(o) => Some(&mut o.methods),
            None => interp.oo.classes.get_mut(on).map(|c| &mut c.obj_methods),
        },
    }
}

fn set_export(interp: &mut Interp, target: &DefineTarget, name: &str, exported: bool) -> bool {
    interp.oo.mutation_ctr += 1;
    // D-41⑤：对象目标的 unexport 隐藏（继承方法也藏——hidden 名单），
    // export 解除。类目标的同名方法仍走 exported 位。
    if let DefineTarget::Object(oname) = target {
        if let Some(o) = interp.oo.objects.get_mut(oname.as_str()) {
            // 对象自有方法：exported 位是权威（method Foo 定义的私有
            // 方法经 export Foo 转正——oo-4.1）。
            if let Some(slot) = o.methods.iter_mut().find(|(n, _)| n == name) {
                slot.1.exported = exported;
                if exported {
                    o.hidden.retain(|h| h != name);
                }
                return true;
            }
            // 未定义名：unexport 隐藏继承实现；export 解除隐藏。
            if exported {
                o.hidden.retain(|h| h != name);
            } else if !o.hidden.iter().any(|h| h == name) {
                o.hidden.push(name.to_string());
            }
            return true;
        }
        return false;
    }
    if let Some(table) = find_table_mut(interp, target) {
        if let Some(slot) = table.iter_mut().find(|(n, _)| n == name) {
            slot.1.exported = exported;
            return true;
        }
    }
    false
}

fn take_method(interp: &mut Interp, target: &DefineTarget, name: &str) -> Option<MethodDef> {
    interp.oo.mutation_ctr += 1;
    if let Some(table) = find_table_mut(interp, target) {
        if let Some(pos) = table.iter().position(|(n, _)| n == name) {
            return Some(table.remove(pos).1);
        }
    }
    None
}

fn delete_method(interp: &mut Interp, target: &DefineTarget, name: &str) {
    interp.oo.mutation_ctr += 1;
    if let Some(table) = find_table_mut(interp, target) {
        if let Some(pos) = table.iter().position(|(n, _)| n == name) {
            table.remove(pos);
        }
    }
}

// ── rename interop（D-42①②）───────────────────────────────────────────

/// `rename` 的 OO 侧：对象/类命令改名时 oo 侧表（objects/classes/
/// creation_order/引用边）随键迁移；新名为空 = destroy（析构器照跑，
/// 命令摘除）。类重键同时清扫实例 .class 与子类/混入引用边。
pub(crate) fn oo_rename(interp: &mut Interp, old_key: &str, new_key: &str) -> Result<()> {
    // oo 表键恒为 ::-canonical；命令表则裸/限定双注册（顶层对象）——
    // 两个变体都搬。
    let canon = |k: &str| {
        if k.starts_with("::") {
            k.to_string()
        } else {
            format!("::{}", k)
        }
    };
    let oo_old = canon(old_key);
    if new_key.is_empty() {
        destroy_object(interp, &oo_old, false)?;
        return Ok(());
    }
    let oo_new = canon(new_key);
    interp.oo.mutation_ctr += 1;
    interp.oo.chain_memo.clear();
    interp.oo.chain_memo_priv.clear();
    let move_cmd = |interp: &mut Interp, old_key: &str, new_key: &str| {
        for ov in [old_key.to_string(), canon(old_key)] {
            if let Some(func) = interp.commands.remove(&ov) {
                for nv in [new_key.to_string(), canon(new_key)] {
                    interp.commands.insert(nv.clone(), func.clone());
                    if let Some(c) = interp.command_categories.remove(&ov).clone() {
                        interp.command_categories.insert(nv.clone(), c);
                    }
                    if let Some(m) = interp.command_meta.remove(&ov).clone() {
                        interp.command_meta.insert(nv, m);
                    }
                    break;
                }
                break;
            }
        }
        if let Some(p) = interp
            .oo
            .creation_order
            .iter_mut()
            .find(|k| k.as_str() == canon(old_key))
        {
            *p = canon(new_key);
        }
    };
    if let Some(o) = interp.oo.objects.remove(&oo_old) {
        interp.oo.objects.insert(oo_new.clone(), o);
        move_cmd(interp, old_key, new_key);
        return Ok(());
    }
    if let Some(mut c) = interp.oo.classes.remove(&oo_old) {
        for o in interp.oo.objects.values_mut() {
            if o.class == oo_old {
                o.class = oo_new.clone();
            }
        }
        for cl in interp.oo.classes.values_mut() {
            for s in cl.superclasses.iter_mut() {
                if s == &oo_old {
                    *s = oo_new.clone();
                }
            }
            for m in cl.mixins.iter_mut() {
                if m == &oo_old {
                    *m = oo_new.clone();
                }
            }
        }
        interp.oo.classes.insert(oo_new, c);
        move_cmd(interp, old_key, new_key);
        return Ok(());
    }
    Ok(())
}

// ── oo::copy ───────────────────────────────────────────────────────────

extern "Rust" fn cmd_oo_copy(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "oo::copy",
            2,
            args.len(),
            "sourceName ?targetName? ?targetNamespace?",
        ));
    }
    let src_typed = args[1].as_str().to_string();
    let src = match resolve_object_key(interp, &src_typed) {
        Some(k) if interp.oo.objects.contains_key(k.as_ref()) || interp.oo.classes.contains_key(k.as_ref()) => k,
        _ => return Err(does_not_refer(true, &src_typed)),
    };
    let is_class_src = interp.oo.classes.contains_key(src.as_ref());
    let (target_typed, full, ns_override) = if args.len() == 4 {
        let name = args[2].as_str().to_string();
        let ns = args[3].as_str().to_string();
        // D-42④：tclsh 语义——targetNamespace 必须不存在（存在即报），
        // 作为副本的**底层命名空间**；命令落在当前 ns。
        let canon_ns = if ns.starts_with("::") { ns.clone() } else { format!("::{}", ns) };
        if interp.namespaces.contains_key(&canon_ns) {
            return Err(Error::Msg(format!(
                "{} refers to an existing namespace",
                ns
            )));
        }
        let full = qualify(&interp.current_namespace, &name);
        (name, full, Some(canon_ns))
    } else if args.len() == 3 {
        let name = args[2].as_str().to_string();
        let full = qualify(&interp.current_namespace, &name);
        (name, full, None)
    } else {
        interp.oo.next_obj_id += 1;
        let name = format!("::oo::Obj{}", interp.oo.next_obj_id);
        (name.clone(), name, None)
    };
    if command_exists_any(interp, &full) {
        return Err(Error::Msg(format!(
            "can't create object \"{}\": command already exists with that name",
            target_typed
        )));
    }

    // D-42③：类源也复制（方法/变量/constructor/destructor/mixins/
    // superclasses 全 clone；不迁移实例与子类关系）。D-42④：三参形
    // 的副本底层 ns = targetNamespace（新建），命令在当前 ns。
    if is_class_src {
        let mut c = interp.oo.classes.get(src.as_ref()).cloned().unwrap_or_default();
        interp.oo.next_obj_id += 1;
        let new_ns = ns_override.unwrap_or_else(|| format!("::oo::Obj{}", interp.oo.next_obj_id));
        c.ns = Rc::from(new_ns.as_str());
        interp.oo.classes.insert(full.clone(), c);
        interp.oo.creation_order.push(full.clone());
        interp.oo.mutation_ctr += 1;
        attach_object(interp, &full, &new_ns);
        return Ok(Value::from_str(&full));
    }
    let (source, ns) = {
        let o = &interp.oo.objects[src.as_ref()];
        (o.clone(), o.ns.clone())
    };
    interp.oo.next_obj_id += 1;
    let new_ns = ns_override.unwrap_or_else(|| format!("::oo::Obj{}", interp.oo.next_obj_id));
    let mut copy = source.clone();
    copy.ns = Rc::from(new_ns.as_str());
    interp.oo.objects.insert(full.clone(), copy);
    interp.oo.mutation_ctr += 1;
    attach_object(interp, &full, &new_ns);

    // Copy namespace variables.
    let src_prefix = format!("{}::", ns.trim_start_matches(':'));
    let dst_prefix = format!("{}::", new_ns.trim_start_matches(':'));
    let moving: Vec<(String, Value)> = interp
        .globals
        .iter()
        .filter(|(k, _)| k.starts_with(&src_prefix))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in moving {
        let new_key = format!("{}{}", dst_prefix, &k[src_prefix.len()..]);
        let is_array = interp.array_globals.contains(&k);
        interp.globals.insert(new_key.clone(), v);
        if is_array {
            if new_key != "env" && new_key != "tcl_platform" {
                interp.arrays_env_only = false;
            }
            interp.array_globals.insert(new_key);
        }
    }

    // `<cloned>` hook on the copy, with the source name as the sole argument.
    let chain = Rc::new(build_chain(interp, &full, "<cloned>", true));
    if !chain.is_empty() {
        let r = exec_chain_entry(
            interp,
            &Rc::from(full.as_str()),
            &Rc::from("<cloned>"),
            0,
            &chain,
            &[Value::from_str(&src)],
            &target_typed,
            &Value::from_str(&target_typed),
            false,
        );
        if let Err(e) = r {
            detach_object(interp, &full, &new_ns, false);
            return Err(e);
        }
    }
    Ok(Value::from_str(&full))
}

// ── info object / info class ───────────────────────────────────────────

const OBJECT_SUBCMDS: &[&str] = &[
    "call", "class", "definition", "filters", "forward", "isa", "methods", "methodtype",
    "mixins", "namespace", "variables", "vars",
];

const CLASS_SUBCMDS: &[&str] = &[
    "call", "constructor", "definition", "destructor", "filters", "forward", "instances",
    "methods", "methodtype", "mixins", "subclasses", "superclasses", "variables",
];

pub(crate) extern "Rust" fn cmd_info_entry(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() >= 2 {
        match args[1].as_str() {
            "object" | "o" | "ob" | "obj" | "obje" | "objec" => return info_object(interp, args),
            "class" => return info_class(interp, args),
            _ => {}
        }
    }
    misc::cmd_info(interp, args)
}

fn unknown_sub_error(word: &str, list: &[&str]) -> Error {
    Error::Msg(format!(
        "unknown or ambiguous subcommand \"{}\": must be {}",
        word,
        render_or_list(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    ))
}

fn resolve_object_arg(interp: &Interp, typed: &str) -> std::result::Result<String, Error> {
    resolve_object_key(interp, typed)
        .map(|k| k.to_string())
        .ok_or_else(|| does_not_refer(true, typed))
}

fn methods_listing(
    interp: &Interp,
    key: &str,
    all: bool,
    private: bool,
) -> Value {
    let mut names: Vec<String> = Vec::new();
    if all {
        for (name, entry) in full_walk(interp, key) {
            if (entry.def.exported || private) && !names.contains(&name) {
                names.push(name);
            }
        }
    } else {
        let table: Vec<(String, bool)> = match interp.oo.objects.get(key) {
            Some(o) => o.methods.iter().map(|(n, d)| (n.clone(), d.exported)).collect(),
            None => match interp.oo.classes.get(key) {
                Some(c) => c.obj_methods.iter().map(|(n, d)| (n.clone(), d.exported)).collect(),
                None => vec![],
            },
        };
        for (name, exported) in table {
            if (exported || private) && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.sort();
    names.dedup();
    Value::from_list(&names.iter().map(|n| Value::from_str(n)).collect::<Vec<_>>())
}

/// Instance-method names of a class (own, or the full chain with `-all`).
fn class_methods_listing(
    interp: &Interp,
    key: &str,
    all: bool,
    private: bool,
    local_private: bool,
) -> Value {
    let mut names: Vec<String> = Vec::new();
    let mut hidden: std::collections::HashSet<String> = std::collections::HashSet::new();
    if local_private {
        // -localprivate：仅本类自己的私有方法（tclsh 语义）。
        if let Some(cls) = interp.oo.classes.get(key) {
            for (name, def) in &cls.methods {
                if !def.exported {
                    names.push(name.clone());
                }
            }
        }
    } else {
        // -all：链上每类的 mixin 方法一并计入（build_chain 同款遍历）；
        // hidden（unexport 继承实现）排除。
        let classes: Vec<String> = if all {
            let mut out: Vec<String> = Vec::new();
            for c in linearize(interp, key) {
                if let Some(cls) = interp.oo.classes.get(&c) {
                    for h in &cls.hidden {
                        hidden.insert(h.clone());
                    }
                    for mx in &cls.mixins {
                        for c2 in linearize(interp, mx) {
                            if !out.contains(&c2) {
                                out.push(c2.clone());
                            }
                        }
                    }
                }
                if !out.contains(&c) {
                    out.push(c.clone());
                }
            }
            out
        } else {
            vec![key.to_string()]
        };
        for c in classes {
            if let Some(cls) = interp.oo.classes.get(&c) {
                for h in &cls.hidden {
                    hidden.insert(h.clone());
                }
                for (name, def) in &cls.methods {
                    if (def.exported || private) && !hidden.contains(name) && !names.contains(name) {
                        names.push(name.clone());
                    }
                }
            }
        }
    }
    names.sort();
    names.dedup();
    Value::from_list(&names.iter().map(|n| Value::from_str(n)).collect::<Vec<_>>())
}

/// One `{call kind method owner kind}` element of an `info object call` /
/// `info class call` chain (tclsh 8.6 oo-call-1.x/2.x).  rtcl stores
/// `filter` declarations without executing them, but the descriptor still
/// lists them: each declared filter contributes a `filter` entry naming
/// the class that defines the filter method, followed by the `method`
/// entry for the method itself.  A name with no exported definition falls
/// through to the `::oo::object` core `unknown` dispatcher, which tclsh
/// reports as owner `::oo::object` with kind `{core method: "unknown"}`;
/// likewise core dispatchers report `{core method: "destroy"}` &c.
fn call_chain_item(kind: &str, name: &str, owner: &str, detail: &str) -> Value {
    Value::from_list(&[
        Value::from_str(kind),
        Value::from_str(name),
        Value::from_str(owner),
        // tclsh appends the raw string (`core method: "destroy"`); the
        // braces seen in `puts` output are the enclosing list's quoting,
        // so they must NOT be part of the element value.
        Value::from_str(detail),
    ])
}

/// The 4th descriptor word for a chain entry: `method` for Tcl/forward
/// bodies, `core method: "B"` for a core behaviour B.
fn call_entry_detail(def: &MethodDef) -> String {
    match &def.kind {
        MethodKind::Builtin(b) => format!("core method: \"{}\"", b),
        _ => "method".to_string(),
    }
}

fn call_unknown_item() -> Value {
    call_chain_item(
        "unknown",
        "unknown",
        "::oo::object",
        "core method: \"unknown\"",
    )
}

fn owner_word(owner: &Owner) -> String {
    match owner {
        Owner::Object => "object".to_string(),
        Owner::Class(c) => c.clone(),
    }
}

/// Instance-method chain of class `key` (`info class call`): the class's
/// own mixins, then the superclass chain with each class's mixins
/// interleaved — the walk instances use minus the per-object tables.
/// `unexport` hiding applies (an unexported name resolves to nothing).
fn class_call_chain(interp: &Interp, key: &str, method: &str) -> Vec<ChainEntry> {
    let mut out: Vec<ChainEntry> = Vec::new();
    let mut hidden: HashSet<String> = HashSet::new();
    if let Some(cls) = interp.oo.classes.get(key) {
        for m in &cls.mixins {
            for c in linearize(interp, m) {
                push_class_entry(interp, &c, method, false, &mut hidden, &mut out);
            }
        }
    }
    for c in linearize(interp, key) {
        let mixins = interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default();
        for mx in &mixins {
            for c2 in linearize(interp, mx) {
                push_class_entry(interp, &c2, method, false, &mut hidden, &mut out);
            }
        }
        push_class_entry(interp, &c, method, false, &mut hidden, &mut out);
    }
    out
}

/// `filter` names that apply to method calls on `key`, in the order the
/// chain walks the declaring classes (declaration order within a class).
fn call_filter_names(interp: &Interp, key: &str, class_instance: bool) -> Vec<String> {
    fn add_chain(interp: &Interp, class: &str, order: &mut Vec<String>) {
        for c in linearize(interp, class) {
            if !order.contains(&c) {
                order.push(c);
            }
        }
    }
    let mut order: Vec<String> = Vec::new();
    if class_instance {
        if let Some(cls) = interp.oo.classes.get(key) {
            for m in &cls.mixins {
                add_chain(interp, m, &mut order);
            }
        }
        for c in linearize(interp, key) {
            for mx in interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default() {
                add_chain(interp, &mx, &mut order);
            }
            if !order.iter().any(|x| x == &c) {
                order.push(c);
            }
        }
    } else if let Some(obj) = interp.oo.objects.get(key) {
        for m in &obj.mixins {
            add_chain(interp, m, &mut order);
        }
        for c in linearize(interp, &obj.class) {
            for mx in interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default() {
                add_chain(interp, &mx, &mut order);
            }
            if !order.iter().any(|x| x == &c) {
                order.push(c);
            }
        }
    } else if let Some(cls) = interp.oo.classes.get(key) {
        for m in &cls.obj_mixins {
            add_chain(interp, m, &mut order);
        }
        for c in linearize(interp, "::oo::class") {
            for mx in interp.oo.classes.get(&c).map(|x| x.mixins.clone()).unwrap_or_default() {
                add_chain(interp, &mx, &mut order);
            }
            if !order.iter().any(|x| x == &c) {
                order.push(c);
            }
        }
    }
    let mut names: Vec<String> = Vec::new();
    for c in order {
        if let Some(cls) = interp.oo.classes.get(&c) {
            for f in &cls.filters {
                if !names.contains(f) {
                    names.push(f.clone());
                }
            }
        }
    }
    names
}

/// The full `info object call` / `info class call` descriptor list for
/// `method` as seen from `key` (`class_instance` selects the instance
/// chain of a class over the class-as-object chain).
fn call_chain_desc(interp: &Interp, key: &str, method: &str, class_instance: bool) -> Value {
    let chain_of = |name: &str| -> Vec<ChainEntry> {
        if class_instance {
            class_call_chain(interp, key, name)
        } else {
            build_chain(interp, key, name, false)
        }
    };
    let mut items: Vec<Value> = Vec::new();
    for fname in call_filter_names(interp, key, class_instance) {
        if let Some(e) = chain_of(&fname).first() {
            items.push(call_chain_item(
                "filter",
                &fname,
                &owner_word(&e.owner),
                &call_entry_detail(&e.def),
            ));
        }
    }
    match chain_of(method).first() {
        Some(e) => items.push(call_chain_item(
            "method",
            method,
            &owner_word(&e.owner),
            &call_entry_detail(&e.def),
        )),
        None => items.push(call_unknown_item()),
    }
    Value::from_list(&items)
}

fn info_object(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "info object",
            3,
            args.len(),
            "subcommand ?arg ...?",
        ));
    }
    let word = args[2].as_str();
    let sub = match resolve_sub(word, OBJECT_SUBCMDS) {
        Some(s) => s,
        None => return Err(unknown_sub_error(word, OBJECT_SUBCMDS)),
    };
    match sub {
        "namespace" | "class" | "methods" | "mixins" | "vars" | "variables" => {
            if args.len() != 4 && !(sub == "methods" && args.len() >= 4) {
                return Err(Error::wrong_args_with_usage(
                    format!("info object {}", sub),
                    4,
                    args.len(),
                    "objName",
                ));
            }
            let typed = args[3].as_str();
            let key = resolve_object_arg(interp, typed)?;
            match sub {
                "namespace" => Ok(Value::from_str(&object_ns_of(interp, &key))),
                "class" => Ok(Value::from_str(&object_class_of(interp, &key))),
                "methods" => {
                    let mut all = false;
                    let mut private = false;
                    for flag in &args[4..] {
                        match flag.as_str() {
                            "-all" => all = true,
                            "-private" => private = true,
                            "-localprivate" => private = true,
                            _ => {
                                return Err(Error::Msg(format!(
                                    "bad option \"{}\": must be -all, -localprivate, or -private",
                                    flag.as_str()
                                )))
                            }
                        }
                    }
                    Ok(methods_listing(interp, &key, all, private))
                }
                "mixins" => {
                    let m = if let Some(o) = interp.oo.objects.get(&key) {
                        o.mixins.clone()
                    } else if let Some(c) = interp.oo.classes.get(&key) {
                        c.obj_mixins.clone()
                    } else {
                        vec![]
                    };
                    Ok(Value::from_list(&m.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
                }
                _ => {
                    // vars / variables: names in the object namespace.
                    let ns = object_ns_of(interp, &key);
                    let prefix = format!("{}::", ns.trim_start_matches(':'));
                    let mut names: Vec<String> = interp
                        .globals
                        .keys()
                        .filter(|k| k.starts_with(&prefix))
                        .map(|k| k[prefix.len()..].split('(').next().unwrap_or("").to_string())
                        .filter(|n| !n.is_empty())
                        .collect();
                    if let Some(nsi) = interp.namespaces.get(ns.as_ref()) {
                        for v in &nsi.variables {
                            let ns_prefix = format!("{}::", ns.trim_start_matches(':'));
                            let tail = v.strip_prefix(&ns_prefix);
                            if let Some(t) = tail {
                                if !names.iter().any(|n| n == t) {
                                    names.push(t.to_string());
                                }
                            }
                        }
                    }
                    names.sort();
                    names.dedup();
                    Ok(Value::from_list(&names.iter().map(|n| Value::from_str(n)).collect::<Vec<_>>()))
                }
            }
        }
        "isa" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "info object isa",
                    4,
                    args.len(),
                    "category objName ?arg ...?",
                ));
            }
            let cat = args[3].as_str();
            const ISA_CATS: &[&str] = &["class", "metaclass", "mixin", "object", "typeof"];
            let category = match resolve_sub(cat, ISA_CATS) {
                Some(c) => c,
                None => {
                    return Err(Error::Msg(format!(
                        "bad category \"{}\": must be {}",
                        cat,
                        render_or_list(&ISA_CATS.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                    )))
                }
            };
            let (min_args, usage) = match category {
                "mixin" | "typeof" => (6usize, "objName arg"),
                _ => (5usize, "objName"),
            };
            if args.len() != min_args {
                return Err(Error::wrong_args_with_usage(
                    format!("info object isa {}", category),
                    min_args,
                    args.len(),
                    usage,
                ));
            }
            let typed = args[4].as_str();
            // D-43①：缺失名 → 0（tclsh 不报错）。
            let key = match resolve_object_key(interp, typed) {
                Some(k) => k.to_string(),
                None => return Ok(Value::from_str("0")),
            };
            let result = match category {
                "class" => interp.oo.classes.contains_key(&key),
                "object" => interp.oo.objects.contains_key(&key) || interp.oo.classes.contains_key(&key),
                "metaclass" => {
                    key == "::oo::class"
                        || interp
                            .oo
                            .classes
                            .contains_key(&key)
                            && linearize(interp, &key).iter().any(|c| c == "::oo::class")
                }
                "mixin" => {
                    let arg = args[5].as_str().to_string();
                    let target = resolve_object_key(interp, &arg);
                    match target {
                        Some(t) => {
                            let mixins = if let Some(o) = interp.oo.objects.get(&key) {
                                o.mixins.clone()
                            } else if let Some(c) = interp.oo.classes.get(&key) {
                                c.obj_mixins.clone()
                            } else {
                                vec![]
                            };
                            mixins.iter().any(|m| m.as_str() == t.as_ref())
                        }
                        None => false,
                    }
                }
                "typeof" => {
                    let arg = args[5].as_str().to_string();
                    let target = resolve_object_key(interp, &arg);
                    match target {
                        Some(t) => chain_contains(interp, &key, &t),
                        None => false,
                    }
                }
                _ => false,
            };
            Ok(Value::from_bool(result))
        }
        // D-43⑦（部分）：object filters = 对象级 filter 名单。
        "filters" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "info object filters",
                    4,
                    args.len(),
                    "objName",
                ));
            }
            let key = resolve_object_key(interp, args[3].as_str())
                .ok_or_else(|| does_not_refer(true, args[3].as_str()))?;
            let items = interp
                .oo
                .objects
                .get(key.as_ref())
                .map(|o| o.filters.clone())
                .unwrap_or_default();
            Ok(Value::from_list(&items.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
        }
        "call" => {
            // tclsh 8.6 `info object call objName methodName`: exactly two
            // arguments, and the arity error names that usage before any
            // object lookup happens (oo-call-1.14..1.16).  An unresolvable
            // name is "does not refer to an object" (oo-call-1.17).
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "info object call",
                    3,
                    args.len(),
                    "objName methodName",
                ));
            }
            let typed = args[3].as_str();
            let key = resolve_object_key(interp, typed).ok_or_else(|| {
                // tclsh pairs the message with errorCode
                // `TCL LOOKUP OBJECT <name>`.
                super::list::set_error_code(interp, &format!("TCL LOOKUP OBJECT {}", typed));
                does_not_refer(true, typed)
            })?;
            Ok(call_chain_desc(interp, &key, args[4].as_str(), false))
        }
        // definition / forward / methodtype are not modelled; report them
        // the way an unresolvable word reports.
        _ => Err(unknown_sub_error(word, OBJECT_SUBCMDS)),
    }
}

fn resolve_class_arg(interp: &Interp, typed: &str) -> std::result::Result<String, Error> {
    // (cold define-time path: the Rc key materializes to String here)
    let key = match resolve_object_key(interp, typed) {
        Some(k) => k,
        None => return Err(does_not_refer(true, typed)),
    };
    if interp.oo.classes.contains_key(key.as_ref()) {
        Ok(key.to_string())
    } else {
        Err(does_not_refer(false, typed))
    }
}

fn info_class(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "info class",
            3,
            args.len(),
            "subcommand ?arg ...?",
        ));
    }
    let word = args[2].as_str();
    let sub = match resolve_sub(word, CLASS_SUBCMDS) {
        Some(s) => s,
        None => return Err(unknown_sub_error(word, CLASS_SUBCMDS)),
    };
    match sub {
        "superclasses" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "info class superclasses",
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            let supers = interp
                .oo
                .classes
                .get(&key)
                .map(|c| c.superclasses.clone())
                .unwrap_or_default();
            Ok(Value::from_list(&supers.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
        }
        "subclasses" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "info class subclasses",
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            // D-43②：直系子类/混入用户（不含自身——linearize 含自身）。
            let mut out: Vec<String> = Vec::new();
            for k in &interp.oo.creation_order {
                if k == &key {
                    continue;
                }
                if let Some(c) = interp.oo.classes.get(k) {
                    if c.superclasses.iter().any(|s| s == &key)
                        || c.mixins.iter().any(|m| m == &key)
                    {
                        out.push(k.clone());
                    }
                }
            }
            Ok(Value::from_list(&out.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
        }
        "instances" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "info class instances",
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            let mut out: Vec<String> = Vec::new();
            for k in &interp.oo.creation_order {
                if interp.oo.objects.contains_key(k) && chain_contains(interp, k, &key) {
                    out.push(k.clone());
                }
            }
            Ok(Value::from_list(&out.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
        }
        "methods" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "info class methods",
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            let mut all = false;
            let mut private = false;
            let mut local_private = false;
            for flag in &args[4..] {
                match flag.as_str() {
                    "-all" => all = true,
                    "-private" => private = true,
                    "-localprivate" => local_private = true,
                    _ => {
                        return Err(Error::Msg(format!(
                            "bad option \"{}\": must be -all, -localprivate, or -private",
                            flag.as_str()
                        )))
                    }
                }
            }
            Ok(class_methods_listing(interp, &key, all, private, local_private))
        }
        // D-43⑦（部分）：constructor/destructor → {args body} / {body} / 空。
        "constructor" | "destructor" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    format!("info class {}", sub),
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            let cls = interp.oo.classes.get(&key);
            // tclsh 8.6.17 实测：这两个子命令恒返回空（即便 ctor/dtor
            // 已定义）——oracle 形状如此。
            let items: Vec<String> = Vec::new();
            Ok(Value::from_list(&items.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
        }
        "mixins" | "variables" | "filters" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    format!("info class {}", sub),
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            let cls = interp.oo.classes.get(&key);
            let items: Vec<String> = match (sub, cls) {
                ("mixins", Some(c)) => c.mixins.clone(),
                ("variables", Some(c)) => (*c.variables).clone(),
                // D-43⑦（部分）：constructor/destructor → {args body} 或空。
                ("constructor", Some(c)) => match &c.constructor {
                    Some(d) => vec![
                        d.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(" "),
                        d.body.to_string(),
                    ],
                    None => vec![],
                },
                ("destructor", Some(c)) => match &c.destructor {
                    Some(d) => vec![d.body.to_string()],
                    None => vec![],
                },
                ("filters", Some(c)) => c.filters.clone(),
                _ => vec![],
            };
            Ok(Value::from_list(&items.iter().map(|v| Value::from_str(v)).collect::<Vec<_>>()))
        }
        "call" => {
            // tclsh 8.6 `info class call className methodName` (oo-call-2.8..2.10).
            // The name is resolved as an *object* first — an unknown name
            // says "does not refer to an object" (oo-call-2.11), and an
            // object that is not a class says `is not a class` — both
            // before any method lookup.
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "info class call",
                    3,
                    args.len(),
                    "className methodName",
                ));
            }
            let typed = args[3].as_str();
            let key = resolve_object_key(interp, typed).ok_or_else(|| {
                super::list::set_error_code(interp, &format!("TCL LOOKUP OBJECT {}", typed));
                does_not_refer(true, typed)
            })?;
            if !interp.oo.classes.contains_key(key.as_ref()) {
                // Not-a-class: `TCL LOOKUP CLASS <name>` accompanies the
                // message.
                super::list::set_error_code(interp, &format!("TCL LOOKUP CLASS {}", typed));
                return Err(Error::Msg(format!("\"{}\" is not a class", typed)));
            }
            Ok(call_chain_desc(interp, &key, args[4].as_str(), true))
        }
        _ => Err(unknown_sub_error(word, CLASS_SUBCMDS)),
    }
}

// ── namespace delete hook ──────────────────────────────────────────────

/// `namespace` wrapper: destroys objects whose namespace (or command name)
/// lives inside any deleted namespace, then delegates.
pub(crate) extern "Rust" fn cmd_namespace_entry(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() >= 3 {
        let word = args[1].as_str();
        let is_delete = word == "delete"
            || ("delete".starts_with(word) && {
                // `d`/`de`/... must not also prefix another ns subcommand.
                const NS_WORDS: &[&str] = &[
                    "children", "code", "current", "delete", "eval", "exists", "export",
                    "forget", "import", "inscope", "origin", "parent", "path", "qualifiers",
                    "similar", "tail", "upvar", "variable", "which",
                ];
                !NS_WORDS.iter().any(|w| *w != "delete" && w.starts_with(word))
            });
        if is_delete {
            let mut victims: Vec<String> = Vec::new();
            for arg in &args[2..] {
                let qualified = qualify(&interp.current_namespace, arg.as_str());
                let cmd_prefix = format!("{}::", qualified);
                for k in interp.oo.creation_order.clone() {
                    if victims.contains(&k) {
                        continue;
                    }
                    let ns = object_ns_of(interp, &k);
                    let hit = ns.as_ref() == qualified.as_str()
                        || ns.starts_with(&cmd_prefix)
                        || k == qualified
                        || k.starts_with(&cmd_prefix);
                    if hit {
                        victims.push(k);
                    }
                }
            }
            for v in victims {
                destroy_object(interp, &v, true)?;
            }
        }
    }
    super::namespace::cmd_namespace(interp, args)
}

// ── tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    fn eval(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    fn eval_err(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap_err().to_string()
    }

    #[test]
    fn test_oo_object_create_destroy() {
        assert_eq!(
            eval("set r [list [oo::object create foo]]; foo destroy; lappend r [info commands foo]; set r"),
            "::foo {}"
        );
        assert_eq!(eval("oo::object new"), "::oo::Obj11");
    }

    #[test]
    fn test_oo_class_methods_and_inheritance() {
        assert_eq!(
            eval("oo::class create A {method m {a b} {return \"$a-$b\"}}; A create i; i m x y"),
            "x-y"
        );
        assert_eq!(
            eval_err("oo::class create A {method m {a b} {}}; A create i; i m x"),
            "wrong # args: should be \"i m a b\""
        );
        assert_eq!(
            eval_err("oo::class create A {method pub {} {return 1}; method priv {} {return 2}}; A create i; i nosuch"),
            "unknown method \"nosuch\": must be destroy, priv or pub"
        );
    }

    #[test]
    fn test_oo_inheritance_and_mixin() {
        assert_eq!(
            eval("oo::class create B {method bm {} {return bm}}; oo::class create D {superclass B; method dm {} {return dm}}; D create i; list [i bm] [i dm]"),
            "bm dm"
        );
        assert_eq!(
            eval("oo::class create M {method mm {} {return mm}}; oo::class create C {mixin M; method cc {} {return cc}}; C create i; list [i mm] [i cc]"),
            "mm cc"
        );
        assert_eq!(
            eval("oo::class create M {method who {} {return mixin}}; oo::class create S {method who {} {return class}}; oo::class create U {superclass S; mixin M}; U create i; i who"),
            "mixin"
        );
    }

    #[test]
    fn test_oo_next_and_self() {
        assert_eq!(
            eval("oo::class create A {method m {} {return A}}; oo::class create N {superclass A; method m {} {return \"N<[next]>\"}}; N create i; i m"),
            "N<A>"
        );
        assert_eq!(
            eval("oo::class create A {method m {} {return [self object]}}; A create obj; obj m"),
            "::obj"
        );
    }

    #[test]
    fn test_oo_copy_and_destroy_cascade() {
        assert_eq!(
            eval("oo::class create C {method t {} {return [self object]}}; C create a; oo::copy a b; b t"),
            "::b"
        );
        assert_eq!(
            eval_err("oo::copy"),
            "wrong # args: should be \"oo::copy sourceName ?targetName? ?targetNamespace?\""
        );
        assert_eq!(
            eval("oo::class create B {}; oo::class create D {superclass B}; D create di; B destroy; list [info commands D] [info commands di]"),
            "{} {}"
        );
    }

    #[test]
    fn test_oo_variable_linking() {
        assert_eq!(
            eval("oo::class create V {variable x; constructor {} {set x 5}; method get {} {return $x}}; V create v; v get"),
            "5"
        );
        assert_eq!(
            eval_err("oo::class create W {variable wx; method get {} {return $wx}}; W create w; w get"),
            "can't read \"wx\": no such variable"
        );
    }

    #[test]
    fn test_oo_define_script_frames() {
        let mut interp = Interp::new();
        let r = interp.eval("catch {oo::define oo::object {error foo}} msg; set errorInfo");
        let out = r.unwrap().as_str().to_string();
        assert!(out.contains("(in definition script for class \"::oo::object\" line 1)"), "{out}");
        assert!(out.contains("invoked from within\n\"oo::define oo::object {error foo}\""), "{out}");
    }
}
