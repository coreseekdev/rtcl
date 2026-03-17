//! Miscellaneous commands: set, expr, incr, unset, info, subst, append, disassemble.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;
use rtcl_vm::Compiler;

pub fn cmd_set(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    match args.len() {
        2 => interp.get_var(args[1].as_str()).cloned(),
        3 => interp.set_var(args[1].as_str(), args[2].clone()),
        _ => Err(Error::wrong_args("set", 2, args.len())),
    }
}

pub fn cmd_expr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("expr", 2, args.len()));
    }
    let expr_str = if args.len() == 2 {
        args[1].as_str().to_string()
    } else {
        args[1..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };
    interp.eval_expr(&expr_str)
}

pub fn cmd_incr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args("incr", 2, args.len()));
    }
    let var_name = args[1].as_str();
    let amount = if args.len() == 3 {
        args[2].as_int().unwrap_or(1)
    } else {
        1
    };
    let current = interp.get_var(var_name).ok().and_then(|v| v.as_int()).unwrap_or(0);
    let new_val = Value::from_int(current + amount);
    interp.set_var(var_name, new_val)
}

pub fn cmd_unset(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("unset", 2, args.len()));
    }
    let mut nocomplain = false;
    let start = if args[1].as_str() == "-nocomplain" {
        nocomplain = true;
        2
    } else {
        1
    };
    for arg in &args[start..] {
        let name = arg.as_str();
        if interp.vars.remove(name).is_none() && !nocomplain {
            return Err(Error::runtime(
                format!("can't unset \"{}\": no such variable", name),
                crate::error::ErrorCode::NotFound,
            ));
        }
    }
    Ok(Value::empty())
}

pub fn cmd_info(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("info", 2, args.len()));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        "commands" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut cmds: Vec<Value> = interp
                .commands
                .keys()
                .chain(interp.procs.keys())
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            cmds.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&cmds))
        }
        "procs" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut names: Vec<Value> = interp
                .procs
                .keys()
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&names))
        }
        "exists" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info exists", 3, args.len()));
            }
            let name = args[2].as_str();
            Ok(Value::from_bool(interp.vars.contains_key(name)))
        }
        "vars" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut vars: Vec<Value> = interp
                .vars
                .keys()
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&vars))
        }
        "globals" => {
            // Same as vars in our flat model
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut vars: Vec<Value> = interp
                .vars
                .keys()
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&vars))
        }
        "body" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info body", 3, args.len()));
            }
            let name = args[2].as_str();
            if let Some(proc_def) = interp.procs.get(name) {
                Ok(Value::from_str(&proc_def.body))
            } else {
                Err(Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                ))
            }
        }
        "args" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info args", 3, args.len()));
            }
            let name = args[2].as_str();
            if let Some(proc_def) = interp.procs.get(name) {
                let arg_names: Vec<Value> = proc_def
                    .params
                    .iter()
                    .map(|(n, _)| Value::from_str(n))
                    .collect();
                Ok(Value::from_list(&arg_names))
            } else {
                Err(Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                ))
            }
        }
        "level" => Ok(Value::from_int(interp.call_depth as i64)),
        _ => Err(Error::runtime(
            format!("unknown info subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

pub fn cmd_subst(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("subst", 2, args.len()));
    }

    let mut nobackslashes = false;
    let mut nocommands = false;
    let mut novariables = false;
    let mut i = 1;

    while i < args.len() - 1 {
        match args[i].as_str() {
            "-nobackslashes" => nobackslashes = true,
            "-nocommands" => nocommands = true,
            "-novariables" => novariables = true,
            _ => break,
        }
        i += 1;
    }

    let template = args[i].as_str();
    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut ci = 0;

    while ci < chars.len() {
        let ch = chars[ci];
        match ch {
            '\\' if !nobackslashes && ci + 1 < chars.len() => {
                ci += 1;
                match chars[ci] {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    other => { result.push('\\'); result.push(other); }
                }
                ci += 1;
            }
            '$' if !novariables && ci + 1 < chars.len() => {
                ci += 1;
                let mut var_name = String::new();
                if ci < chars.len() && chars[ci] == '{' {
                    ci += 1;
                    while ci < chars.len() && chars[ci] != '}' {
                        var_name.push(chars[ci]);
                        ci += 1;
                    }
                    if ci < chars.len() { ci += 1; } // skip '}'
                } else {
                    while ci < chars.len() && (chars[ci].is_alphanumeric() || chars[ci] == '_') {
                        var_name.push(chars[ci]);
                        ci += 1;
                    }
                    // Check for array ref
                    if ci < chars.len() && chars[ci] == '(' {
                        var_name.push('(');
                        ci += 1;
                        while ci < chars.len() && chars[ci] != ')' {
                            var_name.push(chars[ci]);
                            ci += 1;
                        }
                        if ci < chars.len() {
                            var_name.push(')');
                            ci += 1;
                        }
                    }
                }
                if let Ok(val) = interp.get_var(&var_name) {
                    result.push_str(val.as_str());
                } else {
                    result.push('$');
                    result.push_str(&var_name);
                }
            }
            '[' if !nocommands => {
                ci += 1;
                let mut depth = 1;
                let mut cmd = String::new();
                while ci < chars.len() && depth > 0 {
                    if chars[ci] == '[' { depth += 1; }
                    else if chars[ci] == ']' {
                        depth -= 1;
                        if depth == 0 { ci += 1; break; }
                    }
                    cmd.push(chars[ci]);
                    ci += 1;
                }
                let val = interp.eval(&cmd)?;
                result.push_str(val.as_str());
            }
            _ => {
                result.push(ch);
                ci += 1;
            }
        }
    }

    Ok(Value::from_str(&result))
}

pub fn cmd_append(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("append", 2, args.len()));
    }
    let var_name = args[1].as_str();
    let mut current = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
    for arg in &args[2..] {
        current.push_str(arg.as_str());
    }
    let result = Value::from_str(&current);
    interp.set_var(var_name, result.clone())
}

