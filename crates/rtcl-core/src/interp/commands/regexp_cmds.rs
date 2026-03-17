//! Regular expression commands: regexp, regsub.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

#[cfg(feature = "std")]
use regex::Regex;

/// `regexp ?switches? exp string ?matchVar? ?subMatchVar ...?`
///
/// Returns 1 if the regular expression matches, 0 otherwise.
/// If match variables are provided, stores the matched text.
#[cfg(feature = "std")]
pub fn cmd_regexp(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "regexp",
            3,
            args.len(),
            "?switches? exp string ?matchVar? ?subMatchVar ...?",
        ));
    }

    let mut i = 1;
    let mut nocase = false;
    let mut all = false;
    let mut inline = false;

    // Parse switches
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-all" => { all = true; i += 1; }
            "-inline" => { inline = true; i += 1; }
            "--" => { i += 1; break; }
            s => {
                return Err(Error::runtime(
                    format!("bad switch \"{}\": must be -all, -inline, -nocase, or --", s),
                    ErrorCode::Generic,
                ));
            }
        }
    }

    if i + 1 >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "regexp",
            3,
            args.len(),
            "?switches? exp string ?matchVar? ?subMatchVar ...?",
        ));
    }

    let pattern_str = args[i].as_str();
    let string = args[i + 1].as_str();
    let var_args = &args[i + 2..];

    let re_pattern = if nocase {
        format!("(?i){}", pattern_str)
    } else {
        pattern_str.to_string()
    };

    let re = Regex::new(&re_pattern).map_err(|e| {
        Error::runtime(
            format!("couldn't compile regular expression pattern: {}", e),
            ErrorCode::Generic,
        )
    })?;

    if all && inline {
        // -all -inline: return list of all matches
        let mut results = Vec::new();
        for caps in re.captures_iter(string) {
            for j in 0..caps.len() {
                let m = caps.get(j).map(|m| m.as_str()).unwrap_or("");
                results.push(Value::from_str(m));
            }
        }
        return Ok(Value::from_list(&results));
    }

    if all {
        // -all: return count of matches
        let mut count = 0;
        for caps in re.captures_iter(string) {
            count += 1;
            // Set vars to last match
            if !var_args.is_empty() {
                for (vi, var) in var_args.iter().enumerate() {
                    let val = caps.get(vi).map(|m| m.as_str()).unwrap_or("");
                    interp.set_var(var.as_str(), Value::from_str(val))?;
                }
            }
        }
        return Ok(Value::from_int(count));
    }

    if inline {
        // -inline: return matched substrings as list
        if let Some(caps) = re.captures(string) {
            let results: Vec<Value> = (0..caps.len())
                .map(|j| Value::from_str(caps.get(j).map(|m| m.as_str()).unwrap_or("")))
                .collect();
            return Ok(Value::from_list(&results));
        }
        return Ok(Value::from_str(""));
    }

    // Normal mode
    if let Some(caps) = re.captures(string) {
        // Set match variables
        for (vi, var) in var_args.iter().enumerate() {
            let val = caps.get(vi).map(|m| m.as_str()).unwrap_or("");
            interp.set_var(var.as_str(), Value::from_str(val))?;
        }
        Ok(Value::from_int(1))
    } else {
        // No match — set vars to empty
        for var in var_args {
            interp.set_var(var.as_str(), Value::empty())?;
        }
        Ok(Value::from_int(0))
    }
}

/// `regsub ?switches? exp string subSpec ?varName?`
///
/// Substitutes regex matches. Returns the substituted string or count.
#[cfg(feature = "std")]
pub fn cmd_regsub(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "regsub",
            4,
            args.len(),
            "?switches? exp string subSpec ?varName?",
        ));
    }

    let mut i = 1;
    let mut nocase = false;
    let mut all = false;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-all" => { all = true; i += 1; }
            "--" => { i += 1; break; }
            s => {
                return Err(Error::runtime(
                    format!("bad switch \"{}\": must be -all, -nocase, or --", s),
                    ErrorCode::Generic,
                ));
            }
        }
    }

    if i + 2 >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "regsub",
            4,
            args.len(),
            "?switches? exp string subSpec ?varName?",
        ));
    }

    let pattern_str = args[i].as_str();
    let string = args[i + 1].as_str();
    let sub_spec = args[i + 2].as_str();
    let var_name = args.get(i + 3).map(|a| a.as_str());

    let re_pattern = if nocase {
        format!("(?i){}", pattern_str)
    } else {
        pattern_str.to_string()
    };

    let re = Regex::new(&re_pattern).map_err(|e| {
        Error::runtime(
            format!("couldn't compile regular expression pattern: {}", e),
            ErrorCode::Generic,
        )
    })?;

    // Convert Tcl substitution spec to regex replacement:
    // \0 or & → $0 (full match)
    // \1 → $1, \2 → $2, etc.
    let replacement = tcl_sub_to_regex(sub_spec);

    let (result, count) = if all {
        let count = re.find_iter(string).count() as i64;
        let r = re.replace_all(string, replacement.as_str());
        (r.to_string(), count)
    } else {
        let found = re.is_match(string);
        let r = re.replace(string, replacement.as_str());
        (r.to_string(), if found { 1 } else { 0 })
    };

    if let Some(var) = var_name {
        interp.set_var(var, Value::from_str(&result))?;
        Ok(Value::from_int(count))
    } else {
        Ok(Value::from_str(&result))
    }
}

/// Convert Tcl substitution spec to regex replacement syntax.
/// `\0` or `&` → `$0`, `\1` → `$1`, etc.
#[cfg(feature = "std")]
fn tcl_sub_to_regex(spec: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next.is_ascii_digit() {
                result.push('$');
                result.push(next);
                i += 2;
                continue;
            }
            // \\ → \, other \X → X
            result.push(next);
            i += 2;
        } else if chars[i] == '&' {
            result.push_str("$0");
            i += 1;
        } else if chars[i] == '$' {
            // Escape literal $ so regex doesn't interpret it
            result.push_str("$$");
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[cfg(not(feature = "std"))]
pub fn cmd_regexp(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "regexp requires std feature",
        ErrorCode::Generic,
    ))
}

#[cfg(not(feature = "std"))]
pub fn cmd_regsub(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "regsub requires std feature",
        ErrorCode::Generic,
    ))
}
