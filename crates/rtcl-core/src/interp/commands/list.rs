//! List commands: list, llength, lindex, lappend, lrange, lsearch, lsort,
//! linsert, lreplace, lassign, lrepeat, lreverse, concat, split, join, lmap.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::types::parse_index;
use crate::value::Value;

pub fn cmd_list(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    Ok(Value::from_list(&args[1..]))
}

pub fn cmd_llength(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("llength", 2, args.len()));
    }
    let list = args[1].as_list().unwrap_or_default();
    Ok(Value::from_int(list.len() as i64))
}

pub fn cmd_lindex(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args("lindex", 3, args.len()));
    }
    let list = args[1].as_list().unwrap_or_default();
    let idx = parse_index(args[2].as_str(), list.len());
    match idx {
        Some(i) if i < list.len() => Ok(list[i].clone()),
        _ => Ok(Value::empty()),
    }
}

pub fn cmd_lappend(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("lappend", 2, args.len()));
    }
    let var_name = args[1].as_str();
    let mut list = interp
        .get_var(var_name)
        .ok()
        .and_then(|v| v.as_list())
        .unwrap_or_default();
    for arg in &args[2..] {
        list.push(arg.clone());
    }
    let result = Value::from_list(&list);
    interp.set_var(var_name, result.clone())
}

pub fn cmd_lrange(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 4 {
        return Err(Error::wrong_args_with_usage("lrange", 4, args.len(), "list first last"));
    }
    let list = args[1].as_list().unwrap_or_default();
    let first = parse_index(args[2].as_str(), list.len()).unwrap_or(0);
    let end = parse_index(args[3].as_str(), list.len()).unwrap_or(0);

    if first <= end && first < list.len() {
        let result: Vec<Value> = list[first..=end.min(list.len() - 1)].to_vec();
        Ok(Value::from_list(&result))
    } else {
        Ok(Value::empty())
    }
}

pub fn cmd_lsearch(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("lsearch", 3, args.len(), "?options? list pattern"));
    }

    let mut i = 1;
    let mut exact = false;
    let mut all = false;
    let mut inline = false;
    let mut not_match = false;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-exact" => { exact = true; i += 1; }
            "-glob" => { exact = false; i += 1; }
            "-all" => { all = true; i += 1; }
            "-inline" => { inline = true; i += 1; }
            "-not" => { not_match = true; i += 1; }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    if i + 1 >= args.len() {
        return Err(Error::wrong_args("lsearch", 3, args.len()));
    }

    let list = args[i].as_list().unwrap_or_default();
    let pattern = args[i + 1].as_str();

    let matches: Vec<(usize, &Value)> = list
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            let m = if exact {
                v.as_str() == pattern
            } else {
                glob_match(pattern, v.as_str())
            };
            if not_match { !m } else { m }
        })
        .collect();

    if inline {
        let result: Vec<Value> = matches.iter().map(|(_, v)| (*v).clone()).collect();
        Ok(Value::from_list(&result))
    } else if all {
        let result: Vec<Value> = matches.iter().map(|(idx, _)| Value::from_int(*idx as i64)).collect();
        Ok(Value::from_list(&result))
    } else {
        Ok(Value::from_int(matches.first().map(|(idx, _)| *idx as i64).unwrap_or(-1)))
    }
}

pub fn cmd_lsort(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("lsort", 2, args.len(), "?options? list"));
    }

    let mut i = 1;
    let mut decreasing = false;
    let mut unique = false;
    let mut nocase = false;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-decreasing" => { decreasing = true; i += 1; }
            "-increasing" => { decreasing = false; i += 1; }
            "-unique" => { unique = true; i += 1; }
            "-nocase" => { nocase = true; i += 1; }
            "-ascii" | "-dictionary" | "-integer" | "-real" => { i += 1; }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args("lsort", 2, args.len()));
    }

    let mut list = args[i].as_list().unwrap_or_default();
    let mut seen = std::collections::HashSet::new();

    list.sort_by(|a, b| {
        let (a_str, b_str) = if nocase {
            (a.as_str().to_lowercase(), b.as_str().to_lowercase())
        } else {
            (a.as_str().to_string(), b.as_str().to_string())
        };
        let cmp = a_str.cmp(&b_str);
        if decreasing { cmp.reverse() } else { cmp }
    });

    if unique {
        list.retain(|v| seen.insert(v.as_str().to_string()));
    }

    Ok(Value::from_list(&list))
}

