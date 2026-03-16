//! Array commands: array set/get/names/size/exists/unset.

use crate::error::{Error, Result};
use crate::interp::{glob_match, split_array_ref, Interp};
use crate::value::Value;

pub fn cmd_array(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args("array", 3, args.len()));
    }

    let subcmd = args[1].as_str();
    let array_name = args[2].as_str();

    match subcmd {
        "set" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("array set", 4, args.len()));
            }
            let list = args[3].as_list().unwrap_or_default();
            if list.len() % 2 != 0 {
                return Err(Error::runtime(
                    "list must have an even number of elements",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            for chunk in list.chunks(2) {
                let var_name = format!("{}({})", array_name, chunk[0].as_str());
                interp.set_var(&var_name, chunk[1].clone())?;
            }
            Ok(Value::empty())
        }
        "get" => {
            let pattern = if args.len() > 3 { Some(args[3].as_str()) } else { None };
            let mut result: Vec<Value> = Vec::new();
            let prefix = format!("{}(", array_name);
            let vars: Vec<(String, Value)> = interp
                .vars
                .iter()
                .filter_map(|(k, v)| {
                    if k.starts_with(&prefix) && k.ends_with(')') {
                        let elem = &k[prefix.len()..k.len() - 1];
                        if let Some(pat) = pattern {
                            if glob_match(pat, elem) {
                                Some((elem.to_string(), v.clone()))
                            } else {
                                None
                            }
                        } else {
                            Some((elem.to_string(), v.clone()))
                        }
                    } else {
                        None
                    }
                })
                .collect();
            for (elem, val) in vars {
                result.push(Value::from_str(&elem));
                result.push(val);
            }
            Ok(Value::from_list(&result))
        }
        "names" => {
            let pattern = if args.len() > 3 { Some(args[3].as_str()) } else { None };
            let prefix = format!("{}(", array_name);
            let names: Vec<Value> = interp
                .vars
                .keys()
                .filter_map(|k| {
                    if k.starts_with(&prefix) && k.ends_with(')') {
                        let elem = &k[prefix.len()..k.len() - 1];
                        if let Some(pat) = pattern {
                            if glob_match(pat, elem) {
                                Some(Value::from_str(elem))
                            } else {
                                None
                            }
                        } else {
                            Some(Value::from_str(elem))
                        }
                    } else {
                        None
                    }
                })
                .collect();
            Ok(Value::from_list(&names))
        }
        "size" => {
            let prefix = format!("{}(", array_name);
            let count = interp
                .vars
                .keys()
                .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                .count();
            Ok(Value::from_int(count as i64))
        }
        "exists" => {
            let prefix = format!("{}(", array_name);
            let exists = interp
                .vars
                .keys()
                .any(|k| k.starts_with(&prefix) && k.ends_with(')'));
            Ok(Value::from_bool(exists))
        }
        "unset" => {
            let prefix = format!("{}(", array_name);
            let pattern = if args.len() > 3 { Some(args[3].as_str()) } else { None };
            let keys_to_remove: Vec<String> = interp
                .vars
                .keys()
                .filter(|k| {
                    if k.starts_with(&prefix) && k.ends_with(')') {
                        if let Some(pat) = pattern {
                            let elem = &k[prefix.len()..k.len() - 1];
                            glob_match(pat, elem)
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                })
                .cloned()
                .collect();
            for k in keys_to_remove {
                interp.vars.remove(&k);
            }
            Ok(Value::empty())
        }
        _ => Err(Error::runtime(
            format!("unknown array subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

// Make split_array_ref accessible.  Re-export it from interp so array
// consumers don't need to import interp directly.
#[allow(dead_code)]
pub(crate) fn is_array_ref(name: &str) -> bool {
    split_array_ref(name).is_some()
}
