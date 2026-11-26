//! Arbitrary-precision integer support (Tcl "bignum") for expressions.
//!
//! tclsh widens integer arithmetic past 64 bits into exact bignums
//! (tclObj.c / libtommath).  rtcl keeps `Value` strings as the carrier —
//! a bignum value is simply a Value whose string is the canonical decimal
//! — while this module supplies parsing, arithmetic and conversions on
//! `num_bigint::BigInt`.
//!
//! Semantics probed against tclsh 8.6.17 and re-probed against 9.1.0
//! (judge/probes/pbn*.tcl, digits91.tcl):
//! - integer literals (dec / 0x / 0o / 0b / 0d; 9.1 grammar — the bare
//!   leading-0 octal of 8.6 is gone) widen silently
//! - `int()`/`wide()` wrap to the low 64 bits of the exact integer
//! - `entier()`/`round()` produce bignums for out-of-range doubles
//! - `/` and `%` are floor division / modulo
//! - `abs()` returns its argument *unchanged* when already non-negative

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};

use crate::value::Value;

/// An exact integer: the i64 fast path or a bignum.
#[derive(Debug, Clone)]
pub enum IntRep {
    I64(i64),
    Big(BigInt),
}

impl IntRep {
    /// Widen to `BigInt` either way.
    pub fn to_big(&self) -> BigInt {
        match self {
            IntRep::I64(i) => BigInt::from(*i),
            IntRep::Big(b) => b.clone(),
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            IntRep::I64(i) => *i == 0,
            IntRep::Big(b) => b.is_zero(),
        }
    }
}

/// Exact integer view of a Value (no float coercion).
pub fn int_rep(v: &Value) -> Option<IntRep> {
    if let Some(i) = v.as_int() {
        return Some(IntRep::I64(i));
    }
    // D5：Big rep 直接读箱——运算链上的 bignum 不再逐次物化字符串再
    // 十进制重解析（深拷一份位数给本函数的所有权形态 IntRep::Big）。
    if let Some(b) = v.as_big() {
        return Some(IntRep::Big(b.clone()));
    }
    // `as_int` already took every i64-range literal, so this is a bignum.
    match parse_int(v.as_str().trim())? {
        IntRep::Big(b) => Some(IntRep::Big(b)),
        IntRep::I64(i) => Some(IntRep::I64(i)),
    }
}

/// The Tcl integer-literal grammar — the single implementation, shared by
/// `Value::as_int`, [`int_rep`] and the expression result canonicalization
/// (they used to carry four private copies that disagreed about legacy
/// octal: one read `0777…` past 64 bits as decimal, another `018` as 18).
///
/// 9.1 grammar (TIP … / tclStrToD.c TclParseNumber, oracle-probed
/// 2026-11-26, judge/probes/digits91.tcl): optional sign; decimal;
/// `0x`/`0X`, `0o`/`0O`, `0b`/`0B` prefixes; `0d`/`0D` to compel decimal;
/// a bare leading `0` is DECIMAL (`017` is 17 — legacy octal abolished,
/// `09` is 9, `018` is 18, no invalid-octal error anywhere); `_`
/// separators between digits of any radix (`1_000`, `0_1`, `1__0`,
/// `0xf_f`) but never at either end or adjacent to a radix prefix
/// (`1_`, `100_`, `0x_f` are not numbers).
pub fn parse_int(s: &str) -> Option<IntRep> {
    let scan = scan_number(s, true)?;
    if !scan.full {
        return None;
    }
    let (radix, run) = scan.into_int()?;
    // Accumulate in u64 (covers every i64 magnitude plus the -2^63
    // boundary) with zero allocation; only past-u64 widths join the
    // digit region for a bignum.
    let mut mag: u64 = 0;
    let mut overflow = false;
    for &b in s[run..scan.stop].as_bytes() {
        if b == b'_' {
            continue;
        }
        mag = match mag
            .checked_mul(radix as u64)
            .and_then(|m| m.checked_add(digit_value(b) as u64))
        {
            Some(v) => v,
            None => {
                overflow = true;
                break;
            }
        };
    }
    if !overflow {
        if mag <= i64::MAX as u64 {
            return Some(IntRep::I64(if scan.neg { -(mag as i64) } else { mag as i64 }));
        }
        // `-9223372036854775808`: the magnitude is one past i64::MAX
        // while the signed value fits.
        if scan.neg && mag == (i64::MAX as u64) + 1 {
            return Some(IntRep::I64(i64::MIN));
        }
    }
    let digits = strip_separators(&s[run..scan.stop]);
    let big = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(IntRep::Big(if scan.neg { -big } else { big }))
}

