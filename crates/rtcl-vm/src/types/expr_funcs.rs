//! Math function evaluation for the Tcl expression parser.

use num_traits::Signed;
use core::cmp::Ordering;

use crate::error::{Error, Result};
use crate::value::Value;

/// tclsh math-domain failure: `domain error: argument not in valid range`
/// with errorCode `ARITH DOMAIN {...}`.
pub(crate) fn domain_error() -> Error {
    Error::runtime(
        "domain error: argument not in valid range",
        crate::error::ErrorCode::Generic,
    )
}

/// A float `Value` carrying Tcl's canonical string form. Used for every
/// float produced by the expression evaluator so results stringify exactly
/// like tclsh (rtcl-vm's `Value::from_float` uses a different layout).
pub(crate) fn float_value(f: f64) -> Value {
    // Float-typed (not a string): tclsh's expr results are double objects,
    // so canonicalization skips them (`expr {0.0/0.0}` prints NaN while
    // `expr {"nan"}` — a *string* result — domain-errors).
    Value::from_float(f)
}

/// Tcl boolean word parsing: case-insensitive, any unique-prefix
/// abbreviation of true/false/yes/no/on/off. Returns `None` for ambiguous
/// ("o" — on/off) or unrecognized strings.  9.1 起不再 trim：带首尾空白
/// 的串不是布尔（`expr {" true " && 1}` → expected boolean value but
/// got " true "；裸词 ` true ` 由词法空白跳过处理，不受影响）。
pub(crate) fn bool_from_string(s: &str) -> Option<bool> {
    let lower = s.to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let mut candidates = ["true", "false", "yes", "no", "on", "off"]
        .iter()
        .filter(|word| word.starts_with(lower.as_str()));
    let first = *candidates.next()?;
    if candidates.next().is_some() {
        return None; // ambiguous prefix
    }
    Some(matches!(first, "true" | "yes" | "on"))
}

/// Tcl boolean coercion (`Tcl_GetBooleanFromObj`): any numeric value is a
/// valid boolean (zero, including 0.0, is false); otherwise the string
/// must be a boolean word or unambiguous abbreviation. Anything else is an
/// error, as in `while {$x}` with x="foo".
#[inline]
pub fn strict_bool(v: &Value) -> Result<bool> {
    if let Some(i) = v.as_int() {
        return Ok(i != 0);
    }
    if let Some(f) = v.as_float() {
        // 9.1: a NaN is not a usable boolean (`if {NaN}` →
        // floating point value is Not a Number, TCL VALUE DOUBLE NAN).
        if f.is_nan() {
            return Err(Error::Msg(
                "floating point value is Not a Number".to_string(),
            ));
        }
        return Ok(f != 0.0);
    }
    if let Some(b) = bool_from_string(v.as_str()) {
        return Ok(b);
    }
    // A multi-element list value keeps the unquoted `a list` form
    // (`if {$x} {}` with x={1 2} → expected boolean value but got a
    // list); everything else quotes the text.
    if crate::value::parse_list_full(v.as_str())
        .map(|l| l.len() > 1)
        .unwrap_or(false)
    {
        return Err(Error::Msg("expected boolean value but got a list".to_string()));
    }
    Err(Error::Msg(format!(
        "expected boolean value but got \"{}\"",
        v.as_str()
    )))
}

/// Operand coercion for unary `!`: accepts the same values as
/// [`strict_bool`], but Tcl reports the operand-class message for a
/// non-numeric, non-boolean operand of `!` (9.1: `!$x` with x="a" →
/// `cannot use non-numeric string "a" as operand of "!"`; NaN →
/// `cannot use non-numeric floating-point value …`; a list →
/// `cannot use a list as operand of "!"`).

/// `expected <what> but got …` 数值实参报错（9.1 钉形：值按空白切分
/// ≥2 词 → `but got a list`，否则引原文——`hypot("a b", 2)` /
/// `isqrt([lseq 1 3])` 钉形）。
fn value_arg_err(expected: &str, v: &Value) -> Error {
    Error::Msg(crate::value::value_report(expected, v.as_str()))
}

