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

/// `variable ?name ?value? ...?`
pub fn cmd_variable(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "variable", 2, args.len(),
            "?name value...? name ?value?",
        ));
    }

    let ns = interp.current_namespace.clone();
    let mut i = 1;
    while i < args.len() {
        let raw_name = args[i].as_str();
        let qualified = var_key(&qualify(&ns, raw_name));

        // If an initial value is provided, set it.  A bare `variable name`
        // declares the link without creating the variable (tclsh:
        // `namespace eval n {variable v}; info exists n::v` → 0).
        if i + 1 < args.len() {
            let val = args[i + 1].clone();
            interp.globals.insert(qualified.clone(), val);
            i += 2;
        } else {
            i += 1;
        }

        // Register the name in the namespace's variable table either way.
        if let Some(info) = interp.namespaces.get_mut(&ns) {
            info.variables.insert(qualified.clone());
        }

        // If we're inside a proc, create an upvar link from the local
        // name to the namespace-qualified global name.
        if !interp.frames.is_empty() {
            let local_name = ns_tail_str(raw_name).to_string();
            let frame_idx = interp.frames.len() - 1;
            interp.frames[frame_idx].upvars.insert(
                local_name,
                crate::interp::UpvarLink::Global(qualified),
            );
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

    // Push namespace context
    let prev = std::mem::replace(&mut interp.current_namespace, qualified);
    let result = interp.eval(&body);
    interp.current_namespace = prev;
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

        // Remove procs defined in this namespace
        interp.procs.retain(|k, _| k != &qualified && !k.starts_with(&prefix));

        // Remove namespace-scoped global variables (flat keys, no leading ::)
        let var_prefix = var_key(&prefix);
        interp.globals.retain(|k, _| !k.starts_with(&var_prefix));

        // Remove import aliases defined in, or pointing into, the tree.
        interp.import_aliases.retain(|alias, origin| {
            !alias.starts_with(&prefix) && !origin.starts_with(&prefix)
        });
        interp.ns_unknown.retain(|k, _| k != &qualified && !k.starts_with(&prefix));
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
        // (tclsh 8.6.17 namespace-22.1 chain).
        let proc_keys: Vec<String> = interp.procs.keys().cloned().collect();
        let alias_keys: Vec<String> = interp.import_aliases.keys().cloned().collect();
        let matching: Vec<(String, String)> = proc_keys
            .iter()
            .chain(alias_keys.iter())
            .filter(|k| {
                k.strip_prefix(&prefix)
                    .filter(|rest| !rest.contains("::") && crate::interp::glob_match(&tail, rest))
                    .filter(|rest| exports.iter().any(|e| crate::interp::glob_match(e, rest)))
                    .is_some()
            })
            .map(|k| (ns_tail_str(k).to_string(), k.clone()))
            .collect();

        for (short_name, full_name) in matching {
            let target = qualify(&cur, &short_name);
            if let Some(existing) = interp.import_aliases.get(&target) {
                if existing == &full_name {
                    continue; // re-import of the same origin: silent
                }
                if !force {
                    return Err(Error::runtime(
                        format!("can't import command \"{}\": already exists", short_name),
                        ErrorCode::InvalidOp,
                    ));
                }
            } else if interp.procs.contains_key(&target) && !force {
                return Err(Error::runtime(
                    format!("can't import command \"{}\": already exists", short_name),
                    ErrorCode::InvalidOp,
                ));
            }
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
/// exist.
fn ns_inscope(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace inscope", 4, args.len(),
            "name arg ?arg...?",
        ));
    }
    require_ns(interp, args[2].as_str())?;
    ns_eval(interp, args)
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

/// `namespace ensemble subcommand ?arg ...?`
///
/// Full ensemble dispatch is not implemented yet; the argument errors
/// match tclsh so bare/abbreviated forms behave (real ensembles follow in
/// a later batch).
fn ns_ensemble(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace ensemble", 3, args.len(),
            "subcommand ?arg ...?",
        ));
    }
    Err(Error::runtime(
        format!(
            "bad subcommand \"{}\": must be configure, create, or exists",
            args[2].as_str()
        ),
        ErrorCode::InvalidOp,
    ))
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
        assert_eq!(
            eval("namespace eval q {variable w:::x 9}; set ::q::w::x"),
            "9"
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
