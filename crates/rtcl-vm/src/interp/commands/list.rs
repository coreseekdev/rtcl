//! List commands: list, llength, lindex, lappend, lrange, linsert, lreplace,
//! lassign, lrepeat, lreverse, concat, split, join, lmap, lset, lsubst.
//! See list_sort.rs for lsearch and lsort.

use std::borrow::Cow;

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::{is_tcl_space, SeriesRep, Value};


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
    interp.global_insert("errorCode".to_string(), Value::from_str(code));
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

/// Tcl's bad-index error (`TCL VALUE INDEX`). 9.1 dropped the
/// "(looks like invalid octal number)" hint (TclCheckBadOctal) — oracle:
/// `string index abc 0o8` is the plain message.
pub(crate) fn bad_index(interp: &mut Interp, idx: &str) -> Error {
    set_error_code(interp, "TCL VALUE INDEX");
    tcl_err(format!(
        "bad index \"{}\": must be integer?[+-]integer? or end?[+-]integer?",
        idx
    ))
}

/// Tcl's whitespace set (also the list element separators).
pub(crate) fn trim_tcl_space(s: &str) -> &str {
    s.trim_matches(|c: char| is_tcl_space(c as u8))
}

/// Scan a Tcl integer at the start of `s` (9.1 TclParseNumber integer
/// rules: optional sign; `0x`/`0b`/`0o`/`0d` radix prefixes; a bare
/// leading `0` is DECIMAL — legacy octal is gone; `_` separators between
/// digits). Returns (value, bytes consumed) for prefix positions such as
/// index terms; a partial scan yields the accepted prefix.
pub(crate) fn scan_tcl_int(s: &str) -> Option<(i64, usize)> {
    let scan = crate::types::bignum::scan_number(s, true)?;
    let crate::types::bignum::NumPayload::Int { radix, run } = scan.payload else {
        return None;
    };
    let digits = scan.int_digits(s);
    if digits.is_empty() || digits.bytes().all(|b| b == b'_') {
        // Prefix without digits (`0x`) — not a value; the caller reports
        // a bad index for the partial text.
        return None;
    }
    // Accumulate with checked math: an index magnitude past i64 is not a
    // value here (from_str_radix overflowed the same way before).
    let mut mag: u64 = 0;
    for &b in digits.as_bytes() {
        if b == b'_' {
            continue;
        }
        mag = mag
            .checked_mul(radix as u64)
            .and_then(|m| m.checked_add(crate::types::bignum::digit_value(b) as u64))?;
    }
    if mag > i64::MAX as u64 {
        return None;
    }
    let val = mag as i64;
    Some((if scan.neg { -val } else { val }, scan.stop))
}

/// Tcl_GetInt semantics: the whole (whitespace-trimmed) string is an integer.
pub(crate) fn tcl_get_int(s: &str) -> Option<i64> {
    let t = trim_tcl_space(s);
    let (v, used) = scan_tcl_int(t)?;
    if used == t.len() { Some(v) } else { None }
}

/// tclsh's static ceilings, both consequences of C-`int` sized lengths:
/// a string value is at most INT_MAX bytes, a list at most 536870909
/// elements. `string repeat` and `lrepeat` report an exceedance as a
/// `TCL MEMORY` error instead of attempting the allocation.
///
/// 9.1 迁移注记：9.x 的长度是 64 位（Tcl_Size），这两个常数不再是
/// tclsh 的上限——lrepeat 的真上限是 `TCL91_LIST_MAX`（见下），string
/// repeat 的真上限是 `TCL_SIZE_MAX` 字节。它们降格为 **rtcl 的物化
/// 预算**：tclsh 9 对大 lrepeat 走惰性 lrepeatType 表示（不逐元素
/// 分配）、对巨 string repeat 真去 alloc；rtcl 一律急切物化，超过预算
/// 的按 9.1 的错误形状拒绝（形状逐字节可比，数字是预算本身），绝不
/// 进入不可捕获的分配中止。
pub(crate) const MAX_VALUE_BYTES: usize = i32::MAX as usize;
pub(crate) const MAX_LIST_ELEMENTS: usize = 536870909;

/// tclsh 9.1.0（x86-64）的 `LIST_MAX`：tclInt.h 的
/// `(TCL_SIZE_MAX - offsetof(ListStore, slots)) / sizeof(Tcl_Obj *)`，
/// 实测 `lrepeat 9223372036854775807 x` 报
/// `max length (1152921504606846970) of a Tcl list exceeded`。
pub(crate) const TCL91_LIST_MAX: u64 = 1152921504606846970;

/// A repeat/element count in Tcl 9's range（`Tcl_Size`，64 位有符号——
/// 8.6 的 C `int` 上限已随 TIP 626 移除，`4294967296` 是合法 count）：
/// 有效整数但超出 i64（bignum）报 `integer value too large to
/// represent`（ARITH IOVERFLOW），不是数字报 `expected integer but
/// got "…"`（TCL VALUE NUMBER）——都在分配之前（实测钉形）。
pub(crate) fn tcl_count(interp: &mut Interp, v: &Value) -> Result<i64> {
    match v.as_int() {
        Some(n) => Ok(n),
        None => match crate::types::bignum::parse_int(v.as_str().trim()) {
            Some(_) => {
                set_error_code(
                    interp,
                    "ARITH IOVERFLOW {integer value too large to represent}",
                );
                Err(tcl_err("integer value too large to represent"))
            }
            None => {
                set_error_code(interp, "TCL VALUE NUMBER");
                Err(tcl_err(format!("expected integer but got \"{}\"", v.as_str())))
            }
        },
    }
}

/// A fallible allocation that ran out of room (Rust's default handler
/// aborts the process on a failed allocation, which Tcl `catch` cannot
/// intercept).
pub(crate) fn out_of_memory(interp: &mut Interp, bytes: usize) -> Error {
    set_error_code(interp, "TCL MEMORY");
    tcl_err(format!("not enough memory to allocate {} bytes", bytes))
}

