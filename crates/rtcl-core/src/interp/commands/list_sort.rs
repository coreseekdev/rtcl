//! List search and sort commands: lsearch, lsort.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;

pub fn cmd_lsearch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("lsearch", 3, args.len(), "?options? list pattern"));
    }

    #[derive(PartialEq, Clone, Copy)]
    enum MatchMode { Exact, Glob, Regexp }

    let mut i = 1;
    let mut mode = MatchMode::Glob;
    let mut all = false;
    let mut inline = false;
    let mut not_match = false;
    let mut nocase = false;
    let mut bool_mode = false;
    let mut command: Option<String> = None;
    let mut stride: usize = 1;
    let mut index: Option<String> = None;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-exact" => { mode = MatchMode::Exact; i += 1; }
            "-glob" => { mode = MatchMode::Glob; i += 1; }
            "-regexp" => { mode = MatchMode::Regexp; i += 1; }
            "-all" => { all = true; i += 1; }
            "-inline" => { inline = true; i += 1; }
            "-not" => { not_match = true; i += 1; }
            "-nocase" => { nocase = true; i += 1; }
            "-bool" => { bool_mode = true; i += 1; }
            "-command" => {
                i += 1;
                if i >= args.len() - 2 {
                    return Err(Error::runtime(
                        "missing value for -command",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                command = Some(args[i].as_str().to_string());
                i += 1;
            }
            "-stride" => {
                i += 1;
                if i >= args.len() - 2 {
                    return Err(Error::runtime(
                        "missing value for -stride",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                stride = args[i].as_int().unwrap_or(1) as usize;
                if stride < 1 {
                    return Err(Error::runtime(
                        "stride must be >= 1",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                i += 1;
            }
            "-index" => {
                i += 1;
                if i >= args.len() - 2 {
                    return Err(Error::runtime(
                        "missing value for -index",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                index = Some(args[i].as_str().to_string());
                i += 1;
            }
            "--" => { i += 1; break; }
            other => {
                return Err(Error::runtime(
                    format!("bad option \"{}\": must be -all, -bool, -command, -exact, -glob, -index, -inline, -nocase, -not, -regexp, -stride, or --", other),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
    }

    if i + 1 >= args.len() {
        return Err(Error::wrong_args("lsearch", 3, args.len()));
    }

    let list = args[i].as_list().unwrap_or_default();
    let pattern = args[i + 1].as_str();

    // Validate stride
    if stride > 1 && !list.len().is_multiple_of(stride) {
        return Err(Error::runtime(
            format!("list size must be a multiple of the stride length"),
            crate::error::ErrorCode::Generic,
        ));
    }

    // Extract the element to compare (respecting -index and -stride)
    let extract_key = |elem: &Value| -> String {
        if let Some(ref idx_str) = index {
            let sub = elem.as_list().unwrap_or_else(|| vec![elem.clone()]);
            let idx_num: usize = if idx_str == "end" {
                sub.len().saturating_sub(1)
            } else if let Some(rest) = idx_str.strip_prefix("end-") {
                sub.len().saturating_sub(1 + rest.parse::<usize>().unwrap_or(0))
            } else {
                idx_str.parse().unwrap_or(0)
            };
            sub.get(idx_num).map(|v| v.as_str().to_string()).unwrap_or_default()
        } else {
            elem.as_str().to_string()
        }
    };

    // Compile regex once if needed
    #[cfg(feature = "regexp")]
    let re = if mode == MatchMode::Regexp {
        let pat = if nocase { format!("(?i){}", pattern) } else { pattern.to_string() };
        Some(regex::Regex::new(&pat).map_err(|e| {
            Error::runtime(format!("invalid regexp: {}", e), crate::error::ErrorCode::Generic)
        })?)
    } else {
        None
    };

    // Match function
    let do_match = |key: &str| -> Result<bool> {
        let m = match mode {
            MatchMode::Exact => {
                if nocase {
                    key.to_lowercase() == pattern.to_lowercase()
                } else {
                    key == pattern
                }
            }
            MatchMode::Glob => {
                if nocase {
                    glob_match(&pattern.to_lowercase(), &key.to_lowercase())
                } else {
                    glob_match(pattern, key)
                }
            }
            MatchMode::Regexp => {
                #[cfg(feature = "regexp")]
                {
                    re.as_ref().map(|r| r.is_match(key)).unwrap_or(false)
                }
                #[cfg(not(feature = "regexp"))]
                {
                    return Err(Error::runtime(
                        "lsearch -regexp requires 'regexp' feature",
                        crate::error::ErrorCode::InvalidOp,
                    ));
                }
            }
        };
        Ok(if not_match { !m } else { m })
    };

    // -command mode
    let mut do_command_match = |key: &str| -> Result<bool> {
        if let Some(ref cmd) = command {
            let script = format!("{} {} {}",
                cmd,
                crate::value::tcl_quote(pattern),
                crate::value::tcl_quote(key),
            );
            let r = interp.eval(&script)?;
            let matched = r.is_true();
            Ok(if not_match { !matched } else { matched })
        } else {
            do_match(key)
        }
    };

    // Iterate by stride groups
    let mut result_indices: Vec<usize> = Vec::new();
    let step = stride;
    let mut group_idx = 0;
    while group_idx < list.len() {
        // The comparison element is the first element of the group (or indexed element)
        let compare_elem = &list[group_idx];
        let key = extract_key(compare_elem);
        let matched = if command.is_some() {
            do_command_match(&key)?
        } else {
            do_match(&key)?
        };
        if matched {
            result_indices.push(group_idx);
            if !all {
                break;
            }
        }
        group_idx += step;
    }

    // Format output
    if bool_mode {
        return Ok(Value::from_bool(!result_indices.is_empty()));
    }

    if inline {
        if stride > 1 {
            let mut result = Vec::new();
            for &idx in &result_indices {
                let end = (idx + stride).min(list.len());
                result.extend(list[idx..end].iter().cloned());
            }
            Ok(Value::from_list(&result))
        } else {
            let result: Vec<Value> = result_indices.iter()
                .map(|&idx| list[idx].clone())
                .collect();
            Ok(Value::from_list(&result))
        }
    } else if all {
        let result: Vec<Value> = result_indices.iter()
            .map(|&idx| Value::from_int(idx as i64))
            .collect();
        Ok(Value::from_list(&result))
    } else {
        Ok(Value::from_int(result_indices.first().map(|&idx| idx as i64).unwrap_or(-1)))
    }
}

pub fn cmd_lsort(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("lsort", 2, args.len(), "?options? list"));
    }

    let mut i = 1;
    let mut decreasing = false;
    let mut unique = false;
    let mut nocase = false;
    let mut sort_type = SortType::Ascii;
    let mut command: Option<String> = None;
    let mut index: Option<String> = None;
    let mut stride: usize = 1;

    while i < args.len() - 1 && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-decreasing" => { decreasing = true; i += 1; }
            "-increasing" => { decreasing = false; i += 1; }
            "-unique" => { unique = true; i += 1; }
            "-nocase" => { nocase = true; i += 1; }
            "-ascii" => { sort_type = SortType::Ascii; i += 1; }
            "-dictionary" => { sort_type = SortType::Dictionary; i += 1; }
            "-integer" => { sort_type = SortType::Integer; i += 1; }
            "-real" => { sort_type = SortType::Real; i += 1; }
            "-command" => {
                i += 1;
                if i >= args.len() - 1 {
                    return Err(Error::runtime(
                        "missing value for -command",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                command = Some(args[i].as_str().to_string());
                i += 1;
            }
            "-index" => {
                i += 1;
                if i >= args.len() - 1 {
                    return Err(Error::runtime(
                        "missing value for -index",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                index = Some(args[i].as_str().to_string());
                i += 1;
            }
            "-stride" => {
                i += 1;
                if i >= args.len() - 1 {
                    return Err(Error::runtime(
                        "missing value for -stride",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                stride = args[i].as_int().unwrap_or(1) as usize;
                if stride < 2 {
                    return Err(Error::runtime(
                        "stride length must be at least 2",
                        crate::error::ErrorCode::Generic,
                    ));
                }
                i += 1;
            }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args("lsort", 2, args.len()));
    }

    let list = args[i].as_list().unwrap_or_default();

    // Validate stride
    if stride > 1 && !list.is_empty() && !list.len().is_multiple_of(stride) {
        return Err(Error::runtime(
            "list size must be a multiple of the stride length",
            crate::error::ErrorCode::Generic,
        ));
    }

    // Helper: extract sort key from element (for -index in non-stride mode)
    let get_key = |elem: &Value| -> Value {
        if let Some(ref idx) = index {
            let sub = elem.as_list().unwrap_or_else(|| vec![elem.clone()]);
            let idx_num: usize = if idx == "end" {
                sub.len().saturating_sub(1)
            } else if let Some(rest) = idx.strip_prefix("end-") {
                sub.len().saturating_sub(1 + rest.parse::<usize>().unwrap_or(0))
            } else {
                idx.parse().unwrap_or(0)
            };
            sub.get(idx_num).cloned().unwrap_or_else(Value::empty)
        } else {
            elem.clone()
        }
    };

    // Comparison function for non-command sort
    let compare_values = |ka: &Value, kb: &Value| -> std::cmp::Ordering {
        let cmp = match sort_type {
            SortType::Integer => {
                let ai = ka.as_int().unwrap_or(0);
                let bi = kb.as_int().unwrap_or(0);
                ai.cmp(&bi)
            }
            SortType::Real => {
                let af = ka.as_str().parse::<f64>().unwrap_or(0.0);
                let bf = kb.as_str().parse::<f64>().unwrap_or(0.0);
                af.partial_cmp(&bf).unwrap_or(std::cmp::Ordering::Equal)
            }
            SortType::Ascii => {
                if nocase {
                    ka.as_str().to_lowercase().cmp(&kb.as_str().to_lowercase())
                } else {
                    ka.as_str().cmp(kb.as_str())
                }
            }
            SortType::Dictionary => {
                dict_compare(ka.as_str(), kb.as_str())
            }
        };
        if decreasing { cmp.reverse() } else { cmp }
    };

    if stride > 1 {
        // Stride mode: sort groups of `stride` elements
        let mut groups: Vec<Vec<Value>> = list.chunks(stride)
            .map(|c| c.to_vec())
            .collect();

        // In stride mode, -index selects within the group itself
        let stride_key = |group: &[Value]| -> Value {
            if let Some(ref idx) = index {
                let idx_num: usize = if idx == "end" {
                    group.len().saturating_sub(1)
                } else if let Some(rest) = idx.strip_prefix("end-") {
                    group.len().saturating_sub(
                        1 + rest.parse::<usize>().unwrap_or(0),
                    )
                } else {
                    idx.parse().unwrap_or(0)
                };
                group.get(idx_num).cloned().unwrap_or_else(Value::empty)
            } else {
                group[0].clone()
            }
        };

        if let Some(ref cmd_name) = command {
            let cmd = cmd_name.clone();
            let mut err: Option<Error> = None;
            groups.sort_by(|a, b| {
                if err.is_some() { return std::cmp::Ordering::Equal; }
                let ka = stride_key(a);
                let kb = stride_key(b);
                let script = format!("{} {} {}", cmd,
                    crate::value::tcl_quote(ka.as_str()),
                    crate::value::tcl_quote(kb.as_str()),
                );
                match interp.eval(&script) {
                    Ok(v) => {
                        let n = v.as_int().unwrap_or(0);
                        let cmp = n.cmp(&0);
                        if decreasing { cmp.reverse() } else { cmp }
                    }
                    Err(e) => { err = Some(e); std::cmp::Ordering::Equal }
                }
            });
            if let Some(e) = err { return Err(e); }
        } else {
            groups.sort_by(|a, b| {
                let ka = stride_key(a);
                let kb = stride_key(b);
                compare_values(&ka, &kb)
            });
        }

        let mut result: Vec<Value> = Vec::with_capacity(list.len());
        for group in groups {
            result.extend(group);
        }

        if unique {
            // Unique based on key of first element in each group
            let mut deduped: Vec<Value> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for chunk in result.chunks(stride) {
                let key = get_key(&chunk[0]).as_str().to_string();
                if seen.insert(key) {
                    deduped.extend(chunk.iter().cloned());
                }
            }
            return Ok(Value::from_list(&deduped));
        }

        return Ok(Value::from_list(&result));
    }

    // Non-stride mode
    let mut list = list;

    if let Some(ref cmd_name) = command {
        // -command: use a Tcl proc for comparison
        let cmd = cmd_name.clone();
        let mut err: Option<Error> = None;
        list.sort_by(|a, b| {
            if err.is_some() {
                return std::cmp::Ordering::Equal;
            }
            let ka = get_key(a);
            let kb = get_key(b);
            let script = format!("{} {} {}", cmd,
                crate::value::tcl_quote(ka.as_str()),
                crate::value::tcl_quote(kb.as_str()),
            );
            match interp.eval(&script) {
                Ok(v) => {
                    let n = v.as_int().unwrap_or(0);
                    let cmp = n.cmp(&0);
                    if decreasing { cmp.reverse() } else { cmp }
                }
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
    } else {
        list.sort_by(|a, b| {
            let ka = get_key(a);
            let kb = get_key(b);
            compare_values(&ka, &kb)
        });
    }

    if unique {
        let mut seen = std::collections::HashSet::new();
        list.retain(|v| seen.insert(v.as_str().to_string()));
    }

    Ok(Value::from_list(&list))
}

#[derive(Clone, Copy)]
enum SortType {
    Ascii,
    Dictionary,
    Integer,
    Real,
}

/// Dictionary-order comparison (jimtcl behavior):
/// - Letters compare case-insensitively
/// - Numeric segments compare by value (so "a2" < "a10")
/// - Non-alnum characters compare by ASCII value
fn dict_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let mut ai = 0;
    let mut bi = 0;
    while ai < ac.len() && bi < bc.len() {
        let ca = ac[ai];
        let cb = bc[bi];
        // Both digits → compare numeric segments by value
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            // Extract numbers
            let mut na: u64 = 0;
            while ai < ac.len() && ac[ai].is_ascii_digit() {
                na = na.saturating_mul(10).saturating_add(ac[ai].to_digit(10).unwrap_or(0) as u64);
                ai += 1;
            }
            let mut nb: u64 = 0;
            while bi < bc.len() && bc[bi].is_ascii_digit() {
                nb = nb.saturating_mul(10).saturating_add(bc[bi].to_digit(10).unwrap_or(0) as u64);
                bi += 1;
            }
            match na.cmp(&nb) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        // Case-insensitive letter comparison
        let la = ca.to_ascii_lowercase();
        let lb = cb.to_ascii_lowercase();
        match la.cmp(&lb) {
            std::cmp::Ordering::Equal => {
                // Tiebreak on original case (uppercase < lowercase)
                match ca.cmp(&cb) {
                    std::cmp::Ordering::Equal => {}
                    other => {
                        // Continue but remember the tiebreak
                        ai += 1;
                        bi += 1;
                        // Only apply tiebreak if rest is equal
                        let rest = dict_compare(
                            &ac[ai..].iter().collect::<String>(),
                            &bc[bi..].iter().collect::<String>(),
                        );
                        if rest == std::cmp::Ordering::Equal {
                            return other;
                        }
                        return rest;
                    }
                }
            }
            other => return other,
        }
        ai += 1;
        bi += 1;
    }
    ac.len().cmp(&bc.len())
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // -- lsearch tests --

    #[test]
    fn test_lsearch_nocase() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -nocase {Hello World} hello").unwrap();
        assert_eq!(r.as_str(), "0");
    }

    #[test]
    fn test_lsearch_nocase_glob() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -nocase -glob {Hello World Foo} w*").unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_lsearch_bool() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -bool {a b c} b").unwrap();
        assert_eq!(r.as_str(), "1");
        let r2 = interp.eval("lsearch -bool {a b c} z").unwrap();
        assert_eq!(r2.as_str(), "0");
    }

    #[cfg(feature = "regexp")]
    #[test]
    fn test_lsearch_regexp() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"lsearch -regexp {abc def 123} {^\d+$}"#).unwrap();
        assert_eq!(r.as_str(), "2");
    }

    #[cfg(feature = "regexp")]
    #[test]
    fn test_lsearch_regexp_all_inline() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"lsearch -all -inline -regexp {abc def 123} {[a-z]+}"#).unwrap();
        assert_eq!(r.as_str(), "abc def");
    }

    #[cfg(feature = "regexp")]
    #[test]
    fn test_lsearch_regexp_nocase() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"lsearch -regexp -nocase {ABC def GHI} {^abc$}"#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    #[test]
    fn test_lsearch_index() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -index 1 {{a 1} {b 2} {c 3}} 2").unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_lsearch_stride() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -stride 2 {k1 v1 k2 v2 k3 v3} k2").unwrap();
        assert_eq!(r.as_str(), "2");
    }

    #[test]
    fn test_lsearch_stride_inline() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -stride 2 -inline {k1 v1 k2 v2 k3 v3} k2").unwrap();
        assert_eq!(r.as_str(), "k2 v2");
    }

    #[test]
    fn test_lsearch_command() {
        let mut interp = Interp::new();
        interp.eval("proc mycmp {a b} { string equal $a $b }").unwrap();
        let r = interp.eval("lsearch -command mycmp {a b c} b").unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_lsearch_existing_exact() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -exact {a b c} b").unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_lsearch_existing_all() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -all {a b a c a} a").unwrap();
        assert_eq!(r.as_str(), "0 2 4");
    }

    #[test]
    fn test_lsearch_existing_not() {
        let mut interp = Interp::new();
        let r = interp.eval("lsearch -all -not {a b c} a").unwrap();
        assert_eq!(r.as_str(), "1 2");
    }

    // -- lsort tests --

    #[test]
    fn test_lsort_dictionary() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -dictionary {a10 a2 a1}").unwrap();
        assert_eq!(r.as_str(), "a1 a2 a10");
    }

    #[test]
    fn test_lsort_dictionary_case() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -dictionary {bigBoy Bigboy bigboy}").unwrap();
        assert_eq!(r.as_str(), "bigBoy Bigboy bigboy");
    }

    #[test]
    fn test_lsort_stride() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -stride 2 {c 3 a 1 b 2}").unwrap();
        assert_eq!(r.as_str(), "a 1 b 2 c 3");
    }

    #[test]
    fn test_lsort_stride_index() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -stride 2 -index 1 -integer {c 3 a 1 b 2}").unwrap();
        assert_eq!(r.as_str(), "a 1 b 2 c 3");
    }

    #[test]
    fn test_lsort_stride_bad_length() {
        let mut interp = Interp::new();
        // 5 elements not divisible by stride 2
        assert!(interp.eval("lsort -stride 2 {a b c d e}").is_err());
    }

    #[test]
    fn test_lsort_existing_decreasing() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -decreasing {a b c}").unwrap();
        assert_eq!(r.as_str(), "c b a");
    }

    #[test]
    fn test_lsort_existing_integer() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -integer {10 2 1 20}").unwrap();
        assert_eq!(r.as_str(), "1 2 10 20");
    }
}
