//! String commands: string subcommands.

use crate::error::{Error, Result};
use crate::interp::commands::list::{bad_index, parse_tcl_index, tcl_get_int};
use crate::interp::unicode;
use crate::interp::{glob_match, Interp};
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
            // Tcl_UtfToTitle (tclUtf.c): title-case the first character,
            // lowercase the rest — with one explicit exception: Georgian
            // Mtavruli (U+1C90..U+1CBF) are left untouched in the tail
            // ("Special exception for Georgian Asomtavruli chars, no
            // titlecase"), even though `string tolower` maps them to
            // Mkhedruli.
            let mut chars = str_val.chars();
            let result = match chars.next() {
                Some(first) => {
                    let mut s = String::new();
                    s.push(to_titlecase(first));
                    for c in chars.as_str().chars() {
                        // Georgian Mtavruli: exempt from tail lowercasing.
                        if ('\u{1C90}'..='\u{1CBF}').contains(&c) {
                            s.push(c);
                        } else {
                            s.push(to_lower_simple(c));
                        }
                    }
                    s
                }
                None => String::new(),
            };
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
            let len = chars.len() as i64;
            let start_raw = match parse_tcl_index(args[3].as_str(), chars.len()) {
                Some(v) => v,
                None => return Err(bad_index(_interp, args[3].as_str())),
            };
            let end_raw = match parse_tcl_index(args[4].as_str(), chars.len()) {
                Some(v) => v,
                None => return Err(bad_index(_interp, args[4].as_str())),
            };
            // Tcl clamps: start < 0 → 0; end >= len → len-1; empty when
            // the range is backwards or starts past the end.
            let start = start_raw.max(0);
            let end = end_raw.min(len - 1);
            if start <= end && start < len {
                let s: String = chars[start as usize..=end as usize].iter().collect();
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
            match parse_tcl_index(args[3].as_str(), len) {
                Some(idx) if idx >= 0 && (idx as usize) < len => {
                    Ok(Value::from_str(&chars[idx as usize].to_string()))
                }
                Some(_) => Ok(Value::empty()),
                None => Err(bad_index(_interp, args[3].as_str())),
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
            // string first needleString haystackString ?startIndex?
            let needle = args[2].as_str();
            let haystack = args[3].as_str();
            let hchars: Vec<char> = haystack.chars().collect();
            let start = if args.len() > 4 {
                match parse_tcl_index(args[4].as_str(), hchars.len()) {
                    Some(v) => v.max(0) as usize,
                    None => return Err(bad_index(_interp, args[4].as_str())),
                }
            } else {
                0
            };
            let pos = find_chars(&hchars, needle, start)
                .map(|i| i as i64)
                .unwrap_or(-1);
            Ok(Value::from_int(pos))
        }
        "last" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string last", 4, args.len()));
            }
            // string last needleString haystackString ?lastIndex?
            let needle = args[2].as_str();
            let haystack = args[3].as_str();
            let hchars: Vec<char> = haystack.chars().collect();
            // A match must end at or before lastIndex (default: end of string).
            let last = if args.len() > 4 {
                match parse_tcl_index(args[4].as_str(), hchars.len()) {
                    // Negative index: no character is at or before it.
                    Some(i) if i < 0 => return Ok(Value::from_int(-1)),
                    Some(i) => (i as usize).min(hchars.len().saturating_sub(1)),
                    None => return Err(bad_index(_interp, args[4].as_str())),
                }
            } else {
                hchars.len().saturating_sub(1)
            };
            let pos = rfind_chars(&hchars, needle, last)
                .map(|i| i as i64)
                .unwrap_or(-1);
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
            // Tcl_GetInt semantics — a single (trimmed) integer literal,
            // no `int+int` chains: `string repeat ab 1+2` errors with
            // `expected integer but got "1+2"` (probed).
            let count = match tcl_get_int(args[3].as_str()) {
                Some(n) => n,
                None => {
                    return Err(Error::runtime(
                        format!("expected integer but got \"{}\"", args[3].as_str()),
                        crate::error::ErrorCode::InvalidOp,
                    ))
                }
            };
            if count < 0 {
                return Ok(Value::empty());
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
            let len = chars.len() as i64;
            let first_raw = match parse_tcl_index(args[3].as_str(), chars.len()) {
                Some(v) => v,
                None => return Err(bad_index(_interp, args[3].as_str())),
            };
            let last_raw = match parse_tcl_index(args[4].as_str(), chars.len()) {
                Some(v) => v,
                None => return Err(bad_index(_interp, args[4].as_str())),
            };
            let first = first_raw.max(0);
            let last = last_raw.min(len - 1);
            let new_str = if args.len() > 5 { args[5].as_str() } else { "" };
            if first > last || first >= len {
                return Ok(Value::from_str(str_val));
            }
            let mut result: String = chars[..first as usize].iter().collect();
            result.push_str(new_str);
            if last + 1 < len {
                result.extend(&chars[(last + 1) as usize..]);
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
                "integer" | "int" | "wideinteger" => is_tcl_integer(test_val),
                "double" | "real" => test_val.parse::<f64>().is_ok(),
                "boolean" | "bool" | "true" | "false" => is_tcl_boolean(test_val),
                // Unicode classes follow tclsh's category tables (see
                // interp::unicode): digit is any Nd, control is Cc|Cf, etc.
                "alpha" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_alpha(c)),
                "alnum" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_alnum(c)),
                "digit" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_digit(c)),
                "upper" => !test_val.is_empty() && test_val.chars().all(|c| c.is_uppercase()),
                "lower" => !test_val.is_empty() && test_val.chars().all(|c| c.is_lowercase()),
                "space" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_space(c)),
                "ascii" => !test_val.is_empty() && test_val.is_ascii(),
                "print" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_print(c)),
                "control" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_control(c)),
                "xdigit" => !test_val.is_empty() && test_val.chars().all(|c| c.is_ascii_hexdigit()),
                "graph" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_graph(c)),
                "punct" => !test_val.is_empty() && test_val.chars().all(|c| unicode::is_punct(c)),
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
            let len = bytes.len() as i64;
            let first_raw = match parse_tcl_index(args[3].as_str(), bytes.len()) {
                Some(v) => v,
                None => return Err(bad_index(_interp, args[3].as_str())),
            };
            let last_raw = match parse_tcl_index(args[4].as_str(), bytes.len()) {
                Some(v) => v,
                None => return Err(bad_index(_interp, args[4].as_str())),
            };
            let first = first_raw.max(0);
            let last = last_raw.min(len - 1);
            if first <= last && first < len {
                let end = (last + 1).min(len);
                let s = String::from_utf8_lossy(&bytes[first as usize..end as usize]);
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
            let idx = match parse_tcl_index(args[3].as_str(), len) {
                Some(v) => v.max(0).min(len as i64 - 1).max(0) as usize,
                None => return Err(bad_index(_interp, args[3].as_str())),
            };
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
            let idx = match parse_tcl_index(args[3].as_str(), len) {
                Some(v) => v.max(0).min(len as i64 - 1).max(0) as usize,
                None => return Err(bad_index(_interp, args[3].as_str())),
            };
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

/// Find the first occurrence of `needle` in `haystack` at or after char index
/// `start`. Returns the char index of the match. An empty needle never matches
/// (Tcl returns -1).
fn find_chars(haystack: &[char], needle: &str, start: usize) -> Option<usize> {
    let nchars: Vec<char> = needle.chars().collect();
    if nchars.is_empty() || start >= haystack.len() || nchars.len() > haystack.len() {
        return None;
    }
    (start..=haystack.len() - nchars.len()).find(|&i| haystack[i..i + nchars.len()] == nchars[..])
}

/// Find the last occurrence of `needle` in `haystack` that ends at or before
/// char index `last`. Returns the char index of the match start.
fn rfind_chars(haystack: &[char], needle: &str, last: usize) -> Option<usize> {
    let nchars: Vec<char> = needle.chars().collect();
    if nchars.is_empty() || nchars.len() > haystack.len() {
        return None;
    }
    let max_start = (last + 1).min(haystack.len()).checked_sub(nchars.len())?;
    (0..=max_start)
        .rev()
        .find(|&i| haystack[i..i + nchars.len()] == nchars[..])
}

/// Tcl titlecase (UCS4ToTitle, tclUtf.c): only Ll characters with a simple
/// (1:1) uppercase move; Lu and Lt characters are already "titled", and
/// characters whose uppercase expands to multiple chars (ß, ﬀ, ...) have no
/// entry in Tcl's table, so they stay. Georgian Mkhedruli (U+10D0..U+10FF)
/// carry Tcl's special case mode 0x7: `string toupper` maps them to Mtavruli
/// but their titlecase is themselves.
fn to_titlecase(c: char) -> char {
    // The uppercase paired digraphs (ǄǇǊǱ, table mode 0x3) titlecase to the
    // following title form (+1), even though they are Lu and all other Lu
    // characters are returned unchanged.
    match c {
        'Ǆ' => return 'ǅ',
        'Ǉ' => return 'ǈ',
        'Ǌ' => return 'ǋ',
        'Ǳ' => return 'ǲ',
        _ => {}
    }
    if ('\u{10D0}'..='\u{10FF}').contains(&c) || !unicode::is_ll(c) {
        return c;
    }
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => match u {
            // Digraph letters: Tcl's titlecase is the following title form
            // (ǳ→ǲ, ǆ→ǅ, ǉ→ǈ, ǌ→ǋ), not the plain capital digraph.
            'Ǳ' => 'ǲ',
            'Ǆ' => 'ǅ',
            'Ǉ' => 'ǈ',
            'Ǌ' => 'ǋ',
            _ => u,
        },
        _ => c,
    }
}

