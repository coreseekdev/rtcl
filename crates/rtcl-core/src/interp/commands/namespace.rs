//! Namespace support for the rtcl interpreter.
//!
//! Tcl namespaces provide hierarchical scoping for commands and variables.
//! Namespace names are separated by `::`.  The global namespace is `::`.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

/// Metadata for a single namespace.
#[derive(Debug, Clone, Default)]
pub(crate) struct NamespaceInfo {
    pub export_patterns: Vec<String>,
}

// ── namespace command ──────────────────────────────────────────────────

/// `namespace subcommand ?arg ...?`
pub fn cmd_namespace(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "namespace", 2, args.len(),
            "namespace subcommand ?arg ...?",
        ));
    }
    let subcmd = args[1].as_str();
    match subcmd {
        "eval"       => ns_eval(interp, args),
        "current"    => ns_current(interp),
        "delete"     => ns_delete(interp, args),
        "exists"     => ns_exists(interp, args),
        "parent"     => ns_parent(interp, args),
        "children"   => ns_children(interp, args),
        "qualifiers" => ns_qualifiers(args),
        "tail"       => ns_tail(args),
        "which"      => ns_which(interp, args),
        "origin"     => ns_origin(interp, args),
        "code"       => ns_code(interp, args),
        "export"     => ns_export(interp, args),
        "import"     => ns_import(interp, args),
        "inscope"    => ns_inscope(interp, args),
        "path"       => ns_path(interp, args),
        _ => Err(Error::runtime(
            format!(
                "bad option \"{}\": must be children, code, current, delete, eval, \
                 exists, export, import, inscope, origin, parent, path, qualifiers, \
                 tail, or which",
                subcmd
            ),
            ErrorCode::InvalidOp,
        )),
    }
}

/// `variable ?name ?value? ...?`
pub fn cmd_variable(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "variable", 2, args.len(),
            "variable ?name value...? name ?value?",
        ));
    }

    let ns = interp.current_namespace.clone();
    let mut i = 1;
    while i < args.len() {
        let raw_name = args[i].as_str();
        let qualified = qualify(&ns, raw_name);

        // If an initial value is provided, set it
        if i + 1 < args.len() {
            let val = args[i + 1].clone();
            interp.globals.insert(qualified.clone(), val);
            i += 2;
        } else {
            // Ensure the variable exists (even if empty)
            if !interp.globals.contains_key(&qualified) {
                interp.globals.insert(qualified.clone(), Value::empty());
            }
            i += 1;
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

/// `namespace eval namespace arg ?arg ...?`
fn ns_eval(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace eval", 4, args.len(),
            "namespace eval name arg ?arg ...?",
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

/// `namespace current`
fn ns_current(interp: &Interp) -> Result<Value> {
    Ok(Value::from_str(&interp.current_namespace))
}

/// `namespace delete ?namespace ...?`
fn ns_delete(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    for arg in &args[2..] {
        let qualified = qualify(&interp.current_namespace, arg.as_str());
        if qualified == "::" {
            return Err(Error::runtime(
                "cannot delete the global namespace",
                ErrorCode::InvalidOp,
            ));
        }
        // Remove the namespace and all children
        let prefix = format!("{}::", qualified);
        interp.namespaces.retain(|k, _| k != &qualified && !k.starts_with(&prefix));

        // Remove procs defined in this namespace
        interp.procs.retain(|k, _| k != &qualified && !k.starts_with(&prefix));

        // Remove namespace-scoped global variables
        interp.globals.retain(|k, _| !k.starts_with(&prefix));
    }
    Ok(Value::empty())
}

/// `namespace exists name`
fn ns_exists(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace exists", 3, args.len(),
            "namespace exists name",
        ));
    }
    let qualified = qualify(&interp.current_namespace, args[2].as_str());
    Ok(Value::from_bool(interp.namespaces.contains_key(&qualified)))
}

