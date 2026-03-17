//! Control flow commands: if, while, for, foreach, switch, break, continue,
//! return, exit, catch, error.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

pub fn cmd_if(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args("if", 3, args.len()));
    }

    let expr = args[1].as_str();
    let cond = interp.eval_expr(expr)?;

    // Skip optional "then" keyword after the condition
    let mut i = 2;
    if i < args.len() && args[i].as_str() == "then" {
        i += 1;
    }

    if i >= args.len() {
        return Err(Error::wrong_args("if", i + 1, args.len()));
    }

    if cond.is_true() {
        return interp.eval(args[i].as_str());
    }
    i += 1;

    while i < args.len() {
        let word = args[i].as_str();
        match word {
            "elseif" => {
                i += 1;
                if i >= args.len() {
                    return Err(Error::wrong_args("elseif", 2, args.len() - i));
                }
                let expr = args[i].as_str();
                let cond = interp.eval_expr(expr)?;
                i += 1;
                // Skip optional "then" keyword
                if i < args.len() && args[i].as_str() == "then" {
                    i += 1;
                }
                if i >= args.len() {
                    return Err(Error::wrong_args("elseif", 3, 0));
                }
                if cond.is_true() {
                    return interp.eval(args[i].as_str());
                }
                i += 1;
            }
            "else" => {
                if i + 1 >= args.len() {
                    return Err(Error::wrong_args("else", 1, args.len() - i));
                }
                return interp.eval(args[i + 1].as_str());
            }
            _ => {
                return interp.eval(word);
            }
        }
    }

    Ok(Value::empty())
}

pub fn cmd_while(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage("while", 3, args.len(), "test body"));
    }

    let test = args[1].as_str();
    let body = args[2].as_str();
    let mut result = Value::empty();

    loop {
        let cond = interp.eval_expr(test)?;
        if !cond.is_true() {
            break;
        }
        match interp.eval(body) {
            Ok(v) => result = v,
            Err(e) => {
                if e.is_break() { break; }
                if e.is_continue() { continue; }
                return Err(e);
            }
        }
    }

    Ok(result)
}

pub fn cmd_for(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 5 {
        return Err(Error::wrong_args_with_usage("for", 5, args.len(), "start test next body"));
    }

    let start = args[1].as_str();
    let test = args[2].as_str();
    let next = args[3].as_str();
    let body = args[4].as_str();

    interp.eval(start)?;

    let mut result = Value::empty();

    loop {
        let cond = interp.eval_expr(test)?;
        if !cond.is_true() { break; }

        match interp.eval(body) {
            Ok(v) => result = v,
            Err(e) => {
                if e.is_break() { break; }
                if e.is_continue() { /* fall through to next */ }
                else { return Err(e); }
            }
        }

        match interp.eval(next) {
            Ok(_) => {}
            Err(e) => {
                if e.is_break() { break; }
                if e.is_continue() { }
                else { return Err(e); }
            }
        }
    }

    Ok(result)
}

pub fn cmd_foreach(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 || args.len() % 2 != 0 {
        return Err(Error::wrong_args_with_usage(
            "foreach", 4, args.len(),
            "varList list ?varList list ...? body",
        ));
    }

    let body = args[args.len() - 1].as_str();
    let mut result = Value::empty();

    // Collect (var_names, data_list) pairs
    // var_names is a list: single var "x" or multi-var "{a b c}"
    struct VarGroup {
        vars: Vec<String>,
        data: Vec<Value>,
    }
    let mut groups: Vec<VarGroup> = Vec::new();
    let mut i = 1;
    while i < args.len() - 1 {
        let var_list = args[i].as_list().unwrap_or_else(|| vec![args[i].clone()]);
        let vars: Vec<String> = var_list.iter().map(|v| v.as_str().to_string()).collect();
        let data = args[i + 1].as_list().unwrap_or_default();
        groups.push(VarGroup { vars, data });
        i += 2;
    }

    // Compute max iterations: for each group, ceil(data.len() / vars.len())
    let max_iters = groups.iter()
        .map(|g| {
            let n = g.vars.len().max(1);
            (g.data.len() + n - 1) / n
        })
        .max()
        .unwrap_or(0);

    for idx in 0..max_iters {
        for g in &groups {
            let n = g.vars.len();
            for (vi, var) in g.vars.iter().enumerate() {
                let data_idx = idx * n + vi;
                let value = g.data.get(data_idx).cloned().unwrap_or_else(Value::empty);
                interp.set_var(var, value)?;
            }
        }
        match interp.eval(body) {
            Ok(v) => result = v,
            Err(e) => {
                if e.is_break() { break; }
                if e.is_continue() { continue; }
                return Err(e);
            }
        }
    }

    Ok(result)
}

