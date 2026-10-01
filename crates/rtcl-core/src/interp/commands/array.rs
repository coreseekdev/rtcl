//! Array commands: array set/get/names/size/exists/unset plus the
//! startsearch/anymore/nextelement/donesearch/statistics machinery.

use crate::error::{Error, Result};
use crate::interp::{glob_match, split_array_ref, ArraySearch, ArraySearchList, Interp};
use crate::value::Value;

#[cfg(feature = "regexp")]
use regex::Regex;
#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
use regex_lite::Regex;

/// tclsh 8.6.17's `array` subcommand list, in the order it prints on an
/// unknown-or-ambiguous error.
const ARRAY_SUBCMDS: &[&str] = &[
    "anymore", "donesearch", "exists", "get", "names", "nextelement", "set", "size",
    "startsearch", "statistics", "unset",
];

/// Exact matches win; otherwise a unique prefix resolves, and anything
/// else gets tclsh's unknown-or-ambiguous error.
fn resolve_array_subcmd(raw: &str) -> Result<String> {
    if ARRAY_SUBCMDS.contains(&raw) {
        return Ok(raw.to_string());
    }
    let matches: Vec<&str> = ARRAY_SUBCMDS
        .iter()
        .copied()
        .filter(|s| s.starts_with(raw))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).to_string()),
        _ => Err(Error::runtime(
            format!(
                "unknown or ambiguous subcommand \"{}\": must be {}",
                raw,
                ARRAY_SUBCMDS.join(", ").replacen(", unset", ", or unset", 1)
            ),
            crate::error::ErrorCode::NotFound,
        )),
    }
}

fn usage(cmd: &str, sub: &str, usage: &str) -> Error {
    Error::wrong_args_with_usage(cmd, 3, 0, format!("{} {}", sub, usage))
}

/// Search commands require an actual array (missing or scalar both error).
fn require_array(interp: &Interp, array_name: &str) -> Result<()> {
    if interp.is_array_semantic(array_name) {
        Ok(())
    } else {
        Err(Error::runtime(
            format!("\"{}\" isn't an array", array_name),
            crate::error::ErrorCode::Generic,
        ))
    }
}

/// `s-<digits>-<name>` shaped identifiers report "couldn't find search";
/// anything else is an "illegal search identifier".
/// Shape classification of a search id against the array being queried.
enum IdShape {
    /// `s-<digits>-<arrayName>` — plausible id for this array.
    ForArray,
    /// `s-<digits>-<other>` — well-formed, but names a different variable
    /// (set-old-10.9).
    ForOther,
    /// Anything else.
    Illegal,
}

fn classify_id(id: &str, array_name: &str) -> IdShape {
    match id.strip_prefix("s-").and_then(|r| r.split_once('-')) {
        Some((n, name)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            if name == array_name {
                IdShape::ForArray
            } else {
                IdShape::ForOther
            }
        }
        _ => IdShape::Illegal,
    }
}

/// Ok if `id` names an active, uninvalidated search on the array.
fn validate_search(interp: &Interp, array_name: &str, id: &str) -> Result<()> {
    let key = interp.array_stamp_key(array_name);
    let stamp = interp.array_stamp(array_name);
    if let Some(v) = interp.array_searches.get(&key) {
        if let Some(s) = v.active.iter().find(|s| s.rendered == id) {
            if s.stamp == stamp {
                return Ok(());
            }
        }
    }
    match classify_id(id, array_name) {
        IdShape::ForArray => Err(Error::runtime(
            format!("couldn't find search \"{}\"", id),
            crate::error::ErrorCode::NotFound,
        )),
        IdShape::ForOther => Err(Error::runtime(
            format!("search identifier \"{}\" isn't for variable \"{}\"", id, array_name),
            crate::error::ErrorCode::Generic,
        )),
        IdShape::Illegal => Err(Error::runtime(
            format!("illegal search identifier \"{}\"", id),
            crate::error::ErrorCode::Generic,
        )),
    }
}