/// `namespace parent ?namespace?`
fn ns_parent(interp: &Interp, args: &[Value]) -> Result<Value> {
    let ns = if args.len() >= 3 {
        qualify(&interp.current_namespace, args[2].as_str())
    } else {
        interp.current_namespace.clone()
    };
    if ns == "::" {
        return Err(Error::runtime(
            "namespace \"::\" has no parent namespace",
            ErrorCode::InvalidOp,
        ));
    }
    let parent = parent_ns(&ns);
    Ok(Value::from_str(&parent))
}

/// `namespace children ?namespace? ?pattern?`
fn ns_children(interp: &Interp, args: &[Value]) -> Result<Value> {
    let ns = if args.len() >= 3 {
        qualify(&interp.current_namespace, args[2].as_str())
    } else {
        interp.current_namespace.clone()
    };
    let pattern = if args.len() >= 4 { Some(args[3].as_str()) } else { None };

    let prefix = if ns == "::" { "::".to_string() } else { format!("{}::", ns) };
    let mut children = Vec::new();
    for key in interp.namespaces.keys() {
        if key == &ns { continue; }
        // Direct child: starts with prefix and no further `::`
        if let Some(rest) = key.strip_prefix(&prefix) {
            if !rest.contains("::") {
                if let Some(pat) = pattern {
                    if crate::interp::glob_match(pat, key) {
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

/// `namespace qualifiers string` — pure string operation
fn ns_qualifiers(args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace qualifiers", 3, args.len(),
            "namespace qualifiers string",
        ));
    }
    let name = args[2].as_str();
    let q = ns_qualifiers_str(name);
    Ok(Value::from_str(q))
}

/// `namespace tail string` — pure string operation
fn ns_tail(args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace tail", 3, args.len(),
            "namespace tail string",
        ));
    }
    let name = args[2].as_str();
    let t = ns_tail_str(name);
    Ok(Value::from_str(t))
}

/// `namespace which ?-command|-variable? name`
fn ns_which(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace which", 3, args.len(),
            "namespace which ?-command? ?-variable? name",
        ));
    }

    let (kind, name) = if args.len() == 4 {
        let flag = args[2].as_str();
        match flag {
            "-command"  => ("command", args[3].as_str()),
            "-variable" => ("variable", args[3].as_str()),
            _ => return Err(Error::runtime(
                "wrong # args: should be \"namespace which ?-command? ?-variable? name\"".to_string(),
                ErrorCode::InvalidOp,
            )),
        }
    } else {
        ("command", args[2].as_str())  // default is -command
    };

    let qualified = qualify(&interp.current_namespace, name);

    match kind {
        "command" => {
            if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
                Ok(Value::from_str(&qualified))
            } else if interp.procs.contains_key(name) || interp.commands.contains_key(name) {
                Ok(Value::from_str(&qualify("::", name)))
            } else {
                Ok(Value::empty())
            }
        }
        "variable" => {
            if interp.globals.contains_key(&qualified) {
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

/// `namespace origin name` — returns the fully-qualified name of the
/// original command (before any imports).  For now we treat all commands
/// as originals.
fn ns_origin(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace origin", 3, args.len(),
            "namespace origin command",
        ));
    }
    let name = args[2].as_str();
    let qualified = qualify(&interp.current_namespace, name);
    if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
        Ok(Value::from_str(&qualified))
    } else if interp.procs.contains_key(name) || interp.commands.contains_key(name) {
        Ok(Value::from_str(&qualify("::", name)))
    } else {
        Err(Error::runtime(
            format!("\"{}\" is not a known command", name),
            ErrorCode::NotFound,
        ))
    }
}

/// `namespace code script` — wraps the script for later execution in
/// the current namespace context.
fn ns_code(interp: &Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace code", 3, args.len(),
            "namespace code script",
        ));
    }
    let script = args[2].as_str();
    let ns = &interp.current_namespace;
    // Tcl wraps it as: ::namespace inscope <ns> <script>
    Ok(Value::from_str(&format!("::namespace inscope {} {}", ns, script)))
}

