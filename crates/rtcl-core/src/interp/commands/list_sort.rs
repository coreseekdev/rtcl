//! List search and sort commands: lsearch, lsort.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;

/// The lsearch option table, in tclsh 8.6.17's order. `-bool`, `-command`
/// and `-stride` are rtcl extensions: accepted (and prefix-matched), but
/// matching tclsh they never appear in the bad-option error message.
const LSEARCH_OPTIONS: [&str; 21] = [
    "-all", "-ascii", "-bisect", "-bool", "-command", "-decreasing",
    "-dictionary", "-exact", "-glob", "-increasing", "-index", "-inline",
    "-integer", "-nocase", "-not", "-real", "-regexp", "-sorted", "-start",
    "-stride", "-subindices",
];

/// The documented (tclsh 8.6.17) option list, for error messages.
fn lsearch_option_list() -> String {
    let listed: Vec<&str> = LSEARCH_OPTIONS
        .iter()
        .copied()
        .filter(|o| !matches!(*o, "-bool" | "-command" | "-stride"))
        .collect();
    let mut s = String::from("must be ");
    for (i, o) in listed.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        if i + 1 == listed.len() {
            s.push_str("or ");
        }
        s.push_str(o);
    }
    s
}

pub fn cmd_lsearch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    use super::list::{bad_index, parse_tcl_index, set_error_code, strict_list, tcl_err, tcl_get_int, trim_tcl_space, scan_tcl_int};
    use crate::value::is_tcl_space;
    use std::cmp::Ordering;

    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "lsearch",
            1,
            args.len(),
            "?-option value ...? list pattern",
        ));
    }

    #[derive(PartialEq, Clone, Copy)]
    enum Mode {
        Exact,
        Glob,
        Regexp,
        Sorted,
    }
    #[derive(PartialEq, Clone, Copy)]
    enum DataType {
        Ascii,
        Dictionary,
        Integer,
        Real,
    }

    let mut mode = Mode::Glob;
    let mut data = DataType::Ascii;
    let mut increasing = true;
    let mut all = false;
    let mut inline = false;
    let mut subindices = false;
    let mut not_match = false;
    let mut nocase = false;
    let mut bisect = false;
    let mut bool_mode = false;
    let mut command: Option<String> = None;
    let mut stride: i64 = 1;
    let mut start_str: Option<String> = None;
    let mut index_specs: Vec<IndexSpec> = Vec::new();

    // TclIndexEncode precheck for lsearch -index items: negative integers
    // and `end+N` are rejected with "index ... out of range" (lsort uses
    // its own "cannot select" wording).
    let validate_spec = |interp: &mut Interp, s: &str| -> Result<IndexSpec> {
        let reject = |interp: &mut Interp, s: &str| -> Result<IndexSpec> {
            set_error_code(interp, "TCL VALUE INDEX OUTOFRANGE");
            Err(tcl_err(format!("index \"{}\" out of range", s)))
        };
        let t = trim_tcl_space(s);
        if let Some(n) = tcl_get_int(t) {
            if n < 0 {
                return reject(interp, s);
            }
            return Ok(IndexSpec::Plain(n));
        }
        if let Some((first, used)) = scan_tcl_int(t) {
            let rest = &t[used..];
            let rb = rest.as_bytes();
            if rb.len() >= 2 && (rb[0] == b'+' || rb[0] == b'-') && !is_tcl_space(rb[1]) {
                if let Some(second) = tcl_get_int(&rest[1..]) {
                    let n = if rb[0] == b'+' {
                        first.saturating_add(second)
                    } else {
                        first.saturating_sub(second)
                    };
                    if n < 0 {
                        return reject(interp, s);
                    }
                    return Ok(IndexSpec::Plain(n));
                }
            }
        }
        if let Some(rest) = s.strip_prefix("end") {
            let off = if rest.is_empty() {
                Some(0i64)
            } else {
                let rb = rest.as_bytes();
                if rb[0] == b'+' || rb[0] == b'-' {
                    tcl_get_int(&rest[1..]).map(|v| if rb[0] == b'-' { -v } else { v })
                } else {
                    None
                }
            };
            if let Some(off) = off {
                if off > 0 {
                    return reject(interp, s);
                }
                return Ok(IndexSpec::End(off));
            }
        }
        Err(bad_index(interp, s))
    };

    // Option loop: tclsh parses options in positions 1..objc-2 only, and
    // every argument there must match the option table (prefix matching).
    let mut i = 1usize;
    while i < args.len() - 2 {
        let arg = args[i].as_str();
        let opt: &str = if let Some(exact) = LSEARCH_OPTIONS.iter().find(|o| **o == arg) {
            exact
        } else {
            let cands: Vec<&str> =
                LSEARCH_OPTIONS.iter().copied().filter(|o| o.starts_with(arg)).collect();
            match cands.len() {
                1 => cands[0],
                _ => {
                    let kind = if cands.is_empty() { "bad option" } else { "ambiguous option" };
                    set_error_code(interp, "TCL LOOKUP INDEX OPTION");
                    return Err(tcl_err(format!(
                        "{} \"{}\": {}",
                        kind,
                        arg,
                        lsearch_option_list()
                    )));
                }
            }
        };
        match opt {
            "-all" => all = true,
            "-ascii" => data = DataType::Ascii,
            "-bisect" => {
                mode = Mode::Sorted;
                bisect = true;
            }
            "-bool" => bool_mode = true,
            "-command" => {
                if i > args.len() - 4 {
                    set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(tcl_err("missing value for -command"));
                }
                i += 1;
                command = Some(args[i].as_str().to_string());
            }
            "-decreasing" => increasing = false,
            "-dictionary" => data = DataType::Dictionary,
            "-exact" => mode = Mode::Exact,
            "-glob" => mode = Mode::Glob,
            "-increasing" => increasing = true,
            "-index" => {
                if i > args.len() - 4 {
                    set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(tcl_err("\"-index\" option must be followed by list index"));
                }
                i += 1;
                let raw = strict_list(interp, &args[i])?;
                index_specs.clear();
                for iv in &raw {
                    index_specs.push(validate_spec(interp, iv.as_str())?);
                }
            }
            "-inline" => inline = true,
            "-integer" => data = DataType::Integer,
            "-nocase" => nocase = true,
            "-not" => not_match = true,
            "-real" => data = DataType::Real,
            "-regexp" => mode = Mode::Regexp,
            "-sorted" => mode = Mode::Sorted,
            "-start" => {
                if i > args.len() - 4 {
                    set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(tcl_err("missing starting index"));
                }
                i += 1;
                start_str = Some(args[i].as_str().to_string());
            }
            "-stride" => {
                if i > args.len() - 4 {
                    set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(tcl_err("\"-stride\" option must be followed by stride length"));
                }
                i += 1;
                let Some(w) = tcl_get_int(args[i].as_str()) else {
                    set_error_code(interp, "TCL VALUE NUMBER");
                    return Err(tcl_err(format!(
                        "expected integer but got \"{}\"",
                        args[i].as_str()
                    )));
                };
                if w < 1 {
                    set_error_code(interp, "TCL OPERATION LSEARCH BADSTRIDE");
                    return Err(tcl_err("stride length must be at least 1"));
                }
                stride = w;
            }
            "-subindices" => subindices = true,
            _ => unreachable!(),
        }
        i += 1;
    }

    if subindices && index_specs.is_empty() {
        set_error_code(interp, "TCL OPERATION LSEARCH BAD_OPTION_MIX");
        return Err(tcl_err("-subindices cannot be used without -index option"));
    }
    if bisect && (all || not_match) {
        set_error_code(interp, "TCL OPERATION LSEARCH BAD_OPTION_MIX");
        return Err(tcl_err("-bisect is not compatible with -all or -not"));
    }

    // Compile the regexp before the list parse (tclsh does this to avoid
    // shimmering between the regexp and list reps). Uses the dual engine:
    // fast `regex` crate first, backtracking fallback for patterns the
    // fast engine rejects.
    let pat_val = &args[args.len() - 1];
    #[cfg(feature = "regexp")]
    let re = if mode == Mode::Regexp {
        Some(
            super::regexp_cmds::compile_engine(pat_val.as_str(), nocase, false, false, false)
                .map_err(|e| {
                    tcl_err(format!("couldn't compile regular expression pattern: {}", e))
                })?,
        )
    } else {
        None
    };

    let list = strict_list(interp, &args[args.len() - 2])?;

    // -stride group sanity; with -index, the leading index value becomes
    // the offset of the compared element within each group (TIP #351).
    let mut group_offset: i64 = 0;
    if stride > 1 {
        if list.len() as i64 % stride != 0 {
            set_error_code(interp, "TCL OPERATION LSEARCH BADSTRIDE");
            return Err(tcl_err("list size must be a multiple of the stride length"));
        }
        if !index_specs.is_empty() {
            group_offset = index_specs[0].decode(stride - 1);
            if group_offset < 0 || group_offset >= stride {
                set_error_code(interp, "TCL OPERATION LSEARCH BADINDEX");
                return Err(tcl_err(
                    "when used with \"-stride\", the leading \"-index\" value must be within the group",
                ));
            }
            index_specs.remove(0);
        }
    }

    // Resolve -start; a start past the end short-circuits ("did not match
    // anything at all") before the pattern is even examined.
    let mut start: i64 = 0;
    if let Some(ref s) = start_str {
        let Some(v) = parse_tcl_index(s, list.len()) else {
            return Err(bad_index(interp, s));
        };
        start = v.max(0);
        if start as usize >= list.len() {
            if all || inline {
                return Ok(Value::from_str(""));
            }
            return Ok(Value::from_int(-1));
        }
        if stride > 1 {
            start -= start % stride;
        }
    }

    let pat_str = pat_val.as_str();
    let mut pat_wide: i64 = 0;
    let mut pat_double: f64 = 0.0;
    if matches!(mode, Mode::Exact | Mode::Sorted) {
        match data {
            DataType::Integer => match tcl_get_int(pat_str) {
                Some(v) => pat_wide = v,
                None => {
                    set_error_code(interp, "TCL VALUE NUMBER");
                    return Err(tcl_err(format!("expected integer but got \"{}\"", pat_str)));
                }
            },
            DataType::Real => match parse_tcl_double(pat_str) {
                Some(v) => pat_double = v,
                None => {
                    set_error_code(interp, "TCL VALUE NUMBER");
                    return Err(tcl_err(format!(
                        "expected floating-point number but got \"{}\"",
                        pat_str
                    )));
                }
            },
            _ => {}
        }
    }

    let wide_of = |item: &Value| -> Result<i64> {
        tcl_get_int(item.as_str())
            .ok_or_else(|| tcl_err(format!("expected integer but got \"{}\"", item.as_str())))
    };
    let real_of = |item: &Value| -> Result<f64> {
        parse_tcl_double(item.as_str()).ok_or_else(|| {
            tcl_err(format!("expected floating-point number but got \"{}\"", item.as_str()))
        })
    };

    let mut index: i64 = -1;
    let mut matched_any = false;
    let mut all_results: Vec<Value> = Vec::new();

    if mode == Mode::Sorted && !all && !not_match {
        // Binary search over sorted data; finds the leftmost match, or
        // (with -bisect) the last element <= pattern.
        let gs = stride;
        let mut lower: i64 = start - gs;
        let mut upper: i64 = list.len() as i64;
        while lower + gs != upper {
            let mut mid = (lower + upper) / 2;
            mid -= mid % gs;
            let ui = (mid + group_offset) as usize;
            let item = if index_specs.is_empty() {
                list[ui].clone()
            } else {
                select_from_sublist(interp, &list[ui], &index_specs)?
            };
            let cmp: i64 = match data {
                DataType::Ascii => {
                    if nocase {
                        utf_casecmp(pat_str, item.as_str())
                    } else {
                        match pat_str.as_bytes().cmp(item.as_str().as_bytes()) {
                            Ordering::Less => -1,
                            Ordering::Equal => 0,
                            Ordering::Greater => 1,
                        }
                    }
                }
                DataType::Dictionary => dict_compare(pat_str, item.as_str()),
                DataType::Integer => {
                    let w = wide_of(&item)?;
                    if pat_wide == w {
                        0
                    } else if pat_wide < w {
                        -1
                    } else {
                        1
                    }
                }
                DataType::Real => {
                    let d = real_of(&item)?;
                    if pat_double == d {
                        0
                    } else if pat_double < d {
                        -1
                    } else {
                        1
                    }
                }
            };
            if cmp == 0 {
                index = mid;
                if bisect {
                    lower = mid;
                } else {
                    upper = mid;
                }
            } else if cmp > 0 {
                if increasing {
                    lower = mid;
                } else {
                    upper = mid;
                }
            } else if increasing {
                upper = mid;
            } else {
                lower = mid;
            }
        }
        if bisect && index < 0 {
            index = lower;
        }
        matched_any = index >= 0;
    } else {
        // Linear scan: equal-only matchers, negation, or -all collection.
        let mut found: i64 = -1;
        let mut p = start;
        while (p as usize) < list.len() {
            let ui = p as usize + group_offset as usize;
            let item = if index_specs.is_empty() {
                list[ui].clone()
            } else {
                select_from_sublist(interp, &list[ui], &index_specs)?
            };
            let mut m = if let Some(ref cmd) = command {
                let script = format!(
                    "{} {} {}",
                    cmd,
                    crate::value::tcl_quote(pat_str),
                    crate::value::tcl_quote(item.as_str())
                );
                interp.eval(&script)?.is_true()
            } else {
                match mode {
                    Mode::Sorted | Mode::Exact => match data {
                        DataType::Ascii => {
                            let key = item.as_str();
                            key.len() == pat_str.len()
                                && if nocase {
                                    utf_casecmp(key, pat_str) == 0
                                } else {
                                    key == pat_str
                                }
                        }
                        DataType::Dictionary => dict_compare(item.as_str(), pat_str) == 0,
                        DataType::Integer => wide_of(&item)? == pat_wide,
                        DataType::Real => real_of(&item)? == pat_double,
                    },
                    Mode::Glob => {
                        if nocase {
                            glob_match(&pat_str.to_lowercase(), &item.as_str().to_lowercase())
                        } else {
                            glob_match(pat_str, item.as_str())
                        }
                    }
                    Mode::Regexp => {
                        #[cfg(feature = "regexp")]
                        {
                            re.as_ref()
                                .map(|eng| {
                                    super::regexp_cmds::engine_is_match(eng, item.as_str())
                                })
                                .unwrap_or(false)
                        }
                        #[cfg(not(feature = "regexp"))]
                        {
                            false
                        }
                    }
                }
            };
            if not_match {
                m = !m;
            }
            if m {
                matched_any = true;
                if !all {
                    found = p;
                    break;
                }
                if inline {
                    if subindices && !index_specs.is_empty() {
                        all_results.push(select_from_sublist(interp, &list[ui], &index_specs)?);
                    } else if subindices && stride > 1 {
                        all_results.push(list[ui].clone());
                    } else if stride > 1 {
                        let end = (ui + stride as usize).min(list.len());
                        all_results.extend(list[ui..end].iter().cloned());
                    } else {
                        all_results.push(list[ui].clone());
                    }
                } else if subindices {
                    // Displayed path entries decode `end` forms against
                    // listc (not listc-1) — tclsh 8.6's TclIndexDecode in
                    // the result path is passed the bare list length.
                    let mut path = vec![Value::from_int(p + group_offset)];
                    for spec in &index_specs {
                        path.push(Value::from_int(spec.decode(list.len() as i64)));
                    }
                    all_results.push(Value::from_list(&path));
                } else {
                    all_results.push(Value::from_int(p));
                }
            }
            p += stride;
        }
        index = found;
    }

    if bool_mode {
        return Ok(Value::from_bool(matched_any));
    }
    if all {
        return Ok(Value::from_list(&all_results));
    }
    if !inline {
        if subindices {
            let mut path = vec![Value::from_int(index + group_offset)];
            for spec in &index_specs {
                path.push(Value::from_int(spec.decode(list.len() as i64)));
            }
            return Ok(Value::from_list(&path));
        }
        return Ok(Value::from_int(index));
    }
    if index < 0 {
        return Ok(Value::from_str(""));
    }
    if subindices {
        return select_from_sublist(interp, &list[(index + group_offset) as usize], &index_specs);
    }
    if stride > 1 {
        let ui = index as usize;
        let end = (ui + stride as usize).min(list.len());
        return Ok(Value::from_list(&list[ui..end]));
    }
    Ok(list[index as usize].clone())
}

