//! Procedure-related commands: proc, eval, uplevel, upvar, global, rename.

use crate::error::{Error, Result};
use crate::interp::{Interp, ProcDef, UpvarLink};
use crate::value::Value;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

/// Parse a proc/lambda parameter list into (name, default) pairs.
/// Tcl validates the specifiers at definition time:
/// `{}` → "argument with no name", `{a b c}` → "too many fields ...".
fn parse_param_specs(params: &[Value]) -> Result<Vec<(String, Option<String>)>> {
    let mut specs: Vec<(String, Option<String>)> = Vec::new();
    for param in params {
        let parts = param.as_list().unwrap_or_else(|| vec![param.clone()]);
        if parts.is_empty() {
            return Err(Error::Msg("argument with no name".to_string()));
        }
        if parts.len() > 2 {
            return Err(Error::Msg(format!(
                "too many fields in argument specifier \"{}\"",
                param.as_str()
            )));
        }
        if parts.len() == 2 {
            specs.push((
                parts[0].as_str().to_string(),
                Some(parts[1].as_str().to_string()),
            ));
        } else {
            specs.push((parts[0].as_str().to_string(), None));
        }
    }
    Ok(specs)
}

pub fn cmd_proc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // 3-arg form: proc name argList body
    // 4-arg form: proc name argList statics body  (jimtcl-compatible)
    if args.len() < 4 || args.len() > 5 {
        return Err(Error::wrong_args_with_usage(
            "proc", 4, args.len(), "name argList ?statics? body",
        ));
    }

    let raw_name = args[1].as_str();
    // Qualify the proc name if it names a namespace path or we're inside a
    // namespace context — `proc e1::cmd` at global and `namespace eval e1
    // {proc cmd}` must land on the SAME registered key (`::e1::cmd`,
    // tclsh), otherwise redefinition splits into two commands.
    let name = if raw_name.contains("::") || interp.current_namespace != "::" {
        super::namespace::qualify(&interp.current_namespace, raw_name)
    } else {
        raw_name.to_string()
    };

    let (param_arg, statics_arg, body_arg) = if args.len() == 5 {
        // 4-arg form: proc name argList statics body
        (&args[2], Some(&args[3]), &args[4])
    } else {
        // 3-arg form: proc name argList body
        (&args[2], None, &args[3])
    };

    let params = param_arg.as_list().unwrap_or_default();
    let body = body_arg.as_str().to_string();

    let defaults = parse_param_specs(&params)?;

    // Parse statics list: each element is {varName ?initialValue?}
    let mut statics = HashMap::new();
    if let Some(statics_val) = statics_arg {
        let static_list = statics_val.as_list().unwrap_or_default();
        for item in &static_list {
            let parts = item.as_list().unwrap_or_else(|| vec![item.clone()]);
            let var_name = parts[0].as_str().to_string();
            let init_val = if parts.len() >= 2 {
                Value::from_str(parts[1].as_str())
            } else {
                Value::empty()
            };
            statics.insert(var_name, init_val);
        }
    }

    let proc_def = ProcDef {
        params: defaults,
        body,
        statics,
    };

    interp.procs.insert(name, proc_def);
    Ok(Value::empty())
}

pub fn cmd_eval(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("eval", 2, args.len()));
    }

    if args.len() == 2 {
        eval_tagged(interp, args[1].as_str(), "eval")
    } else {
        let script: String = args[1..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        eval_tagged(interp, &script, "eval")
    }
}

/// Evaluate a nested script for `eval`/`uplevel`: a body error gains the
/// construct's exit frame — `("eval" body line N)` (F3) / `("uplevel"
/// body line N)` (N1) — before the enclosing harness names the command.
fn eval_tagged(interp: &mut Interp, script: &str, tag: &str) -> Result<Value> {
    let r = interp.eval(script);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            interp.err_exit_frame(&format!("\"{}\" body", tag));
        }
    }
    r
}

