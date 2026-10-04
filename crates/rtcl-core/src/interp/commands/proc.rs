//! Procedure-related commands: proc, eval, uplevel, upvar, global, rename.

use crate::error::{Error, Result};
use crate::interp::{Interp, ProcDef, UpvarLink};
use crate::value::Value;

use super::super::Rc;

use super::list::{set_error_code, strict_list, tcl_get_int};

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

/// Parse a proc/lambda parameter list into (name, default) pairs.
/// Tcl validates the specifiers at definition time, per parameter, in
/// this order (probed on 8.6.17): too many fields → contains `::` →
/// contains `(` → empty (`a::b(1)` reports "not a simple name", not
/// "array element").
fn parse_param_specs(params: &[Value]) -> Result<Vec<(String, Option<String>)>> {
    let mut specs: Vec<(String, Option<String>)> = Vec::new();
    for param in params {
        let parts = param.as_list().unwrap_or_else(|| vec![param.clone()]);
        if parts.len() > 2 {
            return Err(Error::Msg(format!(
                "too many fields in argument specifier \"{}\"",
                param.as_str()
            )));
        }
        if !parts.is_empty() {
            let name = parts[0].as_str();
            if name.contains("::") {
                return Err(Error::Msg(format!(
                    "formal parameter \"{}\" is not a simple name",
                    name
                )));
            }
            if name.contains('(') {
                return Err(Error::Msg(format!(
                    "formal parameter \"{}\" is an array element",
                    name
                )));
            }
        }
        if parts.is_empty() {
            return Err(Error::Msg("argument with no name".to_string()));
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
    // 4-arg form: proc name argList statics body  (jimtcl-compatible
    // extension kept for compatibility; the arity error still quotes
    // tclsh 8.6's standard usage — proc-old-5.1..5.3).
    if args.len() < 4 || args.len() > 5 {
        return Err(Error::wrong_args_with_usage(
            "proc", 4, args.len(), "name args body",
        ));
    }

    let raw_name = args[1].as_str();
    // Qualify the proc name if it names a namespace path or we're inside a
    // namespace context — `proc e1::cmd` at global and `namespace eval e1
    // {proc cmd}` must land on the SAME registered key (`::e1::cmd`,
    // tclsh), otherwise redefinition splits into two commands.
    let name = if raw_name.contains("::") || interp.current_namespace.as_ref() != "::" {
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

    // The argument list is split with Tcl_SplitList: a malformed list
    // (`proc t {a` …) errors at definition time with the list-parse
    // message and `TCL VALUE LIST …` errorCode (proc-old-5.4).
    let params = strict_list(interp, param_arg)?;
    let body = body_arg.as_str().to_string();

    // Definition-time parameter errors carry a `(creating proc "name")`
    // frame: pre-seed errorInfo so the harness logs the `proc` command
    // as `invoked from within` (tclsh ERR_ALREADY_LOGGED). Malformed
    // per-argument specifiers also raise
    // `TCL OPERATION PROC FORMALARGUMENTFORMAT` (proc-old-5.5..5.7).
    let defaults = match parse_param_specs(&params) {
        Ok(d) => d,
        Err(e) => {
            if let Error::Msg(_) = &e {
                set_error_code(interp, "TCL OPERATION PROC FORMALARGUMENTFORMAT");
            }
            interp.err_info = Some(format!(
                "{}\n    (creating proc \"{}\")",
                e,
                args[1].as_str()
            ));
            return Err(e);
        }
    };

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

    let compiled =
        super::super::vm_exec::compile_proc_body(&defaults, &body, interp.tier1_epoch);
    if let Some(code) = &compiled {
        interp.const_pool_insert(code);
    }
    let proc_def = ProcDef {
        params: Rc::new(defaults),
        body: Rc::from(body),
        statics: Rc::new(statics),
        compiled,
    };

    // Redefining a command is a silent delete+create (tclsh 8.6.17): no
    // rename/delete trace fires and the old command's stored traces are
    // dropped (trace-19.5, trace-20.3.1).
    if interp.procs.contains_key(&name)
        || interp.commands.contains_key(&name)
        || interp.ensembles.contains_key(&name)
        || interp.import_aliases.contains_key(&name)
    {
        interp.wipe_cmd_exec_traces(&name);
    }
    // A proc named like an inline-folded Tier1 command (`proc set {...}`)
    // invalidates every compiled body that folded that command.
    super::super::vm_exec::note_tier1_mutation(interp, &name);
    interp.note_cmd_mutation();
    interp.procs.insert(name, super::super::Rc::new(proc_def));
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
            if interp.err_info.is_none() {
                if let Error::Syntax { .. } = e {
                    // A parse failure inside eval/uplevel has no parsed
                    // command to name: tclsh logs the SCRIPT text itself
                    // as the failing "command", with TclMaxLogLength
                    // truncation (parseOld-10.14).
                    interp.err_info = Some(format!(
                        "{}\n    while executing\n\"{}\"",
                        e,
                        crate::interp::eval::tcl_log_excerpt(script)
                    ));
                }
            }
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

    // Build param defaults (same logic as cmd_proc).  Parse errors carry
    // a `(parsing lambda expression "<term>")` frame between the message
    // and the caller's harness frame: pre-seed errorInfo so the harness
    // logs the command as `invoked from within` (tclsh ERR_ALREADY_LOGGED).
    let defaults = match parse_param_specs(&param_list) {
        Ok(d) => d,
        Err(e) => {
            interp.err_info = Some(format!(
                "{}\n    (parsing lambda expression \"{}\")",
                e,
                args[1].as_str()
            ));
            return Err(e);
        }
    };

    // tclsh compiles lambda bodies like proc bodies (a proc-context unit
    // with compiledLocals): inline foreach/lmap, frameless construct-body
    // errors, plain loop-variable writes.  Memoised by the term string —
    // the term IS the lambda's identity — and gated per call by the same
    // epoch/trace checks as proc bodies in `call_proc`.
    let compiled = match interp.lambda_code_cache.get(args[1].as_str()) {
        Some(c) => Some(Rc::clone(c)),
        None => {
            let c = super::super::vm_exec::compile_proc_body(
                &defaults,
                &body,
                interp.tier1_epoch,
            );
            if let Some(code) = &c {
                interp.const_pool_insert(code);
                if interp.lambda_code_cache.len() >= 1024 {
                    interp.lambda_code_cache.clear();
                }
                interp.lambda_code_cache
                    .insert(args[1].as_str().to_string(), Rc::clone(code));
            }
            c
        }
    };

    let proc_def = ProcDef {
        params: Rc::new(defaults),
        body: Rc::from(body),
        statics: Rc::new(HashMap::new()),
        compiled,
    };

    // Create args for call_proc: [name, arg1, arg2, ...]
    // The proc name renders in arity errors as "apply lambdaExpr" (Tcl-compatible).
    let mut call_args = vec![Value::from_str("apply")];
    for arg in &args[2..] {
        call_args.push(arg.clone());
    }

    // `info level 0` inside the lambda shows `apply {<term>} <args...>`:
    // the term as ONE list element (brace-wrapped) plus the arguments.
    let mut level0 = vec![Value::from_str("apply"), args[1].clone()];
    level0.extend(args[2..].iter().cloned());
    interp.frame_level0_args = Some(level0);

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

/// A parsed `upvar`/`uplevel` level word.  `#N` is absolute (chain index
/// above the global scope), plain integers are relative to the innermost
/// scope.  The original word is kept for `bad level` messages.
enum LevelSpec {
    Rel(i64, String),
    Abs(usize, String),
}

/// A word is a level candidate iff its first char is `#`, `+`, `-` or a
/// digit (tclsh's TclGetLevelFromArg inspects the first byte only:
/// `uplevel .2 {}` runs `.2 {}` as the script).
fn level_shaped(w: &str) -> bool {
    matches!(
        w.as_bytes().first(),
        Some(b'#') | Some(b'+') | Some(b'-') | Some(b'0'..=b'9')
    )
}

/// The stricter shape `upvar` requires at the true global level: `#`
/// (with anything after it), a digit, or a sign followed by a digit.
/// A word that fails this is NOT a level there — `upvar a x y` and
/// `upvar - x y` fall back to pairs at the default level (bad level
/// "1"), while `upvar 0x x y` and `upvar 1.5 x y` error as-typed.
fn level_shaped_global(w: &str) -> bool {
    let b = w.as_bytes();
    match b.first() {
        Some(b'#') => true,
        Some(b'0'..=b'9') => true,
        Some(b'+') | Some(b'-') => matches!(b.get(1), Some(b'0'..=b'9')),
        _ => false,
    }
}

/// Parse a level word with Tcl's integer grammar (`#-1` and `#0.2` fail,
/// `#010` is octal 8, `+2` is relative 2).  Failure yields the word
/// verbatim for the error message.
fn parse_level(spec: &str) -> std::result::Result<LevelSpec, String> {
    if let Some(rest) = spec.strip_prefix('#') {
        return match tcl_get_int(rest) {
            Some(n) if n >= 0 => Ok(LevelSpec::Abs(n as usize, spec.to_string())),
            _ => Err(spec.to_string()),
        };
    }
    match tcl_get_int(spec) {
        Some(n) => Ok(LevelSpec::Rel(n, spec.to_string())),
        None => Err(spec.to_string()),
    }
}

/// Tcl's `bad level` error with its `TCL LOOKUP LEVEL <word>` errorCode.
fn bad_level(interp: &mut Interp, spec: &str) -> Error {
    set_error_code(interp, &format!("TCL LOOKUP LEVEL {}", spec));
    Error::Msg(format!("bad level \"{}\"", spec))
}

pub fn cmd_uplevel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "uplevel",
            2,
            args.len(),
            "?level? command ?arg ...?",
        ));
    }

    // The level word is present iff args[1] merely LOOKS like a level —
    // shape only, no arity check (`uplevel .2 {}` executes ".2 {}").
    let has_level = level_shaped(args[1].as_str());
    let spec = if has_level {
        match parse_level(args[1].as_str()) {
            Ok(s) => s,
            Err(w) => return Err(bad_level(interp, &w)),
        }
    } else {
        LevelSpec::Rel(1, "1".to_string())
    };
    let script_start = if has_level { 2usize } else { 1usize };

    // tclsh counts every varFrame — proc frames AND open `namespace
    // eval`s — as one level.  Rebuild that chain (bottom→top) from the
    // proc frames' `ns_depth` markers and the live ns-eval stack:
    // [ns evals below frame 0] frame 0 [ns evals between 0 and 1] …
    let scopes_total = interp.frames.len() + interp.ns_stack.len();
    let (num_to_pop_frames, ns_trim, target_ns) = match &spec {
        LevelSpec::Abs(n, word) => {
            // `#N` names the absolute Nth scope above the global level
            // (#0 = the global scope itself); past the top: bad level.
            if *n == 0 {
                (interp.frames.len(), 0usize, "::".to_string())
            } else if n - 1 >= scopes_total {
                return Err(bad_level(interp, word));
            } else {
                uplevel_resolve(interp, n - 1)
            }
        }
        LevelSpec::Rel(n, word) => {
            // Relative levels count OUT from the innermost scope; past
            // the top: bad level.  Negative levels report the decimal
            // magnitude (`uplevel -1` at global → bad level "1").
            let m = n.unsigned_abs() as usize;
            if m > scopes_total {
                let shown = if *n < 0 { m.to_string() } else { word.clone() };
                return Err(bad_level(interp, &shown));
            }
            let target = scopes_total as isize - m as isize - 1;
            if target < 0 {
                (interp.frames.len(), 0usize, "::".to_string())
            } else {
                uplevel_resolve(interp, target as usize)
            }
        }
    };

    // Range resolution precedes the empty-command arity check
    // (`uplevel 9` at global → bad level, not wrong # args).
    if script_start >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "uplevel",
            2,
            args.len(),
            "?level? command ?arg ...?",
        ));
    }

    let script = if args.len() - script_start == 1 {
        args[script_start].as_str().to_string()
    } else {
        args[script_start..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };

    // Pop proc frames and trim ns evals down to the target scope, eval,
    // then restore everything.
    let split_point = interp.frames.len() - num_to_pop_frames;
    let saved_frames: Vec<_> = interp.frames.split_off(split_point);
    let saved_ns_stack: Vec<_> = interp.ns_stack.split_off(ns_trim);
    let saved_l0: Vec<_> = interp.ns_level0.split_off(ns_trim);
    let saved_ns = interp.current_namespace.clone();
    interp.current_namespace = Rc::from(target_ns);
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
                .as_deref()
                .unwrap_or("::")
                .to_string();
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

    // Level-word detection: PARITY decides everywhere — an even argument
    // count nominates args[1] as the level, an odd count makes it the
    // first otherVar (`upvar 0 x` at global is the pair ("0", "x")).  A
    // nominated word parses STRICTLY inside any enclosing scope (`upvar
    // a b c` → bad level "a"); at the TRUE global level (no frames, no
    // ns evals) a word that doesn't look like a level falls back to
    // pairs at the default level instead (`upvar a x y` → bad level
    // "1", but `upvar 0x x y` → bad level "0x").
    let has_level = if args.len() % 2 == 0 {
        !interp.frames.is_empty()
            || !interp.ns_stack.is_empty()
            || level_shaped_global(args[1].as_str())
    } else {
        false
    };
    let spec = if has_level {
        match parse_level(args[1].as_str()) {
            Ok(s) => s,
            Err(w) => return Err(bad_level(interp, &w)),
        }
    } else {
        LevelSpec::Rel(1, "1".to_string())
    };
    let start = if has_level { 2usize } else { 1usize };
    if start >= args.len() {
        return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "?level? otherVar localVar ?otherVar localVar ...?"));
    }

    let scopes_total = interp.frames.len() + interp.ns_stack.len();
    let scope: TargetScope = match &spec {
        LevelSpec::Abs(n, word) => {
            if *n == 0 {
                TargetScope::Global
            } else if n - 1 >= scopes_total {
                return Err(bad_level(interp, word));
            } else {
                TargetScope::Idx(n - 1)
            }
        }
        LevelSpec::Rel(n, word) => {
            let m = n.unsigned_abs() as usize;
            if m > scopes_total {
                let shown = if *n < 0 { m.to_string() } else { word.clone() };
                return Err(bad_level(interp, &shown));
            }
            let t = scopes_total as isize - m as isize - 1;
            if t < 0 {
                TargetScope::Bottom
            } else {
                TargetScope::Idx(t as usize)
            }
        }
    };

    // At the global level (no call frames) upvar still links two
    // variables — the link is an eval-level flat alias (var-3.10:
    // `namespace eval {} { variable bar 0; namespace eval foo { upvar
    // bar bar } }` links foo::bar → bar).
    if interp.frames.is_empty() {
        let target_ns = match scope {
            TargetScope::Global | TargetScope::Bottom => "::".to_string(),
            TargetScope::Idx(k) => interp.ns_stack[k].clone(),
        };
        let mut i = start;
        while i + 1 < args.len() {
            let other_var = args[i].as_str();
            let local_var = args[i + 1].as_str();
            let target_key = if target_ns == "::" {
                other_var.to_string()
            } else {
                interp.canonical_global_in(&target_ns, other_var)
            };
            let alias_key = interp.canonical_global(local_var);
            // Same container creation as the frame path: an element link
            // makes the ARRAY exist (`upvar 0 a(e) x` → array exists a).
            if let Some(paren) = target_key.find('(') {
                let base = &target_key[..paren];
                if !interp.is_array_semantic(base) && !interp.var_exists(base) {
                    let _ = interp.mark_array(base);
                }
            }
            // Level 0 at the global level names the current scope
            // itself: `upvar 0 zz zz` is a self-reference (tclsh
            // 8.6.17).
            if target_key == alias_key {
                set_error_code(interp, "TCL UPVAR SELF");
                return Err(Error::Msg(
                    "can't upvar from variable to itself".to_string(),
                ));
            }
            match interp
                .flat_aliases
                .iter_mut()
                .find(|(k, _)| *k == alias_key)
            {
                Some((_, t)) => *t = target_key,
                None => interp.flat_aliases.push((alias_key, target_key)),
            }
            i += 2;
        }
        return Ok(Value::empty());
    }

    let current_idx = interp.frames.len() - 1;

    // Map the scope to the upvar target over the tclsh varFrame chain
    // (proc frames + open namespace evals, one level each).
    let target: UpvarTarget = match scope {
        TargetScope::Global => UpvarTarget::GlobalNs,
        TargetScope::Bottom => {
            // Fell off every scope: the CALLER's namespace context
            // (var-15.1: `namespace eval test A ...` + `upvar $name`
            // lands in ::test, not the proc's own ns).
            let ns = interp.frames[current_idx]
                .call_ns
                .clone()
                .unwrap_or_else(|| Rc::clone(&interp.ns_root));
            UpvarTarget::Ns(ns)
        }
        TargetScope::Idx(k) => upvar_target(interp, k),
    };

    // Create upvar links
    let mut i = start;
    while i + 1 < args.len() {
        let other_var = args[i].as_str().to_string();
        let local_var = args[i + 1].as_str().to_string();

        // Resolve the named variable through any existing upvar links in
        // the target scope, so aliasing an alias lands on the ULTIMATE
        // target (upvar-4.2: p3 aliases p2's alias of p1's local;
        // upvar-8.5: the reverse link closes a cycle onto itself).
        let mut link = match &target {
            UpvarTarget::GlobalNs => UpvarLink::Global(other_var.clone()),
            UpvarTarget::Ns(ns) => {
                UpvarLink::Global(interp.canonical_global_in(ns, &other_var))
            }
            UpvarTarget::Frame(fi) => UpvarLink::Frame {
                frame_index: *fi,
                var_name: other_var.clone(),
            },
        };
        for _ in 0..=interp.frames.len() {
            let next = match &link {
                UpvarLink::Frame { frame_index, var_name } => interp
                    .frames
                    .get(*frame_index)
                    .and_then(|f| f.upvars.get(var_name.as_str()))
                    .cloned(),
                UpvarLink::Global(_) | UpvarLink::Dead { .. } => None,
            };
            match next {
                Some(l) => link = l,
                None => break,
            }
        }

        // A frame-target link must land in the target's name-keyed store
        // (slot cells and upvar links do not alias): degrade the target
        // frame when the linked name sits in its slot table.
        if let UpvarLink::Frame { frame_index, var_name } = &link {
            interp.degrade_frame_local(*frame_index, var_name);
        }

        // Self-reference, direct or through a link chain.
        if let UpvarLink::Frame { frame_index, var_name } = &link {
            if *frame_index == current_idx && *var_name == local_var {
                set_error_code(interp, "TCL UPVAR SELF");
                return Err(Error::Msg(
                    "can't upvar from variable to itself".to_string(),
                ));
            }
        }

        // A traced variable can't be turned into an alias (upvar-8.7).
        // Whole-var traces only: an element trace (`trace add variable
        // a(1) ...`) marks `a` as an existing array instead, so the
        // exists check below reports it (probed on 8.6.17).
        let trace_key = format!("F{}:{}", current_idx, local_var);
        if interp.var_traces.contains_key(&trace_key) {
            set_error_code(interp, "TCL UPVAR TRACED");
            return Err(Error::Msg(format!(
                "variable \"{}\" has traces: can't use for upvar",
                local_var
            )));
        }

        // An existing local that is not itself an alias blocks the link;
        // replacing an existing alias is silent (upvar-6.1 re-links `x`
        // once per loop iteration).  The alias name leaves the slot model
        // first (links and slot cells don't alias; the exists check and
        // every later write go through the map).
        interp.degrade_frame_local(current_idx, &local_var);
        let has_link = interp.frames[current_idx].upvars.contains_key(&local_var);
        if !has_link
            && (interp.frames[current_idx].locals.contains_key(&local_var)
                || interp
                    .frames[current_idx]
                    .array_locals
                    .contains(&local_var))
        {
            set_error_code(interp, "TCL UPVAR EXISTS");
            return Err(Error::Msg(format!(
                "variable \"{}\" already exists",
                local_var
            )));
        }

        interp.frames[current_idx]
            .upvars
            .insert(local_var.clone(), link.clone());
        // Linking to an ELEMENT of a missing array still creates the
        // array container (not the element) in tclsh (probed:
        // `upvar 0 a(e) x` → `array exists a` = 1, `info exists x` = 0).
        if let UpvarLink::Global(gname) = &link {
            if let Some(paren) = gname.find('(') {
                let base = &gname[..paren];
                if !interp.is_array_semantic(base) && !interp.var_exists(base) {
                    let _ = interp.mark_array(base);
                }
            }
        }
        // The linked name is in the proc's variable table (tclsh `info
        // vars` lists it): mirror the current value of the ULTIMATE
        // target when one exists; reads/writes still go through the
        // link.
        let mirror: Option<Value> = match &link {
            UpvarLink::Global(gname) => interp.globals.get(gname).cloned(),
            UpvarLink::Frame { frame_index, var_name } => interp
                .frames
                .get(*frame_index)
                .and_then(|f| f.locals.get(var_name.as_str()))
                .cloned(),
            UpvarLink::Dead { .. } => None,
        };
        if let Some(v) = mirror {
            interp.frames[current_idx].locals.insert(local_var, v);
        } else {
            interp.frames[current_idx].locals.remove(&local_var);
        }
        i += 2;
    }

    Ok(Value::empty())
}

