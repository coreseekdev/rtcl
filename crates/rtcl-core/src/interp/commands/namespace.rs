//! Namespace support for the rtcl interpreter.
//!
//! Tcl namespaces provide hierarchical scoping for commands and variables.
//! Namespace names are separated by `::`.  The global namespace is `::`.
//!
//! Name-resolution rules derived from tclsh 8.6.17:
//! * Runs of two or more colons are a single separator (`:::a::::b` and
//!   `a::b` denote the same namespace) — all resolution goes through
//!   [`normalise`].
//! * `namespace qualifiers`/`tail` split at the *final colon run* without
//!   otherwise transforming the string.
//! * Not-found messages depend on the subcommand: absolute names get a
//!   bare `namespace "::x" not found`, relative names the `... in "::y"`
//!   form; delete/forget/import have their own wordings.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::interp::commands::proc::move_alias_origins;
use crate::value::Value;

/// Metadata for a single namespace.
#[derive(Debug, Clone, Default)]
pub(crate) struct NamespaceInfo {
    pub export_patterns: Vec<String>,
    /// Variables declared in this namespace (canonical flat keys, no
    /// leading `::`).  A bare `variable x` registers the name without
    /// creating storage — `namespace which -variable x` still resolves it,
    /// while `info exists` stays 0 (tclsh 8.6.17).
    pub variables: std::collections::HashSet<String>,
}

/// The full subcommand table (tclsh 8.6.17) — also the order used in the
/// `unknown or ambiguous subcommand` message.
const NS_SUBCOMMANDS: [&str; 19] = [
    "children", "code", "current", "delete", "ensemble", "eval", "exists",
    "export", "forget", "import", "inscope", "origin", "parent", "path",
    "qualifiers", "tail", "unknown", "upvar", "which",
];

/// `a, b, ..., or z` — tclsh's list rendering in error messages.
fn join_or(items: &[&str]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].to_string(),
        n => {
            let head = items[..n - 1].join(", ");
            format!("{}, or {}", head, items[n - 1])
        }
    }
}

/// Resolve a (possibly abbreviated) namespace subcommand.  Exact matches
/// win; otherwise a unique prefix is accepted (tclsh `namespace ch` →
/// `children`).
fn resolve_subcmd(sub: &str) -> Result<&'static str> {
    if let Some(exact) = NS_SUBCOMMANDS.iter().find(|s| **s == sub) {
        return Ok(exact);
    }
    let matches: Vec<&str> = NS_SUBCOMMANDS
        .iter()
        .filter(|s| s.starts_with(sub))
        .copied()
        .collect();
    if matches.len() == 1 {
        return Ok(matches[0]);
    }
    // tclsh lists only the matching candidates (all of them when none do)
    let candidates: &[&str] = if matches.is_empty() { &NS_SUBCOMMANDS } else { &matches };
    Err(Error::runtime(
        format!(
            "unknown or ambiguous subcommand \"{}\": must be {}",
            sub,
            join_or(candidates)
        ),
        ErrorCode::InvalidOp,
    ))
}

// ── namespace command ──────────────────────────────────────────────────

/// `namespace subcommand ?arg ...?`
pub fn cmd_namespace(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "namespace", 2, args.len(),
            "subcommand ?arg ...?",
        ));
    }
    let subcmd = resolve_subcmd(args[1].as_str())?;
    match subcmd {
        "children"   => ns_children(interp, args),
        "code"       => ns_code(interp, args),
        "current"    => ns_current(interp, args),
        "delete"     => ns_delete(interp, args),
        "ensemble"   => ns_ensemble(interp, args),
        "eval"       => ns_eval(interp, args),
        "exists"     => ns_exists(interp, args),
        "export"     => ns_export(interp, args),
        "forget"     => ns_forget(interp, args),
        "import"     => ns_import(interp, args),
        "inscope"    => ns_inscope(interp, args),
        "origin"     => ns_origin(interp, args),
        "parent"     => ns_parent(interp, args),
        "path"       => ns_path(interp, args),
        "qualifiers" => ns_qualifiers(args),
        "tail"       => ns_tail(args),
        "unknown"    => ns_unknown(interp, args),
        "upvar"      => ns_upvar(interp, args),
        "which"      => ns_which(interp, args),
        _ => unreachable!("resolve_subcmd validated"),
    }
}

/// Canonical flat key for a namespace-qualified *variable*: `qualify()`
/// yields `::ns::name`, but variable slots live in `Interp::globals`
/// alongside plain globals (`errorCode`, ...) keyed without the leading
/// `::` — matching the normalization `set ::ns::v` goes through in
/// `vars.rs`.
fn var_key(qualified: &str) -> String {
    qualified
        .strip_prefix("::")
        .unwrap_or(qualified)
        .to_string()
}

/// The local alias `variable`/`global` creates: everything after the last
/// run of two or more colons (`a::b:c` → `b:c`, `ns::` → the empty name).
/// Single colons are name characters.
pub(crate) fn split_var_tail(name: &str) -> &str {
    match name.rfind("::") {
        None => name,
        Some(i) => &name[i + 2..],
    }
}

/// `variable ?name ?value? ...?`
pub fn cmd_variable(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        // Bare `variable` is a no-op (var-7.16, var-7.17).
        return Ok(Value::empty());
    }

    let ns = interp.current_namespace.clone();
    let mut i = 1;
    while i < args.len() {
        let raw_name = args[i].as_str();
        // Flat key: normalise collapses colon runs, but a TRAILING run
        // names the empty variable in that namespace, so it is restored
        // (`variable test_ns_var::` declares test_ns_var's "" variable —
        // var-7.12; `variable :` is a plain name — var-7.13).
        let qualified = if raw_name.is_empty() {
            if ns == "::" {
                String::new()
            } else {
                var_key(&format!("{}::", ns))
            }
        } else {
            let mut key = var_key(&qualify(&ns, raw_name));
            if raw_name.ends_with(':') && !key.ends_with(':') {
                key.push_str("::");
            }
            key
        };

        // The variable's namespace must exist (`can't define "<as-typed>":
        // parent namespace doesn't exist` — var-7.7, var-1.11's family).
        let ns_part = if raw_name.is_empty() {
            ns.clone()
        } else if raw_name.ends_with("::") {
            normalise(&format!("::{}", &qualified))
        } else {
            parent_of(&normalise(&format!("::{}", &qualified)))
        };
        if ns_part != "::" && !interp.namespaces.contains_key(&ns_part) {
            crate::interp::commands::list::set_error_code(
                interp,
                &format!("TCL LOOKUP VARNAME {}", raw_name),
            );
            return Err(Error::runtime(
                format!(
                    "can't define \"{}\": parent namespace doesn't exist",
                    raw_name
                ),
                ErrorCode::Generic,
            ));
        }

        // If an initial value is provided, set it.  A bare `variable name`
        // declares the link without creating the variable (tclsh:
        // `namespace eval n {variable v}; info exists n::v` → 0).
        let init_value = if i + 1 < args.len() {
            let val = args[i + 1].clone();
            interp.globals.insert(qualified.clone(), val);
            i += 2;
            Some(args[i - 1].clone())
        } else {
            i += 1;
            None
        };

        // Register the name in the OWNING namespace's variable table.
        if let Some(info) = interp.namespaces.get_mut(&ns_part) {
            info.variables.insert(qualified.clone());
        }

        // If we're inside a proc, create an upvar link from the local
        // name to the namespace-qualified global name.  An initial value
        // also seeds a real local: the tclsh link is a refcounted shared
        // Var, so the local keeps the value even if the namespace (and its
        // variable) is deleted while the proc runs (46.8).
        if interp.frames.is_empty() {
            // At eval level tclsh's `variable` links the ns-eval varFrame
            // to the namespace Var: the variable stays readable for the
            // rest of the body even if the namespace deletes itself
            // (var-1.16/1.17).  Model the link as a bare-name redirect +
            // keep-alive marker (dropped when this `namespace eval`
            // exits).
            if !raw_name.contains("::") {
                let existing = interp
                    .flat_aliases
                    .iter_mut()
                    .find(|(k, _)| *k == raw_name);
                match existing {
                    Some((_, t)) => *t = qualified.clone(),
                    None => interp
                        .flat_aliases
                        .push((raw_name.to_string(), qualified.clone())),
                }
                if !interp.ns_variable_links.iter().any(|x| x == &qualified) {
                    interp.ns_variable_links.push(qualified.clone());
                }
            }
        } else {
            let local_name = split_var_tail(raw_name).to_string();
            let frame_idx = interp.frames.len() - 1;
            interp.frames[frame_idx].upvars.insert(
                local_name.clone(),
                crate::interp::UpvarLink::Global(qualified.clone()),
            );
            // The alias name is in the proc's variable table (tclsh's
            // `info vars` lists linked variables — var-7.12's `{{}}`,
            // var-7.13's `:`): seed a placeholder when a copy exists to
            // mirror (an initial value, or the namespace variable is
            // already set).  A bare declaration of a not-yet-set variable
            // stays invisible (tclsh: `info exists` → 0).
            let seeded = match &init_value {
                Some(val) => Some(val.clone()),
                None => interp.globals.get(&qualified).map(|v| v.clone()),
            };
            if let Some(val) = seeded {
                interp.frames[frame_idx].locals.insert(local_name, val);
            }
        }
    }
    Ok(Value::empty())
}

