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
pub fn as_int_val(v: &Value) -> Result<i64> {
    v.as_int()
        .or_else(|| v.as_float().map(|f| f as i64))
        .ok_or_else(|| Error::type_mismatch("integer", v.as_str()))
}

/// Operand position in a 9.1 `cannot use …` message: a unary op has no
/// side, a binary op names the failing side (tclsh 9.1: `1 + "a"` →
/// `as right operand of`, `"a" + 1` → `as left operand of`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Unary,
    Left,
    Right,
}

impl Side {
    fn phrase(self) -> &'static str {
        match self {
            Side::Unary => "as operand of",
            Side::Left => "as left operand of",
            Side::Right => "as right operand of",
        }
    }
}

/// A value whose text parses as a Tcl list with more than one element is
/// reported as a list, not as a string (9.1: `expr {"a b" + 1}` →
/// `cannot use a list as left operand of "+"`; a malformed list (`a {b`)
/// or a single-element one keeps the string form with the value
/// verbatim).
fn is_multi_element_list(v: &Value) -> bool {
    crate::value::parse_list_full(v.as_str())
        .map(|l| l.len() > 1)
        .unwrap_or(false)
}

/// 9.1's operand-classification error.  Classification order: a
/// multi-element list, a NaN float, any other float, then a
/// non-numeric string.  The value is inserted verbatim between the
/// quotes — tclsh does not escape embedded quotes
/// (`cannot use non-numeric string "a"b" …`).  errorCode per class in
/// [`crate::error::Error::tcl_error_code`].
pub fn operand_class_error(v: &Value, op: &str, side: Side) -> Error {
    if is_multi_element_list(v) {
        return Error::Msg(format!("cannot use a list {} \"{op}\"", side.phrase()));
    }
    let s = v.as_str();
    if let Some(f) = v.as_float() {
        if f.is_nan() {
            return Error::Msg(format!(
                "cannot use non-numeric floating-point value \"{s}\" {} \"{op}\"",
                side.phrase()
            ));
        }
        return Error::Msg(format!(
            "cannot use floating-point value \"{s}\" {} \"{op}\"",
            side.phrase()
        ));
    }
    Error::Msg(format!(
        "cannot use non-numeric string \"{s}\" {} \"{op}\"",
        side.phrase()
    ))
}

/// Numeric-for-arithmetic (int rep, or a non-NaN float): NaN poisons a
/// side on its own (`NaN + 1` names the left operand first).
fn arith_ok(v: &Value) -> bool {
    super::bignum::int_rep(v).is_some() || v.as_float().is_some_and(|f| !f.is_nan())
}

/// `+`, `-`, `*`, `/` — Tcl numeric semantics.
///
/// Integer operands use integer arithmetic (`/` is floor division,
/// result is int); on i64 overflow the operation widens to an exact
/// bignum (`9223372036854775807 + 1` is `9223372036854775808`,
/// tclsh-style).  If either side is float, compute in f64 and the
/// result stays a float.
/// Operand type error: the first failing operand is named (left before
/// right; NaN counts as failing), 9.1 wording with the value quoted.
fn pair_operand_error(left: &Value, right: &Value, op: &str) -> Error {
    if !arith_ok(left) {
        return operand_class_error(left, op, Side::Left);
    }
    operand_class_error(right, op, Side::Right)
}

