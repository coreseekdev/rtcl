//! List commands: list, llength, lindex, lappend, lrange, linsert, lreplace,
//! lassign, lrepeat, lreverse, concat, split, join, lmap, lset, lsubst.
//! See list_sort.rs for lsearch and lsort.

use std::borrow::Cow;

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::{is_tcl_space, Value};


/// Build an error whose message survives `catch` verbatim (the interp
/// stores `e.to_string()` into the result variable, and `Error::Msg`
/// displays as exactly the payload).
pub(crate) fn tcl_err(msg: impl Into<String>) -> Error {
    Error::Msg(msg.into())
}

/// Tcl's wrong-#-args error text: `wrong # args: should be "name usage"`.
#[allow(dead_code)]
pub(crate) fn wrong_args(name: &str, usage: &str) -> Error {
    tcl_err(format!("wrong # args: should be \"{} {}\"", name, usage))
}

/// Mirror Tcl setting `::errorCode` at the point an error is raised.
pub(crate) fn set_error_code(interp: &mut Interp, code: &str) {
    interp.globals.insert("errorCode".to_string(), Value::from_str(code));
    interp.err_code_raised = true;
}

/// Parse a command argument as a list; malformed lists raise Tcl's error
/// (`unmatched open brace in list` etc.) with the matching `::errorCode`.
pub(crate) fn strict_list(interp: &mut Interp, v: &Value) -> Result<Vec<Value>> {
    v.as_list_strict().map_err(|e| {
        set_error_code(interp, e.code);
        tcl_err(e.message)
    })
}

/// [`strict_list`] with a borrow fast path: a value already carrying a
/// list internal rep is viewed in place (zero-copy, tclsh's
/// `TclListObjGetElements` hands out pointers the same way); only
/// string/dict reps materialise a `Vec`.  Hot read paths (foreach,
/// lindex, llength) go through this.
pub(crate) fn strict_list_cow<'a>(interp: &mut Interp, v: &'a Value) -> Result<Cow<'a, [Value]>> {
    match v.as_list_ref() {
        Some(items) => Ok(Cow::Borrowed(items)),
        None => strict_list(interp, v).map(Cow::Owned),
    }
}

/// Tcl's bad-index error (`TCL VALUE INDEX`), including the
/// "(looks like invalid octal number)" hint Tcl appends when the index
/// resembles a malformed legacy octal literal (TclCheckBadOctal).
pub(crate) fn bad_index(interp: &mut Interp, idx: &str) -> Error {
    set_error_code(interp, "TCL VALUE INDEX");
    let mut msg = format!(
        "bad index \"{}\": must be integer?[+-]integer? or end?[+-]integer?",
        idx
    );
    let check = idx.strip_prefix("end-").unwrap_or(idx);
    if looks_like_bad_octal(check) {
        msg.push_str(" (looks like invalid octal number)");
    }
    tcl_err(msg)
}

/// TclCheckBadOctal: optional whitespace/sign, then `0` (or `0o`) followed
/// by nothing but decimal digits — i.e. a plausible legacy-octal attempt.
fn looks_like_bad_octal(s: &str) -> bool {
    let t = trim_tcl_space(s);
    let t = t.strip_prefix(['+', '-']).unwrap_or(t);
    let Some(t) = t.strip_prefix('0') else { return false };
    let t = t.strip_prefix(['o', 'O']).unwrap_or(t);
    let t = t.trim_end_matches(|c: char| is_tcl_space(c as u8));
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())
}

/// Tcl's whitespace set (also the list element separators).
pub(crate) fn trim_tcl_space(s: &str) -> &str {
    s.trim_matches(|c: char| is_tcl_space(c as u8))
}