/// Parse a Tcl index (`integer?[+-]integer?` or `end?[+-]integer?`),
/// resolving `end` against `len`. The result may be negative or >= len;
/// callers clamp or report out-of-range as appropriate.
///
/// 9.1 semantics (TIP 626 / tclUtil.c Tcl_GetIntForIndexM，oracle 实测
/// 2026-11-27，E1 批 util-9.x)：项是任意大小整数——整个值先按数解析
/// （超出 wide 的 bignum 按符号饱和到 ±WIDE_MAX）；`end±int` /
/// `int±int` 两项各保留自己的符号，bignum 精确求和后按符号截断到
/// ±WIDE_MAX。**越界不再是错误**——`string index a 0x1FFFFFFFF+0` 是
/// 空串而非 bad index；8.6 的 2^32-1 项界与 C int 回绕（wrap_i32）随
/// 之废除（`-4294967295` 不再回绕成 1）。仅真正解析不出（`0x`、
/// `1.5`、`1+1+1`、`1 + 1`、多元素列表）才 None → bad index。
pub(crate) fn parse_tcl_index(s: &str, len: usize) -> Option<i64> {
    use crate::types::bignum::{parse_int, scan_number, IntRep, NumPayload};
    use num_bigint::BigInt;
    use num_traits::{Signed, ToPrimitive};
    let end_value = len as i64 - 1;
    let t = trim_tcl_space(s);
    // 按符号饱和到 wide（Tcl：BIG → ±WIDE_MAX，C 源 mp_isneg 分支）。
    // 热路径（`12345`/`end-7`）全程 i64/i128，零 BigInt 分配——只有
    // bignum 项才提升（基准 e1_iostr_bench：BigInt 回退曾致 ix 命中
    // +20%）。
    fn sat_i128(v: i128) -> i64 {
        if v > i64::MAX as i128 {
            i64::MAX
        } else if v < i64::MIN as i128 {
            i64::MIN
        } else {
            v as i64
        }
    }
    let saturate = |v: &BigInt| -> i64 {
        match v.to_i64() {
            Some(i) => i,
            None => {
                if v.is_negative() {
                    i64::MIN
                } else {
                    i64::MAX
                }
            }
        }
    };
    // 两项求和：双 i64 走 i128 精确（end/int±int 的 wide 数学饱和等价
    // i128 后饱和）；含 bignum 走 BigInt 精确后截断（oracle：任一项
    // BIG → 全式 expr 精确求值再 truncate）。
    fn op_sum(a: IntRep, minus: bool, b: IntRep) -> i64 {
        use num_traits::{Signed, ToPrimitive};
        match (a, b) {
            (IntRep::I64(x), IntRep::I64(y)) => {
                let (x, y) = (x as i128, y as i128);
                sat_i128(if minus { x - y } else { x + y })
            }
            (a, b) => {
                let (x, y) = (a.to_big(), b.to_big());
                let v = if minus { x - y } else { x + y };
                match v.to_i64() {
                    Some(i) => i,
                    None => {
                        if v.is_negative() {
                            i64::MIN
                        } else {
                            i64::MAX
                        }
                    }
                }
            }
        }
    }
    // 1. 整值数字（Tcl_GetNumberFromObj 先行；end/表达式不是数字）。
    if let Some(rep) = parse_int(t) {
        return Some(match rep {
            IntRep::I64(i) => i,
            r => saturate(&r.to_big()),
        });
    }
    // 2. "end[+-]integer"：操作符后无空白，第二项保留自己的符号。
    if let Some(rest) = t.strip_prefix("end") {
        if rest.is_empty() {
            return Some(end_value);
        }
        let rb = rest.as_bytes();
        if rb.len() < 2 || (rb[0] != b'+' && rb[0] != b'-') || is_tcl_space(rb[1]) {
            return None;
        }
        let off = parse_int(&rest[1..])?;
        return Some(op_sum(IntRep::I64(end_value), rb[0] == b'-', off));
    }
    // 3. "integer[+-]integer"：首项是前缀扫描（剩余留给第二项）。
    //    量级在 i64 内攒 u64（与 parse_int 同法），溢出转 bignum。
    let scan = scan_number(t, true)?;
    let NumPayload::Int { radix, run } = scan.payload else {
        return None;
    };
    let digits = scan.int_digits(t);
    if digits.is_empty() || digits.bytes().all(|b| b == b'_') {
        return None;
    }
    let mut mag: u64 = 0;
    let mut overflow = false;
    for &b in digits.as_bytes() {
        if b == b'_' {
            continue;
        }
        mag = match mag
            .checked_mul(radix as u64)
            .and_then(|m| m.checked_add(crate::types::bignum::digit_value(b) as u64))
        {
            Some(v) => v,
            None => {
                overflow = true;
                break;
            }
        };
    }
    let first = if overflow {
        let stripped: String =
            digits.bytes().filter(|&b| b != b'_').map(|b| b as char).collect();
        let big = BigInt::parse_bytes(stripped.as_bytes(), radix)?;
        IntRep::Big(if scan.neg { -big } else { big })
    } else if scan.neg && mag == (i64::MAX as u64) + 1 {
        IntRep::I64(i64::MIN)
    } else if mag > i64::MAX as u64 {
        // 符号化后超 wide：精确 Big（与 parse_int 同）——不可提前饱和，
        // `hugeA-hugeB` 型精确差仍可能落在 wide 内（oracle 钉形）。
        let big = BigInt::from(mag);
        IntRep::Big(if scan.neg { -big } else { big })
    } else {
        IntRep::I64(if scan.neg { -(mag as i64) } else { mag as i64 })
    };
    let rest = &t[scan.stop..];
    if rest.is_empty() {
        return Some(match first {
            IntRep::I64(i) => i,
            IntRep::Big(ref b) => saturate(b),
        });
    }
    let rb = rest.as_bytes();
    if rb.len() < 2 || (rb[0] != b'+' && rb[0] != b'-') || is_tcl_space(rb[1]) {
        return None;
    }
    let second = parse_int(&rest[1..])?;
    Some(op_sum(first, rb[0] == b'-', second))
}