/// Strip `_` separators from a validated digit region.
fn strip_separators(digits: &str) -> String {
    digits.bytes().filter(|&b| b != b'_').map(|b| b as char).collect()
}

/// Numeric value of a validated digit byte in the scan's radix (the
/// scanner only admits digits of its radix, so `a-f` in radix 16 — the
/// only letter case — is safe).
pub fn digit_value(b: u8) -> u32 {
    match b {
        b'0'..=b'9' => (b - b'0') as u32,
        b'a'..=b'f' => (b - b'a' + 10) as u32,
        b'A'..=b'F' => (b - b'A' + 10) as u32,
        _ => 0,
    }
}

/// Integer-region magnitude as f64 (prefixed radixes convert exactly via
/// u64, widening to a bignum conversion past that).
fn int_span_to_f64(s: &str, neg: bool, radix: u32, run: usize, stop: usize) -> Option<f64> {
    let mut mag: u64 = 0;
    let mut overflow = false;
    for &b in s[run..stop].as_bytes() {
        if b == b'_' {
            continue;
        }
        mag = match mag
            .checked_mul(radix as u64)
            .and_then(|m| m.checked_add(digit_value(b) as u64))
        {
            Some(v) => v,
            None => {
                overflow = true;
                break;
            }
        };
    }
    let v = if overflow {
        let digits = strip_separators(&s[run..stop]);
        BigInt::parse_bytes(digits.as_bytes(), radix)?.to_f64()?
    } else {
        mag as f64
    };
    Some(if neg { -v } else { v })
}

/// The full-number float view of a string (9.1 grammar — underscores in
/// mantissa/fraction/exponent, `0d` prefixes, decimal leading zeros;
/// `1_0.5` is 10.5). `Inf`/`-Inf`/`NaN` are the caller's arm here.
pub fn parse_float_str(s: &str) -> Option<f64> {
    let scan = scan_number(s, false)?;
    if !scan.full {
        return None;
    }
    match &scan.payload {
        NumPayload::Float { .. } => {
            let text = float_text_from(s, scan.stop, &scan.payload, scan.neg)?;
            text.parse().ok()
        }
        // Integer forms coerce to floats: `"0x10" + 0.5` is 16.5.
        NumPayload::Int { radix, run } => {
            int_span_to_f64(s, scan.neg, *radix, *run, scan.stop)
        }
    }
}

/// The 9.1 Tcl number grammar scanner — the acceptPoint mirror of
/// tclStrToD.c `TclParseNumber` (without the scan-command legacy-octal
/// flag, which rtcl's `scan` keeps separately in io.rs). Shared by
/// `parse_int`/`parse_float_str`, `string is integer/wideinteger/entier/
/// double` (failindex = stop), the index scanners (`scan_tcl_int`) and
/// the expression lexer's bareword/hint decisions.
///
/// Returns `None` when no numeric prefix exists at all (TclParseNumber's
/// TCL_ERROR arm: the first character never reached an accepting state —
/// `_1`, `abc`, `1_x` where the misplaced `_` kills the whole token).
///
/// `stop` is the acceptPoint: the input length for a full parse; for a
/// prefix parse the index of the first rejected character when its
/// lexing state was accepting (`42x` stops at the `x`), or of the last
/// accepted character when it was not (`12_` stops at the `2`, `1_x`
/// fails outright). Characters in `s[..stop]` are all ASCII, so byte and
/// char offsets agree. `full` additionally accepts trailing whitespace.
pub struct NumScan {
    pub stop: usize,
    pub full: bool,
    /// A leading `-` was consumed (always in the accepted region).
    pub neg: bool,
    pub payload: NumPayload,
}

/// Number pieces as spans of the scanned input — the scanner never
/// allocates; value builders join the digit runs only when a value is
/// actually materialised (separators are skipped while accumulating).
pub enum NumPayload {
    /// Integer: radix + the digit region `s[run..stop)` (digits and `_`
    /// separators, contiguous — prefixes excluded; sign is `scan.neg`).
    Int { radix: u32, run: usize },
    /// Float piece positions (byte indices): mantissa digits `s[f0..dot
    /// or e or stop)`, fraction `s[dot+1..e or stop)`, exponent digits
    /// `s[exp0..stop)` (sign in `exp_neg`); `f0 == dot` marks an empty
    /// mantissa (".5").
    Float {
        f0: usize,
        dot: Option<usize>,
        e_at: Option<usize>,
        exp0: Option<usize>,
        exp_neg: bool,
    },
}

