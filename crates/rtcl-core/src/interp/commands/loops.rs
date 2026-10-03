//! Iteration and timing commands: while, for, foreach, time, timerate, range.

use std::borrow::Cow;

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

use super::dict::demote_level0_return;
use super::list::{set_error_code, strict_list, strict_list_cow, tcl_err};

pub fn cmd_while(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage("while", 3, args.len(), "test command"));
    }

    let test = args[1].as_str();
    let body = &args[2];

    // tclsh compiles the body inline against one absolute line table, so a
    // body command's `(procedure ... line N)` reports its line in the
    // enclosing script, not within the body text (proc-old-5.16: the error
    // sits on the line after `while {…} {`).  When the body word is a
    // verbatim braced/quoted literal its lines are relative to the while
    // command's own line: rebase the eval offset for the body's duration.
    // A body that came from a variable keeps the unshifted numbering (its
    // origin is unknowable; tclsh in that case also numbers from 1).
    let saved_offset = interp.line_offset;
    let body_offset = match interp.body_is_verbatim_script(2, body.as_str()) {
        Some(extra) => Some(saved_offset + interp.cur_cmd_line.max(1) - 1 + extra),
        None => None,
    };

    loop {
        let cond = interp.eval_expr(test)?;
        if !crate::types::expr_funcs::strict_bool(&cond)? {
            break;
        }
        if let Some(off) = body_offset {
            interp.line_offset = off;
        }
        let r = demote_level0_return(interp.eval_body_value(body));
        interp.line_offset = saved_offset;
        match r {
            Ok(_) => {}
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    continue;
                }
                // tclsh compiles while inline: a body error names no frame
                // for the while command itself (N8).
                interp.err_fresh = true;
                return Err(e);
            }
        }
    }

    // Tcl: loop commands always return the empty string
    Ok(Value::empty())
}

pub fn cmd_for(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 5 {
        return Err(Error::wrong_args_with_usage("for", 5, args.len(), "start test next command"));
    }

    let start = args[1].as_str();
    let test = args[2].as_str();
    let next = &args[3];
    let body = &args[4];

    demote_level0_return(interp.eval(start))?;

    loop {
        let cond = interp.eval_expr(test)?;
        if !crate::types::expr_funcs::strict_bool(&cond)? { break; }

        match demote_level0_return(interp.eval_body_value(body)) {
            Ok(_) => {}
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    /* fall through to next */
                }
                else {
                    // tclsh compiles for inline like while: frameless.
                    interp.err_fresh = true;
                    return Err(e);
                }
            }
        }

        match demote_level0_return(interp.eval_body_value(next)) {
            Ok(_) => {}
            Err(e) => {
                if e.is_break() {
                    // `break` in the next script ends the loop (for-8.1).
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    // Unlike a body continue, a `continue` raised by the
                    // next script escapes the `for` itself: tclsh's
                    // compiled next script has no in-loop continue target
                    // (for-8.2..for-8.12 — the enclosing loop sees the
                    // continue).
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    return Err(e);
                }
                else {
                    interp.err_fresh = true;
                    return Err(e);
                }
            }
        }
    }

    // Tcl: loop commands always return the empty string
    Ok(Value::empty())
}