pub(crate) fn not_operand(v: &Value) -> Result<bool> {
    if let Some(i) = v.as_int() {
        return Ok(i != 0);
    }
    if let Some(f) = v.as_float() {
        if f.is_nan() {
            return Err(super::expr_ops::operand_class_error(
                v,
                "!",
                super::expr_ops::Side::Unary,
            ));
        }
        return Ok(f != 0.0);
    }
    if let Some(b) = bool_from_string(v.as_str()) {
        return Ok(b);
    }
    Err(super::expr_ops::operand_class_error(
        v,
        "!",
        super::expr_ops::Side::Unary,
    ))
}

/// Numeric comparison following Tcl rules: if both operands parse as
/// integers, compare as i64 (no precision loss through double); a mixed
/// int/double pair is compared exactly; two doubles use IEEE comparison.
/// Returns `None` when either operand is non-numeric (caller falls back to
/// string comparison).
/// Both operands numeric and at least one NaN — Tcl: every comparison
/// is false except `!=` (expr-22.9: NaN == NaN -> 0).
pub(crate) fn nan_pair(a: &Value, b: &Value) -> bool {
    let numeric = |v: &Value| v.as_int().is_some() || v.as_float().is_some();
    let nan = |v: &Value| v.as_float().map(|f| f.is_nan()).unwrap_or(false);
    numeric(a) && numeric(b) && (nan(a) || nan(b))
}

pub(crate) fn numeric_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    use super::bignum::{cmp_rep_float, int_rep, IntRep};
    match (int_rep(a), int_rep(b)) {
        // i64×i64 compares directly — the widening arm below used to
        // allocate TWO BigInts per comparison (`$n < 2` in every loop
        // condition paid them each iteration).
        (Some(IntRep::I64(x)), Some(IntRep::I64(y))) => Some(x.cmp(&y)),
        (Some(x), Some(y)) => Some(x.to_big().cmp(&y.to_big())),
        (Some(x), None) => b.as_float().and_then(|y| cmp_rep_float(&x, y)),
        (None, Some(y)) => a
            .as_float()
            .and_then(|x| cmp_rep_float(&y, x).map(Ordering::reverse)),
        (None, None) => match (a.as_float(), b.as_float()) {
            (Some(x), Some(y)) => x.partial_cmp(&y),
            _ => None,
        },
    }
}

/// `abs()`: tclsh's ExprAbsCmd returns its argument **unchanged** when the
/// value is already non-negative (`::tcl::mathfunc::abs { \t0x0}` renders
/// " \t0x0", `abs 1e-324` renders "1e-324"), canonical only for negatives
/// (probed).  NaN is an error.
fn abs_value(v: &Value) -> Result<Value> {
    use super::bignum::{to_value, IntRep};
    // A non-negative argument comes back *unchanged* (`abs 0x0` is "0x0",
    // `abs " 5"` is " 5") — except a minus-signed zero, which canonicalizes
    // (`abs -0` is 0, `abs -0x0` is 0, `abs -00` is 0).
    let signed_zero = v.as_str().trim_start().starts_with('-');
    if let Some(rep) = super::bignum::int_rep(v) {
        return match rep {
            IntRep::I64(i) if i > 0 => Ok(v.clone()),
            IntRep::I64(0) if !signed_zero => Ok(v.clone()),
            IntRep::I64(i) => Ok(Value::from_int(match i.checked_abs() {
                Some(r) => r,
                None => {
                    return Ok(to_value(IntRep::Big(
                        num_bigint::BigInt::from(2i8).pow(63u32),
                    )))
                }
            })),
            IntRep::Big(b) if !b.is_negative() && !signed_zero => Ok(v.clone()),
            IntRep::Big(b) => Ok(to_value(IntRep::Big(-b))),
        };
    }
    match v.as_float() {
        Some(d) if d.is_nan() => Err(Error::Msg(
            "floating point value is Not a Number".to_string(),
        )),
        Some(d) if d < 0.0 => Ok(float_value(-d)),
        // -0.0 canonicalizes to +0.0; anything else keeps its string.
        Some(d) if d == 0.0 && d.is_sign_negative() => Ok(float_value(0.0)),
        Some(_) => Ok(v.clone()),
        // 9.1: no "(looks like invalid octal number)" suffix anywhere —
        // `018`-shaped strings are numbers now, so this arm only fires
        // for genuinely non-numeric text.
        None => Err(value_arg_err("number", v)),
    }
}

