//! Iteration and timing commands: while, for, foreach, time, timerate, range.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

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
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    continue;
                }
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
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    /* fall through to next */
                }
                else { return Err(e); }
            }
        }

        match interp.eval(next) {
            Ok(_) => {}
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                }
                else { return Err(e); }
            }
        }
    }

    Ok(result)
}

pub fn cmd_foreach(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 || !args.len().is_multiple_of(2) {
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
                interp.set_var(var, value)?;
            }
        }
        match interp.eval(body) {
            Ok(v) => result = v,
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    continue;
                }
                return Err(e);
            }
        }
    }

    Ok(result)
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

/// `loop var ?first? limit ?incr? body` — Numeric for-loop (jimtcl extension).
///
/// Forms:
///   loop var limit body          — var goes from 0 to limit-1, step 1
///   loop var first limit body    — var goes from first to limit-1, step 1
///   loop var first limit incr body — var goes from first towards limit, step incr
pub fn cmd_loop(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (var, mut i, limit, step, body) = match args.len() {
        // loop var limit body
        4 => {
            let var = args[1].as_str().to_string();
            let limit = args[2].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[2].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            (var, 0i64, limit, 1i64, args[3].as_str().to_string())
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
            (var, first, limit, 1i64, args[4].as_str().to_string())
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
            (var, first, limit, step, args[5].as_str().to_string())
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

    let mut result = Value::empty();

    loop {
        let done = if step > 0 { i >= limit } else { i <= limit };
        if done {
            break;
        }
        interp.set_var(&var, Value::from_int(i))?;
        match interp.eval(&body) {
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

    // -- Multi-level break/continue tests --

    #[test]
    fn test_break_2_nested_for() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            set result ""
            for {set i 0} {$i < 3} {incr i} {
                for {set j 0} {$j < 3} {incr j} {
                    if {$j == 1} { break 2 }
                    append result "$i$j "
                }
            }
            set result
        "#).unwrap();
        assert_eq!(r.as_str(), "00 ");
    }

    #[test]
    fn test_continue_2_nested_for() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            set result ""
            for {set i 0} {$i < 3} {incr i} {
                for {set j 0} {$j < 3} {incr j} {
                    if {$j == 1} { continue 2 }
                    append result "$i$j "
                }
            }
            set result
        "#).unwrap();
        assert_eq!(r.as_str(), "00 10 20 ");
    }

    #[test]
    fn test_break_1_same_as_break() {
        let mut interp = Interp::new();
        let r = interp.eval("set r {}; loop i 5 { if {$i == 2} {break 1}; lappend r $i }; set r").unwrap();
        assert_eq!(r.as_str(), "0 1");
    }

    #[test]
    fn test_break_bad_level() {
        let mut interp = Interp::new();
        assert!(interp.eval("break 0").is_err());
        assert!(interp.eval("break -1").is_err());
        assert!(interp.eval("break abc").is_err());
    }

    #[test]
    fn test_continue_bad_level() {
        let mut interp = Interp::new();
        assert!(interp.eval("for {set i 0} {$i<1} {incr i} { continue 0 }").is_err());
    }

    #[test]
    fn test_break_3_triple_nested() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            set result ""
            for {set i 0} {$i < 2} {incr i} {
                for {set j 0} {$j < 2} {incr j} {
                    for {set k 0} {$k < 2} {incr k} {
                        if {$k == 1} { break 3 }
                        append result "$i$j$k "
                    }
                }
            }
            set result
        "#).unwrap();
        assert_eq!(r.as_str(), "000 ");
    }

    #[test]
    fn test_break_2_while() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            set result ""
            set i 0
            while {$i < 3} {
                set j 0
                while {$j < 3} {
                    if {$j == 1} { break 2 }
                    append result "$i$j "
                    incr j
                }
                incr i
            }
            set result
        "#).unwrap();
        assert_eq!(r.as_str(), "00 ");
    }

    #[test]
    fn test_break_2_foreach() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"
            set result ""
            foreach i {a b c} {
                foreach j {1 2 3} {
                    if {$j == 2} { break 2 }
                    append result "$i$j "
                }
            }
            set result
        "#).unwrap();
        assert_eq!(r.as_str(), "a1 ");
    }
}