pub fn cmd_foreach(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 || !args.len().is_multiple_of(2) {
        return Err(Error::wrong_args_with_usage(
            "foreach", 4, args.len(),
            "varList list ?varList list ...? command",
        ));
    }

    let body = &args[args.len() - 1];

    // Collect (var_names, data_list) pairs
    // var_names is a list: single var "x" or multi-var "{a b c}"
    struct VarGroup<'a> {
        vars: Vec<String>,
        data: Cow<'a, [Value]>,
    }
    let mut groups: Vec<VarGroup> = Vec::new();
    let mut i = 1;
    while i < args.len() - 1 {
        // tclsh parses each varlist/list strictly — malformed elements
        // raise the list scanner's error (foreach-1.12/1.13).
        let var_list = strict_list(interp, &args[i])?;
        let vars: Vec<String> = var_list.iter().map(|v| v.as_str().to_string()).collect();
        // tclsh: an empty varlist is an error, not zero iterations.
        if vars.is_empty() {
            set_error_code(interp, "TCL OPERATION FOREACH NEEDVARS");
            return Err(tcl_err("foreach varlist is empty"));
        }
        // A value already carrying a list rep is viewed in place — the
        // iteration never mutates the source (tclsh hands the foreach
        // body pointers into the source list the same way).
        let data = strict_list_cow(interp, &args[i + 1])?;
        groups.push(VarGroup { vars, data });
        i += 2;
    }

    // Compute max iterations: for each group, ceil(data.len() / vars.len())
    let max_iters = groups.iter()
        .map(|g| {
            let n = g.vars.len().max(1);
            g.data.len().div_ceil(n)
        })
        .max()
        .unwrap_or(0);

    for idx in 0..max_iters {
        for g in &groups {
            let n = g.vars.len();
            for (vi, var) in g.vars.iter().enumerate() {
                let data_idx = idx * n + vi;
                let value = g.data.get(data_idx).cloned().unwrap_or_else(Value::empty);
                if let Err(e) = interp.set_var(var, value) {
                    // Tcl's compiled foreach appends a dedicated frame
                    // between the message and the enclosing command's
                    // frame when a loop-variable write fails
                    // (foreach-1.14: `(setting foreach loop variable "a")`),
                    // and installs TCL WRITE VARNAME as ::errorCode.
                    if interp.err_is_error(&e) {
                        set_error_code(interp, "TCL WRITE VARNAME");
                        if interp.err_info.is_none() {
                            interp.err_info = Some(e.message_text());
                        }
                        if let Some(info) = &mut interp.err_info {
                            info.push_str(&format!(
                                "\n    (setting foreach loop variable \"{}\")",
                                var
                            ));
                        }
                    }
                    return Err(e);
                }
            }
        }
        match demote_level0_return(interp.eval_body_value(body)) {
            Ok(_) => {}
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    continue;
                }
                // Unlike compiled while/for, foreach keeps its own command
                // frame and adds `("foreach" body line N)` (F1).
                interp.err_exit_frame("\"foreach\" body");
                return Err(e);
            }
        }
    }

    // Tcl: loop commands always return the empty string
    Ok(Value::empty())
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
            "command ?count?",
        ));
    }

    let script = &args[1];
    let count: u64 = if args.len() == 3 {
        args[2].as_int().unwrap_or(1) as u64
    } else {
        1
    };

    let start = std::time::Instant::now();
    for _ in 0..count {
        let _ = interp.eval_body_value(script)?;
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

    let script = &args[1];
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
        let _ = interp.eval_body_value(script)?;
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

/// `loop var ?first? limit ?incr? body` — Numeric for-loop (jimtcl extension).
///
/// Forms:
///   loop var limit body          — var goes from 0 to limit-1, step 1
///   loop var first limit body    — var goes from first to limit-1, step 1
///   loop var first limit incr body — var goes from first towards limit, step incr
pub fn cmd_loop(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (var, mut i, limit, step) = match args.len() {
        // loop var limit body
        4 => {
            let var = args[1].as_str().to_string();
            let limit = args[2].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[2].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            (var, 0i64, limit, 1i64)
        }
        // loop var first limit body
        5 => {
            let var = args[1].as_str().to_string();
            let first = args[2].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[2].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            let limit = args[3].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[3].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            (var, first, limit, 1i64)
        }
        // loop var first limit incr body
        6 => {
            let var = args[1].as_str().to_string();
            let first = args[2].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[2].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            let limit = args[3].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[3].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            let step = args[4].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[4].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            if step == 0 {
                return Err(Error::runtime(
                    "step cannot be zero",
                    crate::error::ErrorCode::Generic,
                ));
            }
            (var, first, limit, step)
        }
        _ => {
            return Err(Error::wrong_args_with_usage(
                "loop",
                4,
                args.len(),
                "var ?first? limit ?incr? body",
            ));
        }
    };
    let body = &args[args.len() - 1];

    let mut result = Value::empty();

    loop {
        let done = if step > 0 { i >= limit } else { i <= limit };
        if done {
            break;
        }
        interp.set_var(&var, Value::from_int(i))?;
        match interp.eval_body_value(body) {
            Ok(v) => result = v,
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    i += step;
                    continue;
                }
                return Err(e);
            }
        }
        i += step;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_loop_3arg() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 5 { lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "0 1 2 3 4");
    }

    #[test]
    fn test_loop_4arg() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 2 5 { lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "2 3 4");
    }

    #[test]
    fn test_loop_5arg() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 0 10 3 { lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "0 3 6 9");
    }

    #[test]
    fn test_loop_negative_step() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 5 0 -2 { lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "5 3 1");
    }

    #[test]
    fn test_loop_zero_step_error() {
        let mut interp = Interp::new();
        assert!(interp.eval("loop i 0 10 0 { }").is_err());
    }

    #[test]
    fn test_loop_break() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 10 { if {$i == 3} break; lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "0 1 2");
    }

    #[test]
    fn test_loop_continue() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 5 { if {$i == 2} continue; lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "0 1 3 4");
    }

    #[test]
    fn test_loop_empty_body() {
        let mut interp = Interp::new();
        // Should complete without error
        interp.eval("loop i 0 { }").unwrap();
    }

    #[test]
    fn test_loop_wrong_args() {
        let mut interp = Interp::new();
        assert!(interp.eval("loop i").is_err());
        assert!(interp.eval("loop").is_err());
    }

    // -- Tcl 8.6 break/continue semantics --

    #[test]
    fn test_break_no_args() {
        // `break` takes no arguments in Tcl 8.6 (for-3.1).
        let mut interp = Interp::new();
        let e = interp.eval("break foo").unwrap_err();
        assert_eq!(e.to_string(), "wrong # args: should be \"break\"");
        assert!(interp.eval("break 2").is_err());
    }

    #[test]
    fn test_continue_no_args() {
        // `continue` takes no arguments in Tcl 8.6 (for-2.1).
        let mut interp = Interp::new();
        let e = interp.eval("continue foo").unwrap_err();
        assert_eq!(e.to_string(), "wrong # args: should be \"continue\"");
        assert!(interp.eval("continue 2").is_err());
    }

    #[test]
    fn test_break_in_next_script_ends_loop() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            set log {}
            for {set i 0} {$i < 5} {incr i; break} { lappend log $i }
            list [llength $log] $i
        "#).unwrap();
        // one body run (i=0), then incr i → 1 and the loop ends
        assert_eq!(r.as_str(), "1 1");
    }

    #[test]
    fn test_continue_in_next_script_escapes_for() {
        // Tcl's compiled `for` gives the next script no in-loop continue
        // target: the continue escapes to the enclosing loop (for-8.12).
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            apply {{} {
                for {set k 0} {$k < 3} {incr k} {
                    set j 0
                    for {set i 0} {$i < 5} {incr i;continue} {
                        incr j
                    }
                    incr i
                }
                list $i $j $k
            }}
        "#).unwrap();
        assert_eq!(r.as_str(), "1 1 3");
    }

    #[test]
    fn test_return_level0_in_for_body_is_normal() {
        // `return -level 0` completes the body script normally: the loop
        // keeps iterating (tclsh loops forever on `while {1} {return -level 0}`).
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            proc p {} { for {set i 0} {$i<3} {incr i} { return -level 0 $i } ; list after $i }
            p
        "#).unwrap();
        assert_eq!(r.as_str(), "after 3");
    }

    #[test]
    fn test_foreach_malformed_list_error() {
        // Strict list parsing on both the varlist and the data list
        // (foreach-1.12/1.13).
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            catch {foreach a {{1 2}3} {}} m
            list $m $::errorCode
        "#).unwrap();
        assert_eq!(
            r.as_str(),
            "{list element in braces followed by \"3\" instead of space} {TCL VALUE LIST JUNK}"
        );
    }

    #[test]
    fn test_foreach_set_array_var_error_info() {
        // Failing to write a loop variable frames the write site
        // (foreach-1.14).
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            unset -nocomplain a
            set a(0) 44
            catch {foreach a {1 2 3} {}} m
            list $m $::errorCode $::errorInfo
        "#).unwrap();
        assert_eq!(
            r.as_str(),
            "{can't set \"a\": variable is array} {TCL WRITE VARNAME} {can't set \"a\": variable is array\n    (setting foreach loop variable \"a\")\n    invoked from within\n\"foreach a {1 2 3} {}\"}"
        );
    }
}
