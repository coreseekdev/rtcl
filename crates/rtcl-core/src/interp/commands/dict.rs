//! Dict commands: dict create/get/set/exists/unset/keys/values/size/for/merge/replace etc.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;
use indexmap::IndexMap;

/// Parse a value as a dict, returning a cloned IndexMap (for mutation).
fn parse_dict(val: &Value) -> Result<IndexMap<String, Value>> {
    val.as_dict().ok_or_else(|| {
        Error::runtime(
            "missing value to go with key",
            crate::error::ErrorCode::InvalidOp,
        )
    })
}

pub fn cmd_dict(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("dict", 2, args.len()));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        // ── dict create ?key value ...? ────────────────────────
        "create" => {
            if (args.len() - 2) % 2 != 0 {
                return Err(Error::runtime(
                    "wrong # args: dict create requires key value pairs",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            let mut entries = IndexMap::with_capacity((args.len() - 2) / 2);
            for c in args[2..].chunks(2) {
                entries.insert(c[0].as_str().to_string(), c[1].clone());
            }
            Ok(Value::from_dict_cached(entries))
        }

        // ── dict get dictionary ?key ...? ──────────────────────
        "get" => {
            if args.len() < 3 {
                return Err(Error::wrong_args("dict get", 3, args.len()));
            }
            if args.len() == 3 {
                return Ok(args[2].clone());
            }
            let mut current = args[2].clone();
            for key_arg in &args[3..] {
                let key = key_arg.as_str();
                let entries = parse_dict(&current)?;
                match entries.get(key) {
                    Some(v) => current = v.clone(),
                    None => {
                        return Err(Error::runtime(
                            format!("key \"{}\" not known in dictionary", key),
                            crate::error::ErrorCode::NotFound,
                        ))
                    }
                }
            }
            Ok(current)
        }

        // ── dict set dictVariable key ?key ...? value ──────────
        "set" => {
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict set",
                    5,
                    args.len(),
                    "dictVariable key ?key ...? value",
                ));
            }
            let var_name = args[2].as_str();
            let value = args[args.len() - 1].clone();
            let keys: Vec<&str> = args[3..args.len() - 1]
                .iter()
                .map(|v| v.as_str())
                .collect();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let entries = parse_dict(&dict_val).unwrap_or_default();
            let new_entries = dict_set_nested(entries, &keys, value)?;
            let result_val = Value::from_dict_cached(new_entries);
            interp.set_var(var_name, result_val)
        }

        // ── dict unset dictVariable key ?key ...? ──────────────
        "unset" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict unset",
                    4,
                    args.len(),
                    "dictVariable key ?key ...?",
                ));
            }
            let var_name = args[2].as_str();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(&dict_val)?;
            if args.len() == 4 {
                // Single key — remove directly
                entries.shift_remove(args[3].as_str());
            } else {
                // Nested unset
                let keys: Vec<&str> = args[3..].iter().map(|v| v.as_str()).collect();
                dict_unset_nested(&mut entries, &keys)?;
            }
            let result = Value::from_dict_cached(entries);
            interp.set_var(var_name, result)
        }

        // ── dict exists dictionary key ?key ...? ───────────────
        "exists" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("dict exists", 4, args.len()));
            }
            let mut current = args[2].clone();
            for key_arg in &args[3..] {
                let key = key_arg.as_str();
                match current.as_dict() {
                    Some(entries) => match entries.get(key) {
                        Some(v) => current = v.clone(),
                        None => return Ok(Value::from_bool(false)),
                    },
                    None => return Ok(Value::from_bool(false)),
                }
            }
            Ok(Value::from_bool(true))
        }

        // ── dict keys dictionary ?pattern? ─────────────────────
        "keys" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(Error::wrong_args("dict keys", 3, args.len()));
            }
            let entries = parse_dict(&args[2])?;
            let pattern = if args.len() == 4 {
                Some(args[3].as_str())
            } else {
                None
            };
            let keys: Vec<Value> = entries
                .keys()
                .filter(|k| pattern.is_none() || glob_match(pattern.unwrap(), k))
                .map(|k| Value::from_str(k))
                .collect();
            Ok(Value::from_list(&keys))
        }

        // ── dict values dictionary ?pattern? ───────────────────
        "values" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(Error::wrong_args("dict values", 3, args.len()));
            }
            let entries = parse_dict(&args[2])?;
            let pattern = if args.len() == 4 {
                Some(args[3].as_str())
            } else {
                None
            };
            let values: Vec<Value> = entries
                .values()
                .filter(|v| pattern.is_none() || glob_match(pattern.unwrap(), v.as_str()))
                .cloned()
                .collect();
            Ok(Value::from_list(&values))
        }

        // ── dict size dictionary ───────────────────────────────
        "size" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("dict size", 3, args.len()));
            }
            let entries = parse_dict(&args[2])?;
            Ok(Value::from_int(entries.len() as i64))
        }

        // ── dict for {keyVar valueVar} dictionary body ─────────
        "for" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict for",
                    5,
                    args.len(),
                    "{keyVar valueVar} dictionary body",
                ));
            }
            let var_list = args[2].as_list().unwrap_or_default();
            if var_list.len() != 2 {
                return Err(Error::runtime(
                    "must have exactly two variable names",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            let key_var = var_list[0].as_str().to_string();
            let val_var = var_list[1].as_str().to_string();
            let entries = parse_dict(&args[3])?;
            let body = args[4].as_str();
            let mut result = Value::empty();
            for (k, v) in &entries {
                interp.set_var(&key_var, Value::from_str(k))?;
                interp.set_var(&val_var, v.clone())?;
                match interp.eval(body) {
                    Ok(r) => result = r,
                    Err(e) => {
                        if e.is_break() {
                            break;
                        }
                        if e.is_continue() {
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Ok(result)
        }

        // ── dict merge ?dictionary ...? ────────────────────────
        "merge" => {
            let mut entries = IndexMap::new();
            for arg in &args[2..] {
                let new_entries = parse_dict(arg)?;
                entries.extend(new_entries);
            }
            Ok(Value::from_dict_cached(entries))
        }

        // ── dict replace dictionary ?key value ...? ────────────
        "replace" => {
            if args.len() < 3 {
                return Err(Error::wrong_args("dict replace", 3, args.len()));
            }
            if (args.len() - 3) % 2 != 0 {
                return Err(Error::runtime(
                    "wrong # args: must be key value pairs",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            let mut entries = parse_dict(&args[2])?;
            for chunk in args[3..].chunks(2) {
                entries.insert(chunk[0].as_str().to_string(), chunk[1].clone());
            }
            Ok(Value::from_dict_cached(entries))
        }

        // ── dict append dictVariable key ?string ...? ──────────
        "append" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict append",
                    4,
                    args.len(),
                    "dictVariable key ?string ...?",
                ));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(&dict_val).unwrap_or_default();
            let cur = entries
                .get(key)
                .map(|v| v.as_str().to_string())
                .unwrap_or_default();
            let mut s = cur;
            for v in &args[4..] {
                s.push_str(v.as_str());
            }
            entries.insert(key.to_string(), Value::from_str(&s));
            let result = Value::from_dict_cached(entries);
            interp.set_var(var_name, result)
        }

        // ── dict incr dictVariable key ?increment? ─────────────
        "incr" => {
            if args.len() < 4 || args.len() > 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict incr",
                    4,
                    args.len(),
                    "dictVariable key ?increment?",
                ));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str();
            let incr = if args.len() == 5 {
                args[4].as_int().ok_or_else(|| {
                    Error::runtime("expected integer", crate::error::ErrorCode::InvalidOp)
                })?
            } else {
                1
            };
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(&dict_val).unwrap_or_default();
            let cur = entries.get(key).and_then(|v| v.as_int()).unwrap_or(0);
            entries.insert(key.to_string(), Value::from_int(cur + incr));
            let result = Value::from_dict_cached(entries);
            interp.set_var(var_name, result)
        }

        // ── dict lappend dictVariable key ?value ...? ──────────
        "lappend" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict lappend",
                    4,
                    args.len(),
                    "dictVariable key ?value ...?",
                ));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(&dict_val).unwrap_or_default();
            let cur_val = entries.get(key).cloned().unwrap_or_default();
            let mut list = if cur_val.is_empty() {
                Vec::new()
            } else {
                cur_val.as_list().unwrap_or_default()
            };
            for v in &args[4..] {
                list.push(v.clone());
            }
            let new_val = Value::from_list(&list);
            entries.insert(key.to_string(), new_val);
            let result = Value::from_dict_cached(entries);
            interp.set_var(var_name, result)
        }

        // ── dict remove dictionary ?key ...? ───────────────────
        "remove" => {
            if args.len() < 3 {
                return Err(Error::wrong_args_with_usage(
                    "dict remove",
                    3,
                    args.len(),
                    "dictionary ?key ...?",
                ));
            }
            let mut entries = parse_dict(&args[2])?;
            for key_arg in &args[3..] {
                entries.shift_remove(key_arg.as_str());
            }
            Ok(Value::from_dict_cached(entries))
        }

        // ── dict with dictVariable ?key ...? body ──────────────
        "with" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict with",
                    4,
                    args.len(),
                    "dictVariable ?key ...? body",
                ));
            }
            let var_name = args[2].as_str();
            let body = args[args.len() - 1].as_str();

            // Navigate to nested dict if keys provided
            let dict_val = interp.get_var(var_name)?.clone();
            let mut current = dict_val;
            let keys: Vec<&str> = args[3..args.len() - 1]
                .iter()
                .map(|a| a.as_str())
                .collect();
            for key in &keys {
                let entries = parse_dict(&current)?;
                current = entries.get(*key).cloned().unwrap_or_default();
            }

            let entries = parse_dict(&current)?;

            // Save current variables and set dict keys as variables
            let saved: Vec<(String, Option<Value>)> = entries
                .keys()
                .map(|k| {
                    let old = interp.get_var(k).ok().cloned();
                    (k.clone(), old)
                })
                .collect();

            for (k, v) in &entries {
                interp.set_var(k, v.clone())?;
            }

            let result = interp.eval(body);

            // Read back variables and update dict
            let mut new_entries = IndexMap::with_capacity(entries.len());
            for k in entries.keys() {
                if let Ok(v) = interp.get_var(k) {
                    new_entries.insert(k.clone(), v.clone());
                }
            }

            // Restore saved variables
            for (k, old) in saved {
                if let Some(v) = old {
                    let _ = interp.set_var(&k, v);
                } else {
                    let _ = interp.unset_var(&k);
                }
            }

            if keys.is_empty() {
                interp.set_var(var_name, Value::from_dict_cached(new_entries))?;
            }

            result
        }

        // ── dict filter dictionary filterType ... ──────────────
        "filter" => {
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict filter",
                    5,
                    args.len(),
                    "dictionary filterType ...",
                ));
            }
            let filter_type = args[3].as_str();
            let entries = parse_dict(&args[2])?;

            match filter_type {
                "key" => {
                    let pattern = args[4].as_str();
                    let filtered: IndexMap<String, Value> = entries
                        .into_iter()
                        .filter(|(k, _)| glob_match(pattern, k))
                        .collect();
                    Ok(Value::from_dict_cached(filtered))
                }
                "value" => {
                    let pattern = args[4].as_str();
                    let filtered: IndexMap<String, Value> = entries
                        .into_iter()
                        .filter(|(_, v)| glob_match(pattern, v.as_str()))
                        .collect();
                    Ok(Value::from_dict_cached(filtered))
                }
                "script" => {
                    if args.len() < 6 {
                        return Err(Error::wrong_args_with_usage(
                            "dict filter",
                            6,
                            args.len(),
                            "dictionary script {keyVar valueVar} script",
                        ));
                    }
                    let var_list = args[4].as_list().unwrap_or_default();
                    if var_list.len() != 2 {
                        return Err(Error::runtime(
                            "must have exactly two variable names",
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                    let key_var = var_list[0].as_str().to_string();
                    let val_var = var_list[1].as_str().to_string();
                    let script = args[5].as_str();
                    let mut filtered = IndexMap::new();
                    for (k, v) in &entries {
                        interp.set_var(&key_var, Value::from_str(k))?;
                        interp.set_var(&val_var, v.clone())?;
                        match interp.eval(script) {
                            Ok(r) => {
                                if r.is_true() {
                                    filtered.insert(k.clone(), v.clone());
                                }
                            }
                            Err(e) => {
                                if e.is_break() {
                                    break;
                                }
                                if e.is_continue() {
                                    continue;
                                }
                                return Err(e);
                            }
                        }
                    }
                    Ok(Value::from_dict_cached(filtered))
                }
                _ => Err(Error::runtime(
                    format!(
                        "bad filterType \"{}\": must be key, value, or script",
                        filter_type
                    ),
                    crate::error::ErrorCode::InvalidOp,
                )),
            }
        }

        // ── dict map {keyVar valueVar} dictionary body ─────────
        "map" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict map",
                    5,
                    args.len(),
                    "{keyVar valueVar} dictionary body",
                ));
            }
            let var_list = args[2].as_list().unwrap_or_default();
            if var_list.len() != 2 {
                return Err(Error::runtime(
                    "must have exactly two variable names",
                    crate::error::ErrorCode::Generic,
                ));
            }
            let key_var = var_list[0].as_str().to_string();
            let val_var = var_list[1].as_str().to_string();
            let entries = parse_dict(&args[3])?;
            let body = args[4].as_str();
            let mut result_entries = IndexMap::with_capacity(entries.len());
            for (k, v) in &entries {
                interp.set_var(&key_var, Value::from_str(k))?;
                interp.set_var(&val_var, v.clone())?;
                match interp.eval(body) {
                    Ok(new_v) => {
                        result_entries.insert(k.clone(), new_v);
                    }
                    Err(e) => {
                        if e.is_break() {
                            break;
                        }
                        if e.is_continue() {
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Ok(Value::from_dict_cached(result_entries))
        }

        // ── dict info dictionary ───────────────────────────────
        "info" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "dict info",
                    3,
                    args.len(),
                    "dictionary",
                ));
            }
            let entries = parse_dict(&args[2])?;
            Ok(Value::from_str(&format!(
                "{} entries in dict",
                entries.len()
            )))
        }

        // ── dict getwithdefault dictionary ?key ...? key default
        "getwithdefault" => {
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict getwithdefault",
                    5,
                    args.len(),
                    "dictionary ?key ...? key default",
                ));
            }
            let default = &args[args.len() - 1];
            let mut current = args[2].clone();
            let keys = &args[3..args.len() - 1];
            for key in keys {
                let entries = parse_dict(&current)?;
                match entries.get(key.as_str()) {
                    Some(v) => current = v.clone(),
                    None => return Ok(default.clone()),
                }
            }
            Ok(current)
        }

        // ── dict update dictVariable key varName ?key varName ...? body
        "update" => {
            if args.len() < 5 || (args.len() - 3) % 2 == 0 {
                return Err(Error::wrong_args_with_usage(
                    "dict update",
                    5,
                    args.len(),
                    "dictVariable key varName ?key varName ...? body",
                ));
            }
            let var_name = args[2].as_str().to_string();
            let body = args[args.len() - 1].as_str().to_string();
            let pairs: Vec<(String, String)> = args[3..args.len() - 1]
                .chunks(2)
                .map(|c| (c[0].as_str().to_string(), c[1].as_str().to_string()))
                .collect();

            let dict_val = interp.get_var(&var_name).ok().cloned().unwrap_or_default();
            let entries = parse_dict(&dict_val).unwrap_or_default();

            // Set local variables from dict keys
            for (key, local_var) in &pairs {
                if let Some(v) = entries.get(key.as_str()) {
                    interp.set_var(local_var, v.clone())?;
                }
            }

            let result = interp.eval(&body);

            // Write back from local variables to dict
            if interp.get_var(&var_name).is_ok() {
                let cur_val = interp.get_var(&var_name).ok().cloned().unwrap_or_default();
                let mut new_entries = parse_dict(&cur_val).unwrap_or_default();
                for (key, local_var) in &pairs {
                    if let Ok(val) = interp.get_var(local_var) {
                        new_entries.insert(key.clone(), val.clone());
                    } else {
                        new_entries.shift_remove(key.as_str());
                    }
                }
                interp.set_var(&var_name, Value::from_dict_cached(new_entries))?;
            }

            result
        }

        // ── fallback: check for a proc named "dict $subcmd" ────
        _ => {
            let multi_name = format!("dict {}", subcmd);
            if let Some(proc_def) = interp.procs.get(&multi_name).cloned() {
                let mut new_args = vec![Value::from_str(&multi_name)];
                new_args.extend_from_slice(&args[2..]);
                return interp.call_proc(&proc_def, &new_args, &multi_name);
            }
            Err(Error::runtime(
                format!("unknown dict subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            ))
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────

fn dict_set_nested(
    mut entries: IndexMap<String, Value>,
    keys: &[&str],
    value: Value,
) -> Result<IndexMap<String, Value>> {
    if keys.len() == 1 {
        entries.insert(keys[0].to_string(), value);
    } else {
        let key = keys[0];
        let sub_val = entries.get(key).cloned().unwrap_or_default();
        let sub_entries = parse_dict(&sub_val).unwrap_or_default();
        let new_sub_entries = dict_set_nested(sub_entries, &keys[1..], value)?;
        entries.insert(key.to_string(), Value::from_dict_cached(new_sub_entries));
    }
    Ok(entries)
}

fn dict_unset_nested(entries: &mut IndexMap<String, Value>, keys: &[&str]) -> Result<()> {
    if keys.len() == 1 {
        entries.shift_remove(keys[0]);
    } else {
        let key = keys[0];
        if let Some(sub_val) = entries.get(key).cloned() {
            let mut sub_entries = parse_dict(&sub_val)?;
            dict_unset_nested(&mut sub_entries, &keys[1..])?;
            entries.insert(key.to_string(), Value::from_dict_cached(sub_entries));
        }
    }
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_dict_create_and_get() {
        let mut interp = Interp::new();
        let r = interp.eval("dict create a 1 b 2").unwrap();
        assert_eq!(interp.eval("dict get {a 1 b 2} a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict get {a 1 b 2} b").unwrap().as_str(), "2");
        // round-trip: dict create returns a dict, dict get reads it
        let r2 = interp
            .eval("dict get [dict create x 10 y 20] y")
            .unwrap();
        assert_eq!(r2.as_str(), "20");
        let _ = r;
    }

    #[test]
    fn test_dict_set_and_get() {
        let mut interp = Interp::new();
        interp.eval("dict set d name Jim").unwrap();
        let r = interp.eval("dict get $d name").unwrap();
        assert_eq!(r.as_str(), "Jim");
    }

    #[test]
    fn test_dict_exists() {
        let mut interp = Interp::new();
        let r = interp.eval("dict exists {a 1 b 2} a").unwrap();
        assert_eq!(r.as_str(), "1");
        let r2 = interp.eval("dict exists {a 1 b 2} c").unwrap();
        assert_eq!(r2.as_str(), "0");
    }

    #[test]
    fn test_dict_keys_values() {
        let mut interp = Interp::new();
        let r = interp.eval("dict keys {a 1 b 2 c 3}").unwrap();
        assert_eq!(r.as_str(), "a b c");
        let r2 = interp.eval("dict values {a 1 b 2 c 3}").unwrap();
        assert_eq!(r2.as_str(), "1 2 3");
    }

    #[test]
    fn test_dict_size() {
        let mut interp = Interp::new();
        let r = interp.eval("dict size {a 1 b 2 c 3}").unwrap();
        assert_eq!(r.as_str(), "3");
    }

    #[test]
    fn test_dict_remove() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict get [dict remove {a 1 b 2 c 3} b] a")
            .unwrap();
        assert_eq!(r.as_str(), "1");
        let r2 = interp
            .eval("dict size [dict remove {a 1 b 2 c 3} b]")
            .unwrap();
        assert_eq!(r2.as_str(), "2");
    }

    #[test]
    fn test_dict_merge() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict get [dict merge {a 1 b 2} {c 3 d 4}] c")
            .unwrap();
        assert_eq!(r.as_str(), "3");
    }

    #[test]
    fn test_dict_replace() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict get [dict replace {a 1 b 2} b 99] b")
            .unwrap();
        assert_eq!(r.as_str(), "99");
    }

    #[test]
    fn test_dict_incr() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create count 5]").unwrap();
        interp.eval("dict incr d count").unwrap();
        let r = interp.eval("dict get $d count").unwrap();
        assert_eq!(r.as_str(), "6");
    }

    #[test]
    fn test_dict_for_basic() {
        let mut interp = Interp::new();
        interp.eval("set result {}").unwrap();
        interp
            .eval("dict for {k v} {a 1 b 2} { lappend result $k=$v }")
            .unwrap();
        let r = interp.eval("set result").unwrap();
        assert_eq!(r.as_str(), "a=1 b=2");
    }

    #[test]
    fn test_dict_getwithdefault_found() {
        let mut interp = Interp::new();
        let r = interp
            .eval(r#"dict getwithdefault {a 1 b 2} a "default""#)
            .unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_dict_getwithdefault_not_found() {
        let mut interp = Interp::new();
        let r = interp
            .eval(r#"dict getwithdefault {a 1 b 2} c "default""#)
            .unwrap();
        assert_eq!(r.as_str(), "default");
    }

    #[test]
    fn test_dict_getwithdefault_nested() {
        let mut interp = Interp::new();
        let r = interp
            .eval(r#"dict getwithdefault {a {x 10 y 20} b 2} a y "nope""#)
            .unwrap();
        assert_eq!(r.as_str(), "20");
    }

    #[test]
    fn test_dict_update_basic() {
        let mut interp = Interp::new();
        interp
            .eval(r#"set d [dict create name "Jim" age 30]"#)
            .unwrap();
        interp
            .eval(r#"dict update d name n age a { set n "Updated"; set a 31 }"#)
            .unwrap();
        let r2 = interp.eval("dict get $d name").unwrap();
        assert_eq!(r2.as_str(), "Updated");
        let r3 = interp.eval("dict get $d age").unwrap();
        assert_eq!(r3.as_str(), "31");
    }

    #[test]
    fn test_dict_update_return_body() {
        let mut interp = Interp::new();
        interp.eval(r#"set d {x 10}"#).unwrap();
        let r = interp
            .eval(r#"dict update d x v { expr {$v + 5} }"#)
            .unwrap();
        assert_eq!(r.as_str(), "15");
    }

    #[test]
    fn test_dict_getdef_tcl() {
        let mut interp = Interp::new();
        let r = interp
            .eval(r#"dict getdef {a 1 b 2} c "fallback""#)
            .unwrap();
        assert_eq!(r.as_str(), "fallback");
        let r2 = interp
            .eval(r#"dict getdef {a 1 b 2} a "fallback""#)
            .unwrap();
        assert_eq!(r2.as_str(), "1");
    }

    #[test]
    fn test_dict_filter_key() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict keys [dict filter {abc 1 abd 2 xyz 3} key ab*]")
            .unwrap();
        assert_eq!(r.as_str(), "abc abd");
    }

    #[test]
    fn test_dict_map_basic() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict get [dict map {k v} {a 1 b 2} { expr {$v * 10} }] b")
            .unwrap();
        assert_eq!(r.as_str(), "20");
    }

    #[test]
    fn test_dict_preserves_insertion_order() {
        let mut interp = Interp::new();
        let r = interp.eval("dict keys [dict create z 1 a 2 m 3]").unwrap();
        assert_eq!(r.as_str(), "z a m");
    }

    #[test]
    fn test_dict_nested_set_get() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create]").unwrap();
        interp.eval("dict set d a b 42").unwrap();
        let r = interp.eval("dict get $d a b").unwrap();
        assert_eq!(r.as_str(), "42");
    }
}
