//! VM execution engine — runs compiled [`ByteCode`] via a [`VmContext`].
//!
//! The VM maintains:
//! - A value **stack**
//! - A **program counter** (PC)
//! - A **loop stack** for `break`/`continue` resolution
//!
//! Control-flow commands (if/while/for) are executed as native jump+loop
//! opcodes.  Standard and extension commands are dispatched via `call`
//! on the [`VmContext`] (no HashMap lookup).

use crate::context::VmContext;
use crate::error::{Error, ErrorCode, Result};
use crate::value::Value;
use rtcl_parser::{ByteCode, OpCode};

/// Active-loop descriptor pushed by `LoopEnter`, popped by `LoopExit`.
struct ActiveLoop {
    continue_pc: u32,
    break_pc: u32,
}

/// Execute a compiled [`ByteCode`] block using the given [`VmContext`].
///
/// Returns the final value left on the stack (or empty if the stack is
/// empty after execution).
pub fn execute(ctx: &mut dyn VmContext, code: &ByteCode) -> Result<Value> {
    let ops = code.ops();
    let mut pc: usize = 0;
    let mut stack: Vec<Value> = Vec::with_capacity(32);
    let mut loops: Vec<ActiveLoop> = Vec::new();

    while pc < ops.len() {
        let op = &ops[pc];
        pc += 1;

        match op {
            // ── Stack manipulation ──────────────────────────────────
            OpCode::PushConst(idx) => {
                let s = code.get_const(*idx).unwrap_or("");
                stack.push(Value::from_str(s));
            }
            OpCode::PushEmpty => {
                stack.push(Value::empty());
            }
            OpCode::PushInt(n) => {
                stack.push(Value::from_int(*n));
            }
            OpCode::PushFloat(n) => {
                stack.push(Value::from_float(*n));
            }
            OpCode::PushTrue => {
                stack.push(Value::from_bool(true));
            }
            OpCode::PushFalse => {
                stack.push(Value::from_bool(false));
            }
            OpCode::Pop => {
                stack.pop();
            }
            OpCode::Dup => {
                if let Some(top) = stack.last() {
                    stack.push(top.clone());
                }
            }

            // ── Variables ───────────────────────────────────────────
            OpCode::LoadVar(idx) => {
                let name = code.get_const(*idx).unwrap_or("");
                let val = ctx.get_var(name)?;
                stack.push(val);
            }
            OpCode::StoreVar(idx) => {
                let val = stack.last().cloned().unwrap_or_else(Value::empty);
                let name = code.get_const(*idx).unwrap_or("");
                ctx.set_var(name, val)?;
            }
            OpCode::StoreVarPop(idx) => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                let name = code.get_const(*idx).unwrap_or("");
                ctx.set_var(name, val)?;
            }
            OpCode::LoadLocal(slot) => {
                let name = code.locals().get(*slot as usize).map(|s| s.as_str()).unwrap_or("");
                stack.push(ctx.get_var(name).unwrap_or_else(|_| Value::empty()));
            }
            OpCode::StoreLocal(slot) => {
                let val = stack.last().cloned().unwrap_or_else(Value::empty);
                let name = code.locals().get(*slot as usize).map(|s| s.as_str()).unwrap_or("");
                ctx.set_var(name, val)?;
            }
            OpCode::LoadArrayElem(name_idx) => {
                let index_val = stack.pop().unwrap_or_else(Value::empty);
                let name = code.get_const(*name_idx).unwrap_or("");
                let full = format!("{}({})", name, index_val.as_str());
                let val = ctx.get_var(&full)?;
                stack.push(val);
            }
            OpCode::StoreArrayElem(name_idx) => {
                let index_val = stack.pop().unwrap_or_else(Value::empty);
                let val = stack.last().cloned().unwrap_or_else(Value::empty);
                let name = code.get_const(*name_idx).unwrap_or("");
                let full = format!("{}({})", name, index_val.as_str());
                ctx.set_var(&full, val)?;
            }
            OpCode::IncrVar(idx, amount) => {
                let name = code.get_const(*idx).unwrap_or("");
                let new_val = ctx.incr_var(name, *amount)?;
                stack.push(new_val);
            }
            OpCode::AppendVar(idx) => {
                let append_val = stack.pop().unwrap_or_else(Value::empty);
                let name = code.get_const(*idx).unwrap_or("");
                let new_val = ctx.append_var(name, append_val.as_str())?;
                stack.push(new_val);
            }
            OpCode::UnsetVar(idx) => {
                let name = code.get_const(*idx).unwrap_or("");
                ctx.unset_var(name).ok();
            }
            OpCode::VarExists(idx) => {
                let name = code.get_const(*idx).unwrap_or("");
                stack.push(Value::from_bool(ctx.var_exists(name)));
            }

            // ── Scope ───────────────────────────────────────────────
            OpCode::PushFrame(_) | OpCode::PopFrame => {
                // Scope management is handled at the Interp level for now.
            }
            OpCode::UpVar { .. } | OpCode::Global(_) => {
                // Handled at the Interp level for now.
            }

            // ── Control flow ────────────────────────────────────────
            OpCode::Jump(target) => {
                pc = *target as usize;
            }
            OpCode::JumpTrue(target) => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                if val.is_true() {
                    pc = *target as usize;
                }
            }
            OpCode::JumpFalse(target) => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                if !val.is_true() {
                    pc = *target as usize;
                }
            }

            // ── Loop management ─────────────────────────────────────
            OpCode::LoopEnter { cont, brk } => {
                loops.push(ActiveLoop {
                    continue_pc: *cont,
                    break_pc: *brk,
                });
            }
            OpCode::LoopExit => {
                loops.pop();
            }
            OpCode::Break => {
                if let Some(active) = loops.last() {
                    pc = active.break_pc as usize;
                } else {
                    return Err(Error::brk());
                }
            }
            OpCode::Continue => {
                if let Some(active) = loops.last() {
                    pc = active.continue_pc as usize;
                } else {
                    return Err(Error::cont());
                }
            }

            // ── Return / exit ───────────────────────────────────────
            OpCode::Return => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                return Err(Error::ret(Some(val.as_str().to_string())));
            }
            OpCode::ReturnCode(code) => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                return Err(Error::return_with_code(
                    *code,
                    Some(val.as_str().to_string()),
                ));
            }
            OpCode::Exit(code) => {
                return Err(Error::exit(Some(*code)));
            }

            // ── Arithmetic ──────────────────────────────────────────
            OpCode::Add => { numeric_arith(&mut stack, |a, b| a + b, |a, b| a + b)?; }
            OpCode::Sub => { numeric_arith(&mut stack, |a, b| a - b, |a, b| a - b)?; }
            OpCode::Mul => { numeric_arith(&mut stack, |a, b| a * b, |a, b| a * b)?; }
            OpCode::Div => {
                let (a, b) = pop_numeric_pair(&mut stack)?;
                match (a, b) {
                    (Num::Int(a), Num::Int(b)) => {
                        if b == 0 { return Err(Error::DivisionByZero); }
                        stack.push(Value::from_int(a / b));
                    }
                    _ => {
                        let (af, bf) = (a.as_f64(), b.as_f64());
                        if bf == 0.0 { return Err(Error::DivisionByZero); }
                        stack.push(float_or_int(af / bf));
                    }
                }
            }
            OpCode::Mod => {
                let b = pop_int(&mut stack)?;
                let a = pop_int(&mut stack)?;
                if b == 0 { return Err(Error::DivisionByZero); }
                stack.push(Value::from_int(a % b));
            }
            OpCode::Pow => {
                let (a, b) = pop_numeric_pair(&mut stack)?;
                match (a, b) {
                    (Num::Int(a), Num::Int(b)) => {
                        if b >= 0 {
                            stack.push(Value::from_int(a.wrapping_pow(b as u32)));
                        } else {
                            stack.push(float_or_int((a as f64).powf(b as f64)));
                        }
                    }
                    _ => {
                        stack.push(float_or_int(a.as_f64().powf(b.as_f64())));
                    }
                }
            }
            OpCode::Neg => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                if let Some(n) = val.as_int() {
                    stack.push(Value::from_int(-n));
                } else if let Some(f) = val.as_float() {
                    stack.push(float_or_int(-f));
                } else {
                    return Err(Error::type_mismatch("number", val.as_str()));
                }
            }

            // ── Comparison ──────────────────────────────────────────
            OpCode::Eq => { numeric_cmp(&mut stack, |a, b| a == b, |a, b| (a - b).abs() < f64::EPSILON)?; }
            OpCode::Ne => { numeric_cmp(&mut stack, |a, b| a != b, |a, b| (a - b).abs() >= f64::EPSILON)?; }
            OpCode::Lt => { numeric_cmp(&mut stack, |a, b| a < b, |a, b| a < b)?; }
            OpCode::Gt => { numeric_cmp(&mut stack, |a, b| a > b, |a, b| a > b)?; }
            OpCode::Le => { numeric_cmp(&mut stack, |a, b| a <= b, |a, b| a <= b)?; }
            OpCode::Ge => { numeric_cmp(&mut stack, |a, b| a >= b, |a, b| a >= b)?; }
            OpCode::StrEq => {
                let b = stack.pop().unwrap_or_else(Value::empty);
                let a = stack.pop().unwrap_or_else(Value::empty);
                stack.push(Value::from_bool(a.as_str() == b.as_str()));
            }
            OpCode::StrNe => {
                let b = stack.pop().unwrap_or_else(Value::empty);
                let a = stack.pop().unwrap_or_else(Value::empty);
                stack.push(Value::from_bool(a.as_str() != b.as_str()));
            }

            // ── Logical ─────────────────────────────────────────────
            OpCode::And => {
                let b = stack.pop().unwrap_or_else(Value::empty);
                let a = stack.pop().unwrap_or_else(Value::empty);
                stack.push(Value::from_bool(a.is_true() && b.is_true()));
            }
            OpCode::Or => {
                let b = stack.pop().unwrap_or_else(Value::empty);
                let a = stack.pop().unwrap_or_else(Value::empty);
                stack.push(Value::from_bool(a.is_true() || b.is_true()));
            }
            OpCode::Not => {
                let a = stack.pop().unwrap_or_else(Value::empty);
                stack.push(Value::from_bool(!a.is_true()));
            }

            // ── Bitwise ─────────────────────────────────────────────
            OpCode::BitAnd => { binary_arith(&mut stack, |a, b| a & b)?; }
            OpCode::BitOr  => { binary_arith(&mut stack, |a, b| a | b)?; }
            OpCode::BitXor => { binary_arith(&mut stack, |a, b| a ^ b)?; }
            OpCode::BitNot => {
                let a = pop_int(&mut stack)?;
                stack.push(Value::from_int(!a));
            }
            OpCode::Shl => { binary_arith(&mut stack, |a, b| a << (b & 63))?; }
            OpCode::Shr => { binary_arith(&mut stack, |a, b| a >> (b & 63))?; }

            // ── String / List ───────────────────────────────────────
            OpCode::Concat(n) => {
                let n = *n as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let parts: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let mut result = String::new();
                for p in &parts {
                    result.push_str(p.as_str());
                }
                stack.push(Value::from_str(&result));
            }
            OpCode::MakeList(n) => {
                let n = *n as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let items: Vec<Value> = stack.drain(stack.len() - n..).collect();
                stack.push(Value::from_list(&items));
            }
            OpCode::ListAppend => {
                let elem = stack.pop().unwrap_or_else(Value::empty);
                let list_val = stack.pop().unwrap_or_else(Value::empty);
                let mut items = list_val.as_list().unwrap_or_default();
                items.push(elem);
                stack.push(Value::from_list(&items));
            }
            OpCode::ListIndex => {
                let index = stack.pop().unwrap_or_else(Value::empty);
                let list_val = stack.pop().unwrap_or_else(Value::empty);
                let items = list_val.as_list().unwrap_or_default();
                let idx = index.as_int().unwrap_or(-1) as usize;
                stack.push(items.get(idx).cloned().unwrap_or_else(Value::empty));
            }
            OpCode::ListLength => {
                let list_val = stack.pop().unwrap_or_else(Value::empty);
                let len = list_val.as_list().map(|l| l.len()).unwrap_or(0);
                stack.push(Value::from_int(len as i64));
            }
            OpCode::StrLen => {
                let s = stack.pop().unwrap_or_else(Value::empty);
                stack.push(Value::from_int(s.as_str().len() as i64));
            }
            OpCode::StrIndex => {
                let index = stack.pop().unwrap_or_else(Value::empty);
                let s = stack.pop().unwrap_or_else(Value::empty);
                let idx = index.as_int().unwrap_or(-1) as usize;
                let str_val = s.as_str();
                if idx < str_val.len() {
                    stack.push(Value::from_str(&str_val[idx..idx + 1]));
                } else {
                    stack.push(Value::empty());
                }
            }
            OpCode::ExpandList => {
                let list_val = stack.pop().unwrap_or_else(Value::empty);
                if let Some(items) = list_val.as_list() {
                    for item in items {
                        stack.push(item);
                    }
                } else {
                    stack.push(list_val);
                }
            }

            // ── Command calls ───────────────────────────────────────
            OpCode::Call { cmd_id, argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = ctx.call(*cmd_id, &args)?;
                stack.push(result);
            }
            OpCode::CallExpand { cmd_id, argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = ctx.call(*cmd_id, &args)?;
                stack.push(result);
            }
            OpCode::DynCall { argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = ctx.invoke_command(&args)?;
                stack.push(result);
            }
            OpCode::DynCallExpand { argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = ctx.invoke_command(&args)?;
                stack.push(result);
            }
            OpCode::CallProc { proc_id: _, argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = ctx.invoke_command(&args)?;
                stack.push(result);
            }
            OpCode::TailCallProc { proc_id: _, argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = ctx.invoke_command(&args)?;
                stack.push(result);
            }

            // ── Special ─────────────────────────────────────────────
            OpCode::EvalScript => {
                let script = stack.pop().unwrap_or_else(Value::empty);
                match ctx.eval_script(script.as_str()) {
                    Ok(val) => stack.push(val),
                    Err(e) if e.is_break() => {
                        // Break from within a dynamically evaluated script
                        if let Some(active) = loops.last() {
                            pc = active.break_pc as usize;
                        } else {
                            return Err(e);
                        }
                    }
                    Err(e) if e.is_continue() => {
                        if let Some(active) = loops.last() {
                            pc = active.continue_pc as usize;
                        } else {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            OpCode::EvalExpr => {
                let expr = stack.pop().unwrap_or_else(Value::empty);
                let result = ctx.eval_expr(expr.as_str())?;
                stack.push(result);
            }
            OpCode::CatchStart(target) => {
                let catch_end = *target as usize;
                let result = execute_catch_block(ctx, code, &mut pc, catch_end, &mut stack, &mut loops);
                match result {
                    Ok(()) => {
                        stack.push(Value::from_int(0));
                    }
                    Err(e) => {
                        stack.push(Value::from_str(&e.to_string()));
                        stack.push(Value::from_int(1));
                        pc = catch_end;
                    }
                }
            }
            OpCode::CatchEnd => {
                // Handled by CatchStart logic.
            }

            // ── Debug ───────────────────────────────────────────────
            OpCode::Line(_) => {}
            OpCode::Nop => {}
        }
    }

    Ok(stack.pop().unwrap_or_else(Value::empty))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A numeric value — either integer or float.
#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    fn as_f64(self) -> f64 {
        match self {
            Num::Int(n) => n as f64,
            Num::Float(n) => n,
        }
    }
}