// ── subcommand implementations ─────────────────────────────────────────

/// `namespace eval name arg ?arg...?`
fn ns_eval(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace eval", 4, args.len(),
            "name arg ?arg...?",
        ));
    }

    let ns_name = args[2].as_str();
    let qualified = qualify(&interp.current_namespace, ns_name);

    // Ensure the namespace exists (create it and all ancestors)
    ensure_namespace(&mut interp.namespaces, &qualified);

    // Concatenate remaining args into the body
    let body = if args.len() == 4 {
        args[3].as_str().to_string()
    } else {
        args[3..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };

    // Push namespace context.  tclsh's namespace eval pushes a varFrame
    // visible to `info level 0` as the ns-eval command's source text
    // (25.9), and its exit appends
    // `(in namespace eval "<qualified>" script line N)` on error (25.6).
    interp.ns_level0.push(interp.cur_cmd_text.clone());
    interp.ns_stack.push(qualified.clone());
    // Marks for the eval-level variable links this body creates — they
    // die with it (tclsh's ns-eval varFrame pops).
    let prev = std::mem::replace(&mut interp.current_namespace, qualified.clone());
    let result = interp.eval(&body);
    interp.current_namespace = prev;
    interp.ns_stack.pop();
    interp.ns_level0.pop();
    // Drop the variable links that belong to THIS namespace's variable
    // table: `variable`-declared bare-name links (transient — created for
    // this body) and any kept-alive variables of a self-deleted
    // namespace.  `upvar` links persist with the namespace itself
    // (tclsh: upvar inside `namespace eval` links the namespace's own
    // variable — var-3.10's `set foo::bar` still redirects afterwards).
    let flat_prefix = var_key(&format!("{}::", qualified));
    if !flat_prefix.is_empty() {
        interp.flat_aliases.retain(|(k, t)| {
            !(t.starts_with(&flat_prefix) && !k.contains("::"))
        });
        interp.ns_variable_links.retain(|t| !t.starts_with(&flat_prefix));
        interp.dead_flat.retain(|k| !k.starts_with(&flat_prefix));
        let mut i = 0;
        while i < interp.ns_eval_keep.len() {
            if interp.ns_eval_keep[i].starts_with(&flat_prefix) {
                let key = interp.ns_eval_keep.swap_remove(i);
                interp.globals.remove(&key);
            } else {
                i += 1;
            }
        }
    }
    if let Err(e) = &result {
        if interp.err_is_error(e) {
            let tag = format!("in namespace eval \"{}\" script", qualified);
            interp.err_exit_frame(&tag);
        }
    }
    result
}

/// `namespace current` — no arguments allowed.
fn ns_current(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage(
            "namespace current", 2, args.len(),
            "",
        ));
    }
    Ok(Value::from_str(&interp.current_namespace))
}

/// `namespace delete ?name ...?`
///
/// Each name must exist (message names the argument *as typed*); `::`
/// itself is silently ignored (tclsh: rc 0, no error).
fn ns_delete(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    for arg in &args[2..] {
        let typed = arg.as_str();
        let qualified = qualify(&interp.current_namespace, typed);
        if qualified == "::" {
            continue;
        }
        if !interp.namespaces.contains_key(&qualified) {
            return Err(Error::runtime(
                format!("unknown namespace \"{}\" in namespace delete command", typed),
                ErrorCode::NotFound,
            ));
        }
        // Remove the namespace and all children
        let prefix = format!("{}::", qualified);
        interp.namespaces.retain(|k, _| k != &qualified && !k.starts_with(&prefix));
        // ...and any names its ancestors declared pointing into it
        if let Some(parent) = interp.namespaces.get_mut("::") {
            let child_prefix = var_key(&prefix);
            parent.variables.retain(|v| !v.starts_with(&child_prefix));
        }

        // The *initiating* frame keeps the deleted name as its current
        // namespace until it exits (tclsh: after
        // `namespace delete [namespace current]`, `namespace current`
        // still reports the deleted namespace both in the eval body and in
        // a proc frame; `namespace exists` is 0 for it).  Only the delete
        // trace callbacks below run fresh in `::` (34.4/34.5).
        let inside = interp.current_namespace == qualified
            || interp.current_namespace.starts_with(&prefix);

        let var_prefix = var_key(&prefix);

        // Aliases pointing into the dying tree go dead first: reads keep
        // the seeded local copy, writes error (var-1.15/1.16).
        interp.deaden_links_under(&var_prefix);

        // ── Variables go first (tclsh order) ──
        // Each variable is removed BEFORE its unset trace fires, and the
        // not-yet-processed variables are still visible to `info vars`
        // during the callback (18.4's count).  Element keys collapse into
        // their base (tclsh unsets the whole Var once).
        let mut bases: Vec<String> = Vec::new();
        for k in interp.globals.keys() {
            if !k.starts_with(&var_prefix) {
                continue;
            }
            let base = match k.find('(') {
                Some(i) => k[..i].to_string(),
                None => k.clone(),
            };
            if !bases.contains(&base) {
                bases.push(base);
            }
        }
        bases.sort();
        let mut kept: Vec<String> = Vec::new();
        for base in bases {
            // A `variable`-declared (at eval level) variable outlives the
            // namespace deletion for the rest of the declaring body
            // (var-1.16/1.17 read it after the self-delete).
            if interp.ns_variable_links.iter().any(|x| *x == base) {
                interp.ns_eval_keep.push(base.clone());
                kept.push(base);
                continue;
            }
            let elem_prefix = format!("{}(", base);
            let keys: Vec<String> = interp
                .globals
                .keys()
                .filter(|k| *k == &base || k.starts_with(&elem_prefix))
                .cloned()
                .collect();
            for k in keys {
                interp.globals.remove(&k);
            }
            interp.array_globals.remove(&base);
            // Unset traces fire after the removal; a callback's `set` into
            // the deleted namespace fails its parent-namespace check
            // (18.4's catch=1) and cannot resurrect the variable.
            let _ = interp.fire_traces(&format!("::{}", base), None, "unset");
        }
        // Trace registrations and searches living under the tree die with
        // it.
        let stamp_prefix = format!("G:{}", var_prefix);
        interp.var_traces.retain(|k, _| !k.starts_with(&stamp_prefix));
        interp.elem_traces.retain(|k, _| !k.starts_with(&stamp_prefix));
        interp.trace_phantoms.retain(|k, _| !k.starts_with(&stamp_prefix));
        interp.array_searches.retain(|k, _| !k.starts_with(&stamp_prefix));
        interp.array_stamps.retain(|k, _| !k.starts_with(&stamp_prefix));

        // ── Command delete traces ──
        // When the deletion was initiated from OUTSIDE the tree the
        // namespace still resolves during the trace (`namespace which
        // -command` finds the command — 34.4); from inside, the commands
        // are already gone (`namespace which` is empty — 34.5).
        let mut dead_procs: Vec<String> = interp
            .procs
            .keys()
            .filter(|k| *k == &qualified || k.starts_with(&prefix))
            .cloned()
            .collect();
        dead_procs.sort();
        if inside {
            interp.procs.retain(|k, _| k != &qualified && !k.starts_with(&prefix));
        }
        let saved_ns = Some(std::mem::replace(&mut interp.current_namespace, "::".to_string()));
        for key in &dead_procs {
            interp.fire_cmd_traces(key, key, "", "delete");
        }
        if let Some(ns) = saved_ns {
            interp.current_namespace = ns;
        }
        if !inside {
            interp.procs.retain(|k, _| k != &qualified && !k.starts_with(&prefix));
        }

        // Remove namespace-scoped global variables (flat keys, no leading
        // ::), except the ones a live variable link keeps alive.
        interp
            .globals
            .retain(|k, _| !k.starts_with(&var_prefix) || kept.iter().any(|b| b == k));

        // Remove import aliases defined in, or pointing into, the tree.
        interp.import_aliases.retain(|alias, origin| {
            !alias.starts_with(&prefix) && !origin.starts_with(&prefix)
        });
        interp.ns_unknown.retain(|k, _| k != &qualified && !k.starts_with(&prefix));
        // Ensembles registered in (or under) the tree; an ensemble whose
        // BACKING namespace goes away goes too.
        interp.ensembles.retain(|k, def| {
            !k.starts_with(&prefix)
                && def.namespace != qualified
                && !def.namespace.starts_with(&prefix)
        });
    }
    Ok(Value::empty())
}

/// `namespace exists name`
fn ns_exists(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace exists", 3, args.len(),
            "name",
        ));
    }
    let qualified = qualify(&interp.current_namespace, args[2].as_str());
    Ok(Value::from_bool(interp.namespaces.contains_key(&qualified)))
}

/// Validate that a named namespace exists, returning its qualified name.
///
/// tclsh distinguishes absolute names (bare `namespace "::x" not found`)
/// from relative ones (`namespace "x" not found in "::y"`).
fn require_ns(interp: &Interp, raw: &str) -> Result<String> {
    let qualified = qualify(&interp.current_namespace, raw);
    if interp.namespaces.contains_key(&qualified) {
        return Ok(qualified);
    }
    if raw.starts_with("::") {
        Err(Error::runtime(
            format!("namespace \"{}\" not found", raw),
            ErrorCode::NotFound,
        ))
    } else {
        Err(Error::runtime(
            format!(
                "namespace \"{}\" not found in \"{}\"",
                raw, interp.current_namespace
            ),
            ErrorCode::NotFound,
        ))
    }
}