pub fn numeric_binop(left: &Value, right: &Value, op: char) -> Result<Value> {
    use super::bignum::{floor_div as big_div, int_rep, to_value};
    use num_bigint::BigInt;
    // Immediate×immediate: untag, checked arithmetic, retag — no rep
    // parsing, no box, no refcount.  Overflow/div-edge falls into the
    // exact same widening arms the general path uses (i63 bounds keep
    // every non-overflowing result representable).
    if left.is_imm() && right.is_imm() {
        let (a, b) = (left.imm(), right.imm());
        let wide = |x: i64, y: i64| -> Result<Value> {
            use num_bigint::BigInt;
            let (ba, bb) = (BigInt::from(x), BigInt::from(y));
            Ok(match op {
                '+' => to_value(IntRep::Big(ba + bb)),
                '-' => to_value(IntRep::Big(ba - bb)),
                '*' => to_value(IntRep::Big(ba * bb)),
                '/' => to_value(IntRep::Big(big_div(&ba, &bb))),
                _ => {
                    return Err(Error::runtime(
                        "unknown op",
                        crate::error::ErrorCode::InvalidOp,
                    ))
                }
            })
        };
        // The immediate tag only holds i63: a checked-i64 success can
        // still exceed it (11^18 = 5.6e18 < i64::MAX but > 2^62) — such
        // results widen to the exact BigInt form.
        let fits = |r: i64| ((i64::MIN >> 1)..=(i64::MAX >> 1)).contains(&r);
        return match op {
            '+' => match a.checked_add(b) {
                Some(r) if fits(r) => Ok(Value::from_imm(r)),
                _ => wide(a, b),
            },
            '-' => match a.checked_sub(b) {
                Some(r) if fits(r) => Ok(Value::from_imm(r)),
                _ => wide(a, b),
            },
            '*' => match a.checked_mul(b) {
                Some(r) if fits(r) => Ok(Value::from_imm(r)),
                _ => wide(a, b),
            },
            '/' => {
                if b == 0 {
                    return Err(Error::DivisionByZero);
                }
                if a == i64::MIN && b == -1 {
                    return wide(a, b);
                }
                Ok(Value::from_imm(super::expr::floor_div(a, b)))
            }
            _ => Err(Error::runtime(
                "unknown op",
                crate::error::ErrorCode::InvalidOp,
            )),
        };
    }
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
                _ => Err(Error::runtime(
                    "unknown op",
                    crate::error::ErrorCode::InvalidOp,
                )),
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
            _ => Err(Error::runtime(
                "unknown op",
                crate::error::ErrorCode::InvalidOp,
            )),
        };
    }
    // NaN operands can't take part in arithmetic (9.1: `NaN + 1` →
    // `cannot use non-numeric floating-point value "NaN" as left operand
    // of "+"`).  Checked only now, AFTER the both-integer fast path: an
    // integer never carries NaN, and probing as_float first re-parsed
    // every string operand as a float before the int parse could even
    // run (double parse per `+` on bracket results — several percent of
    // the fib profile).
    match (left.as_float(), right.as_float()) {
        (Some(a), Some(b)) => {
            // The first NaN operand is named, left before right.
            if a.is_nan() {
                return Err(operand_class_error(left, &op.to_string(), Side::Left));
            }
            if b.is_nan() {
                return Err(operand_class_error(right, &op.to_string(), Side::Right));
            }
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
                _ => {
                    return Err(Error::runtime(
                        "unknown op",
                        crate::error::ErrorCode::InvalidOp,
                    ))
                }
            };
            Ok(expr_funcs::float_value(result))
        }
        _ => Err(pair_operand_error(left, right, &op.to_string())),
    }
}

/// `%` with Tcl floor semantics; bignum operands stay exact.
pub fn int_mod(left: &Value, right: &Value) -> Result<Value> {
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
    // (`1.5 % 2` → cannot use floating-point value "1.5" as left operand
    // of "%") from any other non-numeric one (`true % 2` → cannot use
    // non-numeric string "true" as left operand of "%").
    let a = int_operand(left, "%", Side::Left)?;
    let b = int_operand(right, "%", Side::Right)?;
    if b == 0 {
        return Err(Error::DivisionByZero);
    }
    Ok(Value::from_int(if a == i64::MIN && b == -1 {
        0
    } else {
        super::expr::floor_mod(a, b)
    }))
}