/// Shared `int()`/`entier()`/`wide()` body.  NaN and infinities error
/// ("integer value too large to represent" / "floating point value is
/// Not a Number"); doubles truncate toward zero exactly; `wrap` takes the
/// low 64 bits of the exact integer (9.1's `wide()` alone), otherwise
/// bignums survive (`int()`/`entier()` in 9.1).
fn int_or_entier(v: &Value, keep_big: bool) -> Result<Value> {
    use super::bignum::{big_from_f64, to_i64_wrap, to_value, IntRep};
    if let Some(rep) = super::bignum::int_rep(v) {
        return Ok(match rep {
            IntRep::I64(i) => Value::from_int(i),
            big => {
                if keep_big {
                    to_value(big)
                } else {
                    Value::from_int(to_i64_wrap(&big.to_big()))
                }
            }
        });
    }
    match v.as_float() {
        Some(n) if n.is_nan() => Err(Error::Msg(
            "floating point value is Not a Number".to_string(),
        )),
        Some(n) if n.is_infinite() => Err(Error::Msg(
            "integer value too large to represent".to_string(),
        )),
        Some(n) => {
            let t = n.trunc();
            if t >= -(2f64.powi(63)) && t < 2f64.powi(63) {
                Ok(Value::from_int(t as i64))
            } else {
                let b = big_from_f64(t).ok_or_else(|| {
                    Error::Msg("integer value too large to represent".to_string())
                })?;
                Ok(if keep_big {
                    to_value(IntRep::Big(b))
                } else {
                    Value::from_int(to_i64_wrap(&b))
                })
            }
        }
        None => Err(value_arg_err("number", v)),
    }
}

/// Integral result of a float argument: i64 when it fits, exact bignum
/// otherwise (round(1e22) is 10000000000000000000000).
fn float_result(t: f64) -> Result<Value> {
    use super::bignum::{big_from_f64, to_value, IntRep};
    if t >= -(2f64.powi(63)) && t < 2f64.powi(63) {
        Ok(Value::from_int(t as i64))
    } else {
        let b = big_from_f64(t)
            .ok_or_else(|| Error::Msg("integer value too large to represent".to_string()))?;
        Ok(to_value(IntRep::Big(b)))
    }
}

fn require_args(name: &str, expected: usize, actual: usize) -> Result<()> {
    if actual < expected {
        Err(Error::Msg(format!(
            "not enough arguments for math function \"{name}\""
        )))
    } else if actual > expected {
        Err(Error::Msg(format!(
            "too many arguments for math function \"{name}\""
        )))
    } else {
        Ok(())
    }
}

// ── C99 全集（TIP 745 / TIP 521；9.1 的 BuiltinFuncTable 把它们直映射
// 到 C 库同名函数，后端在 [`super::c99`]）────────────────────────────

/// `Tcl_GetDoubleFromObj` 口径的浮点实参：整数/bignum/数字串均可；
/// NaN（含 "nan" 串）拒绝 — `floating point value is Not a Number`；
/// 非数字串 — `expected floating-point number but got "..."`（9.1 钉形）。
pub(crate) fn f64_arg(v: &Value) -> Result<f64> {
    // 整数串（0x/0b/0o 前缀、宽十进制）先走精确整数读再转 double
    // ——GetDoubleFromObj 对 "0x10" 给 16.0（cbrt 0x10 = 2.5198… 钉形）；
    // as_float 的字符串臂只认 bignum，i64 前缀字面量会漏到这里。
    if let Some(rep) = super::bignum::int_rep(v) {
        return Ok(match rep {
            super::bignum::IntRep::I64(i) => i as f64,
            super::bignum::IntRep::Big(b) => super::bignum::to_f64(&b),
        });
    }
    match v.as_float() {
        Some(f) if f.is_nan() => Err(Error::Msg(
            "floating point value is Not a Number".to_string(),
        )),
        Some(f) => Ok(f),
        None => Err(value_arg_err("floating-point number", v)),
    }
}

