//! Shared expr operator semantics — one implementation, two consumers.
//!
//! The recursive-descent expr parser ([`super::expr`]) and the bytecode
//! executor (`interp::vm_exec`, and later the JIT's runtime calls) must
//! apply identical semantics for every operator: error text, bignum
//! widening, NaN handling, string-comparison fallbacks.  These free
//! functions are that single source of truth; the parser's per-level
//! methods are thin callers.
//!
//! All functions are pure `(&Value, &Value) -> Result<Value>` — no
//! interpreter state, so the executor can call them per opcode.

use crate::error::{Error, Result};
use crate::value::Value;

use super::bignum::{self, IntRep};
use super::expr_funcs;

/// Convert `Value` to i64, returning an error if not numeric.
pub(crate) fn as_int_val(v: &Value) -> Result<i64> {
    v.as_int().or_else(|| v.as_float().map(|f| f as i64))
        .ok_or_else(|| Error::type_mismatch("integer", v.as_str()))
}

/// `+`, `-`, `*`, `/` — Tcl numeric semantics.
///
/// Integer operands use integer arithmetic (`/` is floor division,
/// result is int); on i64 overflow the operation widens to an exact
/// bignum (`9223372036854775807 + 1` is `9223372036854775808`,
/// tclsh-style).  If either side is float, compute in f64 and the
/// result stays a float.
pub(crate) fn numeric_binop(left: &Value, right: &Value, op: char) -> Result<Value> {
    use num_bigint::BigInt;
    use super::bignum::{floor_div as big_div, int_rep, to_value};
    if let (Some(ia), Some(ib)) = (int_rep(left), int_rep(right)) {
        if let (IntRep::I64(a), IntRep::I64(b)) = (&ia, &ib) {
            // Widen lazily: the bignum forms are converted only on an
            // actual i64 overflow — a non-overflowing op (the common case
            // by far) allocates nothing.  This used to convert both
            // operands eagerly: two BigInt allocations per `+`/`-`/`*`/`/`,
            // wasted whenever checked arithmetic succeeded.
            let widen = |x: BigInt| Ok(to_value(IntRep::Big(x)));
            return match op {
                '+' => match a.checked_add(*b) {
                    Some(r) => Ok(Value::from_int(r)),
                    None => widen(ia.to_big() + ib.to_big()),
                },
                '-' => match a.checked_sub(*b) {
                    Some(r) => Ok(Value::from_int(r)),
                    None => widen(ia.to_big() - ib.to_big()),
                },
                '*' => match a.checked_mul(*b) {
                    Some(r) => Ok(Value::from_int(r)),
                    None => widen(ia.to_big() * ib.to_big()),
                },
                '/' => {
                    if *b == 0 {
                        return Err(Error::DivisionByZero);
                    }
                    // i64::MIN / -1 overflows in Rust; Tcl widens to bignum
                    if *a == i64::MIN && *b == -1 {
                        return widen(big_div(&ia.to_big(), &ib.to_big()));
                    }
                    Ok(Value::from_int(super::expr::floor_div(*a, *b)))
                }
                _ => Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
            };
        }
        // At least one bignum operand: exact BigInt arithmetic.
        let (ba, bb) = (ia.to_big(), ib.to_big());
        return match op {
            '+' => Ok(to_value(IntRep::Big(ba + bb))),
            '-' => Ok(to_value(IntRep::Big(ba - bb))),
            '*' => Ok(to_value(IntRep::Big(ba * bb))),
            '/' => {
                if ib.is_zero() {
                    return Err(Error::DivisionByZero);
                }
                Ok(to_value(IntRep::Big(big_div(&ba, &bb))))
            }
            _ => Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
        };
    }
    // NaN operands can't take part in arithmetic (`"nan" + 0` →
    // can't use non-numeric floating-point value as operand of "+").
    // Checked only now, AFTER the both-integer fast path: an integer
    // never carries NaN, and probing as_float first re-parsed every
    // string operand as a float before the int parse could even run
    // (double parse per `+` on bracket results — several percent of the
    // fib profile).
    if left.as_float().map_or(false, |f| f.is_nan())
        || right.as_float().map_or(false, |f| f.is_nan())
    {
        return Err(Error::Msg(format!(
            "can't use non-numeric floating-point value as operand of \"{op}\""
        )));
    }
    match (left.as_float(), right.as_float()) {
        (Some(a), Some(b)) => {
            let result = match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => {
                    // tclsh follows IEEE here: x/0.0 is ±Inf (5/0.0 →
                    // Inf, 1.0/-0.0 → -Inf); only a zero dividend gives
                    // the NaN result tclsh refuses to build — 0/0.0 →
                    // "domain error: argument not in valid range".
                    if b == 0.0 && a == 0.0 {
                        return Err(expr_funcs::domain_error());
                    }
                    a / b
                }
                _ => return Err(Error::runtime("unknown op", crate::error::ErrorCode::InvalidOp)),
            };
            Ok(expr_funcs::float_value(result))
        }
        _ => Err(Error::Msg(format!(
            "can't use non-numeric string as operand of \"{op}\""
        ))),
    }
}