/// One `<<`/`>>` operation with Tcl's bignum-widening semantics.
pub fn int_shift(left: &Value, right: &Value, shl: bool) -> Result<Value> {
    use super::bignum::{int_rep, to_value};
    use num_traits::Signed;
    let op = if shl { "<<" } else { ">>" };
    // tclsh shift operands must be integers: a float operand errors
    // with the operator named, separately from other non-numeric
    // strings (`1.5 << 2` vs `"abc" << 1`).
    if int_rep(left).is_none() {
        int_operand(left, op, Side::Left)?;
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
        None => int_operand(right, op, Side::Right)?,
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
            } else if count as u64 >= b.bits() {
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
            if count < 64 {
                let wide = ((a as u64) << (count as u32)) as i64;
                if (wide >> count) == a {
                    return Ok(Value::from_int(wide));
                }
            }
            // Overflow (or a shift past the word): widen to an exact
            // bignum (1 << 63 is 9223372036854775808, not i64::MIN).
            Ok(to_value(IntRep::Big(
                num_bigint::BigInt::from(a) << (count as usize),
            )))
        }
        Some(IntRep::I64(a)) => {
            if count >= 64 {
                // An arithmetic shift past the word sign-extends: 0 for
                // non-negative values, -1 for negative ones.
                Ok(Value::from_int(if a < 0 { -1 } else { 0 }))
            } else {
                Ok(Value::from_int(a >> (count as u32)))
            }
        }
        None => Err(pair_operand_error(left, right, op)),
    }
}

/// `&`, `|`, `^` — exact on bignum operands, error (not wrap) on floats.
pub fn int_bitop(left: &Value, right: &Value, op: char) -> Result<Value> {
    if let (Some(ia), Some(ib)) = (super::bignum::int_rep(left), super::bignum::int_rep(right)) {
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
        return Ok(super::bignum::to_value(IntRep::Big(match op {
            '|' => a | b,
            '^' => a ^ b,
            _ => a & b,
        })));
    }
    // Non-int operands: tclsh rejects floats and other non-numeric
    // strings with the operator in the message, naming the failing side.
    let a = int_operand(left, &op.to_string(), Side::Left)?;
    let b = int_operand(right, &op.to_string(), Side::Right)?;
    Ok(Value::from_int(match op {
        '|' => a | b,
        '^' => a ^ b,
        _ => a & b,
    }))
}

/// [`op_rel`]/[`op_eq`]/[`op_ne`]'s truth value without the `Value`
/// wrapping: the fused comparison branches consume the bool directly,
/// skipping `from_bool`'s cached-singleton handle and the `strict_bool`
/// re-coercion of what is already a boolean.  `cc` is the
/// superinstruction encoding (0=<, 1=>, 2=<=, 3=>=, 4===, 5=!=); the
/// semantics are exactly the `Value` forms' — which are now defined
/// through this one source.
pub(crate) fn cmp_bool(left: &Value, right: &Value, cc: u8) -> bool {
    use core::cmp::Ordering;
    if left.is_imm() && right.is_imm() {
        let (a, b) = (left.imm(), right.imm());
        return match cc {
            0 => a < b,
            1 => a > b,
            2 => a <= b,
            3 => a >= b,
            4 => a == b,
            _ => a != b,
        };
    }
    match cc {
        // Relational: NaN → false; numeric first, string fallback.
        0..=3 => {
            if expr_funcs::nan_pair(left, right) {
                return false;
            }
            match expr_funcs::numeric_cmp(left, right) {
                Some(ord) => match cc {
                    0 => ord == Ordering::Less,
                    1 => ord == Ordering::Greater,
                    2 => ord != Ordering::Greater,
                    _ => ord != Ordering::Less,
                },
                None => match cc {
                    0 => left.as_str() < right.as_str(),
                    1 => left.as_str() > right.as_str(),
                    2 => left.as_str() <= right.as_str(),
                    _ => left.as_str() >= right.as_str(),
                },
            }
        }
        // `==`: NaN unequal to all; numeric else string equality.
        4 => {
            if expr_funcs::nan_pair(left, right) {
                return false;
            }
            match expr_funcs::numeric_cmp(left, right) {
                Some(ord) => ord == Ordering::Equal,
                None => left.as_str() == right.as_str(),
            }
        }
        // `!=`: negation of the above, NaN unequal to all.
        _ => {
            if expr_funcs::nan_pair(left, right) {
                return true;
            }
            match expr_funcs::numeric_cmp(left, right) {
                Some(ord) => ord != Ordering::Equal,
                None => left.as_str() != right.as_str(),
            }
        }
    }
}

/// `==` — exact numeric comparison when both operands are numeric
/// (int pairs compare as i64, no EPSILON tolerance); string comparison
/// otherwise.  NaN compares unequal to everything (expr-22.9).
pub fn op_eq(left: &Value, right: &Value) -> Value {
    Value::from_bool(cmp_bool(left, right, 4))
}

