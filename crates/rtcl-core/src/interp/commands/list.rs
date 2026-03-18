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

pub fn cmd_lsort(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("lsort", 2, args.len(), "?options? list"));
    }

    let mut i = 1;
    let mut decreasing = false;
    let mut unique = false;
    let mut nocase = false;
    let mut sort_type = SortType::Ascii;
    let mut command: Option<String> = None;
    let mut index: Option<String> = None;

    while i < args.len() - 1 && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-decreasing" => { decreasing = true; i += 1; }
            "-increasing" => { decreasing = false; i += 1; }
            "-unique" => { unique = true; i += 1; }
            "-nocase" => { nocase = true; i += 1; }
            "-ascii" | "-dictionary" => { sort_type = SortType::Ascii; i += 1; }
            "-integer" => { sort_type = SortType::Integer; i += 1; }
            "-real" => { sort_type = SortType::Real; i += 1; }
            "-command" => {
                i += 1;
                if i >= args.len() - 1 {
                    return Err(Error::runtime(
                        "missing value for -command",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                command = Some(args[i].as_str().to_string());
                i += 1;
            }
            "-index" => {
                i += 1;
                if i >= args.len() - 1 {
                    return Err(Error::runtime(
                        "missing value for -index",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                index = Some(args[i].as_str().to_string());
                i += 1;
            }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args("lsort", 2, args.len()));
    }

    let mut list = args[i].as_list().unwrap_or_default();

    // Helper: extract sort key from element (for -index)
    let get_key = |elem: &Value| -> Value {
        if let Some(ref idx) = index {
            let sub = elem.as_list().unwrap_or_else(|| vec![elem.clone()]);
            let idx_num: usize = if idx == "end" {
                sub.len().saturating_sub(1)
            } else if let Some(rest) = idx.strip_prefix("end-") {
                sub.len().saturating_sub(1 + rest.parse::<usize>().unwrap_or(0))
            } else {
                idx.parse().unwrap_or(0)
            };
            sub.get(idx_num).cloned().unwrap_or_else(Value::empty)
        } else {
            elem.clone()
        }
    };

    if let Some(ref cmd_name) = command {
        // -command: use a Tcl proc for comparison
        let cmd = cmd_name.clone();
        let mut err: Option<Error> = None;
        list.sort_by(|a, b| {
            if err.is_some() {
                return std::cmp::Ordering::Equal;
            }
            let ka = get_key(a);
            let kb = get_key(b);
            let script = format!("{} {} {}", cmd,
                crate::value::tcl_quote(ka.as_str()),
                crate::value::tcl_quote(kb.as_str()),
            );
            match interp.eval(&script) {
                Ok(v) => {
                    let n = v.as_int().unwrap_or(0);
                    let cmp = n.cmp(&0);
                    if decreasing { cmp.reverse() } else { cmp }
                }
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
    } else {
        list.sort_by(|a, b| {
            let ka = get_key(a);
            let kb = get_key(b);
            let cmp = match sort_type {
                SortType::Integer => {
                    let ai = ka.as_int().unwrap_or(0);
                    let bi = kb.as_int().unwrap_or(0);
                    ai.cmp(&bi)
                }
                SortType::Real => {
                    let af = ka.as_str().parse::<f64>().unwrap_or(0.0);
                    let bf = kb.as_str().parse::<f64>().unwrap_or(0.0);
                    af.partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortType::Ascii => {
                    if nocase {
                        ka.as_str().to_lowercase().cmp(&kb.as_str().to_lowercase())
                    } else {
                        ka.as_str().cmp(kb.as_str())
                    }
                }
            };
            if decreasing { cmp.reverse() } else { cmp }
        });
    }

    if unique {
        let mut seen = std::collections::HashSet::new();
        list.retain(|v| seen.insert(v.as_str().to_string()));
    }

    Ok(Value::from_list(&list))
}

#[derive(Clone, Copy)]
enum SortType {
    Ascii,
    Integer,
    Real,
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
        let trimmed = arg.as_str().trim();
        if trimmed.is_empty() { continue; }
        if !result.is_empty() { result.push(' '); }
        result.push_str(trimmed);
    }
    Ok(Value::from_str(&result))
}

pub fn cmd_split(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("split", 2, args.len(), "string ?splitChars?"));
    }
    let string = args[1].as_str();
    if args.len() == 2 {
        let result: Vec<Value> = string.split_whitespace().map(Value::from_str).collect();
        Ok(Value::from_list(&result))
    } else {
        let split_chars = args[2].as_str();
        if split_chars.is_empty() {
            let result: Vec<Value> = string.chars().map(|c| Value::from_str(&c.to_string())).collect();
            Ok(Value::from_list(&result))
        } else {
            let result: Vec<Value> = string
                .split(|c| split_chars.contains(c))
                .map(Value::from_str)
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
    if args.len() < 4 || !args.len().is_multiple_of(2) {
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

/// lset varName ?index ...? value
/// Set an element in a list variable.
pub fn cmd_lset(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "lset",
            3,
            args.len(),
            "varName ?index ...? value",
        ));
    }

    let var_name = args[1].as_str();
    let value = &args[args.len() - 1];

    let current = interp.get_var(var_name)?.clone();
    let mut list = current.as_list().unwrap_or_default();

    if args.len() == 3 {
        // lset var value — replace entire list with value
        interp.set_var(var_name, value.clone())?;
        return Ok(value.clone());
    }

    // Single index case: lset var index value
    // Multi-index: lset var i1 i2 ... value (nested lists)
    let indices: Vec<&Value> = args[2..args.len() - 1].iter().collect();

    if indices.len() == 1 {
        // Check if the single index is actually a list of indices
        let idx_list = indices[0].as_list().unwrap_or_default();
        if idx_list.len() > 1 {
            // Treat as nested indices: lset var {0 1} value
            let result = lset_nested(&list, &idx_list, value)?;
            interp.set_var(var_name, result.clone())?;
            Ok(result)
        } else {
            let idx_str = indices[0].as_str();
            let len = list.len();
            let idx = resolve_index(idx_str, len)?;
            if idx >= len {
                return Err(Error::runtime(
                    "list index out of range",
                    crate::error::ErrorCode::Generic,
                ));
            }
            list[idx] = value.clone();
            let result = Value::from_list(&list);
            interp.set_var(var_name, result.clone())?;
            Ok(result)
        }
    } else {
        // Multiple separate index args: lset var i1 i2 ... value
        let idx_values: Vec<Value> = indices.iter().map(|v| (*v).clone()).collect();
        let result = lset_nested(&list, &idx_values, value)?;
        interp.set_var(var_name, result.clone())?;
        Ok(result)
    }
}

/// Recursively set a nested element in a list.
fn lset_nested(list: &[Value], indices: &[Value], value: &Value) -> Result<Value> {
    if indices.is_empty() {
        return Ok(value.clone());
    }
    let idx_str = indices[0].as_str();
    let len = list.len();
    let idx = resolve_index(idx_str, len)?;
    if idx >= len {
        return Err(Error::runtime(
            "list index out of range".to_string(),
            crate::error::ErrorCode::Generic,
        ));
    }
    let mut new_list = list.to_vec();
    if indices.len() == 1 {
        new_list[idx] = value.clone();
    } else {
        let sub_list = new_list[idx].as_list().unwrap_or_default();
        new_list[idx] = lset_nested(&sub_list, &indices[1..], value)?;
    }
    Ok(Value::from_list(&new_list))
}

fn resolve_index(idx_str: &str, len: usize) -> Result<usize> {
    if idx_str == "end" {
        if len == 0 {
            return Err(Error::runtime("list index out of range", crate::error::ErrorCode::Generic));
        }
        return Ok(len - 1);
    }
    if let Some(rest) = idx_str.strip_prefix("end-") {
        let n: usize = rest.parse().map_err(|_| {
            Error::runtime(format!("bad index \"{}\"", idx_str), crate::error::ErrorCode::Generic)
        })?;
        if n >= len {
            return Err(Error::runtime("list index out of range", crate::error::ErrorCode::Generic));
        }
        return Ok(len - 1 - n);
    }
    let idx: i64 = idx_str.parse().map_err(|_| {
        Error::runtime(format!("bad index \"{}\"", idx_str), crate::error::ErrorCode::Generic)
    })?;
    if idx < 0 {
        return Err(Error::runtime("list index out of range", crate::error::ErrorCode::Generic));
    }
    Ok(idx as usize)
}

/// `lsubst ?-command? ?-variable? ?-nobackslashes? ?-nocommands? ?-novariables? string` —
/// Perform substitution like `subst` but split the result into a proper Tcl list.
///
/// This is the jimtcl extension: parse a string with substitutions and return the result as a list.
pub fn cmd_lsubst(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "lsubst",
            2,
            args.len(),
            "?-nobackslashes? ?-nocommands? ?-novariables? string",
        ));
    }

    // Parse flags
    let mut no_backslashes = false;
    let mut no_commands = false;
    let mut no_variables = false;
    let mut string_idx = 1;

    while string_idx < args.len() - 1 {
        match args[string_idx].as_str() {
            "-nobackslashes" => no_backslashes = true,
            "-nocommands" => no_commands = true,
            "-novariables" => no_variables = true,
            other => {
                return Err(Error::runtime(
                    format!("bad option \"{}\": must be -nobackslashes, -nocommands, or -novariables", other),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
        string_idx += 1;
    }

    let input = args[string_idx].as_str();

    // Perform substitution respecting flags
    let substituted = if no_backslashes && no_commands && no_variables {
        // No substitution at all
        input.to_string()
    } else {
        // Build a subst call with appropriate flags
        let mut subst_args = vec![Value::from_str("subst")];
        if no_backslashes {
            subst_args.push(Value::from_str("-nobackslashes"));
        }
        if no_commands {
            subst_args.push(Value::from_str("-nocommands"));
        }
        if no_variables {
            subst_args.push(Value::from_str("-novariables"));
        }
        subst_args.push(Value::from_str(input));
        super::misc::cmd_subst(interp, &subst_args)?.as_str().to_string()
    };

    // Split result into a list
    let elements = Value::from_str(&substituted).as_list().unwrap_or_default();
    Ok(Value::from_list(&elements))
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_lsubst_simple() {
        let mut interp = Interp::new();
        let r = interp.eval("lsubst {a b c}").unwrap();
        assert_eq!(r.as_str(), "a b c");
    }

    #[test]
    fn test_lsubst_variable() {
        let mut interp = Interp::new();
        interp.eval("set x hello").unwrap();
        let r = interp.eval("lsubst {$x world}").unwrap();
        assert_eq!(r.as_str(), "hello world");
    }

    #[test]
    fn test_lsubst_novariables() {
        let mut interp = Interp::new();
        interp.eval("set x hello").unwrap();
        let r = interp.eval("lsubst -novariables {$x world}").unwrap();
        // $x should not be substituted
        let list = r.as_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_str(), "$x");
        assert_eq!(list[1].as_str(), "world");
    }

    #[test]
    fn test_lsubst_no_args_error() {
        let mut interp = Interp::new();
        assert!(interp.eval("lsubst").is_err());
    }

    #[test]
    fn test_lsubst_bad_option() {
        let mut interp = Interp::new();
        assert!(interp.eval("lsubst -badopt {a b}").is_err());
    }
}
