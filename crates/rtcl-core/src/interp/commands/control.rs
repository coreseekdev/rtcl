//! Control flow commands: if, switch, break, continue, return, exit,
//! catch, error, try, tailcall.

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

pub fn cmd_switch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "switch", 3, args.len(),
            "?options? string pattern body ?pattern body ...?",
        ));
    }

    let mut i = 1;
    #[derive(PartialEq)]
    enum MatchMode { Exact, Glob, Regexp }
    let mut mode = MatchMode::Glob;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-exact" => { mode = MatchMode::Exact; i += 1; }
            "-glob" => { mode = MatchMode::Glob; i += 1; }
            "-regexp" => { mode = MatchMode::Regexp; i += 1; }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args("switch", 3, args.len()));
    }

    let string = args[i].as_str();
    i += 1;

    let patterns: Vec<(String, String)> = if args.len() - i == 1 {
        let list = args[i].as_list().unwrap_or_default();
        if !list.len().is_multiple_of(2) {
            return Err(Error::runtime(
                "switch list must have even number of elements",
                crate::error::ErrorCode::InvalidOp,
            ));
        }
        list
            .chunks(2)
            .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
            .collect()
    } else {
        if !(args.len() - i).is_multiple_of(2) {
            return Err(Error::runtime(
                "switch must have even number of pattern/body pairs",
                crate::error::ErrorCode::InvalidOp,
            ));
        }
        args[i..]
            .chunks(2)
            .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
            .collect()
    };

    for (pattern, body) in &patterns {
        let matches = if pattern == "default" {
            true
        } else {
            match mode {
                MatchMode::Exact => string == pattern,
                MatchMode::Glob => super::super::glob_match(pattern, string),
                MatchMode::Regexp => {
                    #[cfg(feature = "std")]
                    {
                        regex::Regex::new(pattern)
                            .map(|re| re.is_match(string))
                            .unwrap_or(false)
                    }
                    #[cfg(not(feature = "std"))]
                    {
                        return Err(Error::runtime(
                            "switch -regexp requires std feature",
                            crate::error::ErrorCode::InvalidOp,
                        ));
                    }
                }
            }
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
    if let Some(script) = finally_script {
        interp.eval(script)?;
    }

    // Determine the final result
    if let Some(hr) = handler_result {
        // Handler was executed — its result is the return value
        hr
    } else {
        // No handler matched — re-raise original error/result
        body_result
    }
}

/// `tailcall command ?arg ...?`
/// Replaces the current proc invocation with a call to the given command.
/// Simplified implementation: evaluates and returns via return control flow.
pub fn cmd_tailcall(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "tailcall",
            2,
            args.len(),
            "command ?arg ...?",
        ));
    }

    // Signal a tail-call: collect arg strings for TCO re-dispatch in call_proc
    let tc_args: Vec<String> = args[1..]
        .iter()
        .map(|a| a.as_str().to_string())
        .collect();
    Err(Error::tail_call(tc_args))
}