/// apply lambdaExpr ?arg ...?
/// lambdaExpr is a two-element list: {params body}
pub fn cmd_apply(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "apply",
            2,
            args.len(),
            "lambdaExpr ?arg ...?",
        ));
    }

    let lambda = args[1].as_list().ok_or_else(|| {
        Error::runtime(
            format!(
                "can't interpret \"{}\" as a lambda expression",
                args[1].as_str()
            ),
            crate::error::ErrorCode::Generic,
        )
    })?;

    if lambda.len() < 2 || lambda.len() > 3 {
        return Err(Error::runtime(
            format!(
                "can't interpret \"{}\" as a lambda expression",
                args[1].as_str()
            ),
            crate::error::ErrorCode::Generic,
        ));
    }

    let param_list = lambda[0].as_list().unwrap_or_default();
    let body = lambda[1].as_str().to_string();

    // Optional third element: namespace in which the lambda body runs
    // (tclsh: `apply {{args} {body} ::ns}` — must already exist).
    let ns_override = if lambda.len() == 3 {
        let qualified =
            super::namespace::qualify(&interp.current_namespace, lambda[2].as_str());
        if !interp.namespaces.contains_key(&qualified) {
            return Err(Error::runtime(
                format!("namespace \"{}\" not found", qualified),
                crate::error::ErrorCode::NotFound,
            ));
        }
        Some(qualified)
    } else {
        None
    };

    // Build param defaults (same logic as cmd_proc)
    let defaults = parse_param_specs(&param_list)?;

    let proc_def = ProcDef {
        params: defaults,
        body,
        statics: HashMap::new(),
    };

    // Create args for call_proc: [name, arg1, arg2, ...]
    // The proc name renders in arity errors as "apply lambdaExpr" (Tcl-compatible).
    let mut call_args = vec![Value::from_str("apply")];
    for arg in &args[2..] {
        call_args.push(arg.clone());
    }

    let r = interp.call_proc(&proc_def, &call_args, "apply lambdaExpr", ns_override);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            // F5: `(lambda term "{} {boomap}" line N)` — the lambda TERM's
            // value renders in the tag.
            let tag = format!("lambda term \"{}\"", args[1].as_str());
            interp.err_exit_frame(&tag);
        }
    }
    r
}

pub fn cmd_uplevel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("uplevel", 2, args.len()));
    }

    let explicit_global = args.len() > 2 && args[1].as_str().starts_with('#');
    let script_start = if args.len() > 2 { 2usize } else { 1usize };
    let script = if args.len() - script_start == 1 {
        args[script_start].as_str().to_string()
    } else {
        args[script_start..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };

    // tclsh counts every varFrame — proc frames AND open `namespace
    // eval`s — as one level.  Rebuild that chain (bottom→top) from the
    // proc frames' `ns_depth` markers and the live ns-eval stack:
    // [ns evals below frame 0] frame 0 [ns evals between 0 and 1] …
    let scopes_total = interp.frames.len() + interp.ns_stack.len();
    let (num_to_pop_frames, ns_trim, target_ns) = if explicit_global {
        // `#N` names the absolute Nth scope above the global level
        // (#0 = the global scope itself); past the top: bad level.
        let lvl_text = args[1].as_str();
        let n: usize = lvl_text[1..].parse().unwrap_or(0);
        if n == 0 {
            (interp.frames.len(), 0usize, "::".to_string())
        } else if n - 1 >= scopes_total {
            return Err(Error::Msg(format!("bad level \"{}\"", lvl_text)));
        } else {
            uplevel_resolve(interp, n - 1)
        }
    } else {
        let n = match args.len() > 2 {
            true => args[1].as_int().unwrap_or(1).max(0) as usize,
            false => 1,
        };
        // Target scope index (bottom→top); `uplevel 1` is one scope OUT
        // from the innermost.  Past the top: bad level (uplevel-4.2).
        if n > scopes_total {
            return Err(Error::Msg(format!(
                "bad level \"{}\"",
                args[1].as_str()
            )));
        }
        let target = scopes_total as isize - n as isize - 1;
        if target < 0 {
            (interp.frames.len(), 0usize, "::".to_string())
        } else {
            uplevel_resolve(interp, target as usize)
        }
    };

    // Pop proc frames and trim ns evals down to the target scope, eval,
    // then restore everything.
    let split_point = interp.frames.len() - num_to_pop_frames;
    let saved_frames: Vec<_> = interp.frames.split_off(split_point);
    let saved_ns_stack: Vec<_> = interp.ns_stack.split_off(ns_trim);
    let saved_l0: Vec<_> = interp.ns_level0.split_off(ns_trim);
    let saved_ns = interp.current_namespace.clone();
    interp.current_namespace = target_ns;
    let result = eval_tagged(interp, &script, "uplevel");
    interp.current_namespace = saved_ns;
    interp.ns_level0.extend(saved_l0);
    interp.ns_stack.extend(saved_ns_stack);
    interp.frames.extend(saved_frames);
    result
}