/// Which scope an `upvar` level names at the top of the chain.
enum TargetScope {
    /// Fell off every scope: the caller's namespace context inside a
    /// proc, the true global namespace at eval level.
    Bottom,
    /// The true global namespace (`upvar #0`).
    Global,
    /// A scope-chain index (bottom→top over frames + ns evals).
    Idx(usize),
}

/// Which scope an `upvar` level names.
enum UpvarTarget {
    /// The true global namespace (`upvar #0`, fell off everything).
    GlobalNs,
    /// An open `namespace eval`'s (or fallen-off caller's) variable table.
    Ns(Rc<str>),
    /// A proc frame by absolute index.
    Frame(usize),
}

/// Map a scope-chain index (bottom→top) to its upvar target.
fn upvar_target(interp: &Interp, target: usize) -> UpvarTarget {
    let mut idx = target;
    for (i, f) in interp.frames.iter().enumerate() {
        let below = f.ns_depth.min(interp.ns_stack.len());
        if idx < below {
            return UpvarTarget::Ns(Rc::from(interp.ns_stack[idx].as_str()));
        }
        idx -= below;
        if idx == 0 {
            return UpvarTarget::Frame(i);
        }
        idx -= 1;
    }
    if idx < interp.ns_stack.len() {
        UpvarTarget::Ns(Rc::from(interp.ns_stack[idx].as_str()))
    } else {
        UpvarTarget::GlobalNs
    }
}