pub fn cmd_array(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "array", 2, args.len(), "subcommand ?arg ...?",
        ));
    }

    let subcmd = resolve_array_subcmd(args[1].as_str())?;
    let array_name = if args.len() > 2 { args[2].as_str() } else { "" };

    match subcmd.as_str() {
        "anymore" | "donesearch" | "nextelement" => {
            if args.len() != 4 {
                let u = match subcmd.as_str() {
                    "anymore" => "anymore arrayName searchId",
                    "donesearch" => "donesearch arrayName searchId",
                    _ => "nextelement arrayName searchId",
                };
                return Err(Error::wrong_args_with_usage(
                    "array", 4, args.len(), u,
                ));
            }
            require_array(interp, array_name)?;
            let id = args[3].as_str();
            validate_search(interp, array_name, id)?;
            let key = interp.array_stamp_key(array_name);
            let list = interp.array_searches.get_mut(&key).unwrap();
            let search = list.active.iter_mut().find(|s| s.rendered == id).unwrap();
            match subcmd.as_str() {
                "anymore" => Ok(Value::from_bool(!search.elements.is_empty())),
                "donesearch" => {
                    list.active.retain(|s| s.rendered != id);
                    // tclsh: an emptied search list resets the id counter.
                    if list.active.is_empty() {
                        list.ctr = 0;
                    }
                    Ok(Value::empty())
                }
                _ => {
                    if search.elements.is_empty() {
                        Ok(Value::empty())
                    } else {
                        Ok(Value::from_str(search.elements.remove(0).as_str()))
                    }
                }
            }
        }
        "startsearch" => {
            if args.len() != 3 {
                return Err(usage("array", "startsearch", "arrayName"));
            }
            require_array(interp, array_name)?;
            let key = interp.array_stamp_key(array_name);
            let stamp = interp.array_stamp(array_name);
            let elements = interp.array_element_names(array_name);
            let list = interp
                .array_searches
                .entry(key)
                .or_insert_with(|| ArraySearchList { ctr: 0, active: Vec::new() });
            list.ctr += 1;
            let n = list.ctr;
            let rendered = format!("s-{}-{}", n, array_name);
            list.active.push(ArraySearch {
                id: n,
                rendered: rendered.clone(),
                elements,
                stamp,
            });
            Ok(Value::from_str(&rendered))
        }
        "statistics" => {
            if args.len() != 3 {
                return Err(usage("array", "statistics", "arrayName"));
            }
            require_array(interp, array_name)?;
            let n = interp.array_element_names(array_name).len();
            Ok(Value::from_str(&format!(
                "{} entries in table, {} buckets\n\
                 number of buckets with 0 entries: 0\n\
                 number of buckets with 1 entries: {}\n\
                 number of buckets with 2 entries: 0\n\
                 number of buckets with 3 entries: 0\n\
                 number of buckets with 4 entries: 0\n\
                 number of buckets with 5 entries: 0\n\
                 number of buckets with 6 entries: 0\n\
                 number of buckets with 7 entries: 0\n\
                 number of buckets with 8 entries: 0\n\
                 number of buckets with 9 entries: 0\n\
                 number of buckets with 10 or more entries: 0\n\
                 average search distance for entry: 1.0",
                n, n, n
            )))
        }
        "set" => {
            if args.len() != 4 {
                return Err(usage("array", "set", "arrayName list"));
            }
            // tclsh looks the variable up (creating) before parsing the
            // pair list, so the parent-namespace error wins over a
            // malformed list.
            interp.check_parent_ns(array_name)?;
            let list = super::list::strict_list(interp, &args[3])?;
            if !list.len().is_multiple_of(2) {
                return Err(Error::runtime(
                    "list must have an even number of elements",
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
            // tclsh quirk: with an empty pair list the upfront scalar
            // check fires ("can't array set"), with pairs the first
            // element write produces "can't set" via set_var (set-old-8.35).
            // An empty list on a MISSING variable still creates the array
            // (set-old-8.38: `info exists` -> 1, read -> "is array").
            let created =
                !interp.is_array_semantic(array_name) && !interp.var_exists(array_name);
            if !interp.is_array_semantic(array_name) && interp.var_exists(array_name)
                && list.is_empty()
            {
                return Err(Error::runtime(
                    format!("can't array set \"{}\": variable isn't array", array_name),
                    crate::error::ErrorCode::Generic,
                ));
            }
            if created && list.is_empty() {
                interp.mark_array(array_name)?;
                // 'array' traces fire when `array set` creates the
                // array (tclsh probe: not on plain element writes).
                let _ = interp.fire_traces(array_name, None, "array");
                return Ok(Value::empty());
            }
            for chunk in list.chunks(2) {
                let var_name = format!("{}({})", array_name, chunk[0].as_str());
                interp.set_var(&var_name, chunk[1].clone())?;
            }
            if created {
                let _ = interp.fire_traces(array_name, None, "array");
            }
            Ok(Value::empty())
        }
        "get" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(usage("array", "get", "arrayName ?pattern?"));
            }
            let pattern = if args.len() > 3 { Some(args[3].as_str()) } else { None };
            let mut result: Vec<Value> = Vec::new();
            // Resolve through upvar links to the owning table (`upvar a
            // x; array get x` lists the target's elements).
            let (owner_fi, base) = interp.array_owner(array_name);
            let prefix = format!("{}(", base);
            let table = match owner_fi {
                Some(i) => &interp.frames[i].locals,
                None => &interp.globals,
            };
            // Element-name snapshot: callbacks that add elements during
            // the get don't join this enumeration (trace-1.11: the read
            // trace's `set x(foo) 1` never shows up in the same get).
            let vars: Vec<String> = table
                .keys()
                .filter_map(|k| {
                    if k.starts_with(&prefix) && k.ends_with(')') {
                        let elem = &k[prefix.len()..k.len() - 1];
                        match pattern {
                            Some(pat) if glob_match(pat, elem) => Some(elem.to_string()),
                            Some(_) => None,
                            None => Some(elem.to_string()),
                        }
                    } else {
                        None
                    }
                })
                .collect();
            for elem in vars {
                // Each element is read through the traced access path —
                // per-element read traces fire with tclsh's interleaved
                // abort/skip semantics (`array names`/`array size` read
                // no values and fire none).
                match interp.array_get_element(owner_fi, &base, &elem) {
                    Ok(Some(val)) => {
                        result.push(Value::from_str(&elem));
                        result.push(val);
                    }
                    Ok(None) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(Value::from_list(&result))
        }
        "names" => {
            // array names arrayName ?mode? ?pattern?
            if args.len() < 3 || args.len() > 5 {
                return Err(usage("array", "names", "arrayName ?mode? ?pattern?"));
            }
            let mut mode = "glob";
            let mut pattern: Option<&str> = None;
            if args.len() > 3 {
                let first = args[3].as_str();
                if args.len() > 4 {
                    // Two extras: ?mode? ?pattern? — the first must be a mode.
                    mode = match first {
                        "-exact" => "exact",
                        "-glob" => "glob",
                        "-regexp" => "regexp",
                        _ => {
                            return Err(Error::runtime(
                                format!(
                                    "bad option \"{}\": must be -exact, -glob, or -regexp",
                                    first
                                ),
                                crate::error::ErrorCode::Generic,
                            ))
                        }
                    };
                    pattern = Some(args[4].as_str());
                } else {
                    // One extra: it is always a PATTERN (set-old-8.53:
                    // `array names a -regexp` matches the element "-regexp").
                    pattern = Some(first);
                }
            }
            // tclsh quirk: `array names` on a MISSING array still fires
            // 'array' traces (trace-5.8); size/get do not. The trace script
            // may create elements, which then show up in the listing.
            if !interp.is_array_semantic(array_name) && !interp.var_exists(array_name) {
                let name = array_name.to_string();
                let _ = interp.fire_traces(&name, None, "array");
            }
            let prefix = format!(
                "{}(",
                interp.array_owner(array_name).1
            );
            let matches_elem = |elem: &str| -> bool {
                match pattern {
                    None => true,
                    Some(pat) => match mode {
                        "exact" => elem == pat,
                        "regexp" => {
                            #[cfg(feature = "regexp")]
                            {
                                Regex::new(pat).map(|re| re.is_match(elem)).unwrap_or(false)
                            }
                            #[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
                            {
                                regex_lite::Regex::new(pat)
                                    .map(|re| re.is_match(elem))
                                    .unwrap_or(false)
                            }
                            #[cfg(not(any(feature = "regexp", feature = "regexp-lite")))]
                            {
                                let _ = elem;
                                false
                            }
                        }
                        _ => glob_match(pat, elem),
                    },
                }
            };
            let (owner_fi, _) = interp.array_owner(array_name);
            let table = match owner_fi {
                Some(i) => &interp.frames[i].locals,
                None => &interp.globals,
            };
            let names: Vec<Value> = table
                .keys()
                .filter_map(|k| {
                    if k.starts_with(&prefix) && k.ends_with(')') {
                        let elem = &k[prefix.len()..k.len() - 1];
                        if matches_elem(elem) {
                            Some(Value::from_str(elem))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
            Ok(Value::from_list(&names))
        }
        "size" => {
            if args.len() != 3 {
                return Err(usage("array", "size", "arrayName"));
            }
            let (owner_fi, base) = interp.array_owner(array_name);
            let prefix = format!("{}(", base);
            let table = match owner_fi {
                Some(i) => &interp.frames[i].locals,
                None => &interp.globals,
            };
            let count = table
                .keys()
                .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                .count();
            Ok(Value::from_int(count as i64))
        }
        "exists" => {
            if args.len() != 3 {
                return Err(usage("array", "exists", "arrayName"));
            }
            // Semantic check: an array stays alive after its last element
            // is unset (registry marker), a scalar is not an array.
            let (owner_fi, base) = interp.array_owner(array_name);
            let prefix = format!("{}(", base);
            let table = match owner_fi {
                Some(i) => &interp.frames[i].locals,
                None => &interp.globals,
            };
            let exists = interp.is_array_semantic(array_name)
                || table
                    .keys()
                    .any(|k| k.starts_with(&prefix) && k.ends_with(')'));
            Ok(Value::from_bool(exists))
        }
        "unset" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(usage("array", "unset", "arrayName ?pattern?"));
            }
            let (owner_fi, base) = interp.array_owner(array_name);
            let prefix = format!("{}(", base);
            let pattern = if args.len() > 3 { Some(args[3].as_str()) } else { None };
            let table = match owner_fi {
                Some(i) => &interp.frames[i].locals,
                None => &interp.globals,
            };
            let keys_to_remove: Vec<String> = table
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
                let _ = interp.unset_var(&k);
            }
            Ok(Value::empty())
        }
        _ => unreachable!("resolve_array_subcmd validated the subcommand"),
    }
}

// Make split_array_ref accessible.  Re-export it from interp so array
// consumers don't need to import interp directly.
#[allow(dead_code)]
pub(crate) fn is_array_ref(name: &str) -> bool {
    split_array_ref(name).is_some()
}

#[cfg(test)]
mod search_tests {
    use crate::interp::Interp;

    /// tclsh: search ids are `s-<n>-<arrayName>` with n = smallest free.
    #[test]
    fn test_startsearch_ids_smallest_free() {
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(a) 1; \
                 list [array star a] [array startsearch a] \
                      [array done a s-1-a; array startsearch a] \
                      [array done a s-2-a; array done a s-3-a; array start a]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "s-1-a s-2-a s-3-a s-1-a");
    }

    #[test]
    fn test_nextelement_iteration_and_exhaustion() {
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(a) 1; set a(b) 1; set a(c) 1; \
                 set x [array startsearch a]; \
                 set n1 [array next a $x]; set n2 [array ne a $x]; \
                 set n3 [array nextelement a $x]; \
                 list [lsort [list $n1 $n2 $n3]] [array next a $x] [array next a $x]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "{a b c} {} {}");
    }

    #[test]
    fn test_anymore_peek() {
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(a) 1; set x [array startsearch a]; \
                 list [array anymore a $x] [array next a $x] [array anymore a $x]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "1 a 0");
    }

    #[test]
    fn test_done_then_next_errors() {
        let mut interp = Interp::new();
        let e = interp
            .eval(
                "set a(a) 1; set x [array startsearch a]; array done a $x; \
                 catch {array next a $x} m; set m",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(e, "couldn't find search \"s-1-a\"");
    }

    #[test]
    fn test_illegal_search_identifier() {
        let mut interp = Interp::new();
        let e = interp
            .eval("set a(a) 1; catch {array nextelement a bogus} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(e, "illegal search identifier \"bogus\"");
    }

    /// Adding an element invalidates existing searches; overwriting or a
    /// failed unset does not.
    #[test]
    fn test_search_invalidation() {
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(a) 1; set x [array startsearch a]; set a(b) 1; \
                 catch {array next a $x} m; set m",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "couldn't find search \"s-1-a\"");

        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(a) 1; set x [array startsearch a]; set a(a) 2; \
                 catch {array next a $x} m; list $m [catch {array next a $x}]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "a 0");

        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(a) 1; set x [array startsearch a]; catch {unset a(c)}; \
                 catch {array next a $x} m; list $m [catch {array next a $x}]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "a 0");
    }

    /// Missing or scalar arrayName → "X" isn't an array.
    #[test]
    fn test_search_isnt_array() {
        let mut interp = Interp::new();
        for expr in [
            "catch {array anymore a b} m",
            "catch {array donesearch a b} m",
            "catch {array nextelement a b} m",
            "catch {array startsearch a} m",
        ] {
            let e = interp
                .eval(&format!("catch {{unset a}}; {}; set m", expr))
                .unwrap()
                .as_str()
                .to_string();
            assert_eq!(e, "\"a\" isn't an array", "expr={}", expr);
        }
        // scalar
        let e = interp
            .eval("set a 44; catch {array startsearch a} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(e, "\"a\" isn't an array");
    }

    /// tclsh usage strings (wrong # args) incl. prefix-resolved subcommands.
    #[test]
    fn test_array_usage_strings() {
        let mut interp = Interp::new();
        let cases: &[(&str, &str)] = &[
            ("catch {array} m", "wrong # args: should be \"array subcommand ?arg ...?\""),
            ("catch {array get} m", "wrong # args: should be \"array get arrayName ?pattern?\""),
            ("catch {array get a b c} m", "wrong # args: should be \"array get arrayName ?pattern?\""),
            ("catch {array exists a b} m", "wrong # args: should be \"array exists arrayName\""),
            ("catch {array names a 4 5} m", "bad option \"4\": must be -exact, -glob, or -regexp"),
            ("catch {array names a b c d} m", "wrong # args: should be \"array names arrayName ?mode? ?pattern?\""),
            ("catch {array start a b} m", "wrong # args: should be \"array startsearch arrayName\""),
            ("catch {array start} m", "wrong # args: should be \"array startsearch arrayName\""),
            ("catch {array d} m", "wrong # args: should be \"array donesearch arrayName searchId\""),
            ("catch {array e a b} m", "wrong # args: should be \"array exists arrayName\""),
            ("catch {array u a b c} m", "wrong # args: should be \"array unset arrayName ?pattern?\""),
        ];
        for (expr, expected) in cases {
            let m = interp.eval(&format!("{}; set m", expr)).unwrap().as_str().to_string();
            assert_eq!(m, *expected, "expr={}", expr);
        }
    }

    /// Prefix ambiguity mirrors tclsh's canonical subcommand list.
    #[test]
    fn test_array_prefix_ambiguity() {
        let mut interp = Interp::new();
        let e = interp
            .eval("catch {array sta a} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(
            e,
            "unknown or ambiguous subcommand \"sta\": must be anymore, donesearch, exists, get, names, nextelement, set, size, startsearch, statistics, or unset"
        );
        let e = interp
            .eval("catch {array for a} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(
            e,
            "unknown or ambiguous subcommand \"for\": must be anymore, donesearch, exists, get, names, nextelement, set, size, startsearch, statistics, or unset"
        );
    }

    #[test]
    fn test_unset_no_args_ok() {
        // set-old-7.2: bare `unset` is a no-op, not an error
        let mut interp = Interp::new();
        assert_eq!(interp.eval("catch {unset} m").unwrap().as_str(), "0");
    }

    #[test]
    fn test_unset_dashed_name() {
        // set-old-7.14: `--` is an end-of-options marker, not a name —
        // `unset --` alone is a no-op; `unset -- --` unsets var "--".
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set -- abc; list [info exists --] [catch {unset --}] [info exists --] \
                 [catch {unset -- --}] [info exists --]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "1 0 1 0 0");
    }

    #[test]
    fn test_unset_dashed_nocomplain_combo() {
        // set-old-7.15: `unset -- -nocomplain` unsets the var "-nocomplain"
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set -nocomplain abc; set -- abc; \
                 list [info exists -nocomplain] [catch {unset -- -nocomplain}] \
                 [info exists -nocomplain] [info exists --] \
                 [catch {unset -- -nocomplain}] [info exists --] \
                 [catch {unset -- --}] [info exists --]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "1 0 0 1 1 1 0 0");
    }

    #[test]
    fn test_unset_missing_var_errors() {
        // set-old-7.3: unset of a missing variable errors with the
        // canonical message; catch sees code 1.
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("catch {unset a} m; list $m [catch {unset a}]")
                .unwrap()
                .as_str(),
            "{can't unset \"a\": no such variable} 1"
        );
    }

    #[test]
    fn test_unset_stops_at_first_missing_name() {
        // set-old-7.6: error names the FIRST failing variable in the list
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("set a 1; catch {unset a a a(14)} m; list $m [info exists a]")
                .unwrap()
                .as_str(),
            "{can't unset \"a\": no such variable} 0"
        );
    }

    #[test]
    fn test_array_set_empty_creates_array() {
        // set-old-8.38: `array set q {}` on a missing variable creates
        // the array (exists, but plain reads report the array conflict).
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "array set q {}; \
                 list [info exists q] [catch {set q} m] $m [array size q]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "1 1 {can't read \"q\": variable is array} 0");
    }

    #[test]
    fn test_array_set_strict_list_parse() {
        // set-old-8.34: malformed pair list raises the list parse error
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval(r#"catch {array set a "a \{ c"} m; set m"#)
                .unwrap()
                .as_str(),
            "unmatched open brace in list"
        );
    }

    #[test]
    fn test_names_single_extra_is_pattern() {
        // set-old-8.53/8.54/8.55: with ONE extra arg it is always a
        // pattern, even when it looks like a mode flag.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set a(-glob) 1; set a(-regexp) 1; set a(-exact) 1; \
                 list [array names a -regexp] [array names a -exact] [array names a -glob]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "-regexp -exact -glob");
    }

    #[test]
    fn test_names_two_extras_require_mode() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("array set a {x 1}; catch {array names a -bogus x} m; set m")
                .unwrap()
                .as_str(),
            "bad option \"-bogus\": must be -exact, -glob, or -regexp"
        );
    }

    #[test]
    fn test_parent_namespace_write_check() {
        // set-old-8.38.5/6/7: writes into a missing namespace error;
        // reads, info exists, and unset stay plain misses.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "list [catch {set bogus::x 1} m1] $m1 [set ::errorCode] \
                 [catch {array set bogus::a {}} m2] $m2 \
                 [info exists bogus::x] [catch {unset bogus::x} m3] \
                 [catch {set bogus::x} m4]",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(
            r,
            "1 {can't set \"bogus::x\": parent namespace doesn't exist} {TCL LOOKUP VARNAME bogus::x} 1 {can't set \"bogus::a\": parent namespace doesn't exist} 0 1 1"
        );
    }

    #[test]
    fn test_parent_namespace_existing_ns_ok() {
        let mut interp = Interp::new();
        let r = interp
            .eval("namespace eval ok {variable v}; set ok::v 1; set ok::other 2; list $ok::v $ok::other")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "1 2");
    }
}