fn pop_num(stack: &mut Vec<Value>) -> Result<Num> {
    let val = stack.pop().unwrap_or_else(Value::empty);
    if let Some(n) = val.as_int() {
        Ok(Num::Int(n))
    } else if let Some(f) = val.as_float() {
        Ok(Num::Float(f))
    } else {
        Err(Error::type_mismatch("number", val.as_str()))
    }
}

fn pop_numeric_pair(stack: &mut Vec<Value>) -> Result<(Num, Num)> {
    let b = pop_num(stack)?;
    let a = pop_num(stack)?;
    Ok((a, b))
}

fn pop_int(stack: &mut Vec<Value>) -> Result<i64> {
    let val = stack.pop().unwrap_or_else(Value::empty);
    val.as_int().ok_or_else(|| {
        Error::type_mismatch("integer", val.as_str())
    })
}

/// Float-or-int: return int if result is a whole number, float otherwise.
fn float_or_int(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
        Value::from_int(f as i64)
    } else {
        Value::from_float(f)
    }
}

/// Numeric arithmetic: integer fast path, float fallback.
fn numeric_arith(
    stack: &mut Vec<Value>,
    int_op: impl FnOnce(i64, i64) -> i64,
    float_op: impl FnOnce(f64, f64) -> f64,
) -> Result<()> {
    let (a, b) = pop_numeric_pair(stack)?;
    match (a, b) {
        (Num::Int(a), Num::Int(b)) => stack.push(Value::from_int(int_op(a, b))),
        _ => stack.push(float_or_int(float_op(a.as_f64(), b.as_f64()))),
    }
    Ok(())
}