/// `namespace parent ?name?` — the parent of `::` is the empty string
/// (not an error; tclsh 8.6.17).
fn ns_parent(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace parent", 3, args.len(),
            "?name?",
        ));
    }
    let ns = if args.len() == 3 {
        require_ns(interp, args[2].as_str())?
    } else {
        interp.current_namespace.clone()
    };
    if ns == "::" {
        return Ok(Value::empty());
    }
    Ok(Value::from_str(&parent_ns(&ns)))
}

/// `namespace children ?name? ?pattern?`
///
/// The pattern matches the child's *simple name* (tclsh: `namespace
/// children foo b*` lists foo's children whose tail starts with b).
fn ns_children(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace children", 4, args.len(),
            "?name? ?pattern?",
        ));
    }
    let ns = if args.len() >= 3 {
        require_ns(interp, args[2].as_str())?
    } else {
        interp.current_namespace.clone()
    };
    let pattern = if args.len() >= 4 { Some(args[3].as_str()) } else { None };

    let prefix = if ns == "::" { "::".to_string() } else { format!("{}::", ns) };
    let mut children = Vec::new();
    for key in interp.namespaces.keys() {
        if key == &ns { continue; }
        // Direct child: starts with prefix and no further `::`
        if let Some(tail) = key.strip_prefix(&prefix) {
            if !tail.contains("::") {
                if let Some(pat) = pattern {
                    if crate::interp::glob_match(pat, tail) {
                        children.push(key.as_str());
                    }
                } else {
                    children.push(key.as_str());
                }
            }
        }
    }
    children.sort();
    Ok(Value::from_str(&children.join(" ")))
}

/// `namespace qualifiers string` — everything before the final colon run.
fn ns_qualifiers(args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace qualifiers", 3, args.len(),
            "string",
        ));
    }
    Ok(Value::from_str(split_colon_run(args[2].as_str()).0))
}

/// `namespace tail string` — everything after the final colon run.
fn ns_tail(args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace tail", 3, args.len(),
            "string",
        ));
    }
    Ok(Value::from_str(split_colon_run(args[2].as_str()).1))
}

/// `namespace which ?-command|-variable? name`
fn ns_which(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace which", 3, args.len(),
            "?-command? ?-variable? name",
        ));
    }

    let (kind, name) = if args.len() == 4 {
        let flag = args[2].as_str();
        match flag {
            "-command"  => ("command", args[3].as_str()),
            "-variable" => ("variable", args[3].as_str()),
            _ => return Err(Error::wrong_args_with_usage(
                "namespace which", 3, args.len(),
                "?-command? ?-variable? name",
            )),
        }
    } else {
        ("command", args[2].as_str())  // default is -command
    };

    let qualified = qualify(&interp.current_namespace, name);

    match kind {
        "command" => {
            if interp.procs.contains_key(&qualified)
                || interp.import_aliases.contains_key(&qualified)
            {
                Ok(Value::from_str(&qualified))
            } else if interp.commands.contains_key(&qualified) {
                Ok(Value::from_str(&qualified))
            } else if interp.procs.contains_key(name)
                || interp.import_aliases.contains_key(name)
                || interp.commands.contains_key(name)
            {
                Ok(Value::from_str(&qualify("::", name)))
            } else {
                // Builtins are keyed unqualified: `::set` → `::set`
                let stripped = qualified.strip_prefix("::").unwrap_or(&qualified);
                if !stripped.contains("::") && interp.commands.contains_key(stripped) {
                    Ok(Value::from_str(&qualified))
                } else {
                    Ok(Value::empty())
                }
            }
        }
        "variable" => {
            let key = var_key(&qualified);
            let declared = interp
                .namespaces
                .get(&interp.current_namespace)
                .map(|info| info.variables.contains(&key))
                .unwrap_or(false);
            if interp.globals.contains_key(&key) || declared {
                Ok(Value::from_str(&qualified))
            } else if interp.globals.contains_key(name) {
                Ok(Value::from_str(&qualify("::", name)))
            } else {
                Ok(Value::empty())
            }
        }
        _ => unreachable!(),
    }
}

/// `namespace origin name` — follow the import-alias chain to the original
/// command.  Builtins report their `::`-qualified name; unknown commands
/// raise `invalid command name "..."`.
fn ns_origin(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace origin", 3, args.len(),
            "name",
        ));
    }
    match origin_of(interp, args[2].as_str()) {
        Some(full) => Ok(Value::from_str(&full)),
        None => Err(Error::invalid_command(args[2].as_str())),
    }
}

/// `namespace code arg` — build `::namespace inscope <ns> <arg>` as a
/// proper Tcl list (multi-word args get braced by the list formatting).
fn ns_code(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace code", 3, args.len(),
            "arg",
        ));
    }
    // An already-scoped value passes through unchanged — deliberately
    // unforgiving, only [::namespace code]'s own output style qualifies
    // (tclsh bug 3202171).
    let arg = args[2].as_str();
    if arg.starts_with("::namespace inscope ") && arg.len() > 20 {
        return Ok(args[2].clone());
    }
    Ok(Value::from_list(&[
        Value::from_str("::namespace"),
        Value::from_str("inscope"),
        Value::from_str(&interp.current_namespace.clone()),
        args[2].clone(),
    ]))
}

/// `namespace export ?-clear? ?pattern ...?` — patterns must not contain
/// `::`.  With no patterns, returns the current export list.
fn ns_export(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let ns = interp.current_namespace.clone();
    let mut clear = false;
    let mut given: Vec<String> = Vec::new();
    let mut i = 2;
    if i < args.len() && args[i].as_str() == "-clear" {
        clear = true;
        i += 1;
    }
    for arg in &args[i..] {
        let pat = arg.as_str();
        if pat.contains("::") {
            return Err(Error::runtime(
                format!(
                    "invalid export pattern \"{}\": pattern can't specify a namespace",
                    pat
                ),
                ErrorCode::InvalidOp,
            ));
        }
        given.push(pat.to_string());
    }

    let info = interp.namespaces.entry(ns).or_default();
    if clear {
        info.export_patterns.clear();
    }
    if given.is_empty() {
        // No patterns: report the current list
        return Ok(Value::from_list(
            &info.export_patterns.iter().map(|s| Value::from_str(s)).collect::<Vec<_>>(),
        ));
    }
    info.export_patterns.extend(given);
    Ok(Value::empty())
}

/// `namespace import ?-force? ?pattern ...?`
///
/// Only commands listed in the source namespace's export table are
/// imported.  Imported commands are recorded as aliases whose body always
/// dispatches to the origin (visible after `proc` redefinition).
fn ns_import(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut i = 2;
    let mut force = false;
    if i < args.len() && args[i].as_str() == "-force" {
        force = true;
        i += 1;
    }
    let cur = interp.current_namespace.clone();

    // `namespace import` with no patterns lists this namespace's
    // imported commands (namespace-old-9.5).
    if i >= args.len() {
        let prefix = ns_child_prefix(&cur);
        let mut names: Vec<String> = interp
            .import_aliases
            .keys()
            .filter_map(|k| k.strip_prefix(prefix.as_str()))
            .filter(|r| !r.contains("::"))
            .map(|s| s.to_string())
            .collect();
        names.sort();
        return Ok(Value::from_list(
            &names.iter().map(|s| Value::from_str(s)).collect::<Vec<_>>(),
        ));
    }

    for pat in &args[i..] {
        let p = pat.as_str();
        if p.is_empty() {
            return Err(Error::runtime(
                "empty import pattern".to_string(),
                ErrorCode::InvalidOp,
            ));
        }
        let qualified = qualify(&cur, p);
        let src = {
            let q = ns_qualifiers_str(&qualified);
            if q.is_empty() { "::".to_string() } else { q.to_string() }
        };
        // Tail comes from the pattern AS TYPED: `::e1::` has an empty tail
        // (imports nothing, silently) even though it resolves to `::e1`.
        let tail = ns_tail_str(p).to_string();

        if !interp.namespaces.contains_key(&src) {
            return Err(Error::runtime(
                format!("unknown namespace in import pattern \"{}\"", p),
                ErrorCode::NotFound,
            ));
        }
        if src == cur {
            // tclsh names the source namespace by its simple name
            let simple = ns_tail_str(&src).to_string();
            return Err(Error::runtime(
                format!(
                    "import pattern \"{}\" tries to import from namespace \"{}\" into itself",
                    p, simple
                ),
                ErrorCode::InvalidOp,
            ));
        }

        let exports = interp
            .namespaces
            .get(&src)
            .map(|info| info.export_patterns.clone())
            .unwrap_or_default();
        let prefix = ns_child_prefix(&src);
        // Candidates: the source's own procs plus its imported aliases —
        // an imported command re-exported by the source (`namespace
        // export *`) is imported as a NEW alias on the ultimate origin
        // (tclsh 8.6.17 namespace-22.1 chain).  Ensemble commands import
        // too (48.1: `namespace import foo::bar` where bar is an ensemble).
        let proc_keys: Vec<String> = interp.procs.keys().cloned().collect();
        let alias_keys: Vec<String> = interp.import_aliases.keys().cloned().collect();
        let ens_keys: Vec<String> = interp.ensembles.keys().cloned().collect();
        let matching: Vec<(String, String)> = proc_keys
            .iter()
            .chain(alias_keys.iter())
            .chain(ens_keys.iter())
            .filter(|k| {
                k.strip_prefix(&prefix)
                    .filter(|rest| !rest.contains("::") && crate::interp::glob_match(&tail, rest))
                    .filter(|rest| exports.iter().any(|e| crate::interp::glob_match(e, rest)))
                    .is_some()
            })
            .map(|k| (ns_tail_str(k).to_string(), k.clone()))
            .collect();

        // First pass: classify each candidate without mutating — tclsh
        // reports the first conflict before binding anything, so a failed
        // import leaves no partial aliases behind (namespace-old-9.15:
        // `can't import command "cmd1": already exists` imports no cmd2).
        let mut binds: Vec<(String, String)> = Vec::new();
        let mut removes: Vec<String> = Vec::new();
        let mut conflict: Option<String> = None;
        for (short_name, full_name) in &matching {
            let target = qualify(&cur, short_name);
            if let Some(existing) = interp.import_aliases.get(&target) {
                if existing == full_name {
                    continue; // re-import of the same origin: silent
                }
                if !force {
                    conflict = Some(short_name.clone());
                    break;
                }
                removes.push(target.clone());
                binds.push((target, full_name.clone()));
                continue;
            }
            // An existing command of that name in the target namespace
            // conflicts.  Procs defined at global scope are keyed by their
            // as-typed name ("cmd1", not "::cmd1").
            let at_global = cur == "::";
            let bare = target.trim_start_matches("::").to_string();
            let proc_hit = interp.procs.contains_key(&target)
                || (at_global && interp.procs.contains_key(&bare));
            let ens_hit = interp.ensembles.contains_key(&target)
                || (at_global && interp.ensembles.contains_key(&bare));
            // Importing over a builtin conflicts too (unknown is not a
            // command tclsh has pre-defined, so it never conflicts).
            let builtin_hit = at_global
                && bare != "unknown"
                && interp.commands.contains_key(&bare);
            if proc_hit || ens_hit || builtin_hit {
                if !force {
                    conflict = Some(short_name.clone());
                    break;
                }
                // -force replaces the existing binding.  Replacing a proc
                // (or ensemble) deletes it — and aliases re-imported from
                // it; builtin replacements keep the builtin registered
                // (dispatch finds builtins first; no corpus case imports
                // over one).
                removes.push(target.clone());
                if at_global && bare != target {
                    removes.push(bare.clone());
                }
                binds.push((target, full_name.clone()));
                continue;
            }
            binds.push((target, full_name.clone()));
        }
        if let Some(name) = conflict {
            return Err(Error::runtime(
                format!("can't import command \"{}\": already exists", name),
                ErrorCode::InvalidOp,
            ));
        }
        for key in removes {
            interp.procs.remove(&key);
            interp.ensembles.remove(&key);
            move_alias_origins(interp, &key, "");
        }
        for (target, full_name) in binds {
            interp.import_aliases.insert(target, full_name);
        }
    }

    Ok(Value::empty())
}

