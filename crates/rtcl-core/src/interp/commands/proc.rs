//! Procedure-related commands: proc, eval, uplevel, upvar, global, rename.

use crate::error::{Error, Result};
use crate::interp::{Interp, ProcDef, UpvarLink};
use crate::value::Value;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

pub fn cmd_proc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // 3-arg form: proc name argList body
    // 4-arg form: proc name argList statics body  (jimtcl-compatible)
    if args.len() < 4 || args.len() > 5 {
        return Err(Error::wrong_args_with_usage(
            "proc", 4, args.len(), "name argList ?statics? body",
        ));
    }

    let raw_name = args[1].as_str();
    // Qualify the proc name if we're inside a namespace context
    let name = if raw_name.starts_with("::") {
        raw_name.to_string()
    } else if interp.current_namespace != "::" {
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

    let mut defaults: Vec<(String, Option<String>)> = Vec::new();
    for param in &params {
        let parts = param.as_list().unwrap_or_else(|| vec![param.clone()]);
        if parts.len() == 2 {
            defaults.push((
                parts[0].as_str().to_string(),
                Some(parts[1].as_str().to_string()),
            ));
        } else {
            defaults.push((parts[0].as_str().to_string(), None));
        }
    }

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
        interp.eval(args[1].as_str())
    } else {
        let script: String = args[1..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        interp.eval(&script)
    }
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
            "can't interpret lambda as a list",
            crate::error::ErrorCode::Generic,
        )
    })?;

    if lambda.len() < 2 {
        return Err(Error::runtime(
            "can't interpret lambda as {params body}: must have exactly 2 elements",
            crate::error::ErrorCode::Generic,
        ));
    }

    let param_list = lambda[0].as_list().unwrap_or_default();
    let body = lambda[1].as_str().to_string();

    // Build param defaults (same logic as cmd_proc)
    let mut defaults: Vec<(String, Option<String>)> = Vec::new();
    for param in &param_list {
        let parts = param.as_list().unwrap_or_else(|| vec![param.clone()]);
        if parts.len() == 2 {
            defaults.push((
                parts[0].as_str().to_string(),
                Some(parts[1].as_str().to_string()),
            ));
        } else {
            defaults.push((parts[0].as_str().to_string(), None));
        }
    }

    let proc_def = ProcDef {
        params: defaults,
        body,
        statics: HashMap::new(),
    };

    // Create args for call_proc: [name, arg1, arg2, ...]
    // We use a synthetic name "apply lambdaExpr"
    let mut call_args = vec![Value::from_str("apply")];
    for arg in &args[2..] {
        call_args.push(arg.clone());
    }

    interp.call_proc(&proc_def, &call_args, "apply")
}

pub fn cmd_uplevel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("uplevel", 2, args.len()));
    }

    let (num_to_pop, script_start) = if args.len() > 2 && args[1].as_str().starts_with('#') {
        // #0 → global level: pop ALL frames
        (interp.frames.len(), 2usize)
    } else if args.len() > 2 {
        match args[1].as_int() {
            Some(n) => {
                let n = (n as usize).min(interp.frames.len());
                (n, 2usize)
            }
            None => (1usize.min(interp.frames.len()), 1usize),
        }
    } else {
        (1usize.min(interp.frames.len()), 1usize)
    };

    let script = if args.len() - script_start == 1 {
        args[script_start].as_str().to_string()
    } else {
        args[script_start..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };

    // Pop the top N frames, eval in the target scope, then restore.
    let split_point = interp.frames.len() - num_to_pop;
    let saved_frames: Vec<_> = interp.frames.split_off(split_point);
    let result = interp.eval(&script);
    interp.frames.extend(saved_frames);
    result
}

pub fn cmd_upvar(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "?level? otherVar localVar ?otherVar localVar ...?"));
    }

    // If not inside a proc, upvar is a no-op
    if interp.frames.is_empty() {
        return Ok(Value::empty());
    }

    let (start, is_global, level) = if args.len() > 3 && args[1].as_str().starts_with('#') {
        (2usize, true, 0usize)
    } else if args.len() > 3 {
        match args[1].as_int() {
            Some(n) => (2usize, false, n as usize),
            None => (1usize, false, 1usize),
        }
    } else {
        (1usize, false, 1usize)
    };

    let current_idx = interp.frames.len() - 1;

    // Determine the target scope
    let target_frame = if is_global || level > current_idx {
        None // Links to globals
    } else {
        Some(current_idx - level)
    };

    // Create upvar links
    let mut i = start;
    while i + 1 < args.len() {
        let other_var = args[i].as_str().to_string();
        let local_var = args[i + 1].as_str().to_string();

        let link = match target_frame {
            None => UpvarLink::Global(other_var),
            Some(fi) => UpvarLink::Frame { frame_index: fi, var_name: other_var },
        };

        interp.frames[current_idx].upvars.insert(local_var, link);
        i += 2;
    }

    Ok(Value::empty())
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

    // Rename in builtins
    if let Some(func) = interp.commands.remove(&old_name) {
        let cat = interp.command_categories.remove(&old_name);
        let meta = interp.command_meta.remove(&old_name);
        if !new_name.is_empty() {
            interp.commands.insert(new_name.clone(), func);
            if let Some(c) = cat {
                interp.command_categories.insert(new_name.clone(), c);
            }
            if let Some(m) = meta {
                interp.command_meta.insert(new_name, m);
            }
        }
        return Ok(Value::empty());
    }

    // Rename in procs
    if let Some(proc_def) = interp.procs.remove(&old_name) {
        if !new_name.is_empty() {
            interp.procs.insert(new_name, proc_def);
        }
        return Ok(Value::empty());
    }

    Err(Error::runtime(
        format!("can't rename: command \"{}\" not found", old_name),
        crate::error::ErrorCode::NotFound,
    ))
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