// ============================================================================
// lsort — faithful port of Tcl_LsortObjCmd + MergeLists + SortCompare +
// DictionaryCompare + SelectObjFromSublist (tclCmdIL.c, Tcl 8.6).
// ============================================================================

/// Binary-counter merge sort bucket count (tclCmdIL.c NUM_LISTS).
const NUM_LISTS: usize = 30;

/// Comparison modes (SORTMODE_*). AsciiNc is ascii + -nocase.
#[derive(Clone, Copy, PartialEq)]
enum SortMode {
    Ascii,
    AsciiNc,
    Dictionary,
    Integer,
    Real,
    Command,
}

/// The extracted sort key of one element (collationKey).
enum Key {
    Str(String),
    Wide(i64),
    Real(f64),
    Obj(Value),
}

impl Key {
    fn as_str(&self) -> &str {
        match self { Key::Str(s) => s, _ => unreachable!("string key expected") }
    }
    fn as_wide(&self) -> i64 {
        match self { Key::Wide(n) => *n, _ => unreachable!("integer key expected") }
    }
    fn as_real(&self) -> f64 {
        match self { Key::Real(f) => *f, _ => unreachable!("real key expected") }
    }
    fn as_obj(&self) -> &Value {
        match self { Key::Obj(v) => v, _ => unreachable!("object key expected") }
    }
}