/// Scan a Tcl integer at the start of `s` (TclParseNumber integer rules:
/// optional sign; `0x`/`0b`/`0o` radix prefixes; a leading `0` followed by
/// more characters selects legacy octal). Returns (value, bytes consumed).
pub(crate) fn scan_tcl_int(s: &str) -> Option<(i64, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let (radix, start) = if b.len() - i >= 2 && b[i] == b'0' && matches!(b[i + 1], b'x' | b'X') {
        (16, i + 2)
    } else if b.len() - i >= 2 && b[i] == b'0' && matches!(b[i + 1], b'b' | b'B') {
        (2, i + 2)
    } else if b.len() - i >= 2 && b[i] == b'0' && matches!(b[i + 1], b'o' | b'O') {
        (8, i + 2)
    } else if i < b.len() && b[i] == b'0' && i + 1 < b.len() {
        (8, i + 1)
    } else {
        (10, i)
    };
    let mut n = start;
    while n < b.len() && (b[n] as char).is_digit(radix) {
        n += 1;
    }
    if n == start {
        // A bare leading `0` is the value zero, even in octal position.
        if radix == 8 && start == i + 1 {
            return Some((0, i + 1));
        }
        return None;
    }
    let val = i64::from_str_radix(&s[start..n], radix).ok()?;
    Some((if neg { -val } else { val }, n))
}

/// Tcl_GetInt semantics: the whole (whitespace-trimmed) string is an integer.
pub(crate) fn tcl_get_int(s: &str) -> Option<i64> {
    let t = trim_tcl_space(s);
    let (v, used) = scan_tcl_int(t)?;
    if used == t.len() { Some(v) } else { None }
}

/// Parse a Tcl index (`integer?[+-]integer?` or `end?[+-]integer?`),
/// resolving `end` against `len`. The result may be negative or >= len;
/// callers clamp or report out-of-range as appropriate.
/// Mirrors Tcl 8.6's TclGetIntForIndex (probed on 8.6.17): at most two
/// terms with no internal whitespace, each term's magnitude bounded by
/// 2^32-1 (`4294967295` accepted, `4294967296` rejected), and the sum
/// wraps to C `int` — `end+4294967295` on an 8-char string is index 6,
/// and `-4294967295` is index 1.
pub(crate) fn parse_tcl_index(s: &str, len: usize) -> Option<i64> {
    const MAX_TERM: i64 = 4294967295; // 2^32 - 1
    let t = trim_tcl_space(s);
    // 1. "end" or a leading integer (leading whitespace already trimmed).
    let (base, rest) = if let Some(rest) = t.strip_prefix("end") {
        (len as i64 - 1, rest)
    } else {
        let (v, used) = scan_tcl_int(t)?;
        if v.unsigned_abs() > MAX_TERM as u64 {
            return None;
        }
        (v, &t[used..])
    };
    if rest.is_empty() {
        return Some(wrap_i32(base));
    }
    // 2. Optional second term: an operator with no whitespace before the
    //    number, whose value keeps its own sign (`-1--2` is 1, `1++1` is 2).
    let rb = rest.as_bytes();
    if rb.len() < 2 || (rb[0] != b'+' && rb[0] != b'-') || is_tcl_space(rb[1]) {
        return None;
    }
    let off = tcl_get_int(&rest[1..])?;
    if off.unsigned_abs() > MAX_TERM as u64 {
        return None;
    }
    Some(wrap_i32(if rb[0] == b'+' { base + off } else { base - off }))
}

/// C `int` wraparound, which Tcl index arithmetic is subject to.
fn wrap_i32(v: i64) -> i64 {
    v as i32 as i64
}

pub fn cmd_list(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    Ok(Value::from_list(&args[1..]))
}

pub fn cmd_llength(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("llength", "list"));
    }
    let list = strict_list_cow(interp, &args[1])?;
    Ok(Value::from_int(list.len() as i64))
}

pub fn cmd_lindex(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lindex", "list ?index ...?"));
    }
    // Tcl: with no index, the list is returned unparsed.
    if args.len() == 2 {
        return Ok(args[1].clone());
    }
    let mut current = args[1].clone();
    // Tcl: lindex list i j k ... descends into nested lists.
    // A single index argument is itself parsed as a list of indices;
    // a malformed one is treated as a single index string.
    let indices: Vec<Value> = if args.len() == 3 {
        match args[2].as_list() {
            Some(l) => l,
            None => vec![args[2].clone()],
        }
    } else {
        args[2..].to_vec()
    };
    for idx_val in &indices {
        let next = {
            let list = strict_list_cow(interp, &current)?;
            let idx_str = idx_val.as_str();
            let len = list.len();
            match parse_tcl_index(idx_str, len) {
                Some(raw) if raw >= 0 && (raw as usize) < len => list[raw as usize].clone(),
                Some(_) => return Ok(Value::empty()),
                None => return Err(bad_index(interp, idx_str)),
            }
        };
        current = next;
    }
    Ok(current)
}

