//! 9.1 新增的浮点/整数分解命令：`fpclassify`（TIP 521）、`divmod`/
//! `frexp`/`modf`/`remquo`（TIP 745）。语义与文案按 9.1.0 源码
//! （generic/tclBasic.c 的 FloatClassifyObjCmd / DivModObjCmd /
//! FracExpObjCmd / ModFObjCmd / RemQuoObjCmd）与 oracle 逐字节钉形：
//!
//! - `divmod` 的宽整路径保留上游 quirk——商为负时再减一的判断是
//!   `(q*y) != y`（8.6 是 `!= x`），因此 `divmod -6 1` 是 `{-7 1}`、
//!   `divmod -100 10` 是 `{-11 10}`，与 `/`、`%` 不一致；bignum 路径
//!   则是真正的 floor 除法。`MIN / -1` 上游直接 SIGFPE，这里用精确
//!   整数运算兜住不崩。
//! - `remquo` 域错误时上游返回的 quo 是未初始化栈垃圾（不可钉形），
//!   rtcl 返回 0；remainder 与上游同为带符号 NaN（印作 "-NaN"）。

use super::super::Interp;
use crate::error::{Error, ErrorCode, Result};
use crate::types::bignum::{int_rep, to_value, IntRep};
use crate::types::c99::sys;
use crate::types::expr_funcs::{f64_arg, fp_classify_word};
use crate::value::Value;

fn wrong_args(cmd: &str, usage: &str) -> Error {
    Error::wrong_args_msg(format!("wrong # args: should be \"{cmd} {usage}\""))
}

fn expected_number(v: &Value) -> Error {
    Error::Msg(format!("expected number but got \"{}\"", v.as_str()))
}

/// `fpclassify value` — 返回 zero/subnormal/normal/infinite/nan。
pub fn cmd_fpclassify(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(wrong_args("fpclassify", "floatValue"));
    }
    Ok(Value::from_str(fp_classify_word(&args[1])?))
}

/// `frexp floatValue` — {fraction exponent}（frexp.n）。
pub fn cmd_frexp(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(wrong_args("frexp", "floatValue"));
    }
    let d = f64_arg(&args[1])?;
    let (frac, exp) = sys::frexp(d);
    Ok(Value::from_list(&[
        crate::types::expr_funcs::float_value(frac),
        Value::from_int(exp as i64),
    ]))
}

/// `modf floatValue` — {integralPart fractionalPart}（modf.n）。
pub fn cmd_modf(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(wrong_args("modf", "floatValue"));
    }
    let d = f64_arg(&args[1])?;
    let (int_part, frac_part) = sys::modf(d);
    Ok(Value::from_list(&[
        crate::types::expr_funcs::float_value(int_part),
        crate::types::expr_funcs::float_value(frac_part),
    ]))
}

/// `remquo dividend divisor` — {remainder quotient-bits}（remquo.n）。
pub fn cmd_remquo(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(wrong_args("remquo", "dividend divisor"));
    }
    let d1 = f64_arg(&args[1])?;
    let d2 = f64_arg(&args[2])?;
    let (mut rem, mut quo) = sys::remquo(d1, d2);
    if rem.is_nan() {
        // 上游域错误的 NaN 带符号位（印作 "-NaN"）；quo 是垃圾，给 0。
        rem = f64::from_bits(0xfff8_0000_0000_0000u64);
        quo = 0;
    }
    Ok(Value::from_list(&[
        crate::types::expr_funcs::float_value(rem),
        Value::from_int(quo as i64),
    ]))
}

/// `divmod dividend divisor` — {quotient remainder}（divmod.n）。
pub fn cmd_divmod(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(wrong_args("divmod", "dividend divisor"));
    }
    let (x, y) = (&args[1], &args[2]);
    // GetNumberFromObj 口径：先看是不是数字（浮点也算），再要求整数。
    let number = |v: &Value| int_rep(v).is_some() || v.as_float().is_some();
    if !number(x) {
        return Err(expected_number(x));
    }
    let rep_x = int_rep(x);
    if rep_x.is_none() {
        return Err(Error::runtime(
            "dividend must be an integer",
            ErrorCode::Generic,
        ));
    }
    if !number(y) {
        return Err(expected_number(y));
    }
    let rep_y = int_rep(y);
    if rep_y.is_none() {
        return Err(Error::runtime(
            "divisor must be an integer",
            ErrorCode::Generic,
        ));
    }

    // bignum 参与即走大数路径（9.1 的 typeX/typeY BIG 分支）：真正的
    // floor 除法——商向 -inf 取整，余数与除数同号。
    if matches!(rep_x, Some(IntRep::Big(_))) || matches!(rep_y, Some(IntRep::Big(_))) {
        use num_integer::Integer;
        let bx = rep_x.as_ref().unwrap().to_big();
        let by = rep_y.as_ref().unwrap().to_big();
        if by == num_bigint::BigInt::from(0i8) {
            return Err(divide_by_zero());
        }
        let q = Integer::div_floor(&bx, &by);
        let r = Integer::mod_floor(&bx, &by);
        return Ok(Value::from_list(&[to_value(IntRep::Big(q)), big_value(r)]));
    }

    // 宽整路径：C 截断商 + 上游 quirk 调整（(q*y) != y），余数按
    // Tcl_WideUInt 回绕算术计算。
    let (xi, yi) = (
        match rep_x {
            Some(IntRep::I64(i)) => i,
            _ => unreachable!("i64 path pre-checked"),
        },
        match rep_y {
            Some(IntRep::I64(i)) => i,
            _ => unreachable!("i64 path pre-checked"),
        },
    );
    if yi == 0 {
        return Err(divide_by_zero());
    }
    // MIN / -1 上游 SIGFPE（有符号溢出）；这里 wrapping 兜住不崩
    // （该输入在 oracle 上无钉形——crash-gate 禁复刻崩溃）。
    let mut q = xi.wrapping_div(yi);
    let adjust = q < 0 || (q == 0 && ((xi < 0 && yi > 0) || (xi > 0 && yi < 0)));
    if adjust && q.wrapping_mul(yi) != yi {
        q = q.wrapping_sub(1);
    }
    let r = (xi as u64).wrapping_sub((yi as u64).wrapping_mul(q as u64)) as i64;
    Ok(Value::from_list(&[
        Value::from_int(q),
        Value::from_int(r),
    ]))
}

fn big_value(b: num_bigint::BigInt) -> Value {
    match i64::try_from(b.clone()) {
        Ok(i) => Value::from_int(i),
        Err(_) => to_value(IntRep::Big(b)),
    }
}

fn divide_by_zero() -> Error {
    Error::runtime("divide by zero", ErrorCode::InvalidOp)
}