/// Result representation: either the element itself or its original index
/// (used by -indices and -stride rebuilds).
enum Payload {
    Obj(Value),
    Index(i64),
}

struct SortElement {
    key: Key,
    payload: Payload,
    next: Option<usize>,
}

/// Comparison context threaded through the merges.
struct SortState {
    mode: SortMode,
    increasing: bool,
    unique: bool,
    /// `-command` words (already list-quoted), with room for the two
    /// comparison arguments appended at eval time.
    command_prefix: String,
    /// First error raised by a comparison command; latched per SortCompare.
    error: Option<Error>,
}

const LSORT_OPTIONS: [&str; 12] = [
    "-ascii", "-command", "-decreasing", "-dictionary", "-increasing",
    "-index", "-indices", "-integer", "-nocase", "-real", "-stride", "-unique",
];

fn option_must_be() -> String {
    let mut s = String::from("must be ");
    for (i, o) in LSORT_OPTIONS.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        if i + 1 == LSORT_OPTIONS.len() {
            s.push_str("or ");
        }
        s.push_str(o);
    }
    s
}

/// Parse a Tcl double the way Tcl_GetDoubleFromObj does for our purposes:
/// whitespace-trimmed decimal float, signed, with hex integers and
/// Inf/Infinity/NaN accepted.
fn parse_tcl_double(s: &str) -> Option<f64> {
    let t = super::list::trim_tcl_space(s);
    let (sign, body) = match t.as_bytes().first() {
        Some(b'+') => (1.0f64, &t[1..]),
        Some(b'-') => (-1.0f64, &t[1..]),
        _ => (1.0f64, t),
    };
    let lower = body.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        let v = i64::from_str_radix(hex, 16).ok()?;
        return Some(sign * v as f64);
    }
    match lower.as_str() {
        "inf" | "infinity" => return Some(sign * f64::INFINITY),
        "nan" => return Some(f64::NAN),
        _ => {}
    }
    body.parse::<f64>().ok()
}