/// `fpclassify` 命令的分类词（fpclassify.n：zero/subnormal/normal/
/// infinite/nan）；非数字串报 expected number（fp_class 的钉形）。
pub(crate) fn fp_classify_word(v: &Value) -> Result<&'static str> {
    Ok(match fp_class(v)? {
        FpClass::Zero => "zero",
        FpClass::Subnormal => "subnormal",
        FpClass::Normal => "normal",
        FpClass::Infinite => "infinite",
        FpClass::Nan => "nan",
    })
}

/// `Tcl_CheckDoubleResult` 的 errno 等价判定（tclBasic.c）：NaN → 域
/// 错误；`errno_err`（本机 oracle 实测会置 ERANGE 的函数：asinh/atanh/
/// erfc/nextafter）的子正规结果 → `floating-point value too large to
/// represent`；0.0 / ±Inf / 其余原样接受（exp2(1e300) 是 Inf，不报错）。
fn check_double(r: f64, errno_err: bool) -> Result<Value> {
    if r.is_nan() {
        return Err(domain_error());
    }
    if errno_err && r.is_subnormal() {
        return Err(Error::Msg(
            "floating-point value too large to represent".to_string(),
        ));
    }
    Ok(float_value(r))
}

fn c99_unary(
    name: &str,
    args: &[Value],
    errno_err: bool,
    f: impl Fn(f64) -> f64,
) -> Result<Value> {
    require_args(name, 1, args.len())?;
    let x = f64_arg(&args[0])?;
    check_double(f(x), errno_err)
}

fn c99_binary(
    name: &str,
    args: &[Value],
    errno_err: bool,
    f: impl Fn(f64, f64) -> f64,
) -> Result<Value> {
    require_args(name, 2, args.len())?;
    let (a, b) = (f64_arg(&args[0])?, f64_arg(&args[1])?);
    check_double(f(a, b), errno_err)
}

/// IEEE 浮点分类（fpclassify 的五类）。
#[derive(Clone, Copy, PartialEq)]
enum FpClass {
    Zero,
    Subnormal,
    Normal,
    Infinite,
    Nan,
}

fn classify(d: f64) -> FpClass {
    if d.is_nan() {
        FpClass::Nan
    } else if d.is_infinite() {
        FpClass::Infinite
    } else if d == 0.0 {
        FpClass::Zero
    } else if d.is_subnormal() {
        FpClass::Subnormal
    } else {
        FpClass::Normal
    }
}

/// `Tcl_GetNumberFromObj` 口径的浮点分类（isinf/isnan/isnormal/
/// issubnormal/isunordered/fpclassify/signbit 共用）：接受 "nan"/"inf"
/// 串；整数 0 → zero、非零 → normal（i64→double 恒为 normal）；bignum
/// 转 double 可溢出为 ±Inf（isinf(2**400) 是 1 的钉形）；非数字串
/// `expected number but got "..."`（无 floating-point 字样，9.1 钉形）。
fn fp_class(v: &Value) -> Result<FpClass> {
    use super::bignum::IntRep;
    // 整数走原始值口径：0 是 zero，其余恒 normal（转 double 不可能
    // subnormal；i64::MIN 也远超 normal 下界）。
    if let Some(IntRep::I64(i)) = super::bignum::int_rep(v) {
        return Ok(if i == 0 {
            FpClass::Zero
        } else {
            FpClass::Normal
        });
    }
    match v.as_float() {
        Some(d) => Ok(classify(d)),
        None => Err(value_arg_err("number", v)),
    }
}

