//! Regular expression commands: regexp, regsub.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

use regex::Regex;

/// Build regex pattern string from flags.
fn build_pattern(pattern: &str, nocase: bool, expanded: bool, line: bool) -> String {
    let mut prefix = String::new();
    if nocase { prefix.push_str("(?i)"); }
    if line { prefix.push_str("(?m)"); } // multiline: ^ $ match line boundaries
    if expanded { prefix.push_str("(?x)"); }
    format!("{}{}", prefix, pattern)
}

/// Parse a `-start` index, supporting negative/end-relative values.
fn parse_start_offset(s: &str, len: usize) -> std::result::Result<usize, String> {
    if let Some(rest) = s.strip_prefix("end") {
        if rest.is_empty() {
            return Ok(len.saturating_sub(1));
        }
        if let Some(off) = rest.strip_prefix('-') {
            let n: usize = off.parse().map_err(|_| format!("bad index \"{}\"", s))?;
            return Ok(len.saturating_sub(1 + n));
        }
    }
    let n: i64 = s.parse().map_err(|_| format!("bad index \"{}\"", s))?;
    if n < 0 {
        Ok(0)
    } else {
        Ok(n as usize)
    }
}

/// `regexp ?switches? exp string ?matchVar? ?subMatchVar ...?`
///
/// Returns 1 if the regular expression matches, 0 otherwise.
/// If match variables are provided, stores the matched text.
pub fn cmd_regexp(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "regexp", 3, args.len(),
            "?switches? exp string ?matchVar? ?subMatchVar ...?",
        ));
    }

    let mut i = 1;
    let mut nocase = false;
    let mut all = false;
    let mut inline = false;
    let mut indices = false;
    let mut expanded = false;
    let mut line = false;
    let mut start_offset: Option<String> = None;

    // Parse switches
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-all" => { all = true; i += 1; }
            "-inline" => { inline = true; i += 1; }
            "-indices" => { indices = true; i += 1; }
            "-expanded" => { expanded = true; i += 1; }
            "-line" => { line = true; i += 1; }
            "-linestop" | "-lineanchor" => { line = true; i += 1; } // approximate
            "-start" => {
                if i + 1 >= args.len() {
                    return Err(Error::runtime(
                        "missing argument to \"-start\"", ErrorCode::Generic,
                    ));
                }
                start_offset = Some(args[i + 1].as_str().to_string());
                i += 2;
            }
            "--" => { i += 1; break; }
            s => {
                return Err(Error::runtime(
                    format!(
                        "bad switch \"{}\": must be -all, -expanded, -indices, \
                         -inline, -line, -lineanchor, -linestop, -nocase, -start, or --", s
                    ),
                    ErrorCode::Generic,
                ));
            }
        }
    }

    if i + 1 >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "regexp", 3, args.len(),
            "?switches? exp string ?matchVar? ?subMatchVar ...?",
        ));
    }

    // -inline conflicts with match variables
    if inline && i + 2 < args.len() {
        return Err(Error::runtime(
            "regexp match variables not allowed when using -inline",
            ErrorCode::Generic,
        ));
    }

    let pattern_str = args[i].as_str();
    let full_string = args[i + 1].as_str();
    let var_args = &args[i + 2..];

    // Handle -start offset
    let byte_start = if let Some(ref off_str) = start_offset {
        let char_off = parse_start_offset(off_str, full_string.chars().count())
            .map_err(|e| Error::runtime(e, ErrorCode::Generic))?;
        // Convert char offset to byte offset
        full_string.char_indices()
            .nth(char_off)
            .map(|(b, _)| b)
            .unwrap_or(full_string.len())
    } else {
        0
    };
    let string = &full_string[byte_start..];

    let re_pattern = build_pattern(pattern_str, nocase, expanded, line);
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
                if indices {
                    match caps.get(j) {
                        Some(m) => {
                            let s = byte_start + m.start();
                            let e = byte_start + m.end() - 1;
                            results.push(Value::from_str(&format!("{} {}", s, e)));
                        }
                        None => results.push(Value::from_str("-1 -1")),
                    }
                } else {
                    let m = caps.get(j).map(|m| m.as_str()).unwrap_or("");
                    results.push(Value::from_str(m));
                }
            }
        }
        return Ok(Value::from_list(&results));
    }

    if all {
        // -all: return count of matches; set vars to last match
        let mut count = 0;
        for caps in re.captures_iter(string) {
            count += 1;
            if !var_args.is_empty() {
                set_match_vars(interp, &caps, var_args, indices, byte_start)?;
            }
        }
        return Ok(Value::from_int(count));
    }

    if inline {
        // -inline: return matched substrings as list
        if let Some(caps) = re.captures(string) {
            let results: Vec<Value> = (0..caps.len())
                .map(|j| {
                    if indices {
                        match caps.get(j) {
                            Some(m) => {
                                let s = byte_start + m.start();
                                let e = byte_start + m.end() - 1;
                                Value::from_str(&format!("{} {}", s, e))
                            }
                            None => Value::from_str("-1 -1"),
                        }
                    } else {
                        Value::from_str(caps.get(j).map(|m| m.as_str()).unwrap_or(""))
                    }
                })
                .collect();
            return Ok(Value::from_list(&results));
        }
        return Ok(Value::from_str(""));
    }

    // Normal mode
    if let Some(caps) = re.captures(string) {
        set_match_vars(interp, &caps, var_args, indices, byte_start)?;
        Ok(Value::from_int(1))
    } else {
        // No match — set vars to empty / -1 -1
        for var in var_args {
            if indices {
                interp.set_var(var.as_str(), Value::from_str("-1 -1"))?;
            } else {
                interp.set_var(var.as_str(), Value::empty())?;
            }
        }
        Ok(Value::from_int(0))
    }
}