pub fn cmd_lappend(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lappend", "varName ?value ...?"));
    }
    let var_name = args[1].as_str();
    // Fast path: the variable already holds a list rep and every
    // mutation guard holds — take the value out of its slot (sole
    // owner), append in place through the COW rep, put it back.
    // Amortised O(1); tclsh appends to the unshared list object the
    // same way.  Creation, string-rep sources, links, arrays and
    // traced variables keep the rebuilding path below.
    if args.len() >= 3 {
        // take_var_fast owns every guard (links, arrays, traces, ns
        // qualifiers) and the old get_var pre-probe cost a full extra
        // resolution per append; a non-list rep is restored and the slow
        // path parses it, exactly as before.
        if let Some(mut v) = interp.take_var_fast(var_name) {
            if let Some(items) = v.as_list_mut() {
                for arg in &args[2..] {
                    items.push(arg.clone());
                }
                return interp.set_var(var_name, v);
            }
            // Not a list rep: restore raw and rebuild through the slow
            // path (the strict parse below owns the same errors).
            interp.store_var(var_name, v);
        }
    }
    let mut list = match interp.get_var(var_name) {
        Ok(v) => {
            let v = v.clone();
            strict_list(interp, &v)?
        }
        Err(_) => Vec::new(),
    };
    for arg in &args[2..] {
        list.push(arg.clone());
    }
    let result = Value::from_list(&list);
    interp.set_var(var_name, result.clone())
}

pub fn cmd_lrange(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 4 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lrange", "list first last"));
    }
    let list = strict_list(interp, &args[1])?;
    let len = list.len();
    let first_raw = match parse_tcl_index(args[2].as_str(), len) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[2].as_str())),
    };
    let last_raw = match parse_tcl_index(args[3].as_str(), len) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[3].as_str())),
    };

    // Tcl clamps: first < 0 → 0; last >= len → len-1; empty when the
    // resulting range is backwards or starts past the end.
    let first = first_raw.max(0);
    let last = last_raw.min(len as i64 - 1);
    if first <= last && first < len as i64 {
        let result: Vec<Value> = list[first as usize..=last as usize].to_vec();
        Ok(Value::from_list(&result))
    } else {
        Ok(Value::empty())
    }
}

pub fn cmd_linsert(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("linsert", "list index ?element ...?"));
    }
    let list = strict_list(interp, &args[1])?;
    let len = list.len();
    // Tcl resolves the index against len+1 (`end` means "after the last
    // element") and clamps into [0, len].
    let raw = match parse_tcl_index(args[2].as_str(), len + 1) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[2].as_str())),
    };
    let index = raw.clamp(0, len as i64) as usize;
    let elements: Vec<Value> = args[3..].to_vec();
    let mut result = Vec::with_capacity(list.len() + elements.len());
    result.extend(list[..index].iter().cloned());
    result.extend(elements);
    result.extend(list[index..].iter().cloned());
    Ok(Value::from_list(&result))
}

pub fn cmd_lreplace(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lreplace", "list first last ?element ...?"));
    }
    let list = strict_list(interp, &args[1])?;
    let len = list.len();
    let first_raw = match parse_tcl_index(args[2].as_str(), len) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[2].as_str())),
    };
    let last_raw = match parse_tcl_index(args[3].as_str(), len) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[3].as_str())),
    };

    // Tcl clamps first into [0, len] and last into [-inf, len-1];
    // last < first is a pure insertion at `first`.
    let first = first_raw.clamp(0, len as i64) as usize;
    let last = last_raw.min(len as i64 - 1);

    let mut result = Vec::with_capacity(list.len() + args.len() - 4);
    result.extend(list[..first].iter().cloned());
    result.extend(args[4..].iter().cloned());
    if last >= first as i64 {
        result.extend(list[(last + 1) as usize..].iter().cloned());
    } else {
        result.extend(list[first..].iter().cloned());
    }
    Ok(Value::from_list(&result))
}