/// `namespace export ?-clear? ?pattern ...?`
fn ns_export(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let ns = interp.current_namespace.clone();
    let info = interp.namespaces.entry(ns).or_default();

    let mut i = 2;
    if i < args.len() && args[i].as_str() == "-clear" {
        info.export_patterns.clear();
        i += 1;
    }
    while i < args.len() {
        info.export_patterns.push(args[i].as_str().to_string());
        i += 1;
    }
    // If no patterns given, return current export list
    if args.len() == 2 {
        return Ok(Value::from_str(&info.export_patterns.join(" ")));
    }
    Ok(Value::empty())
}

/// `namespace import ?-force? ?pattern ...?`
///
/// For now: accept the syntax but only store the import mappings.
fn ns_import(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut i = 2;
    let _force = if i < args.len() && args[i].as_str() == "-force" {
        i += 1;
        true
    } else {
        false
    };

    while i < args.len() {
        let pattern = args[i].as_str();
        // Resolve the source namespace and command pattern
        let qualified = qualify(&interp.current_namespace, pattern);
        let src_ns = ns_qualifiers_str(&qualified);
        let cmd_pat = ns_tail_str(&qualified);

        // Find matching commands in the source namespace
        let prefix = if src_ns == "::" {
            "::".to_string()
        } else {
            format!("{}::", src_ns)
        };

        let matching: Vec<(String, String)> = interp.procs.keys()
            .filter(|k| {
                if let Some(rest) = k.strip_prefix(&prefix) {
                    !rest.contains("::") && crate::interp::glob_match(cmd_pat, rest)
                } else {
                    false
                }
            })
            .map(|k| {
                let tail = ns_tail_str(k).to_string();
                (tail, k.clone())
            })
            .collect();

        // Create aliases in current namespace
        for (short_name, full_name) in matching {
            if let Some(proc_def) = interp.procs.get(&full_name).cloned() {
                let local = qualify(&interp.current_namespace, &short_name);
                interp.procs.insert(local, proc_def);
            }
        }
        i += 1;
    }

    Ok(Value::empty())
}

/// `namespace inscope namespace arg ?arg ...?`
fn ns_inscope(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "namespace inscope", 4, args.len(),
            "namespace inscope name arg ?arg ...?",
        ));
    }
    // inscope is like eval but with extra args appended
    ns_eval(interp, args)
}

/// `namespace path ?pathList?` — stub
fn ns_path(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "namespace path", 2, args.len(),
            "namespace path ?pathList?",
        ));
    }
    // TODO: command resolution path
    Ok(Value::empty())
}

// ── helper functions ───────────────────────────────────────────────────

/// Qualify a name relative to a namespace.
/// If the name already starts with `::`, return it as-is.
/// Otherwise, prepend the current namespace.
pub(crate) fn qualify(ns: &str, name: &str) -> String {
    if name.starts_with("::") {
        // Normalize: remove redundant leading `::`
        normalise(name)
    } else if ns == "::" {
        format!("::{}", name)
    } else {
        format!("{}::{}", ns, name)
    }
}

/// Normalise a fully-qualified name — collapse multiple `::` and ensure
/// the result starts with `::`.
fn normalise(name: &str) -> String {
    let parts: Vec<&str> = name.split("::").filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return "::".to_string();
    }
    format!("::{}", parts.join("::"))
}

/// Return the parent namespace of a fully-qualified namespace.
fn parent_ns(ns: &str) -> String {
    if ns == "::" {
        return "::".to_string();
    }
    let q = ns_qualifiers_str(ns);
    if q.is_empty() {
        "::".to_string()
    } else {
        q.to_string()
    }
}

/// Extract the qualifiers part (everything before the last `::`) from a name.
fn ns_qualifiers_str(name: &str) -> &str {
    // Find the last "::" — everything before it is the qualifier
    if let Some(pos) = name.rfind("::") {
        let q = &name[..pos];
        if q.is_empty() { "::" } else { q }
    } else {
        ""
    }
}

/// Extract the tail (everything after the last `::`) from a name.
fn ns_tail_str(name: &str) -> &str {
    if let Some(pos) = name.rfind("::") {
        &name[pos + 2..]
    } else {
        name
    }
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
