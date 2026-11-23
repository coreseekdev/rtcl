//! Arbitrary-precision integer support (Tcl "bignum") for expressions.
//!
//! tclsh widens integer arithmetic past 64 bits into exact bignums
//! (tclObj.c / libtommath).  rtcl keeps `Value` strings as the carrier —
//! a bignum value is simply a Value whose string is the canonical decimal
//! — while this module supplies parsing, arithmetic and conversions on
//! `num_bigint::BigInt`.
//!
//! Semantics probed against tclsh 8.6.17 (see judge/probes/pbn*.tcl):
//! - integer literals (dec / 0x / 0o / 0b, legacy octal) widen silently
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
/// Optional sign; decimal; `0x`/`0X`, `0o`/`0O`, `0b`/`0B` prefixes; a
/// leading `0` followed by more octal digits is legacy octal (`017` is
/// 15), at any width. A leading `0` followed by a digit 8 or 9 is not a
/// number at all (see [`is_invalid_octal`]).
pub fn parse_int(s: &str) -> Option<IntRep> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if body.is_empty() {
        return None;
    }
    // The prefixes are ASCII, so dispatch on leading bytes: slicing the
    // `&str` at a fixed byte offset panics when the offset falls inside a
    // multi-byte character (`expr {"中a" + 1}` reached the old copy with
    // any non-ASCII operand).
    let b = body.as_bytes();
    let (radix, digits) = match b {
        [b'0', b'x' | b'X', rest @ ..] => (16u32, rest),
        [b'0', b'o' | b'O', rest @ ..] => (8, rest),
        [b'0', b'b' | b'B', rest @ ..] => (2, rest),
        // Legacy octal: `017` is 15, at any width (a bare `0` is decimal).
        [b'0', rest @ ..] if !rest.is_empty() && rest.iter().all(|d| (b'0'..=b'7').contains(d)) => (8, rest),
        // `018` is an invalid octal number, never the decimal 18.
        [b'0', rest @ ..] if !rest.is_empty() && rest.iter().all(|d| d.is_ascii_digit()) => return None,
        _ => (10, b),
    };
    // Only an all-digit remainder can be a number at this point; the
    // slice is ASCII then, so the view back to `&str` is unchecked-free.
    let digits = core::str::from_utf8(digits).ok()?;
    if digits.is_empty() || !digits.bytes().all(|d| (d as char).is_digit(radix)) {
        return None;
    }
    // i64 first; the magnitude is non-negative so the signed form cannot
    // overflow. Past i64 the same digits widen to a bignum.
    if let Ok(mag) = i64::from_str_radix(digits, radix) {
        return Some(IntRep::I64(if neg { -mag } else { mag }));
    }
    // `-9223372036854775808`: the magnitude is one past i64::MAX while the
    // signed value fits (only this boundary takes the allocation).
    if neg {
        if let Ok(v) = format!("-{digits}").parse::<i64>() {
            return Some(IntRep::I64(v));
        }
    }
    let mag = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(IntRep::Big(if neg { -mag } else { mag }))
}

/// A leading `0` followed by more decimal digits among which is an 8 or a
/// 9: Tcl treats the text as an *invalid octal number* everywhere, never
/// as the decimal it looks like (`expr {"018" + 0}` errors, `expr {"018"}`
/// stays the string "018").
pub fn is_invalid_octal(s: &str) -> bool {
    let body = s.strip_prefix('-').or_else(|| s.strip_prefix('+')).unwrap_or(s);
    let b = body.as_bytes();
    b.len() > 1
        && b[0] == b'0'
        && b.iter().all(|d| d.is_ascii_digit())
        && b.iter().any(|d| *d == b'8' || *d == b'9')
}

/// Parse a Tcl integer literal at arbitrary width (bignum form; an
/// i64-range literal is widened to its `BigInt`).
pub fn parse_literal(s: &str) -> Option<BigInt> {
    Some(match parse_int(s)? {
        IntRep::I64(i) => BigInt::from(i),
        IntRep::Big(b) => b,
    })
}

/// Build a Value from an exact integer (canonical decimal for bignums).
pub fn to_value(r: IntRep) -> Value {
    match r {
        IntRep::I64(i) => Value::from_int(i),
        IntRep::Big(b) => Value::from_str(&b.to_string()),
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