pub fn cmd_lassign(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lassign", "list ?varName ...?"));
    }
    let list = strict_list(interp, &args[1])?;
    let vars: Vec<&str> = args[2..].iter().map(|v| v.as_str()).collect();
    for (i, var) in vars.iter().enumerate() {
        let value = list.get(i).cloned().unwrap_or_else(Value::empty);
        interp.set_var(var, value)?;
    }
    if list.len() > vars.len() {
        Ok(Value::from_list(&list[vars.len()..]))
    } else {
        Ok(Value::empty())
    }
}

pub fn cmd_lrepeat(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lrepeat", "count ?value ...?"));
    }
    let count = match args[1].as_int() {
        Some(n) => n,
        None => {
            return Err(tcl_err(format!(
                "expected integer but got \"{}\"",
                args[1].as_str()
            )));
        }
    };
    if count < 0 {
        return Err(tcl_err(format!(
            "bad count \"{}\": must be integer >= 0",
            args[1].as_str()
        )));
    }
    let count = count as usize;
    let elements: Vec<Value> = args[2..].to_vec();
    if elements.is_empty() {
        return Ok(Value::empty());
    }
    let mut result = Vec::with_capacity(count * elements.len());
    for _ in 0..count {
        result.extend(elements.iter().cloned());
    }
    Ok(Value::from_list(&result))
}

pub fn cmd_lreverse(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lreverse", "list"));
    }
    let mut list = strict_list(interp, &args[1])?;
    list.reverse();
    Ok(Value::from_list(&list))
}

pub fn cmd_concat(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Tcl_Concat joins the *string reps* of the arguments, stripping
    // leading/trailing whitespace — but a trailing whitespace run whose
    // last char is backslash-escaped stays, so `{b\   }` trims to `b\ `
    // (util-4.2) while `b\\   ` trims to `b\\` (util-4.3).  No list
    // parsing, no re-escaping (probed: `{"x y"}` keeps its quotes,
    // `{a$b}` keeps the dollar, `a {b{c}` is not validated).
    let mut result = String::new();
    for arg in &args[1..] {
        let trimmed = concat_trim(arg.as_str());
        if trimmed.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(trimmed);
    }
    Ok(Value::from_str(&result))
}

/// Strip leading Tcl whitespace; strip trailing whitespace but stop as
/// soon as the whitespace char being removed has a backslash before it
/// (an escaped space is element content, not a separator).
fn concat_trim(s: &str) -> &str {
    let b = s.as_bytes();
    let mut start = 0;
    while start < b.len() && is_tcl_space(b[start]) {
        start += 1;
    }
    let mut end = b.len();
    while end > start && is_tcl_space(b[end - 1]) {
        if end >= 2 && b[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &s[start..end]
}

pub fn cmd_split(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("split", "string ?splitChars?"));
    }
    let string = args[1].as_str();
    if string.is_empty() {
        return Ok(Value::empty());
    }
    let split_chars = if args.len() == 3 { args[2].as_str() } else { " \t\n\r" };
    let result: Vec<Value> = if split_chars.is_empty() {
        string.chars().map(|c| Value::from_str(&c.to_string())).collect()
    } else {
        // Tcl splits on each splitChars byte individually, producing
        // empty fields for adjacent separators.
        string
            .split(|c| split_chars.contains(c))
            .map(Value::from_str)
            .collect()
    };
    Ok(Value::from_list(&result))
}

pub fn cmd_join(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("join", "list ?joinString?"));
    }
    let list = strict_list(interp, &args[1])?;
    let sep = if args.len() == 3 { args[2].as_str() } else { " " };
    let result: String = list.iter().map(|v| v.as_str()).collect::<Vec<&str>>().join(sep);
    Ok(Value::from_str(&result))
}