pub fn cmd_switch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "switch", 3, args.len(),
            "?options? string pattern body ?pattern body ...?",
        ));
    }

    let mut i = 1;
    let mut exact_match = false;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-exact" => { exact_match = true; i += 1; }
            "-glob" => { exact_match = false; i += 1; }
            "-regexp" => {
                return Err(Error::runtime(
                    "regexp mode not supported",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args("switch", 3, args.len()));
    }

    let string = args[i].as_str();
    i += 1;

    let patterns: Vec<(String, String)>;
    if args.len() - i == 1 {
        let list = args[i].as_list().unwrap_or_default();
        if list.len() % 2 != 0 {
            return Err(Error::runtime(
                "switch list must have even number of elements",
                crate::error::ErrorCode::InvalidOp,
            ));
        }
        patterns = list
            .chunks(2)
            .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
            .collect();
    } else {
        if (args.len() - i) % 2 != 0 {
            return Err(Error::runtime(
                "switch must have even number of pattern/body pairs",
                crate::error::ErrorCode::InvalidOp,
            ));
        }
        patterns = args[i..]
            .chunks(2)
            .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
            .collect();
    }

    for (pattern, body) in &patterns {
        let matches = if pattern == "default" {
            true
        } else if exact_match {
            string == pattern
        } else {
            super::super::glob_match(pattern, string)
        };
        if matches {
            if body == "-" { continue; }
            return interp.eval(body);
        }
    }

    Ok(Value::empty())
}

pub fn cmd_break(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::brk())
}

pub fn cmd_continue(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::cont())
}

pub fn cmd_return(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Parse options: return ?-code code? ?-level level? ?value?
    let mut code: Option<i32> = None;
    let mut _level: i32 = 1; // default level
    let mut i = 1;

    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "-code" {
            i += 1;
            if i >= args.len() {
                return Err(Error::runtime(
                    "missing value for -code option",
                    crate::error::ErrorCode::Generic,
                ));
            }
            let code_arg = args[i].as_str();
            code = Some(match code_arg {
                "ok" => 0,
                "error" => 1,
                "return" => 2,
                "break" => 3,
                "continue" => 4,
                _ => code_arg.parse::<i32>().map_err(|_| {
                    Error::runtime(
                        format!("bad completion code \"{}\": must be ok, error, return, break, continue, or an integer", code_arg),
                        crate::error::ErrorCode::Generic,
                    )
                })?,
            });
            i += 1;
        } else if arg == "-level" {
            i += 1;
            if i >= args.len() {
                return Err(Error::runtime(
                    "missing value for -level option",
                    crate::error::ErrorCode::Generic,
                ));
            }
            _level = args[i].as_str().parse::<i32>().map_err(|_| {
                Error::runtime(
                    format!("bad -level value \"{}\"", args[i].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            i += 1;
        } else {
            break;
        }
    }

    let value = if i < args.len() {
        Some(args[i].as_str().to_string())
    } else {
        None
    };

    match code {
        Some(c) => Err(Error::return_with_code(c, value)),
        None => Err(Error::ret(value)),
    }
}

pub fn cmd_exit(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let code = if args.len() > 1 { args[1].as_int().unwrap_or(0) as i32 } else { 0 };
    Err(Error::exit(Some(code)))
}

pub fn cmd_catch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("catch", 2, args.len()));
    }

    let script = args[1].as_str();
    let result_var = if args.len() > 2 { Some(args[2].as_str()) } else { None };

    match interp.eval(script) {
        Ok(v) => {
            if let Some(var) = result_var {
                interp.set_var(var, v)?;
            }
            Ok(Value::from_int(0))
        }
        Err(e) => {
            if let Some(var) = result_var {
                interp.set_var(var, Value::from_str(&e.to_string()))?;
            }
            let code = if e.is_return() { 2 }
            else if e.is_break() { 3 }
            else if e.is_continue() { 4 }
            else { 1 };
            Ok(Value::from_int(code))
        }
    }
}