/// `!=` — negation of [`op_eq`]'s comparison, with NaN unequal to all.
pub fn op_ne(left: &Value, right: &Value) -> Value {
    Value::from_bool(cmp_bool(left, right, 5))
}

/// `<`, `>`, `<=`, `>=` — numeric when both operands are numeric,
/// string comparison otherwise; every comparison is false on NaN.
pub fn op_rel(left: &Value, right: &Value, op: &str) -> Value {
    let cc = match op {
        "<" => 0,
        ">" => 1,
        "<=" => 2,
        ">=" => 3,
        _ => 0,
    };
    Value::from_bool(cmp_bool(left, right, cc))
}

/// `eq` / `ne` — pure string comparison.
///
/// lazy-string 白名单（§6）：两个操作数都持宿主池句柄时走免物化比较
/// （UTF-8↔UTF-16 对合法标量值双射，深层相等安全；Dual 态有字节缓存也
/// 不影响等值判定）。其余形态走既有字节路径。
pub fn op_str_eq(left: &Value, right: &Value) -> Value {
    if let (Some(a), Some(b)) = (left.host_str_handle(), right.host_str_handle()) {
        if let Some(eq) = crate::host_string::with_backing(|bk| bk.equal(a, b)) {
            return Value::from_bool(eq);
        }
    }
    Value::from_bool(left.as_str() == right.as_str())
}

pub fn op_str_ne(left: &Value, right: &Value) -> Value {
    if let (Some(a), Some(b)) = (left.host_str_handle(), right.host_str_handle()) {
        if let Some(eq) = crate::host_string::with_backing(|bk| bk.equal(a, b)) {
            return Value::from_bool(!eq);
        }
    }
    Value::from_bool(left.as_str() != right.as_str())
}

/// `**` — Tcl: two integer operands use integer exponentiation (units
/// short-circuit, negative exponent yields 0, ≥2^28-bit results error);
/// any float computes in f64.  D5: bignum operands stay exact —
/// `(2**64)**2` is the exact bignum, not a double shimmer.
pub fn op_pow(base: Value, exp: Value) -> Result<Value> {
    use num_bigint::BigInt;
    use num_traits::{Signed, ToPrimitive};
    if let (Some(ia), Some(ib)) = (bignum::int_rep(&base), bignum::int_rep(&exp)) {
        // Exponent sign/parity/zero for both reps (D5: the exponent can
        // itself be a bignum); the magnitude also feeds the 2**28-bit cap.
        let (b_neg, b_even, bmag) = match &ib {
            bignum::IntRep::I64(b) => (*b < 0, b % 2 == 0, BigInt::from(b.unsigned_abs())),
            bignum::IntRep::Big(b) => {
                let mag = b.abs();
                let even = mag.bit(0) == false;
                (b.is_negative(), even, mag)
            }
        };
        let b_zero = ib.is_zero();
        // Units short-circuit before any range check — tclsh
        // computes 1**268435456 and (-1)**268435456 instantly.
        match &ia {
            bignum::IntRep::I64(0) => {
                return if b_neg {
                    Err(Error::Msg(
                        "exponentiation of zero by negative power".to_string(),
                    ))
                } else {
                    // 0**0 is 1 (expr-23.15); 0**n is 0 (expr-23.14).
                    Ok(Value::from_int(if b_zero { 1 } else { 0 }))
                };
            }
            bignum::IntRep::I64(1) => return Ok(Value::from_int(1)),
            bignum::IntRep::I64(-1) => {
                return Ok(Value::from_int(if b_even { 1 } else { -1 }))
            }
            _ => {}
        }
        // Integer base with negative exponent yields 0
        // (tclsh: `expr {2**-1}` → 0).
        if b_neg {
            return Ok(Value::from_int(0));
        }
        // tclsh refuses to build numbers of ≥ 2**28 bits
        // (expr-23.54.12: 3**268435456 → "exponent too large"); the cap
        // keys on the exponent magnitude in both reps.
        if bmag >= BigInt::from(1u32 << 28) {
            return Err(Error::Msg("exponent too large".to_string()));
        }
        let b32 = match &ib {
            bignum::IntRep::I64(b) => *b as u32,
            bignum::IntRep::Big(b) => b.to_u32().unwrap_or(u32::MAX),
        };
        // i64×i64: checked first, the common small case allocates nothing.
        if let bignum::IntRep::I64(a) = &ia {
            if let Some(r) = a.checked_pow(b32) {
                return Ok(Value::from_int(r));
            }
        }
        // Overflow — or a bignum base — widens to an exact bignum
        // (expr-23.48: 2**81 is 2417851639229258349412352; D5:
        // (2**64)**2 stays exact through the Big rep).
        return Ok(bignum::to_value(bignum::IntRep::Big(
            ia.to_big().pow(b32),
        )));
    }
    match (base.as_float(), exp.as_float()) {
        (Some(a), Some(b)) => {
            // The first NaN operand is named (left before right).
            if a.is_nan() {
                return Err(operand_class_error(&base, "**", Side::Left));
            }
            if b.is_nan() {
                return Err(operand_class_error(&exp, "**", Side::Right));
            }
            if a == 0.0 && b < 0.0 {
                // 0.0**-1 / 0**-1.0 error like the integer form.
                return Err(Error::Msg(
                    "exponentiation of zero by negative power".to_string(),
                ));
            }
            Ok(expr_funcs::float_value(a.powf(b)))
        }
        _ => {
            // 9.1: the first failing operand is named
            // (`"a" ** 2` → as left operand of "**").
            if !arith_ok(&base) {
                return Err(operand_class_error(&base, "**", Side::Left));
            }
            Err(operand_class_error(&exp, "**", Side::Right))
        }
    }
}