impl NumScan {
    /// The integer parts of the payload, or `None` for a float:
    /// `(radix, digit-run start)`.
    pub fn into_int(&self) -> Option<(u32, usize)> {
        match self.payload {
            NumPayload::Int { radix, run } => Some((radix, run)),
            NumPayload::Float { .. } => None,
        }
    }

    /// The integer digit region `s[run..stop)` — digits and `_`
    /// separators, contiguous. Empty for a prefix without digits (`0x`).
    pub fn int_digits<'a>(&self, s: &'a str) -> &'a str {
        let run = match &self.payload {
            NumPayload::Int { run, .. } => *run,
            _ => self.stop,
        };
        &s[run..self.stop]
    }
}

/// The float's Rust-parseable text (separators stripped; mantissa "0"
/// supplied where absent; dot-exponent junctions normalized — Rust
/// rejects "3.e2", tclsh accepts it as 300.0).
pub fn float_text_from(s: &str, stop: usize, p: &NumPayload, neg: bool) -> Option<String> {
    let NumPayload::Float { f0, dot, e_at, exp0, exp_neg } = p else {
        return None;
    };
    let e_end = e_at.unwrap_or(stop);
    let mut text = String::with_capacity(stop + 8);
    if neg {
        text.push('-');
    }
    // Mantissa: empty only for a leading-dot float (".5") — f0 == dot.
    if matches!(dot, Some(d) if *f0 == *d) {
        text.push('0');
    } else {
        text.push_str(&s[*f0..dot.unwrap_or(e_end)].replace('_', ""));
    }
    if let Some(d) = dot {
        text.push('.');
        let frac = &s[d + 1..e_end];
        if frac.is_empty() && e_at.is_some() {
            // "3.e2" → "3.0e2" (a trailing dot alone parses in Rust).
            text.push('0');
        } else {
            text.push_str(&frac.replace('_', ""));
        }
    }
    if let (Some(_), Some(x0)) = (e_at, exp0) {
        text.push('e');
        if *exp_neg {
            text.push('-');
        }
        text.push_str(&s[*x0..stop].replace('_', ""));
    }
    Some(text)
}

