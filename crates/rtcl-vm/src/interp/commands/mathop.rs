//! `::tcl::mathop::*` — the expr-operator command set (tclsh 8.6.17
//! registers 23 of them; rtcl previously exposed only `+`).  One generic
//! handler dispatches on the operator; semantics come from
//! [`super::super::super::types::expr_ops`] so commands, the expr parser
//! and the bytecode executor share one implementation.
//!
//! Arity quirks pinned against tclsh 8.6.17:
//! - folds (`+ * & | ^ **`) accept zero arguments and return the identity
//!   (`+` → 0, `*`/`**` → 1, `&` → -1, `|`/`^` → 0);
//! - `-` needs ≥1 (unary negates), `/` needs ≥1 (unary reciprocal);
//! - comparisons accept n ≥ 1 (single operand is trivially true);
//! - `~`/`!` take exactly one operand, `%`/`<<`/`>>` exactly two, and
//!   `in`/`ni` exactly `value list`.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::types::expr_ops;
use crate::value::Value;

pub fn cmd_mathop(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // args[0] is the invoked word (tcl::mathop::<<) — the operator is its
    // last path segment.
    let op = args[0].as_str().rsplit("::").next().unwrap_or("");
    let rest = &args[1..];
    match op {
        "+" | "*" | "&" | "|" | "^" | "**" => fold(interp, op, rest),
        "-" => {
            if rest.is_empty() {
                return Err(arity(op, "value ?value ...?"));
            }
            let mut acc = rest[0].clone();
            if rest.len() == 1 {
                numeric_binop_str(&Value::from_int(0), &acc, '-', op)
            } else {
                for v in &rest[1..] {
                    acc = numeric_binop_str(&acc, v, '-', op)?;
                }
                Ok(acc)
            }
        }
        "/" => {
            if rest.is_empty() {
                return Err(arity(op, "value ?value ...?"));
            }
            let mut acc = rest[0].clone();
            if rest.len() == 1 {
                // Unary / is the reciprocal: real division, not expr's
                // integer floor division (tclsh: `/ 2` = 0.5).
                numeric_binop_str(&Value::from_float(1.0), &acc, '/', op)
            } else {
                for v in &rest[1..] {
                    acc = numeric_binop_str(&acc, v, '/', op)?;
                }
                Ok(acc)
            }
        }
        "%" => {
            if rest.len() != 2 {
                return Err(arity(op, "integer integer"));
            }
            expr_ops::int_mod(&rest[0], &rest[1])
        }
        "<<" | ">>" => {
            if rest.len() != 2 {
                return Err(arity(op, "integer integer"));
            }
            expr_ops::int_shift(&rest[0], &rest[1], op == "<<")
        }
        "<" | "<=" | ">" | ">=" | "==" | "!=" => chain(rest, |a, b| {
            let cc = match op {
                "<" => 0,
                "<=" => 1,
                ">" => 2,
                ">=" => 3,
                "==" => 4,
                _ => 5,
            };
            Ok(Value::from_bool(expr_ops::cmp_bool(a, b, cc)))
        }),
        "eq" | "ne" => chain(rest, |a, b| {
            let r = if op == "eq" {
                expr_ops::op_str_eq(a, b)
            } else {
                expr_ops::op_str_ne(a, b)
            };
            Ok(r)
        }),
        "~" => {
            if rest.len() != 1 {
                return Err(arity(op, "integer"));
            }
            bitwise_not(&rest[0])
        }
        "!" => {
            if rest.len() != 1 {
                return Err(arity(op, "boolean"));
            }
            expr_ops::op_not(&rest[0])
        }
        "in" | "ni" => {
            if rest.len() != 2 {
                return Err(arity(op, "value list"));
            }
            let list = rest[1].as_list().unwrap_or_default();
            let found = list.iter().any(|item| item.as_str() == rest[0].as_str());
            Ok(Value::from_bool(found != (op == "ni")))
        }
        _ => Err(Error::runtime(
            format!("unknown math operator \"{}\"", op),
            ErrorCode::Generic,
        )),
    }
}

fn arity(op: &str, sig: &str) -> Error {
    Error::wrong_args_msg(format!(
        "wrong # args: should be \"tcl::mathop::{} {}\"",
        op, sig
    ))
}

fn fold(interp: &mut Interp, op: &str, rest: &[Value]) -> Result<Value> {
    let _ = interp;
    let mut acc = match op {
        "+" => Value::from_int(0),
        "*" | "**" => Value::from_int(1),
        "&" => Value::from_int(-1),
        "|" | "^" => Value::from_int(0),
        _ => unreachable!(),
    };
    if rest.is_empty() {
        return Ok(acc);
    }
    if op == "**" {
        // ** folds RIGHT-associative (tclsh: `** 2 3 2` = 2**(3**2) = 512).
        let mut acc = rest[rest.len() - 1].clone();
        for v in rest[..rest.len() - 1].iter().rev() {
            acc = expr_ops::op_pow(v.clone(), acc)?;
        }
        return Ok(acc);
    }
    let ch = op.chars().next().unwrap();
    if op == "&" || op == "|" || op == "^" {
        for v in rest {
            acc = expr_ops::int_bitop(&acc, v, ch)?;
        }
        return Ok(acc);
    }
    for v in rest {
        acc = numeric_binop_str(&acc, v, ch, op)?;
    }
    Ok(acc)
}

fn chain(rest: &[Value], f: impl Fn(&Value, &Value) -> Result<Value>) -> Result<Value> {
    if rest.is_empty() {
        return Ok(Value::from_int(1));
    }
    for w in rest.windows(2) {
        let ok = f(&w[0], &w[1])?;
        if ok.as_str() == "0" {
            return Ok(Value::from_int(0));
        }
    }
    Ok(Value::from_int(1))
}

fn numeric_binop_str(a: &Value, b: &Value, ch: char, op: &str) -> Result<Value> {
    let _ = op;
    expr_ops::numeric_binop(a, b, ch)
}

fn bitwise_not(v: &Value) -> Result<Value> {
    // expr's ~ on a non-integer (incl. float): "can't use ... operand of "~"".
    if let Some(i) = v.as_int() {
        return Ok(Value::from_int(!i));
    }
    Err(Error::runtime(
        format!(
            "can't use {} string as operand of \"~\"",
            if v.as_str().is_empty() { "empty" } else { "non-numeric" }
        ),
        ErrorCode::InvalidOp,
    ))
}