/// Set match variables from captures.
fn set_match_vars(
    interp: &mut Interp,
    caps: &regex::Captures,
    var_args: &[Value],
    indices: bool,
    byte_start: usize,
) -> Result<()> {
    for (vi, var) in var_args.iter().enumerate() {
        if indices {
            match caps.get(vi) {
                Some(m) => {
                    let s = byte_start + m.start();
                    let e = byte_start + m.end() - 1;
                    interp.set_var(var.as_str(), Value::from_str(&format!("{} {}", s, e)))?;
                }
                None => {
                    interp.set_var(var.as_str(), Value::from_str("-1 -1"))?;
                }
            }
        } else {
            let val = caps.get(vi).map(|m| m.as_str()).unwrap_or("");
            interp.set_var(var.as_str(), Value::from_str(val))?;
        }
    }
    Ok(())
}

/// `regsub ?switches? exp string subSpec ?varName?`
///
/// Substitutes regex matches. Returns the substituted string or count.
pub fn cmd_regsub(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(Error::wrong_args_with_usage(
            "regsub", 4, args.len(),
            "?switches? exp string subSpec ?varName?",
        ));
    }

    let mut i = 1;
    let mut nocase = false;
    let mut all = false;
    let mut expanded = false;
    let mut line = false;
    let mut start_offset: Option<String> = None;
    let mut command_mode = false;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-all" => { all = true; i += 1; }
            "-expanded" => { expanded = true; i += 1; }
            "-line" => { line = true; i += 1; }
            "-linestop" | "-lineanchor" => { line = true; i += 1; }
            "-start" => {
                if i + 1 >= args.len() {
                    return Err(Error::runtime(
                        "missing argument to \"-start\"", ErrorCode::Generic,
                    ));
                }
                start_offset = Some(args[i + 1].as_str().to_string());
                i += 2;
            }
            "-command" => { command_mode = true; i += 1; }
            "--" => { i += 1; break; }
            s => {
                return Err(Error::runtime(
                    format!(
                        "bad switch \"{}\": must be -all, -command, -expanded, \
                         -line, -lineanchor, -linestop, -nocase, -start, or --", s
                    ),
                    ErrorCode::Generic,
                ));
            }
        }
    }

    if i + 2 >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "regsub", 4, args.len(),
            "?switches? exp string subSpec ?varName?",
        ));
    }

    let pattern_str = args[i].as_str();
    let full_string = args[i + 1].as_str();
    let sub_spec = args[i + 2].as_str();
    let var_name = args.get(i + 3).map(|a| a.as_str());

    // Handle -start offset
    let byte_start = if let Some(ref off_str) = start_offset {
        let char_off = parse_start_offset(off_str, full_string.chars().count())
            .map_err(|e| Error::runtime(e, ErrorCode::Generic))?;
        full_string.char_indices()
            .nth(char_off)
            .map(|(b, _)| b)
            .unwrap_or(full_string.len())
    } else {
        0
    };

    let prefix = &full_string[..byte_start];
    let string = &full_string[byte_start..];

    let re_pattern = build_pattern(pattern_str, nocase, expanded, line);
    let re = Regex::new(&re_pattern).map_err(|e| {
        Error::runtime(
            format!("couldn't compile regular expression pattern: {}", e),
            ErrorCode::Generic,
        )
    })?;

    if command_mode {
        // -command: evaluate sub_spec as command prefix for each match
        let mut result = String::from(prefix);
        let mut count = 0i64;
        let mut last_end = 0;

        let captures: Vec<_> = if all {
            re.captures_iter(string).collect()
        } else {
            re.captures(string).into_iter().collect()
        };

        for caps in &captures {
            let full_match = caps.get(0).unwrap();
            result.push_str(&string[last_end..full_match.start()]);

            // Build command: subSpec fullMatch capture1 capture2 ...
            let mut cmd_str = sub_spec.to_string();
            for j in 0..caps.len() {
                let m = caps.get(j).map(|m| m.as_str()).unwrap_or("");
                cmd_str.push(' ');
                // Quote the argument for Tcl eval
                cmd_str.push('{');
                cmd_str.push_str(m);
                cmd_str.push('}');
            }
            let replacement = interp.eval(&cmd_str)?;
            result.push_str(replacement.as_str());
            last_end = full_match.end();
            count += 1;
        }
        result.push_str(&string[last_end..]);

        if let Some(var) = var_name {
            interp.set_var(var, Value::from_str(&result))?;
            return Ok(Value::from_int(count));
        }
        return Ok(Value::from_str(&result));
    }

    // Standard mode: Tcl substitution spec
    let replacement = tcl_sub_to_regex(sub_spec);

    let (subst_result, count) = if all {
        let count = re.find_iter(string).count() as i64;
        let r = re.replace_all(string, replacement.as_str());
        (r.to_string(), count)
    } else {
        let found = re.is_match(string);
        let r = re.replace(string, replacement.as_str());
        (r.to_string(), if found { 1 } else { 0 })
    };

    // Prepend the prefix (text before -start offset)
    let result = format!("{}{}", prefix, subst_result);

    if let Some(var) = var_name {
        interp.set_var(var, Value::from_str(&result))?;
        Ok(Value::from_int(count))
    } else {
        Ok(Value::from_str(&result))
    }
}

/// Convert Tcl substitution spec to regex replacement syntax.
/// `\0` or `&` → `$0`, `\1` → `$1`, etc.
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