/// Tcl's whitespace set (`TclIsSpaceProc`).
fn is_tcl_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// Scan a Tcl number per the 9.1 grammar. `integer_only` rejects `.`/`e`
/// (string is integer's TCL_PARSE_INTEGER_ONLY); the flag is also latched
/// after a `0d` prefix, whose forms are integers only.
///
/// State machine mirroring tclStrToD.c: accepting states (Zero after the
/// leading 0, and each radix body, decimal digit, fraction and exponent
/// state) record an accept point before rejecting a character; prefix
/// states (the radix specifier itself, exponent sign states) do not. The
/// `_` separator filter runs before the state machine and kills the whole
/// token when misplaced (no numeric prefix). End-of-input fails the same
/// way as any rejected character, so a full parse stops exactly at the
/// input length.
pub fn scan_number(s: &str, integer_only: bool) -> Option<NumScan> {
    #[derive(Clone, Copy, PartialEq)]
    enum St {
        /// INITIAL: optional sign, then the number proper.
        Initial,
        /// SIGNUM: a sign was seen.
        Signum,
        Zero,
        Hex0,
        Hex,
        Oct0,
        Oct,
        Bin0,
        Bin,
        Dec0,
        Dec,
        Frac,
        Lrp,
        ExpStart,
        ExpSign,
        Exp,
    }
    use St::*;

    let b = s.as_bytes();
    let mut i = 0;
    // Leading whitespace is consumed without ever setting an accept
    // point (a whitespace-only string scans to None).
    while i < b.len() && is_tcl_ws(b[i]) {
        i += 1;
    }
    let mut int_only = integer_only;
    let mut state = Initial;
    // Accept point: the index of the last character dispatched through an
    // accepting state (None = nothing accepted yet). One past the last
    // digit of the number when the parse is complete.
    let mut accept: Option<usize> = None;
    let mut neg = false;
    // Digit regions as positions (zero-allocation payload): the integer
    // run start, the float piece positions. `d0` follows the current
    // digit cursor — prefixes reset it, digits only ever extend to the
    // cursor, so each region is contiguous to `stop`.
    let mut d0: Option<usize> = None;
    let mut f0: Option<usize> = None;
    let mut dot: Option<usize> = None;
    let mut e_at: Option<usize> = None;
    let mut exp0: Option<usize> = None;
    let mut exp_neg = false;
    // Position of the first float-making character ('.'/'e') consumed by
    // the scanner. A rejection rewinds to the accept point, and the form
    // is float only when the accepted region reaches past this mark
    // (`3e` accepts just the int `3`; `3e2x` accepts the double `3e2`).
    let mut float_at: Option<usize> = None;
    let mut radix = 10u32;

    // Valid `_` neighbours for the current state family (tclStrToD.c's
    // switch: binary/octal/hex/decimal each pin their own digit set;
    // decimal separators may additionally not hug a `.`).
    fn sep_ok(state: St, before: Option<u8>, after: Option<u8>) -> bool {
        fn digit(c: u8) -> bool {
            c.is_ascii_digit()
        }
        fn xdigit(c: u8) -> bool {
            c.is_ascii_digit() || matches!(c, b'a'..=b'f' | b'A'..=b'F')
        }
        match state {
            Bin0 | Bin => {
                before.map(|c| c == b'0' || c == b'1').unwrap_or(false)
                    && after.map(|c| c == b'0' || c == b'1').unwrap_or(false)
            }
            Oct0 | Oct => {
                before.map(|c| (b'0'..=b'7').contains(&c)).unwrap_or(false)
                    && after.map(|c| (b'0'..=b'7').contains(&c)).unwrap_or(false)
            }
            Hex0 | Hex => {
                before.map(xdigit).unwrap_or(false) && after.map(xdigit).unwrap_or(false)
            }
            _ => before.map(digit).unwrap_or(false) && after.map(digit).unwrap_or(false),
        }
    }
    // Nearest non-separator characters either side of a `_`.
    fn nearest(mut p: usize, b: &[u8], forward: bool) -> Option<u8> {
        loop {
            if forward {
                if p >= b.len() {
                    return None;
                }
                let c = b[p];
                if c != b'_' {
                    return Some(c);
                }
                p += 1;
            } else {
                if p == 0 {
                    return None;
                }
                p -= 1;
                let c = b[p];
                if c != b'_' {
                    return Some(c);
                }
            }
        }
    }

    // Reject the character at `i` (or end-of-input at `b.len()`).
    macro_rules! reject {
        () => {{
            // The NUL dispatch of end-of-input sets the accept point in
            // accepting states, exactly like a rejected character.
            if i >= b.len() && matches!(state, Zero | Hex | Oct | Bin | Dec | Frac | Exp) {
                accept = Some(b.len());
            }
            let stop = accept?;
            let is_float = matches!(float_at, Some(p) if accept.is_some_and(|a| a > p));
            let full = b[stop..].iter().all(|&c| is_tcl_ws(c));
            // A prefix rewinds past its digits (`0x` → stop 1, digit run
            // from 2) — clamp the spans to the accepted region.
            let payload = if is_float {
                NumPayload::Float {
                    f0: f0.unwrap_or(stop).min(stop),
                    dot: dot.map(|d| d.min(stop)),
                    e_at: e_at.map(|e| e.min(stop)),
                    exp0: exp0.map(|e| e.min(stop)),
                    exp_neg,
                }
            } else {
                NumPayload::Int {
                    radix,
                    run: d0.unwrap_or(stop).min(stop),
                }
            };
            return Some(NumScan {
                stop,
                full,
                neg,
                payload,
            });
        }};
    }

    while i < b.len() {
        let c = b[i];
        // `_` numeric whitespace: validated against the state's digit
        // family before the state machine sees it (a valid separator is
        // consumed without touching the state).
        if c == b'_' {
            let before = nearest(i, b, false);
            let after = nearest(i + 1, b, true).filter(|&c| c != 0 && c.is_ascii());
            if !sep_ok(state, before, after) {
                reject!();
            }
            i += 1;
            continue;
        }
        match state {
            Initial => match c {
                b'+' => state = Signum,
                b'-' => {
                    neg = true;
                    state = Signum;
                }
                b'0' => {
                    accept = Some(i);
                    d0 = Some(i);
                    f0 = Some(i);
                    state = Zero;
                }
                b'.' => {
                    // LEADING_RADIX_POINT never accepts; the fraction
                    // digits that may follow do.
                    float_at = Some(i);
                    dot = Some(i);
                    f0 = Some(i); // empty mantissa: f0 == dot
                    state = Lrp;
                }
                c if c.is_ascii_digit() => {
                    accept = Some(i);
                    d0 = Some(i);
                    f0 = Some(i);
                    state = Dec;
                }
                _ => reject!(),
            },
            Signum => match c {
                b'0' => {
                    accept = Some(i);
                    d0 = Some(i);
                    f0 = Some(i);
                    state = Zero;
                }
                b'.' if !int_only => {
                    float_at = Some(i);
                    dot = Some(i);
                    f0 = Some(i);
                    state = Lrp;
                }
                c if c.is_ascii_digit() => {
                    accept = Some(i);
                    d0 = Some(i);
                    f0 = Some(i);
                    state = Dec;
                }
                _ => reject!(),
            },
            Zero => {
                accept = Some(i);
                match c {
                    b'x' | b'X' => {
                        radix = 16;
                        d0 = Some(i + 1);
                        f0 = Some(i + 1);
                        state = Hex0;
                    }
                    b'b' | b'B' => {
                        radix = 2;
                        d0 = Some(i + 1);
                        f0 = Some(i + 1);
                        state = Bin0;
                    }
                    b'o' | b'O' => {
                        radix = 8;
                        d0 = Some(i + 1);
                        f0 = Some(i + 1);
                        state = Oct0;
                    }
                    b'd' | b'D' => {
                        radix = 10;
                        int_only = true;
                        d0 = Some(i + 1);
                        f0 = Some(i + 1);
                        // ZERO_D: the prefix character itself was never
                        // accepted (`0d` alone stops at index 1); the
                        // first digit falls straight through into DECIMAL
                        // (the Dec0 → Dec arm below).
                        state = Dec0;
                    }
                    b'.' if !int_only => {
                        float_at = Some(i);
                        dot = Some(i);
                        state = Frac;
                    }
                    b'e' | b'E' if !int_only => {
                        float_at = Some(i);
                        e_at = Some(i);
                        state = ExpStart;
                    }
                    c if c.is_ascii_digit() => {
                        state = Dec;
                    }
                    _ => reject!(),
                }
            }
            Hex0 | Hex => {
                if state == Hex {
                    accept = Some(i);
                }
                match c {
                    c if c.is_ascii_digit() || matches!(c, b'a'..=b'f' | b'A'..=b'F') => {
                        state = Hex;
                    }
                    _ => reject!(),
                }
            }
            Oct0 | Oct => {
                if state == Oct {
                    accept = Some(i);
                }
                match c {
                    c if (b'0'..=b'7').contains(&c) => {
                        state = Oct;
                    }
                    _ => reject!(),
                }
            }
            Bin0 | Bin => {
                if state == Bin {
                    accept = Some(i);
                }
                match c {
                    b'0' | b'1' => {
                        state = Bin;
                    }
                    _ => reject!(),
                }
            }
            Dec0 | Dec => {
                // Dec0 (the `0d` digit) accepts: ZERO_D falls through to
                // DECIMAL, which sets the accept point.
                if state == Dec {
                    accept = Some(i);
                } else {
                    state = Dec;
                }
                match c {
                    c if c.is_ascii_digit() => {}
                    b'.' if !int_only => {
                        float_at = Some(i);
                        dot = Some(i);
                        state = Frac;
                    }
                    b'e' | b'E' if !int_only => {
                        float_at = Some(i);
                        e_at = Some(i);
                        state = ExpStart;
                    }
                    _ => reject!(),
                }
            }
            Frac => {
                accept = Some(i);
                match c {
                    c if c.is_ascii_digit() => {}
                    b'e' | b'E' => {
                        float_at = Some(i);
                        e_at = Some(i);
                        state = ExpStart;
                    }
                    _ => reject!(),
                }
            }
            Lrp => match c {
                // LEADING_RADIX_POINT never sets an accept point: ".5_"
                // fails as a whole, not as the prefix ".5".
                c if c.is_ascii_digit() => {
                    state = Frac;
                }
                _ => reject!(),
            },
            ExpStart => match c {
                b'+' | b'-' => {
                    exp_neg = c == b'-';
                    state = ExpSign;
                }
                // EXPONENT_START/EXPONENT_SIGNUM set no accept point:
                // "1e2x" fails at the 'e' (index 1), not at the 'x'.
                c if c.is_ascii_digit() => {
                    exp0 = Some(i);
                    state = Exp;
                }
                _ => reject!(),
            },
            ExpSign => match c {
                c if c.is_ascii_digit() => {
                    exp0 = Some(i);
                    state = Exp;
                }
                _ => reject!(),
            },
            Exp => {
                accept = Some(i);
                match c {
                    c if c.is_ascii_digit() => {}
                    _ => reject!(),
                }
            }
        }
        i += 1;
    }
    // End-of-input: the NUL dispatch fails like any rejected character.
    if matches!(state, Zero | Hex | Oct | Bin | Dec | Frac | Exp) {
        accept = Some(b.len());
    }
    let stop = accept?;
    let is_float = matches!(float_at, Some(p) if accept.is_some_and(|a| a > p));
    let full = b[stop..].iter().all(|&c| is_tcl_ws(c));
    let payload = if is_float {
        NumPayload::Float {
            f0: f0.unwrap_or(stop).min(stop),
            dot: dot.map(|d| d.min(stop)),
            e_at: e_at.map(|e| e.min(stop)),
            exp0: exp0.map(|e| e.min(stop)),
            exp_neg,
        }
    } else {
        NumPayload::Int {
            radix,
            run: d0.unwrap_or(stop).min(stop),
        }
    };
    Some(NumScan {
        stop,
        full,
        neg,
        payload,
    })
}

