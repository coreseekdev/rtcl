//! Dict commands: dict create/get/set/exists/unset/keys/values/size/for/merge/replace etc.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;

/// Parse a string as a dict (flat key-value list).
fn parse_dict(s: &str) -> Result<Vec<(String, String)>> {
    let list = Value::from_str(s).as_list().unwrap_or_default();
    if list.len() % 2 != 0 {
        return Err(Error::runtime(
            "missing value to go with key",
            crate::error::ErrorCode::InvalidOp,
        ));
    }
    Ok(list
        .chunks(2)
        .map(|c| (c[0].as_str().to_string(), c[1].as_str().to_string()))
        .collect())
}

/// Serialise dict entries back to a flat Tcl list string.
fn dict_to_string(entries: &[(String, String)]) -> String {
    let vals: Vec<Value> = entries
        .iter()
        .flat_map(|(k, v)| vec![Value::from_str(k), Value::from_str(v)])
        .collect();
    Value::from_list(&vals).as_str().to_string()
}

pub fn cmd_dict(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("dict", 2, args.len()));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        "create" => {
            if (args.len() - 2) % 2 != 0 {
                return Err(Error::runtime(
                    "wrong # args: dict create requires key value pairs",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            let entries: Vec<(String, String)> = args[2..]
                .chunks(2)
                .map(|c| (c[0].as_str().to_string(), c[1].as_str().to_string()))
                .collect();
            Ok(Value::from_str(&dict_to_string(&entries)))
        }
        "get" => {
            if args.len() < 3 {
                return Err(Error::wrong_args("dict get", 3, args.len()));
            }
            let entries = parse_dict(args[2].as_str())?;
            if args.len() == 3 {
                return Ok(args[2].clone());
            }
            let key = args[3].as_str();
            for (k, v) in &entries {
                if k == key {
                    if args.len() > 4 {
                        let sub_entries = parse_dict(v)?;
                        let sub_key = args[4].as_str();
                        for (sk, sv) in &sub_entries {
                            if sk == sub_key {
                                return Ok(Value::from_str(sv));
                            }
                        }
                        return Err(Error::runtime(
                            format!("key \"{}\" not known in dictionary", sub_key),
                            crate::error::ErrorCode::NotFound,
                        ));
                    }
                    return Ok(Value::from_str(v));
                }
            }
            Err(Error::runtime(
                format!("key \"{}\" not known in dictionary", key),
                crate::error::ErrorCode::NotFound,
            ))
        }
        "set" => {
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage("dict set", 5, args.len(), "dictVariable key ?key ...? value"));
            }
            let var_name = args[2].as_str();
            let value = args[args.len() - 1].as_str().to_string();
            let keys: Vec<&str> = args[3..args.len() - 1].iter().map(|v| v.as_str()).collect();

            let dict_str = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
            let result = dict_set_nested(&dict_str, &keys, &value)?;
            let result_val = Value::from_str(&result);
            interp.set_var(var_name, result_val.clone())
        }
        "unset" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage("dict unset", 4, args.len(), "dictVariable key ?key ...?"));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str();
            let dict_str = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
            let mut entries = parse_dict(&dict_str)?;
            entries.retain(|(k, _)| k != key);
            let result = Value::from_str(&dict_to_string(&entries));
            interp.set_var(var_name, result.clone())
        }
        "exists" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("dict exists", 4, args.len()));
            }
            let entries = parse_dict(args[2].as_str())?;
            let key = args[3].as_str();
            let exists = entries.iter().any(|(k, _)| k == key);
            Ok(Value::from_bool(exists))
        }
        "keys" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(Error::wrong_args("dict keys", 3, args.len()));
            }
            let entries = parse_dict(args[2].as_str())?;
            let pattern = if args.len() == 4 { Some(args[3].as_str()) } else { None };
            let keys: Vec<Value> = entries
                .iter()
                .filter(|(k, _)| pattern.is_none() || glob_match(pattern.unwrap(), k))
                .map(|(k, _)| Value::from_str(k))
                .collect();
            Ok(Value::from_list(&keys))
        }
        "values" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(Error::wrong_args("dict values", 3, args.len()));
            }
            let entries = parse_dict(args[2].as_str())?;
            let pattern = if args.len() == 4 { Some(args[3].as_str()) } else { None };
            let values: Vec<Value> = entries
                .iter()
                .filter(|(_, v)| pattern.is_none() || glob_match(pattern.unwrap(), v))
                .map(|(_, v)| Value::from_str(v))
                .collect();
            Ok(Value::from_list(&values))
        }
        "size" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("dict size", 3, args.len()));
            }
            let entries = parse_dict(args[2].as_str())?;
            Ok(Value::from_int(entries.len() as i64))
        }
        "for" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage("dict for", 5, args.len(), "{keyVar valueVar} dictionary body"));
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
            let entries = parse_dict(args[3].as_str())?;
            let body = args[4].as_str();
            let mut result = Value::empty();
            for (k, v) in &entries {
                interp.set_var(&key_var, Value::from_str(k))?;
                interp.set_var(&val_var, Value::from_str(v))?;
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
        "merge" => {
            let mut entries: Vec<(String, String)> = Vec::new();
            for i in 2..args.len() {
                let new_entries = parse_dict(args[i].as_str())?;
                for (k, v) in new_entries {
                    if let Some(pos) = entries.iter().position(|(ek, _)| *ek == k) {
                        entries[pos].1 = v;
                    } else {
                        entries.push((k, v));
                    }
                }
            }
            Ok(Value::from_str(&dict_to_string(&entries)))
        }
        "replace" => {
            if args.len() < 3 {
                return Err(Error::wrong_args("dict replace", 3, args.len()));
            }
            let mut entries = parse_dict(args[2].as_str())?;
            let pairs: Vec<&Value> = args[3..].iter().collect();
            if pairs.len() % 2 != 0 {
                return Err(Error::runtime(
                    "wrong # args: must be key value pairs",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            for chunk in pairs.chunks(2) {
                let k = chunk[0].as_str().to_string();
                let v = chunk[1].as_str().to_string();
                if let Some(pos) = entries.iter().position(|(ek, _)| *ek == k) {
                    entries[pos].1 = v;
                } else {
                    entries.push((k, v));
                }
            }
            Ok(Value::from_str(&dict_to_string(&entries)))
        }
        "append" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("dict append", 4, args.len()));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str().to_string();
            let append_val = if args.len() > 4 { args[4].as_str() } else { "" };
            let dict_str = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
            let mut entries = parse_dict(&dict_str)?;
            if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
                entries[pos].1.push_str(append_val);
            } else {
                entries.push((key, append_val.to_string()));
            }
            let result = Value::from_str(&dict_to_string(&entries));
            interp.set_var(var_name, result.clone())
        }
        "incr" => {
            if args.len() < 4 || args.len() > 5 {
                return Err(Error::wrong_args_with_usage("dict incr", 4, args.len(), "dictVariable key ?increment?"));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str().to_string();
            let incr = if args.len() == 5 { args[4].as_int().unwrap_or(1) } else { 1 };
            let dict_str = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
            let mut entries = parse_dict(&dict_str)?;
            if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
                let cur: i64 = entries[pos].1.parse().unwrap_or(0);
                entries[pos].1 = (cur + incr).to_string();
            } else {
                entries.push((key, incr.to_string()));
            }
            let result = Value::from_str(&dict_to_string(&entries));
            interp.set_var(var_name, result.clone())
        }
        "lappend" => {
            // dict lappend dictVariable key ?value ...?
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage("dict lappend", 4, args.len(), "dictVariable key ?value ...?"));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str().to_string();
            let dict_str = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
            let mut entries = parse_dict(&dict_str)?;
            let cur_val = entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()).unwrap_or_default();
            let mut list = if cur_val.is_empty() {
                Vec::new()
            } else {
                Value::from_str(&cur_val).as_list().unwrap_or_default()
            };
            for v in &args[4..] {
                list.push(v.clone());
            }
            let new_val = Value::from_list(&list);
            if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
                entries[pos].1 = new_val.as_str().to_string();
            } else {
                entries.push((key, new_val.as_str().to_string()));
            }
            let result = Value::from_str(&dict_to_string(&entries));
            interp.set_var(var_name, result.clone())
        }
        "remove" => {
            // dict remove dict ?key ...?
            if args.len() < 3 {
                return Err(Error::wrong_args_with_usage("dict remove", 3, args.len(), "dictionary ?key ...?"));
            }
            let dict_str = args[2].as_str();
            let mut entries = parse_dict(dict_str)?;
            for key_arg in &args[3..] {
                let key = key_arg.as_str();
                entries.retain(|(k, _)| k != key);
            }
            Ok(Value::from_str(&dict_to_string(&entries)))
        }
        "with" => {
            // dict with dictVariable ?key ...? body
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage("dict with", 4, args.len(), "dictVariable ?key ...? body"));
            }
            let var_name = args[2].as_str();
            let body = args[args.len() - 1].as_str();

            // Navigate to nested dict if keys provided
            let dict_val = interp.get_var(var_name)?.clone();
            let mut dict_str = dict_val.as_str().to_string();

            let keys: Vec<&str> = args[3..args.len() - 1].iter().map(|a| a.as_str()).collect();
            for key in &keys {
                let entries = parse_dict(&dict_str)?;
                dict_str = entries.iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
            }

            let entries = parse_dict(&dict_str)?;

            // Set dict keys as variables
            let saved: Vec<(String, Option<Value>)> = entries.iter().map(|(k, _)| {
                let old = interp.get_var(k).ok().cloned();
                (k.clone(), old)
            }).collect();

            for (k, v) in &entries {
                interp.set_var(k, Value::from_str(v))?;
            }

            // Execute body
            let result = interp.eval(body);

            // Read back variables and update dict
            let mut new_entries: Vec<(String, String)> = Vec::new();
            for (k, _) in &entries {
                if let Ok(v) = interp.get_var(k) {
                    new_entries.push((k.clone(), v.as_str().to_string()));
                }
            }

            // Restore saved variables
            for (k, old) in saved {
                if let Some(v) = old {
                    let _ = interp.set_var(&k, v);
                } else {
                    interp.vars.remove(&k);
                }
            }

            // Update the dict variable
            let new_dict = dict_to_string(&new_entries);
            if keys.is_empty() {
                interp.set_var(var_name, Value::from_str(&new_dict))?;
            }
            // TODO: handle nested key path update

            result
        }
        "filter" => {
            // dict filter dict key pattern
            // dict filter dict value pattern
            // dict filter dict script {keyVar valueVar} script
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage("dict filter", 5, args.len(), "dictionary filterType ..."));
            }
            let dict_str = args[2].as_str();
            let filter_type = args[3].as_str();
            let entries = parse_dict(dict_str)?;

            match filter_type {
                "key" => {
                    let pattern = args[4].as_str();
                    let filtered: Vec<(String, String)> = entries.into_iter()
                        .filter(|(k, _)| glob_match(pattern, k))
                        .collect();
                    Ok(Value::from_str(&dict_to_string(&filtered)))
                }
                "value" => {
                    let pattern = args[4].as_str();
                    let filtered: Vec<(String, String)> = entries.into_iter()
                        .filter(|(_, v)| glob_match(pattern, v))
                        .collect();
                    Ok(Value::from_str(&dict_to_string(&filtered)))
                }
                "script" => {
                    if args.len() < 6 {
                        return Err(Error::wrong_args_with_usage("dict filter", 6, args.len(), "dictionary script {keyVar valueVar} script"));
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
                    let mut filtered = Vec::new();
                    for (k, v) in &entries {
                        interp.set_var(&key_var, Value::from_str(k))?;
                        interp.set_var(&val_var, Value::from_str(v))?;
                        let result = interp.eval(script)?;
                        if result.is_true() {
                            filtered.push((k.clone(), v.clone()));
                        }
                    }
                    Ok(Value::from_str(&dict_to_string(&filtered)))
                }
                _ => Err(Error::runtime(
                    format!("unknown filter type \"{}\": must be key, value, or script", filter_type),
                    crate::error::ErrorCode::Generic,
                )),
            }
        }
        "map" => {
            // dict map {keyVar valueVar} dictionary body
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage("dict map", 5, args.len(), "{keyVar valueVar} dictionary body"));
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
            let dict_str = args[3].as_str();
            let body = args[4].as_str();
            let entries = parse_dict(dict_str)?;
            let mut result_entries = Vec::new();
            for (k, v) in &entries {
                interp.set_var(&key_var, Value::from_str(k))?;
                interp.set_var(&val_var, Value::from_str(v))?;
                match interp.eval(body) {
                    Ok(new_v) => {
                        result_entries.push((k.clone(), new_v.as_str().to_string()));
                    }
                    Err(e) => {
                        if e.is_break() { break; }
                        if e.is_continue() { continue; }
                        return Err(e);
                    }
                }
            }
            Ok(Value::from_str(&dict_to_string(&result_entries)))
        }
        "info" => {
            // dict info dictionary — return diagnostic info
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage("dict info", 3, args.len(), "dictionary"));
            }
            let dict_str = args[2].as_str();
            let entries = parse_dict(dict_str)?;
            Ok(Value::from_str(&format!("{} entries in dict", entries.len())))
        }
        _ => Err(Error::runtime(
            format!("unknown dict subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

fn dict_set_nested(dict_str: &str, keys: &[&str], value: &str) -> Result<String> {
    let mut entries = parse_dict(dict_str)?;
    if keys.len() == 1 {
        let key = keys[0].to_string();
        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
            entries[pos].1 = value.to_string();
        } else {
            entries.push((key, value.to_string()));
        }
    } else {
        let key = keys[0].to_string();
        let sub_dict = entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let new_sub = dict_set_nested(&sub_dict, &keys[1..], value)?;
        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
            entries[pos].1 = new_sub;
        } else {
            entries.push((key, new_sub));
        }
    }
    Ok(dict_to_string(&entries))
}