/// Numeric comparison: integer fast path, float fallback.
fn numeric_cmp(
    stack: &mut Vec<Value>,
    int_op: impl FnOnce(i64, i64) -> bool,
    float_op: impl FnOnce(f64, f64) -> bool,
) -> Result<()> {
    let (a, b) = pop_numeric_pair(stack)?;
    let result = match (a, b) {
        (Num::Int(a), Num::Int(b)) => int_op(a, b),
        _ => float_op(a.as_f64(), b.as_f64()),
    };
    stack.push(Value::from_bool(result));
    Ok(())
}

fn binary_arith(stack: &mut Vec<Value>, f: impl FnOnce(i64, i64) -> i64) -> Result<()> {
    let b = pop_int(stack)?;
    let a = pop_int(stack)?;
    stack.push(Value::from_int(f(a, b)));
    Ok(())
}

/// Execute instructions between a CatchStart and its matching CatchEnd.
fn execute_catch_block(
    ctx: &mut dyn VmContext,
    code: &ByteCode,
    pc: &mut usize,
    catch_end: usize,
    stack: &mut Vec<Value>,
    loops: &mut Vec<ActiveLoop>,
) -> Result<()> {
    let ops = code.ops();
    while *pc < ops.len() && *pc < catch_end {
        if matches!(ops[*pc], OpCode::CatchEnd) {
            *pc += 1;
            return Ok(());
        }
        let op = &ops[*pc];
        *pc += 1;

        match op {
            OpCode::EvalScript => {
                let script = stack.pop().unwrap_or_else(Value::empty);
                let result = ctx.eval_script(script.as_str())?;
                stack.push(result);
            }
            OpCode::EvalExpr => {
                let expr = stack.pop().unwrap_or_else(Value::empty);
                let result = ctx.eval_expr(expr.as_str())?;
                stack.push(result);
            }
            OpCode::PushConst(idx) => {
                let s = code.get_const(*idx).unwrap_or("");
                stack.push(Value::from_str(s));
            }
            OpCode::PushEmpty => stack.push(Value::empty()),
            OpCode::PushInt(n) => stack.push(Value::from_int(*n)),
            OpCode::PushFloat(n) => stack.push(Value::from_float(*n)),
            OpCode::Pop => { stack.pop(); }
            OpCode::Line(_) | OpCode::Nop => {}
            OpCode::DynCall { argc } => {
                let n = *argc as usize;
                let args: Vec<Value> = stack.drain(stack.len().saturating_sub(n)..).collect();
                let result = ctx.invoke_command(&args)?;
                stack.push(result);
            }
            OpCode::Call { cmd_id, argc } | OpCode::CallExpand { cmd_id, argc } => {
                let n = *argc as usize;
                let args: Vec<Value> = stack.drain(stack.len().saturating_sub(n)..).collect();
                let result = ctx.call(*cmd_id, &args)?;
                stack.push(result);
            }
            OpCode::LoopEnter { cont, brk } => {
                loops.push(ActiveLoop {
                    continue_pc: *cont,
                    break_pc: *brk,
                });
            }
            OpCode::LoopExit => { loops.pop(); }
            _ => {
                // For other opcodes inside catch, simplified handling.
            }
        }
    }
    Ok(())
}