/// Whether a Tcl index is spelled from `end` — the one form whose value
/// depends on the length it is resolved against.
pub(crate) fn index_is_from_end(s: &str) -> bool {
    trim_tcl_space(s).starts_with("end")
}

pub fn cmd_list(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    Ok(Value::from_list(&args[1..]))
}

pub fn cmd_llength(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("llength", "list"));
    }
    if let Some(s) = args[1].series() {
        return Ok(Value::from_int(s.len));
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
            if let Some(s) = current.series() {
                let idx_str = idx_val.as_str();
                match parse_tcl_index(idx_str, s.len.max(0) as usize) {
                    Some(raw) if raw >= 0 && (raw as i64) < s.len => s.element(raw as i64),
                    Some(_) => return Ok(Value::empty()),
                    None => return Err(bad_index(interp, idx_str)),
                }
            } else {
                let list = strict_list_cow(interp, &current)?;
                let idx_str = idx_val.as_str();
                let len = list.len();
                match parse_tcl_index(idx_str, len) {
                    Some(raw) if raw >= 0 && (raw as usize) < len => list[raw as usize].clone(),
                    Some(_) => return Ok(Value::empty()),
                    None => return Err(bad_index(interp, idx_str)),
                }
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
    // mutation guard holds (`var_slot_fast`: no link, array, trace or
    // qualifier) — append in place to the list its slot holds, as tclsh
    // appends to the unshared list object.  Amortised O(1), and nothing
    // leaves a table (a value taken out and put back left a second entry
    // in the insertion log, which moved the variable at the next
    // listing).  Creation, string-rep sources, links, arrays and traced
    // variables keep the rebuilding path below; so does a value that is
    // not a list rep — the strict parse there owns its errors.
    if args.len() >= 3 {
        // D1 步骤 1：with_var_slot（全局臂双写 arena + 镜像重放）。
        if let Some(v) = interp.with_var_slot(var_name, |cell| {
            let items = cell.as_list_mut()?;
            for arg in &args[2..] {
                items.push(arg.clone());
            }
            Some(cell.clone())
        }) {
            return Ok(v);
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
    // Lazy lseq series: resolve the requested window without materializing
    // the whole (possibly 2^31-1 element) list.
    if let Some(s) = args[1].series() {
        let len = usize::try_from(s.len.max(0)).unwrap_or(usize::MAX);
        let first_raw = match parse_tcl_index(args[2].as_str(), len) {
            Some(r) => r,
            None => return Err(bad_index(interp, args[2].as_str())),
        };
        let last_raw = match parse_tcl_index(args[3].as_str(), len) {
            Some(r) => r,
            None => return Err(bad_index(interp, args[3].as_str())),
        };
        let first = first_raw.max(0);
        let last = last_raw.min(len as i64 - 1);
        if first <= last && (first as usize) < len {
            let mut out = Vec::with_capacity((last - first + 1) as usize);
            for i in first..=last {
                out.push(s.element(i));
            }
            return Ok(Value::from_list(&out));
        }
        return Ok(Value::empty());
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
    let count = tcl_count(interp, &args[1])?;
    if count < 0 {
        set_error_code(interp, "TCL OPERATION LREPEAT NEGARG");
        return Err(tcl_err(format!(
            "bad count \"{}\": must be integer >= 0",
            args[1].as_str()
        )));
    }
    let elements: Vec<Value> = args[2..].to_vec();
    if elements.is_empty() {
        return Ok(Value::empty());
    }
    // tclsh 9.1 checks `objc > LIST_MAX/repeatCount` before building
    // (Tcl_ListObjRepeat) and reports `max length (N) of a Tcl list
    // exceeded` with TCL MEMORY.  rtcl materializes eagerly, so a second,
    // smaller budget applies below LIST_MAX — the same message shape with
    // the budget as the number (tclsh never errors inside its budget, so
    // that band has no byte-comparable oracle).
    let over = |interp: &mut Interp, max: u64| -> Error {
        set_error_code(interp, "TCL MEMORY");
        tcl_err(format!("max length ({max}) of a Tcl list exceeded"))
    };
    // S21 wasm32 兼容：count×elements 的守卫留在 u128 域——usize=u32
    // 时 `count as usize` 先窄化会把 2^32 截成 0，守卫全被绕过且循环
    // 体静默产出空列表（silently wrong）。预算内（≤ MAX_LIST_ELEMENTS
    // < u32::MAX）再窄化，两个目标都安全。
    let total_wide = (count as u128) * (elements.len() as u128);
    if total_wide > TCL91_LIST_MAX as u128 {
        return Err(over(interp, TCL91_LIST_MAX));
    }
    if total_wide > MAX_LIST_ELEMENTS as u128 {
        return Err(over(interp, MAX_LIST_ELEMENTS as u64));
    }
    let count = count as usize;
    let total = total_wide as usize;
    let mut result = Vec::new();
    result
        .try_reserve(total)
        .map_err(|_| out_of_memory(interp, total * core::mem::size_of::<Value>()))?;
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
        return super::loops::foreach_lexical(interp, args, super::loops::LoopKind::Lmap);
    }

    let body = &args[args.len() - 1];
    let mut collected: Vec<Value> = Vec::new();

    struct VarGroup<'a> {
        vars: Vec<String>,
        data: super::loops::ListData<'a>,
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
        let data = super::loops::list_data(interp, &args[i + 1])?;
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
                let value = g.data.get(data_idx).unwrap_or_else(Value::empty);
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

/// lfilter varList list ?varList list ...? command — TIP 735 (tclsh 9.1).
/// Like lmap, but collects the value of the FIRST variable of the FIRST
/// varlist for every iteration whose body completes with a TRUE boolean
/// (tclCmdAH.c `EachloopCmd(TCL_EACH_FILTER)` + `ForeachLoopStep`).
pub fn cmd_lfilter(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 || !args.len().is_multiple_of(2) {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lfilter", "varList list ?varList list ...? command"));
    }

    // Same compiled-shape gate as foreach/lmap (tclsh compiles lfilter
    // inside proc bodies the same way — TclCompileLfilterCmd).
    if interp.lexical_body && super::loops::foreach_inline_shape(interp, args) {
        return super::loops::foreach_lexical(interp, args, super::loops::LoopKind::Lfilter);
    }

    let body = &args[args.len() - 1];
    let mut collected: Vec<Value> = Vec::new();

    struct VarGroup<'a> {
        vars: Vec<String>,
        data: super::loops::ListData<'a>,
    }
    let mut groups: Vec<VarGroup> = Vec::new();
    let mut i = 1;
    while i < args.len() - 1 {
        let var_list = strict_list(interp, &args[i])?;
        if var_list.is_empty() {
            set_error_code(interp, "TCL OPERATION LFILTER NEEDVARS");
            return Err(tcl_err("lfilter varlist is empty"));
        }
        let vars: Vec<String> = var_list.iter().map(|v| v.as_str().to_string()).collect();
        let data = super::loops::list_data(interp, &args[i + 1])?;
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
        // primeValueObj — the value assigned to the first variable of the
        // first varlist, captured before the body runs (tclCmdAH.c).
        let mut prime: Option<Value> = None;
        for (gi, g) in groups.iter().enumerate() {
            let n = g.vars.len();
            for (vi, var) in g.vars.iter().enumerate() {
                let data_idx = idx * n + vi;
                let value = g.data.get(data_idx).unwrap_or_else(Value::empty);
                if gi == 0 && vi == 0 {
                    prime = Some(value.clone());
                }
                if let Err(e) = interp.set_var(var, value) {
                    // Dispatched lfilter decorates a loop-variable write
                    // failure like dispatched foreach/lmap.
                    if interp.err_is_error(&e) {
                        set_error_code(interp, "TCL WRITE VARNAME");
                        if interp.err_info.is_none() {
                            interp.err_info = Some(e.message_text());
                        }
                        if let Some(info) = &mut interp.err_info {
                            info.push_str(&format!(
                                "\n    (setting lfilter loop variable \"{}\")",
                                var
                            ));
                        }
                    }
                    return Err(e);
                }
            }
        }
        // A `return -level 0 $v` completes as an ordinary command whose
        // result is $v; the boolean decides collection, the loop keeps
        // iterating (same shape as lmap's handling).
        match interp.eval_body_value(body) {
            Ok(v) => {
                if let Some(p) = prime {
                    match crate::types::expr_funcs::strict_bool(&v) {
                        Ok(true) => collected.push(p),
                        Ok(false) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(e) => {
                if e.is_break() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    break;
                }
                if e.is_continue() {
                    if e.loop_level() > 1 { return Err(e.with_decremented_loop_level()); }
                    continue;
                }
                // Dispatched lfilter adds its own body exit frame between
                // the failing command's frame and the lfilter command's
                // harness frame (tclCmdAH.c: `("lfilter" body line N)`).
                interp.err_exit_frame("\"lfilter\" body");
                return Err(e);
            }
        }
    }

    Ok(Value::from_list(&collected))
}

/// lseq — numeric series (tclsh 9.1, TIP 746: no expr evaluation; the
/// arguments decode per tclCmdIL.c `Tcl_LseqObjCmd`'s arg_key state
/// machine, the math per tclArithSeries.c).
pub fn cmd_lseq(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    use crate::types::bignum::{int_rep, IntRep};

    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        Dots,
        To,
        Count,
        By,
    }
    #[derive(Clone, Copy)]
    enum Num {
        Int(i64),
        Big,
        Dbl(f64),
    }

    let wrong = |interp: &mut Interp| -> Error {
        set_error_code(interp, "TCL WRONGARGS");
        wrong_args("lseq", "n ??op? n ??by? n??")
    };
    // Tcl_GetNumberFromObj's view of a value: integer (i64/big), double,
    // or none. Booleans are not numbers here.
    fn as_number(v: &Value) -> Option<Num> {
        if let Some(rep) = int_rep(v) {
            return Some(match rep {
                IntRep::I64(i) => Num::Int(i),
                IntRep::Big(_) => Num::Big,
            });
        }
        v.as_float().map(Num::Dbl)
    }
    let num_as_f64 = |n: Num| -> f64 {
        match n {
            Num::Int(i) => i as f64,
            Num::Big => f64::NAN, // bignum: unsupported (C errors via GetWideInt)
            Num::Dbl(d) => d,
        }
    };

    // ── Decode ──────────────────────────────────────────────────────────
    if args.len() > 6 {
        return Err(wrong(interp));
    }
    // allowedArgs bit flags, mirroring the C decode loop.
    const NUMERIC: u8 = 1;
    const RANGE_KW: u8 = 2;

    let objc = args.len(); // includes "lseq"
    let mut arg_key: u32 = 0;
    let mut nums: Vec<(Value, Num)> = Vec::new(); // numeric args + kind
    let mut op_values: Vec<Op> = Vec::new(); // keyword arg opcodes
    let mut use_doubles = 0usize;
    let mut rem_nums = 3i32;
    let mut allowed = NUMERIC;

    for i in 1..objc {
        arg_key = arg_key.wrapping_mul(10);
        let v = &args[i];
        let is_last = i == objc - 1;
        if allowed & NUMERIC != 0 {
            if let Some(n) = as_number(v) {
                rem_nums -= 1;
                arg_key += 1;
                allowed = RANGE_KW;
                if rem_nums != 1 || (objc - 1 - i) != 2 {
                    allowed |= NUMERIC;
                }
                if matches!(n, Num::Dbl(_)) {
                    use_doubles += 1;
                }
                nums.push((v.clone(), n));
                continue;
            }
        }
        if allowed & RANGE_KW != 0 {
            // tclsh's abbreviation lookup is not deterministic across
            // invocation shapes (internal index-rep caching quirk); the
            // manual and the corpus spell the operators out, so exact
            // match only.
            let op = match v.as_str() {
                ".." => Some(Op::Dots),
                "to" => Some(Op::To),
                "count" => Some(Op::Count),
                "by" => Some(Op::By),
                _ => None,
            };
            if let Some(op) = op {
                if is_last {
                    return Err(tcl_err(format!("missing \"{}\" value.", v.as_str())));
                }
                arg_key += 2;
                allowed = NUMERIC;
                op_values.push(op);
                continue;
            }
        }
        if allowed & NUMERIC == 0 {
            // NoneArg: report through the operation table (tclCmdIL.c
            // calls Tcl_GetIndexFromObj with msg "operation").
            set_error_code(interp, "TCL LOOKUP INDEX operation");
            return Err(tcl_err(format!(
                "bad operation \"{}\": must be .., to, count, or by",
                v.as_str()
            )));
        }
        // ErrArg: numeric was allowed but the value is not a number
        // (Tcl_GetNumberFromObj's error, TCL VALUE NUMBER).
        set_error_code(interp, "TCL VALUE NUMBER");
        return Err(tcl_err(format!("expected number but got \"{}\"", v.as_str())));
    }

    // ── Shape dispatch (arg_key) ────────────────────────────────────────
    let mut start: Option<Value> = None;
    let mut end: Option<Value> = None;
    let mut step: Option<Value> = None;
    let mut count: Option<Value> = None;
    let nth = |nums: &mut Vec<(Value, Num)>, i: usize| -> Option<Value> {
        nums.get(i).map(|(v, _)| v.clone())
    };
    let num_at = |nums: &[(Value, Num)], v: &Option<Value>| -> Option<Num> {
        let v = v.as_ref()?;
        nums.iter().find(|(val, _)| val.same_allocation(v)).map(|(_, n)| *n)
    };
    match arg_key {
        1 => {
            // lseq n — integers only ("3.0" converts, "3.1" errors)
            start = Some(Value::from_int(0));
            count = nth(&mut nums, 0);
            use_doubles = 0;
        }
        11 => {
            start = nth(&mut nums, 0);
            end = nth(&mut nums, 1);
        }
        111 => {
            start = nth(&mut nums, 0);
            end = nth(&mut nums, 1);
            step = nth(&mut nums, 2);
        }
        121 => {
            let first = nth(&mut nums, 0);
            let third = nth(&mut nums, 1);
            match op_values[0] {
                Op::Dots | Op::To => {
                    start = first;
                    end = third;
                }
                Op::By => {
                    start = Some(Value::from_int(0));
                    count = first;
                    step = third;
                }
                Op::Count => {
                    start = first;
                    count = third;
                }
            }
        }
        1211 => {
            let first = nth(&mut nums, 0);
            let third = nth(&mut nums, 1);
            let fourth = nth(&mut nums, 2);
            match op_values[0] {
                Op::Dots | Op::To => {
                    start = first;
                    end = third;
                    step = fourth;
                }
                Op::Count => {
                    start = first;
                    count = third;
                    step = fourth;
                }
                _ => return Err(wrong(interp)),
            }
        }
        1121 => {
            start = nth(&mut nums, 0);
            end = nth(&mut nums, 1);
            let fourth = nth(&mut nums, 2);
            if op_values[0] != Op::By {
                return Err(wrong(interp));
            }
            step = fourth;
        }
        12121 => {
            let first = nth(&mut nums, 0);
            let third = nth(&mut nums, 1);
            let fifth = nth(&mut nums, 2);
            if op_values[1] != Op::By {
                return Err(wrong(interp));
            }
            step = fifth;
            match op_values[0] {
                Op::Dots | Op::To => {
                    start = first;
                    end = third;
                }
                Op::Count => {
                    start = first;
                    count = third;
                }
                Op::By => return Err(wrong(interp)),
            }
        }
        _ => return Err(wrong(interp)),
    }

    // ── Count conversion (tclCmdIL.c: integral double converts; anything
    // else fails Tcl_GetWideIntFromObj) ─────────────────────────────────
    let mut count_val: Option<i64> = None;
    if let Some(cv) = &count {
        let wide = |interp: &mut Interp, v: &Value| -> Error {
            set_error_code(interp, "TCL VALUE INTEGER");
            tcl_err(format!("expected integer but got \"{}\"", v.as_str()))
        };
        count_val = Some(match num_at(&nums, &count) {
            Some(Num::Int(i)) => i,
            Some(Num::Big) => return Err(wide(interp, cv)),
            Some(Num::Dbl(d)) => {
                use_doubles = use_doubles.saturating_sub(1);
                if d.is_finite() && d.floor() == d && d >= -(2f64.powi(63)) && d < 2f64.powi(63) {
                    d as i64
                } else {
                    return Err(wide(interp, cv));
                }
            }
            None => return Err(wide(interp, cv)),
        });
    }

    // ── Extract numbers (ExtractNumber) ─────────────────────────────────
    let use_d = use_doubles != 0;
    let as_wide = |interp: &mut Interp, n: Num, v: &Value| -> Result<i64> {
        match n {
            Num::Int(i) => Ok(i),
            Num::Big | Num::Dbl(_) => {
                // int mode: a double truncates; out of wide range or a
                // bignum fails GetWideIntFromObj.
                let d = num_as_f64(n);
                if d.is_finite() && d >= -(2f64.powi(63)) && d < 2f64.powi(63) {
                    Ok(d as i64)
                } else {
                    set_error_code(interp, "TCL VALUE INTEGER");
                    Err(tcl_err(format!("expected integer but got \"{}\"", v.as_str())))
                }
            }
        }
    };
    let (mut istart, mut iend, mut istep) = (0i64, 0i64, 1i64);
    let (mut dstart, mut dend, mut dstep) = (0.0f64, 0.0f64, 1.0f64);
    let start_obj = start.clone();
    let end_obj = end.clone();
    let step_obj = step.clone();
    if let Some(sv) = &step {
        let n = num_at(&nums, &step).unwrap_or(Num::Int(1));
        if use_d {
            dstep = num_as_f64(n);
        } else {
            istep = as_wide(interp, n, sv)?;
        }
    }
    if let Some(sv) = &start {
        let n = num_at(&nums, &start).unwrap_or(Num::Int(0));
        if use_d {
            dstart = num_as_f64(n);
        } else {
            istart = as_wide(interp, n, sv)?;
        }
    }
    if let Some(ev) = &end {
        let n = num_at(&nums, &end).unwrap_or(Num::Int(0));
        if use_d {
            dend = num_as_f64(n);
        } else {
            iend = as_wide(interp, n, ev)?;
        }
    }

    // ── Precision (ObjPrecision: fraction digits of the arg text for
    // doubles, 0 for ints and e-notation) ────────────────────────────────
    fn obj_precision(v: &Value) -> u32 {
        let s = v.as_str();
        if s.contains('e') || s.contains('E') {
            return 0;
        }
        match s.find('.') {
            Some(pos) => (s.len() - pos - 1) as u32,
            None => 0,
        }
    }
    let prec_of = |v: &Option<Value>| -> u32 { v.as_ref().map(|v| obj_precision(v)).unwrap_or(0) };

    // ── Series math helpers (tclArithSeries.c) ──────────────────────────
    let arith_round = |d: f64, n: u32| -> f64 {
        if n == 0 {
            return d;
        }
        let scale = 10f64.powi(n as i32);
        (d * scale).round() / scale
    };
    let arith_len_int = |start: i64, end: i64, step: i64| -> i64 {
        if (step < 0 && end.wrapping_sub(start) > 0)
            || (step > 0 && end.wrapping_sub(start) < 0)
        {
            return 0;
        }
        if step == 0 {
            return 1;
        }
        let len = end.wrapping_sub(start).wrapping_div(step).wrapping_add(1);
        if len < 0 {
            0
        } else {
            len
        }
    };
    let arith_len_dbl =
        |mut start: f64, mut end: f64, mut step: f64, precision: u32| -> i64 {
            if step == 0.0 {
                return 1;
            }
            if precision > 0 {
                let scale = 10f64.powi(precision as i32);
                start *= scale;
                end *= scale;
                step *= scale;
            }
            end -= start;
            const WIDE_MIN_F: f64 = -9223372036854775808.0;
            const WIDE_MAX_F: f64 = 9223372036854775808.0;
            if (WIDE_MIN_F..=WIDE_MAX_F).contains(&end)
                && (WIDE_MIN_F..=WIDE_MAX_F).contains(&step)
            {
                let iend = if end < 0.0 {
                    (end - 0.5) as i64
                } else {
                    (end + 0.5) as i64
                };
                let istep = if step < 0.0 {
                    (step - 0.5) as i64
                } else {
                    (step + 0.5) as i64
                };
                if istep != 0 {
                    let ilen = iend.wrapping_div(istep).wrapping_add(1);
                    return if ilen < 0 { 0 } else { ilen };
                }
            }
            let mut len = (end / step) + 1.0;
            // TCL_SIZE_MAX (Tcl_Size = wide in 9.1); the C cast of a value
            // beyond i64 range lands on i64::MIN (x86-64 cvttsd2si), which
            // NewArithSeriesDbl then treats as empty (lseq 0 1e20 -> {}).
            if len >= 1.8446744073709552e19 {
                len = 1.8446744073709552e19;
            }
            if len <= 0.0 {
                len = 1.0;
            }
            if len >= 9.223372036854776e18 {
                i64::MIN
            } else {
                len as i64
            }
        };

    // ── Length ──────────────────────────────────────────────────────────
    let mut len: i64;
    if end_obj.is_some() {
        if step_obj.is_none() {
            if use_d {
                if dstart > dend {
                    dstep = -1.0;
                }
            } else if istart > iend {
                istep = -1;
            }
        }
        if count_val.is_none() {
            if use_d {
                if dstart.is_infinite() || dend.is_infinite() {
                    set_error_code(interp, "TCL MEMORY");
                    return Err(tcl_err("max length of a Tcl list exceeded"));
                }
                if dstart.is_nan() || dend.is_nan() {
                    set_error_code(interp, "ARITH DOMAIN non-numeric floating-point value");
                    return Err(tcl_err(
                        "cannot use non-numeric floating-point value \"NaN\" to estimate length of arith-series",
                    ));
                }
                let precision = prec_of(&start_obj).max(prec_of(&end_obj)).max(prec_of(&step_obj));
                len = arith_len_dbl(dstart, dend, dstep, precision);
            } else {
                len = arith_len_int(istart, iend, istep);
            }
        } else {
            len = count_val.unwrap_or(1);
        }
    } else if use_d {
        // count form: the computed end participates in nothing further,
        // but its precision-matched value is what tclsh caches (kept for
        // parity of the abstract series end).
        let precision = prec_of(&start_obj).max(prec_of(&step_obj));
        len = count_val.unwrap_or(1);
        if len > 0 {
            dend = arith_round(dstart + dstep * ((len - 1) as f64), precision);
        }
    } else {
        len = count_val.unwrap_or(1);
    }
    if len < 0 {
        len = 0;
    }

    // ── Validation (NewArithSeriesInt's overflow checks) ────────────────
    let invalid = |interp: &mut Interp| -> Error {
        set_error_code(interp, "ARITH DOMAIN invalid arithmetic series parameter values");
        tcl_err("invalid arithmetic series parameter values")
    };
    if !use_d && len >= 1 {
        let num_intervals = (len - 1) as u64;
        let abs_step: u64 = if istep >= 0 {
            istep as u64
        } else if istep == i64::MIN {
            1u64 << 63
        } else {
            (-(istep as i128)) as u64
        };
        if abs_step != 0 && (u64::MAX / abs_step) < num_intervals {
            return Err(invalid(interp));
        }
        let product = (abs_step as i128) * (num_intervals as i128);
        if istep > 0 && (istart as i128) > (i64::MAX as i128) - product {
            return Err(invalid(interp));
        }
        if istep == i64::MIN && (num_intervals > 0 || istart < 0) {
            return Err(invalid(interp));
        }
        if istep < 0 && istep != i64::MIN && (istart as i128) < (i64::MIN as i128) + product {
            return Err(invalid(interp));
        }
    }
    if use_d && len >= 1 {
        let d = dstart + ((len - 1) as f64) * dstep;
        if d.is_nan() {
            set_error_code(interp, "ARITH DOMAIN domain error: argument not in valid range");
            return Err(tcl_err("domain error: argument not in valid range"));
        }
    }

    // ── Materialize (rtcl has no abstract series; the corpus exercises
    // bounded series only.  A length beyond the list guard fails instead
    // of OOMing.) ────────────────────────────────────────────────────────
    if len > MAX_LIST_ELEMENTS as i64 {
        let precision = prec_of(&start_obj).max(prec_of(&end_obj)).max(prec_of(&step_obj));
        return Ok(Value::from_series(SeriesRep {
            is_double: use_d,
            start: istart,
            step: istep,
            dstart,
            dstep,
            len,
            precision,
        }));
    }
    let n = len as usize;
    let mut out = Vec::new();
    out.try_reserve(n)
        .map_err(|_| out_of_memory(interp, n * core::mem::size_of::<Value>()))?;
    if use_d {
        let precision = prec_of(&start_obj).max(prec_of(&end_obj)).max(prec_of(&step_obj));
        for i in 0..n {
            let d = if i == 0 {
                dstart
            } else {
                dstart + (i as f64) * dstep
            };
            out.push(Value::from_float(arith_round(d, precision)));
        }
    } else {
        for i in 0..n {
            out.push(Value::from_int(
                istart.wrapping_add((i as i64).wrapping_mul(istep)),
            ));
        }
    }
    Ok(Value::from_list(&out))
}

/// lpop varName ?index ...? — remove and return an element (tclsh 9.1;
/// tclCmdIL.c Tcl_LpopObjCmd: extraction via TclLindexFlat, removal via
/// TclLsetFlat — an out-of-range index pops empty and then fails in the
/// removal with `index "raw" out of range`).
pub fn cmd_lpop(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lpop", "listvar ?index?"));
    }
    let var_name = args[1].as_str().to_string();
    let current = interp.get_var(&var_name)?.clone();
    let list = strict_list(interp, &current)?;

    // No index argument: pop the last element (an empty list reports the
    // same error as an explicit "end").
    let indices: Vec<Value> = if args.len() == 2 {
        if list.is_empty() {
            set_error_code(interp, "TCL VALUE INDEX OUTOFRANGE");
            return Err(tcl_err("index \"end\" out of range"));
        }
        vec![Value::from_int(list.len() as i64 - 1)]
    } else if args.len() == 3 {
        // lindex-style: a single index argument that itself parses as a
        // list is an index list; a malformed one is one index string.
        match args[2].as_list() {
            Some(l) => l.to_vec(),
            None => vec![args[2].clone()],
        }
    } else {
        args[2..].to_vec()
    };

    // Extract (lindex-flat semantics: out of range yields the empty
    // string, malformed intermediates error from the strict list parse).
    let extracted = lindex_flat(interp, &list, &indices)?;

    // Remove (lset-flat semantics: out of range errors with the raw
    // index spelling at the failing level).
    let new_list = lremove_flat(interp, &list, &indices)?;
    interp.set_var(&var_name, new_list)?;
    Ok(extracted)
}

/// lindex-flat: navigate `indices` from `list`, lenient about
/// out-of-range (empty result), strict about malformed intermediates.
fn lindex_flat(interp: &mut Interp, list: &[Value], indices: &[Value]) -> Result<Value> {
    let mut cur: Vec<Value> = list.to_vec();
    let mut element: Option<Value> = None;
    for idx in indices {
        let s = idx.as_str();
        let raw = parse_tcl_index(s, cur.len()).ok_or_else(|| bad_index(interp, s))?;
        if raw < 0 || raw >= cur.len() as i64 {
            element = Some(Value::empty());
            // Further indices operate on the empty list (lindex keeps
            // navigating; every remaining index is out of range).
            cur = Vec::new();
            continue;
        }
        let v = cur[raw as usize].clone();
        element = Some(v.clone());
        cur = strict_list(interp, &v)?;
    }
    Ok(element.unwrap_or_else(Value::empty))
}

/// lset-flat with a NULL value (removal): mirrors TclLsetFlat's errors —
/// `index "raw" out of range` at the failing level, list-parse errors for
/// malformed intermediates.
fn lremove_flat(interp: &mut Interp, list: &[Value], indices: &[Value]) -> Result<Value> {
    let idx_str = indices[0].as_str();
    let raw = parse_tcl_index(idx_str, list.len()).ok_or_else(|| bad_index(interp, idx_str))?;
    if raw < 0 || raw >= list.len() as i64 {
        set_error_code(interp, "TCL VALUE INDEX OUTOFRANGE");
        return Err(tcl_err(format!("index \"{}\" out of range", idx_str)));
    }
    let idx = raw as usize;
    let mut new_list = list.to_vec();
    if indices.len() == 1 {
        new_list.remove(idx);
    } else {
        let sub = strict_list(interp, &new_list[idx])?;
        let new_sub = lremove_flat(interp, &sub, &indices[1..])?;
        new_list[idx] = new_sub;
    }
    Ok(Value::from_list(&new_list))
}

/// lremove list ?index ...? — remove elements by index, simultaneously
/// (tclsh 9.1; tclCmdIL.c Tcl_LremoveObjCmd: indices sorted descending,
/// duplicates and out-of-range values skipped).
pub fn cmd_lremove(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("lremove", "list ?index ...?"));
    }
    let list = strict_list(interp, &args[1])?;
    if args.len() == 2 {
        return Ok(args[1].clone());
    }
    let len = list.len();
    let mut idxs: Vec<i64> = Vec::with_capacity(args.len() - 2);
    for a in &args[2..] {
        let s = a.as_str();
        match parse_tcl_index(s, len) {
            Some(r) => idxs.push(r),
            None => return Err(bad_index(interp, s)),
        }
    }
    idxs.sort_unstable_by(|a, b| b.cmp(a));
    idxs.dedup();
    let mut result = list.clone();
    for &idx in &idxs {
        if idx < 0 || idx >= len as i64 {
            continue;
        }
        result.remove(idx as usize);
    }
    Ok(Value::from_list(&result))
}

/// ledit listVar first last ?element ...? — replace a range in a list
/// variable (tclsh 9.1; tclCmdIL.c Tcl_LeditObjCmd: first clamped into
/// [0, len], last clamped to < len, last < first is a pure insert).
pub fn cmd_ledit(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(wrong_args("ledit", "listVar first last ?element ...?"));
    }
    let var_name = args[1].as_str().to_string();
    let current = interp.get_var(&var_name)?.clone();
    let list = strict_list(interp, &current)?;
    let len = list.len();
    let first_raw = match parse_tcl_index(args[2].as_str(), len) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[2].as_str())),
    };
    let last_raw = match parse_tcl_index(args[3].as_str(), len) {
        Some(r) => r,
        None => return Err(bad_index(interp, args[3].as_str())),
    };
    let first = first_raw.clamp(0, len as i64) as usize;
    let last = last_raw.min(len as i64 - 1);

    let mut result = Vec::with_capacity(len + args.len() - 4);
    result.extend(list[..first].iter().cloned());
    result.extend(args[4..].iter().cloned());
    if last >= first as i64 {
        result.extend(list[(last + 1) as usize..].iter().cloned());
    } else {
        result.extend(list[first..].iter().cloned());
    }
    let result = Value::from_list(&result);
    interp.set_var(&var_name, result.clone())?;
    Ok(result)
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
        // 9.1 重钉（digits91 批，oracle 实测）：0NNN 是十进制，`008`/
        // `08`/`+008` 都是合法索引 8；真正残缺的前缀（`0x`、`0b2`）与
        // 非数（`1.5`）仍解析不出。bad_index() 的 octal hint 已随 9.1
        // 取消（TclCheckBadOctal 移除）。
        assert_eq!(idx("008", 8), Some(8));
        assert_eq!(idx("08", 8), Some(8));
        assert_eq!(idx("+008", 8), Some(8));
        assert_eq!(idx("009", 8), Some(9));
        assert_eq!(idx("0d1", 8), Some(1));
        assert_eq!(idx("1_0", 8), Some(10));
        assert_eq!(idx("0x", 8), None);
        assert_eq!(idx("0b2", 8), None);
        assert_eq!(idx("0o8", 8), None);
        assert_eq!(idx("1.5", 8), None);
    }

    #[test]
    fn index_91_wide_and_bignum() {
        // 9.1 重钉（E1 批 util-9.x，oracle 实测）：项不再限 2^32-1、
        // 求和不再回绕 C int——任意大小整数按符号饱和到 ±wide，越界
        // 由调用方钳制（string index → 空串），不再报 bad index。
        assert_eq!(idx("4294967295", 8), Some(4294967295)); // out of range
        assert_eq!(idx("4294967296", 8), Some(4294967296));
        assert_eq!(idx("-4294967295", 8), Some(-4294967295)); // no i32 wrap
        assert_eq!(idx("9223372036854775807", 8), Some(i64::MAX));
        assert_eq!(idx("9223372036854775808", 8), Some(i64::MAX)); // BIG→WIDE_MAX
        assert_eq!(idx("-9223372036854775808", 8), Some(i64::MIN));
        assert_eq!(idx("9223372036854775807+1", 8), Some(i64::MAX)); // saturate
        assert_eq!(idx("2147483647+1", 8), Some(2147483648)); // no wrap
        assert_eq!(idx("end+4294967295", 8), Some(4294967302)); // out of range
        assert_eq!(idx("4294967295-4294967295", 8), Some(0));
        assert_eq!(idx("0x10000000000000000", 8), Some(i64::MAX));
        assert_eq!(
            idx("99999999999999999999-99999999999999999999", 8),
            Some(0) // both BIG → exact bignum difference
        );
        assert_eq!(idx("0d100000000000+0", 8), Some(100000000000));
        assert_eq!(idx("end+2305843009213693950", 8), Some(2305843009213693957));
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

    #[test]
    fn test_lseq_huge_is_lazy() {
        let mut interp = Interp::new();
        assert_eq!(
            interp.eval("llength [lseq 0x7fffffff]").unwrap().as_str(),
            "2147483647"
        );
        assert_eq!(
            interp.eval("lindex [lseq 0x7fffffff] 0x80000000").unwrap().as_str(),
            ""
        );
        assert_eq!(
            interp.eval("lindex [lseq 0x7fffffff] 5").unwrap().as_str(),
            "5"
        );
        assert_eq!(
            interp.eval("lrange [lseq 0x7fffffff] 3 5").unwrap().as_str(),
            "3 4 5"
        );
        assert_eq!(
            interp.eval("lrange [lseq 0x7fffffff] end-1 end").unwrap().as_str(),
            "2147483645 2147483646"
        );
        assert_eq!(
            interp.eval("lmap x [lseq 0x7fffffff] {if {$x > 2} break; set x}").unwrap().as_str(),
            "0 1 2"
        );
        assert_eq!(
            interp.eval("lfilter x [lseq 0x7fffffff] {if {$x > 2} break; expr 1}").unwrap().as_str(),
            "0 1 2"
        );
        assert_eq!(
            interp.eval("foreach x [lseq 0x7fffffff] {if {$x > 2} break; set x}; set x").unwrap().as_str(),
            "3"
        );
    }
}