/// Locate the scope at chain index `target` (bottom→top over interleaved
/// proc frames and open `namespace eval`s): returns the proc-frame split
/// point or the ns-eval stack position plus the scope's namespace.
fn uplevel_resolve(interp: &Interp, target: usize) -> (usize, usize, String) {
    let mut idx = target;
    let mut split: Option<usize> = None;
    let mut ns_at: Option<usize> = None;
    for (i, f) in interp.frames.iter().enumerate() {
        let below = f.ns_depth.min(interp.ns_stack.len());
        if idx < below {
            // An ns eval below this frame
            ns_at = Some(idx);
            break;
        }
        idx -= below;
        if idx == 0 {
            split = Some(i);
            break;
        }
        idx -= 1;
    }
    if split.is_none() && ns_at.is_none() && idx < interp.ns_stack.len() {
        // The ns evals above the outermost frame (or all of them at
        // global level)
        ns_at = Some(idx);
    }
    match (split, ns_at) {
        (Some(fi), _) => {
            let depth = interp.frames[fi].ns_depth.min(interp.ns_stack.len());
            let ns = interp.frames[fi]
                .ns
                .clone()
                .unwrap_or_else(|| "::".to_string());
            // Frames strictly above the target pop; `uplevel 0` stays in
            // the running frame.
            (interp.frames.len() - fi - 1, depth, ns)
        }
        (None, Some(k)) => (interp.frames.len(), k, interp.ns_stack[k].clone()),
        (None, None) => (interp.frames.len(), 0, "::".to_string()),
    }
}
pub fn cmd_upvar(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "?level? otherVar localVar ?otherVar localVar ...?"));
    }

    // If not inside a proc, upvar is a no-op
    if interp.frames.is_empty() {
        return Ok(Value::empty());
    }

    let (start, level_arg) = if args.len() > 3 {
        let t = args[1].as_str();
        // A leading integer (or `#N`) is a level; anything else starts
        // the var pairs (`upvar a x1 b x2` = two links at level 1).
        if t.starts_with('#') || t.parse::<i64>().is_ok() {
            (2usize, Some(t.to_string()))
        } else {
            (1usize, None)
        }
    } else {
        (1usize, None)
    };

    let current_idx = interp.frames.len() - 1;

    // Determine the target scope over the tclsh varFrame chain (proc
    // frames + open namespace evals, one level each).
    let scopes_total = interp.frames.len() + interp.ns_stack.len();
    let target: UpvarTarget = match level_arg.as_deref() {
        None => {
            // No level: upvar 1 — the caller's scope.  One scope out
            // from the innermost.
            let t = scopes_total as isize - 2;
            if t < 0 {
                let ns = interp.frames[current_idx]
                    .call_ns
                    .clone()
                    .unwrap_or_else(|| "::".to_string());
                UpvarTarget::Ns(ns)
            } else {
                upvar_target(interp, t as usize)
            }
        }
        Some(lt) if lt.starts_with('#') => {
            // `#N` = absolute Nth scope above global; #0 = global itself.
            let n: usize = lt[1..].parse().unwrap_or(0);
            if n == 0 {
                UpvarTarget::GlobalNs
            } else if n - 1 >= scopes_total {
                return Err(Error::Msg(format!("bad level \"{}\"", lt)));
            } else {
                upvar_target(interp, n - 1)
            }
        }
        Some(lt) => {
            let level: usize = lt.parse().unwrap_or(1);
            if level > scopes_total {
                return Err(Error::Msg(format!("bad level \"{}\"", lt)));
            }
            let t = scopes_total as isize - level as isize - 1;
            if t < 0 {
                // Fell off every scope: the CALLER's namespace context
                // (var-15.1: `namespace eval test A ...` + `upvar $name`
                // lands in ::test, not the proc's own ns).
                let ns = interp.frames[current_idx]
                    .call_ns
                    .clone()
                    .unwrap_or_else(|| "::".to_string());
                UpvarTarget::Ns(ns)
            } else {
                upvar_target(interp, t as usize)
            }
        }
    };

    // Create upvar links
    let mut i = start;
    while i + 1 < args.len() {
        let other_var = args[i].as_str().to_string();
        let local_var = args[i + 1].as_str().to_string();

        let link = match &target {
            UpvarTarget::GlobalNs => UpvarLink::Global(other_var),
            UpvarTarget::Ns(ns) => {
                UpvarLink::Global(interp.canonical_global_in(ns, &other_var))
            }
            UpvarTarget::Frame(fi) => UpvarLink::Frame {
                frame_index: *fi,
                var_name: other_var,
            },
        };

        interp.frames[current_idx].upvars.insert(local_var, link);
        i += 2;
    }

    Ok(Value::empty())
}

