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
                return Err(arity_invoked(&args[0], "value ?value ...?"));
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
                return Err(arity_invoked(&args[0], "value ?value ...?"));
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
                return Err(arity_invoked(&args[0], "integer integer"));
            }
            expr_ops::int_mod(&rest[0], &rest[1])
        }
        "<<" | ">>" => {
            if rest.len() != 2 {
                return Err(arity_invoked(&args[0], "integer shift"));
            }
            expr_ops::int_shift(&rest[0], &rest[1], op == "<<")
        }
        "<" | "<=" | ">" | ">=" | "==" => chain(rest, |a, b| {
            let cc = match op {
                "<" => 0,
                "<=" => 1,
                ">" => 2,
                ">=" => 3,
                _ => 4,
            };
            Ok(Value::from_bool(expr_ops::cmp_bool(a, b, cc)))
        }),
        // 9.1 钉形（tclBasic.c mathOpCmds）：`!=` 与 `ne` 是
        // TclSingleOpCmd numArgs=2——严格两参，不是排序链；同表的
        // `< <= > >= == eq lt le gt ge` 才是 TclSortingOpCmd nary
        // （0 参空真 1）。实测 `::tcl::mathop::!= 1` 与 `ne 1 2 3` 报
        // wrong # args "value value"。
        "!=" | "ne" => {
            if rest.len() != 2 {
                return Err(arity_invoked(&args[0], "value value"));
            }
            if op == "!=" {
                Ok(Value::from_bool(!expr_ops::cmp_bool(&rest[0], &rest[1], 4)))
            } else {
                Ok(expr_ops::op_str_ne(&rest[0], &rest[1]))
            }
        }
        // lt/le/gt/ge（9.0 新增，TIP 461）：纯 UNICODE 字符串比较链
        // —— `::tcl::mathop::lt 2 10` 是 0（"2">"10" 的字典序），与
        // 数值比较运算符 `<` 系分界。少于两个实参恒真（9.1 手册）。
        "lt" | "le" | "gt" | "ge" => chain(rest, |a, b| {
            let ord = a.as_str().cmp(b.as_str());
            let ok = match op {
                "lt" => ord == core::cmp::Ordering::Less,
                "le" => ord != core::cmp::Ordering::Greater,
                "gt" => ord == core::cmp::Ordering::Greater,
                _ => ord != core::cmp::Ordering::Less,
            };
            Ok(Value::from_bool(ok))
        }),
        "eq" => chain(rest, |a, b| Ok(expr_ops::op_str_eq(a, b))),
        "~" => {
            if rest.len() != 1 {
                return Err(arity_invoked(&args[0], "integer"));
            }
            bitwise_not(&rest[0])
        }
        "!" => {
            if rest.len() != 1 {
                return Err(arity_invoked(&args[0], "boolean"));
            }
            expr_ops::op_not(&rest[0])
        }
        "in" | "ni" => {
            if rest.len() != 2 {
                return Err(arity_invoked(&args[0], "value list"));
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

fn arity_invoked(invoked: &Value, sig: &str) -> Error {
    // 9.1 钉形：usage 回显调用点的命令字面（tclsh 的 Tcl_WrongNumArgs
    // 用 objv[0]）——`::tcl::mathop::in` 报 "::tcl::mathop::in value
    // list"，`tcl::mathop::in` 报 "tcl::mathop::in value list"。
    Error::wrong_args_msg(format!(
        "wrong # args: should be \"{} {}\"",
        invoked.as_str(),
        sig
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
    // C16（9.1 钉形）：折叠从**第一个操作数**起步（0/1 等单位元不作为
    // 左操作数参与）——首个非法操作数按 9.1 报 left operand
    // (`+ abc 1` → cannot use non-numeric string "abc" as left operand
    // of "+"; 单操作数 `+ abc` 同为 left)。
    if op == "+" || op == "*" {
        let identity = if op == "+" { Value::from_int(0) } else { Value::from_int(1) };
        let mut acc = rest[0].clone();
        if rest.len() == 1 {
            // 单操作数仍要过数值校验（9.1：`+ abc` 报 left operand 错）
            return numeric_binop_str(&acc, &identity, ch, op);
        }
        for v in &rest[1..] {
            acc = numeric_binop_str(&acc, v, ch, op)?;
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
