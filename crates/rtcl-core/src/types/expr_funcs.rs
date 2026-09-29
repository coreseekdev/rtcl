//! Math function evaluation for the Tcl expression parser.

use core::cmp::Ordering;

use crate::error::{Error, Result};
use crate::value::Value;

/// tclsh math-domain failure: `domain error: argument not in valid range`
/// with errorCode `ARITH DOMAIN {...}`.
pub(crate) fn domain_error() -> Error {
    Error::runtime(
        "domain error: argument not in valid range",
        crate::error::ErrorCode::Generic,
    )
}

/// Format a double the way Tcl 8.6 stringifies one: shortest round-trip
/// decimal digits, fixed notation when the decimal exponent of the leading
/// digit is in `[-4, 16]` (appending `.0` when the digits would otherwise
/// read as an integer), scientific notation (`1.5e+300`, `5e-324`,
/// `1e+17`) outside that range.
///
/// This lives here (not in rtcl-vm's `value.rs`) so expression results
/// match tclsh 8.6 byte-for-byte without changing the shared formatter.
pub(crate) fn format_tcl_float(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f < 0.0 { "-Inf".to_string() } else { "Inf".to_string() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    // Rust's LowerExp yields the shortest round-trip decimal as "d[.ddd]e<exp>".
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp_str) = match sci.split_once('e') {
        Some(parts) => parts,
        None => return sci,
    };
    let exp10: i32 = exp_str.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let n = digits.len() as i32;

    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if (-4..=16).contains(&exp10) {
        if exp10 >= n - 1 {
            // Integral value: digits left-padded to the decimal point, then ".0"
            out.push_str(&digits);
            for _ in 0..(exp10 - (n - 1)) {
                out.push('0');
            }
            out.push_str(".0");
        } else if exp10 >= 0 {
            let split = (exp10 + 1) as usize;
            out.push_str(&digits[..split]);
            out.push('.');
            out.push_str(&digits[split..]);
        } else {
            out.push_str("0.");
            for _ in 0..(-exp10 - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        if n > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp10 < 0 { '-' } else { '+' });
        out.push_str(&exp10.unsigned_abs().to_string());
    }
    out
}

/// A float `Value` carrying Tcl's canonical string form. Used for every
/// float produced by the expression evaluator so results stringify exactly
/// like tclsh (rtcl-vm's `Value::from_float` uses a different layout).
pub(crate) fn float_value(f: f64) -> Value {
    Value::from_str(&format_tcl_float(f))
}

/// Tcl boolean word parsing: case-insensitive, any unique-prefix
/// abbreviation of true/false/yes/no/on/off. Returns `None` for ambiguous
/// ("o" — on/off) or unrecognized strings.
fn bool_from_string(s: &str) -> Option<bool> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let mut candidates = ["true", "false", "yes", "no", "on", "off"]
        .iter()
        .filter(|word| word.starts_with(lower.as_str()));
    let first = *candidates.next()?;
    if candidates.next().is_some() {
        return None; // ambiguous prefix
    }
    Some(matches!(first, "true" | "yes" | "on"))
}

/// Tcl boolean coercion (`Tcl_GetBooleanFromObj`): any numeric value is a
/// valid boolean (zero, including 0.0, is false); otherwise the string
/// must be a boolean word or unambiguous abbreviation. Anything else is an
/// error, as in `while {$x}` with x="foo".
pub(crate) fn strict_bool(v: &Value) -> Result<bool> {
    if let Some(i) = v.as_int() {
        return Ok(i != 0);
    }
    if let Some(f) = v.as_float() {
        return Ok(f != 0.0);
    }
    if let Some(b) = bool_from_string(v.as_str()) {
        return Ok(b);
    }
    Err(Error::Msg(format!(
        "expected boolean value but got \"{}\"",
        v.as_str()
    )))
}

/// Operand coercion for unary `!`: accepts the same values as
/// [`strict_bool`], but Tcl reports a different message for a
/// non-numeric, non-boolean operand of `!`.
pub(crate) fn not_operand(v: &Value) -> Result<bool> {
    if let Some(i) = v.as_int() {
        return Ok(i != 0);
    }
    if let Some(f) = v.as_float() {
        return Ok(f != 0.0);
    }
    if let Some(b) = bool_from_string(v.as_str()) {
        return Ok(b);
    }
    Err(Error::Msg(
        "can't use non-numeric string as operand of \"!\"".to_string(),
    ))
}

/// Numeric comparison following Tcl rules: if both operands parse as
/// integers, compare as i64 (no precision loss through double); a mixed
/// int/double pair is compared exactly; two doubles use IEEE comparison.
/// Returns `None` when either operand is non-numeric (caller falls back to
/// string comparison).
/// Both operands numeric and at least one NaN — Tcl: every comparison
/// is false except `!=` (expr-22.9: NaN == NaN -> 0).
pub(crate) fn nan_pair(a: &Value, b: &Value) -> bool {
    let numeric = |v: &Value| v.as_int().is_some() || v.as_float().is_some();
    let nan = |v: &Value| v.as_float().map(|f| f.is_nan()).unwrap_or(false);
    numeric(a) && numeric(b) && (nan(a) || nan(b))
}

