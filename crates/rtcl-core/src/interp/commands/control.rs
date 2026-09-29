//! Control flow commands: if, switch, break, continue, return, exit,
//! catch, error, try, tailcall.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

pub fn cmd_if(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: no expression after \"if\" argument",
        ));
    }

    let expr = args[1].as_str();
    let cond = interp.eval_expr(expr)?;

    // Skip optional "then" keyword after the condition
    let mut i = 2;
    if i < args.len() && args[i].as_str() == "then" {
        i += 1;
    }

    if i >= args.len() {
        return Err(Error::wrong_args_msg(format!(
            "wrong # args: no script following \"{}\" argument",
            args[i - 1].as_str()
        )));
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
                    return Err(Error::wrong_args_msg(
                        "wrong # args: no expression after \"elseif\" argument",
                    ));
                }
                let expr = args[i].as_str();
                let cond = interp.eval_expr(expr)?;
                i += 1;
                // Skip optional "then" keyword
                if i < args.len() && args[i].as_str() == "then" {
                    i += 1;
                }
                if i >= args.len() {
                    return Err(Error::wrong_args_msg(format!(
                        "wrong # args: no script following \"{}\" argument",
                        args[i - 1].as_str()
                    )));
                }
                if cond.is_true() {
                    return interp.eval(args[i].as_str());
                }
                i += 1;
            }
            "else" => {
                if i + 1 >= args.len() {
                    return Err(Error::wrong_args_msg(
                        "wrong # args: no script following \"else\" argument",
                    ));
                }
                if i + 1 != args.len() - 1 {
                    return Err(Error::wrong_args_msg(
                        "wrong # args: extra words after \"else\" clause in \"if\" command",
                    ));
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
    const SWITCH_USAGE: &str = "?-option ...? string ?pattern body ...? ?default body?";
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "switch", 3, args.len(),
            SWITCH_USAGE,
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
            opt => {
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": must be -exact, -glob, -indexvar, -matchvar, -nocase, -regexp, or --",
                        opt
                    ),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "switch", 3, args.len(),
            SWITCH_USAGE,
        ));
    }

    let string = args[i].as_str();
    i += 1;

    let patterns: Vec<(String, String)> = if args.len() - i == 1 {
        let list = args[i].as_list().unwrap_or_default();
        if list.is_empty() {
            return Err(Error::wrong_args_msg(
                "wrong # args: should be \"switch ?-option ...? string {?pattern body ...? ?default body?}\"",
            ));
        }
        if !list.len().is_multiple_of(2) {
            return Err(Error::runtime(
                "extra switch pattern with no body",
                crate::error::ErrorCode::InvalidOp,
            ));
        }
        list
            .chunks(2)
            .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
            .collect()
    } else {
        if args.len() == i {
            return Err(Error::wrong_args_with_usage(
                "switch", 3, args.len(),
                SWITCH_USAGE,
            ));
        }
        if !(args.len() - i).is_multiple_of(2) {
            return Err(Error::runtime(
                "extra switch pattern with no body",
                crate::error::ErrorCode::InvalidOp,
            ));
        }
        args[i..]
            .chunks(2)
            .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
            .collect()
    };

    let mut matched = false;
    for (pattern, body) in &patterns {
        if !matched {
            let matches = if pattern == "default" {
                true
            } else {
                match mode {
                    MatchMode::Exact => string == pattern,
                    MatchMode::Glob => super::super::glob_match(pattern, string),
                    MatchMode::Regexp => {
                        #[cfg(feature = "regexp")]
                        {
                            regex::Regex::new(pattern)
                                .map(|re| re.is_match(string))
                                .unwrap_or(false)
                        }
                        #[cfg(not(feature = "regexp"))]
                        {
                            return Err(Error::runtime(
                                "switch -regexp requires 'regexp' feature",
                                crate::error::ErrorCode::InvalidOp,
                            ));
                        }
                    }
                }
            };
            if matches {
                matched = true;
            }
        }
        if matched {
            if body == "-" { continue; }
            return interp.eval(body);
        }
    }

    Ok(Value::empty())
}

pub fn cmd_break(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 2 {
        return Err(Error::wrong_args_with_usage("break", 1, args.len(), ""));
    }
    if args.len() == 2 {
        let n = args[1].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", args[1].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })? as i32;
        if n <= 0 {
            return Err(Error::runtime(
                "bad level: must be > 0",
                crate::error::ErrorCode::Generic,
            ));
        }
        return Err(Error::brk_level(n));
    }
    Err(Error::brk())
}

pub fn cmd_continue(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 2 {
        return Err(Error::wrong_args_with_usage("continue", 1, args.len(), ""));
    }
    if args.len() == 2 {
        let n = args[1].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", args[1].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })? as i32;
        if n <= 0 {
            return Err(Error::runtime(
                "bad level: must be > 0",
                crate::error::ErrorCode::Generic,
            ));
        }
        return Err(Error::cont_level(n));
    }
    Err(Error::cont())
}

pub fn cmd_return(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Parse options: return ?-code code? ?-level level? ?-errorinfo info? ?-errorcode code? ?value?
    let mut code: Option<i32> = None;
    let mut _level: i32 = 1; // default level
    let mut error_info: Option<String> = None;
    let mut error_code: Option<String> = None;
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
        } else if arg == "-errorinfo" {
            i += 1;
            if i >= args.len() {
                return Err(Error::runtime(
                    "missing value for -errorinfo option",
                    crate::error::ErrorCode::Generic,
                ));
            }
            error_info = Some(args[i].as_str().to_string());
            i += 1;
        } else if arg == "-errorcode" {
            i += 1;
            if i >= args.len() {
                return Err(Error::runtime(
                    "missing value for -errorcode option",
                    crate::error::ErrorCode::Generic,
                ));
            }
            error_code = Some(args[i].as_str().to_string());
            i += 1;
        } else {
            break;
        }
    }

    let value = if i < args.len() {
        Some(args[i].clone())
    } else {
        None
    };

    match code {
        Some(c) => Err(Error::return_with_options(c, value, error_info, error_code)),
        None => {
            if error_info.is_some() || error_code.is_some() {
                Err(Error::return_with_options(0, value, error_info, error_code))
            } else {
                Err(Error::ret(value))
            }
        }
    }
}