pub fn cmd_error(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("error", 2, args.len()));
    }
    Err(Error::Msg(args[1].as_str().to_string()))
}

/// try body ?on code varList script? ... ?finally script?
pub fn cmd_try(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "try",
            2,
            args.len(),
            "body ?on code varList script? ... ?finally script?",
        ));
    }

    // Execute the body
    let body = args[1].as_str();
    let body_result = interp.eval(body);

    // Determine the exit code and result value
    let (exit_code, result_value) = match &body_result {
        Ok(v) => (0i32, v.as_str().to_string()),
        Err(e) => {
            let code = e.return_code();
            let msg = match e {
                Error::ControlFlow { value: Some(v), .. } => v.clone(),
                Error::ControlFlow { value: None, .. } => String::new(),
                _ => e.to_string(),
            };
            (code, msg)
        }
    };

    // Parse on/finally handlers
    let mut i = 2;
    let mut handler_result: Option<Result<Value>> = None;
    let mut finally_script: Option<&str> = None;

    while i < args.len() {
        let keyword = args[i].as_str();
        match keyword {
            "on" => {
                // on code varList script
                if i + 3 >= args.len() {
                    return Err(Error::wrong_args_with_usage(
                        "try",
                        4,
                        args.len() - i,
                        "on code varList script",
                    ));
                }
                let code_spec = args[i + 1].as_str();
                let var_list = args[i + 2].as_str();
                let handler_body = args[i + 3].as_str();

                // Parse the code spec
                let match_code = match code_spec {
                    "ok" => 0,
                    "error" => 1,
                    "return" => 2,
                    "break" => 3,
                    "continue" => 4,
                    "*" => -1, // match any
                    s => s.parse::<i32>().unwrap_or(-2),
                };

                // Check if this handler matches (only use first match)
                if handler_result.is_none() && (match_code == -1 || match_code == exit_code) {
                    // Set variables from varList
                    let vars: Vec<&str> = var_list.split_whitespace().collect();
                    if let Some(msg_var) = vars.first() {
                        if !msg_var.is_empty() {
                            interp.set_var(msg_var, Value::from_str(&result_value))?;
                        }
                    }
                    if let Some(opts_var) = vars.get(1) {
                        if !opts_var.is_empty() {
                            // Build opts dict: -code N -level 0
                            let opts = format!("-code {} -level 0", exit_code);
                            interp.set_var(opts_var, Value::from_str(&opts))?;
                        }
                    }
                    handler_result = Some(interp.eval(handler_body));
                }
                i += 4;
            }
            "finally" => {
                if i + 1 >= args.len() {
                    return Err(Error::wrong_args_with_usage(
                        "try",
                        2,
                        args.len() - i,
                        "finally script",
                    ));
                }
                finally_script = Some(args[i + 1].as_str());
                i += 2;
            }
            _ => {
                return Err(Error::runtime(
                    format!("bad handler \"{}\": must be on, trap, or finally", keyword),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
    }

    // Execute finally script if present
    let finally_result = if let Some(script) = finally_script {
        let r = interp.eval(script);
        if r.is_err() {
            // Finally error replaces the original error
            return r;
        }
        Some(r)
    } else {
        None
    };

    // Determine the final result
    if let Some(hr) = handler_result {
        // Handler was executed — its result is the return value
        hr
    } else if finally_result.is_some() {
        // No handler matched, finally ran OK — re-raise original error/result
        body_result
    } else {
        // No handler, no finally — re-raise original
        body_result
    }
}

/// `tailcall command ?arg ...?`
/// Replaces the current proc invocation with a call to the given command.
/// Simplified implementation: evaluates and returns via return control flow.
pub fn cmd_tailcall(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "tailcall",
            2,
            args.len(),
            "command ?arg ...?",
        ));
    }

    // Build the command string
    let cmd_parts: Vec<String> = args[1..]
        .iter()
        .map(|a| {
            let s = a.as_str();
            if s.is_empty() || s.contains(' ') || s.contains('\t') || s.contains('\n') {
                format!("{{{}}}", s)
            } else {
                s.to_string()
            }
        })
        .collect();
    let script = cmd_parts.join(" ");
    let result = interp.eval(&script)?;
    Err(Error::ret(Some(result.as_str().to_string())))
}