/// `Tcl_GetIntFromObj`（64 位平台）口径：接受 [INT_MIN, UINT_MAX] 的
/// 整数，超界报 `integer value too large to represent`；随后按 C int
/// 截断（2147483648 → -2147483648 — oracle `ldexp 1 2147483648` 是 0.0
/// 的钉形；非整数 `expected integer but got "..."`）。
fn int_arg_32(v: &Value) -> Result<i32> {
    use super::bignum::IntRep;
    let l: i64 = match super::bignum::int_rep(v) {
        Some(IntRep::I64(i)) => i,
        Some(IntRep::Big(b)) => i64::try_from(b.clone()).map_err(|_| {
            Error::Msg("integer value too large to represent".to_string())
        })?,
        None => {
            return Err(Error::Msg(format!(
                "expected integer but got \"{}\"",
                v.as_str()
            )))
        }
    };
    if l < i32::MIN as i64 || l > u32::MAX as i64 {
        return Err(Error::Msg(
            "integer value too large to represent".to_string(),
        ));
    }
    Ok((l as u32) as i32)
}

/// C99 函数族执行体：expr 内 `f(x)` 与 `tcl::mathfunc::f` 命令共用。
/// 各函数的 `errno_err` 旗标与边界钉形见 [`check_double`]。
pub(crate) fn call_c99_math_func(name: &str, args: Vec<Value>) -> Result<Value> {
    use super::c99::sys;
    match name {
        "acosh" => c99_unary(name, &args, false, sys::acosh),
        "asinh" => c99_unary(name, &args, true, sys::asinh),
        "atanh" => c99_unary(name, &args, true, sys::atanh),
        "cbrt" => c99_unary(name, &args, false, sys::cbrt),
        "erf" => c99_unary(name, &args, false, sys::erf),
        "erfc" => c99_unary(name, &args, true, sys::erfc),
        "exp2" => c99_unary(name, &args, false, sys::exp2),
        "expm1" => c99_unary(name, &args, false, sys::expm1),
        // 9.1 的 gamma 就是 tgamma（gamma(-0.5) 是负数 — 手册的
        // "absolute value" 表述与实现不符，实现为准）。
        "gamma" => c99_unary(name, &args, false, sys::tgamma),
        "lgamma" => c99_unary(name, &args, false, sys::lgamma),
        "log1p" => c99_unary(name, &args, false, sys::log1p),
        "log2" => c99_unary(name, &args, false, sys::log2),
        "logb" => c99_unary(name, &args, false, sys::logb),
        "trunc" => c99_unary(name, &args, false, sys::trunc),
        "copysign" => c99_binary(name, &args, false, sys::copysign),
        "dim" => c99_binary(name, &args, false, sys::fdim),
        "nextafter" => c99_binary(name, &args, true, sys::nextafter),
        "remainder" => c99_binary(name, &args, false, sys::remainder),
        "fma" => {
            require_args(name, 3, args.len())?;
            let (a, b, c) = (f64_arg(&args[0])?, f64_arg(&args[1])?, f64_arg(&args[2])?);
            check_double(sys::fma(a, b, c), false)
        }
        // ldexp 的第二实参走 GetIntFromObj 口径（32 位 int 截断）；子
        // 正规结果不置 ERANGE（ldexp(1,-1074) 是 5e-324，接受）。
        "ldexp" => {
            require_args(name, 2, args.len())?;
            let x = f64_arg(&args[0])?;
            let y = int_arg_32(&args[1])?;
            check_double(sys::ldexp(x, y), false)
        }
        // 分类函数：isinf/isnan 等只看类别，NaN 实参显式接受。
        "isinf" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_int(
                (fp_class(&args[0])? == FpClass::Infinite) as i64,
            ))
        }
        "isfinite" => {
            require_args(name, 1, args.len())?;
            let c = fp_class(&args[0])?;
            Ok(Value::from_int(
                (c != FpClass::Infinite && c != FpClass::Nan) as i64,
            ))
        }
        "isnan" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_int((fp_class(&args[0])? == FpClass::Nan) as i64))
        }
        "isnormal" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_int(
                (fp_class(&args[0])? == FpClass::Normal) as i64,
            ))
        }
        "issubnormal" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_int(
                (fp_class(&args[0])? == FpClass::Subnormal) as i64,
            ))
        }
        "isunordered" => {
            require_args(name, 2, args.len())?;
            let a = fp_class(&args[0])?;
            let b = fp_class(&args[1])?;
            Ok(Value::from_int(((a == FpClass::Nan) || (b == FpClass::Nan)) as i64))
        }
        // signbit：整数负值（含 bignum）按值号；浮点按 IEEE 符号位
        // （-0.0 是 1；int 的 -0 是 0 — oracle 钉形）。
        "signbit" => {
            require_args(name, 1, args.len())?;
            use super::bignum::IntRep;
            match super::bignum::int_rep(&args[0]) {
                Some(IntRep::I64(i)) => Ok(Value::from_int((i < 0) as i64)),
                Some(IntRep::Big(b)) => Ok(Value::from_int(b.is_negative() as i64)),
                None => match args[0].as_float() {
                    Some(d) => Ok(Value::from_int(d.is_sign_negative() as i64)),
                    None => Err(value_arg_err("number", &args[0])),
                },
            }
        }
        _ => Err(Error::Msg(format!(
            "invalid command name \"tcl::mathfunc::{name}\""
        ))),
    }
}