/// `disassemble script` — compile and display the bytecode for a script.
pub fn cmd_disassemble(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("disassemble", 2, args.len()));
    }
    let script = args[1].as_str();
    let code = Compiler::compile_script(script)
        .map_err(|e| Error::syntax(&e.to_string(), 0, 0))?;
    Ok(Value::from_str(&format!("{}", code)))
}

/// `scan string format ?varName ...?`
///
/// Parses `string` according to `format` (subset of C sscanf).
/// If varNames are given, stores results and returns count of conversions.
/// If no varNames, returns a list of converted values.
pub fn cmd_scan(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "scan",
            3,
            args.len(),
            "string format ?varName ...?",
        ));
    }

    let input = args[1].as_str();
    let format = args[2].as_str();
    let has_vars = args.len() > 3;

    let mut results: Vec<String> = Vec::new();
    let mut input_pos = 0;
    let fmt_bytes = format.as_bytes();
    let mut fmt_pos = 0;

    while fmt_pos < fmt_bytes.len() {
        if fmt_bytes[fmt_pos] == b'%' {
            fmt_pos += 1;
            if fmt_pos >= fmt_bytes.len() {
                break;
            }

            // Handle %%
            if fmt_bytes[fmt_pos] == b'%' {
                if input_pos < input.len() && input.as_bytes()[input_pos] == b'%' {
                    input_pos += 1;
                }
                fmt_pos += 1;
                continue;
            }

            // Check for suppress flag *
            let suppress = fmt_bytes[fmt_pos] == b'*';
            if suppress {
                fmt_pos += 1;
            }

            // Optional width
            let mut width: Option<usize> = None;
            let width_start = fmt_pos;
            while fmt_pos < fmt_bytes.len() && fmt_bytes[fmt_pos].is_ascii_digit() {
                fmt_pos += 1;
            }
            if fmt_pos > width_start {
                width = format[width_start..fmt_pos].parse().ok();
            }

            if fmt_pos >= fmt_bytes.len() {
                break;
            }

            let spec = fmt_bytes[fmt_pos];
            fmt_pos += 1;

            let inp = &input[input_pos..];

            match spec {
                b'd' | b'i' => {
                    // Skip whitespace
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let s = &trimmed[..max.min(trimmed.len())];
                    let end = s.find(|c: char| !c.is_ascii_digit() && c != '-' && c != '+')
                        .unwrap_or(s.len());
                    if end == 0 {
                        break;
                    }
                    let num_str = &s[..end];
                    if !suppress {
                        results.push(num_str.to_string());
                    }
                    input_pos += end;
                }
                b'x' | b'X' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let s = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X"))
                        .unwrap_or(trimmed);
                    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                        input_pos += 2;
                    }
                    let max = width.unwrap_or(s.len());
                    let end = s[..max.min(s.len())].find(|c: char| !c.is_ascii_hexdigit())
                        .unwrap_or(max.min(s.len()));
                    if end == 0 {
                        break;
                    }
                    let val = i64::from_str_radix(&s[..end], 16).unwrap_or(0);
                    if !suppress {
                        results.push(val.to_string());
                    }
                    input_pos += end;
                }
                b'o' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let end = trimmed[..max.min(trimmed.len())]
                        .find(|c: char| !('0'..='7').contains(&c))
                        .unwrap_or(max.min(trimmed.len()));
                    if end == 0 {
                        break;
                    }
                    let val = i64::from_str_radix(&trimmed[..end], 8).unwrap_or(0);
                    if !suppress {
                        results.push(val.to_string());
                    }
                    input_pos += end;
                }
                b'f' | b'e' | b'g' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let s = &trimmed[..max.min(trimmed.len())];
                    let end = s.find(|c: char| {
                        !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e' && c != 'E'
                    }).unwrap_or(s.len());
                    if end == 0 {
                        break;
                    }
                    if !suppress {
                        results.push(s[..end].to_string());
                    }
                    input_pos += end;
                }
                b's' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let s = &trimmed[..max.min(trimmed.len())];
                    let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
                    if end == 0 {
                        break;
                    }
                    if !suppress {
                        results.push(s[..end].to_string());
                    }
                    input_pos += end;
                }
                b'c' => {
                    if inp.is_empty() {
                        break;
                    }
                    let ch = inp.chars().next().unwrap();
                    if !suppress {
                        results.push((ch as u32).to_string());
                    }
                    input_pos += ch.len_utf8();
                }
                b'n' => {
                    if !suppress {
                        results.push(input_pos.to_string());
                    }
                }
                _ => {
                    return Err(Error::runtime(
                        format!("bad scan conversion character '{}'", spec as char),
                        crate::error::ErrorCode::Generic,
                    ));
                }
            }
        } else if fmt_bytes[fmt_pos].is_ascii_whitespace() {
            // Whitespace in format matches any whitespace in input
            fmt_pos += 1;
            while input_pos < input.len() && input.as_bytes()[input_pos].is_ascii_whitespace() {
                input_pos += 1;
            }
        } else {
            // Literal match
            if input_pos < input.len() && input.as_bytes()[input_pos] == fmt_bytes[fmt_pos] {
                input_pos += 1;
                fmt_pos += 1;
            } else {
                break;
            }
        }
    }

    if has_vars {
        // Store in variables, return count
        let mut count = 0;
        for (idx, var_arg) in args[3..].iter().enumerate() {
            if let Some(val) = results.get(idx) {
                interp.set_var(var_arg.as_str(), Value::from_str(val))?;
                count += 1;
            }
        }
        Ok(Value::from_int(count))
    } else {
        // Return as list
        let list_str = results
            .iter()
            .map(|s| {
                if s.is_empty() || s.contains(' ') {
                    format!("{{{}}}", s)
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Value::from_str(&list_str))
    }
}