/// `namespace forget ?pattern ...?` — remove imports in the *current*
/// namespace whose origin matches each pattern.
fn ns_forget(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let cur = interp.current_namespace.clone();
    for pat in &args[2..] {
        let p = pat.as_str();
        let qualified = qualify(&cur, p);
        let src = {
            let q = ns_qualifiers_str(&qualified);
            if q.is_empty() { "::".to_string() } else { q.to_string() }
        };
        let tail = ns_tail_str(p).to_string();
        if !interp.namespaces.contains_key(&src) {
            return Err(Error::runtime(
                format!("unknown namespace in namespace forget pattern \"{}\"", p),
                ErrorCode::NotFound,
            ));
        }
        let src_prefix = ns_child_prefix(&src);
        interp.import_aliases.retain(|alias, origin| {
            // The alias lives in the current namespace iff its qualifiers
            // ARE the current namespace (global aliases have none).
            let alias_ns = ns_qualifiers_str(alias);
            let in_cur = if cur == "::" {
                alias_ns.is_empty()
            } else {
                alias_ns == cur
            };
            let from_src = origin
                .strip_prefix(&src_prefix)
                .filter(|rest| !rest.contains("::") && crate::interp::glob_match(&tail, rest))
                .is_some();
            !(in_cur && from_src)
        });
    }
    Ok(Value::empty())
}

/// `namespace inscope name arg ?arg...?` — the namespace must already
/// exist.  With a single `arg` tclsh evaluates it verbatim as a script;
/// with several, each *extra* argument becomes one word of the script
/// (a `{}` extra argument stays one empty word: `inscope ns cb a {} b`
/// calls `cb` with 3 args; `{a b}` stays one word).
fn ns_inscope(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace inscope", 4, args.len(),
            "name arg ?arg...?",
        ));
    }
    require_ns(interp, args[2].as_str())?;
    let mut script = args[3].as_str().to_string();
    for a in &args[4..] {
        script.push(' ');
        script.push_str(Value::from_list(&[a.clone()]).as_str());
    }
    let sub = [
        args[0].clone(),
        args[1].clone(),
        args[2].clone(),
        Value::from_str(&script),
    ];
    ns_eval(interp, &sub)
}

/// `namespace path ?pathList?` — stub.
fn ns_path(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace path", 2, args.len(),
            "?pathList?",
        ));
    }
    // TODO: command resolution path
    Ok(Value::empty())
}

/// `namespace unknown ?script?` — per-namespace unknown-command handler.
/// Reading returns the namespace's own handler ("" when unset); the global
/// namespace reports the default `::unknown`.
fn ns_unknown(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace unknown", 3, args.len(),
            "?script?",
        ));
    }
    let cur = interp.current_namespace.clone();
    if args.len() == 3 {
        let script = args[2].as_str().to_string();
        interp.ns_unknown.insert(cur, script);
        return Ok(args[2].clone());
    }
    if let Some(h) = interp.ns_unknown.get(&cur) {
        return Ok(Value::from_str(h));
    }
    if cur == "::" {
        return Ok(Value::from_str("::unknown"));
    }
    Ok(Value::empty())
}

/// `namespace upvar ns ?otherVar myVar ...?`
fn ns_upvar(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 || (args.len() - 3) % 2 != 0 {
        return Err(Error::wrong_args_with_usage(
            "namespace upvar", 3, args.len(),
            "ns ?otherVar myVar ...?",
        ));
    }
    let nsq = require_ns(interp, args[2].as_str())?;
    let mut i = 3;
    while i + 1 < args.len() {
        let other = args[i].as_str();
        let my = args[i + 1].as_str();
        let target = var_key(&qualify(&nsq, other));
        if !interp.frames.is_empty() {
            let frame_idx = interp.frames.len() - 1;
            interp.frames[frame_idx]
                .upvars
                .insert(my.to_string(), crate::interp::UpvarLink::Global(target));
        }
        i += 2;
    }
    Ok(Value::empty())
}

const ENS_SUBCOMMANDS: [&str; 3] = ["configure", "create", "exists"];

fn resolve_ensemble_subcmd(sub: &str) -> Result<&'static str> {
    if let Some(exact) = ENS_SUBCOMMANDS.iter().find(|s| **s == sub) {
        return Ok(exact);
    }
    let matches: Vec<&str> = ENS_SUBCOMMANDS
        .iter()
        .filter(|s| s.starts_with(sub))
        .copied()
        .collect();
    if matches.len() == 1 {
        return Ok(matches[0]);
    }
    Err(Error::runtime(
        format!(
            "bad subcommand \"{}\": must be configure, create, or exists",
            sub
        ),
        ErrorCode::InvalidOp,
    ))
}

/// `namespace ensemble subcommand ?arg ...?`
fn ns_ensemble(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace ensemble", 3, args.len(),
            "subcommand ?arg ...?",
        ));
    }
    match resolve_ensemble_subcmd(args[2].as_str())? {
        "configure" => ens_configure(interp, &args[3..]),
        "create" => ens_create(interp, &args[3..]),
        "exists" => {
            let mut out = Vec::new();
            for a in &args[3..] {
                out.push(Value::from_bool(
                    find_ensemble_key(interp, a.as_str()).is_some(),
                ));
            }
            Ok(Value::from_list(&out))
        }
        _ => unreachable!("resolve_ensemble_subcmd validated"),
    }
}

/// Options accepted by `namespace ensemble create` (tclsh order in the
/// bad-option message).
const ENS_CREATE_OPTIONS: [&str; 6] = [
    "-command", "-map", "-parameters", "-prefixes", "-subcommands", "-unknown",
];

/// Resolve an ensemble option by exact match or unique prefix (tclsh:
/// `-subcomm` selects `-subcommands`).  `None` = no match or ambiguous.
fn resolve_ens_opt(opt: &str) -> Option<&'static str> {
    if let Some(exact) = ENS_CREATE_OPTIONS.iter().find(|o| **o == opt) {
        return Some(exact);
    }
    let matches: Vec<&'static str> = ENS_CREATE_OPTIONS
        .iter()
        .copied()
        .filter(|o| o.starts_with(opt))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

/// Parse a boolean option value (`-prefixes 0`).
fn ens_bool(v: &Value, opt: &str) -> Result<bool> {
    let s = v.as_str();
    if let Some(n) = v.as_int() {
        return Ok(n != 0);
    }
    match s {
        "true" | "yes" | "on" => Ok(true),
        "false" | "no" | "off" => Ok(false),
        _ => Err(Error::runtime(
            format!(
                "expected boolean value but got \"{}\" for \"{}\"",
                s, opt
            ),
            ErrorCode::Generic,
        )),
    }
}