/// One parsed `-index` entry: plain integer, or `end` offset (negative
/// offset means `end-N`; positive would be `end+N`, rejected upstream).
#[derive(Clone, Copy)]
enum IndexSpec {
    Plain(i64),
    End(i64),
}

impl IndexSpec {
    /// TclIndexDecode against `end_value` (= length - 1).
    fn decode(&self, end_value: i64) -> i64 {
        match self {
            IndexSpec::Plain(n) => *n,
            IndexSpec::End(delta) => end_value + delta,
        }
    }
}

/// TclIndexEncode precheck (lsort flavor): syntax errors get the
/// `bad index` message; anything that would select before the start
/// (negative ints, `end+N`) gets the `cannot select` error.
fn validate_index_spec(interp: &mut Interp, s: &str) -> Result<IndexSpec> {
    use crate::value::is_tcl_space;
    const CANNOT_SELECT: &str = "cannot select an element from any list";
    let reject = |interp: &mut Interp, s: &str| -> Result<IndexSpec> {
        super::list::set_error_code(interp, "TCL VALUE INDEXOUTOFRANGE");
        Err(super::list::tcl_err(format!("index \"{}\" {}", s, CANNOT_SELECT)))
    };
    // TclGetIntForIndex grammar: integer?[+-]integer? or end?[+-]integer?
    let t = super::list::trim_tcl_space(s);
    if let Some(n) = super::list::tcl_get_int(t) {
        if n < 0 {
            return reject(interp, s);
        }
        return Ok(IndexSpec::Plain(n));
    }
    // integer[+-]integer (e.g. "1+3" — official test cmdIL-3.5.1).
    if let Some((first, used)) = super::list::scan_tcl_int(t) {
        let rest = &t[used..];
        let rb = rest.as_bytes();
        if rb.len() >= 2
            && (rb[0] == b'+' || rb[0] == b'-')
            && !is_tcl_space(rb[1])
        {
            if let Some(second) = super::list::tcl_get_int(&rest[1..]) {
                let n = if rb[0] == b'+' {
                    first.saturating_add(second)
                } else {
                    first.saturating_sub(second)
                };
                if n < 0 {
                    return reject(interp, s);
                }
                return Ok(IndexSpec::Plain(n));
            }
        }
    }
    if let Some(rest) = s.strip_prefix("end") {
        let off = if rest.is_empty() {
            Some(0i64)
        } else {
            let rb = rest.as_bytes();
            if rb[0] == b'+' || rb[0] == b'-' {
                super::list::tcl_get_int(&rest[1..])
                    .map(|v| if rb[0] == b'-' { -v } else { v })
            } else {
                None
            }
        };
        if let Some(off) = off {
            if off > 0 {
                return reject(interp, s);
            }
            return Ok(IndexSpec::End(off));
        }
    }
    Err(super::list::bad_index(interp, s))
}

/// SelectObjFromSublist: walk `-index` specs through nested sublists.
fn select_from_sublist(interp: &mut Interp, obj: &Value, specs: &[IndexSpec]) -> Result<Value> {
    let mut cur = obj.clone();
    for spec in specs {
        let elems = super::list::strict_list(interp, &cur)?;
        let idx = spec.decode(elems.len() as i64 - 1);
        if idx < 0 {
            super::list::set_error_code(interp, "TCL OPERATION LSORT INDEXFAILED");
            let off = match spec { IndexSpec::End(o) => -*o, IndexSpec::Plain(_) => 0 };
            return Err(super::list::tcl_err(format!(
                "element end-{} missing from sublist \"{}\"",
                off,
                cur.as_str()
            )));
        }
        if idx as usize >= elems.len() {
            super::list::set_error_code(interp, "TCL OPERATION LSORT INDEXFAILED");
            return Err(super::list::tcl_err(format!(
                "element {} missing from sublist \"{}\"",
                idx,
                cur.as_str()
            )));
        }
        cur = elems[idx as usize].clone();
    }
    Ok(cur)
}

/// TclUtfCasecmp: codepoint-by-codepoint on lowercased characters.
fn utf_casecmp(a: &str, b: &str) -> i64 {
    let mut ia = a.chars().map(|c| c.to_lowercase().next().unwrap_or(c));
    let mut ib = b.chars().map(|c| c.to_lowercase().next().unwrap_or(c));
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return 0,
            // Any remaining character sorts above end-of-string — even a
            // NUL (`cmdIL-4.37`: `a\0a` follows `a`).
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(x), Some(y)) => {
                if x != y {
                    return x as i64 - y as i64;
                }
            }
        }
    }
}

/// Decode one UTF-8 scalar at `i`; on malformed input fall back to a single
/// byte (TclUtfToUniChar tolerates and replaces).
fn next_char(b: &[u8], i: usize) -> (char, usize) {
    let max = (i + 4).min(b.len());
    for n in (1..=max - i).rev() {
        if let Ok(s) = std::str::from_utf8(&b[i..i + n]) {
            if let Some(c) = s.chars().next() {
                return (c, i + c.len_utf8());
            }
        }
    }
    ('\u{FFFD}', i + 1)
}