pub fn cmd_linsert(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("linsert", 3, args.len(), "list index ?element ...?"));
    }
    let list = args[1].as_list().unwrap_or_default();
    let index = parse_index(args[2].as_str(), list.len()).unwrap_or(list.len());
    let index = index.min(list.len());
    let elements: Vec<Value> = args[3..].to_vec();
    let mut result = Vec::with_capacity(list.len() + elements.len());
    result.extend(list[..index].iter().cloned());
    result.extend(elements);
    result.extend(list[index..].iter().cloned());
    Ok(Value::from_list(&result))
}

pub fn cmd_lreplace(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage("lreplace", 4, args.len(), "list first last ?element ...?"));
    }
    let list = args[1].as_list().unwrap_or_default();
    let first = parse_index(args[2].as_str(), list.len()).unwrap_or(0).min(list.len());
    let last = parse_index(args[3].as_str(), list.len()).unwrap_or(0).min(list.len().saturating_sub(1));

    let mut result = Vec::with_capacity(list.len());
    result.extend(list[..first].iter().cloned());
    result.extend(args[4..].iter().cloned());
    if last + 1 < list.len() {
        result.extend(list[last + 1..].iter().cloned());
    }
    Ok(Value::from_list(&result))
}

pub fn cmd_lassign(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("lassign", 3, args.len(), "list varname ?varname ...?"));
    }
    let list = args[1].as_list().unwrap_or_default();
    let vars: Vec<&str> = args[2..].iter().map(|v| v.as_str()).collect();
    for (i, var) in vars.iter().enumerate() {
        let value = list.get(i).cloned().unwrap_or_else(Value::empty);
        interp.set_var(var, value)?;
    }
    if list.len() > vars.len() {
        Ok(Value::from_list(&list[vars.len()..]))
    } else {
        Ok(Value::empty())
    }
}

pub fn cmd_lrepeat(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("lrepeat", 2, args.len(), "count ?element ...?"));
    }
    let count = args[1].as_int().unwrap_or(0) as usize;
    let elements: Vec<Value> = args[2..].to_vec();
    if elements.is_empty() {
        return Ok(Value::empty());
    }
    let mut result = Vec::with_capacity(count * elements.len());
    for _ in 0..count {
        result.extend(elements.iter().cloned());
    }
    Ok(Value::from_list(&result))
}

pub fn cmd_lreverse(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("lreverse", 2, args.len()));
    }
    let mut list = args[1].as_list().unwrap_or_default();
    list.reverse();
    Ok(Value::from_list(&list))
}

pub fn cmd_concat(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut result = String::new();
    for arg in &args[1..] {
        if !result.is_empty() { result.push(' '); }
        result.push_str(arg.as_str());
    }
    Ok(Value::from_str(&result))
}

pub fn cmd_split(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("split", 2, args.len(), "string ?splitChars?"));
    }
    let string = args[1].as_str();
    if args.len() == 2 {
        let result: Vec<Value> = string.split_whitespace().map(|s| Value::from_str(s)).collect();
        Ok(Value::from_list(&result))
    } else {
        let split_chars = args[2].as_str();
        if split_chars.is_empty() {
            let result: Vec<Value> = string.chars().map(|c| Value::from_str(&c.to_string())).collect();
            Ok(Value::from_list(&result))
        } else {
            let result: Vec<Value> = string
                .split(|c| split_chars.contains(c))
                .map(|s| Value::from_str(s))
                .collect();
            Ok(Value::from_list(&result))
        }
    }
}

pub fn cmd_join(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("join", 2, args.len(), "list ?joinString?"));
    }
    let list = args[1].as_list().unwrap_or_default();
    let sep = if args.len() == 3 { args[2].as_str() } else { " " };
    let result: String = list.iter().map(|v| v.as_str()).collect::<Vec<&str>>().join(sep);
    Ok(Value::from_str(&result))
}

/// lmap — Like foreach but collects body results into a list.
/// Usage: lmap varList list ?varList list ...? body
pub fn cmd_lmap(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 || args.len() % 2 != 0 {
        return Err(Error::wrong_args_with_usage(
            "lmap", 4, args.len(),
            "varList list ?varList list ...? body",
        ));
    }

    let body = args[args.len() - 1].as_str();
    let mut collected: Vec<Value> = Vec::new();

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
            Ok(v) => collected.push(v),
            Err(e) => {
                if e.is_break() { break; }
                if e.is_continue() { continue; }
                return Err(e);
            }
        }
    }

    Ok(Value::from_list(&collected))
}
