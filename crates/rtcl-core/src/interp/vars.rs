//! Variable access methods on [`Interp`].
//!
//! All variable reads/writes are scope-aware: at global level they use
//! `Interp::globals`; inside a proc they use the current `CallFrame`'s
//! locals (or follow upvar links to globals / other frames).

use super::util::{has_ns_sep, split_array_ref};
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

    // ── E2 slot-locals aliasing ───────────────────────────────────
    //
    // tclsh's compiledLocals array and its locals hash table alias the
    // same Var entries.  rtcl models that with slots as the canonical
    // store of table names plus these consults on every name-keyed path:
    // an uncompiled writer (foreach var, lappend, catch result, `global`
    // mirror…) observes the same variable the compiled ops touch.

    /// Value of slot `slot` in the current frame, when it holds one
    /// (unset slots and name-keyed frames both miss — the callers fall
    /// back to the name path).
    pub(crate) fn frame_slot_value(&self, slot: usize) -> Option<Value> {
        let f = self.frames.last()?;
        if f.slot_aliased.get(slot).copied().unwrap_or(false) {
            // Global link: ONE probe with the canonical key (the name
            // path would pay upvars + globals = two hashes).
            let key = f.link_keys.get(slot)?.as_ref()?;
            return self.globals.get(key.as_ref()).cloned();
        }
        f.slots.get(slot)?.clone()
    }

    /// Write `value` into slot `slot` of the current frame.  `false` when
    /// the frame is not slot-compiled (or has degraded) — the caller then
    /// takes the name path.  Takes the value by reference: the refcount
    /// bump happens only on the write, so a caller whose fallback consumes
    /// the value pays nothing extra on the attempt.
    pub(crate) fn frame_slot_write(&mut self, slot: usize, value: &Value) -> bool {
        let aliased = self
            .frames
            .last()
            .map(|f| f.slot_aliased.get(slot).copied().unwrap_or(false))
            .unwrap_or(true);
        if aliased {
            // Global link: write the target directly through the cached
            // canonical key (one probe; creation goes through the name
            // path — the link install seeded the variable, so the hot
            // case is always the hit).
            let key = self
                .frames
                .last()
                .and_then(|f| f.link_keys.get(slot))
                .and_then(|k| k.clone());
            if let Some(key) = key {
                if let Some(cell) = self.globals.get_mut(key.as_ref()) {
                    *cell = value.clone();
                    return true;
                }
                let name = self.frames.last().and_then(|f| {
                    f.slot_table.as_ref().and_then(|t| t.locals().get(slot).cloned())
                });
                if let Some(name) = name {
                    let _ = self.set_var(&name, value.clone());
                    return true;
                }
            }
            return false;
        }
        match self.frames.last_mut().and_then(|f| f.slots.get_mut(slot)) {
            Some(cell) => {
                *cell = Some(value.clone());
                true
            }
            None => false,
        }
    }

    /// `incr` on a frame slot, mutating the int rep in place
    /// (the slot analogue of [`Interp::incr_var_fast`]; `None` falls back
    /// to the real `incr`, which owns creation and the exact errors).
    pub(crate) fn frame_slot_incr(&mut self, slot: usize, amount: i64) -> Option<Value> {
        if !self.var_traces.is_empty() {
            return None;
        }
        {
            let f = self.frames.last()?;
            if f.slot_aliased.get(slot).copied().unwrap_or(false) {
                // Global link: one probe, in-place int rewrite when the
                // stored value is unshared/immediate.
                let key = f.link_keys.get(slot)?.as_ref()?;
                let cell = self.globals.get_mut(key.as_ref())?;
                let current = cell.as_int()?;
                let next = current.checked_add(amount)?;
                cell.set_int_rep(next);
                return Some(cell.clone());
            }
        }
        let cell = self.frames.last_mut()?.slots.get_mut(slot)?.as_mut()?;
        let current = cell.as_int()?;
        // Overflow belongs to the real `incr` (ARITH IOVERFLOW error).
        let next = current.checked_add(amount)?;
        cell.set_int_rep(next);
        Some(cell.clone())
    }

    /// Degrade `frames[frame_idx]` back to the name-keyed store — but
    /// only when `name` actually lives in its slot table (otherwise the
    /// exceptional structure being installed coexists with the slots
    /// fine, and the frame keeps its fast paths).  Slot values migrate in
    /// slot order: params first, then `set`-discovered names in
    /// compilation order — the same sequence in which the tree-walk
    /// inserts them at runtime, so hash iteration order (and anything
    /// derived from it after a later degrade) matches too.
    pub(crate) fn degrade_frame_local(&mut self, frame_idx: usize, name: &str) {
        let base = match name.find('(') {
            Some(i) => &name[..i],
            None => name,
        };
        let Some(f) = self.frames.get_mut(frame_idx) else { return };
        if f.slots.is_empty() || f.slot_index_of(base).is_none() {
            return;
        }
        if let Some(table) = f.slot_table.take() {
            for (i, name) in table.locals().iter().enumerate() {
                if let Some(v) = f.slots.get_mut(i).and_then(|c| c.take()) {
                    f.locals.insert(name.clone(), v);
                }
            }
        }
        f.slots.clear();
        f.slot_aliased.clear();
        f.link_keys.clear();
    }

    /// Link-only aliasing of ONE slot: move the cell's value into
    /// `locals` and mark the slot aliased, so slot ops on that name defer
    /// to the name path (which resolves the link) while every OTHER slot
    /// keeps its compiled fast path.  Used by `upvar` / `global` /
    /// `variable` — the links a hot method body re-installs per call —
    /// where the old whole-frame flush demoted every variable of the
    /// frame to name-keyed lookups.  Traces / array-ification / unset
    /// keep [`Self::degrade_frame_local`]'s wholesale flush (their
    /// storage-kind change is frame-wide by nature, and they are cold).
    pub(crate) fn degrade_frame_link(&mut self, frame_idx: usize, name: &str, link: &crate::interp::UpvarLink) {
        let base = match name.find('(') {
            Some(i) => &name[..i],
            None => name,
        };
        let Some(f) = self.frames.get_mut(frame_idx) else { return };
        if f.slots.is_empty() {
            return;
        }
        let Some(i) = f.slot_index_of(base) else { return };
        if let Some(v) = f.slots[i].take() {
            f.locals.insert(base.to_string(), v);
        }
        if f.slot_aliased.len() != f.slots.len() {
            f.slot_aliased.clear();
            f.slot_aliased.resize(f.slots.len(), false);
        }
        f.slot_aliased[i] = true;
        // Global links get the direct-key fast path: slot ops on this
        // name resolve through ONE globals probe with the canonical key
        // (frame/dead links keep None — their name paths handle the
        // indirection).
        if f.link_keys.len() != f.slots.len() {
            f.link_keys.clear();
            f.link_keys.resize(f.slots.len(), None);
        }
        f.link_keys[i] = match link {
            crate::interp::UpvarLink::Global(g) => {
                Some(std::rc::Rc::from(g.as_str()))
            }
            _ => None,
        };
    }

    /// Whole-frame degrade of `frames[frame_idx]` — for sites that enumerate
    /// the name-keyed store (`info vars`, `info locals`, `info frame`):
    /// after this, `frame.locals` alone is a complete view again.
    pub(crate) fn degrade_frame_all_at(&mut self, frame_idx: usize) {
        let table = {
            let f = match self.frames.get_mut(frame_idx) {
                Some(f) => f,
                None => return,
            };
            if f.slots.is_empty() {
                return;
            }
            f.slot_table.take()
        };
        let f = &mut self.frames[frame_idx];
        if let Some(table) = table {
            for (i, name) in table.locals().iter().enumerate() {
                if let Some(v) = f.slots.get_mut(i).and_then(|c| c.take()) {
                    f.locals.insert(name.clone(), v);
                }
            }
        }
        f.slots.clear();
        f.slot_aliased.clear();
        f.link_keys.clear();
    }

    /// [`Self::degrade_frame_all_at`] on the current frame (no-op at the
    /// global level).
    pub(crate) fn degrade_frame_all(&mut self) {
        if let Some(i) = self.frames.len().checked_sub(1) {
            self.degrade_frame_all_at(i);
        }
    }

    /// [`Self::degrade_frame_local`] on the current frame (no-op at the
    /// global level, where there is nothing to degrade).
    pub(crate) fn degrade_frame_local_here(&mut self, name: &str) {
        if let Some(i) = self.frames.len().checked_sub(1) {
            self.degrade_frame_local(i, name);
        }
    }

    /// Resolve the owning scope of a variable name (upvar links followed).
    pub(crate) fn resolve_loc(&self, name: &str) -> VarLoc {
        if let Some(gname) = Self::split_global(name) {
            return VarLoc::Global(self.redirect_flat(gname));
        }
        if let Some(frame) = self.frames.last() {
            if let Some(link) = frame.upvars.get(name) {
                return match link {
                    UpvarLink::Global(gname) => {
                        VarLoc::Global(self.redirect_flat(gname.clone()))
                    }
                    UpvarLink::Frame { frame_index, var_name } => {
                        VarLoc::Frame(*frame_index, var_name.clone())
                    }
                    // A dead link reads (and exists) against the alias's
                    // seeded local copy; writes are rejected in set_var.
                    UpvarLink::Dead { .. } => {
                        VarLoc::Frame(self.frames.len() - 1, name.to_string())
                    }
                };
            }
            return VarLoc::Frame(self.frames.len() - 1, name.to_string());
        }
        VarLoc::Global(self.redirect_flat(self.canonical_global(name)))
    }

    /// Follow an eval-level `upvar` alias (global level): the alias's flat
    /// key maps to the target's flat key.
    fn redirect_flat(&self, key: String) -> String {
        for (k, t) in &self.flat_aliases {
            if *k == key {
                return t.clone();
            }
        }
        key
    }

    /// Dead-link write check (tclsh 8.6.17): only WRITES through an alias
    /// whose target was destroyed error; reads/`info exists`/unset keep the
    /// pre-deletion behavior against the seeded local copy.  Returns
    /// `Some(is_element_error)` when the write must fail.
    fn dead_link_error(&self, name: &str) -> Option<bool> {
        let base = match name.find('(') {
            Some(i) => &name[..i],
            None => name,
        };
        if let Some(frame) = self.frames.last() {
            match frame.upvars.get(base) {
                Some(UpvarLink::Dead { array, .. }) => Some(*array),
                _ => None,
            }
        } else {
            let key = self.canonical_global(base);
            if self.dead_flat.iter().any(|k| *k == key) {
                Some(false)
            } else {
                None
            }
        }
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
                .map(|f| f.locals.contains_key(n) || f.slot_value(n).is_some())
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
        // Container generation bump (see loc_remove_array): recreation
        // counts as a container change for in-flight `array get` reads.
        *self
            .array_generations
            .entry(stamp_key(loc))
            .or_insert(0) += 1;
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
        // (generation bump at the end of this function)
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
        // Container generation bump: a destroyed array's snapshot entries
        // are invisible to an in-flight `array get` even when a callback
        // recreates same-named elements (tclsh holds the old Var).
        *self
            .array_generations
            .entry(stamp_key(loc))
            .or_insert(0) += 1;
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

    /// [`Self::split_global`] for callers outside `vars.rs` (`global`'s
    /// link targets follow the same normalization as `set`).
    pub(crate) fn global_key(name: &str) -> Option<String> {
        Self::split_global(name)
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
            return self.globals.get(self.redirect_flat(gname).as_str());
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
                        .get(self.redirect_flat(gname.clone()).as_str())
                        .or_else(|| frame.locals.get(name)),
                    UpvarLink::Frame { frame_index, var_name } => {
                        // Link installation degrades the target frame, so
                        // the map probe is the live path; the slot consult
                        // is defense in depth.
                        self.frames.get(*frame_index)
                            .and_then(|f| f.slot_value(var_name.as_str())
                                .or_else(|| f.locals.get(var_name.as_str())))
                    }
                    UpvarLink::Dead { target, .. } => self
                        .globals
                        .get(target.as_str())
                        .or_else(|| frame.locals.get(name)),
                };
            }
            // Slotted name: the slot is the canonical store (unset slots
            // fall through — the map never holds a table name).
            frame
                .slot_value(name)
                .or_else(|| frame.locals.get(name))
        } else {
            // Outside procs an unqualified name resolves in the current
            // namespace first, then the global namespace (tclsh 8.6.17:
            // `set v` inside `namespace eval n` reads a `variable`-declared
            // variable; plain global names still resolve).  Plain `set`/
            // `unset` at that level always operate on the global table.  A
            // bare-declared (valueless) name stops the chain: reads fail
            // rather than falling through to the global variable.
            if self.current_namespace.as_ref() != "::" {
                let key = self.redirect_flat(self.canonical_global(name));
                if let Some(v) = self.globals.get(key.as_str()) {
                    return Some(v);
                }
                if self.ns_var_declared(name) {
                    return None;
                }
            }
            self.globals.get(self.redirect_flat(name.to_string()).as_str())
        }
    }

    /// Is `name` (plain, no `::`) declared by the current namespace's
    /// `variable` command?
    fn ns_var_declared(&self, name: &str) -> bool {
        let key = format!("{}::{}", &self.current_namespace[2..], name);
        self.namespaces
            .get(self.current_namespace.as_ref())
            .map(|info| info.variables.contains(&key))
            .unwrap_or(false)
    }

    /// Set a variable in the current scope, following upvar links.
    pub(crate) fn store_var(&mut self, name: &str, value: Value) {
        if let Some(gname) = Self::split_global(name) {
            let gname = self.redirect_flat(gname);
            self.globals.insert(gname, value);
            return;
        }
        if self.frames.is_empty() {
            // Inside a namespace every write lands in that namespace's
            // own variable (tclsh: `set x 9` in `namespace eval n` writes
            // ::n::x; `variable`-declared names behave the same way).
            let key = self.redirect_flat(self.canonical_global(name));
            self.globals.insert(key, value);
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    // A link write lands on the SAME global every time:
                    // probe in place first (one hash, no key churn) — the
                    // unconditional insert dropped and re-stored the key
                    // String per write.
                    let key = self.redirect_flat(gname);
                    if let Some(slot) = self.globals.get_mut(&key) {
                        *slot = value;
                    } else {
                        self.globals.insert(key, value);
                    }
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.insert(var_name, value);
                    }
                }
                // Writes through dead links error in set_var before
                // reaching here; keep the write local for safety.
                UpvarLink::Dead { .. } => {
                    self.frames[frame_idx].locals.insert(name.to_string(), value);
                }
            }
        } else {
            let f = &mut self.frames[frame_idx];
            match f.slot_index_of(name) {
                // Slotted name: the slot is the canonical store.
                Some(i) => f.slots[i] = Some(value),
                None => {
                    f.locals.insert(name.to_string(), value);
                }
            }
        }
    }

    /// Does `name` resolve (through upvar/flat aliases) to an ELEMENT slot
    /// rather than an array base?  A link whose target keeps the `a(e)`
    /// shape addresses a scalar element even though the written name has
    /// no parentheses (set-old-8.38.3: `upvar 0 a(e) x; array set x {}`
    /// → "variable isn't array").
    pub(crate) fn resolves_to_element(&self, name: &str) -> bool {
        if name.contains('(') {
            return true;
        }
        if let Some(frame) = self.frames.last() {
            match frame.upvars.get(name) {
                Some(UpvarLink::Global(g)) => g.contains('('),
                _ => false,
            }
        } else {
            self.flat_aliases
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, t)| t.contains('('))
                .unwrap_or(false)
        }
    }

    /// Drop the seeded mirror copies for every frame's alias to a scalar
    /// global that was just unset, KEEPING the links (set-old-7.7..7.9
    /// probed semantics: after the target's deletion a `global` link
    /// dangles — reads/`info exists` see nothing (7.7), but a WRITE
    /// through the still-live link recreates the global, visible outside
    /// (7.8/7.9) — so only the stale seed must go, not the link).
    fn drop_global_seeds(&mut self, key: &str) {
        for f in self.frames.iter_mut() {
            let dead: Vec<String> = f
                .upvars
                .iter()
                .filter_map(|(n, l)| match l {
                    UpvarLink::Global(g) => {
                        // Mirror resolve_loc's redirect so aliased flat
                        // keys (global-level `upvar`) compare equal too.
                        let flat = self
                            .flat_aliases
                            .iter()
                            .find(|(k, _)| k == g)
                            .map(|(_, t)| t.as_str())
                            .unwrap_or(g);
                        (flat == key).then(|| n.clone())
                    }
                    _ => None,
                })
                .collect();
            for n in dead {
                f.locals.remove(&n);
                f.array_locals.remove(&n);
            }
        }
    }

    /// Remove a variable from the current scope, following upvar links.
    fn remove_var(&mut self, name: &str) {
        if let Some(gname) = Self::split_global(name) {
            let gname = self.redirect_flat(gname);
            self.globals.remove(gname.as_str());
            return;
        }
        if self.frames.is_empty() {
            let key = self.redirect_flat(self.canonical_global(name));
            self.globals.remove(key.as_str());
            // Unsetting also forgets the `variable` declaration (tclsh:
            // `namespace which -variable` afterwards is empty and reads
            // fall through to the global namespace again).
            if let Some(info) = self.namespaces.get_mut(self.current_namespace.as_ref()) {
                info.variables.remove(&key);
            }
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    let gname = self.redirect_flat(gname);
                    self.globals.remove(gname.as_str());
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.remove(&var_name);
                    }
                }
                // Unset through a dead alias keeps its pre-deletion
                // behavior (removes the seeded local copy).
                UpvarLink::Dead { .. } => {
                    self.frames[frame_idx].locals.remove(name);
                }
            }
        } else {
            let f = &mut self.frames[frame_idx];
            match f.slot_index_of(name) {
                // Unset of a slotted name empties the cell (tclsh: the
                // compiledLocal's Var is cleared, the table entry stays).
                Some(i) => f.slots[i] = None,
                None => {
                    f.locals.remove(name);
                }
            }
        }
    }

    // ── public API ─────────────────────────────────────────────

    pub fn get_var(&self, name: &str) -> Result<&Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let mut loc = self.resolve_loc(array_name);
            if !self.loc_is_array(&loc)
                && self.frames.is_empty()
                && self.current_namespace.as_ref() != "::"
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
            // Fast path: unqualified scalar in the innermost frame (no
            // `::` qualifier, no upvar link) — the proc-local and
            // loop-variable case.  One hash probe, no allocation; the
            // slow path below keeps the full scope-chain semantics.
            // The upvar/array sidecar probes are guarded by emptiness:
            // an empty set can't contain the name, and the common frame
            // (no upvars, no local arrays) then pays probes only for
            // `locals` itself.
            if !has_ns_sep(name) {
                if let Some(frame) = self.frames.last() {
                    if frame.upvars.is_empty() || !frame.upvars.contains_key(name) {
                        // Slotted name: the slot is the canonical store (a
                        // table name is never in the map).  An empty cell is
                        // the unset state — the map probe below must not run.
                        if let Some(i) = frame.slot_index_of(name) {
                            return match frame.slots[i].as_ref() {
                                Some(v) => Ok(v),
                                None => Err(Error::var_not_found(name)),
                            };
                        }
                        if let Some(v) = frame.locals.get(name) {
                            if !frame.array_locals.is_empty()
                                && frame.array_locals.contains(name)
                            {
                                return Err(Error::runtime(
                                    format!("can't read \"{}\": variable is array", name),
                                    ErrorCode::Generic,
                                ));
                            }
                            return Ok(v);
                        }
                    }
                } else if self.current_namespace.as_ref() == "::" && self.flat_aliases.is_empty() {
                    if let Some(v) = self.globals.get(name) {
                        if !self.array_globals.is_empty() && self.array_globals.contains(name) {
                            return Err(Error::runtime(
                                format!("can't read \"{}\": variable is array", name),
                                ErrorCode::Generic,
                            ));
                        }
                        return Ok(v);
                    }
                }
            }
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

    /// tclsh: a WRITE to a namespace-qualified variable requires the
    /// variable's namespace to exist (`can't set "bogus::x": parent
    /// namespace doesn't exist`, errorCode TCL LOOKUP VARNAME).  A trailing
    /// colon run names the EMPTY variable in that namespace — the namespace
    /// to check is then the qualifiers part itself (`set ::pns::` is legal,
    /// `set ::nope2::` is not — var-1.12).  Reads, `info exists`, and unset
    /// stay plain misses.
    pub(crate) fn check_parent_ns(&mut self, name: &str) -> Result<()> {
        let var = match name.find('(') {
            Some(i) => &name[..i],
            None => name,
        };
        if !var.contains("::") {
            return Ok(());
        }
        let full = super::commands::namespace::qualify(&self.current_namespace, var);
        if full == "::" {
            return Ok(());
        }
        // Trailing run: the variable itself lives in `full`; otherwise the
        // variable lives in full's parent.
        let ns_to_check = if var.ends_with(':') {
            full
        } else {
            match full.rsplit_once("::") {
                Some((p, leaf)) if !leaf.is_empty() => Self::normalise_ns(p),
                _ => return Ok(()),
            }
        };
        if ns_to_check == "::" || self.namespaces.contains_key(&ns_to_check) {
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
        // A write through an alias whose target namespace/array was
        // destroyed errors (var-1.15/1.16/1.17); reads keep working.
        if let Some(elem) = self.dead_link_error(name) {
            let what = if elem {
                "element in deleted array"
            } else {
                "variable in deleted namespace"
            };
            return Err(Error::runtime(
                format!("can't set \"{}\": upvar refers to {}", name, what),
                ErrorCode::Generic,
            ));
        }
        self.check_parent_ns(name)?;
        if let Some((array_name, index)) = split_array_ref(name) {
            // Array-ification of a slotted name migrates the frame back to
            // the name-keyed store (element keys are flat map entries; the
            // slot model has no element form).
            self.degrade_frame_local_here(array_name);
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
                let base = name.split('(').next().unwrap_or(name);
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", base),
                );
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
            // Fast path: unqualified scalar write hitting an existing
            // variable in the innermost frame (or the global table at
            // "::" top level) — no upvar/dead link, not an array.  One
            // get_mut probe, zero allocations; creation, links, arrays
            // and qualified names keep the slow path below (same errors,
            // same trace firing).
            if !has_ns_sep(name) && self.dead_flat.is_empty() {
                let mut wrote = false;
                if let Some(frame) = self.frames.last_mut() {
                    if (frame.upvars.is_empty() || !frame.upvars.contains_key(name))
                        && (frame.array_locals.is_empty() || !frame.array_locals.contains(name))
                    {
                        match frame.slot_index_of(name) {
                            // Slotted name: write the cell (creation of an
                            // unset slot reaches here via the slow path's
                            // store_var, which consults the same table).
                            Some(i) => {
                                frame.slots[i] = Some(value.clone());
                                wrote = true;
                            }
                            None => {
                                if let Some(slot) = frame.locals.get_mut(name) {
                                    *slot = value.clone();
                                    wrote = true;
                                }
                            }
                        }
                    }
                } else if self.current_namespace.as_ref() == "::" && self.flat_aliases.is_empty() {
                    if self.array_globals.is_empty() || !self.array_globals.contains(name) {
                        if let Some(slot) = self.globals.get_mut(name) {
                            *slot = value.clone();
                            wrote = true;
                        }
                    }
                }
                if wrote {
                    if !self.var_traces.is_empty() {
                        self.fire_traces(name, None, "write").map_err(|e| {
                            Error::runtime(format!("can't set \"{}\": {}", name, e), ErrorCode::Generic)
                        })?;
                    }
                    return Ok(value);
                }
            }
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

    /// Take the variable's value out of its slot for in-place mutation
    /// (tclsh: mutate the refcnt==1 object).  Returns `None` unless
    /// every guard for the mutation fast path holds: unqualified scalar
    /// name, no upvar/dead link, not an array, no variable traces
    /// registered anywhere, and the variable currently exists.  The
    /// caller MUST put a value back (e.g. via `set_var`, which fires the
    /// write traces) before any user code runs — nothing observes the
    /// empty window.
    pub(crate) fn take_var_fast(&mut self, name: &str) -> Option<Value> {
        if has_ns_sep(name)
            || name.contains('(')
            || !self.var_traces.is_empty()
            || !self.dead_flat.is_empty()
        {
            return None;
        }
        if let Some(frame) = self.frames.last_mut() {
            if !frame.upvars.is_empty() && frame.upvars.contains_key(name) {
                return None;
            }
            if !frame.array_locals.is_empty() && frame.array_locals.contains(name) {
                return None;
            }
            match frame.slot_index_of(name) {
                // Slotted name: lift the value out of the cell; the caller
                // puts the replacement back (slot-None is the interim
                // state, same as the map-remove window).
                Some(i) => frame.slots.get_mut(i)?.take(),
                None => frame.locals.remove(name),
            }
        } else {
            if self.current_namespace.as_ref() != "::"
                || !self.flat_aliases.is_empty()
                || (!self.array_globals.is_empty() && self.array_globals.contains(name))
            {
                return None;
            }
            self.globals.remove(name)
        }
    }

    /// `incr` fast path: read-modify-write an integer scalar in one slot
    /// lookup, mutating the stored value in place when it is unshared
    /// (tclsh: TclIncrObj on a refcnt==1 object — no allocation, no string
    /// rendering).  `None` whenever any guard fails — the caller falls
    /// back to the real `incr`, which owns the exact errors (undefined
    /// variable, non-integer value, overflow), creation, and trace
    /// semantics.
    pub(crate) fn incr_var_fast(&mut self, name: &str, amount: i64) -> Option<Value> {
        if has_ns_sep(name)
            || name.contains('(')
            || !self.var_traces.is_empty()
            || !self.dead_flat.is_empty()
        {
            return None;
        }
        let slot = if let Some(frame) = self.frames.last_mut() {
            if !frame.upvars.is_empty() && frame.upvars.contains_key(name) {
                return None;
            }
            if !frame.array_locals.is_empty() && frame.array_locals.contains(name) {
                return None;
            }
            match frame.slot_index_of(name) {
                // Slotted name: unset cell → the real `incr` creates it
                // from 0 (its set_var consults the table again).
                Some(i) => frame.slots.get_mut(i)?.as_mut()?,
                None => frame.locals.get_mut(name)?,
            }
        } else {
            if self.current_namespace.as_ref() != "::"
                || !self.flat_aliases.is_empty()
                || (!self.array_globals.is_empty() && self.array_globals.contains(name))
            {
                return None;
            }
            self.globals.get_mut(name)?
        };
        let current = slot.as_int()?;
        // Overflow belongs to the real `incr` (ARITH IOVERFLOW error).
        let next = current.checked_add(amount)?;
        slot.set_int_rep(next);
        Some(slot.clone())
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
                    crate::interp::commands::list::set_error_code(
                        self,
                        &format!("TCL LOOKUP ELEMENT {}", idx),
                    );
                    Err(Error::runtime(
                        format!("can't unset \"{}\": no such element in array", name),
                        ErrorCode::NotFound,
                    ))
                }
            } else if self.loc_base_exists(&loc) {
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", base),
                );
                return Err(Error::runtime(
                    format!("can't unset \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ));
            } else {
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", base),
                );
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
                // Element aliases into this array now error on write
                // (var-1.17: `upvar 0 arr(1) foo; unset arr; set foo(3)` →
                // "element in deleted array").
                self.deaden_element_links(&loc);
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
                // set-old-7.7..7.9: removing the target retires every
                // frame's seeded mirror (stale reads) while the links
                // stay live so writes recreate the global.
                if let VarLoc::Global(k) = &loc {
                    let k = k.clone();
                    self.drop_global_seeds(&k);
                }
                let sk = stamp_key(&loc);
                let _ = self.fire_traces(&given, None, "unset");
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                Ok(())
            } else if self.current_namespace.as_ref() != "::" && self.ns_var_declared(&given) {
                // `variable`-declared but valueless (or declared with the
                // value living in the namespace's own slot): unset removes
                // the declaration and succeeds (tclsh 8.6.17).
                self.remove_var(&given);
                Ok(())
            } else {
                // tclsh deletes a missing variable's trace records too
                // (`unset` on an untraced-existence variable still wipes
                // its registrations — trace-14.17/14.19/33.1).
                let sk = stamp_key(&loc);
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", name),
                );
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
                && self.current_namespace.as_ref() != "::"
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

    /// Locate the scope that owns the array `name` refers to, following
    /// upvar links to the ultimate owner: returns `(frame_index, base)`
    /// where `frame_index = None` means the global table and `base` is
    /// the array's key there (`upvar a x` at eval level names the
    /// aliased target, ns-qualified).  Callers enumerate elements with
    /// `base(`-prefixed keys in the returned table.
    pub(crate) fn array_owner(&self, name: &str) -> (Option<usize>, String) {
        let mut scope: Option<usize> = self.frames.last().map(|_| self.frames.len() - 1);
        let mut base = match scope {
            Some(_) => name.to_string(),
            None => self.canonical_global(name),
        };
        // Follow upvar links transitively (with a hop bound so a link
        // cycle can't spin).
        for _ in 0..=self.frames.len() {
            let next = match scope {
                Some(fi) => self
                    .frames
                    .get(fi)
                    .and_then(|f| f.upvars.get(base.as_str()))
                    .cloned(),
                None => self
                    .flat_aliases
                    .iter()
                    .find(|(k, _)| *k == base)
                    .map(|(_, t)| UpvarLink::Global(t.clone())),
            };
            match next {
                Some(UpvarLink::Global(g)) => {
                    scope = None;
                    base = g;
                }
                Some(UpvarLink::Frame {
                    frame_index,
                    var_name,
                }) => {
                    scope = Some(frame_index);
                    base = var_name;
                }
                // A dead link terminates the chain (var-1.17 semantics:
                // the alias stands on its own seeded copy).
                Some(UpvarLink::Dead { .. }) => break,
                None => break,
            }
        }
        (scope, base)
    }

    /// Fire variable traces for `op` on (base, elem). Callback scripts run
    /// in the current scope with `name1 name2 op` appended (tclsh passes
    /// name2 as an empty string for whole-variable ops).
    pub(crate) fn fire_traces(&mut self, base: &str, elem: Option<&str>, op: &str) -> Result<()> {
        let key = self.array_stamp_key(base);
        let mut scripts: Vec<String> = Vec::new();
        let scripts = self.collect_trace_scripts(&key, elem, op);
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

    /// Trace scripts registered for `op` on (base-key `key`, optional
    /// element): whole-var traces first, then the element's own.
    fn collect_trace_scripts(
        &self,
        key: &str,
        elem: Option<&str>,
        op: &str,
    ) -> Vec<String> {
        let mut scripts: Vec<String> = Vec::new();
        if let Some(trs) = self.var_traces.get(key) {
            for t in trs {
                if t.ops.iter().any(|o| o == op) {
                    scripts.push(t.script.clone());
                }
            }
        }
        if let Some(e) = elem {
            if let Some(map) = self.elem_traces.get(key) {
                if let Some(trs) = map.get(e) {
                    for t in trs {
                        if t.ops.iter().any(|o| o == op) {
                            scripts.push(t.script.clone());
                        }
                    }
                }
            }
        }
        scripts
    }

    /// Read one element for `array get`: fire its read traces one script
    /// at a time (pt1: `array get x` fires `{x a read} {x b read}`) with
    /// tclsh's interleaved checks (probes P1-P7, trace-1.11..14):
    ///  - the array container gone right after a script → the whole get
    ///    aborts `can't read "base(elem)": no such variable` and later
    ///    traces never run (1.13/1.14: the set-foo trace after unset-x
    ///    never fires, x stays gone);
    ///  - the container destroyed AND recreated by the callbacks (fresh
    ///    base-key value) → the snapshot element is skipped even when a
    ///    same-named element exists again (P2: recreated `bar 9` invisible);
    ///  - a callback error is swallowed and skips the element (P6:
    ///    `error BOOM` trace → get returns {}) without firing the rest;
    ///  - otherwise the element's CURRENT value is used (P5: trace set
    ///    `bar 5` → get shows 5), a simply-unset element is skipped (P1).
    pub(crate) fn array_get_element(
        &mut self,
        owner_fi: Option<usize>,
        base: &str,
        elem: &str,
    ) -> std::result::Result<Option<Value>, Error> {
        let key = self.array_stamp_key(base);
        let gen0 = self.array_generations.get(&key).copied();
        let alive = |t: &Self| match owner_fi {
            Some(i) => t.frames.get(i).map(|f| f.locals.contains_key(base)),
            None => Some(t.globals.contains_key(base)),
        };
        let scripts = self.collect_trace_scripts(&key, Some(elem), "read");
        let q = |v: &str| Value::from_list(&[Value::from_str(v)]).as_str().to_string();
        let mut recreated = false;
        for s in scripts.into_iter().rev() {
            let cmd = format!("{} {} {} {}", s, q(base), q(elem), "read");
            if self.eval(&cmd).is_err() {
                return Ok(None);
            }
            match alive(self) {
                None | Some(false) => {
                    crate::interp::commands::list::set_error_code(self, "TCL READ VARNAME");
                    return Err(Error::runtime(
                        format!("can't read \"{}({})\": no such variable", base, elem),
                        ErrorCode::Generic,
                    ));
                }
                Some(true) => {}
            }
            if self.array_generations.get(&key).copied() != gen0 {
                recreated = true;
            }
        }
        if recreated {
            return Ok(None);
        }
        let ekey = format!("{}({})", base, elem);
        Ok(match owner_fi {
            Some(i) => self.frames.get(i).and_then(|f| f.locals.get(&ekey)).cloned(),
            None => self.globals.get(&ekey).cloned(),
        })
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

    /// Mark every upvar alias pointing INTO the array at `loc` (any
    /// element of it) dead: writes through those aliases now error with
    /// "upvar refers to element in deleted array" (var-1.17).
    fn deaden_element_links(&mut self, loc: &VarLoc) {
        let (scope_idx, base) = match loc {
            VarLoc::Global(n) => (None, n.clone()),
            VarLoc::Frame(i, n) => (Some(*i), n.clone()),
        };
        let elem_prefix = format!("{}(", base);
        for f in self.frames.iter_mut() {
            for link in f.upvars.values_mut() {
                match link {
                    UpvarLink::Frame { frame_index, var_name }
                        if Some(*frame_index) == scope_idx
                            && var_name.starts_with(&elem_prefix) =>
                    {
                        *link = UpvarLink::Dead {
                            array: true,
                            target: var_name.clone(),
                        };
                    }
                    UpvarLink::Global(g) if scope_idx.is_none() && g.starts_with(&elem_prefix) => {
                        *link = UpvarLink::Dead {
                            array: true,
                            target: g.clone(),
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    /// `namespace delete <prefix>`: every alias (frame upvar or eval-level
    /// flat alias) whose target lives under the deleted namespace turns
    /// dead — reads keep the seeded copy, writes error (var-1.15/1.16).
    /// Variables a `variable` command declared at eval level are exempt:
    /// the declaring scope still references them.
    pub(crate) fn deaden_links_under(&mut self, var_prefix: &str) {
        let aliases: Vec<(String, String)> = self.flat_aliases.clone();
        for (k, t) in aliases {
            if t.starts_with(var_prefix) && !self.ns_variable_links.iter().any(|x| *x == t) {
                self.dead_flat.push(k);
            }
        }
        // Aliases living IN the deleted namespace's variable table die
        // with it (an `upvar` inside `namespace eval ns` links ns's own
        // variable).
        self.flat_aliases.retain(|(k, _)| !k.starts_with(var_prefix));
        for f in self.frames.iter_mut() {
            for link in f.upvars.values_mut() {
                if let UpvarLink::Global(g) = link {
                    if g.starts_with(var_prefix) {
                        *link = UpvarLink::Dead {
                            array: false,
                            target: g.clone(),
                        };
                    }
                }
            }
        }
    }

    /// Read a variable, firing read traces the way tclsh does: scalar
    /// reads fire even when the variable is missing; element reads fire
    /// when the element exists OR the array itself exists — a missing
    /// element read on a live array fires the whole-var read trace, whose
    /// callback may create the element (trace-1.7).  No fire when the
    /// array is missing or the base is a scalar (trace-1.8).
    pub fn read_var(&mut self, name: &str) -> Result<Value> {
        // Fast path: scalar read with no variable traces registered — the
        // loop-body `$x` case.  The base-name allocation and the read
        // trace pass are both no-ops without traces, so skip them whole
        // (tclsh: scalar reads take the var-table direct hit).
        if !name.contains('(') && self.var_traces.is_empty() {
            return self.get_var(name).cloned();
        }
        let (base, idx) = match split_array_ref(name) {
            Some((b, i)) => (b.to_string(), Some(i.to_string())),
            None => (name.to_string(), None),
        };
        let fire = match &idx {
            None => true,
            Some(_) => self.is_array_semantic(&base),
        };
        if fire {
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
        let fire = match &idx {
            None => true,
            Some(_) => self.is_array_semantic(&base),
        };
        if fire {
            let _ = self.fire_traces(&base, idx.as_deref(), "read");
        }
        self.var_exists(name)
    }

    /// Force array semantics at `name` (marker + base key) without adding
    /// elements — `array set x {}` on a missing variable still creates it.
    pub(crate) fn mark_array(&mut self, name: &str) -> Result<()> {
        self.check_parent_ns(name)?;
        // Array-ification degrades (element keys are flat map entries).
        self.degrade_frame_local_here(name);
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
        // tclsh: a missing variable read stamps `TCL LOOKUP VARNAME` (no
        // ARITH code); a missing variable WRITE doesn't exist (set
        // creates), and a wholly fresh interp has no errorCode at all.
        assert_eq!(
            interp_eval("catch {set nosuchvar} m; set ::errorCode"),
            "TCL LOOKUP VARNAME nosuchvar"
        );
        assert_eq!(
            interp_eval("set created 5; catch {set ::errorCode} m; set m"),
            "can't read \"::errorCode\": no such variable"
        );
    }
}
