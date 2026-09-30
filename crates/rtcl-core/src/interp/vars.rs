//! Variable access methods on [`Interp`].
//!
//! All variable reads/writes are scope-aware: at global level they use
//! `Interp::globals`; inside a proc they use the current `CallFrame`'s
//! locals (or follow upvar links to globals / other frames).

use super::util::split_array_ref;
use super::{Interp, UpvarLink};
use crate::error::{Error, ErrorCode, Result};
use crate::value::Value;

/// Where a resolved variable lives, after following upvar links and
/// `::`-qualification to its *owning* scope.
#[derive(Debug, Clone)]
enum VarLoc {
    /// Global slot: `Interp::globals[<name>]` (name canonical, no `::`).
    Global(String),
    /// Frame slot: `frames[i].locals[<name>]`.
    Frame(usize, String),
}

impl VarLoc {
    fn base_name(&self) -> &str {
        match self {
            VarLoc::Global(n) | VarLoc::Frame(_, n) => n,
        }
    }

    /// Flat key for an array element in this scope.
    fn element_key(&self, index: &str) -> String {
        format!("{}({})", self.base_name(), index)
    }
}

/// Classify an error produced while reading a variable: Tcl treats a
/// missing variable differently from a type conflict in several commands
/// (`incr` starts from 0 for missing, but propagates type errors).
pub(crate) fn is_type_conflict(err: &Error) -> bool {
    let msg = err.to_string();
    msg.contains("variable is array") || msg.contains("variable isn't array")
}

/// Canonical identity key for a variable location (scope-aware).
pub(crate) fn stamp_key(loc: &VarLoc) -> String {
    match loc {
        VarLoc::Global(n) => format!("G:{}", n),
        VarLoc::Frame(i, n) => format!("F{}:{}", i, n),
    }
}

impl Interp {
    // ── internal helpers ────────────────────────────────────────

    /// Resolve the owning scope of a variable name (upvar links followed).
    pub(crate) fn resolve_loc(&self, name: &str) -> VarLoc {
        if let Some(gname) = Self::split_global(name) {
            return VarLoc::Global(gname);
        }
        if let Some(frame) = self.frames.last() {
            if let Some(link) = frame.upvars.get(name) {
                return match link {
                    UpvarLink::Global(gname) => VarLoc::Global(gname.clone()),
                    UpvarLink::Frame { frame_index, var_name } => {
                        VarLoc::Frame(*frame_index, var_name.clone())
                    }
                };
            }
            return VarLoc::Frame(self.frames.len() - 1, name.to_string());
        }
        VarLoc::Global(self.canonical_global(name))
    }

    /// Is the variable at `loc` an array?  (Authoritative registry; the
    /// empty base-key value is only an enumeration marker.)
    fn loc_is_array(&self, loc: &VarLoc) -> bool {
        match loc {
            VarLoc::Global(n) => self.array_globals.contains(n),
            VarLoc::Frame(i, n) => self
                .frames
                .get(*i)
                .map(|f| f.array_locals.contains(n))
                .unwrap_or(false),
        }
    }

    /// Does a variable slot exist at `loc` at all (scalar marker, array
    /// marker, or otherwise)?
    fn loc_base_exists(&self, loc: &VarLoc) -> bool {
        match loc {
            VarLoc::Global(n) => self.globals.contains_key(n),
            VarLoc::Frame(i, n) => self
                .frames
                .get(*i)
                .map(|f| f.locals.contains_key(n))
                .unwrap_or(false),
        }
    }

    /// Get a flat key in the scope owning `loc`.
    fn loc_get(&self, loc: &VarLoc, key: &str) -> Option<&Value> {
        match loc {
            VarLoc::Global(_) => self.globals.get(key),
            VarLoc::Frame(i, _) => self.frames.get(*i).and_then(|f| f.locals.get(key)),
        }
    }