/// Parse a `-map` value: pairs of subcommand → implementation prefix.
/// The implementation prefix's FIRST word is qualified against the
/// backing namespace at set time (tclsh: `-map {b b}` reads back as
/// `{b ::b}`); remaining words stay as typed.
fn parse_ens_map(
    interp: &Interp,
    ns: &str,
    v: &Value,
) -> Result<Vec<(String, Vec<String>)>> {
    let _ = ns;
    let items = v
        .as_list()
        .ok_or_else(|| Error::runtime("missing value to go with key".to_string(), ErrorCode::InvalidOp))?;
    if items.len() % 2 != 0 {
        return Err(Error::runtime(
            "missing value to go with key".to_string(),
            ErrorCode::InvalidOp,
        ));
    }
    let mut map = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let sub = items[i].as_str().to_string();
        let impl_val = &items[i + 1];
        let words: Vec<String> = impl_val
            .as_list()
            .unwrap_or_else(|| vec![impl_val.clone()])
            .iter()
            .map(|w| w.as_str().to_string())
            .collect();
        if words.is_empty() {
            return Err(Error::runtime(
                "ensemble subcommand implementations must be non-empty lists"
                    .to_string(),
                ErrorCode::InvalidOp,
            ));
        }
        // The implementation prefix's first word qualifies against the
        // namespace current AT SET TIME (tclsh: `-map {b b}` configured
        // at global reads back `{b ::b}`; inside the namespace it reads
        // `{b ::ns::b}`).
        let first = if words[0].starts_with("::") {
            words[0].clone()
        } else {
            qualify(&interp.current_namespace, &words[0])
        };
        let mut resolved = vec![first];
        resolved.extend_from_slice(&words[1..]);
        map.push((sub, resolved));
        i += 2;
    }
    Ok(map)
}

/// `namespace ensemble create ?option value ...?` — registers the current
/// namespace as an ensemble command (`-command name` overrides the name).
fn ens_create(interp: &mut Interp, opts: &[Value]) -> Result<Value> {
    if opts.len() % 2 != 0 {
        return Err(Error::wrong_args_with_usage(
            "namespace ensemble create", 2, opts.len() + 2,
            "?option value ...?",
        ));
    }
    let ns = interp.current_namespace.clone();
    // The ensemble command is named after the namespace and lives in its
    // PARENT (tclsh: `namespace eval ns {namespace ensemble create}` yields
    // the command `ns`, callable from the parent level).
    let mut name = if ns == "::" {
        ns.clone()
    } else {
        let tail = ns.rsplit("::").next().unwrap_or("").to_string();
        match parent_of(&ns) {
            p if p == "::" => tail,
            p => format!("{}::{}", p, tail),
        }
    };
    let mut def = crate::interp::EnsembleDef {
        namespace: ns.clone(),
        map: Vec::new(),
        prefixes: true,
        subcommands: None,
        unknown: None,
        parameters: Vec::new(),
    };
    let mut i = 0;
    while i < opts.len() {
        let val = &opts[i + 1];
        let opt = match resolve_ens_opt(opts[i].as_str()) {
            Some(o) => o,
            None => {
                let list: Vec<&str> = ENS_CREATE_OPTIONS.iter().copied().collect();
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": must be {}",
                        opts[i].as_str(),
                        join_or(&list)
                    ),
                    ErrorCode::InvalidOp,
                ));
            }
        };
        match opt {
            "-command" => name = qualify(&ns, val.as_str()),
            "-map" => def.map = parse_ens_map(interp, &ns, val)?,
            "-parameters" => {
                def.parameters = val
                    .as_list()
                    .unwrap_or_default()
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect();
            }
            "-prefixes" => def.prefixes = ens_bool(val, opt)?,
            "-subcommands" => {
                def.subcommands = Some(
                    val.as_list()
                        .unwrap_or_default()
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .collect(),
                );
            }
            "-unknown" => {
                def.unknown = Some(
                    val.as_list()
                        .unwrap_or_default()
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .collect(),
                );
            }
            _ => unreachable!(),
        }
        i += 2;
    }
    interp.ensembles.insert(name, def);
    Ok(Value::empty())
}

/// [`find_ensemble_key`] plus a clone of the ensemble's definition, for
/// dispatch.
pub(crate) fn find_ensemble(
    interp: &Interp,
    name: &str,
) -> Option<(String, crate::interp::EnsembleDef)> {
    let key = find_ensemble_key(interp, name)?;
    let def = interp.ensembles.get(&key)?.clone();
    Some((key, def))
}

/// Resolve an ensemble command name the way dispatch does (exact,
/// namespace-qualified, `::`-prefixed, colon-run normalise, then through
/// import-alias origin chains).
pub(crate) fn find_ensemble_key(interp: &Interp, name: &str) -> Option<String> {
    let cur = &interp.current_namespace;
    let mut try_keys: Vec<String> = vec![name.to_string()];
    if cur != "::" && !name.starts_with("::") {
        try_keys.push(qualify(cur, name));
    }
    if !name.starts_with("::") {
        // A relative name from the global level names `::name`.
        try_keys.push(format!("::{}", name));
    }
    if name.contains("::") {
        let norm = normalise(name);
        if norm != name {
            try_keys.push(norm.clone());
        }
        // An absolute simple name (`::ns`, or a colon run like `::::n1`
        // that normalises to one) reaches an ensemble registered under the
        // bare tail in the parent namespace (ensemble create's default).
        if let Some(bare) = norm.strip_prefix("::") {
            if !bare.contains("::") {
                try_keys.push(bare.to_string());
            }
        }
    }
    for k in &try_keys {
        if interp.ensembles.contains_key(k) {
            return Some(k.clone());
        }
    }
    // Import alias pointing at an ensemble
    if let Some(found) = lookup_command_key(interp, name) {
        if let Some(origin) = origin_of(interp, &found) {
            if interp.ensembles.contains_key(&origin) {
                return Some(origin);
            }
        }
    }
    None
}