/// tclCompExpr.c's radix hint on an invalid bareword: a token led by `0`
/// whose number scan stops at a digit or right after the leading zero is
/// tagged by its second character — `0b` → binary, `0o` (or a bare
/// `0`-digit attempt like `08x` where the scan stops at the offending
/// digit) → octal; `0d`/`0x` and everything else carry no hint. Oracle
/// pins (digits91): `0b12`/`0b`/`0o8`/`0o_1` are hinted, `0b1x`/`0x_f`/
/// `0d_1`/`08g`/`1_` are not.
pub fn bareword_hint(token: &str) -> &'static str {
    let b = token.as_bytes();
    if b.first() != Some(&b'0') || b.len() < 2 {
        return "";
    }
    let Some(scan) = scan_number(token, false) else {
        return "";
    };
    if scan.stop >= token.len() || scan.stop == 0 {
        return "";
    }
    if !b[scan.stop].is_ascii_digit() && scan.stop != 1 {
        return "";
    }
    match b[1] {
        b'b' | b'B' => " (invalid binary number?)",
        b'o' | b'O' => " (invalid octal number?)",
        c if c.is_ascii_digit() => " (invalid octal number?)",
        _ => "",
    }
}

/// Parse a Tcl integer literal at arbitrary width (bignum form; an
/// i64-range literal is widened to its `BigInt`).
pub fn parse_literal(s: &str) -> Option<BigInt> {
    Some(match parse_int(s)? {
        IntRep::I64(i) => BigInt::from(i),
        IntRep::Big(b) => b,
    })
}

