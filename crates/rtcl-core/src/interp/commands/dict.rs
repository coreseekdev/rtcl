//! Dict commands: dict create/get/set/exists/unset/keys/values/size/for/merge/replace etc.
//!
//! Performance notes:
//! - Read-only operations (get, exists, keys, values, size, info, getwithdefault)
//!   use `borrow_dict()` which returns `Cow<DictMap>` — zero-copy when the value
//!   already has a Dict internal rep.
//! - Mutating operations (set, unset, append, incr, lappend) clone on demand.
//! - `dict create` supports `-ordered` (default) and `-unordered` flags.

use std::borrow::Cow;

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::{DictMap, Value};

/// Parse a value as a dict, returning an owned DictMap (for mutation).
/// Apply `dict with` write-back updates at the end of a (possibly nested)
/// key path: descend `path` through nested dicts, then set each mapped key
/// to the variable's present value (or remove keys whose variable was
/// unset).
fn dict_with_writeback(
    interp: &Interp,
    val: &Value,
    path: &[&str],
    mapped: &DictMap,
) -> Result<Value> {
    let mut entries = parse_dict_raw(val).map_err(|e| super::list::tcl_err(e.msg))?;
    if path.is_empty() {
        for k in mapped.keys() {
            match interp.get_var(k) {
                Ok(v) => {
                    entries.insert(k.clone(), v.clone());
                }
                Err(_) => {
                    entries.shift_remove(k);
                }
            }
        }
    } else {
        let child = entries.get(path[0]).cloned().unwrap_or_default();
        let new_child = dict_with_writeback(interp, &child, &path[1..], mapped)?;
        entries.insert(path[0].to_string(), new_child);
    }
    Ok(Value::from_dict_cached(entries))
}

/// A failed dict parse: Tcl's message plus the `::errorCode` tail that
/// tclDictObj.c raises with it.
struct DictParseFail {
    msg: String,
    code: &'static str,
}

/// Parse without touching the interp: used by nested re-parses whose
/// error paths only need the message.
fn parse_dict_raw(val: &Value) -> std::result::Result<DictMap, DictParseFail> {
    match val.as_dict() {
        Some(m) => Ok(m),
        None => {
            // Strict re-parse so malformed strings report the dict-specific
            // texts ("dict element in braces followed by ...") and
            // DICTIONARY errorCodes instead of the list ones.
            let list = crate::value::parse_dict_full(&val.to_str()).map_err(|e| DictParseFail {
                msg: e.message,
                code: e.code,
            })?;
            if list.len() % 2 != 0 {
                return Err(DictParseFail {
                    msg: "missing value to go with key".to_string(),
                    code: "TCL VALUE DICTIONARY",
                });
            }
            let mut map = DictMap::ordered_with_capacity(list.len() / 2);
            for c in list.chunks(2) {
                map.insert(c[0].as_str().to_string(), c[1].clone());
            }
            Ok(map)
        }
    }
}

/// Parse a value as a dict, mirroring Tcl's `::errorCode` on failure
/// (raise-time global plus the `-errorcode` carried in the error itself
/// so `catch ... -> opt` reports it, as `error msg info code` does).
fn parse_dict(interp: &mut Interp, val: &Value) -> Result<DictMap> {
    parse_dict_raw(val).map_err(|e| {
        super::list::set_error_code(interp, e.code);
        Error::ControlFlow {
            kind: crate::error::ControlFlow::Error,
            value: Some(Value::from_str(&e.msg)),
            level: 1,
            error_info: None,
            error_code: Some(e.code.to_string()),
        }
    })
}

/// Borrow a value as a dict without cloning when possible.
/// Returns `Cow::Borrowed` for zero-copy access to cached dicts.
fn borrow_dict<'a>(interp: &mut Interp, val: &'a Value) -> Result<Cow<'a, DictMap>> {
    match val.as_dict_cow() {
        Some(cow) => Ok(cow),
        None => Ok(std::borrow::Cow::Owned(parse_dict(interp, val)?)),
    }
}