/// The ensemble's candidate subcommands: `-subcommands` when configured,
/// otherwise the backing namespace's exported commands (procs and import
/// aliases one level under the namespace matching its export patterns).
fn ens_candidates(interp: &Interp, def: &crate::interp::EnsembleDef) -> Vec<String> {
    if let Some(subs) = &def.subcommands {
        return subs.clone();
    }
    if !def.map.is_empty() {
        // A non-empty -map defines the subcommand set by itself (tclsh
        // 43.1: with `-map {a x1 b x2}` a miss lists "a, or b", not the
        // exported procs).
        let mut keys: Vec<String> = def.map.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        keys.dedup();
        return keys;
    }
    let exports = interp
        .namespaces
        .get(&def.namespace)
        .map(|info| info.export_patterns.clone())
        .unwrap_or_default();
    let prefix = ns_child_prefix(&def.namespace);
    let mut names: Vec<String> = interp
        .procs
        .keys()
        .map(|s| s.as_str())
        .chain(interp.import_aliases.keys().map(|s| s.as_str()))
        .filter_map(|k| {
            k.strip_prefix(&prefix)
                .filter(|rest| !rest.contains("::"))
                .filter(|rest| {
                    exports
                        .iter()
                        .any(|e| crate::interp::glob_match(e, rest))
                })
                .map(|rest| rest.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Resolve `sub` against an ensemble: exact map key, exact candidate, then
/// (when prefixes are on) a unique prefix over map keys + candidates.
/// `Ok(Some(words))` = implementation prefix; `Ok(None)` = no match.
/// `Err` = ambiguous prefix (tclsh lists only the matching candidates).
fn ens_resolve(
    interp: &Interp,
    def: &crate::interp::EnsembleDef,
    sub: &str,
) -> Result<Option<Vec<String>>> {
    if let Some((_, words)) = def.map.iter().find(|(k, _)| k == sub) {
        return Ok(Some(words.clone()));
    }
    let candidates = ens_candidates(interp, def);
    let exported_hit = candidates.iter().any(|c| c == sub);
    if exported_hit {
        return Ok(Some(vec![qualify(&def.namespace, sub)]));
    }
    if def.prefixes {
        // Prefix pool: map keys first, then the candidates.
        let mut pool: Vec<String> =
            def.map.iter().map(|(k, _)| k.clone()).collect();
        for c in &candidates {
            if !pool.contains(c) {
                pool.push(c.clone());
            }
        }
        let matches: Vec<&String> =
            pool.iter().filter(|s| s.starts_with(sub)).collect();
        if matches.len() == 1 {
            let m = matches[0];
            if let Some((_, words)) = def.map.iter().find(|(k, _)| k == m) {
                return Ok(Some(words.clone()));
            }
            return Ok(Some(vec![qualify(&def.namespace, m)]));
        }
        if matches.len() > 1 {
            let listed: Vec<&str> =
                matches.iter().map(|s| s.as_str()).collect();
            return Err(Error::runtime(
                format!(
                    "unknown or ambiguous subcommand \"{}\": must be {}",
                    sub,
                    join_or(&listed)
                ),
                ErrorCode::InvalidOp,
            ));
        }
    }
    Ok(None)
}

/// The no-match error for `sub` (candidates listed in tclsh's format).
fn ens_miss_error(
    interp: &Interp,
    def: &crate::interp::EnsembleDef,
    sub: &str,
) -> Error {
    let mut candidates = ens_candidates(interp, def);
    // Map keys are candidates too (tclsh: `-map {a a}` + miss lists `a`).
    for (k, _) in &def.map {
        if !candidates.contains(k) {
            candidates.push(k.clone());
        }
    }
    if let Some(subs) = &def.subcommands {
        if subs.is_empty() || candidates.is_empty() {
            // fall through to the same wording as below
        }
    }
    if candidates.is_empty() {
        return Error::runtime(
            format!(
                "unknown subcommand \"{}\": namespace {} does not export any commands",
                sub, def.namespace
            ),
            ErrorCode::InvalidOp,
        );
    }
    let listed: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let word = if def.prefixes {
        "unknown or ambiguous subcommand"
    } else {
        "unknown subcommand"
    };
    Error::runtime(
        format!("{} \"{}\": must be {}", word, sub, join_or(&listed)),
        ErrorCode::InvalidOp,
    )
}

/// Run `call` (a resolved command word list).  tclsh's EnsembleInvoke
/// evaluates the implementation with TCL_EVAL_INVOKE — the *caller's*
/// namespace context, no switch to the backing namespace (46.7: the impl
/// `::namespace delete ns` must resolve `ns` where the caller resolves it).
fn ens_run(
    interp: &mut Interp,
    ns: &str,
    call: &[Value],
) -> Result<Value> {
    let _ = ns;
    interp.dispatch_values(call)
}

/// Seed the errorInfo for the bad-completion errors raised at the
/// `-unknown` handler site (47.4): a fresh info — message plus the
/// `result of ensemble unknown subcommand handler: <call>` tag — that the
/// enclosing harness then appends `invoked from within` to.
fn ens_tag_handler_error(interp: &mut Interp, msg: &str, call_text: &str) {
    interp.err_info = Some(format!(
        "{}\n    result of ensemble unknown subcommand handler: {}",
        msg, call_text
    ));
}

/// Ensemble command dispatch: `ens sub ?args...?`.
pub(crate) fn dispatch_ensemble(
    interp: &mut Interp,
    ens_key: &str,
    def: crate::interp::EnsembleDef,
    args: &[Value],
) -> Result<Value> {
    let words = &args[1..];
    // `-parameters`: that many leading words are ALWAYS parameter values
    // (kept so at least one word remains for the subcommand).
    let nparams = def
        .parameters
        .len()
        .min(words.len().saturating_sub(1));
    let (params, rest) = words.split_at(nparams);
    if rest.is_empty() {
        return Err(Error::wrong_args_with_usage(
            ens_key, 2, args.len(), "subcommand ?arg ...?",
        ));
    }
    let sub = rest[0].as_str();

    match ens_resolve(interp, &def, sub) {
        Err(e) => return Err(e),
        Ok(Some(impl_words)) => {
            let mut call: Vec<Value> = impl_words
                .iter()
                .map(|w| Value::from_str(w.as_str()))
                .collect();
            call.extend(params.iter().cloned());
            call.extend(rest[1..].iter().cloned());
            return ens_run(interp, &def.namespace, &call);
        }
        Ok(None) => {}
    }

    // No match: run the `-unknown` handler when configured.  The handler
    // gets (ensemble, subcommand, args...); its return value is a command
    // prefix used when the retry still doesn't find the subcommand.
    if let Some(handler) = def.unknown.clone() {
        let mut call: Vec<Value> =
            handler.iter().map(|w| Value::from_str(w.as_str())).collect();
        // The handler sees the ensemble's fully-qualified name (47.2:
        // `ns spong` at global passes `::ns` to `::ns::Magic`).
        let ens_display = if ens_key.starts_with("::") {
            ens_key.to_string()
        } else {
            qualify(&interp.current_namespace, ens_key)
        };
        call.push(Value::from_str(&ens_display));
        call.push(Value::from_str(sub));
        call.extend(rest[1..].iter().cloned());
        // tclsh logs the handler invocation as a real frame: the call text
        // (argv joined) plus the `(ensemble unknown subcommand handler)` tag.
        let call_text = call
            .iter()
            .map(|v| v.as_str().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let handler_result = match interp.dispatch_values(&call) {
            Err(e) if e.is_break() => {
                ens_tag_handler_error(
                    interp,
                    "unknown subcommand handler returned bad code: break",
                    &call_text,
                );
                return Err(Error::runtime(
                    "unknown subcommand handler returned bad code: break"
                        .to_string(),
                    ErrorCode::InvalidOp,
                ));
            }
            Err(e) if e.is_continue() => {
                ens_tag_handler_error(
                    interp,
                    "unknown subcommand handler returned bad code: continue",
                    &call_text,
                );
                return Err(Error::runtime(
                    "unknown subcommand handler returned bad code: continue"
                        .to_string(),
                    ErrorCode::InvalidOp,
                ));
            }
            Err(e) => {
                if interp.err_is_error(&e) {
                    if let Some(info) = &mut interp.err_info {
                        info.push_str(&format!(
                            "\n    invoked from within\n\"{}\"\n    (ensemble unknown subcommand handler)",
                            call_text
                        ));
                    }
                }
                return Err(e);
            }
            Ok(v) => v,
        };
        // Retry the subcommand lookup once (the handler may have defined
        // implementations); refresh the definition first.
        let def2 = interp.ensembles.get(ens_key).cloned();
        if let Some(def2) = def2 {
            match ens_resolve(interp, &def2, sub) {
                Err(e) => return Err(e),
                Ok(Some(impl_words)) => {
                    let mut call: Vec<Value> = impl_words
                        .iter()
                        .map(|w| Value::from_str(w.as_str()))
                        .collect();
                    call.extend(params.iter().cloned());
                    call.extend(rest[1..].iter().cloned());
                    return ens_run(interp, &def2.namespace, &call);
                }
                Ok(None) => {}
            }
        }
        // Retry failed: dispatch the handler's returned prefix, or report
        // the (refreshed) miss.
        match handler_result.as_list_strict() {
            Ok(prefix) if !prefix.is_empty() => {
                let mut call: Vec<Value> = prefix.clone();
                call.extend(params.iter().cloned());
                call.extend(rest[1..].iter().cloned());
                return ens_run(interp, &def.namespace, &call);
            }
            Ok(_) => {
                return Err(ens_miss_error(interp, &def, sub));
            }
            Err(e) => {
                // The handler's result must parse as a list; unparseable
                // results error here (47.6: `return "\{"` → unmatched
                // open brace).
                interp.err_info = Some(format!(
                    "{}\n    while parsing result of ensemble unknown subcommand handler",
                    e.message
                ));
                return Err(Error::Msg(e.message.to_string()));
            }
        }
    }

    Err(ens_miss_error(interp, &def, sub))
}

/// `namespace ensemble configure ensemble ?-option value ...?`
fn ens_configure(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(Error::wrong_args_with_usage(
            "namespace ensemble configure", 3, args.len() + 3,
            "cmdname ?-option value ...? ?arg ...?",
        ));
    }
    let typed = args[0].as_str().to_string();
    // The ensemble command name — or the name of its backing namespace
    // (45.1: `namespace ensemble configure ::ns` inside `ns` names the
    // namespace whose attached ensemble is meant).
    let key = find_ensemble_key(interp, &typed)
        .or_else(|| {
            let ns = if typed.starts_with("::") {
                normalise(&typed)
            } else if interp.current_namespace == "::" {
                format!("::{}", typed)
            } else {
                qualify(&interp.current_namespace, &typed)
            };
            interp
                .ensembles
                .iter()
                .find(|(_, def)| def.namespace == ns)
                .map(|(k, _)| k.clone())
        })
        .ok_or_else(|| {
        if command_exists(interp, &typed) {
            Error::runtime(
                format!("\"{}\" is not an ensemble command", typed),
                ErrorCode::InvalidOp,
            )
        } else {
            Error::runtime(
                format!("unknown command \"{}\"", typed),
                ErrorCode::NotFound,
            )
        }
    })?;
    // Commands that exist as procs/builtins/aliases but not ensembles.
    if args.len() == 1 {
        // Full option list, tclsh's fixed order.
        let def = interp.ensembles.get(&key).unwrap();
        let map_items: Vec<Value> = def
            .map
            .iter()
            .flat_map(|(k, words)| {
                let mut v = vec![Value::from_str(k)];
                v.push(Value::from_list(
                    &words.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>(),
                ));
                v
            })
            .collect();
        let unknown = Value::from_list(
            &def.unknown
                .as_ref()
                .map(|u| u.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        let subcmds = Value::from_list(
            &def.subcommands
                .as_ref()
                .map(|u| u.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        return Ok(Value::from_list(&[
            Value::from_str("-map"),
            Value::from_list(&map_items),
            Value::from_str("-namespace"),
            Value::from_str(&def.namespace),
            Value::from_str("-parameters"),
            Value::from_list(&def.parameters.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>()),
            Value::from_str("-prefixes"),
            Value::from_bool(def.prefixes),
            Value::from_str("-subcommands"),
            subcmds,
            Value::from_str("-unknown"),
            unknown,
        ]));
    }
    let mut i = 1;
    while i < args.len() {
        // Configure accepts the create options (plus get-only -namespace)
        // by exact name or unique prefix.
        let raw = args[i].as_str();
        let opt = {
            let mut pool: Vec<&'static str> = ENS_CREATE_OPTIONS.to_vec();
            pool.push("-namespace");
            match pool.iter().find(|o| **o == raw).copied().or_else(|| {
                let m: Vec<&'static str> =
                    pool.iter().copied().filter(|o| o.starts_with(raw)).collect();
                if m.len() == 1 { Some(m[0]) } else { None }
            }) {
                Some(o) => o,
                None => {
                    return Err(Error::runtime(
                        format!("bad option \"{}\": must be {}", raw, join_or(&pool)),
                        ErrorCode::InvalidOp,
                    ));
                }
            }
        };
        let get_only = opt == "-namespace";
        if i + 1 >= args.len() || get_only {
            // GET
            let def = interp.ensembles.get(&key).unwrap();
            return match opt {
                "-map" => {
                    let items: Vec<Value> = def
                        .map
                        .iter()
                        .flat_map(|(k, words)| {
                            let mut v = vec![Value::from_str(k)];
                            v.push(Value::from_list(
                                &words
                                    .iter()
                                    .map(|w| Value::from_str(w.as_str()))
                                    .collect::<Vec<_>>(),
                            ));
                            v
                        })
                        .collect();
                    Ok(Value::from_list(&items))
                }
                "-namespace" => Ok(Value::from_str(&def.namespace)),
                "-parameters" => Ok(Value::from_list(
                    &def.parameters.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>(),
                )),
                "-prefixes" => Ok(Value::from_bool(def.prefixes)),
                "-subcommands" => Ok(Value::from_list(
                    &def.subcommands
                        .as_ref()
                        .map(|u| {
                            u.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                )),
                "-unknown" => Ok(Value::from_list(
                    &def.unknown
                        .as_ref()
                        .map(|u| {
                            u.iter().map(|w| Value::from_str(w.as_str())).collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                )),
                other => {
                    let mut list: Vec<&str> =
                        ENS_CREATE_OPTIONS.iter().copied().collect();
                    list.push("-namespace");
                    Err(Error::runtime(
                        format!(
                            "bad option \"{}\": must be {}",
                            other,
                            join_or(&list)
                        ),
                        ErrorCode::InvalidOp,
                    ))
                }
            };
        }
        let val = &args[i + 1];
        match opt {
            "-map" => {
                let ns = interp.ensembles.get(&key).unwrap().namespace.clone();
                let parsed = parse_ens_map(interp, &ns, val)?;
                interp.ensembles.get_mut(&key).unwrap().map = parsed;
            }
            _ => {
                let def = interp.ensembles.get_mut(&key).unwrap();
                match opt {
                "-parameters" => {
                    def.parameters = val
                        .as_list()
                        .unwrap_or_default()
                        .iter()
                        .map(|s| s.as_str().to_string())
                        .collect();
                }
                "-prefixes" => def.prefixes = ens_bool(val, opt)?,
                "-subcommands" => {
                    def.subcommands = Some(
                        val.as_list()
                            .unwrap_or_default()
                            .iter()
                            .map(|s| s.as_str().to_string())
                            .collect(),
                    );
                }
                "-unknown" => {
                    def.unknown = Some(
                        val.as_list()
                            .unwrap_or_default()
                            .iter()
                            .map(|s| s.as_str().to_string())
                            .collect(),
                    );
                }
                    other => {
                        let mut list: Vec<&str> =
                            ENS_CREATE_OPTIONS.iter().copied().collect();
                        list.push("-namespace");
                        return Err(Error::runtime(
                            format!(
                                "bad option \"{}\": must be {}",
                                other,
                                join_or(&list)
                            ),
                            ErrorCode::InvalidOp,
                        ));
                    }
                }
            }
        }
        i += 2;
    }
    Ok(Value::empty())
}

/// Does `name` resolve to any command (proc, builtin, alias) right now?
fn command_exists(interp: &Interp, name: &str) -> bool {
    lookup_command_key(interp, name).is_some()
}

// ── helper functions ───────────────────────────────────────────────────

/// Qualify a name relative to a namespace and normalise the result
/// (collapsing runs of `:` and ensuring the leading `::`).
pub(crate) fn qualify(ns: &str, name: &str) -> String {
    let joined = if name.starts_with("::") {
        name.to_string()
    } else if ns == "::" {
        format!("::{}", name)
    } else {
        format!("{}::{}", ns, name)
    };
    normalise(&joined)
}

/// Normalise a fully-qualified name — collapse runs of two or more colons
/// into a single `::` separator and ensure the result starts with `::`.
/// Single colons are ordinary name characters (`a:b` stays one component).
pub(crate) fn normalise(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b':' {
                j += 1;
            }
            if j - i >= 2 {
                // a run of >=2 colons is the separator
                if i > start {
                    parts.push(&name[start..i]);
                }
                start = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    if start < bytes.len() {
        parts.push(&name[start..]);
    }
    if parts.is_empty() {
        return "::".to_string();
    }
    format!("::{}", parts.join("::"))
}

/// Parent namespace of a `::`-qualified name (`::a::b` → `::a`,
/// `::a` → `::`).
pub(crate) fn parent_of(ns: &str) -> String {
    match ns.rfind("::") {
        Some(0) => "::".to_string(),
        Some(i) => ns[..i].to_string(),
        None => "::".to_string(),
    }
}

/// Prefix under which the direct children of `ns` are registered.
fn ns_child_prefix(ns: &str) -> String {
    if ns == "::" {
        "::".to_string()
    } else {
        format!("{}::", ns)
    }
}

/// Split at the final run of colons: `(qualifiers, tail)`.  tclsh walks
/// back over the whole run — `a::::b` → `("a", "b")`,
/// `:::x::y:::` → `(":::x::y", "")`.
fn split_colon_run(name: &str) -> (&str, &str) {
    match name.rfind(':') {
        None => ("", name),
        Some(e) => {
            let bytes = name.as_bytes();
            let mut s = e;
            while s > 0 && bytes[s - 1] == b':' {
                s -= 1;
            }
            (&name[..s], &name[e + 1..])
        }
    }
}

/// Qualifiers part of a name (see [`split_colon_run`]).
fn ns_qualifiers_str(name: &str) -> &str {
    split_colon_run(name).0
}

/// Tail part of a name (see [`split_colon_run`]).
fn ns_tail_str(name: &str) -> &str {
    split_colon_run(name).1
}

/// Return the parent namespace of a fully-qualified namespace.
fn parent_ns(ns: &str) -> String {
    let q = ns_qualifiers_str(ns);
    if q.is_empty() {
        "::".to_string()
    } else {
        q.to_string()
    }
}

/// Resolve a command name (relative or `::`-qualified) to its registered
/// key: procs are stored fully qualified, builtins unqualified.  Returns
/// the *alias* key when the name is an import alias.
pub(crate) fn lookup_command_key(interp: &Interp, name: &str) -> Option<String> {
    let qualified = qualify(&interp.current_namespace, name);
    if interp.procs.contains_key(&qualified) || interp.import_aliases.contains_key(&qualified) {
        return Some(qualified);
    }
    if interp.commands.contains_key(&qualified) {
        return Some(qualified);
    }
    if interp.procs.contains_key(name)
        || interp.import_aliases.contains_key(name)
        || interp.commands.contains_key(name)
    {
        return Some(qualify("::", name));
    }
    // Builtins are keyed unqualified: `::set` resolves to `set`
    let stripped = qualified.strip_prefix("::")?;
    if !stripped.contains("::") && interp.commands.contains_key(stripped) {
        return Some(qualified);
    }
    None
}

/// Follow the import-alias chain from a command name to the fully
/// qualified name of the original command.
pub(crate) fn origin_of(interp: &Interp, name: &str) -> Option<String> {
    let mut key = lookup_command_key(interp, name)?;
    while let Some(next) = interp.import_aliases.get(&key) {
        key = next.clone();
    }
    Some(key)
}

/// Ensure that a namespace and all its ancestors exist in the namespace table.
fn ensure_namespace(
    namespaces: &mut std::collections::HashMap<String, NamespaceInfo>,
    qualified: &str,
) {
    // Always ensure "::" exists
    namespaces.entry("::".to_string()).or_default();
    if qualified == "::" {
        return;
    }
    // Walk from root to leaf, creating any missing intermediate namespaces.
    let parts: Vec<&str> = qualified.split("::").filter(|s| !s.is_empty()).collect();
    let mut path = String::from("::");
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            path = format!("::{}", part);
        } else {
            path = format!("{}::{}", path, part);
        }
        namespaces.entry(path.clone()).or_default();
    }
}

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

    // -- namespace variables share one flat key space with globals
    //    (tclsh 8.6.17: `variable v` in `namespace eval n` stores n::v,
    //    and `set ::n::v` from anywhere resolves to the same slot) --

    #[test]
    fn test_ns_variable_readable_as_qualified_global() {
        assert_eq!(eval("namespace eval n { variable v 5 }; set ::n::v"), "5");
    }

    #[test]
    fn test_ns_variable_readable_from_ns_scope() {
        assert_eq!(
            eval("namespace eval n { variable v 5 }; namespace eval n { set ::n::v }"),
            "5"
        );
    }

    #[test]
    fn test_ns_delete_removes_ns_variables() {
        assert_eq!(
            eval("namespace eval n { variable w 6 }; namespace delete n; info exists ::n::w"),
            "0"
        );
    }

    #[test]
    fn test_ns_which_finds_ns_variable() {
        assert_eq!(
            eval("namespace eval m { variable v 7 }; namespace which -variable ::m::v"),
            "::m::v"
        );
    }

    #[test]
    fn test_ns_which_finds_declared_but_unset_variable() {
        // tclsh 8.6.17: a bare `variable x` registers the name in the
        // namespace's variable table — `namespace which -variable` resolves
        // it even though `info exists` is still 0.
        assert_eq!(
            eval("namespace eval n { variable martha; namespace which -variable martha }"),
            "::n::martha"
        );
    }
    // -- procs execute in their definition namespace (tclsh 8.6.17) --

    #[test]
    fn test_proc_runs_in_definition_namespace() {
        assert_eq!(
            eval("namespace eval foo {proc p {} {namespace current}}; foo::p"),
            "::foo"
        );
    }

    // -- Tcl scoping: a proc frame sees ONLY locals, upvar/variable links,
    //    and `::`-qualified names.  No fallback to the definition
    //    namespace's variables or to globals (tclsh 8.6.17: both
    //    `proc foo::p {} {return $v}` and `proc p {} {return $x}` raise
    //    "can't read" even with `variable v` / `set x` visible outside) --

    #[test]
    fn test_proc_cannot_read_global_without_declaration() {
        let mut interp = Interp::new();
        interp.eval("set x 40; proc p {} {return $x}").unwrap();
        let err = interp.eval("p").unwrap_err().to_string();
        assert!(err.contains("can't read \"x\""), "{err}");
    }

    #[test]
    fn test_proc_cannot_read_ns_variable_without_declaration() {
        let mut interp = Interp::new();
        interp
            .eval("namespace eval foo {variable v 5}; proc foo::p {} {return $v}")
            .unwrap();
        let err = interp.eval("foo::p").unwrap_err().to_string();
        assert!(err.contains("can't read \"v\""), "{err}");
    }

    #[test]
    fn test_proc_variable_declaration_links_ns_var() {
        assert_eq!(
            eval(
                "namespace eval foo {variable v 5}; proc foo::p {} {variable v; return $v}; foo::p"
            ),
            "5"
        );
    }

    #[test]
    fn test_qualified_proc_callable_unqualified() {
        assert_eq!(
            eval("namespace eval foo {proc p {} {return 42}}; foo::p"),
            "42"
        );
    }

    #[test]
    fn test_proc_at_global_sees_definition_ns_qualified() {
        // tclsh: `proc test_ns_basic::cmd` defined at global scope runs in
        // ::test_ns_basic — the definition namespace always renders with a
        // leading `::`.
        assert_eq!(
            eval("namespace eval test_ns_basic {}; proc test_ns_basic::cmd {} {namespace current}; test_ns_basic::cmd"),
            "::test_ns_basic"
        );
    }

    // -- namespace existence validation (tclsh 8.6.17: relative names
    //    report the context ns, absolute names the bare form) --

    #[test]
    fn test_ns_children_parent_inscope_require_existing() {
        assert_eq!(
            eval_err("namespace children xyzzy"),
            "namespace \"xyzzy\" not found in \"::\""
        );
        assert_eq!(
            eval_err("namespace parent xyzzy"),
            "namespace \"xyzzy\" not found in \"::\""
        );
        assert_eq!(
            eval_err("namespace inscope xyzzy {set a 1}"),
            "namespace \"xyzzy\" not found in \"::\""
        );
        assert_eq!(
            eval_err("namespace children ::xyzzy"),
            "namespace \"::xyzzy\" not found"
        );
    }

    #[test]
    fn test_ns_code_builds_inscope_list() {
        // tclsh: namespace code works even for nonexistent namespaces and
        // renders as a proper Tcl list (multi-word args get braced).
        assert_eq!(
            eval("namespace code xyzzy::sub"),
            "::namespace inscope :: xyzzy::sub"
        );
        assert_eq!(
            eval("namespace code {a b c}"),
            "::namespace inscope :: {a b c}"
        );
    }

    // -- colon-run collapse (tclsh 8.6.17: `:::a::::b` == `::a::b`) --

    #[test]
    fn test_colon_run_collapse() {
        assert_eq!(
            eval("namespace eval :::m1::: {namespace current}"),
            "::m1"
        );
        assert_eq!(
            eval("namespace eval :::tnsB::::foo {namespace current}"),
            "::tnsB::foo"
        );
        assert_eq!(eval("namespace eval m1 {}; namespace exists :::m1:::"), "1");
        // Colon runs collapse for NAMESPACES, but `variable` still demands
        // an existing parent (tclsh 8.6.17: `variable w:::x 9` in q errors
        // `can't define "w:::x": parent namespace doesn't exist`).
        assert_eq!(
            eval_err("namespace eval q {variable w:::x 9}"),
            "can't define \"w:::x\": parent namespace doesn't exist"
        );
        assert_eq!(eval("namespace tail a::::b"), "b");
        assert_eq!(eval("namespace qualifiers a::::b"), "a");
        assert_eq!(eval("namespace qualifiers :::x::y:::"), ":::x::y");
        assert_eq!(eval("namespace tail :::x::y:::"), "");
        assert_eq!(eval("namespace qualifiers ::"), "");
    }

    // -- import/forget/origin (tclsh 8.6.17) --

    #[test]
    fn test_import_alias_dispatches_to_origin_body() {
        assert_eq!(
            eval(
                r#"
                namespace eval e1 { namespace export c1; proc c1 {} {namespace current} }
                namespace eval e2 { namespace import ::e1::c1 }
                list [e2::c1] [namespace origin e2::c1] [info commands ::e2::*]
            "#,
            ),
            "::e1 ::e1::c1 ::e2::c1"
        );
    }

    #[test]
    fn test_import_sees_redefined_origin() {
        assert_eq!(
            eval(
                r#"
                namespace eval e1 { namespace export cmd; proc cmd {} {return old} }
                namespace eval e2 { namespace import ::e1::cmd }
                proc e1::cmd {} {return new}
                e2::cmd
            "#,
            ),
            "new"
        );
    }

    #[test]
    fn test_import_errors() {
        assert_eq!(eval_err("namespace import {}"), "empty import pattern");
        assert_eq!(
            eval_err("namespace import fred::x"),
            "unknown namespace in import pattern \"fred::x\""
        );
        assert_eq!(
            eval_err("namespace eval t {namespace import ::t::puts}"),
            "import pattern \"::t::puts\" tries to import from namespace \"t\" into itself"
        );
        // only exported commands are imported; non-matching is silent
        assert_eq!(
            eval(
                "namespace eval s1 {proc c2 {} {}}; namespace eval s2 {namespace import ::s1::c2}; info commands ::s2::*"
            ),
            ""
        );
    }

    #[test]
    fn test_forget_removes_matching_imports() {
        assert_eq!(
            eval(
                r#"
                namespace eval e1 {namespace export c*; proc c1 {} {}; proc c2 {} {}}
                namespace eval e2 {namespace import ::e1::c*; namespace forget ::e1::c1}
                info commands ::e2::*
            "#,
            ),
            "::e2::c2"
        );
    }

    #[test]
    fn test_import_conflict() {
        assert_eq!(
            eval_err(
                "namespace eval e1 {namespace export c1; proc c1 {} {}}; \
                 namespace eval e2 {proc c1 {} {}}; \
                 namespace eval e2 {namespace import ::e1::c1}",
            ),
            "can't import command \"c1\": already exists"
        );
    }

    #[test]
    fn test_parent_of_global_is_empty() {
        assert_eq!(eval("namespace parent"), "");
        assert_eq!(eval("namespace parent ::"), "");
    }

    #[test]
    fn test_delete_unknown_message_and_silent_global() {
        assert_eq!(
            eval_err("namespace delete ::nope"),
            "unknown namespace \"::nope\" in namespace delete command"
        );
        assert_eq!(eval_err("namespace delete nosuch"), "unknown namespace \"nosuch\" in namespace delete command");
        assert_eq!(eval("set x 5; namespace delete ::; set x"), "5");
    }

    #[test]
    fn test_subcommand_abbreviation_and_errors() {
        assert_eq!(eval("namespace ch :: nosuch_*"), "");
        assert_eq!(
            eval_err("namespace zz"),
            "unknown or ambiguous subcommand \"zz\": must be children, code, current, delete, ensemble, eval, exists, export, forget, import, inscope, origin, parent, path, qualifiers, tail, unknown, upvar, or which"
        );
        assert_eq!(
            eval_err("namespace c"),
            "unknown or ambiguous subcommand \"c\": must be children, code, or current"
        );
        assert_eq!(
            eval_err("namespace"),
            "wrong # args: should be \"namespace subcommand ?arg ...?\""
        );
    }

    #[test]
    fn test_unknown_handler_get_set() {
        assert_eq!(eval("namespace unknown"), "::unknown");
        assert_eq!(eval("namespace eval t {namespace unknown}"), "");
        assert_eq!(
            eval("namespace unknown myhandler; namespace unknown"),
            "myhandler"
        );
        assert_eq!(
            eval("namespace eval t {namespace unknown myh}; namespace eval t {namespace unknown}"),
            "myh"
        );
    }

    #[test]
    fn test_qualified_builtin_command() {
        assert_eq!(eval("::set x 7; ::set x"), "7");
        assert_eq!(eval("::list 1 2 3"), "1 2 3");
    }
}