/// `time script ?count?`
/// Time the execution of a script, returns "N microseconds per iteration".
#[cfg(feature = "std")]
pub fn cmd_time(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "time",
            2,
            args.len(),
            "script ?count?",
        ));
    }

    let script = args[1].as_str();
    let count: u64 = if args.len() == 3 {
        args[2].as_int().unwrap_or(1) as u64
    } else {
        1
    };

    let start = std::time::Instant::now();
    for _ in 0..count {
        let _ = interp.eval(script)?;
    }
    let elapsed = start.elapsed();
    let us_per_iter = if count > 0 {
        elapsed.as_micros() as f64 / count as f64
    } else {
        0.0
    };
    Ok(Value::from_str(&format!(
        "{} microseconds per iteration",
        us_per_iter as u64
    )))
}

#[cfg(not(feature = "std"))]
pub fn cmd_time(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime("time requires std feature", crate::error::ErrorCode::Generic))
}

/// `timerate script ?duration? ?maxcount?`
/// Calibrated timing: runs script repeatedly for at least `duration` ms.
#[cfg(feature = "std")]
pub fn cmd_timerate(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "timerate",
            2,
            args.len(),
            "script ?duration? ?maxcount?",
        ));
    }

    let script = args[1].as_str();
    let duration_ms: u64 = if args.len() >= 3 {
        args[2].as_int().unwrap_or(1000) as u64
    } else {
        1000
    };
    let max_count: u64 = if args.len() >= 4 {
        args[3].as_int().unwrap_or(u64::MAX as i64) as u64
    } else {
        u64::MAX
    };

    let deadline = std::time::Duration::from_millis(duration_ms);
    let start = std::time::Instant::now();
    let mut count: u64 = 0;

    while start.elapsed() < deadline && count < max_count {
        let _ = interp.eval(script)?;
        count += 1;
    }

    let elapsed = start.elapsed();
    let us_per_iter = if count > 0 {
        elapsed.as_micros() as f64 / count as f64
    } else {
        0.0
    };

    Ok(Value::from_str(&format!(
        "{} microseconds per iteration",
        us_per_iter as u64
    )))
}

#[cfg(not(feature = "std"))]
pub fn cmd_timerate(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime("timerate requires std feature", crate::error::ErrorCode::Generic))
}

/// `range ?start? end ?step?`
/// Generate a list of integers. jimtcl extension — like Python's range().
pub fn cmd_range(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (start, end, step) = match args.len() {
        2 => {
            // range end
            let end = args[1].as_int().ok_or_else(|| {
                Error::runtime("expected integer", crate::error::ErrorCode::Generic)
            })?;
            (0i64, end, 1i64)
        }
        3 => {
            // range start end
            let start = args[1].as_int().ok_or_else(|| {
                Error::runtime("expected integer", crate::error::ErrorCode::Generic)
            })?;
            let end = args[2].as_int().ok_or_else(|| {
                Error::runtime("expected integer", crate::error::ErrorCode::Generic)
            })?;
            let step = if end >= start { 1 } else { -1 };
            (start, end, step)
        }
        4 => {
            // range start end step
            let start = args[1].as_int().ok_or_else(|| {
                Error::runtime("expected integer", crate::error::ErrorCode::Generic)
            })?;
            let end = args[2].as_int().ok_or_else(|| {
                Error::runtime("expected integer", crate::error::ErrorCode::Generic)
            })?;
            let step = args[3].as_int().ok_or_else(|| {
                Error::runtime("expected integer", crate::error::ErrorCode::Generic)
            })?;
            if step == 0 {
                return Err(Error::runtime(
                    "step cannot be zero",
                    crate::error::ErrorCode::Generic,
                ));
            }
            (start, end, step)
        }
        _ => {
            return Err(Error::wrong_args_with_usage(
                "range",
                2,
                args.len(),
                "?start? end ?step?",
            ));
        }
    };

    let mut result = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < end {
            result.push(Value::from_int(i));
            i += step;
        }
    } else {
        while i > end {
            result.push(Value::from_int(i));
            i += step;
        }
    }
    Ok(Value::from_list(&result))
}
