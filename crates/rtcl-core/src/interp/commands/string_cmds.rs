//! String commands: string subcommands.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;

pub fn cmd_string(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args("string", 3, args.len()));
    }

    let subcmd = args[1].as_str();
    let str_val = args[2].as_str();

    match subcmd {
        "length" => Ok(Value::from_int(str_val.len() as i64)),
        "tolower" => Ok(Value::from_str(&str_val.to_lowercase())),
        "toupper" => Ok(Value::from_str(&str_val.to_uppercase())),
        "trim" => {
            let chars = if args.len() > 3 { args[3].as_str() } else { " \t\n\r" };
            Ok(Value::from_str(str_val.trim_matches(|c| chars.contains(c))))
        }
        "trimleft" => {
            let chars = if args.len() > 3 { args[3].as_str() } else { " \t\n\r" };
            Ok(Value::from_str(str_val.trim_start_matches(|c| chars.contains(c))))
        }
        "trimright" => {
            let chars = if args.len() > 3 { args[3].as_str() } else { " \t\n\r" };
            Ok(Value::from_str(str_val.trim_end_matches(|c| chars.contains(c))))
        }
        "range" => {
            if args.len() != 5 {
                return Err(Error::wrong_args("string range", 5, args.len()));
            }
            let start: usize = args[3].as_int().unwrap_or(0) as usize;
            let end: usize = args[4].as_int().unwrap_or(str_val.len() as i64) as usize;
            let end = end.min(str_val.len() - 1);
            if start <= end && start < str_val.len() {
                Ok(Value::from_str(&str_val[start..=end]))
            } else {
                Ok(Value::empty())
            }
        }
        "index" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string index", 4, args.len()));
            }
            let idx: usize = args[3].as_int().unwrap_or(-1) as usize;
            if idx < str_val.len() {
                Ok(Value::from_str(&str_val[idx..idx + 1]))
            } else {
                Ok(Value::empty())
            }
        }
        "equal" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string equal", 4, args.len()));
            }
            let other = args[3].as_str();
            Ok(Value::from_bool(str_val == other))
        }
        "match" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string match", 4, args.len()));
            }
            let pattern = args[3].as_str();
            Ok(Value::from_bool(glob_match(pattern, str_val)))
        }
        "first" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string first", 4, args.len()));
            }
            let needle = args[3].as_str();
            let start = if args.len() > 4 {
                args[4].as_int().unwrap_or(0) as usize
            } else {
                0
            };
            let pos = str_val[start..]
                .find(needle)
                .map(|i| (i + start) as i64)
                .unwrap_or(-1);
            Ok(Value::from_int(pos))
        }
        "last" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string last", 4, args.len()));
            }
            let needle = args[3].as_str();
            let pos = str_val.rfind(needle).map(|i| i as i64).unwrap_or(-1);
            Ok(Value::from_int(pos))
        }
        _ => Err(Error::runtime(
            format!("unknown string subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}