/// `%` with Tcl floor semantics; bignum operands stay exact.
pub(crate) fn int_mod(left: &Value, right: &Value) -> Result<Value> {
    use super::bignum::{floor_mod as big_mod, int_rep, to_value};
    if let (Some(ia), Some(ib)) = (int_rep(left), int_rep(right)) {
        if ib.is_zero() {
            return Err(Error::DivisionByZero);
        }
        if let (IntRep::I64(a), IntRep::I64(b)) = (&ia, &ib) {
            // i64::MIN % -1 would overflow in Rust; the result is 0
            if !(*a == i64::MIN && *b == -1) {
                return Ok(Value::from_int(super::expr::floor_mod(*a, *b)));
            }
            return Ok(Value::from_int(0));
        }
        return Ok(to_value(IntRep::Big(big_mod(&ia.to_big(), &ib.to_big()))));
    }
    // Non-int operands: tclsh distinguishes a float operand
    // (`1.5 % 2` → can't use floating-point value as operand of "%")
    // from any other non-numeric one (`true % 2` → can't use
    // non-numeric string as operand of "%").
    let a = int_operand(left, "%")?;
    let b = int_operand(right, "%")?;
    if b == 0 { return Err(Error::DivisionByZero); }
    Ok(Value::from_int(if a == i64::MIN && b == -1 {
        0
    } else {
        super::expr::floor_mod(a, b)
    }))
}

/// One `<<`/`>>` operation with Tcl's bignum-widening semantics.
pub(crate) fn int_shift(left: &Value, right: &Value, shl: bool) -> Result<Value> {
    use num_traits::Signed;
    use super::bignum::{int_rep, to_value};
    let op = if shl { "<<" } else { ">>" };
    // tclsh shift operands must be integers: a float operand errors
    // with the operator named, separately from other non-numeric
    // strings (`1.5 << 2` vs `"abc" << 1`).
    if int_rep(left).is_none() {
        int_operand(left, op)?;
    }
    // Count: i64 fast path; a bignum count behaves like an unbounded
    // one (>> saturates below, << errors above — `-0x8000000000000001
    // >> 0x8000000000000000` is -1, expr-48.1).
    let count = match int_rep(right) {
        Some(IntRep::I64(c)) => c,
        Some(big) => {
            if big.to_big().is_negative() {
                return Err(Error::Msg("negative shift argument".to_string()));
            }
            i64::MAX
        }
        None => int_operand(right, op)?,
    };
    if count < 0 {
        return Err(Error::Msg("negative shift argument".to_string()));
    }
    // Bignum cap: results wider than ~2^27 bits exceed tclsh's
    // bignum range ("integer value too large to represent", probed
    // with `1 << 0x8000000000000000`).
    if shl && count > 1 << 27 {
        return Err(Error::Msg(
            "integer value too large to represent".to_string(),
        ));
    }
    match int_rep(left) {
        Some(IntRep::I64(a)) if !shl && count >= 64 => {
            Ok(Value::from_int(if a < 0 { -1 } else { 0 }))
        }
        Some(IntRep::Big(b)) => {
            let shifted = if shl {
                b << (count as usize)
            } else if count as u64 >= b.bits() as u64 {
                // Saturate without building a 2^count denominator.
                if b.is_negative() {
                    num_bigint::BigInt::from(-1)
                } else {
                    num_bigint::BigInt::from(0)
                }
            } else {
                b >> (count as usize)
            };
            Ok(to_value(IntRep::Big(shifted)))
        }
        Some(IntRep::I64(a)) if shl => {
            let wide = ((a as u64) << (count as u32)) as i64;
            if count < 64 && (wide >> count) == a {
                Ok(Value::from_int(wide))
            } else {
                // Overflow: widen to an exact bignum (1 << 63 is
                // 9223372036854775808, not i64::MIN).
                Ok(to_value(IntRep::Big(
                    num_bigint::BigInt::from(a) << (count as usize),
                )))
            }
        }
        Some(IntRep::I64(a)) => Ok(Value::from_int(a >> (count as u32))),
        None => Err(Error::Msg(format!(
            "can't use non-numeric string as operand of \"{op}\""
        ))),
    }
}