pub(crate) fn numeric_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    match (a.as_int(), b.as_int()) {
        (Some(x), Some(y)) => Some(x.cmp(&y)),
        (Some(x), None) => b.as_float().and_then(|y| cmp_int_float(x, y)),
        (None, Some(y)) => a
            .as_float()
            .and_then(|x| cmp_int_float(y, x).map(Ordering::reverse)),
        (None, None) => match (a.as_float(), b.as_float()) {
            (Some(x), Some(y)) => x.partial_cmp(&y),
            _ => None,
        },
    }
}

/// Exact comparison of an i64 against an f64 without rounding the integer
/// to double (which would conflate e.g. 2^53+1 with 2^53).
fn cmp_int_float(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    const TWO63: f64 = 9223372036854775808.0; // 2^63
    if f >= TWO63 {
        return Some(Ordering::Less);
    }
    if f < -TWO63 {
        return Some(Ordering::Greater);
    }
    // |f| < 2^63, so its truncation is exactly representable as i64.
    let fi = f.trunc() as i64;
    Some(match i.cmp(&fi) {
        Ordering::Equal => 0.0f64.partial_cmp(&f.fract()).unwrap_or(Ordering::Equal),
        ord => ord,
    })
}

fn require_args(name: &str, expected: usize, actual: usize) -> Result<()> {
    if actual != expected {
        Err(Error::wrong_args(format!("{name}()"), expected, actual))
    } else {
        Ok(())
    }
}

/// Evaluate a built-in math function by name.
///
/// `rand_seed` is used only for `rand()` — caller provides a unique value.
pub(crate) fn call_math_func(name: &str, args: Vec<Value>, rand_seed: usize) -> Result<Value> {
    match name {
        "abs" => {
            require_args(name, 1, args.len())?;
            if let Some(i) = args[0].as_int() {
                // i64::MIN: Tcl widens abs to bignum; we promote to double
                return Ok(match i.checked_abs() {
                    Some(r) => Value::from_int(r),
                    None => float_value(-(i as f64)),
                });
            }
            match args[0].as_float() {
                Some(n) => Ok(float_value(n.abs())),
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
                Some(n) => Ok(float_value(n)),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "bool" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_bool(strict_bool(&args[0])?))
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
                Some(n) => Ok(float_value(n.floor())),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "ceil" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(float_value(n.ceil())),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "sqrt" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) if n >= 0.0 => Ok(float_value(n.sqrt())),
                Some(_) => Err(domain_error()),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "pow" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                // tclsh: negative base with a non-integer exponent is a
                // domain error (integer exponents are exact).
                (Some(a), Some(b)) if a < 0.0 && b.fract() != 0.0 => Err(domain_error()),
                (Some(a), Some(b)) => Ok(float_value(a.powf(b))),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "fmod" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(_), Some(b)) if b == 0.0 => Err(domain_error()),
                (Some(a), Some(b)) => Ok(float_value(a % b)),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "atan2" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(float_value(a.atan2(b))),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "hypot" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(float_value(a.hypot(b))),
                _ => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "log" | "log10" | "exp"
        | "sinh" | "cosh" | "tanh" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => {
                    // tclsh domain rules: log/log10 reject negatives
                    // (zero → -Inf is fine), asin/acos reject |x| > 1.
                    match name {
                        "log" | "log10" if n < 0.0 => return Err(domain_error()),
                        "asin" | "acos" if !(-1.0..=1.0).contains(&n) => {
                            return Err(domain_error())
                        }
                        _ => {}
                    }
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
                    Ok(float_value(result))
                }
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "min" | "max" => {
            if args.is_empty() {
                return Err(Error::wrong_args(format!("{}()", name), 1, args.len()));
            }
            // Tcl returns the winning operand itself, preserving its type:
            // min(2.0, 3) => 2.0 but min(2, 3.0) => 2.
            let mut best = args[0].clone();
            for v in &args[1..] {
                let ord = numeric_cmp(v, &best)
                    .ok_or_else(|| Error::type_mismatch("number", "non-numeric value"))?;
                let take = if name == "min" {
                    ord == Ordering::Less
                } else {
                    ord == Ordering::Greater
                };
                if take {
                    best = v.clone();
                }
            }
            Ok(best)
        }
        "rand" => {
            let val = ((rand_seed as u64).wrapping_mul(6364136223846793005u64).wrapping_add(1) as f64)
                / (u64::MAX as f64);
            Ok(float_value(val.abs() % 1.0))
        }
        "srand" => {
            require_args(name, 1, args.len())?;
            Ok(Value::empty())
        }
        "isqrt" => {
            if args.len() > 1 {
                return Err(Error::runtime(
                    format!("too many arguments for math function \"{}\"", name),
                    crate::error::ErrorCode::Generic,
                ));
            }
            if args.len() < 1 {
                return Err(Error::runtime(
                    format!("too few arguments for math function \"{}\"", name),
                    crate::error::ErrorCode::Generic,
                ));
            }
            match args[0].as_float() {
                Some(n) if n >= 0.0 => Ok(Value::from_int((n.sqrt()) as i64)),
                Some(_) => Err(Error::runtime(
                    "square root of negative argument",
                    crate::error::ErrorCode::InvalidOp,
                )),
                None => Err(Error::runtime(
                    format!("expected number but got \"{}\"", args[0].as_str()),
                    crate::error::ErrorCode::Generic,
                )),
            }
        }
        _ => Err(Error::runtime(
            format!("unknown math function \"{}\"", name),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}