/// lmap — Like foreach but collects body results into a list.
/// Usage: lmap varList list ?varList list ...? body
pub fn cmd_lmap(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 || !args.len().is_multiple_of(2) {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lmap", "varList list ?varList list ...? command"));
    }

    // The compiled shape — identical gates to foreach (tclsh compiles both
    // the same way inside proc bodies): frameless body errors, plain
    // var-writes, results collected.
    if interp.lexical_body && super::loops::foreach_inline_shape(interp, args) {
        return super::loops::foreach_lexical(interp, args, true);
    }

    let body = &args[args.len() - 1];
    let mut collected: Vec<Value> = Vec::new();

    struct VarGroup {
        vars: Vec<String>,
        data: Vec<Value>,
    }
    let mut groups: Vec<VarGroup> = Vec::new();
    let mut i = 1;
    while i < args.len() - 1 {
        let var_list = strict_list(interp, &args[i])?;
        if var_list.is_empty() {
            set_error_code(interp, "TCL OPERATION LMAP NEEDVARS");
            return Err(tcl_err("lmap varlist is empty"));
        }
        let vars: Vec<String> = var_list.iter().map(|v| v.as_str().to_string()).collect();
        let data = strict_list(interp, &args[i + 1])?;
        groups.push(VarGroup { vars, data });
        i += 2;
    }

    let max_iters = groups.iter()
        .map(|g| {
            let n = g.vars.len().max(1);
            g.data.len().div_ceil(n)
        })
        .max()
        .unwrap_or(0);

    for idx in 0..max_iters {
        interp.charge_step()?;
        for g in &groups {
            let n = g.vars.len();
            for (vi, var) in g.vars.iter().enumerate() {
                let data_idx = idx * n + vi;
                let value = g.data.get(data_idx).cloned().unwrap_or_else(Value::empty);
                if let Err(e) = interp.set_var(var, value) {
                    // Dispatched lmap decorates a loop-variable write
                    // failure exactly like dispatched foreach (probed
                    // tclsh 8.6.17: the `(setting lmap loop variable)`
                    // frame replaces the `while executing` one, and TCL
                    // WRITE VARNAME installs as ::errorCode).
                    if interp.err_is_error(&e) {
                        set_error_code(interp, "TCL WRITE VARNAME");
                        if interp.err_info.is_none() {
                            interp.err_info = Some(e.message_text());
                        }
                        if let Some(info) = &mut interp.err_info {
                            info.push_str(&format!(
                                "\n    (setting lmap loop variable \"{}\")",
                                var
                            ));
                        }
                    }
                    return Err(e);
                }
            }
        }
        // A `return -level 0 $v` completes as an ordinary command
        // whose result is $v: lmap collects it and keeps iterating
        // (lmap-1.2a).
        match interp.eval_body_value(body) {
            Ok(v) => collected.push(v),
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    continue;
                }
                // Dispatched lmap adds its own body exit frame between the
                // failing command's frame and the lmap command's harness
                // frame (probed: `("lmap" body line N)`, body-relative).
                interp.err_exit_frame("\"lmap\" body");
                return Err(e);
            }
        }
    }

    Ok(Value::from_list(&collected))
}

/// lset varName ?index ...? value
/// Set an element in a list variable.
pub fn cmd_lset(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lset", "listVar ?index? ?index ...? value"));
    }

    let var_name = args[1].as_str();
    let value = args[args.len() - 1].clone();

    // The variable must exist even when it is replaced wholesale.
    let current = interp.get_var(var_name)?.clone();

    if args.len() == 3 {
        // lset var value — replace entire list with value
        interp.set_var(var_name, value.clone())?;
        return Ok(value);
    }

    // Single index case: lset var index value
    // Multi-index: lset var i1 i2 ... value (nested lists)
    // A single index argument is itself parsed as a list of indices;
    // a malformed one is treated as a single index string.
    let indices: Vec<Value> = if args.len() == 4 {
        match args[2].as_list() {
            Some(l) => l,
            None => vec![args[2].clone()],
        }
    } else {
        args[2..args.len() - 1].to_vec()
    };

    if indices.is_empty() {
        // lset var {} value — replace the whole variable
        interp.set_var(var_name, value.clone())?;
        return Ok(value);
    }

    let list = strict_list(interp, &current)?;
    let result = lset_nested(interp, &list, &indices, &value)?;
    interp.set_var(var_name, result.clone())?;
    Ok(result)
}