/// `&`, `|`, `^` — exact on bignum operands, error (not wrap) on floats.
pub(crate) fn int_bitop(left: &Value, right: &Value, op: char) -> Result<Value> {
    if let (Some(ia), Some(ib)) = (
        super::bignum::int_rep(left),
        super::bignum::int_rep(right),
    ) {
        // i64×i64 never overflows a bitwise op — no bignum conversion on
        // the common path (this used to widen BOTH operands on every
        // `&`/`|`/`^`).
        if let (IntRep::I64(a), IntRep::I64(b)) = (&ia, &ib) {
            return Ok(Value::from_int(match op {
                '|' => a | b,
                '^' => a ^ b,
                _ => a & b,
            }));
        }
        let (a, b) = (ia.to_big(), ib.to_big());
        return Ok(super::bignum::to_value(IntRep::Big(
            match op {
                '|' => a | b,
                '^' => a ^ b,
                _ => a & b,
            },
        )));
    }
    // Non-int operands: tclsh rejects floats and other non-numeric
    // strings with the operator in the message.
    let a = int_operand(left, &op.to_string())?;
    let b = int_operand(right, &op.to_string())?;
    Ok(Value::from_int(match op {
        '|' => a | b,
        '^' => a ^ b,
        _ => a & b,
    }))
}

/// `==` — exact numeric comparison when both operands are numeric
/// (int pairs compare as i64, no EPSILON tolerance); string comparison
/// otherwise.  NaN compares unequal to everything (expr-22.9).
pub(crate) fn op_eq(left: &Value, right: &Value) -> Value {
    if expr_funcs::nan_pair(left, right) {
        return Value::from_bool(false);
    }
    match expr_funcs::numeric_cmp(left, right) {
        Some(ord) => Value::from_bool(ord == core::cmp::Ordering::Equal),
        None => Value::from_bool(left.as_str() == right.as_str()),
    }
}

/// `!=` — negation of [`op_eq`]'s comparison, with NaN unequal to all.
pub(crate) fn op_ne(left: &Value, right: &Value) -> Value {
    if expr_funcs::nan_pair(left, right) {
        return Value::from_bool(true);
    }
    match expr_funcs::numeric_cmp(left, right) {
        Some(ord) => Value::from_bool(ord != core::cmp::Ordering::Equal),
        None => Value::from_bool(left.as_str() != right.as_str()),
    }
}

/// `<`, `>`, `<=`, `>=` — numeric when both operands are numeric,
/// string comparison otherwise; every comparison is false on NaN.
pub(crate) fn op_rel(left: &Value, right: &Value, op: &str) -> Value {
    use core::cmp::Ordering;
    // NaN: every relational comparison is false.
    if expr_funcs::nan_pair(left, right) {
        return Value::from_bool(false);
    }
    match expr_funcs::numeric_cmp(left, right) {
        Some(ord) => Value::from_bool(match op {
            "<" => ord == Ordering::Less,
            ">" => ord == Ordering::Greater,
            "<=" => ord != Ordering::Greater,
            ">=" => ord != Ordering::Less,
            _ => false,
        }),
        None => Value::from_bool(match op {
            "<" => left.as_str() < right.as_str(),
            ">" => left.as_str() > right.as_str(),
            "<=" => left.as_str() <= right.as_str(),
            ">=" => left.as_str() >= right.as_str(),
            _ => false,
        }),
    }
}

/// `eq` / `ne` — pure string comparison.
pub(crate) fn op_str_eq(left: &Value, right: &Value) -> Value {
    Value::from_bool(left.as_str() == right.as_str())
}