/// Evaluate a built-in math function by name.
///
/// `rand_seed` is used only for `rand()` — caller provides a unique value.
pub(crate) fn call_math_func(name: &str, args: Vec<Value>, rand_seed: usize) -> Result<Value> {
    match name {
        "abs" => {
            require_args(name, 1, args.len())?;
            abs_value(&args[0])
        }
        // Tcl 9.1（D5 重钉）：int() truncates toward zero and keeps the
        // exact value — no word wrap (`int(1e22)` is
        // 10000000000000000000000, `int(9223372036854775808)` stays
        // positive; the 8.6 low-64 wrap is retired).  entier() is the
        // same truncation (they differ only for floats in
        // Tcl_GetIntForIndex-era semantics — both exact here).  wide()
        // alone wraps to the low 64 bits (wide(1e22) is
        // 1864712049423024128); out-of-range infinities error.
        "int" => {
            require_args(name, 1, args.len())?;
            int_or_entier(&args[0], true)
        }
        "entier" => {
            require_args(name, 1, args.len())?;
            int_or_entier(&args[0], true)
        }
        "wide" => {
            require_args(name, 1, args.len())?;
            int_or_entier(&args[0], false)
        }
        "double" => {
            require_args(name, 1, args.len())?;
            // Integer literals of any syntax convert exactly
            // (double(0x7fffffffffffffff) is 9.223372036854776e+18).
            if let Some(rep) = super::bignum::int_rep(&args[0]) {
                return Ok(float_value(match rep {
                    super::bignum::IntRep::Big(b) => super::bignum::to_f64(&b),
                    super::bignum::IntRep::I64(i) => i as f64,
                }));
            }
            match args[0].as_float() {
                Some(n) if n.is_nan() => Err(Error::Msg(
                    "floating point value is Not a Number".to_string(),
                )),
                Some(n) => Ok(float_value(n)),
                None => Err(Error::Msg(format!(
                    "expected floating-point number but got \"{}\"",
                    args[0].as_str()
                ))),
            }
        }
        "bool" => {
            require_args(name, 1, args.len())?;
            Ok(Value::from_bool(strict_bool(&args[0])?))
        }
        // round() keeps bignum precision for out-of-range doubles
        // (round(9.2233720368547758e+018) is 9223372036854775808, expr-46.5).
        "round" => {
            require_args(name, 1, args.len())?;
            if let Some(rep) = super::bignum::int_rep(&args[0]) {
                return Ok(super::bignum::to_value(rep));
            }
            match args[0].as_float() {
                Some(n) => float_result(n.round()),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "floor" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(float_value(n.floor())),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "ceil" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => Ok(float_value(n.ceil())),
                None => Err(Error::type_mismatch("number", "non-numeric value")),
            }
        }
        "sqrt" => {
            require_args(name, 1, args.len())?;
            // Bignum operands take the sqrt of the true value, then round
            // (sqrt(10**616) is 1e+308, expr-50.1).
            if let Some(rep) = super::bignum::int_rep(&args[0]) {
                let b = rep.to_big();
                if b.is_negative() {
                    return Err(domain_error());
                }
                return Ok(float_value(super::bignum::sqrt_to_f64(&b)));
            }
            match args[0].as_float() {
                Some(n) if n >= 0.0 => Ok(float_value(n.sqrt())),
                Some(_) => Err(domain_error()),
                None => Err(value_arg_err("floating-point number", &args[0])),
            }
        }
        "pow" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                // tclsh: negative base with a non-integer exponent is a
                // domain error (integer exponents are exact).
                (Some(a), Some(b)) if a < 0.0 && b.fract() != 0.0 => Err(domain_error()),
                (Some(a), Some(b)) => Ok(float_value(a.powf(b))),
                (None, _) => Err(value_arg_err("floating-point number", &args[0])),
                (_, None) => Err(value_arg_err("floating-point number", &args[1])),
            }
        }
        "fmod" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                // Pattern, not a `b == 0.0` guard: identical semantics
                // (constant float patterns compare with `==`, so -0.0
                // matches too — tclsh rejects fmod(x, -0.0) as a domain
                // error, pinned in expr_tests).
                (Some(_), Some(0.0)) => Err(domain_error()),
                (Some(a), Some(b)) => Ok(float_value(a % b)),
                (None, _) => Err(value_arg_err("floating-point number", &args[0])),
                (_, None) => Err(value_arg_err("floating-point number", &args[1])),
            }
        }
        "atan2" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(float_value(a.atan2(b))),
                (None, _) => Err(value_arg_err("floating-point number", &args[0])),
                (_, None) => Err(value_arg_err("floating-point number", &args[1])),
            }
        }
        "hypot" => {
            require_args(name, 2, args.len())?;
            match (args[0].as_float(), args[1].as_float()) {
                (Some(a), Some(b)) => Ok(float_value(a.hypot(b))),
                (None, _) => Err(value_arg_err("floating-point number", &args[0])),
                (_, None) => Err(value_arg_err("floating-point number", &args[1])),
            }
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "log" | "log10" | "exp"
        | "sinh" | "cosh" | "tanh" => {
            require_args(name, 1, args.len())?;
            match args[0].as_float() {
                Some(n) => {
                    // tclsh domain rules: log/log10 reject negatives
                    // (zero → -Inf is fine), asin/acos reject |x| > 1.
                    match name {
                        "log" | "log10" if n < 0.0 => return Err(domain_error()),
                        "asin" | "acos" if !(-1.0..=1.0).contains(&n) => {
                            return Err(domain_error())
                        }
                        _ => {}
                    }
                    let result = match name {
                        "sin" => n.sin(),
                        "cos" => n.cos(),
                        "tan" => n.tan(),
                        "asin" => n.asin(),
                        "acos" => n.acos(),
                        "atan" => n.atan(),
                        "log" => n.ln(),
                        "log10" => n.log10(),
                        "exp" => n.exp(),
                        "sinh" => n.sinh(),
                        "cosh" => n.cosh(),
                        "tanh" => n.tanh(),
                        _ => n,
                    };
                    Ok(float_value(result))
                }
                None => Err(value_arg_err("floating-point number", &args[0])),
            }
        }
        "min" | "max" => {
            if args.is_empty() {
                // 9.1：零实参走 mathfunc 参数不足口径（expr-old-40.3：
                // not enough arguments for math function "min"），
                // 不是 wrong # args。
                return Err(Error::Msg(format!(
                    "not enough arguments for math function \"{name}\""
                )));
            }
            // Tcl returns the winning operand itself, preserving its type:
            // min(2.0, 3) => 2.0 but min(2, 3.0) => 2.
            let mut best = args[0].clone();
            for v in &args[1..] {
                let ord = numeric_cmp(v, &best).ok_or_else(|| {
                    // min/max 实参非数字按 9.1 数值口径报首个失败者
                    // （`min("a", 1)` → expected number but got "a"）。
                    let v_numeric = super::bignum::int_rep(v).is_some()
                        || v.as_float().is_some();
                    value_arg_err("number", if v_numeric { &best } else { v })
                })?;
                let take = if name == "min" {
                    ord == Ordering::Less
                } else {
                    ord == Ordering::Greater
                };
                if take {
                    best = v.clone();
                }
            }
            Ok(best)
        }
        "rand" => {
            let val = ((rand_seed as u64).wrapping_mul(6364136223846793005u64).wrapping_add(1) as f64)
                / (u64::MAX as f64);
            Ok(float_value(val.abs() % 1.0))
        }
        "srand" => {
            require_args(name, 1, args.len())?;
            Ok(Value::empty())
        }
        "isqrt" => {
            if args.len() > 1 {
                return Err(Error::runtime(
                    format!("too many arguments for math function \"{}\"", name),
                    crate::error::ErrorCode::Generic,
                ));
            }
            if args.is_empty() {
                return Err(Error::runtime(
                    format!("not enough arguments for math function \"{}\"", name),
                    crate::error::ErrorCode::Generic,
                ));
            }
            if let Some(rep) = super::bignum::int_rep(&args[0]) {
                let neg = match &rep {
                    super::bignum::IntRep::I64(i) => *i < 0,
                    super::bignum::IntRep::Big(b) => b.is_negative(),
                };
                if neg {
                    return Err(Error::runtime(
                        "square root of negative argument",
                        crate::error::ErrorCode::InvalidOp,
                    ));
                }
                let r = match rep {
                    super::bignum::IntRep::I64(i) => {
                        let mut g = (i as f64).sqrt() as i64;
                        while g > 0 && g.checked_mul(g).is_none_or(|g2| g2 > i) {
                            g -= 1;
                        }
                        while g.checked_add(1).and_then(|g1| g1.checked_mul(g1)).is_some_and(|g2| g2 <= i) {
                            g += 1;
                        }
                        g
                    }
                    super::bignum::IntRep::Big(b) => {
                        // isqrt of a big number stays a bignum
                        // (isqrt(2**2048+1) is 2**1024, expr-47.12).
                        let r = super::bignum::isqrt(&b);
                        return match i64::try_from(r.clone()) {
                            Ok(i) => Ok(Value::from_int(i)),
                            Err(_) => Ok(super::bignum::to_value(
                                super::bignum::IntRep::Big(r),
                            )),
                        };
                    }
                };
                return Ok(Value::from_int(r));
            }
            match args[0].as_float() {
                Some(n) if n >= 0.0 => Ok(Value::from_int((n.sqrt()) as i64)),
                Some(_) => Err(Error::runtime(
                    "square root of negative argument",
                    crate::error::ErrorCode::InvalidOp,
                )),
                None => Err(Error::runtime(
                    crate::value::value_report("number", args[0].as_str()),
                    crate::error::ErrorCode::Generic,
                )),
            }
        }
        // `nan` (integer payload) reaches the mathfunc and fails at the
        // NaN conversion — tclsh's TclExprFloatError (bn7/bn8).
        "nan" => Err(Error::runtime(
            "domain error: argument not in valid range",
            crate::error::ErrorCode::Generic,
        )),
        // C99 全集（TIP 745/521）：acosh/asinh/atanh/cbrt/copysign/dim/
        // erf/erfc/exp2/expm1/fma/gamma/ldexp/lgamma/log1p/log2/logb/
        // nextafter/remainder/signbit/trunc + isinf/isnan/isfinite/
        // isnormal/issubnormal/isunordered — 统一入口见 call_c99_math_func。
        "acosh" | "asinh" | "atanh" | "cbrt" | "copysign" | "dim" | "erf" | "erfc"
        | "exp2" | "expm1" | "fma" | "gamma" | "isfinite" | "isinf" | "isnan"
        | "isnormal" | "issubnormal" | "isunordered" | "ldexp" | "lgamma"
        | "log1p" | "log2" | "logb" | "nextafter" | "remainder" | "signbit"
        | "trunc" => call_c99_math_func(name, args),
        // tclsh dispatches unknown functions to ::tcl::mathfunc::<name>
        // — rtcl has no mathfunc namespace, so the dispatch misses with
        // tclsh's exact message (expr-38.12).
        _ => Err(Error::Msg(format!(
            "invalid command name \"tcl::mathfunc::{}\"",
            name
        ))),
    }
}
