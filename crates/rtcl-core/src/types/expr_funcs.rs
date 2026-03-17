//! Math function evaluation for the Tcl expression parser.

use crate::error::{Error, Result};
use crate::value::Value;

/// Return int if the float has no fractional part and fits in i64.
pub(crate) fn float_or_int(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
        Value::from_int(f as i64)
    } else {
        Value::from_float(f)
    }
}

fn require_args(name: &str, expected: usize, actual: usize) -> Result<()> {
    if actual != expected {
        Err(Error::wrong_args(&format!("{}()", name), expected, actual))
    } else {
        Ok(())
    }
}

/// Evaluate a built-in math function by name.
///
/// `rand_seed` is used only for `rand()` — caller provides a unique value.
pub(super) fn call_math_func(name: &str, args: Vec<Value>, rand_seed: usize) -> Result<Value> {
    match name {
        "abs" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(float_or_int(n.abs())),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "int" | "entier" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_int(n as i64)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "wide" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_int(n as i64)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "double" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_float(n)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "bool" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_bool(args[0].is_true()))
        }
        "round" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_int(n.round() as i64)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "floor" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_int(n.floor() as i64)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "ceil" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_int(n.ceil() as i64)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "sqrt" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(Value::from_float(n.sqrt())),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "pow" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(float_or_int(a.powf(b))),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "fmod" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(Value::from_float(a % b)),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "atan2" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(Value::from_float(a.atan2(b))),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "hypot" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(Value::from_float(a.hypot(b))),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "log" | "log10" | "exp"
        | "sinh" | "cosh" | "tanh" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => {
                    let result = match name {
                        "sin" => n.sin(),
                        "cos" => n.cos(),
                        "tan" => n.tan(),
                        "asin" => n.asin(),
                        "acos" => n.acos(),
                        "atan" => n.atan(),
                        "log" => n.ln(),
                        "log10" => n.log10(),
                        "exp" => n.exp(),
                        "sinh" => n.sinh(),
                        "cosh" => n.cosh(),
                        "tanh" => n.tanh(),
                        _ => n,
                    };
                    Ok(Value::from_float(result))
                }
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "min" | "max" => {
            if args.is_empty() {
                return Err(Error::wrong_args(&format!("{}()", name), 1, args.len()));
            }
            let nums: std::result::Result<Vec<f64>, _> = args.iter()
                .map(|v| v.as_float().ok_or_else(|| Error::type_mismatch("number", "non-numeric value")))
                .collect();
            let nums = nums?;
            let result = if name == "min" {
                nums.into_iter().fold(f64::INFINITY, f64::min)
            } else {
                nums.into_iter().fold(f64::NEG_INFINITY, f64::max)
            };
            Ok(float_or_int(result))
        }
        "rand" => {
            let val = ((rand_seed.wrapping_mul(6364136223846793005).wrapping_add(1)) as f64)
                / (usize::MAX as f64);
            Ok(Value::from_float(val.abs() % 1.0))
        }
        "srand" => {
            require_args(name, 1, args.len())?;
            Ok(Value::empty())
        }
        "isqrt" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) if n >= 0.0 => Ok(Value::from_int((n.sqrt()) as i64)),
                _ => Err(Error::runtime("domain error: argument not in valid range", crate::error::ErrorCode::InvalidOp)),
            }
        }
        _ => Err(Error::runtime(
            format!("unknown math function \"{}\"", name),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}