/// Tcl's simple per-character lowercase (TclUCS4ToLower): the first char of
/// the (possibly full) mapping is the simple mapping in every current
/// Unicode case-expansion (e.g. U+0130 -> "i\u{307}", simple U+0069).
fn to_lower_simple(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Tcl boolean: 0/1 or an unambiguous prefix of true/false/yes/no/on/off
/// (case-insensitive).
fn is_tcl_boolean(s: &str) -> bool {
    if s == "0" || s == "1" {
        return true;
    }
    if s.is_empty() {
        return false;
    }
    let lower = s.to_lowercase();
    const WORDS: [&str; 6] = ["true", "false", "yes", "no", "on", "off"];
    WORDS.iter().filter(|w| w.starts_with(lower.as_str())).count() == 1
}

/// Tcl integer syntax: optional ASCII whitespace, optional sign, then decimal,
/// legacy octal (`010`), or 0x/0b/0o-prefixed digits; value must fit in i64.
fn is_tcl_integer(s: &str) -> bool {
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return false;
    }
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() {
        return false;
    }
    let (base, digits) = if let Some(rest) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        (16, rest)
    } else if let Some(rest) = digits.strip_prefix("0b").or_else(|| digits.strip_prefix("0B")) {
        (2, rest)
    } else if let Some(rest) = digits.strip_prefix("0o").or_else(|| digits.strip_prefix("0O")) {
        (8, rest)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (8, digits)
    } else {
        (10, digits)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(base)) {
        return false;
    }
    match u64::from_str_radix(digits, base) {
        Ok(mag) if !neg => mag <= i64::MAX as u64,
        Ok(mag) => mag <= (i64::MAX as u64) + 1,
        Err(_) => false,
    }
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