/// Which scope an `upvar` level names.
enum UpvarTarget {
    /// The true global namespace (`upvar #0`, fell off everything).
    GlobalNs,
    /// An open `namespace eval`'s (or fallen-off caller's) variable table.
    Ns(String),
    /// A proc frame by absolute index.
    Frame(usize),
}

/// Map a scope-chain index (bottom→top) to its upvar target.
fn upvar_target(interp: &Interp, target: usize) -> UpvarTarget {
    let mut idx = target;
    for (i, f) in interp.frames.iter().enumerate() {
        let below = f.ns_depth.min(interp.ns_stack.len());
        if idx < below {
            return UpvarTarget::Ns(interp.ns_stack[idx].clone());
        }
        idx -= below;
        if idx == 0 {
            return UpvarTarget::Frame(i);
        }
        idx -= 1;
    }
    if idx < interp.ns_stack.len() {
        UpvarTarget::Ns(interp.ns_stack[idx].clone())
    } else {
        UpvarTarget::GlobalNs
    }
}

pub fn cmd_global(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("global", 2, args.len()));
    }

    // At global level, global is a no-op
    if interp.frames.is_empty() {
        return Ok(Value::empty());
    }

    let current_idx = interp.frames.len() - 1;

    for arg in &args[1..] {
        let name = arg.as_str().to_string();
        // Create a link from local "name" to globals["name"]
        interp.frames[current_idx].upvars.insert(
            name.clone(),
            UpvarLink::Global(name),
        );
    }

    Ok(Value::empty())
}