/// Recursively set a nested element in a list.
fn lset_nested(interp: &mut Interp, list: &[Value], indices: &[Value], value: &Value) -> Result<Value> {
    if indices.is_empty() {
        return Ok(value.clone());
    }
    let idx_str = indices[0].as_str();
    let len = list.len();
    let raw = match parse_tcl_index(idx_str, len) {
        Some(r) => r,
        None => return Err(bad_index(interp, idx_str)),
    };
    // Tcl allows idx == len: the list grows by one element.
    if raw < 0 || raw > len as i64 {
        set_error_code(interp, "TCL OPERATION LSET BADINDEX");
        return Err(tcl_err("list index out of range"));
    }
    let idx = raw as usize;
    let mut new_list = list.to_vec();
    if indices.len() == 1 {
        if idx == len {
            new_list.push(value.clone());
        } else {
            new_list[idx] = value.clone();
        }
    } else {
        let sub_list = if idx == len {
            Vec::new()
        } else {
            strict_list(interp, &new_list[idx])?
        };
        let new_sub = lset_nested(interp, &sub_list, &indices[1..], value)?;
        if idx == len {
            new_list.push(new_sub);
        } else {
            new_list[idx] = new_sub;
        }
    }
    Ok(Value::from_list(&new_list))
}

/// `lsubst ?-command? ?-variable? ?-nobackslashes? ?-nocommands? ?-novariables? string` —
/// Perform substitution like `subst` but split the result into a proper Tcl list.
///
/// This is the jimtcl extension: parse a string with substitutions and return the result as a list.
pub fn cmd_lsubst(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "lsubst",
            2,
            args.len(),
            "?-nobackslashes? ?-nocommands? ?-novariables? string",
        ));
    }

    // Parse flags
    let mut no_backslashes = false;
    let mut no_commands = false;
    let mut no_variables = false;
    let mut string_idx = 1;

    while string_idx < args.len() - 1 {
        match args[string_idx].as_str() {
            "-nobackslashes" => no_backslashes = true,
            "-nocommands" => no_commands = true,
            "-novariables" => no_variables = true,
            other => {
                return Err(Error::runtime(
                    format!("bad option \"{}\": must be -nobackslashes, -nocommands, or -novariables", other),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
        string_idx += 1;
    }

    let input = args[string_idx].as_str();

    // Perform substitution respecting flags
    let substituted = if no_backslashes && no_commands && no_variables {
        // No substitution at all
        input.to_string()
    } else {
        // Build a subst call with appropriate flags
        let mut subst_args = vec![Value::from_str("subst")];
        if no_backslashes {
            subst_args.push(Value::from_str("-nobackslashes"));
        }
        if no_commands {
            subst_args.push(Value::from_str("-nocommands"));
        }
        if no_variables {
            subst_args.push(Value::from_str("-novariables"));
        }
        subst_args.push(Value::from_str(input));
        super::misc::cmd_subst(interp, &subst_args)?.as_str().to_string()
    };

    // Split result into a list
    let elements = Value::from_str(&substituted).as_list().unwrap_or_default();
    Ok(Value::from_list(&elements))
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;
    use super::{concat_trim, parse_tcl_index};

    // --- parse_tcl_index: Tcl 8.6 index grammar (all values probed) ---

    fn idx(s: &str, len: usize) -> Option<i64> {
        parse_tcl_index(s, len)
    }

    #[test]
    fn index_plain_and_prefixed() {
        assert_eq!(idx("0", 5), Some(0));
        assert_eq!(idx("3", 5), Some(3));
        assert_eq!(idx(" 3 ", 5), Some(3)); // surrounding whitespace ok
        assert_eq!(idx("0x0", 5), Some(0));
        assert_eq!(idx("-0x0", 5), Some(0));
        assert_eq!(idx("01", 5), Some(1)); // legacy octal
        assert_eq!(idx("007", 8), Some(7));
        assert_eq!(idx("0b10", 8), Some(2));
        assert_eq!(idx("0o3", 8), Some(3));
        assert_eq!(idx("+1", 5), Some(1));
        assert_eq!(idx("-1", 5), Some(-1));
    }

    #[test]
    fn index_chains() {
        // Exactly two terms; the second keeps its own sign.
        assert_eq!(idx("0+0", 8), Some(0));
        assert_eq!(idx("1+1", 8), Some(2));
        assert_eq!(idx("1-1", 8), Some(0));
        assert_eq!(idx("1++1", 8), Some(2));
        assert_eq!(idx("-1+2", 8), Some(1));
        assert_eq!(idx("-1--2", 8), Some(1));
        assert_eq!(idx("end+-1", 4), Some(2));
        assert_eq!(idx("end--1", 4), Some(4)); // past the end; caller clamps
        assert_eq!(idx("end+1", 4), Some(4));
        assert_eq!(idx("end-1", 4), Some(2));
        // Internal whitespace is rejected, chains of 3 terms too.
        assert_eq!(idx("1 + 1", 8), None);
        assert_eq!(idx(" 1+ 1 ", 8), None);
        assert_eq!(idx("1+1+1", 8), None);
        assert_eq!(idx("end - 1", 8), None);
        assert_eq!(idx("1+", 8), None);
        assert_eq!(idx("++1", 8), None);
        assert_eq!(idx("x+1", 8), None);
        assert_eq!(idx("end1", 8), None);
        assert_eq!(idx("END", 8), None);
        assert_eq!(idx("", 8), None);
    }

    #[test]
    fn index_malformed_octal() {
        // These still parse nothing; the caller's bad_index() adds the
        // "(looks like invalid octal number)" hint for them.
        assert_eq!(idx("008", 8), None);
        assert_eq!(idx("08", 8), None);
        assert_eq!(idx("+008", 8), None);
        assert_eq!(idx("0x", 8), None);
        assert_eq!(idx("0b2", 8), None);
        assert_eq!(idx("1.5", 8), None);
    }

    #[test]
    fn index_32bit_bounds_and_wrap() {
        // Term magnitude must stay under 2^32; the sum wraps to C int.
        assert_eq!(idx("4294967295", 8), Some(-1)); // wraps, out of range
        assert_eq!(idx("4294967296", 8), None);
        assert_eq!(idx("-4294967295", 8), Some(1)); // wraps to index 1
        assert_eq!(idx("-4294967296", 8), None);
        assert_eq!(idx("9223372036854775807", 8), None);
        assert_eq!(idx("9223372036854775807+1", 8), None);
        assert_eq!(idx("2147483647+1", 8), Some(-2147483648));
        assert_eq!(idx("end+4294967295", 8), Some(6)); // (7 + 4294967295) as i32
        assert_eq!(idx("4294967295-4294967295", 8), Some(0));
    }

    // --- concat_trim: Tcl_Concat's backslash-aware whitespace trim ---

    #[test]
    fn concat_trim_escapes() {
        assert_eq!(concat_trim("a b"), "a b");
        assert_eq!(concat_trim("  x  "), "x");
        assert_eq!(concat_trim("b\\ "), "b\\ "); // escaped space is kept
        assert_eq!(concat_trim("b\\   "), "b\\ "); // util-4.2
        // util-4.3: the escaped backslash's trailing space survives too —
        // tclsh's `a b\\  c` has two spaces before `c`.
        assert_eq!(concat_trim("b\\\\   "), "b\\\\ ");
        assert_eq!(concat_trim("x\\\\ "), "x\\\\ "); // probed round 5
        assert_eq!(concat_trim("   "), "");
    }

    #[test]
    fn test_lsubst_simple() {
        let mut interp = Interp::new();
        let r = interp.eval("lsubst {a b c}").unwrap();
        assert_eq!(r.as_str(), "a b c");
    }

    #[test]
    fn test_lsubst_variable() {
        let mut interp = Interp::new();
        interp.eval("set x hello").unwrap();
        let r = interp.eval("lsubst {$x world}").unwrap();
        assert_eq!(r.as_str(), "hello world");
    }

    #[test]
    fn test_lsubst_novariables() {
        let mut interp = Interp::new();
        interp.eval("set x hello").unwrap();
        let r = interp.eval("lsubst -novariables {$x world}").unwrap();
        // $x should not be substituted
        let list = r.as_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_str(), "$x");
        assert_eq!(list[1].as_str(), "world");
    }

    #[test]
    fn test_lsubst_no_args_error() {
        let mut interp = Interp::new();
        assert!(interp.eval("lsubst").is_err());
    }

    #[test]
    fn test_lsubst_bad_option() {
        let mut interp = Interp::new();
        assert!(interp.eval("lsubst -badopt {a b}").is_err());
    }
}