pub fn cmd_global(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        // Bare `global` is a no-op (var-6.5, var-6.6).
        return Ok(Value::empty());
    }

    // At global level, global is a no-op
    if interp.frames.is_empty() {
        return Ok(Value::empty());
    }

    let current_idx = interp.frames.len() - 1;

    for arg in &args[1..] {
        let name = arg.as_str();
        // Every `global` name resolves against the TRUE global namespace:
        // `::`-qualified targets normalise exactly like `set` does (colon
        // runs collapse, a trailing run survives — var-6.3's
        // `global ::test_ns_var::test_ns_nested::` links the empty-named
        // variable), plain names are verbatim.
        let target =
            crate::interp::Interp::global_key(name).unwrap_or_else(|| name.to_string());
        // The local alias is the tail after the last `::` run
        // (`global a::b` binds `b` — tclsh), `ns::` binds the empty name.
        let local = super::namespace::split_var_tail(name).to_string();
        // The alias name leaves the slot model (links and slot cells
        // don't alias); a frame whose `global`d name is NOT slotted keeps
        // its fast paths — this is the common hot-proc shape.
        interp.degrade_frame_local(current_idx, &local);
        interp.frames[current_idx].upvars.insert(
            local.clone(),
            UpvarLink::Global(target.clone()),
        );
        // The linked name lives in the proc's variable table (tclsh's
        // `info vars` lists it) — mirror the current value when one
        // exists; reads/writes still go through the link.
        if let Some(v) = interp.globals.get(&target) {
            let v = v.clone();
            interp.frames[current_idx].locals.insert(local, v);
        }
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
    } else if new_name.contains("::") || interp.current_namespace.as_ref() != "::" {
        // Namespace-qualified targets store fully-qualified, exactly like
        // `proc` definition does (basic-18.6: `rename q test_ns_basic::p`
        // must land on the key the ns-first dispatch — and the namespace
        // deletion sweep — looks up: ::test_ns_basic::p).
        super::namespace::qualify(&interp.current_namespace, &new_name)
    } else {
        new_name.clone()
    };

    // Renaming into a namespace creates it, including intermediates
    // (tclsh probed: `rename q deep::nest::p` → `namespace exists
    // deep::nest` = 1).
    if !new_key.is_empty() && new_key.contains("::") {
        let cut = new_key.rfind("::").unwrap();
        let ns = if cut == 0 { "::" } else { &new_key[..cut] };
        super::namespace::ensure_namespace(&mut interp.namespaces, ns);
    }

    // A rename touching a Tier1-named command — either end — invalidates
    // compiled bodies (the old binding's fold may now be shadowed, or a
    // shadow just went away; the bump only costs those bodies the VM).
    super::super::vm_exec::note_tier1_mutation(interp, &old_key);
    if !new_key.is_empty() {
        super::super::vm_exec::note_tier1_mutation(interp, &new_key);
    }
    // The rename moves entries across every resolution table (commands,
    // procs, ensembles, import aliases) — age the resolution cache out.
    interp.note_cmd_mutation();

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
    if interp.current_namespace.as_ref() != "::" && !name.starts_with("::") {
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
    if interp.current_namespace.as_ref() != "::" && !name.starts_with("::") {
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
    } else if interp.current_namespace.as_ref() != "::" {
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