pub fn cmd_rename(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage("rename", 3, args.len(), "oldName newName"));
    }

    let old_name = args[1].as_str().to_string();
    let new_name = args[2].as_str().to_string();

    // Command names resolve through the same fallback chain dispatch uses:
    // exact, namespace-qualified, then `::`-prefixed (tclsh: `rename
    // nb::pp nb::qq` finds the proc registered as `::nb::pp`).
    let old_key = resolve_command_key(interp, &old_name)
        .ok_or_else(|| {
            // tclsh: can't rename "name": command doesn't exist
            Error::runtime(
                format!("can't rename \"{}\": command doesn't exist", old_name),
                crate::error::ErrorCode::NotFound,
            )
        })?;
    let new_key = if new_name.is_empty() {
        String::new()
    } else if new_name.starts_with("::") {
        new_name.clone()
    } else if interp.current_namespace != "::" {
        super::namespace::qualify(&interp.current_namespace, &new_name)
    } else {
        new_name.clone()
    };

    // Rename in builtins
    if let Some(func) = interp.commands.remove(&old_key) {
        let cat = interp.command_categories.remove(&old_key);
        let meta = interp.command_meta.remove(&old_key);
        if !new_key.is_empty() {
            interp.commands.insert(new_key.clone(), func);
            if let Some(c) = cat {
                interp.command_categories.insert(new_key.clone(), c);
            }
            if let Some(m) = meta {
                interp.command_meta.insert(new_key.clone(), m);
            }
        }
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Rename in procs
    if let Some(proc_def) = interp.procs.remove(&old_key) {
        if !new_key.is_empty() {
            interp.procs.insert(new_key.clone(), proc_def);
        }
        move_alias_origins(interp, &old_key, &new_key);
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Rename a namespace ensemble command
    if let Some(def) = interp.ensembles.remove(&old_key) {
        if !new_key.is_empty() {
            interp.ensembles.insert(new_key.clone(), def);
        }
        move_alias_origins(interp, &old_key, &new_key);
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Rename an import alias: the alias entry itself moves (the origin
    // keeps its key; alias-table keys are always fully qualified).
    if let Some(origin) = interp.import_aliases.remove(&old_key) {
        if !new_key.is_empty() {
            let new_alias_key = if new_key.starts_with("::") {
                new_key.clone()
            } else {
                format!("::{}", new_key)
            };
            interp.import_aliases.insert(new_alias_key.clone(), origin);
            // Aliases imported *from* this alias track the origin command,
            // not its name, so they follow the rename.
            let moved: Vec<String> = interp
                .import_aliases
                .iter()
                .filter(|(_, o)| o.as_str() == old_key)
                .map(|(a, _)| a.clone())
                .collect();
            for dep in moved {
                interp.import_aliases.insert(dep, new_alias_key.clone());
            }
        } else {
            // Deleting the alias deletes its dependent re-imports too
            // (namespace-old-9.18).
            move_alias_origins(interp, &old_key, "");
        }
        return Ok(Value::empty());
    }

    Err(Error::runtime(
        format!("can't rename \"{}\": command doesn't exist", old_name),
        crate::error::ErrorCode::NotFound,
    ))
}

/// Post-rename bookkeeping shared by the builtin and proc paths: move
/// command traces to the new key, then fire rename/delete traces
/// (tclsh fires them after the operation; callback errors are
/// background errors and never fail the rename).
fn finish_cmd_rename(
    interp: &mut Interp,
    old_key: &str,
    new_key: &str,
    old_name: &str,
    new_name: &str,
) {
    if new_key.is_empty() {
        interp.fire_cmd_traces(old_key, old_name, "", "delete");
    } else {
        interp.rekey_cmd_traces(old_key, new_key);
        interp.fire_cmd_traces(new_key, old_name, new_name, "rename");
    }
}

/// Import aliases follow renames of their origin (tclsh 48.2: renaming
/// `foo::bar` keeps the imported `bar` dispatching).
pub(crate) fn move_alias_origins(interp: &mut Interp, old_key: &str, new_key: &str) {
    if old_key == new_key {
        return;
    }
    if new_key.is_empty() {
        // Deleting the origin deletes the imported alias with it (48.2:
        // `rename foo::bar2 {}` → exist spong = 0) — and transitively,
        // aliases imported *from* that alias (namespace-old-9.18:
        // renaming `test_ns_import::cmd1` {} removes the global `::cmd1`
        // and `test_ns_import_use::cmd1` re-imports of it).
        let mut removed_keys: Vec<String> = vec![old_key.to_string()];
        loop {
            let doomed: Vec<String> = interp
                .import_aliases
                .iter()
                .filter(|(_, origin)| removed_keys.iter().any(|k| origin == &k))
                .map(|(a, _)| a.clone())
                .collect();
            if doomed.is_empty() {
                break;
            }
            for alias in &doomed {
                interp.import_aliases.remove(alias);
                removed_keys.push(alias.clone());
            }
        }
    } else {
        let moved: Vec<String> = interp
            .import_aliases
            .iter()
            .filter(|(_, origin)| origin.as_str() == old_key)
            .map(|(a, _)| a.clone())
            .collect();
        for alias in moved {
            interp.import_aliases.insert(alias, new_key.to_string());
        }
    }
    // An ensemble renaming onto/away from an alias-resolved key is handled
    // by find_ensemble_key's alias chain at dispatch time.
}

/// Resolve a command name to its registered key, mirroring dispatch's
/// fallback chain (exact → namespace-qualified → `::`-prefixed).
pub(crate) fn resolve_command_key(interp: &Interp, name: &str) -> Option<String> {
    if interp.procs.contains_key(name) || interp.commands.contains_key(name) {
        return Some(name.to_string());
    }
    if interp.current_namespace != "::" && !name.starts_with("::") {
        let qualified =
            super::namespace::qualify(&interp.current_namespace, name);
        if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
            return Some(qualified);
        }
    }
    if !name.starts_with("::") && name.contains("::") {
        let qualified = format!("::{}", name);
        if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
            return Some(qualified);
        }
    }
    // Colon runs collapse in command names: `p1:::g` names `::p1::g`.
    if name.contains("::") {
        let norm = super::namespace::normalise(name);
        if norm != name
            && (interp.procs.contains_key(&norm) || interp.commands.contains_key(&norm))
        {
            return Some(norm);
        }
    }
    // A `::`-qualified simple name in the global namespace: builtins and
    // global procs are keyed unqualified (`rename ::unknown ...`, 52.6).
    if name.starts_with("::") && !name[2..].contains("::") {
        let bare = &name[2..];
        if interp.procs.contains_key(bare)
            || interp.commands.contains_key(bare)
            || interp.ensembles.contains_key(bare)
        {
            return Some(bare.to_string());
        }
    }
    // Namespace ensembles resolve like commands for rename.
    let mut keys: Vec<String> = vec![name.to_string()];
    if interp.current_namespace != "::" && !name.starts_with("::") {
        keys.push(super::namespace::qualify(&interp.current_namespace, name));
    }
    if !name.starts_with("::") {
        // A relative name from the global level names `::name`
        // (ensembles created with `-command ::foo`, renamed as `foo`).
        keys.push(format!("::{}", name));
    }
    if let Some(k) = keys.into_iter().find(|k| interp.ensembles.contains_key(k)) {
        return Some(k);
    }
    // Import aliases are real commands: renaming one moves the alias
    // itself, not the origin (48.2).
    let alias_key = if name.starts_with("::") {
        name.to_string()
    } else if interp.current_namespace != "::" {
        super::namespace::qualify(&interp.current_namespace, name)
    } else {
        format!("::{}", name)
    };
    if interp.import_aliases.contains_key(&alias_key) {
        return Some(alias_key);
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_proc_statics_counter() {
        let mut interp = Interp::new();
        interp.eval("proc counter {} {{count 0}} { incr count; return $count }").unwrap();
        assert_eq!(interp.eval("counter").unwrap().as_str(), "1");
        assert_eq!(interp.eval("counter").unwrap().as_str(), "2");
        assert_eq!(interp.eval("counter").unwrap().as_str(), "3");
    }

    #[test]
    fn test_proc_statics_multiple() {
        let mut interp = Interp::new();
        interp.eval("proc accum {val} {{sum 0} {n 0}} { incr n; set sum [expr {$sum + $val}]; return \"$sum $n\" }").unwrap();
        assert_eq!(interp.eval("accum 10").unwrap().as_str(), "10 1");
        assert_eq!(interp.eval("accum 20").unwrap().as_str(), "30 2");
        assert_eq!(interp.eval("accum 5").unwrap().as_str(), "35 3");
    }

    #[test]
    fn test_proc_statics_default_empty() {
        let mut interp = Interp::new();
        // Static with no initial value defaults to empty string
        interp.eval("proc setter {} {x} { if {$x eq {}} { set x hello }; return $x }").unwrap();
        assert_eq!(interp.eval("setter").unwrap().as_str(), "hello");
        assert_eq!(interp.eval("setter").unwrap().as_str(), "hello");
    }

    #[test]
    fn test_proc_statics_persists_across_calls() {
        let mut interp = Interp::new();
        interp.eval("proc tracker {} {{items {}}} { append items x; return $items }").unwrap();
        assert_eq!(interp.eval("tracker").unwrap().as_str(), "x");
        assert_eq!(interp.eval("tracker").unwrap().as_str(), "xx");
        assert_eq!(interp.eval("tracker").unwrap().as_str(), "xxx");
    }

    #[test]
    fn test_proc_statics_with_args() {
        let mut interp = Interp::new();
        interp.eval("proc add_to {val} {{total 0}} { set total [expr {$total + $val}]; return $total }").unwrap();
        assert_eq!(interp.eval("add_to 5").unwrap().as_str(), "5");
        assert_eq!(interp.eval("add_to 3").unwrap().as_str(), "8");
        assert_eq!(interp.eval("add_to 2").unwrap().as_str(), "10");
    }

    #[test]
    fn test_proc_no_statics_unchanged() {
        // Standard 3-arg proc still works
        let mut interp = Interp::new();
        interp.eval("proc double {x} { expr {$x * 2} }").unwrap();
        assert_eq!(interp.eval("double 5").unwrap().as_str(), "10");
    }

    #[test]
    fn test_info_statics_shows_values() {
        let mut interp = Interp::new();
        interp.eval("proc counter {} {{count 0}} { incr count; return $count }").unwrap();
        interp.eval("counter").unwrap();
        interp.eval("counter").unwrap();
        let r = interp.eval("info statics counter").unwrap();
        assert_eq!(r.as_str(), "count 2");
    }
}

#[cfg(test)]
mod apply_rename_tests {
    use crate::interp::Interp;

    // -- apply lambdaExpr shapes (tclsh 8.6.17) --

    #[test]
    fn test_apply_non_lambda_message() {
        let mut interp = Interp::new();
        let e = interp.eval("apply xx").unwrap_err().to_string();
        assert_eq!(e, "can't interpret \"xx\" as a lambda expression");
    }

    #[test]
    fn test_apply_one_element_lambda_message() {
        let mut interp = Interp::new();
        let e = interp.eval("apply {x}").unwrap_err().to_string();
        assert_eq!(e, "can't interpret \"x\" as a lambda expression");
    }

    #[test]
    fn test_apply_four_element_lambda_message() {
        let mut interp = Interp::new();
        let e = interp
            .eval(r#"apply {{a} {set a} ::NN extra} 1"#)
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "can't interpret \"{a} {set a} ::NN extra\" as a lambda expression"
        );
    }

    // -- apply namespace element --

    #[test]
    fn test_apply_runs_in_named_namespace() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("namespace eval NN10 {}; apply {{} {namespace current} ::NN10}")
                .unwrap()
                .as_str(),
            "::NN10"
        );
        // relative name qualifies against the current namespace
        assert_eq!(
            interp
                .eval("namespace eval NN13 {}; apply {{} {namespace current} NN13}")
                .unwrap()
                .as_str(),
            "::NN13"
        );
    }

    #[test]
    fn test_apply_ns_resolves_procs_of_that_ns() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("namespace eval NN11 {proc helper {} {return H}}; apply {{} {helper} ::NN11}")
                .unwrap()
                .as_str(),
            "H"
        );
    }

    #[test]
    fn test_apply_ns_must_exist() {
        let mut interp = Interp::new();
        let e = interp
            .eval(r#"apply [list x {set x 1} ::NONEXIST::FOR::SURE] x"#)
            .unwrap_err()
            .to_string();
        assert_eq!(e, "namespace \"::NONEXIST::FOR::SURE\" not found");
        let e2 = interp
            .eval("apply {{a} {set a} NOPE} 1")
            .unwrap_err()
            .to_string();
        assert_eq!(e2, "namespace \"::NOPE\" not found");
    }

    // -- rename of namespace-qualified procs --

    #[test]
    fn test_rename_qualified_proc() {
        // tclsh: rename test_ns_basic::p test_ns_basic::q works
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("namespace eval nb {proc pp {} {return 1}}; rename nb::pp nb::qq; nb::qq")
                .unwrap()
                .as_str(),
            "1"
        );
    }

    #[test]
    fn test_rename_qualified_delete() {
        let mut interp = Interp::new();
        let r = interp
            .eval("namespace eval nb2 {proc pp {} {return 1}}; rename nb2::pp {}; info procs nb2::*")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "");
    }

    #[test]
    fn test_rename_missing_message() {
        // tclsh: can't rename "name": command doesn't exist
        let mut interp = Interp::new();
        let e = interp
            .eval("catch {rename nosuch::cmd other} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(e, "can't rename \"nosuch::cmd\": command doesn't exist");
    }
}