/// Build a Value from an exact integer (D5：bignum 落 `Big(Rc<BigInt>)`
/// 堆箱——此前是字符串箱，每次运算链都要物化+重解析）。
pub fn to_value(r: IntRep) -> Value {
    match r {
        IntRep::I64(i) => Value::from_int(i),
        IntRep::Big(b) => Value::from_bigint(b),
    }
}

/// Exact integral value of a finite double (truncation already applied).
/// Out-of-i64-range doubles convert precisely — `entier(1e22)` is
/// `10000000000000000000000`, not a rounded decimal.
pub fn big_from_f64(t: f64) -> Option<BigInt> {
    if !t.is_finite() {
        return None;
    }
    let s = format!("{:.0}", t);
    BigInt::parse_bytes(s.as_bytes(), 10)
}

/// Low 64 bits of a bignum, two's complement (`int()`/`wide()` wrapping:
/// `wide(-9223372036854775809)` is `9223372036854775807`).
pub fn to_i64_wrap(b: &BigInt) -> i64 {
    let m = BigInt::from(1u8) << 64u32;
    let low = b.mod_floor(&m);
    match low.to_u64() {
        Some(u) => u as i64,
        None => 0,
    }
}

/// Nearest-double conversion of a bignum (rounds to even like tclsh's
/// bignum→double path).
pub fn to_f64(b: &BigInt) -> f64 {
    b.to_f64().unwrap_or_else(|| {
        if b.is_negative() {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    })
}

/// Floor division on bignums (`/` keeps Tcl's floor semantics for
/// negative operands).
pub fn floor_div(a: &BigInt, b: &BigInt) -> BigInt {
    a.div_floor(b)
}

/// Floor modulo on bignums.
pub fn floor_mod(a: &BigInt, b: &BigInt) -> BigInt {
    a.mod_floor(b)
}

/// Exact integer square root (Newton, converges from above).
pub fn isqrt(x: &BigInt) -> BigInt {
    if x.is_zero() {
        return BigInt::from(0u8);
    }
    // Initial guess: 2^ceil(bits/2) — within a factor of 2 of √x.
    let bits = x.bits();
    let mut g: BigInt = BigInt::from(1u8) << (bits.div_ceil(2) + 1) as usize;
    loop {
        let next = (g.clone() + x.div_floor(&g)) >> 1u32;
        if next >= g {
            break;
        }
        g = next;
    }
    // Final exactness correction (guess is ≥ isqrt, off by at most 2).
    let mut r = g;
    while &r * &r > *x {
        r -= BigInt::from(1u8);
    }
    while (&r + 1u8) * (&r + 1u8) <= *x {
        r += BigInt::from(1u8);
    }
    r
}

/// Nearest-double square root of a bignum.  tclsh takes the sqrt of the
/// *true* value and converts only afterwards — `sqrt(10**616)` is
/// `1e+308` and `sqrt(10**617)` is `Inf`, even though `double(10**616)`
/// itself is already `Inf` — so saturating the operand first would give
/// wrong results.  Truncating to the top 64 bits loses relative precision
/// below 2^-63; the sqrt halves that, far under the 2^-53 rounding
/// granularity, so the scaled result rounds like the exact one.
pub fn sqrt_to_f64(b: &BigInt) -> f64 {
    use num_traits::ToPrimitive;
    if b.is_zero() {
        return 0.0;
    }
    let bits = b.bits();
    let shift = bits.saturating_sub(64);
    let top = (b >> (shift as usize)).to_u64().unwrap_or(u64::MAX) as f64;
    // Fold one factor of 2 into the mantissa when the exponent is odd so
    // the scale stays a clean power of 4.
    let (mant, exp) = if shift.is_multiple_of(2) {
        (top, shift / 2)
    } else {
        (top * 2.0, (shift - 1) / 2)
    };
    mant.sqrt() * 2f64.powi(exp as i32)
}

/// Exact comparison of an integer rep against a double.
pub fn cmp_rep_float(a: &IntRep, f: f64) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering;
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let t = f.trunc();
    let b = big_from_f64(t)?;
    match a.to_big().cmp(&b) {
        Ordering::Equal => {
            if f > t {
                Some(Ordering::Less)
            } else {
                Some(Ordering::Equal)
            }
        }
        ord => Some(ord),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_literal_forms() {
        assert_eq!(parse_literal("18446744073709551616").unwrap().to_string(), "18446744073709551616");
        assert_eq!(parse_literal("-9223372036854775809").unwrap().to_string(), "-9223372036854775809");
        assert_eq!(parse_literal("0x10000000000000000").unwrap().to_string(), "18446744073709551616");
        assert_eq!(parse_literal("0b1111111111111111111111111111111111111111111111111111111111111111").unwrap().to_string(), "18446744073709551615");
        assert!(parse_literal("abc").is_none());
    }

    #[test]
    fn i64_wrap_matches_tclsh() {
        // wide(-9223372036854775809) == 9223372036854775807 (probed)
        let b = parse_literal("-9223372036854775809").unwrap();
        assert_eq!(to_i64_wrap(&b), i64::MAX);
        // int(1e22) == 1864712049423024128 (probed)
        let b = big_from_f64(1e22).unwrap();
        assert_eq!(b.to_string(), "10000000000000000000000");
        assert_eq!(to_i64_wrap(&b), 1864712049423024128);
    }

    #[test]
    fn isqrt_exact() {
        // isqrt(123456789012345678901234567890) == 351364182882014 (probed)
        assert_eq!(
            isqrt(&parse_literal("123456789012345678901234567890").unwrap()).to_string(),
            "351364182882014"
        );
        assert_eq!(isqrt(&BigInt::from(0u8)).to_string(), "0");
        assert_eq!(isqrt(&BigInt::from(4294967296u64)).to_string(), "65536");
    }
}