pub(crate) fn op_str_ne(left: &Value, right: &Value) -> Value {
    Value::from_bool(left.as_str() != right.as_str())
}

/// `**` — Tcl: two integer operands use integer exponentiation (units
/// short-circuit, negative exponent yields 0, ≥2^28-bit results error);
/// any float computes in f64.
pub(crate) fn op_pow(base: Value, exp: Value) -> Result<Value> {
    if let (Some(a), Some(b)) = (base.as_int(), exp.as_int()) {
        return match a {
            // Units short-circuit before any range check — tclsh
            // computes 1**268435456 and (-1)**268435456 instantly.
            0 if b < 0 => Err(Error::Msg(
                "exponentiation of zero by negative power".to_string(),
            )),
            // 0**0 is 1 (expr-23.15); 0**n is 0 (expr-23.14).
            0 if b == 0 => Ok(Value::from_int(1)),
            0 => Ok(Value::from_int(0)),
            1 => Ok(Value::from_int(1)),
            -1 => Ok(Value::from_int(if b % 2 == 0 { 1 } else { -1 })),
            // Integer base with negative exponent yields 0
            // (tclsh: `expr {2**-1}` → 0).
            _ if b < 0 => Ok(Value::from_int(0)),
            // tclsh refuses to build numbers of ≥ 2**28 bits
            // (expr-23.54.12: 3**268435456 → "exponent too large").
            _ if b >= 1 << 28 => Err(Error::Msg("exponent too large".to_string())),
            _ => match a.checked_pow(b as u32) {
                Some(r) => Ok(Value::from_int(r)),
                // Overflow widens to an exact bignum (expr-23.48:
                // 2**81 is 2417851639229258349412352).
                None => Ok(bignum::to_value(
                    bignum::IntRep::Big(
                        num_bigint::BigInt::from(a)
                            .pow(b as u32),
                    ),
                )),
            },
        };
    }
    match (base.as_float(), exp.as_float()) {
        (Some(a), Some(b)) => {
            if a == 0.0 && b < 0.0 {
                // 0.0**-1 / 0**-1.0 error like the integer form.
                return Err(Error::Msg(
                    "exponentiation of zero by negative power".to_string(),
                ));
            }
            Ok(expr_funcs::float_value(a.powf(b)))
        }
        _ => Err(Error::type_mismatch("number", "non-numeric value")),
    }
}

/// `!` — strict-boolean operand, integral result.
pub(crate) fn op_not(val: &Value) -> Result<Value> {
    Ok(Value::from_bool(!expr_funcs::not_operand(val)?))
}

/// `~` — two's-complement complement, exact on bignums
/// (~2^63 is -9223372036854775809).
pub(crate) fn op_bitnot(val: &Value) -> Result<Value> {
    if let Some(rep) = bignum::int_rep(val) {
        return Ok(bignum::to_value(bignum::IntRep::Big(
            -rep.to_big() - num_bigint::BigInt::from(1),
        )));
    }
    let n = as_int_val(val)?;
    Ok(Value::from_int(!n))
}

/// Unary `-` — integer operands stay integers (bignum-exact on
/// overflow), float operands stay floats.
pub(crate) fn op_neg(val: &Value) -> Result<Value> {
    if let Some(rep) = bignum::int_rep(val) {
        return Ok(match rep {
            bignum::IntRep::I64(i) => match i.checked_neg() {
                Some(r) => Value::from_int(r),
                None => bignum::to_value(bignum::IntRep::Big(
                    num_bigint::BigInt::from(2i8).pow(63u32),
                )),
            },
            big => bignum::to_value(bignum::IntRep::Big(
                -big.to_big(),
            )),
        });
    }
    match val.as_float() {
        Some(n) => Ok(expr_funcs::float_value(-n)),
        None => Err(Error::type_mismatch("number", "non-numeric value")),
    }
}

/// Shift/bitop operand coercion: ints pass; the error names the
/// operator and distinguishes floats from other non-numeric strings.
fn int_operand(v: &Value, op: &str) -> Result<i64> {
    if let Some(i) = v.as_int() {
        return Ok(i);
    }
    if v.as_float().is_some() {
        return Err(Error::Msg(format!(
            "can't use floating-point value as operand of \"{op}\""
        )));
    }
    Err(Error::Msg(format!(
        "can't use non-numeric string as operand of \"{op}\""
    )))
}
