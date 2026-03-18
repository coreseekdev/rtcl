//! String commands: string subcommands.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::types::parse_index;
use crate::value::Value;

pub fn cmd_string(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args("string", 3, args.len()));
    }

    let subcmd = args[1].as_str();
    let str_val = args[2].as_str();

    match subcmd {
        "length" => Ok(Value::from_int(str_val.chars().count() as i64)),
        "bytelength" => Ok(Value::from_int(str_val.len() as i64)),
        "tolower" => Ok(Value::from_str(&str_val.to_lowercase())),
        "toupper" => Ok(Value::from_str(&str_val.to_uppercase())),
        "totitle" => {
            let mut result = String::with_capacity(str_val.len());
            let mut capitalize_next = true;
            for c in str_val.chars() {
                if capitalize_next {
                    result.extend(c.to_uppercase());
                    capitalize_next = false;
                } else {
                    result.extend(c.to_lowercase());
                }
                if c.is_whitespace() {
                    capitalize_next = true;
                }
            }
            Ok(Value::from_str(&result))
        }
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
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            let start = parse_index(args[3].as_str(), len).unwrap_or(0);
            let end = parse_index(args[4].as_str(), len).unwrap_or(len.saturating_sub(1));
            if start <= end && start < len {
                let s: String = chars[start..=end.min(len - 1)].iter().collect();
                Ok(Value::from_str(&s))
            } else {
                Ok(Value::empty())
            }
        }
        "index" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string index", 4, args.len()));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            match parse_index(args[3].as_str(), len) {
                Some(idx) if idx < len => Ok(Value::from_str(&chars[idx].to_string())),
                _ => Ok(Value::empty()),
            }
        }
        "equal" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string equal", 4, args.len()));
            }
            let (nocase, length, s1, s2) = parse_string_opts(args)?;
            let (a, b) = if let Some(n) = length {
                let n = n as usize;
                (
                    s1.chars().take(n).collect::<String>(),
                    s2.chars().take(n).collect::<String>(),
                )
            } else {
                (s1, s2)
            };
            if nocase {
                Ok(Value::from_bool(a.to_lowercase() == b.to_lowercase()))
            } else {
                Ok(Value::from_bool(a == b))
            }
        }
        "compare" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string compare", 4, args.len()));
            }
            let (nocase, length, s1, s2) = parse_string_opts(args)?;
            let (a, b) = if let Some(n) = length {
                let n = n as usize;
                (
                    s1.chars().take(n).collect::<String>(),
                    s2.chars().take(n).collect::<String>(),
                )
            } else {
                (s1, s2)
            };
            let cmp = if nocase {
                a.to_lowercase().cmp(&b.to_lowercase())
            } else {
                a.cmp(&b)
            };
            Ok(Value::from_int(match cmp {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        }
        "match" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string match", 4, args.len()));
            }
            // -nocase support
            let (nocase, _length, pattern, text) = parse_string_opts(args)?;
            if nocase {
                Ok(Value::from_bool(glob_match(&pattern.to_lowercase(), &text.to_lowercase())))
            } else {
                Ok(Value::from_bool(glob_match(&pattern, &text)))
            }
        }
        "first" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string first", 4, args.len()));
            }
            let needle = args[3].as_str();
            let start = if args.len() > 4 {
                parse_index(args[4].as_str(), str_val.len()).unwrap_or(0)
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
        "map" => {
            if args.len() < 4 || args.len() > 5 {
                return Err(Error::wrong_args_with_usage("string map", 4, args.len(), "?-nocase? mapping string"));
            }
            let (nocase, idx) = if args.len() == 5 && args[2].as_str() == "-nocase" {
                (true, 3)
            } else {
                (false, 2)
            };
            let mapping = args[idx].as_list().unwrap_or_default();
            let input = args[idx + 1].as_str();
            if !mapping.len().is_multiple_of(2) {
                return Err(Error::runtime("char map list unbalanced", crate::error::ErrorCode::InvalidOp));
            }
            let pairs: Vec<(String, String)> = mapping
                .chunks(2)
                .map(|c| (c[0].as_str().to_string(), c[1].as_str().to_string()))
                .collect();
            let mut result = String::new();
            let chars: Vec<char> = input.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let remaining: String = chars[i..].iter().collect();
                let mut matched = false;
                for (from, to) in &pairs {
                    let matches = if nocase {
                        remaining.to_lowercase().starts_with(&from.to_lowercase())
                    } else {
                        remaining.starts_with(from.as_str())
                    };
                    if matches && !from.is_empty() {
                        result.push_str(to);
                        i += from.chars().count();
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            Ok(Value::from_str(&result))
        }
        "repeat" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string repeat", 4, args.len()));
            }
            let count = args[3].as_int().unwrap_or(0);
            if count < 0 {
                return Err(Error::runtime("bad count", crate::error::ErrorCode::InvalidOp));
            }
            Ok(Value::from_str(&str_val.repeat(count as usize)))
        }
        "reverse" => {
            Ok(Value::from_str(&str_val.chars().rev().collect::<String>()))
        }
        "replace" => {
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage("string replace", 5, args.len(), "string first last ?newString?"));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            let first = parse_index(args[3].as_str(), len).unwrap_or(0);
            let last = parse_index(args[4].as_str(), len).unwrap_or(len.saturating_sub(1));
            let new_str = if args.len() > 5 { args[5].as_str() } else { "" };
            if first > last || first >= len {
                return Ok(Value::from_str(str_val));
            }
            let mut result: String = chars[..first].iter().collect();
            result.push_str(new_str);
            if last + 1 < len {
                result.extend(&chars[last + 1..]);
            }
            Ok(Value::from_str(&result))
        }
        "is" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string is", 4, args.len()));
            }
            // string is class ?-strict? string
            // args[2] = class, args[3..] may include -strict, last arg is string
            let class = str_val; // args[2]
            let test_val = args[args.len() - 1].as_str();
            let result = match class {
                "integer" | "int" | "wideinteger" => test_val.parse::<i64>().is_ok(),
                "double" | "real" => test_val.parse::<f64>().is_ok(),
                "boolean" | "bool" | "true" | "false" => {
                    matches!(test_val.to_lowercase().as_str(),
                        "1" | "0" | "true" | "false" | "yes" | "no" | "on" | "off")
                }
                "alpha" => !test_val.is_empty() && test_val.chars().all(|c| c.is_alphabetic()),
                "alnum" => !test_val.is_empty() && test_val.chars().all(|c| c.is_alphanumeric()),
                "digit" => !test_val.is_empty() && test_val.chars().all(|c| c.is_ascii_digit()),
                "upper" => !test_val.is_empty() && test_val.chars().all(|c| c.is_uppercase()),
                "lower" => !test_val.is_empty() && test_val.chars().all(|c| c.is_lowercase()),
                "space" => !test_val.is_empty() && test_val.chars().all(|c| c.is_whitespace()),
                "ascii" => !test_val.is_empty() && test_val.is_ascii(),
                "print" => !test_val.is_empty() && test_val.chars().all(|c| !c.is_control()),
                "control" => !test_val.is_empty() && test_val.chars().all(|c| c.is_control()),
                "xdigit" => !test_val.is_empty() && test_val.chars().all(|c| c.is_ascii_hexdigit()),
                "graph" => !test_val.is_empty() && test_val.chars().all(|c| !c.is_whitespace() && !c.is_control()),
                "punct" => !test_val.is_empty() && test_val.chars().all(|c| c.is_ascii_punctuation()),
                "list" => Value::from_str(test_val).as_list().is_some(),
                _ => return Err(Error::runtime(
                    format!("bad class \"{}\": must be alnum, alpha, ascii, boolean, control, digit, double, graph, integer, list, lower, print, punct, space, upper, wideinteger, or xdigit", class),
                    crate::error::ErrorCode::InvalidOp,
                )),
            };
            Ok(Value::from_bool(result))
        }
        "cat" => {
            // string cat str1 ?str2 ...?
            let mut result = String::new();
            for arg in &args[2..] {
                result.push_str(arg.as_str());
            }
            Ok(Value::from_str(&result))
        }
        "byterange" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "string byterange", 5, args.len(), "string first last",
                ));
            }
            let bytes = str_val.as_bytes();
            let len = bytes.len();
            let first = parse_index(args[3].as_str(), len).unwrap_or(0);
            let last = parse_index(args[4].as_str(), len)
                .unwrap_or(len.saturating_sub(1));
            if first <= last && first < len {
                let end = (last + 1).min(len);
                let s = String::from_utf8_lossy(&bytes[first..end]);
                Ok(Value::from_str(&s))
            } else {
                Ok(Value::empty())
            }
        }
        "wordstart" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "string wordstart", 4, args.len(), "string charIndex",
                ));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            let idx = parse_index(args[3].as_str(), len).unwrap_or(0).min(len.saturating_sub(1));
            let mut start = idx;
            while start > 0 && is_word_char(chars[start - 1]) {
                start -= 1;
            }
            Ok(Value::from_int(start as i64))
        }
        "wordend" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "string wordend", 4, args.len(), "string charIndex",
                ));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            let idx = parse_index(args[3].as_str(), len).unwrap_or(0).min(len.saturating_sub(1));
            let mut end = idx;
            while end < len && is_word_char(chars[end]) {
                end += 1;
            }
            Ok(Value::from_int(end as i64))
        }
        _ => Err(Error::runtime(
            format!("unknown string subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

/// Parse -nocase / -length N / -- options and return (nocase, length, str1, str2).
fn parse_string_opts(args: &[Value]) -> Result<(bool, Option<i64>, String, String)> {
    let mut i = 2;
    let mut nocase = false;
    let mut length: Option<i64> = None;
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-length" => {
                i += 1;
                if i >= args.len() {
                    return Err(Error::runtime(
                        "missing value for -length",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                length = Some(args[i].as_int().ok_or_else(|| {
                    Error::runtime(
                        format!("expected integer but got \"{}\"", args[i].as_str()),
                        crate::error::ErrorCode::Generic,
                    )
                })?);
                i += 1;
            }
            "--" => { i += 1; break; }
            _ => break,
        }
    }
    if i + 1 >= args.len() {
        return Err(Error::wrong_args("string", 4, args.len()));
    }
    Ok((nocase, length, args[i].as_str().to_string(), args[i + 1].as_str().to_string()))
}

/// A word character: alphanumeric or underscore (jimtcl convention).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // -- string equal -length --

    #[test]
    fn test_string_equal_length() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string equal -length 3 "abcdef" "abcxyz""#).unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_string_equal_length_nocase() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string equal -nocase -length 3 "ABCdef" "abcxyz""#).unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_string_equal_length_mismatch() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string equal -length 4 "abcdef" "abcxyz""#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    // -- string compare -length --

    #[test]
    fn test_string_compare_length() {
        let mut interp = Interp::new();
        // First 2 chars of "abc" and "abd" are both "ab" → equal
        let r = interp.eval(r#"string compare -length 2 "abc" "abd""#).unwrap();
        assert_eq!(r.as_str(), "0");
        // First 3 chars differ: "abc" < "abd"
        let r2 = interp.eval(r#"string compare -length 3 "abc" "abd""#).unwrap();
        assert_eq!(r2.as_str(), "-1");
    }

    #[test]
    fn test_string_compare_nocase_length() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string compare -nocase -length 5 "Hello World" "HELLO THERE""#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    // -- string byterange --

    #[test]
    fn test_string_byterange() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string byterange "hello" 0 2"#).unwrap();
        assert_eq!(r.as_str(), "hel");
    }

    #[test]
    fn test_string_byterange_end() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string byterange "hello" 2 end"#).unwrap();
        assert_eq!(r.as_str(), "llo");
    }

    // -- string wordstart / wordend --

    #[test]
    fn test_string_wordstart() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordstart "hello world foo" 6"#).unwrap();
        assert_eq!(r.as_str(), "6");
    }

    #[test]
    fn test_string_wordstart_at_word_begin() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordstart "hello world" 0"#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    #[test]
    fn test_string_wordend() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordend "hello world foo" 6"#).unwrap();
        assert_eq!(r.as_str(), "11");
    }

    #[test]
    fn test_string_wordend_at_end() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordend "hello" 0"#).unwrap();
        assert_eq!(r.as_str(), "5");
    }
}
