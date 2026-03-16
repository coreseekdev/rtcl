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

    if cond.is_true() {
        return interp.eval(args[2].as_str());
    }

    let mut i = 3;
    while i < args.len() {
        let word = args[i].as_str();
        match word {
            "elseif" => {
                if i + 2 >= args.len() {
                    return Err(Error::wrong_args("elseif", 2, args.len() - i));
                }
                let expr = args[i + 1].as_str();
                let cond = interp.eval_expr(expr)?;
                if cond.is_true() {
                    return interp.eval(args[i + 2].as_str());
                }
                i += 3;
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
            "varname list body ?varname list body ...?",
        ));
    }

    let body = args[args.len() - 1].as_str();
    let mut result = Value::empty();

    let mut var_lists: Vec<(&str, Vec<Value>)> = Vec::new();
    let mut i = 1;
    while i < args.len() - 1 {
        let var = args[i].as_str();
        let list = args[i + 1].as_list().unwrap_or_default();
        var_lists.push((var, list));
        i += 2;
    }

    let max_len = var_lists.iter().map(|(_, l)| l.len()).max().unwrap_or(0);

    for idx in 0..max_len {
        for (var, list) in &var_lists {
            let value = list.get(idx).cloned().unwrap_or_else(Value::empty);
            interp.set_var(var, value)?;
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
    let value = if args.len() > 1 { args[1].clone() } else { Value::empty() };
    Err(Error::ret(Some(value.as_str().to_string())))
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
