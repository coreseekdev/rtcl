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

/// A method definition (own methods, forwards, constructors, destructors).
#[derive(Debug, Clone)]
pub(crate) struct MethodDef {
    pub params: Vec<(String, Option<String>)>,
    pub body: String,
    pub exported: bool,
    pub kind: MethodKind,
    /// Compiled form of the assembled method [`ProcDef`], memoised for the
    /// no-`variable` case (see `memoised_proc_def`): `(typed method name,
    /// ProcDef)`.  `Rc<RefCell<..>>` so chain clones share one cell; a
    /// redefinition installs a fresh `MethodDef` (fresh cell), retiring the
    /// memo with its inputs.
    pub proc_memo: Rc<RefCell<Option<(String, Rc<ProcDef>)>>>,
}

/// A fresh [`MethodDef::proc_memo`] cell.
fn fresh_memo() -> Rc<RefCell<Option<(String, Rc<ProcDef>)>>> {
    Rc::new(RefCell::new(None))
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

/// One resolvable link in a method chain.
#[derive(Debug, Clone)]
pub(crate) struct ChainEntry {
    pub owner: Owner,
    pub def: MethodDef,
}

/// A plain object.
#[derive(Debug, Default, Clone)]
pub(crate) struct OoObject {
    /// Per-object namespace (`::oo::ObjN`).
    pub ns: String,
    /// Canonical class key.
    pub class: String,
    /// Per-object mixins (canonical keys).
    pub mixins: Vec<String>,
    /// Per-object methods in definition order.
    pub methods: Vec<(String, MethodDef)>,
    /// `variable` declarations (definition order).
    pub variables: Vec<String>,
}

/// A class (both the behaviour of its instances and, via `obj_*`, the
/// class-as-object's own per-object definitions).
#[derive(Debug, Default, Clone)]
pub(crate) struct OoClass {
    pub ns: String,
    pub superclasses: Vec<String>,
    /// Class-level mixins.
    pub mixins: Vec<String>,
    /// Methods of *instances* of this class.
    pub methods: Vec<(String, MethodDef)>,
    pub variables: Vec<String>,
    /// Names hidden by `unexport` of a method this class does not itself
    /// define (hides the inherited implementation).
    pub hidden: Vec<String>,
    pub constructor: Option<MethodDef>,
    pub destructor: Option<MethodDef>,
    /// `filter` names (stored; not executed).
    pub filters: Vec<String>,
    // ── the class object's own per-object definitions (oo::objdefine) ──
    pub obj_methods: Vec<(String, MethodDef)>,
    pub obj_variables: Vec<String>,
    pub obj_mixins: Vec<String>,
}

/// A method execution in progress (`self`/`my`/`next` context).
#[derive(Debug, Clone)]
pub(crate) struct ActiveMethod {
    pub obj: String,
    pub method: String,
    pub chain: Vec<ChainEntry>,
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

/// All OO state, hanging off [`Interp`].
#[derive(Debug, Default)]
pub(crate) struct OoState {
    pub next_obj_id: usize,
    pub objects: HashMap<String, OoObject>,
    pub classes: HashMap<String, OoClass>,
    /// Objects and classes in creation order (listing/cascade order).
    pub creation_order: Vec<String>,
    pub active: Vec<ActiveMethod>,
    pub define_stack: Vec<DefineTarget>,
    /// `my`/`self`/`next` command keys → owning object key.  Entries whose
    /// key no longer holds such a command were renamed away and are swept
    /// when their owner is destroyed (oo-1.20).
    pub my_commands: HashMap<String, String>,
}

// ── init ───────────────────────────────────────────────────────────────

fn builtin_method(name: &'static str) -> MethodDef {
    MethodDef { params: Vec::new(), body: String::new(), exported: true, kind: MethodKind::Builtin(name), proc_memo: fresh_memo() }
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
    let mut root = OoClass { ns: format!("::oo::Obj{}", id), ..Default::default() };
    root.methods.push(("destroy".to_string(), builtin_method("destroy")));
    interp.oo.classes.insert("::oo::object".to_string(), root);
    interp.oo.creation_order.push("::oo::object".to_string());

    // ::oo::class — the metaclass.
    let id = interp.oo.next_obj_id + 1;
    interp.oo.next_obj_id = id;
    let mut metaclass =
        OoClass { ns: format!("::oo::Obj{}", id), superclasses: vec!["::oo::object".to_string()], ..Default::default() };
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
}

// ── namespace helpers ──────────────────────────────────────────────────

/// Ensure a namespace and its ancestors exist (local copy of the namespace
/// module's `ensure_namespace`, which is private).
fn ensure_ns(namespaces: &mut HashMap<String, super::namespace::NamespaceInfo>, qualified: &str) {
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
fn resolve_object_key(interp: &Interp, typed: &str) -> Option<String> {
    for cand in object_key_candidates(interp, typed) {
        if interp.oo.objects.contains_key(&cand) || interp.oo.classes.contains_key(&cand) {
            return Some(cand);
        }
    }
    None
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
        if def.exported || include_private {
            if !out.iter().any(|e| e.owner == Owner::Class(class.to_string())) {
                out.push(ChainEntry { owner: Owner::Class(class.to_string()), def: def.clone() });
                hidden.remove(method);
            }
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
                out.push(ChainEntry { owner: Owner::Object, def: def.clone() });
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
                out.push(ChainEntry { owner: Owner::Object, def: def.clone() });
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
                out.push((name.clone(), ChainEntry { owner: Owner::Object, def: def.clone() }));
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
                out.push((name.clone(), ChainEntry { owner: Owner::Object, def: def.clone() }));
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
fn owner_variables(interp: &Interp, owner: &Owner, key: &str) -> Vec<String> {
    match owner {
        Owner::Object => match interp.oo.objects.get(key) {
            Some(o) => o.variables.clone(),
            None => match interp.oo.classes.get(key) {
                Some(c) => c.obj_variables.clone(),
                None => vec![],
            },
        },
        Owner::Class(c) => match interp.oo.classes.get(c) {
            Some(cls) => cls.variables.clone(),
            None => vec![],
        },
    }
}

// ── active-method context ──────────────────────────────────────────────

/// Topmost live active-method entry (dropping stale ones).
fn top_active(interp: &mut Interp) -> Option<ActiveMethod> {
    loop {
        match interp.oo.active.last() {
            Some(a) => {
                if interp.frames.len() > a.frame_depth {
                    return Some(a.clone());
                }
                interp.oo.active.pop();
            }
            None => return None,
        }
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
    let mut names: Vec<String> = Vec::new();
    for (name, entry) in full_walk(interp, key) {
        if entry.def.exported && !names.contains(&name) {
            names.push(name);
        }
    }
    names.push("destroy".to_string());
    if key == "::oo::class" {
        names.retain(|n| n != "new");
    }
    names.sort();
    names.dedup();
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
fn exec_chain_entry(
    interp: &mut Interp,
    key: &str,
    method: &str,
    index: usize,
    chain: &[ChainEntry],
    call_args: &[Value],
    typed: &str,
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
            "new" => oo_new(interp, key, call_args),
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
            // Link the defining owner's `variable`s (skipping names bound
            // as parameters, including the synthetic method-name param).
            let vars = owner_variables(interp, &entry.owner, key);

            // tclsh compiles a method body once and caches the compiled
            // form on the method record; rtcl rebuilt this ProcDef per
            // call — two full-body String copies plus the bytecode
            // cache's full-text probe inside call_proc's eval.  An owner
            // with no `variable` declarations assembles to `def.body`
            // verbatim (a pure function of def + invoked method name),
            // so that case memoises on the MethodDef; owners WITH
            // variables keep the rebuild (the prefix follows the live
            // variable lists).
            let proc_def: Rc<ProcDef> = if vars.is_empty() {
                memoised_proc_def(interp, &entry.def, method)
            } else {
                let mut body = link_prefix(&vars, &entry.def.params);
                body.push_str(&entry.def.body);
                Rc::new(method_proc_def(&body, &entry.def.params, method))
            };

            let mut args: Vec<Value> =
                vec![Value::from_str(typed), Value::from_str(method)];
            args.extend_from_slice(call_args);

            let ns = object_ns_of(interp, key);
            interp.oo.active.push(ActiveMethod {
                obj: key.to_string(),
                method: method.to_string(),
                chain: chain.to_vec(),
                index,
                frame_depth: interp.frames.len(),
            });
            let r = interp.call_proc(&proc_def, &args, typed, Some(ns));
            cleanup_active(interp);
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
    if link.is_empty() {
        String::new()
    } else {
        format!("variable {};", link.join(" "))
    }
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

/// Memoised [`ProcDef`] for a method whose defining owner declares no
/// `variable`s: the assembled body is `def.body` verbatim, so
/// params/body/bytecode are pure functions of (def, method) — built and
/// compiled once, shared by every invocation (tclsh caches the compiled
/// body on its method record the same way).  Keyed by the typed method
/// name (params[0] feeds the arity-usage text and `info level 0`); a
/// mismatch or an empty memo just rebuilds.  Method (re)definition
/// installs a fresh `MethodDef`, retiring the memo with its inputs;
/// `export` toggling is the only in-place mutation and does not enter
/// the memo.  The shared `Rc<ProcDef>` is never written back: OO
/// statics are always empty, so `call_proc`'s `Rc::make_mut` write-back
/// cannot fire, and its per-call epoch/applicability gate keeps a stale
/// compiled form on the tree-walk exactly like a named proc's.
fn memoised_proc_def(interp: &mut Interp, def: &MethodDef, method: &str) -> Rc<ProcDef> {
    {
        let memo = def.proc_memo.borrow();
        if let Some((n, d)) = memo.as_ref() {
            if n == method {
                return Rc::clone(d);
            }
        }
    }
    let mut pd = method_proc_def(&def.body, &def.params, method);
    // Compile at the same seam named `proc`s use; the compiled form also
    // unlocks call_proc's slot-locals binding (empty statics, params
    // seeded in order — the synthetic name first, by construction).
    let compiled =
        super::super::vm_exec::compile_proc_body(&pd.params, &def.body, interp.tier1_epoch);
    if let Some(code) = &compiled {
        interp.const_pool_insert(code);
    }
    pd.compiled = compiled;
    let proc_def = Rc::new(pd);
    *def.proc_memo.borrow_mut() = Some((method.to_string(), Rc::clone(&proc_def)));
    proc_def
}

fn object_ns_of(interp: &Interp, key: &str) -> String {
    if let Some(o) = interp.oo.objects.get(key) {
        return o.ns.clone();
    }
    if let Some(c) = interp.oo.classes.get(key) {
        return c.ns.clone();
    }
    "::".to_string()
}

/// Invoke `method` on `key`, or produce the unknown-method error.
fn invoke_method(
    interp: &mut Interp,
    key: &str,
    method: &str,
    call_args: &[Value],
    typed: &str,
    include_private: bool,
) -> Result<Value> {
    let chain = build_chain(interp, key, method, include_private);
    if chain.is_empty() {
        return Err(unknown_method_error(interp, key, typed, method));
    }
    exec_chain_entry(interp, key, method, 0, &chain, call_args, typed)
}

// ── the object dispatcher ──────────────────────────────────────────────

extern "Rust" fn cmd_oo_object(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let typed = args[0].as_str().to_string();
    let key = match resolve_object_key(interp, &typed) {
        Some(k) => k,
        None => return Err(Error::invalid_command(&typed)),
    };
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            typed,
            2,
            args.len(),
            "method ?arg ...?",
        ));
    }
    let method = args[1].as_str().to_string();
    let call_args: Vec<Value> = args[2..].to_vec();
    invoke_method(interp, &key, &method, &call_args, &typed, false)
}

// ── my / self / next ───────────────────────────────────────────────────

extern "Rust" fn cmd_oo_my(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let typed = args[0].as_str().to_string();
    let cmd_key = qualify(&interp.current_namespace, &typed);
    let key = match interp.oo.my_commands.get(&cmd_key) {
        Some(k) => k.clone(),
        None => return Err(Error::invalid_command(&typed)),
    };
    let ns = object_ns_of(interp, &key);
    if !frame_in_object_ns(interp, &ns) {
        return Err(Error::invalid_command(&typed));
    }
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("my", 2, args.len(), "method ?arg ...?"));
    }
    let method = args[1].as_str().to_string();
    let call_args: Vec<Value> = args[2..].to_vec();
    invoke_method(interp, &key, &method, &call_args, &typed, true)
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
    let mut hits: Vec<&'a str> = list.iter().filter(|i| i.starts_with(word)).map(|i| *i).collect();
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
            let entry = &act.chain[act.index];
            Ok(Value::from_list(&[
                chain_desc(entry, &act.method),
                Value::from_int(act.index as i64),
            ]))
        }
        "next" => {
            let mut items = Vec::new();
            for e in act.chain.iter().skip(act.index + 1) {
                items.push(chain_desc(e, &act.method));
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
        None => return Err(Error::Msg("no next method implementation".to_string())),
    };
    let next_index = act.index + 1;
    if next_index >= act.chain.len() {
        return Err(Error::Msg("no next method implementation".to_string()));
    }
    let call_args: Vec<Value> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        // Re-pass the current invocation's arguments (level0 words minus
        // the object and method words).
        let l0 = interp
            .frames
            .last()
            .map(crate::interp::frame_level0)
            .unwrap_or_default();
        let words = Value::from_str(&l0).as_list().unwrap_or_default();
        if words.len() > 2 {
            words[2..].to_vec()
        } else {
            Vec::new()
        }
    };
    // The as-typed object word, as recorded in the current level0.
    let l0 = interp
        .frames
        .last()
        .map(crate::interp::frame_level0)
        .unwrap_or_default();
    let typed = Value::from_str(&l0)
        .as_list()
        .and_then(|l| l.first().map(|v| v.as_str().to_string()))
        .unwrap_or_else(|| act.obj.clone());
    exec_chain_entry(interp, &act.obj, &act.method, next_index, &act.chain, &call_args, &typed)
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
            let is_oo = **f as usize == cmd_oo_my as usize
                || **f as usize == cmd_oo_self as usize
                || **f as usize == cmd_oo_next as usize;
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
    interp.oo.creation_order.retain(|k| k != key);
    // Namespace-scoped storage.
    let var_prefix = format!("{}::", ns.trim_start_matches(':'));
    let globals: Vec<String> = interp
        .globals
        .keys()
        .filter(|k| k.starts_with(&var_prefix))
        .cloned()
        .collect();
    for g in globals {
        interp.globals.remove(&g);
        interp.array_globals.remove(&g);
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
    let is_meta = class_key == "::oo::class";
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
    let ns = next_ns(interp);
    if is_meta {
        interp.oo.classes.insert(
            canonical.clone(),
            OoClass {
                ns: ns.clone(),
                superclasses: vec!["::oo::object".to_string()],
                ..Default::default()
            },
        );
    } else {
        interp.oo.objects.insert(
            canonical.clone(),
            OoObject { ns: ns.clone(), class: class_key.to_string(), ..Default::default() },
        );
    }
    attach_object(interp, &canonical, &ns);

    // Definition script (metaclass create only).
    if is_meta && rest.len() > 1 {
        if rest.len() > 2 {
            let e = Error::wrong_args_with_usage(
                typed,
                2,
                2 + rest.len(),
                &format!("create {} ?definitionScript?", name_typed),
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

    // Constructor.
    if !is_meta {
        let ctor = interp
            .oo
            .classes
            .get(class_key)
            .and_then(|c| c.constructor.clone());
        if let Some(ctor) = ctor {
            let mut params: Vec<(String, Option<String>)> =
                vec![("constructor".to_string(), None)];
            params.extend(ctor.params.iter().cloned());
            let vars = interp
                .oo
                .classes
                .get(class_key)
                .map(|c| c.variables.clone())
                .unwrap_or_default();
            let body = format!("{}{}", link_prefix(&vars, &ctor.params), ctor.body);
            let proc_def = ProcDef {
                params: super::super::Rc::new(params),
                body: super::super::Rc::from(body),
                statics: super::super::Rc::new(HashMap::new()),
                compiled: None,
            };
            let mut args: Vec<Value> =
                vec![Value::from_str(&name_typed), Value::from_str("constructor")];
            args.extend_from_slice(&rest[1..]);
            interp.oo.active.push(ActiveMethod {
                obj: canonical.clone(),
                method: "constructor".to_string(),
                chain: ctor_chain(interp, class_key),
                index: 0,
                frame_depth: interp.frames.len(),
            });
            let r = interp.call_proc(&proc_def, &args, &name_typed, Some(ns.clone()));
            cleanup_active(interp);
            if let Err(e) = r {
                detach_object(interp, &canonical, &ns, false);
                return Err(e);
            }
        }
    }
    Ok(Value::from_str(&canonical))
}

fn oo_new(interp: &mut Interp, class_key: &str, rest: &[Value]) -> Result<Value> {
    let name = next_ns(interp);
    let ns = name.clone();
    interp.oo.objects.insert(
        name.clone(),
        OoObject { ns: ns.clone(), class: class_key.to_string(), ..Default::default() },
    );
    attach_object(interp, &name, &ns);

    let ctor = interp.oo.classes.get(class_key).and_then(|c| c.constructor.clone());
    if let Some(ctor) = ctor {
        let mut params: Vec<(String, Option<String>)> = vec![("constructor".to_string(), None)];
        params.extend(ctor.params.iter().cloned());
        let vars = interp
            .oo
            .classes
            .get(class_key)
            .map(|c| c.variables.clone())
            .unwrap_or_default();
        let body = format!("{}{}", link_prefix(&vars, &ctor.params), ctor.body);
        let proc_def = ProcDef {
            params: super::super::Rc::new(params),
            body: super::super::Rc::from(body),
            statics: super::super::Rc::new(HashMap::new()),
            compiled: None,
        };
        let typed = name.clone();
        let mut args: Vec<Value> = vec![Value::from_str(&typed), Value::from_str("constructor")];
        args.extend_from_slice(rest);
        interp.oo.active.push(ActiveMethod {
            obj: name.clone(),
            method: "constructor".to_string(),
            chain: ctor_chain(interp, class_key),
            index: 0,
            frame_depth: interp.frames.len(),
        });
        let r = interp.call_proc(&proc_def, &args, &typed, Some(ns.clone()));
        cleanup_active(interp);
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
                out.push(ChainEntry { owner: Owner::Class(c.clone()), def: def.clone() });
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
                out.push(ChainEntry { owner: Owner::Class(c.clone()), def: def.clone() });
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
        let class_key = interp.oo.objects.get(key).map(|o| o.class.clone()).unwrap_or_default();
        let chain = dtor_chain(interp, &class_key);
        for (i, entry) in chain.iter().enumerate() {
            let vars = match &entry.owner {
                Owner::Class(c) => interp
                    .oo
                    .classes
                    .get(c)
                    .map(|cls| cls.variables.clone())
                    .unwrap_or_default(),
                Owner::Object => vec![],
            };
            let body = format!("{}{}", link_prefix(&vars, &[]), entry.def.body);
            let proc_def = ProcDef {
                params: super::super::Rc::new(vec![("destructor".to_string(), None)]),
                body: super::super::Rc::from(body),
                statics: super::super::Rc::new(HashMap::new()), compiled: None,
            };
            let args = vec![Value::from_str(key), Value::from_str("destructor")];
            interp.oo.active.push(ActiveMethod {
                obj: key.to_string(),
                method: "destructor".to_string(),
                chain: chain.clone(),
                index: i,
                frame_depth: interp.frames.len(),
            });
            let r = interp.call_proc(&proc_def, &args, key, Some(object_ns_of(interp, key)));
            cleanup_active(interp);
            if let Err(e) = r {
                dtor_error = Some(e);
                break;
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
        Some(k) if interp.oo.classes.contains_key(&k) => k,
        Some(_) => return Err(does_not_refer(false, &typed)),
        None => return Err(does_not_refer(true, &typed)),
    };
    if args.len() == 3 {
        let script = args[2].as_str().to_string();
        run_define_script(interp, DefineTarget::Class(canonical), &script, true)
    } else {
        // Multi-word form: the words form a single command evaluated in the
        // definition context, with no definition-script error frame
        // (tclsh: `oo::define C error foo` reports only the outer frame).
        interp.oo.define_stack.push(DefineTarget::Class(canonical));
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
        run_define_script(interp, DefineTarget::Object(canonical), &script, false)
    } else {
        // Multi-word form — see cmd_oo_define.
        interp.oo.define_stack.push(DefineTarget::Object(canonical));
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
                params,
                body: rest[2].as_str().to_string(),
                exported: name_exports(&name),
                kind: MethodKind::Tcl,
                proc_memo: fresh_memo(),
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
                params: Vec::new(),
                body: String::new(),
                exported: name_exports(&name),
                kind: MethodKind::Forward {
                    cmd: rest[1].as_str().to_string(),
                    prefix: rest[2..].iter().map(|v| v.as_str().to_string()).collect(),
                },
                proc_memo: fresh_memo(),
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
                if !interp.oo.classes.contains_key(&canonical) {
                    if canonical == def_target_name(&target) {
                        return Err(Error::Msg("may not mix a class into itself".to_string()));
                    }
                    return Err(Error::Msg("may only mix in classes".to_string()));
                }
                if canonical == def_target_name(&target) {
                    return Err(Error::Msg("may not mix a class into itself".to_string()));
                }
                mixins.push(canonical);
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
                if !interp.oo.classes.contains_key(&canonical) {
                    return Err(Error::Msg("only a class can be a superclass".to_string()));
                }
                if canonical == target_name
                    || linearize(interp, &canonical).iter().any(|c| *c == target_name)
                {
                    return Err(Error::Msg(
                        "attempt to form circular dependency graph".to_string(),
                    ));
                }
                supers.push(canonical);
            }
            if let DefineTarget::Class(name) = target {
                if let Some(c) = interp.oo.classes.get_mut(&name) {
                    c.superclasses = supers;
                }
            }
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
                        params,
                        body: rest[1].as_str().to_string(),
                        exported: false,
                        kind: MethodKind::Tcl,
                        proc_memo: fresh_memo(),
                    });
                }
            }
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
                        params: Vec::new(),
                        body: rest[0].as_str().to_string(),
                        exported: false,
                        kind: MethodKind::Tcl,
                        proc_memo: fresh_memo(),
                    });
                }
            }
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
                                c.variables.push(name);
                            }
                        }
                    }
                    DefineTarget::Object(name2) => {
                        if let Some(o) = interp.oo.objects.get_mut(name2) {
                            if !o.variables.contains(&name) {
                                o.variables.push(name);
                            }
                        } else if let Some(c) = interp.oo.classes.get_mut(name2) {
                            if !c.obj_variables.contains(&name) {
                                c.obj_variables.push(name);
                            }
                        }
                    }
                }
            }
            Ok(Value::empty())
        }
        "filter" => {
            if let DefineTarget::Class(name) = target {
                if let Some(c) = interp.oo.classes.get_mut(&name) {
                    c.filters = rest.iter().map(|v| v.as_str().to_string()).collect();
                }
            }
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
    if let Some(table) = find_table_mut(interp, target) {
        if let Some(slot) = table.iter_mut().find(|(n, _)| n == name) {
            slot.1.exported = exported;
            return true;
        }
    }
    false
}