/// DictionaryCompare, ported line-for-line from tclCmdIL.c: embedded digit
/// runs compare numerically (leading zeros as a secondary tiebreak), letters
/// compare lowercased, with uppercase-before-lowercase as another secondary
/// tiebreak.
fn dict_compare(l: &str, r: &str) -> i64 {
    let (lb, rb) = (l.as_bytes(), r.as_bytes());
    let at = |b: &[u8], i: usize| -> u8 { *b.get(i).unwrap_or(&0) };
    let is_d = |b: &[u8], i: usize| -> bool { at(b, i).is_ascii_digit() };
    let mut li = 0usize;
    let mut ri = 0usize;
    let mut secondary: i64 = 0;

    'outer: loop {
        if is_d(rb, ri) && is_d(lb, li) {
            // Both strings sit on a digit run: skip leading zeros (counting
            // them as a secondary difference), then compare the runs by
            // length and digits without converting to integers.
            let mut zeros: i64 = 0;
            while at(rb, ri) == b'0' && is_d(rb, ri + 1) {
                ri += 1;
                zeros -= 1;
            }
            while at(lb, li) == b'0' && is_d(lb, li + 1) {
                li += 1;
                zeros += 1;
            }
            if secondary == 0 {
                secondary = zeros;
            }

            let mut diff: i64 = 0;
            loop {
                if diff == 0 {
                    diff = at(lb, li) as i64 - at(rb, ri) as i64;
                }
                ri += 1;
                li += 1;
                if !is_d(rb, ri) {
                    if is_d(lb, li) {
                        return 1;
                    }
                    if diff != 0 {
                        return diff;
                    }
                    break; // equal numbers: resume the outer scan
                } else if !is_d(lb, li) {
                    return -1;
                }
            }
            continue 'outer;
        }

        if li < lb.len() && ri < rb.len() {
            let (cl, nl) = next_char(lb, li);
            let (cr, nr) = next_char(rb, ri);
            li = nl;
            ri = nr;
            let ll = cl.to_lowercase().next().unwrap_or(cl);
            let rl = cr.to_lowercase().next().unwrap_or(cr);
            let diff = ll as i64 - rl as i64;
            if diff != 0 {
                return diff;
            }
            if secondary == 0 {
                if cl.is_uppercase() && cr.is_lowercase() {
                    secondary = -1;
                } else if cr.is_uppercase() && cl.is_lowercase() {
                    secondary = 1;
                }
            }
        } else {
            // At least one string is exhausted: byte compare at the NULs.
            let diff = at(lb, li) as i64 - at(rb, ri) as i64;
            if diff != 0 {
                return diff;
            }
            // Both sit on a NUL (real or pad): the longer string sorts
            // last (`a` precedes `a\0a`, cmdIL-4.37).
            return match (lb.len() - li).cmp(&(rb.len() - ri)) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => secondary,
                std::cmp::Ordering::Greater => 1,
            };
        }
    }
}

