//! Procedure-related commands: proc, eval, uplevel, upvar, global, rename.

use crate::error::{Error, Result};
use crate::interp::{Interp, ProcDef};
use crate::value::Value;

pub fn cmd_proc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 4 {
        return Err(Error::wrong_args_with_usage("proc", 4, args.len(), "name argList body"));
    }

    let name = args[1].as_str().to_string();
    let params = args[2].as_list().unwrap_or_default();
    let body = args[3].as_str().to_string();

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

    let proc_def = ProcDef {
        params: defaults,
        body,
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
    };

    // Create args for call_proc: [name, arg1, arg2, ...]
    // We use a synthetic name "apply lambdaExpr"
    let mut call_args = vec![Value::from_str("apply")];
    for arg in &args[2..] {
        call_args.push(arg.clone());
    }

    interp.call_proc(&proc_def, &call_args)
}

pub fn cmd_uplevel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("uplevel", 2, args.len()));
    }

    let (level, script_start) = if args.len() > 2 && args[1].as_str().starts_with('#') {
        (0usize, 2usize)
    } else if args.len() > 2 {
        match args[1].as_int() {
            Some(n) => (n as usize, 2usize),
            None => (1usize, 1usize),
        }
    } else {
        (1usize, 1usize)
    };

    let _ = level; // TODO: implement proper call-frame uplevel

    if args.len() - script_start == 1 {
        interp.eval(args[script_start].as_str())
    } else {
        let script: String = args[script_start..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        interp.eval(&script)
    }
}

pub fn cmd_upvar(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "?level? otherVar localVar ?otherVar localVar ...?"));
    }

    let (start, _level) = if args.len() > 3 && args[1].as_str().starts_with('#') {
        (2usize, 0usize)
    } else if args.len() > 3 {
        match args[1].as_int() {
            Some(n) => (2usize, n as usize),
            None => (1usize, 1usize),
        }
    } else {
        (1usize, 1usize)
    };

    // Simple implementation: link variables by copying
    let mut i = start;
    while i + 1 < args.len() {
        let other_var = args[i].as_str();
        let local_var = args[i + 1].as_str();
        if let Ok(val) = interp.get_var(other_var).cloned() {
            interp.set_var(local_var, val)?;
        }
        i += 2;
    }

    Ok(Value::empty())
}

pub fn cmd_global(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("global", 2, args.len()));
    }

    // Flat variable model — all variables are global already, so this is a no-op
    // except for ensuring variables exist.
    for arg in &args[1..] {
        let name = arg.as_str();
        if interp.get_var(name).is_err() {
            interp.set_var(name, Value::empty())?;
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

    // Rename in builtins
    if let Some(func) = interp.commands.remove(&old_name) {
        if !new_name.is_empty() {
            interp.commands.insert(new_name, func);
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