pub fn cmd_exit(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 2 {
        return Err(Error::wrong_args_with_usage("exit", 1, args.len(), "?returnCode?"));
    }
    let code = if args.len() > 1 {
        args[1].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", args[1].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })? as i32
    } else {
        0
    };
    Err(Error::exit(Some(code)))
}

pub fn cmd_catch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "catch",
            2,
            args.len(),
            "script ?resultVarName? ?optionVarName?",
        ));
    }

    let script = args[1].as_str();
    let result_var = if args.len() > 2 { Some(args[2].as_str()) } else { None };
    let opts_var = if args.len() > 3 { Some(args[3].as_str()) } else { None };

    match interp.eval(script) {
        Ok(v) => {
            if let Some(var) = result_var {
                interp.set_var(var, v)?;
            }
            if let Some(ov) = opts_var {
                interp.set_var(ov, return_options_dict(0, 1))?;
            }
            Ok(Value::from_int(0))
        }
        Err(e) => {
            if let Some(var) = result_var {
                // Tcl: the result variable receives the result payload itself,
                // not a rendering of the error (e.g. `return hi` → "hi",
                // `break` → "").
                interp.set_var(var, Value::from_str(&e.message_text()))?;
            }
            let code = if e.is_return() { 2 }
            else if e.is_break() { 3 }
            else if e.is_continue() { 4 }
            else { 1 };
            if let Some(ov) = opts_var {
                let level = if e.is_return() { 0 } else { 1 };
                let opts = if code == 1 {
                    // Tcl error completions carry -errorcode and -errorinfo.
                    error_options_dict(1, 0, &e, script)
                } else {
                    return_options_dict(code, level)
                };
                interp.set_var(ov, opts)?;
            }
            Ok(Value::from_int(code))
        }
    }
}

/// Minimal Tcl return-options dict: `-code` and `-level` are the keys
/// real scripts consult (e.g. `dict get $opts -code`).
fn return_options_dict(code: i64, level: i64) -> Value {
    Value::from_str(&format!("-code {} -level {}", code, level))
}

/// Tcl return-options dict for an error completion: adds `-errorcode`
/// and `-errorinfo` next to `-code`/`-level`.
fn error_options_dict(code: i64, level: i64, err: &Error, script: &str) -> Value {
    let error_code = err.tcl_error_code();
    let error_info = error_info_for(err, script);
    Value::from_str(&format!(
        "-code {} -level {} -errorcode {} -errorinfo {}",
        code,
        level,
        crate::value::tcl_quote(&error_code),
        crate::value::tcl_quote(&error_info),
    ))
}

/// Build Tcl-style `-errorinfo`: an explicitly supplied `-errorinfo`
/// (from `error msg info` or `return -errorinfo`) is used verbatim;
/// otherwise synthesize the message plus a `while executing` /
/// `invoked from within` line quoting the failing script.
fn error_info_for(err: &Error, script: &str) -> String {
    if let Error::ControlFlow { error_info: Some(info), .. } = err {
        if !info.is_empty() {
            return info.clone();
        }
    }
    let how = if matches!(err, Error::DivisionByZero) {
        "invoked from within"
    } else {
        "while executing"
    };
    format!("{}\n    {}\n\"{}\"", err.message_text(), how, script)
}

pub fn cmd_error(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "error",
            2,
            args.len(),
            "message ?errorInfo? ?errorCode?",
        ));
    }
    let msg = args[1].as_str().to_string();
    let error_info = args.get(2).map(|v| v.as_str().to_string());
    let error_code = args.get(3).map(|v| v.as_str().to_string());
    if error_info.is_none() && error_code.is_none() {
        return Err(Error::Msg(msg));
    }
    Err(Error::ControlFlow {
        kind: crate::error::ControlFlow::Error,
        value: Some(Value::from_str(&msg)),
        level: 1,
        error_info,
        error_code,
    })
}

/// try body ?on code varList script? ... ?finally script?
pub fn cmd_try(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "try",
            2,
            args.len(),
            "body ?handler ...? ?finally script?",
        ));
    }

    // Execute the body
    let body = args[1].as_str();
    let body_result = interp.eval(body);

    // Determine the exit code and result value
    let (exit_code, result_value) = match &body_result {
        Ok(v) => (0i32, v.as_str().to_string()),
        Err(e) => (e.return_code(), e.message_text()),
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
                    return Err(Error::wrong_args_msg(
                        "wrong # args to on clause: must be \"... on code variableList script\"",
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
                            let opts = match &body_result {
                                Err(e) if exit_code == 1 => {
                                    error_options_dict(exit_code as i64, 0, e, body)
                                }
                                _ => return_options_dict(exit_code as i64, 0),
                            };
                            interp.set_var(opts_var, opts)?;
                        }
                    }
                    handler_result = Some(interp.eval(handler_body));
                }
                i += 4;
            }
            "finally" => {
                if i + 1 >= args.len() {
                    return Err(Error::wrong_args_msg(
                        "wrong # args to finally clause: must be \"... finally script\"",
                    ));
                }
                finally_script = Some(args[i + 1].as_str());
                i += 2;
            }
            _ => {
                return Err(Error::runtime(
                    format!("bad handler type \"{}\": must be finally, on, or trap", keyword),
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