/// SortCompare: undirected comparison happens per mode, the direction is
/// applied at the end (exactly like the C, including -decreasing negating
/// command results).
fn sort_compare(
    arena: &[SortElement],
    a: usize,
    b: usize,
    st: &mut SortState,
    interp: &mut Interp,
) -> i64 {
    let order: i64 = match st.mode {
        SortMode::Ascii => {
            let (x, y) = (arena[a].key.as_str(), arena[b].key.as_str());
            match x.cmp(y) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
        SortMode::AsciiNc => utf_casecmp(arena[a].key.as_str(), arena[b].key.as_str()),
        SortMode::Dictionary => dict_compare(arena[a].key.as_str(), arena[b].key.as_str()),
        SortMode::Integer => {
            let (x, y) = (arena[a].key.as_wide(), arena[b].key.as_wide());
            (x >= y) as i64 - (x <= y) as i64
        }
        SortMode::Real => {
            let (x, y) = (arena[a].key.as_real(), arena[b].key.as_real());
            (x >= y) as i64 - (x <= y) as i64
        }
        SortMode::Command => {
            if st.error.is_some() {
                // Preserve the first comparison error; skip further compares.
                return 0;
            }
            let script = format!(
                "{} {} {}",
                st.command_prefix,
                crate::value::tcl_quote(arena[a].key.as_obj().as_str()),
                crate::value::tcl_quote(arena[b].key.as_obj().as_str()),
            );
            match interp.eval(&script) {
                Ok(v) => match v.as_int() {
                    Some(n) => n,
                    None => {
                        super::list::set_error_code(interp, "TCL OPERATION LSORT COMPARISONFAILED");
                        st.error = Some(super::list::tcl_err(
                            "-compare command returned non-integer result",
                        ));
                        return 0;
                    }
                },
                Err(e) => {
                    st.error = Some(e);
                    return 0;
                }
            }
        }
    };
    if st.increasing { order } else { -order }
}

/// MergeLists, ported from tclCmdIL.c: on `cmp > 0` the right run supplies
/// the next element; with -unique an equality drops the LEFT element (so the
/// later-seen survivor wins, per Tcl's merge order).
fn merge_lists(
    arena: &mut Vec<SortElement>,
    left: Option<usize>,
    right: Option<usize>,
    st: &mut SortState,
    interp: &mut Interp,
) -> Option<usize> {
    let mut l = left;
    let mut r = right;
    if l.is_none() {
        return r;
    }
    if r.is_none() {
        return l;
    }

    let cmp = sort_compare(arena, l.unwrap(), r.unwrap(), st, interp);
    let head;
    let mut tail;
    if cmp > 0 || (cmp == 0 && st.unique) {
        if cmp == 0 {
            l = arena[l.unwrap()].next;
        }
        tail = r.unwrap();
        r = arena[tail].next;
    } else {
        tail = l.unwrap();
        l = arena[tail].next;
    }
    head = Some(tail);

    if !st.unique {
        while l.is_some() && r.is_some() {
            let cmp = sort_compare(arena, l.unwrap(), r.unwrap(), st, interp);
            if cmp > 0 {
                arena[tail].next = r;
                tail = r.unwrap();
                r = arena[tail].next;
            } else {
                arena[tail].next = l;
                tail = l.unwrap();
                l = arena[tail].next;
            }
        }
    } else {
        while l.is_some() && r.is_some() {
            let cmp = sort_compare(arena, l.unwrap(), r.unwrap(), st, interp);
            if cmp >= 0 {
                if cmp == 0 {
                    l = arena[l.unwrap()].next;
                }
                arena[tail].next = r;
                tail = r.unwrap();
                r = arena[tail].next;
            } else {
                arena[tail].next = l;
                tail = l.unwrap();
                l = arena[tail].next;
            }
        }
    }
    arena[tail].next = if l.is_some() { l } else { r };
    head
}

pub fn cmd_lsort(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        super::list::set_error_code(interp, "TCL WRONGARGS");
        return Err(super::list::tcl_err(
            "wrong # args: should be \"lsort ?-option value ...? list\"",
        ));
    }

    let mut mode = SortMode::Ascii;
    let mut decreasing = false;
    let mut unique = false;
    let mut nocase = false;
    let mut indices = false;
    let mut command_value: Option<Value> = None;
    let mut index_specs: Vec<IndexSpec> = Vec::new();
    let mut group_size: i64 = 1;
    let mut group = false;

    // Option loop covers args[1..len-1]; the final argument is always the
    // list. Every option must be recognized — abbreviations included, but
    // there is no `--` terminator in Tcl 8.6's lsort.
    let mut i = 1;
    while i < args.len() - 1 {
        let name = args[i].as_str();
        let opt = if LSORT_OPTIONS.contains(&name) {
            name
        } else {
            let matches: Vec<&str> = LSORT_OPTIONS
                .iter()
                .copied()
                .filter(|o| o.starts_with(name))
                .collect();
            match matches.as_slice() {
                [only] => *only,
                _ => {
                    let kind = if matches.is_empty() { "bad option" } else { "ambiguous option" };
                    super::list::set_error_code(
                        interp,
                        &format!("TCL LOOKUP INDEX option {}", name),
                    );
                    return Err(super::list::tcl_err(format!(
                        "{} \"{}\": {}",
                        kind,
                        name,
                        option_must_be()
                    )));
                }
            }
        };
        match opt {
            "-ascii" => mode = SortMode::Ascii,
            "-command" => {
                if i + 1 >= args.len() - 1 {
                    super::list::set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(super::list::tcl_err(
                        "\"-command\" option must be followed by comparison command",
                    ));
                }
                mode = SortMode::Command;
                command_value = Some(args[i + 1].clone());
                i += 1;
            }
            "-decreasing" => decreasing = true,
            "-dictionary" => mode = SortMode::Dictionary,
            "-increasing" => decreasing = false,
            "-index" => {
                if i + 1 >= args.len() - 1 {
                    super::list::set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(super::list::tcl_err(
                        "\"-index\" option must be followed by list index",
                    ));
                }
                let elems = super::list::strict_list(interp, &args[i + 1])?;
                index_specs = Vec::with_capacity(elems.len());
                for e in &elems {
                    index_specs.push(validate_index_spec(interp, e.as_str())?);
                }
                i += 1;
            }
            "-indices" => indices = true,
            "-integer" => mode = SortMode::Integer,
            "-nocase" => nocase = true,
            "-real" => mode = SortMode::Real,
            "-stride" => {
                if i + 1 >= args.len() - 1 {
                    super::list::set_error_code(interp, "TCL ARGUMENT MISSING");
                    return Err(super::list::tcl_err(
                        "\"-stride\" option must be followed by stride length",
                    ));
                }
                let wide = match super::list::tcl_get_int(args[i + 1].as_str()) {
                    Some(w) => w,
                    None => {
                        super::list::set_error_code(interp, "TCL VALUE NUMBER");
                        return Err(super::list::tcl_err(format!(
                            "expected integer but got \"{}\"",
                            args[i + 1].as_str()
                        )));
                    }
                };
                if wide < 2 {
                    super::list::set_error_code(interp, "TCL OPERATION LSORT BADSTRIDE");
                    return Err(super::list::tcl_err("stride length must be at least 2"));
                }
                group_size = wide;
                group = true;
                i += 1;
            }
            "-unique" => unique = true,
            _ => unreachable!("option resolved above"),
        }
        i += 1;
    }

    if nocase && mode == SortMode::Ascii {
        mode = SortMode::AsciiNc;
    }

    // -command words, quoted for eval (the C flattens the command list and
    // reserves two slots for the comparison arguments).
    let command_prefix = match &command_value {
        Some(v) => super::list::strict_list(interp, v)?
            .iter()
            .map(|w| crate::value::tcl_quote(w.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    };

    let list = super::list::strict_list(interp, &args[args.len() - 1])?;
    if list.is_empty() {
        return Ok(Value::from_list(&[]));
    }

    // -stride sanity: the list must decompose into whole groups.
    if group && list.len() as i64 % group_size != 0 {
        super::list::set_error_code(interp, "TCL OPERATION LSORT BADSTRIDE");
        return Err(super::list::tcl_err(
            "list size must be a multiple of the stride length",
        ));
    }
    let effective_len = if group { list.len() / group_size as usize } else { list.len() };

    // With -stride, the leading -index value selects the in-group offset to
    // sort by; the rest index within each element.
    let mut group_offset: i64 = 0;
    if group && !index_specs.is_empty() {
        group_offset = index_specs[0].decode(group_size - 1);
        if group_offset < 0 || group_offset >= group_size {
            super::list::set_error_code(interp, "TCL OPERATION LSORT BADINDEX");
            return Err(super::list::tcl_err(
                "when used with \"-stride\", the leading \"-index\" value must be within the group",
            ));
        }
        if index_specs.len() == 1 {
            index_specs.clear();
        } else {
            index_specs.remove(0);
        }
    }

    let mut st = SortState {
        mode,
        increasing: !decreasing,
        unique,
        command_prefix,
        error: None,
    };

    // Build one SortElement per logical element, merging as we go: after the
    // loop, sublists[j] holds a sorted run of 2**j elements.
    let mut arena: Vec<SortElement> = Vec::with_capacity(effective_len);
    let mut sublists: [Option<usize>; NUM_LISTS] = [None; NUM_LISTS];

    for i in 0..effective_len {
        let idx = (group_size * i as i64 + group_offset) as usize;
        let key_obj: Value = if index_specs.is_empty() {
            list[idx].clone()
        } else {
            select_from_sublist(interp, &list[idx], &index_specs)?
        };

        let key = match mode {
            SortMode::Ascii | SortMode::AsciiNc | SortMode::Dictionary => {
                Key::Str(key_obj.as_str().to_string())
            }
            SortMode::Integer => match super::list::tcl_get_int(key_obj.as_str()) {
                Some(n) => Key::Wide(n),
                None => {
                    super::list::set_error_code(interp, "TCL VALUE NUMBER");
                    return Err(super::list::tcl_err(format!(
                        "expected integer but got \"{}\"",
                        key_obj.as_str()
                    )));
                }
            },
            SortMode::Real => match parse_tcl_double(key_obj.as_str()) {
                Some(f) => Key::Real(f),
                None => {
                    super::list::set_error_code(interp, "TCL VALUE NUMBER");
                    return Err(super::list::tcl_err(format!(
                        "expected floating-point number but got \"{}\"",
                        key_obj.as_str()
                    )));
                }
            },
            SortMode::Command => Key::Obj(key_obj),
        };

        let payload = if indices || group {
            Payload::Index(idx as i64)
        } else {
            Payload::Obj(list[idx].clone())
        };

        arena.push(SortElement { key, payload, next: None });
        let mut cur = arena.len() - 1;
        let mut j = 0;
        while j < NUM_LISTS && sublists[j].is_some() {
            let left = sublists[j];
            sublists[j] = None;
            cur = merge_lists(&mut arena, left, Some(cur), &mut st, interp)
                .expect("merge of two non-empty runs");
            j += 1;
        }
        if j >= NUM_LISTS {
            j = NUM_LISTS - 1;
        }
        sublists[j] = Some(cur);
    }

    // Merge all remaining runs.
    let mut head = sublists[0];
    for j in 1..NUM_LISTS {
        head = merge_lists(&mut arena, sublists[j], head, &mut st, interp);
    }
    if let Some(e) = st.error.take() {
        return Err(e);
    }

    // Rebuild the result: plain elements, original indices, or stride groups.
    let mut out: Vec<Value> = Vec::new();
    let mut cur = head;
    while let Some(ci) = cur {
        let e = &arena[ci];
        match &e.payload {
            Payload::Index(base) => {
                if group {
                    for j in 0..group_size {
                        let k = base + j - group_offset;
                        if indices {
                            out.push(Value::from_int(k));
                        } else {
                            out.push(list[k as usize].clone());
                        }
                    }
                } else {
                    out.push(Value::from_int(*base));
                }
            }
            Payload::Obj(v) => out.push(v.clone()),
        }
        cur = e.next;
    }
    Ok(Value::from_list(&out))
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
        // tclsh 8.6.17: leading-case mismatch is only a secondary tiebreak,
        // so Bigboy < bigBoy < bigboy (all letters compare lowercase-equal).
        let mut interp = Interp::new();
        let r = interp.eval("lsort -dictionary {bigBoy Bigboy bigboy}").unwrap();
        assert_eq!(r.as_str(), "Bigboy bigBoy bigboy");
        assert_eq!(interp.eval("lsort -dictionary {Z z A a}").unwrap().as_str(), "A a Z z");
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

    // -- L4: -integer/-real validate every sort key (tclsh 8.6.17 oracle) --

    #[test]
    fn test_lsort_integer_rejects_non_integer() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -integer {a 1}").unwrap_err();
        assert!(e.to_string().contains("expected integer but got \"a\""), "{}", e);
    }

    #[test]
    fn test_lsort_integer_rejects_single_element() {
        // Tcl validates keys even when no comparison is needed.
        let mut interp = Interp::new();
        let e = interp.eval("lsort -integer {a}").unwrap_err();
        assert!(e.to_string().contains("expected integer but got \"a\""), "{}", e);
    }

    #[test]
    fn test_lsort_real_rejects_non_float() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -real {x 1.5}").unwrap_err();
        assert!(e.to_string().contains("expected floating-point number but got \"x\""), "{}", e);
    }

    #[test]
    fn test_lsort_integer_accepts_whitespace_and_hex() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort -integer { 42 7}").unwrap().as_str(), "7 42");
        assert_eq!(interp.eval("lsort -integer {0x10 2}").unwrap().as_str(), "2 0x10");
    }

    // -- L5: -unique dedupes on the comparison key, Tcl merge order --

    #[test]
    fn test_lsort_unique_nocase_keeps_last_of_equal() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -unique -nocase {a A b}").unwrap();
        assert_eq!(r.as_str(), "A b");
    }

    #[test]
    fn test_lsort_unique_integer_key() {
        // Oracle: {3 03 2} -> {2 03} — 3 and 03 compare equal, the one that
        // survives follows from Tcl's binary-counter merge (right wins).
        let mut interp = Interp::new();
        let r = interp.eval("lsort -unique -integer {3 03 2}").unwrap();
        assert_eq!(r.as_str(), "2 03");
    }

    #[test]
    fn test_lsort_unique_real_key() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -unique -real {1 1.0 2}").unwrap();
        assert_eq!(r.as_str(), "1.0 2");
    }

    #[test]
    fn test_lsort_unique_index_cmdil_1_23() {
        // Official test suite case cmdIL-1.23.
        let mut interp = Interp::new();
        let r = interp
            .eval("lsort -unique -index 0 {{a b} {c b} {a c} {d a}}")
            .unwrap();
        assert_eq!(r.as_str(), "{a c} {c b} {d a}");
    }

    #[test]
    fn test_lsort_unique_integer_merge_order() {
        // Oracle: {2 1 03 3} -> {1 2 3} (merge order drops 03, keeps 3).
        let mut interp = Interp::new();
        let r = interp.eval("lsort -unique -integer {2 1 03 3}").unwrap();
        assert_eq!(r.as_str(), "1 2 3");
    }

    #[test]
    fn test_lsort_indices_cmdil_1_28() {
        // Official test suite case cmdIL-1.28.
        let mut interp = Interp::new();
        let r = interp
            .eval("lsort -indices -unique -decreasing -real {1.2 34.5 34.5 5.6}")
            .unwrap();
        assert_eq!(r.as_str(), "2 3 0");
    }

    // -- strict list parsing + option errors --

    #[test]
    fn test_lsort_malformed_list_is_error() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort \"{\"").unwrap_err();
        assert!(e.to_string().contains("unmatched open brace in list"), "{}", e);
    }

    #[test]
    fn test_lsort_unknown_option_is_error() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -bogus {a b}").unwrap_err();
        assert!(e.to_string().contains("bad option \"-bogus\""), "{}", e);
    }

    #[test]
    fn test_lsort_command_missing_value() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -command {3 1 2}").unwrap_err();
        assert!(
            e.to_string()
                .contains("\"-command\" option must be followed by comparison command"),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_multi_index() {
        // -index accepts an index list traversing nested sublists.
        let mut interp = Interp::new();
        let r = interp
            .eval("lsort -integer -index {1 0} {{a {2 x}} {b {1 y}}}")
            .unwrap();
        assert_eq!(r.as_str(), "{b {1 y}} {a {2 x}}");
    }

    #[test]
    fn test_lsort_index_missing_sublist_element() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -index 1 {{a b} {c}}").unwrap_err();
        assert!(
            e.to_string().contains("element 1 missing from sublist \"c\""),
            "{}",
            e
        );
    }

    // -- option resolution: abbreviation + ambiguous + bad (tclsh probes) --

    #[test]
    fn test_lsort_option_abbreviation() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort -de {b a c}").unwrap().as_str(), "c b a");
        assert_eq!(interp.eval("lsort -u {a A B b}").unwrap().as_str(), "A B a b");
    }

    #[test]
    fn test_lsort_ambiguous_option() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -d {b a}").unwrap_err();
        assert!(e.to_string().starts_with("ambiguous option \"-d\": must be -ascii,"), "{}", e);
    }

    #[test]
    fn test_lsort_unknown_option_full_message() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -bogus {a b}").unwrap_err();
        assert!(
            e.to_string().starts_with(
                "bad option \"-bogus\": must be -ascii, -command, -decreasing, -dictionary, -increasing, -index, -indices, -integer, -nocase, -real, -stride, or -unique"
            ),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_double_dash_is_not_an_option() {
        // 8.6.17 lsort has no `--` terminator.
        let mut interp = Interp::new();
        let e = interp.eval("lsort -- -decreasing {b a}").unwrap_err();
        assert!(e.to_string().starts_with("bad option \"--\":"), "{}", e);
    }

    // -- -index prevalidation (TclIndexEncode, tclsh probes) --

    #[test]
    fn test_lsort_index_negative_is_out_of_range() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -index -1 {{a 1} {b 0}}").unwrap_err();
        assert!(
            e.to_string().contains("index \"-1\" cannot select an element from any list"),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_index_end_plus_is_out_of_range() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -index end+5 {{a}}").unwrap_err();
        assert!(
            e.to_string().contains("index \"end+5\" cannot select an element from any list"),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_index_overflow_is_bad_index() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -index 99999999999999999999 {{a}}").unwrap_err();
        assert!(
            e.to_string().contains("bad index \"99999999999999999999\""),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_index_end_minus() {
        let mut interp = Interp::new();
        // tclsh 8.6.17: end-1 keys of {c 9}/{a 1} are "c"/"a" (ascii sort),
        // while -integer -index end keys are the LAST elements "9"/"1".
        let r = interp.eval("lsort -index end-1 {{c 9} {a 1}}").unwrap();
        assert_eq!(r.as_str(), "{a 1} {c 9}");
        let r = interp.eval("lsort -integer -index end {{c 9} {a 1}}").unwrap();
        assert_eq!(r.as_str(), "{a 1} {c 9}");
        let e = interp
            .eval("lsort -integer -index end-1 {{c 9} {a 1}}")
            .unwrap_err();
        assert!(e.to_string().contains("expected integer but got \"c\""), "{}", e);
    }

    // -- numeric key parsing mirrors TclGetWideIntFromObj/GetDouble (probes) --

    #[test]
    fn test_lsort_integer_legacy_octal() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("lsort -integer {010 8 10}").unwrap().as_str(),
            "010 8 10"
        );
    }

    #[test]
    fn test_lsort_integer_bad_octal_is_error() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -integer {09 8}").unwrap_err();
        assert!(e.to_string().contains("expected integer but got \"09\""), "{}", e);
    }

    #[test]
    fn test_lsort_integer_signs() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("lsort -integer {+5 -3 0x10}").unwrap().as_str(),
            "-3 +5 0x10"
        );
    }

    #[test]
    fn test_lsort_real_hex_and_inf() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort -real {0x10 1}").unwrap().as_str(), "1 0x10");
        assert_eq!(interp.eval("lsort -real {Inf -Inf 1 2}").unwrap().as_str(), "-Inf 1 2 Inf");
    }

    // -- stride + indices + unique combinations (probes / cmdIL) --

    #[test]
    fn test_lsort_indices_stride() {
        // Official-suite style: groups sorted by leading -index value.
        let mut interp = Interp::new();
        let r = interp
            .eval("lsort -indices -stride 3 -index 0 {x 1 z 2 y 3}")
            .unwrap();
        assert_eq!(r.as_str(), "3 4 5 0 1 2");
    }

    #[test]
    fn test_lsort_stride_unique_drops_groups() {
        let mut interp = Interp::new();
        let r = interp.eval("lsort -stride 2 -unique {a b c d a b}").unwrap();
        assert_eq!(r.as_str(), "a b c d");
    }

    #[test]
    fn test_lsort_stride_index_within_group_error() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -stride 3 -index 3 {a b c d}").unwrap_err();
        assert!(
            e.to_string().contains("list size must be a multiple of the stride length"),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_stride_index_inside_group() {
        let mut interp = Interp::new();
        let r = interp
            .eval("lsort -stride 2 -index 1 -integer {z 1 y 2 x 3}")
            .unwrap();
        assert_eq!(r.as_str(), "z 1 y 2 x 3");
    }

    // -- -command details (probes) --

    #[test]
    fn test_lsort_command_with_extra_words() {
        let mut interp = Interp::new();
        let r = interp
            .eval("lsort -command {string compare -nocase} {B a C}")
            .unwrap();
        assert_eq!(r.as_str(), "a B C");
    }

    #[test]
    fn test_lsort_command_error_propagates() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort -command nosuchcmd {a b}").unwrap_err();
        assert!(e.to_string().contains("invalid command name \"nosuchcmd\""), "{}", e);
    }

    #[test]
    fn test_lsort_command_non_integer_result() {
        let mut interp = Interp::new();
        interp.eval("proc ret {a b} { return xyz }").unwrap();
        let e = interp.eval("lsort -command ret {a b}").unwrap_err();
        assert!(
            e.to_string().contains("-compare command returned non-integer result"),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_command_error_latches() {
        // The FIRST comparison error wins and later compares are skipped.
        let mut interp = Interp::new();
        interp
            .eval("proc boom {a b} { error \"cmp failure\" }; proc ok {a b} { return 0 }")
            .unwrap();
        let e = interp.eval("lsort -command boom {a b c}").unwrap_err();
        assert!(e.to_string().contains("cmp failure"), "{}", e);
    }

    // -- dictionary details (probes) --

    #[test]
    fn test_lsort_dictionary_embedded_numbers() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort -dictionary {10 9 2 1}").unwrap().as_str(), "1 2 9 10");
        assert_eq!(
            interp.eval("lsort -dictionary {aB Ab a1 A0}").unwrap().as_str(),
            "A0 a1 Ab aB"
        );
    }

    #[test]
    fn test_lsort_nocase_ignored_for_dictionary() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort -nocase -dictionary {b A}").unwrap().as_str(), "A b");
    }

    // -- misc shapes --

    #[test]
    fn test_lsort_wrong_args_usage() {
        let mut interp = Interp::new();
        let e = interp.eval("lsort").unwrap_err();
        assert!(
            e.to_string().contains("wrong # args: should be \"lsort ?-option value ...? list\""),
            "{}",
            e
        );
    }

    #[test]
    fn test_lsort_bare_option_as_list() {
        // objc==2: the "option" is actually the list argument.
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort -ascii").unwrap().as_str(), "-ascii");
    }

    #[test]
    fn test_lsort_empty_list() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("lsort {}").unwrap().as_str(), "");
        assert_eq!(interp.eval("lsort -indices {}").unwrap().as_str(), "");
    }
}