fn take_method(interp: &mut Interp, target: &DefineTarget, name: &str) -> Option<MethodDef> {
    if let Some(table) = find_table_mut(interp, target) {
        if let Some(pos) = table.iter().position(|(n, _)| n == name) {
            return Some(table.remove(pos).1);
        }
    }
    None
}

fn delete_method(interp: &mut Interp, target: &DefineTarget, name: &str) {
    if let Some(table) = find_table_mut(interp, target) {
        if let Some(pos) = table.iter().position(|(n, _)| n == name) {
            table.remove(pos);
        }
    }
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
        Some(k) if interp.oo.objects.contains_key(&k) => k,
        _ => return Err(does_not_refer(true, &src_typed)),
    };
    let (target_typed, full) = if args.len() == 4 {
        let name = args[2].as_str().to_string();
        let ns = args[3].as_str().to_string();
        let full = qualify(&ns, &name);
        (name, full)
    } else if args.len() == 3 {
        let name = args[2].as_str().to_string();
        let full = qualify(&interp.current_namespace, &name);
        (name, full)
    } else {
        interp.oo.next_obj_id += 1;
        let name = format!("::oo::Obj{}", interp.oo.next_obj_id);
        (name.clone(), name)
    };
    if command_exists_any(interp, &full) {
        return Err(Error::Msg(format!(
            "can't create object \"{}\": command already exists with that name",
            target_typed
        )));
    }

    let (source, ns) = {
        let o = &interp.oo.objects[&src];
        (o.clone(), o.ns.clone())
    };
    interp.oo.next_obj_id += 1;
    let new_ns = format!("::oo::Obj{}", interp.oo.next_obj_id);
    let mut copy = source.clone();
    copy.ns = new_ns.clone();
    interp.oo.objects.insert(full.clone(), copy);
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
            interp.array_globals.insert(new_key);
        }
    }

    // `<cloned>` hook on the copy, with the source name as the sole argument.
    let chain = build_chain(interp, &full, "<cloned>", true);
    if !chain.is_empty() {
        let r = exec_chain_entry(
            interp,
            &full,
            "<cloned>",
            0,
            &chain,
            &[Value::from_str(&src)],
            &target_typed,
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
    resolve_object_key(interp, typed).ok_or_else(|| does_not_refer(true, typed))
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
fn class_methods_listing(interp: &Interp, key: &str, all: bool, private: bool) -> Value {
    let mut names: Vec<String> = Vec::new();
    let classes: Vec<String> = if all {
        linearize(interp, key)
    } else {
        vec![key.to_string()]
    };
    for c in classes {
        if let Some(cls) = interp.oo.classes.get(&c) {
            for (name, def) in &cls.methods {
                if (def.exported || private) && !names.contains(name) {
                    names.push(name.clone());
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
            if !order.iter().any(|x| *x == c) {
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
                    &format!("info object {}", sub),
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
                            _ => {}
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
                    if let Some(nsi) = interp.namespaces.get(&ns) {
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
                    &format!("info object isa {}", category),
                    min_args,
                    args.len(),
                    usage,
                ));
            }
            let typed = args[4].as_str();
            let key = resolve_object_arg(interp, typed)?;
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
                            mixins.iter().any(|m| *m == t)
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
    let key = match resolve_object_key(interp, typed) {
        Some(k) => k,
        None => return Err(does_not_refer(true, typed)),
    };
    if interp.oo.classes.contains_key(&key) {
        Ok(key)
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
            let mut out: Vec<String> = Vec::new();
            for k in &interp.oo.creation_order {
                if interp.oo.classes.contains_key(k) && linearize(interp, k).iter().any(|c| c == &key) {
                    out.push(k.clone());
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
            for flag in &args[4..] {
                match flag.as_str() {
                    "-all" => all = true,
                    "-private" => private = true,
                    _ => {}
                }
            }
            Ok(class_methods_listing(interp, &key, all, private))
        }
        "mixins" | "variables" | "filters" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    &format!("info class {}", sub),
                    4,
                    args.len(),
                    "className",
                ));
            }
            let key = resolve_class_arg(interp, args[3].as_str())?;
            let cls = interp.oo.classes.get(&key);
            let items: Vec<String> = match (sub, cls) {
                ("mixins", Some(c)) => c.mixins.clone(),
                ("variables", Some(c)) => c.variables.clone(),
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
            if !interp.oo.classes.contains_key(&key) {
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
                    let hit = ns == qualified
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
use crate::interp::Rc;

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
        assert_eq!(eval("oo::object new"), "::oo::Obj3");
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