    /// Insert a flat key in the scope owning `loc`.
    fn loc_insert(&mut self, loc: &VarLoc, key: String, value: Value) {
        match loc {
            VarLoc::Global(_) => {
                self.globals.insert(key, value);
            }
            VarLoc::Frame(i, _) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    f.locals.insert(key, value);
                }
            }
        }
    }

    /// Remove a flat key in the scope owning `loc`; returns previous presence.
    fn loc_remove(&mut self, loc: &VarLoc, key: &str) -> bool {
        match loc {
            VarLoc::Global(_) => self.globals.remove(key).is_some(),
            VarLoc::Frame(i, _) => self
                .frames
                .get_mut(*i)
                .map(|f| f.locals.remove(key).is_some())
                .unwrap_or(false),
        }
    }

    /// Mark the variable at `loc` as an array.
    fn loc_mark_array(&mut self, loc: &VarLoc) {
        match loc {
            VarLoc::Global(n) => {
                self.array_globals.insert(n.clone());
            }
            VarLoc::Frame(i, n) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    f.array_locals.insert(n.clone());
                }
            }
        }
    }

    /// Remove an entire array: all element keys, the array marker, and the
    /// base key (tclsh: `unset arr` kills the array itself).
    fn loc_remove_array(&mut self, loc: &VarLoc) {
        let prefix = format!("{}(", loc.base_name());
        match loc {
            VarLoc::Global(_) => {
                let keys: Vec<String> = self
                    .globals
                    .keys()
                    .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                    .cloned()
                    .collect();
                for k in keys {
                    self.globals.remove(&k);
                }
                let base = loc.base_name().to_string();
                self.array_globals.remove(&base);
                self.globals.remove(&base);
            }
            VarLoc::Frame(i, _) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    let keys: Vec<String> = f
                        .locals
                        .keys()
                        .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                        .cloned()
                        .collect();
                    for k in keys {
                        f.locals.remove(&k);
                    }
                    let base = loc.base_name().to_string();
                    f.array_locals.remove(&base);
                    f.locals.remove(&base);
                }
            }
        }
    }


    /// Resolve a variable name, following upvar links.
    /// Returns `Some(&Value)` if found.
    /// A leading `::` qualifies a variable as global regardless of the
    /// current call frame (Tcl namespace-qualified variable access).
    fn split_global(name: &str) -> Option<String> {
        let rest = name.strip_prefix("::")?;
        if rest.is_empty() {
            return None;
        }
        // Colon runs collapse: `set ::a::::b` and `set ::a::b` are the same
        // variable (tclsh 8.6.17).  Single colons are name characters.
        let norm = super::commands::namespace::normalise(rest);
        let mut key = norm.strip_prefix("::").unwrap_or(&norm).to_string();
        if rest.ends_with(':') {
            // A trailing `::` names the EMPTY variable in that namespace
            // (`set ns::` reads ns's variable named ""); normalise would
            // otherwise swallow it.
            key.push_str("::");
        }
        Some(key)
    }

    /// Canonical key for a variable name reached at the global frame
    /// level: a name (plain or relative-qualified) inside a namespace is
    /// that namespace's variable (`set x` in `namespace eval n` writes
    /// `n::x`, visible as `set n::x` — namespace-old-5.5); colon runs
    /// collapse.  Plain names concatenate verbatim (a single `:` is a
    /// name character — var-1.14); names at the global level pass
    /// through.
    pub(crate) fn canonical_global(&self, name: &str) -> String {
        self.canonical_global_in(&self.current_namespace.clone(), name)
    }

    /// [`Interp::canonical_global`] against an explicit namespace (the
    /// caller's context — a proc's own `current_namespace` is its
    /// definition namespace, not the `namespace eval` it was called
    /// from).
    pub(crate) fn canonical_global_in(&self, ns: &str, name: &str) -> String {
        if ns == "::" {
            return name.to_string();
        }
        let qualified = if name.contains("::") {
            super::commands::namespace::qualify(ns, name)
        } else {
            format!("{}::{}", ns, name)
        };
        qualified.strip_prefix("::").unwrap_or(&qualified).to_string()
    }

    fn resolve_var(&self, name: &str) -> Option<&Value> {
        if let Some(gname) = Self::split_global(name) {
            return self.globals.get(gname.as_str());
        }
        if let Some(frame) = self.frames.last() {
            if let Some(link) = frame.upvars.get(name) {
                return match link {
                    // A `variable`-linked name whose namespace variable was
                    // deleted mid-proc still reads the local copy the
                    // `variable` command seeded (tclsh: the linked Var is
                    // refcounted; 46.8 `info exist x` → 1 after
                    // `namespace delete [namespace current]`).
                    UpvarLink::Global(gname) => self
                        .globals
                        .get(gname.as_str())
                        .or_else(|| frame.locals.get(name)),
                    UpvarLink::Frame { frame_index, var_name } => {
                        self.frames.get(*frame_index)
                            .and_then(|f| f.locals.get(var_name.as_str()))
                    }
                };
            }
            frame.locals.get(name)
        } else {
            // Outside procs an unqualified name resolves in the current
            // namespace first, then the global namespace (tclsh 8.6.17:
            // `set v` inside `namespace eval n` reads a `variable`-declared
            // variable; plain global names still resolve).  Plain `set`/
            // `unset` at that level always operate on the global table.  A
            // bare-declared (valueless) name stops the chain: reads fail
            // rather than falling through to the global variable.
            if self.current_namespace != "::" {
                let key = self.canonical_global(name);
                if let Some(v) = self.globals.get(key.as_str()) {
                    return Some(v);
                }
                if self.ns_var_declared(name) {
                    return None;
                }
            }
            self.globals.get(name)
        }
    }

    /// Is `name` (plain, no `::`) declared by the current namespace's
    /// `variable` command?
    fn ns_var_declared(&self, name: &str) -> bool {
        let key = format!("{}::{}", &self.current_namespace[2..], name);
        self.namespaces
            .get(&self.current_namespace)
            .map(|info| info.variables.contains(&key))
            .unwrap_or(false)
    }

    /// Set a variable in the current scope, following upvar links.
    fn store_var(&mut self, name: &str, value: Value) {
        if let Some(gname) = Self::split_global(name) {
            self.globals.insert(gname, value);
            return;
        }
        if self.frames.is_empty() {
            // Inside a namespace every write lands in that namespace's
            // own variable (tclsh: `set x 9` in `namespace eval n` writes
            // ::n::x; `variable`-declared names behave the same way).
            let key = self.canonical_global(name);
            self.globals.insert(key, value);
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    self.globals.insert(gname, value);
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.insert(var_name, value);
                    }
                }
            }
        } else {
            self.frames[frame_idx].locals.insert(name.to_string(), value);
        }
    }

    /// Remove a variable from the current scope, following upvar links.
    fn remove_var(&mut self, name: &str) {
        if let Some(gname) = Self::split_global(name) {
            self.globals.remove(gname.as_str());
            return;
        }
        if self.frames.is_empty() {
            let key = self.canonical_global(name);
            self.globals.remove(key.as_str());
            // Unsetting also forgets the `variable` declaration (tclsh:
            // `namespace which -variable` afterwards is empty and reads
            // fall through to the global namespace again).
            if let Some(info) = self.namespaces.get_mut(&self.current_namespace) {
                info.variables.remove(&key);
            }
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    self.globals.remove(&gname);
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.remove(&var_name);
                    }
                }
            }
        } else {
            self.frames[frame_idx].locals.remove(name);
        }
    }

    // ── public API ─────────────────────────────────────────────

    pub fn get_var(&self, name: &str) -> Result<&Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let mut loc = self.resolve_loc(array_name);
            if !self.loc_is_array(&loc)
                && self.frames.is_empty()
                && self.current_namespace != "::"
                && !array_name.contains("::")
            {
                // Unqualified array read outside procs: the current
                // namespace's `variable`-declared array shadows the global.
                let alt = VarLoc::Global(format!(
                    "{}::{}",
                    &self.current_namespace[2..],
                    array_name
                ));
                if self.loc_is_array(&alt) {
                    loc = alt;
                }
            }
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                self.loc_get(&loc, &key).ok_or_else(|| {
                    Error::runtime(
                        format!("can't read \"{}\": no such element in array", name),
                        ErrorCode::NotFound,
                    )
                })
            } else if self.loc_base_exists(&loc) {
                Err(Error::runtime(
                    format!("can't read \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ))
            } else {
                Err(Error::var_not_found(name))
            }
        } else {
            let loc = self.resolve_loc(name);
            if self.loc_is_array(&loc) {
                return Err(Error::runtime(
                    format!("can't read \"{}\": variable is array", name),
                    ErrorCode::Generic,
                ));
            }
            self.resolve_var(name).ok_or_else(|| Error::var_not_found(name))
        }
    }

    /// tclsh: a WRITE to a namespace-qualified variable requires every
    /// namespace on the path to exist (`can't set "bogus::x": parent
    /// namespace doesn't exist`, errorCode TCL LOOKUP VARNAME). Reads,
    /// `info exists`, and unset stay plain misses.
    pub(crate) fn check_parent_ns(&mut self, name: &str) -> Result<()> {
        let var = match name.find('(') {
            Some(i) => &name[..i],
            None => name,
        };
        if !var.contains("::") {
            return Ok(());
        }
        let full = super::commands::namespace::qualify(&self.current_namespace, var);
        let parent_ns = match full.rsplit_once("::") {
            Some((p, leaf)) if !leaf.is_empty() => {
                if p.is_empty() || p == "::" {
                    return Ok(()); // root always exists
                }
                Self::normalise_ns(p)
            }
            _ => return Ok(()),
        };
        if self.namespaces.contains_key(&parent_ns) {
            return Ok(());
        }
        crate::interp::commands::list::set_error_code(
            self,
            &format!("TCL LOOKUP VARNAME {}", name),
        );
        Err(Error::runtime(
            format!("can't set \"{}\": parent namespace doesn't exist", name),
            ErrorCode::Generic,
        ))
    }

    pub fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        self.check_parent_ns(name)?;
        if let Some((array_name, index)) = split_array_ref(name) {
            let base = array_name.to_string();
            let idx = index.to_string();
            let loc = self.resolve_loc(array_name);
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                let existed = self.loc_get(&loc, &key).is_some();
                self.loc_insert(&loc, key, value.clone());
                if !existed {
                    // New element: element-set mutation invalidates searches.
                    self.bump_stamp(&loc);
                }
            } else if self.loc_base_exists(&loc) {
                return Err(Error::runtime(
                    format!("can't set \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ));
            } else {
                // Create the array: marker + base key (for enumeration and
                // `info exists`) + the first element.
                let base_key = loc.base_name().to_string();
                self.loc_mark_array(&loc);
                self.loc_insert(&loc, base_key, Value::empty());
                let key = loc.element_key(index);
                self.loc_insert(&loc, key, value.clone());
            }
            self.clear_phantom(&base, &idx);
            // Write traces fire after the write lands; a trace error is
            // reported as the set's failure (trace-0.0) but the write
            // itself persists.
            self.fire_traces(&base, Some(&idx), "write")
                .map_err(|e| {
                    Error::runtime(format!("can't set \"{}\": {}", name, e), ErrorCode::Generic)
                })?;
        } else {
            let loc = self.resolve_loc(name);
            if self.loc_is_array(&loc) {
                return Err(Error::runtime(
                    format!("can't set \"{}\": variable is array", name),
                    ErrorCode::Generic,
                ));
            }
            self.store_var(name, value.clone());
            self.fire_traces(name, None, "write").map_err(|e| {
                Error::runtime(format!("can't set \"{}\": {}", name, e), ErrorCode::Generic)
            })?;
        }
        Ok(value)
    }

    pub fn unset_var(&mut self, name: &str) -> Result<()> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let base = array_name.to_string();
            let idx = index.to_string();
            let loc = self.resolve_loc(array_name);
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                if self.loc_remove(&loc, &key) {
                    // Removed element: mutation invalidates searches.
                    self.bump_stamp(&loc);
                    self.clear_phantom(&base, &idx);
                    // Unset traces fire after deletion; errors are
                    // discarded (tclsh: the unset still succeeds).
                    let _ = self.fire_traces(&base, Some(&idx), "unset");
                    Ok(())
                } else {
                    Err(Error::runtime(
                        format!("can't unset \"{}\": no such element in array", name),
                        ErrorCode::NotFound,
                    ))
                }
            } else if self.loc_base_exists(&loc) {
                return Err(Error::runtime(
                    format!("can't unset \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ));
            } else {
                return Err(Error::runtime(
                    format!("can't unset \"{}\": no such variable", name),
                    ErrorCode::NotFound,
                ));
            }
        } else {
            let given = name.to_string();
            let loc = self.resolve_loc(name);
            if self.loc_is_array(&loc) {
                self.loc_remove_array(&loc);
                // Whole-array unset kills its searches and stamp. Unset
                // traces fire after the deletion, so drop the tables after
                // firing them.
                let sk = stamp_key(&loc);
                self.array_searches.remove(&sk);
                self.array_stamps.remove(&sk);
                let _ = self.fire_traces(&given, None, "unset");
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                Ok(())
            } else if self.loc_base_exists(&loc) {
                self.remove_var(name);
                let sk = stamp_key(&loc);
                let _ = self.fire_traces(&given, None, "unset");
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                Ok(())
            } else if self.current_namespace != "::" && self.ns_var_declared(&given) {
                // `variable`-declared but valueless (or declared with the
                // value living in the namespace's own slot): unset removes
                // the declaration and succeeds (tclsh 8.6.17).
                self.remove_var(&given);
                Ok(())
            } else {
                Err(Error::runtime(
                    format!("can't unset \"{}\": no such variable", name),
                    ErrorCode::NotFound,
                ))
            }
        }
    }

    pub fn var_exists(&self, name: &str) -> bool {
        if let Some((array_name, index)) = split_array_ref(name) {
            let mut loc = self.resolve_loc(array_name);
            if !self.loc_is_array(&loc)
                && self.frames.is_empty()
                && self.current_namespace != "::"
                && !array_name.contains("::")
            {
                let alt = VarLoc::Global(format!(
                    "{}::{}",
                    &self.current_namespace[2..],
                    array_name
                ));
                if self.loc_is_array(&alt) {
                    loc = alt;
                }
            }
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                self.loc_get(&loc, &key).is_some()
            } else {
                // Element of a scalar / of a missing variable: never exists.
                false
            }
        } else {
            self.resolve_var(name).is_some()
        }
    }

    /// `incr varName ?amount?` — Tcl reads first: a missing variable starts
    /// at 0, but a type conflict (scalar where element expected, or array
    /// read as scalar) is an error.
    pub fn incr_var(&mut self, name: &str, amount: i64) -> Result<Value> {
        let current: i64 = match self.get_var(name) {
            Ok(v) => v
                .as_int()
                .ok_or_else(|| Error::type_mismatch("integer", v.as_str()))?,
            Err(e) => {
                if is_type_conflict(&e) {
                    return Err(e);
                }
                0
            }
        };
        self.set_var(name, Value::from_int(current + amount))
    }

    /// Is `name` (whole-name form) an array in the scope that owns it?
    pub(crate) fn is_array_semantic(&self, name: &str) -> bool {
        let loc = self.resolve_loc(name);
        self.loc_is_array(&loc)
    }

    /// Canonical identity of the array at `name` (scope-aware), used to key
    /// search lists and mutation stamps.
    pub(crate) fn array_stamp_key(&self, name: &str) -> String {
        stamp_key(&self.resolve_loc(name))
    }

    /// Fire variable traces for `op` on (base, elem). Callback scripts run
    /// in the current scope with `name1 name2 op` appended (tclsh passes
    /// name2 as an empty string for whole-variable ops).
    pub(crate) fn fire_traces(&mut self, base: &str, elem: Option<&str>, op: &str) -> Result<()> {
        let key = self.array_stamp_key(base);
        let mut scripts: Vec<String> = Vec::new();
        if let Some(trs) = self.var_traces.get(&key) {
            for t in trs {
                if t.ops.iter().any(|o| o == op) {
                    scripts.push(t.script.clone());
                }
            }
        }
        if let Some(e) = elem {
            if let Some(map) = self.elem_traces.get(&key) {
                if let Some(trs) = map.get(e) {
                    for t in trs {
                        if t.ops.iter().any(|o| o == op) {
                            scripts.push(t.script.clone());
                        }
                    }
                }
            }
        }
        for s in scripts.into_iter().rev() {
            // Most-recent-registered fires first (tclsh prepends to the
            // trace list: T2 T1 for two write traces). The script string
            // gets the args appended (so scripts can end in `;#` to
            // swallow them) and is evaluated as a script.
            let q = |v: &str| {
                Value::from_list(&[Value::from_str(v)]).as_str().to_string()
            };
            let cmd = format!("{} {} {} {}", s, q(base), q(elem.unwrap_or("")), op);
            self.eval(&cmd)?;
        }
        Ok(())
    }

    /// A trace on an element was satisfied: the element now has a real
    /// value, so drop its phantom marker.
    fn clear_phantom(&mut self, base: &str, elem: &str) {
        let key = self.array_stamp_key(base);
        if let Some(s) = self.trace_phantoms.get_mut(&key) {
            s.remove(elem);
        }
    }

    pub(crate) fn bump_stamp_by_name(&mut self, name: &str) {
        let loc = self.resolve_loc(name);
        self.bump_stamp(&loc);
    }

    /// Read a variable, firing read traces the way tclsh does: scalar
    /// reads fire even when the variable is missing; element reads fire
    /// only when the element exists (set-old-9.x, trace-1.8).
    pub fn read_var(&mut self, name: &str) -> Result<Value> {
        let (base, idx) = match split_array_ref(name) {
            Some((b, i)) => (b.to_string(), Some(i.to_string())),
            None => (name.to_string(), None),
        };
        let found = self.get_var(name).is_ok();
        if idx.is_none() || found {
            if let Err(e) = self.fire_traces(&base, idx.as_deref(), "read") {
                return Err(Error::runtime(
                    format!("can't read \"{}\": {}", name, e),
                    ErrorCode::Generic,
                ));
            }
        }
        self.get_var(name).cloned()
    }

    /// `info exists`, which fires read traces (tclsh probe: yes, even for
    /// missing scalars) but swallows trace errors.
    pub fn exists_firing(&mut self, name: &str) -> bool {
        let (base, idx) = match split_array_ref(name) {
            Some((b, i)) => (b.to_string(), Some(i.to_string())),
            None => (name.to_string(), None),
        };
        if idx.is_none() || self.var_exists(name) {
            let _ = self.fire_traces(&base, idx.as_deref(), "read");
        }
        self.var_exists(name)
    }

    /// Force array semantics at `name` (marker + base key) without adding
    /// elements — `array set x {}` on a missing variable still creates it.
    pub(crate) fn mark_array(&mut self, name: &str) -> Result<()> {
        self.check_parent_ns(name)?;
        let loc = self.resolve_loc(name);
        if !self.loc_is_array(&loc) {
            let base_key = loc.base_name().to_string();
            self.loc_mark_array(&loc);
            self.loc_insert(&loc, base_key, Value::empty());
        }
        Ok(())
    }

    /// Normalise a fully-qualified namespace path (leading `::`, no
    /// empty components).
    fn normalise_ns(p: &str) -> String {
        let parts: Vec<&str> = p.split("::").filter(|s| !s.is_empty()).collect();
        format!("::{}", parts.join("::"))
    }

    /// Current mutation stamp of the array at `name` (0 if never mutated).
    pub(crate) fn array_stamp(&self, name: &str) -> u64 {
        self.array_stamps.get(&self.array_stamp_key(name)).copied().unwrap_or(0)
    }

    fn bump_stamp(&mut self, loc: &VarLoc) {
        let k = stamp_key(loc);
        *self.array_stamps.entry(k.clone()).or_insert(0) += 1;
        // tclsh: any element-set change kills every search on the array;
        // an emptied search list also resets the id counter.
        if let Some(list) = self.array_searches.get_mut(&k) {
            list.active.clear();
            list.ctr = 0;
        }
    }

    /// Snapshot of the element names of the array at `name`.
    pub(crate) fn array_element_names(&self, name: &str) -> Vec<String> {
        let loc = self.resolve_loc(name);
        let prefix = format!("{}(", loc.base_name());
        let mut out: Vec<String> = match &loc {
            VarLoc::Global(_) => self
                .globals
                .keys()
                .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                .map(|k| k[prefix.len()..k.len() - 1].to_string())
                .collect(),
            VarLoc::Frame(i, _) => self
                .frames
                .get(*i)
                .map(|f| {
                    f.locals
                        .keys()
                        .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                        .map(|k| k[prefix.len()..k.len() - 1].to_string())
                        .collect()
                })
                .unwrap_or_default(),
        };
        out.sort();
        out
    }

    pub fn result(&self) -> &Value {
        &self.result
    }

    #[cfg(feature = "std")]
    pub fn set_script_name(&mut self, name: &str) {
        self.script_name = name.to_string();
    }

    #[cfg(feature = "std")]
    pub fn script_name(&self) -> &str {
        &self.script_name
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // -- `::name` global qualification (tclsh 8.6.17) --

    #[test]
    fn test_global_qualification_read() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("set x 5; set ::x").unwrap().as_str(), "5");
    }

    #[test]
    fn test_global_qualification_write() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("set ::y 7; set y").unwrap().as_str(), "7");
    }

    #[test]
    fn test_global_qualification_from_proc() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("set e 1; proc p {} { return $::e }; p").unwrap().as_str(), "1");
    }

    #[test]
    fn test_global_qualification_never_sees_locals() {
        // ::z inside a proc is always the global, even when a local z exists.
        let mut interp = Interp::new();
        let r = interp
            .eval("proc q {} { set ::z 9; set z 8; return [list $z $::z] }; q")
            .unwrap();
        assert_eq!(r.as_str(), "8 9");
        assert_eq!(interp.eval("set z").unwrap().as_str(), "9");
    }

    #[test]
    fn test_global_qualification_unset_and_exists() {
        assert_eq!(interp_eval("set k 1; unset ::k; info exists k"), "0");
        assert_eq!(interp_eval("set k2 1; unset k2; info exists ::k2"), "0");
    }

    fn interp_eval(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    // -- ::errorCode observability (tclsh 8.6.17) --

    #[test]
    fn test_errorcode_from_strict_list_error() {
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {lsort -integer {09 8}} m; set ::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "TCL VALUE NUMBER");
    }

    #[test]
    fn test_errorcode_none_after_bare_error() {
        let mut interp = Interp::new();
        let r = interp.eval("catch {error foo} m; set ::errorCode").unwrap();
        assert_eq!(r.as_str(), "NONE");
    }

    #[test]
    fn test_errorcode_from_error_third_arg() {
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {error msg {} {CUSTOM X}} m; set ::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "CUSTOM X");
    }

    #[test]
    fn test_errorcode_persists_after_successful_command() {
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {lsort -index -1 x} m; set unused 1; set ::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "TCL VALUE INDEXOUTOFRANGE");
    }

    // -- scalar/array distinction (tclsh 8.6.17 oracle probes) --

    fn eval_err(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap_err().to_string()
    }

    #[test]
    fn test_element_set_on_scalar_errors() {
        assert_eq!(
            eval_err("set x {}; set x(0) 44"),
            "can't set \"x(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_element_read_on_scalar_errors() {
        assert_eq!(
            eval_err("set y 5; set y(0)"),
            "can't read \"y(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_scalar_set_on_array_errors() {
        assert_eq!(
            eval_err("set a(0) 1; set a 5"),
            "can't set \"a\": variable is array"
        );
    }

    #[test]
    fn test_scalar_read_of_array_errors() {
        assert_eq!(
            eval_err("set b(0) 1; set b"),
            "can't read \"b\": variable is array"
        );
    }

    #[test]
    fn test_append_element_on_scalar_and_array() {
        assert_eq!(
            eval_err("set z 1; append z(2) a"),
            "can't set \"z(2)\": variable isn't array"
        );
        assert_eq!(
            eval_err("set c(0) 1; append c 5"),
            "can't set \"c\": variable is array"
        );
    }

    #[test]
    fn test_append_element_creates_missing_array() {
        // tclsh: append nm(0) z → nm becomes an array
        assert_eq!(interp_eval("append nm(0) z; set nm(0)"), "z");
    }

    #[test]
    fn test_array_survives_last_element_unset() {
        // tclsh: unset of the last element keeps the array alive
        assert_eq!(interp_eval("set d(0) 1; unset d(0); array exists d"), "1");
        assert_eq!(
            eval_err("set d2(0) 1; unset d2(0); set d2 5"),
            "can't set \"d2\": variable is array"
        );
        assert_eq!(interp_eval("set e(0) 1; unset e(0); info exists e"), "1");
    }

    #[test]
    fn test_unset_whole_array_removes_everything() {
        assert_eq!(
            interp_eval("set g(0) 1; set g(1) 2; unset g; list [info exists g] [array exists g]"),
            "0 0"
        );
    }

    #[test]
    fn test_element_read_error_kinds() {
        assert_eq!(
            eval_err("set a2(0) 1; set a2(5)"),
            "can't read \"a2(5)\": no such element in array"
        );
        assert_eq!(
            eval_err("set zz(5)"),
            "can't read \"zz(5)\": no such variable"
        );
    }

    #[test]
    fn test_element_unset_error_kinds() {
        assert_eq!(
            eval_err("set u9 5; unset u9(3)"),
            "can't unset \"u9(3)\": variable isn't array"
        );
        assert_eq!(
            eval_err("set a3(0) 1; unset a3(5)"),
            "can't unset \"a3(5)\": no such element in array"
        );
        assert_eq!(
            eval_err("unset zz2(5)"),
            "can't unset \"zz2(5)\": no such variable"
        );
    }

    #[test]
    fn test_proc_local_array_shadows_global_scalar() {
        // tclsh: inside a proc, set gs(0) creates a LOCAL array gs even
        // though a global scalar gs exists.
        assert_eq!(
            interp_eval("set gs 5; proc pp {} {set gs(0) 1; return [array exists gs]}; pp"),
            "1"
        );
        assert_eq!(interp_eval("set gs 5; proc pp {} {set gs(0) 1; return [array exists gs]}; pp; set gs"), "5");
    }

    #[test]
    fn test_upvar_element_uses_alias_name_in_errors() {
        // tclsh: upvar to a scalar, element write errors naming the ALIAS.
        assert_eq!(
            eval_err("set us 5; proc up2 {} {upvar 1 us b; set b(0) y}; up2"),
            "can't set \"b(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_upvar_element_read_and_write() {
        // upvar to an array: element ops follow the link.
        assert_eq!(
            interp_eval("set ua(0) x; proc up {} {upvar 1 ua b; set b(0) y; return $b(0)}; up"),
            "y"
        );
        assert_eq!(
            interp_eval("set ua2(0) x; proc up2 {} {upvar 1 ua2 b; return $b(0)}; up2"),
            "x"
        );
    }

    #[test]
    fn test_array_set_on_scalar() {
        // tclsh quirk: empty pairs → "can't array set", non-empty →
        // first element write error.
        assert_eq!(
            eval_err("set s9 5; array set s9 {}"),
            "can't array set \"s9\": variable isn't array"
        );
        assert_eq!(
            eval_err("set arr3 5; array set arr3 {0 1}"),
            "can't set \"arr3(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_incr_element_on_scalar_reads_first() {
        assert_eq!(
            eval_err("set ii 5; incr ii(0)"),
            "can't read \"ii(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_info_exists_element_of_scalar_is_false() {
        assert_eq!(interp_eval("set is2 5; info exists is2(0)"), "0");
    }

    #[test]
    fn test_global_element_access_from_proc() {
        assert_eq!(
            interp_eval("set ga(0) 1; proc gp {} {set ::ga(0) 2; return $::ga(0)}; gp"),
            "2"
        );
    }

    #[test]
    fn test_array_exists_and_size_on_scalar() {
        assert_eq!(interp_eval("set as2 5; array exists as2"), "0");
        assert_eq!(interp_eval("set as2 5; array size as2"), "0");
        assert_eq!(interp_eval("set aw(0) 1; array size aw"), "1");
    }

    #[test]
    fn test_builtin_arrays_exist() {
        assert_eq!(interp_eval("array exists tcl_platform"), "1");
        assert_eq!(interp_eval("array exists env"), "1");
    }

    // -- ARITH errorCode on arithmetic errors (tclsh 8.6.17) --

    #[test]
    fn test_arith_divzero_errorcode() {
        assert_eq!(
            interp_eval("catch {expr {1/0}} m; set ::errorCode"),
            "ARITH DIVZERO {divide by zero}"
        );
        assert_eq!(
            interp_eval("catch {expr {1%0}} m; set ::errorCode"),
            "ARITH DIVZERO {divide by zero}"
        );
    }

    #[test]
    fn test_arith_domain_errorcode() {
        assert_eq!(
            interp_eval("catch {expr {sqrt(-1)}} m; set ::errorCode"),
            "ARITH DOMAIN {domain error: argument not in valid range}"
        );
    }

    #[test]
    fn test_arith_errorcode_in_catch_options() {
        // tclsh: the optionVar is a dict; element syntax is an error.
        assert_eq!(
            interp_eval("catch {expr {1/0}} m o; dict get $o -errorcode"),
            "ARITH DIVZERO {divide by zero}"
        );
    }

    #[test]
    fn test_no_errorcode_write_for_plain_lookup_errors() {
        // tclsh: a missing variable/command does not set an ARITH errorCode.
        assert_eq!(interp_eval("catch {set nosuchvar} m; set ::errorCode"), "NONE");
    }
}