/// `!` — strict-boolean operand, integral result.
pub fn op_not(val: &Value) -> Result<Value> {
    Ok(Value::from_bool(!expr_funcs::not_operand(val)?))
}

/// `~` — two's-complement complement, exact on bignums
/// (~2^63 is -9223372036854775809).  A float operand errors (9.1:
/// `~4.0` → `cannot use floating-point value "4.0" as operand of "~"`).
pub fn op_bitnot(val: &Value) -> Result<Value> {
    if let Some(rep) = bignum::int_rep(val) {
        return Ok(bignum::to_value(bignum::IntRep::Big(
            -rep.to_big() - num_bigint::BigInt::from(1),
        )));
    }
    let n = int_operand(val, "~", Side::Unary)?;
    Ok(Value::from_int(!n))
}

/// Unary `+` — 9.1 语义 = 数值断言：整数/浮点原样通过（NaN 报浮点
/// 操作数类错误），非数字串报操作数类错误（`+"a"` → cannot use
/// non-numeric string "a" as operand of "+"——tclsh 9.1 实测）。
pub fn op_pos(val: &Value) -> Result<Value> {
    if bignum::int_rep(val).is_some() {
        return Ok(val.clone());
    }
    match val.as_float() {
        Some(n) if n.is_nan() => Err(operand_class_error(val, "+", Side::Unary)),
        Some(_) => Ok(val.clone()),
        None => Err(operand_class_error(val, "+", Side::Unary)),
    }
}

/// Unary `-` — integer operands stay integers (bignum-exact on
/// overflow), float operands stay floats (NaN errors).
pub fn op_neg(val: &Value) -> Result<Value> {
    if let Some(rep) = bignum::int_rep(val) {
        return Ok(match rep {
            bignum::IntRep::I64(i) => match i.checked_neg() {
                Some(r) => Value::from_int(r),
                None => bignum::to_value(bignum::IntRep::Big(
                    num_bigint::BigInt::from(2i8).pow(63u32),
                )),
            },
            big => bignum::to_value(bignum::IntRep::Big(-big.to_big())),
        });
    }
    match val.as_float() {
        Some(n) if n.is_nan() => Err(operand_class_error(val, "-", Side::Unary)),
        Some(n) => Ok(expr_funcs::float_value(-n)),
        None => Err(operand_class_error(val, "-", Side::Unary)),
    }
}

/// Shift/bitop operand coercion: ints pass; the error names the
/// operator, the failing side, and the operand class (9.1 wording).
fn int_operand(v: &Value, op: &str, side: Side) -> Result<i64> {
    if let Some(i) = v.as_int() {
        return Ok(i);
    }
    Err(operand_class_error(v, op, side))
}