pub fn cmd_dict(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("dict", 2, args.len(), "subcommand ?arg ...?"));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        // ── dict create ?-ordered|-unordered? ?key value ...? ──
        "create" => {
            let mut start = 2;
            let mut ordered = true;
            if args.len() > 2 {
                match args[2].as_str() {
                    "-unordered" => {
                        ordered = false;
                        start = 3;
                    }
                    "-ordered" => {
                        start = 3;
                    }
                    _ => {}
                }
            }
            if (args.len() - start) % 2 != 0 {
                return Err(Error::wrong_args_with_usage(
                    "dict create",
                    2,
                    args.len(),
                    "?key value ...?",
                ));
            }
            let mut entries = if ordered {
                DictMap::ordered_with_capacity((args.len() - start) / 2)
            } else {
                DictMap::unordered_with_capacity((args.len() - start) / 2)
            };
            for c in args[start..].chunks(2) {
                entries.insert(c[0].as_str().to_string(), c[1].clone());
            }
            Ok(Value::from_dict_cached(entries))
        }

        // ── dict get dictionary ?key ...? ──────────────────────
        "get" => {
            if args.len() < 3 {
                return Err(Error::wrong_args_with_usage(
                    "dict get", 3, args.len(), "dictionary ?key ...?",
                ));
            }
            if args.len() == 3 {
                return Ok(args[2].clone());
            }
            // Single-key fast path: zero-copy via borrow
            if args.len() == 4 {
                let entries = borrow_dict(interp, &args[2])?;
                let key = args[3].as_str();
                return match entries.get(key) {
                    Some(v) => Ok(v.clone()),
                    None => Err(Error::runtime(
                        format!("key \"{}\" not known in dictionary", key),
                        crate::error::ErrorCode::NotFound,
                    )),
                };
            }
            // Multi-key: must clone intermediate dicts
            let mut current = args[2].clone();
            for key_arg in &args[3..] {
                let key = key_arg.as_str();
                let entries = borrow_dict(interp, &current)?;
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
                    "dictVarName key ?key ...? value",
                ));
            }
            let var_name = args[2].as_str();
            // Fast path: single-key `dict set var k v` whose variable
            // already holds a dict rep — take the value out of its slot
            // (sole owner), insert in place through the COW rep, put it
            // back.  Amortised O(1); multi-key nesting, creation,
            // string-rep sources, links and traced variables keep the
            // rebuilding path below.
            if args.len() == 5
                && interp
                    .get_var(var_name)
                    .is_ok_and(|v| v.as_dict_ref().is_some())
            {
                if let Some(mut v) = interp.take_var_fast(var_name) {
                    if let Some(map) = v.as_dict_mut() {
                        map.insert(args[3].as_str().to_string(), args[4].clone());
                        return interp.set_var(var_name, v);
                    }
                    // Unreachable (rep checked before the take); restore
                    // raw and rebuild through the slow path.
                    interp.store_var(var_name, v);
                }
            }
            let value = args[args.len() - 1].clone();
            let keys: Vec<&str> = args[3..args.len() - 1]
                .iter()
                .map(|v| v.as_str())
                .collect();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let entries = parse_dict(interp, &dict_val)?;
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
                    "dictVarName key ?key ...?",
                ));
            }
            let var_name = args[2].as_str();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(interp, &dict_val)?;
            if args.len() == 4 {
                entries.shift_remove(args[3].as_str());
            } else {
                let keys: Vec<&str> = args[3..].iter().map(|v| v.as_str()).collect();
                dict_unset_nested(&mut entries, &keys)?;
            }
            let result = Value::from_dict_cached(entries);
            interp.set_var(var_name, result)
        }

        // ── dict exists dictionary key ?key ...? ───────────────
        "exists" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict exists", 4, args.len(), "dictionary key ?key ...?",
                ));
            }
            // Single-key fast path: zero-copy
            if args.len() == 4 {
                let entries = borrow_dict(interp, &args[2]);
                let key = args[3].as_str();
                return Ok(Value::from_bool(
                    entries.map_or(false, |e| e.contains_key(key)),
                ));
            }
            // Multi-key: walk nested dicts
            let mut current = args[2].clone();
            for key_arg in &args[3..] {
                let key = key_arg.as_str();
                match current.as_dict_cow() {
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
                return Err(Error::wrong_args_with_usage(
                    "dict keys", 3, args.len(), "dictionary ?pattern?",
                ));
            }
            let entries = borrow_dict(interp, &args[2])?;
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
            Ok(Value::from_list_cached(keys))
        }

        // ── dict values dictionary ?pattern? ───────────────────
        "values" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict values", 3, args.len(), "dictionary ?pattern?",
                ));
            }
            let entries = borrow_dict(interp, &args[2])?;
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
            Ok(Value::from_list_cached(values))
        }

        // ── dict size dictionary ───────────────────────────────
        "size" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "dict size", 3, args.len(), "dictionary",
                ));
            }
            let entries = borrow_dict(interp, &args[2])?;
            Ok(Value::from_int(entries.len() as i64))
        }

        // ── dict for {keyVar valueVar} dictionary body ─────────
        "for" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "dict for",
                    5,
                    args.len(),
                    "{keyVarName valueVarName} dictionary script",
                ));
            }
            let var_list = super::list::strict_list(interp, &args[2])?;
            if var_list.len() != 2 {
                return Err(Error::runtime(
                    "must have exactly two variable names",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            let key_var = var_list[0].as_str().to_string();
            let val_var = var_list[1].as_str().to_string();
            // Clone the dict: body evaluation may modify variables
            let entries = parse_dict(interp, &args[3])?;
            let body = args[4].as_str();
            let mut result = Value::empty();
            for (k, v) in &entries {
                interp.set_var(&key_var, Value::from_str(k))?;
                interp.set_var(&val_var, v.clone())?;
                match demote_level0_return(interp.eval(body)) {
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
            // tclsh seeds the result with a duplicate of the first
            // dictionary; the duplicate only becomes observable (keeps the
            // original string representation, spaces and all) when no
            // later dict inserts anything. Any non-empty later dict — even
            // one whose entries duplicate existing pairs (dict-20.23) —
            // invalidates the string rep and forces a rebuild.
            let first = if args.len() > 2 {
                Some(parse_dict(interp, &args[2])?)
            } else {
                None
            };
            let mut entries = first.clone().unwrap_or_else(DictMap::ordered);
            let mut touched = false;
            for arg in args.get(3..).unwrap_or(&[]) {
                let new_entries = parse_dict(interp, arg)?;
                touched = touched || !new_entries.is_empty();
                entries.extend(new_entries);
            }
            if first.is_some() && !touched {
                return Ok(args[2].clone());
            }
            Ok(Value::from_dict_cached(entries))
        }

        // ── dict replace dictionary ?key value ...? ────────────
        "replace" => {
            if args.len() < 3 {
                return Err(Error::wrong_args_with_usage(
                    "dict replace", 3, args.len(), "dictionary ?key value ...?",
                ));
            }
            if (args.len() - 3) % 2 != 0 {
                return Err(Error::wrong_args_with_usage(
                    "dict replace",
                    3,
                    args.len(),
                    "dictionary ?key value ...?",
                ));
            }
            let mut entries = parse_dict(interp, &args[2])?;
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
                    "dictVarName key ?value ...?",
                ));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(interp, &dict_val)?;
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
            let mut entries = parse_dict(interp, &dict_val)?;
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
                    "dictVarName key ?value ...?",
                ));
            }
            let var_name = args[2].as_str();
            let key = args[3].as_str();
            let dict_val = interp.get_var(var_name).ok().cloned().unwrap_or_default();
            let mut entries = parse_dict(interp, &dict_val)?;
            let cur_val = entries.get(key).cloned().unwrap_or_default();
            let mut list = if cur_val.is_empty() {
                Vec::new()
            } else {
                cur_val.as_list().unwrap_or_default()
            };
            for v in &args[4..] {
                list.push(v.clone());
            }
            let new_val = Value::from_list_cached(list);
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
            let mut entries = parse_dict(interp, &args[2])?;
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

            let mut current = interp.get_var(var_name)?.clone();
            let keys: Vec<&str> = args[3..args.len() - 1]
                .iter()
                .map(|a| a.as_str())
                .collect();
            for key in &keys {
                let entries = parse_dict(interp, &current)?;
                match entries.get(key) {
                    Some(v) => current = v.clone(),
                    None => {
                        return Err(Error::runtime(
                            format!("key \"{}\" not known in dictionary", key),
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                }
            }

            let entries = parse_dict(interp, &current)?;

            for (k, v) in &entries {
                interp.set_var(k, v.clone())?;
            }

            let result = demote_level0_return(interp.eval(body));
            // tclsh leaves the mapped variables in place on error and does
            // not write the dict back; break/continue still write back.
            match &result {
                Err(Error::ControlFlow { kind, .. }) => {
                    if *kind == crate::error::ControlFlow::Error {
                        return result;
                    }
                }
                Err(_) => return result,
                Ok(_) => {}
            }

            // Write-back updates the variable's CURRENT dict (mid-body
            // `dict set d ...` survives, dict-22.12): mapped keys take the
            // variables' present values, deleted variables remove their
            // keys. A variable deleted mid-body stays deleted. Nested key
            // paths write back through the chain (dict-22.16).
            if let Ok(var_now) = interp.get_var(var_name) {
                let fresh = dict_with_writeback(interp, var_now, &keys, &entries)?;
                interp.set_var(var_name, fresh)?;
            }

            result
        }

        // ── dict filter dictionary filterType ... ──────────────
        "filter" => {
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "dict filter",
                    4,
                    args.len(),
                    "dictionary filterType ?arg ...?",
                ));
            }
            // The dictionary string is parsed before the filter type is
            // examined (dict-17.5: odd list beats everything else).
            let entries = parse_dict(interp, &args[2])?;
            let ordered = entries.is_ordered();
            let filter_type = args[3].as_str();

            match filter_type {
                "key" => {
                    // Zero or more patterns; a key matching ANY is kept
                    // (dict-17.4: no patterns -> empty result).
                    let patterns: Vec<String> =
                        args[4..].iter().map(|a| a.as_str().to_string()).collect();
                    let filtered = DictMap::from_iter_with_order(
                        ordered,
                        entries
                            .into_iter()
                            .filter(|(k, _)| patterns.iter().any(|p| glob_match(p, k))),
                    );
                    Ok(Value::from_dict_cached(filtered))
                }
                "value" => {
                    let patterns: Vec<String> =
                        args[4..].iter().map(|a| a.as_str().to_string()).collect();
                    let filtered = DictMap::from_iter_with_order(
                        ordered,
                        entries.into_iter().filter(|(_, v)| {
                            patterns.iter().any(|p| glob_match(p, v.as_str()))
                        }),
                    );
                    Ok(Value::from_dict_cached(filtered))
                }
                "script" => {
                    if args.len() < 6 {
                        return Err(Error::wrong_args_with_usage(
                            "dict filter",
                            6,
                            args.len(),
                            "dictionary script {keyVarName valueVarName} filterScript",
                        ));
                    }
                    let var_list =
                        super::list::strict_list(interp, &args[4])?;
                    if var_list.len() != 2 {
                        return Err(Error::runtime(
                            "must have exactly two variable names",
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                    let key_var = var_list[0].as_str().to_string();
                    let val_var = var_list[1].as_str().to_string();
                    let script = args[5].as_str();
                    let mut filtered = entries.empty_like(0);
                    for (k, v) in &entries {
                        interp.set_var(&key_var, Value::from_str(k))?;
                        interp.set_var(&val_var, v.clone())?;
                        match demote_level0_return(interp.eval(script)) {
                            Ok(r) => {
                                // tclsh requires a boolean body result
                                // ("expected boolean value but got ...").
                                if crate::types::expr_funcs::strict_bool(&r)? {
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
                        "bad filterType \"{}\": must be key, script, or value",
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
                    "{keyVarName valueVarName} dictionary script",
                ));
            }
            let var_list = super::list::strict_list(interp, &args[2])?;
            if var_list.len() != 2 {
                return Err(Error::runtime(
                    "must have exactly two variable names",
                    crate::error::ErrorCode::Generic,
                ));
            }
            let key_var = var_list[0].as_str().to_string();
            let val_var = var_list[1].as_str().to_string();
            let entries = parse_dict(interp, &args[3])?;
            let body = args[4].as_str();
            let mut result_entries = entries.empty_like(entries.len());
            for (k, v) in &entries {
                interp.set_var(&key_var, Value::from_str(k))?;
                interp.set_var(&val_var, v.clone())?;
                match demote_level0_return(interp.eval(body)) {
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
            let entries = borrow_dict(interp, &args[2])?;
            let kind = if entries.is_ordered() { "ordered" } else { "unordered" };
            Ok(Value::from_str(&format!(
                "{} entries in {} dict",
                entries.len(),
                kind,
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
                let entries = borrow_dict(interp, &current)?;
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
            let entries = parse_dict(interp, &dict_val)?;

            for (key, local_var) in &pairs {
                if let Some(v) = entries.get(key.as_str()) {
                    interp.set_var(local_var, v.clone())?;
                }
            }

            let result = interp.eval(&body);

            if interp.get_var(&var_name).is_ok() {
                let cur_val = interp.get_var(&var_name).ok().cloned().unwrap_or_default();
                let mut new_entries = parse_dict(interp, &cur_val)?;
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
                return interp.call_proc(&proc_def, &new_args, &multi_name, None);
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
    mut entries: DictMap,
    keys: &[&str],
    value: Value,
) -> Result<DictMap> {
    if keys.len() == 1 {
        entries.insert(keys[0].to_string(), value);
    } else {
        let key = keys[0];
        let sub_val = entries.get(key).cloned().unwrap_or_default();
        let sub_entries = parse_dict_raw(&sub_val).map_err(|e| super::list::tcl_err(e.msg))?;
        let new_sub_entries = dict_set_nested(sub_entries, &keys[1..], value)?;
        entries.insert(key.to_string(), Value::from_dict_cached(new_sub_entries));
    }
    Ok(entries)
}

fn dict_unset_nested(entries: &mut DictMap, keys: &[&str]) -> Result<()> {
    if keys.len() == 1 {
        entries.shift_remove(keys[0]);
    } else {
        let key = keys[0];
        // A missing key along the path is an error ("key "c" not known in
        // dictionary"); a present key whose value is not a dict fails in
        // the recursive parse below (dict-16.14/16.15).
        let sub_val = entries.get(key).cloned().ok_or_else(|| not_known(key))?;
        let mut sub_entries = parse_dict_raw(&sub_val).map_err(|e| super::list::tcl_err(e.msg))?;
        dict_unset_nested(&mut sub_entries, &keys[1..])?;
        entries.insert(key.to_string(), Value::from_dict_cached(sub_entries));
    }
    Ok(())
}

/// A `return -level 0` ends the current *script* with its value: command
/// bodies see a normal completion carrying that value (tclsh converts
/// TCL_RETURN with level 0 at the script boundary; cmd_return encodes
/// explicit `-level 0` as level −1). Plain `return` and `-level N ≥ 1`
/// keep propagating.
pub(crate) fn demote_level0_return(r: Result<Value>) -> Result<Value> {
    match r {
        Err(Error::ControlFlow {
            kind: crate::error::ControlFlow::Return,
            level: -1,
            value,
            ..
        }) => Ok(value.unwrap_or_default()),
        other => other,
    }
}

/// `tcl::dict::<sub>` ensemble commands: dispatch to `dict <sub> ...`
/// with the invoked name preserved for error messages.
macro_rules! dict_ensemble {
    ($($fn_name:ident => $sub:literal),* $(,)?) => { $(
        pub fn $fn_name(interp: &mut Interp, args: &[Value]) -> Result<Value> {
            let mut full: Vec<Value> = Vec::with_capacity(args.len() + 1);
            if let Some(first) = args.first() {
                full.push(first.clone());
            }
            full.push(Value::from_str($sub));
            full.extend_from_slice(&args[1..]);
            cmd_dict(interp, &full)
        }
    )* };
}

dict_ensemble! {
    cmd_dict_ens_append => "append",
    cmd_dict_ens_create => "create",
    cmd_dict_ens_exists => "exists",
    cmd_dict_ens_filter => "filter",
    cmd_dict_ens_for => "for",
    cmd_dict_ens_get => "get",
    cmd_dict_ens_incr => "incr",
    cmd_dict_ens_info => "info",
    cmd_dict_ens_keys => "keys",
    cmd_dict_ens_lappend => "lappend",
    cmd_dict_ens_map => "map",
    cmd_dict_ens_merge => "merge",
    cmd_dict_ens_remove => "remove",
    cmd_dict_ens_replace => "replace",
    cmd_dict_ens_set => "set",
    cmd_dict_ens_size => "size",
    cmd_dict_ens_unset => "unset",
    cmd_dict_ens_update => "update",
    cmd_dict_ens_values => "values",
    cmd_dict_ens_with => "with",
}

/// Tcl's `key "X" not known in dictionary`.
fn not_known(key: &str) -> Error {
    Error::runtime(
        format!("key \"{}\" not known in dictionary", key),
        crate::error::ErrorCode::NotFound,
    )
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // ── Ordered dict tests ─────────────────────────────────────

    // ── dict with (tclsh 8.6 semantics, corpus dict-22.x) ──────

    #[test]
    fn test_dict_with_writeback_updates_current_dict() {
        let mut interp = Interp::new();
        // 22.12: mid-body `dict set d c 3` survives the write-back; the
        // unset mapped variable removes its key; set vars take new values.
        let r = interp
            .eval(r#"set d {a 1 b 2}
                list [dict with d {
                    set a $b
                    unset b
                    dict set d c 3
                    list ok
                }] $d"#)
            .unwrap();
        assert_eq!(r.as_str(), "ok {a 2 c 3}");
    }

    #[test]
    fn test_dict_with_break_writes_back() {
        let mut interp = Interp::new();
        // 22.14: break mid-body still writes back the mapped vars.
        let r = interp
            .eval(r#"set d {a 1 b 2}
                foreach x {1 2 3} {
                    dict with d {
                        incr a $b
                        if {$x == 2} break
                    }
                    unset a b
                }
                list $a $b $x $d"#)
            .unwrap();
        assert_eq!(r.as_str(), "5 2 2 {a 5 b 2}");
    }

    #[test]
    fn test_dict_with_nested_key_path() {
        let mut interp = Interp::new();
        // 22.16: nested path writes back through the chain.
        let r = interp
            .eval(r#"set d {p {q {a 1 b 2}}}
                dict with d p q {
                    set a $b.$a
                }
                set d"#)
            .unwrap();
        assert_eq!(r.as_str(), "p {q {a 2.1 b 2}}");
        // A path key that does not exist is an error.
        assert_eq!(
            interp
                .eval("catch {dict with d nope {zz}} m; set m")
                .unwrap()
                .as_str(),
            "key \"nope\" not known in dictionary"
        );
    }

    #[test]
    fn test_dict_with_vars_persist_no_restore() {
        let mut interp = Interp::new();
        // 22.20: empty body — the mapped variables remain afterwards.
        let r = interp
            .eval(r#"apply {d {
                dict with d {
                }
                return $a,$b
            }} {a 1 b 2}"#)
            .unwrap();
        assert_eq!(r.as_str(), "1,2");
    }

    #[test]
    fn test_dict_with_error_no_writeback() {
        let mut interp = Interp::new();
        // Body error: variables stay mapped, dict not written back.
        let r = interp
            .eval(r#"set d {a 1 b 2}
                catch {dict with d {error boom}}
                list [info exists a] $d"#)
            .unwrap();
        assert_eq!(r.as_str(), "1 {a 1 b 2}");
    }

    #[test]
    fn test_dict_create_and_get() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("dict get {a 1 b 2} a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict get {a 1 b 2} b").unwrap().as_str(), "2");
        let r = interp
            .eval("dict get [dict create x 10 y 20] y")
            .unwrap();
        assert_eq!(r.as_str(), "20");
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
        assert_eq!(interp.eval("dict exists {a 1 b 2} a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict exists {a 1 b 2} c").unwrap().as_str(), "0");
    }

    #[test]
    fn test_dict_keys_values() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("dict keys {a 1 b 2 c 3}").unwrap().as_str(), "a b c");
        assert_eq!(interp.eval("dict values {a 1 b 2 c 3}").unwrap().as_str(), "1 2 3");
    }

    #[test]
    fn test_dict_size() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("dict size {a 1 b 2 c 3}").unwrap().as_str(), "3");
    }

    #[test]
    fn test_dict_remove() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict get [dict remove {a 1 b 2 c 3} b] a").unwrap().as_str(),
            "1"
        );
        assert_eq!(
            interp.eval("dict size [dict remove {a 1 b 2 c 3} b]").unwrap().as_str(),
            "2"
        );
    }

    #[test]
    fn test_dict_merge() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict get [dict merge {a 1 b 2} {c 3 d 4}] c").unwrap().as_str(),
            "3"
        );
    }

    #[test]
    fn test_dict_replace() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict get [dict replace {a 1 b 2} b 99] b").unwrap().as_str(),
            "99"
        );
    }

    #[test]
    fn test_dict_incr() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create count 5]").unwrap();
        interp.eval("dict incr d count").unwrap();
        assert_eq!(interp.eval("dict get $d count").unwrap().as_str(), "6");
    }

    #[test]
    fn test_dict_for_basic() {
        let mut interp = Interp::new();
        interp.eval("set result {}").unwrap();
        interp.eval("dict for {k v} {a 1 b 2} { lappend result $k=$v }").unwrap();
        assert_eq!(interp.eval("set result").unwrap().as_str(), "a=1 b=2");
    }

    #[test]
    fn test_dict_getwithdefault_found() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval(r#"dict getwithdefault {a 1 b 2} a "default""#).unwrap().as_str(),
            "1"
        );
    }

    #[test]
    fn test_dict_getwithdefault_not_found() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval(r#"dict getwithdefault {a 1 b 2} c "default""#).unwrap().as_str(),
            "default"
        );
    }

    #[test]
    fn test_dict_getwithdefault_nested() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval(r#"dict getwithdefault {a {x 10 y 20} b 2} a y "nope""#).unwrap().as_str(),
            "20"
        );
    }

    #[test]
    fn test_dict_update_basic() {
        let mut interp = Interp::new();
        interp.eval(r#"set d [dict create name "Jim" age 30]"#).unwrap();
        interp.eval(r#"dict update d name n age a { set n "Updated"; set a 31 }"#).unwrap();
        assert_eq!(interp.eval("dict get $d name").unwrap().as_str(), "Updated");
        assert_eq!(interp.eval("dict get $d age").unwrap().as_str(), "31");
    }

    #[test]
    fn test_dict_update_return_body() {
        let mut interp = Interp::new();
        interp.eval(r#"set d {x 10}"#).unwrap();
        let r = interp.eval(r#"dict update d x v { expr {$v + 5} }"#).unwrap();
        assert_eq!(r.as_str(), "15");
    }

    #[test]
    fn test_dict_getdef_tcl() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval(r#"dict getdef {a 1 b 2} c "fallback""#).unwrap().as_str(),
            "fallback"
        );
        assert_eq!(
            interp.eval(r#"dict getdef {a 1 b 2} a "fallback""#).unwrap().as_str(),
            "1"
        );
    }

    #[test]
    fn test_dict_filter_key() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict keys [dict filter {abc 1 abd 2 xyz 3} key ab*]").unwrap().as_str(),
            "abc abd"
        );
    }

    #[test]
    fn test_dict_parse_junk_errors() {
        // dict-4.14/4.15: tclDictObj.c's dict-specific element errors.
        let mut interp = Interp::new();
        let e = interp.eval("dict replace { a b {}c d }").unwrap_err();
        assert_eq!(
            e.to_string(),
            "dict element in braces followed by \"c\" instead of space"
        );
        let e = interp.eval("dict replace { a b \"\"c d }").unwrap_err();
        assert_eq!(
            e.to_string(),
            "dict element in quotes followed by \"c\" instead of space"
        );
        let e = interp.eval(r#"dict replace " a b \"c d ""#).unwrap_err();
        assert_eq!(e.to_string(), "unmatched open quote in dict");
        let e = interp.eval("dict replace \"{\"").unwrap_err();
        assert_eq!(e.to_string(), "unmatched open brace in dict");
    }

    #[test]
    fn test_dict_parse_junk_errorcode() {
        // dict-4.14a/4.16a: DICTIONARY JUNK / QUOTE errorCodes.
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {dict replace { a b {}c d }} -> opt; dict get $opt -errorcode")
            .unwrap();
        assert_eq!(r.as_str(), "TCL VALUE DICTIONARY JUNK");
        let r = interp
            .eval(r#"catch {dict replace " a b \"c d "} -> opt; dict get $opt -errorcode"#)
            .unwrap();
        assert_eq!(r.as_str(), "TCL VALUE DICTIONARY QUOTE");
    }

    #[test]
    fn test_dict_merge_no_args() {
        // `dict merge` with no dictionaries is an empty dict.
        let mut interp = Interp::new();
        assert_eq!(interp.eval("dict merge").unwrap().as_str(), "");
    }

    #[test]
    fn test_dict_merge_preserves_first_string() {
        // dict-20.21/20.22: merging without additions returns the first
        // dictionary unchanged, spaces and all.
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict merge { a b c d }").unwrap().as_str(),
            " a b c d "
        );
        assert_eq!(
            interp.eval("dict merge { a b c d } {}").unwrap().as_str(),
            " a b c d "
        );
        // Adding keys rebuilds the string rep.
        assert_eq!(
            interp.eval("dict merge { a b c d } { e f }").unwrap().as_str(),
            "a b c d e f"
        );
        // An empty first dict yields the rebuilt result.
        assert_eq!(
            interp.eval("dict merge {} { a b c d }").unwrap().as_str(),
            "a b c d"
        );
    }

    #[test]
    fn test_dict_unset_missing_path_key() {
        // dict-16.15: a missing key along a multi-key unset path errors;
        // a single missing key is a no-op.
        let mut interp = Interp::new();
        interp.eval("set d {a b}").unwrap();
        let e = interp.eval("dict unset d c d").unwrap_err();
        assert_eq!(e.to_string(), "key \"c\" not known in dictionary");
        let r = interp.eval("dict unset d c").unwrap();
        assert_eq!(r.as_str(), "a b");
    }

    #[test]
    fn test_dict_for_varlist_strict() {
        // dict-14.7: the varlist is parsed as a strict list, before the dict.
        let mut interp = Interp::new();
        let e = interp.eval("dict for \"\\{x\" x x").unwrap_err();
        assert!(e.to_string().contains("unmatched open brace in list"));
    }

    #[test]
    fn test_return_level0_in_command_bodies() {
        // dict-24.22: `return -level 0` completes the body with its value
        // instead of unwinding; a plain return still propagates.
        let mut interp = Interp::new();
        let r = interp
            .eval("dict map {k v} {a 1 b 2} {return -level 0 \"$k,$v\"}")
            .unwrap();
        assert_eq!(r.as_str(), "a a,1 b b,2");
        let r = interp
            .eval("apply {{} {dict map {k v} {a 1} {return -level 0 X}}}")
            .unwrap();
        assert_eq!(r.as_str(), "a X");
        // plain return in a body propagates to the proc boundary
        let r = interp
            .eval("apply {{} {dict map {k v} {a 1} {return X}}} ")
            .unwrap();
        assert_eq!(r.as_str(), "X");
        // return -level 0 inside a caught script is a normal completion
        let r = interp.eval("catch {return -level 0 xyz} m; set m").unwrap();
        assert_eq!(r.as_str(), "xyz");
    }

    #[test]
    fn test_tcl_dict_ensemble() {
        // dict-23.3/23.5: tcl::dict::lappend / tcl::dict::incr.
        let mut interp = Interp::new();
        let r = interp
            .eval("apply {{} {tcl::dict::lappend foo bar [format baz]}}")
            .unwrap();
        assert_eq!(r.as_str(), "bar baz");
        let r = interp
            .eval("apply {{} {tcl::dict::incr foo2 [format bar]}}")
            .unwrap();
        assert_eq!(r.as_str(), "bar 1");
    }

    #[test]
    fn test_tcl_mathop_plus() {
        // dict-24.24: tcl::mathop::+ folds with expr semantics.
        let mut interp = Interp::new();
        assert_eq!(interp.eval("tcl::mathop::+ 1 2 3").unwrap().as_str(), "6");
        assert_eq!(interp.eval("tcl::mathop::+").unwrap().as_str(), "0");
        assert_eq!(interp.eval("tcl::mathop::+ {*}[list 1 2]").unwrap().as_str(), "3");
        let e = interp.eval("tcl::mathop::+ abc").unwrap_err();
        assert!(e.to_string().contains("can't use non-numeric string"));
    }

    #[test]
    fn test_dict_filter_zero_patterns_empty() {
        // dict-17.4: key/value with no patterns yields nothing (no usage error).
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict filter {a b c d} key").unwrap().as_str(),
            ""
        );
    }

    #[test]
    fn test_dict_filter_parses_dict_before_type() {
        // dict-17.5: an odd dictionary beats every other error, even a bad type.
        let mut interp = Interp::new();
        let e = interp.eval("dict filter {a b c} key").unwrap_err();
        assert!(e.to_string().contains("missing value to go with key"));
        let e = interp.eval("dict filter {a b c} JUNK").unwrap_err();
        assert!(e.to_string().contains("missing value to go with key"));
        let e = interp.eval("dict filter {a b} JUNK").unwrap_err();
        assert_eq!(
            e.to_string(),
            "bad filterType \"JUNK\": must be key, script, or value"
        );
    }

    #[test]
    fn test_dict_filter_multi_pattern_or() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("dict keys [dict filter {abc 1 abd 2 xyz 3} key xy* ab*]")
                .unwrap()
                .as_str(),
            "abc abd xyz"
        );
    }

    #[test]
    fn test_dict_filter_script_varlist_strict() {
        // dict-17.20: a malformed varlist is a strict list parse error.
        let mut interp = Interp::new();
        let e = interp
            .eval("dict filter {a b} script \\{k v {expr 1}")
            .unwrap_err();
        assert!(e.to_string().contains("unmatched open brace in list"));
        let e = interp
            .eval("dict map \\{k {a 1} {set x 1}")
            .unwrap_err();
        assert!(e.to_string().contains("unmatched open brace in list"));
    }

    #[test]
    fn test_dict_filter_script_requires_boolean() {
        // dict-17.29: a non-boolean body result is an error, not a keep.
        let mut interp = Interp::new();
        let e = interp
            .eval("dict filter {a 1 b 2} script {k v} {list $k $v}")
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "expected boolean value but got \"a 1\""
        );
    }

    #[test]
    fn test_dict_map_basic() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict get [dict map {k v} {a 1 b 2} { expr {$v * 10} }] b").unwrap().as_str(),
            "20"
        );
    }

    #[test]
    fn test_dict_nested_set_get() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create]").unwrap();
        interp.eval("dict set d a b 42").unwrap();
        assert_eq!(interp.eval("dict get $d a b").unwrap().as_str(), "42");
    }

    // ── Ordered-specific tests ─────────────────────────────────

    #[test]
    fn test_dict_preserves_insertion_order() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict keys [dict create z 1 a 2 m 3]").unwrap().as_str(),
            "z a m"
        );
    }

    #[test]
    fn test_dict_ordered_explicit_flag() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("dict keys [dict create -ordered z 1 a 2 m 3]").unwrap().as_str(),
            "z a m"
        );
    }

    #[test]
    fn test_dict_ordered_for_preserves_order() {
        let mut interp = Interp::new();
        interp.eval("set result {}").unwrap();
        interp.eval("dict for {k v} [dict create z 1 a 2 m 3] { lappend result $k }").unwrap();
        assert_eq!(interp.eval("set result").unwrap().as_str(), "z a m");
    }

    #[test]
    fn test_dict_ordered_replace_preserves_order() {
        let mut interp = Interp::new();
        // Replace existing key — order should remain
        let r = interp.eval("dict keys [dict replace [dict create z 1 a 2 m 3] a 99]").unwrap();
        assert_eq!(r.as_str(), "z a m");
    }

    #[test]
    fn test_dict_ordered_merge_preserves_order() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict keys [dict merge [dict create z 1 a 2] [dict create m 3 b 4]]")
            .unwrap();
        assert_eq!(r.as_str(), "z a m b");
    }

    #[test]
    fn test_dict_ordered_remove_preserves_order() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict keys [dict remove [dict create z 1 a 2 m 3] a]")
            .unwrap();
        assert_eq!(r.as_str(), "z m");
    }

    #[test]
    fn test_dict_ordered_filter_preserves_order() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict keys [dict filter [dict create bz 1 aa 2 ba 3 ab 4] key a*]")
            .unwrap();
        assert_eq!(r.as_str(), "aa ab");
    }

    #[test]
    fn test_dict_ordered_map_preserves_order() {
        let mut interp = Interp::new();
        let r = interp
            .eval("dict keys [dict map {k v} [dict create z 1 a 2 m 3] { expr {$v * 10} }]")
            .unwrap();
        assert_eq!(r.as_str(), "z a m");
    }

    #[test]
    fn test_dict_ordered_set_preserves_existing_order() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create z 1 a 2 m 3]").unwrap();
        interp.eval("dict set d a 99").unwrap();
        assert_eq!(interp.eval("dict keys $d").unwrap().as_str(), "z a m");
        assert_eq!(interp.eval("dict get $d a").unwrap().as_str(), "99");
    }

    // ── Unordered dict tests ───────────────────────────────────

    #[test]
    fn test_dict_unordered_create() {
        let mut interp = Interp::new();
        // Create unordered — all keys/values should be present (order not guaranteed)
        interp.eval("set d [dict create -unordered a 1 b 2 c 3]").unwrap();
        assert_eq!(interp.eval("dict size $d").unwrap().as_str(), "3");
        assert_eq!(interp.eval("dict get $d a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict get $d b").unwrap().as_str(), "2");
        assert_eq!(interp.eval("dict get $d c").unwrap().as_str(), "3");
    }

    #[test]
    fn test_dict_unordered_set_get() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered]").unwrap();
        interp.eval("dict set d x 10").unwrap();
        interp.eval("dict set d y 20").unwrap();
        assert_eq!(interp.eval("dict get $d x").unwrap().as_str(), "10");
        assert_eq!(interp.eval("dict get $d y").unwrap().as_str(), "20");
        assert_eq!(interp.eval("dict size $d").unwrap().as_str(), "2");
    }

    #[test]
    fn test_dict_unordered_exists() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered a 1 b 2]").unwrap();
        assert_eq!(interp.eval("dict exists $d a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict exists $d c").unwrap().as_str(), "0");
    }

    #[test]
    fn test_dict_unordered_remove() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered a 1 b 2 c 3]").unwrap();
        let r = interp.eval("dict size [dict remove $d b]").unwrap();
        assert_eq!(r.as_str(), "2");
        assert_eq!(
            interp.eval("dict exists [dict remove $d b] b").unwrap().as_str(),
            "0"
        );
    }

    #[test]
    fn test_dict_unordered_incr() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered count 5]").unwrap();
        interp.eval("dict incr d count 3").unwrap();
        assert_eq!(interp.eval("dict get $d count").unwrap().as_str(), "8");
    }

    #[test]
    fn test_dict_unordered_replace() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered a 1 b 2]").unwrap();
        let r = interp.eval("dict get [dict replace $d b 99] b").unwrap();
        assert_eq!(r.as_str(), "99");
    }

    #[test]
    fn test_dict_unordered_for() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered x 10 y 20]").unwrap();
        interp.eval("set total 0").unwrap();
        interp.eval("dict for {k v} $d { set total [expr {$total + $v}] }").unwrap();
        assert_eq!(interp.eval("set total").unwrap().as_str(), "30");
    }

    #[test]
    fn test_dict_unordered_merge() {
        let mut interp = Interp::new();
        interp.eval("set a [dict create -unordered x 1 y 2]").unwrap();
        interp.eval("set b [dict create -unordered z 3]").unwrap();
        let r = interp.eval("dict size [dict merge $a $b]").unwrap();
        assert_eq!(r.as_str(), "3");
    }

    #[test]
    fn test_dict_unordered_filter_key() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered ab 1 ac 2 ba 3]").unwrap();
        let r = interp.eval("dict size [dict filter $d key a*]").unwrap();
        assert_eq!(r.as_str(), "2");
    }

    #[test]
    fn test_dict_unordered_info() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered a 1 b 2]").unwrap();
        let r = interp.eval("dict info $d").unwrap();
        assert!(r.as_str().contains("unordered"));
        assert!(r.as_str().contains("2 entries"));
    }

    #[test]
    fn test_dict_ordered_info() {
        let mut interp = Interp::new();
        let r = interp.eval("dict info {a 1 b 2}").unwrap();
        assert!(r.as_str().contains("ordered"));
        assert!(r.as_str().contains("2 entries"));
    }

    #[test]
    fn test_dict_unordered_keys_values_contain_all() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered x 10 y 20 z 30]").unwrap();
        // We can't assert order, but we can assert all values are present
        let keys = interp.eval("dict keys $d").unwrap();
        let keys_str = keys.as_str();
        assert!(keys_str.contains("x"));
        assert!(keys_str.contains("y"));
        assert!(keys_str.contains("z"));
        let vals = interp.eval("dict values $d").unwrap();
        let vals_str = vals.as_str();
        assert!(vals_str.contains("10"));
        assert!(vals_str.contains("20"));
        assert!(vals_str.contains("30"));
    }

    #[test]
    fn test_dict_unordered_serialization_roundtrip() {
        let mut interp = Interp::new();
        interp.eval("set d [dict create -unordered a 1 b 2 c 3]").unwrap();
        // Serialize to string, then parse back — all keys should survive
        interp.eval("set s [set d]").unwrap();
        assert_eq!(interp.eval("dict size $s").unwrap().as_str(), "3");
        assert_eq!(interp.eval("dict get $s a").unwrap().as_str(), "1");
        assert_eq!(interp.eval("dict get $s b").unwrap().as_str(), "2");
        assert_eq!(interp.eval("dict get $s c").unwrap().as_str(), "3");
    }
}
