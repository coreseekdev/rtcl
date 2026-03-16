//! VM execution engine — runs compiled [`ByteCode`] within an [`Interp`].
//!
//! The VM maintains a value stack and a program counter, and dispatches
//! opcodes in a loop.  Commands that cannot be compiled (dynamic names, user
//! procs) are handled by falling back to the interpreter's command table.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;
use rtcl_vm::{ByteCode, OpCode};

/// Execute a compiled [`ByteCode`] block inside the given interpreter.
///
/// Returns the final value left on the stack (or empty if the stack is empty
/// after execution).
pub fn execute(interp: &mut Interp, code: &ByteCode) -> Result<Value> {
    let ops = code.ops();
    let mut pc: usize = 0;
    let mut stack: Vec<Value> = Vec::with_capacity(32);

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
            OpCode::Pop => {
                stack.pop();
            }
            OpCode::Dup => {
                if let Some(top) = stack.last() {
                    stack.push(top.clone());
                }
            }

            // ── Variables ───────────────────────────────────────────
            OpCode::LoadLocal(slot) => {
                let name = code.locals().get(*slot as usize).map(|s| s.as_str()).unwrap_or("");
                stack.push(interp.get_var(name).cloned().unwrap_or_else(|_| Value::empty()));
            }
            OpCode::StoreLocal(slot) => {
                let val = stack.last().cloned().unwrap_or_else(Value::empty);
                let name = code.locals().get(*slot as usize).map(|s| s.as_str()).unwrap_or("");
                interp.set_var(name, val)?;
            }
            OpCode::LoadGlobal(idx) => {
                let name = code.get_const(*idx).unwrap_or("");
                let val = interp.get_var(name).cloned()?;
                stack.push(val);
            }
            OpCode::StoreGlobal(idx) => {
                let val = stack.last().cloned().unwrap_or_else(Value::empty);
                let name = code.get_const(*idx).unwrap_or("");
                interp.set_var(name, val)?;
            }
            OpCode::LoadArray(name_idx) => {
                let index_val = stack.pop().unwrap_or_else(Value::empty);
                let name = code.get_const(*name_idx).unwrap_or("");
                let full = format!("{}({})", name, index_val.as_str());
                let val = interp.get_var(&full).cloned()?;
                stack.push(val);
            }
            OpCode::StoreArray(name_idx) => {
                let index_val = stack.pop().unwrap_or_else(Value::empty);
                let val = stack.last().cloned().unwrap_or_else(Value::empty);
                let name = code.get_const(*name_idx).unwrap_or("");
                let full = format!("{}({})", name, index_val.as_str());
                interp.set_var(&full, val)?;
            }
            OpCode::UnsetVar(idx) => {
                let name = code.get_const(*idx).unwrap_or("");
                interp.unset_var(name).ok(); // ignore if not found
            }

            // ── Command invocation ──────────────────────────────────
            OpCode::InvokeBuiltin { cmd_id: _, argc } => {
                // Currently all commands go through InvokeDynamic.
                // InvokeBuiltin can be wired once we assign stable cmd ids.
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", crate::error::ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = invoke_dynamic(interp, &args)?;
                stack.push(result);
            }
            OpCode::InvokeProc { proc_id: _, argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", crate::error::ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = invoke_dynamic(interp, &args)?;
                stack.push(result);
            }
            OpCode::InvokeDynamic { argc } => {
                let n = *argc as usize;
                if stack.len() < n {
                    return Err(Error::runtime("stack underflow", crate::error::ErrorCode::Generic));
                }
                let args: Vec<Value> = stack.drain(stack.len() - n..).collect();
                let result = invoke_dynamic(interp, &args)?;
                stack.push(result);
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
            OpCode::Return => {
                let val = stack.pop().unwrap_or_else(Value::empty);
                return Err(Error::ret(Some(val.as_str().to_string())));
            }
            OpCode::Break => {
                return Err(Error::brk());
            }
            OpCode::Continue => {
                return Err(Error::cont());
            }
            OpCode::EnterScope | OpCode::LeaveScope => {
                // Scope management is handled at the Interp level for now.
            }

            // ── Arithmetic ──────────────────────────────────────────
            OpCode::Add => { binary_arith(&mut stack, |a, b| a + b)?; }
            OpCode::Sub => { binary_arith(&mut stack, |a, b| a - b)?; }
            OpCode::Mul => { binary_arith(&mut stack, |a, b| a * b)?; }
            OpCode::Div => {
                let b = pop_int(&mut stack)?;
                let a = pop_int(&mut stack)?;
                if b == 0 { return Err(Error::DivisionByZero); }
                stack.push(Value::from_int(a / b));
            }
            OpCode::Mod => {
                let b = pop_int(&mut stack)?;
                let a = pop_int(&mut stack)?;
                if b == 0 { return Err(Error::DivisionByZero); }
                stack.push(Value::from_int(a % b));
            }
            OpCode::Pow => {
                let b = pop_int(&mut stack)?;
                let a = pop_int(&mut stack)?;
                stack.push(Value::from_int(a.wrapping_pow(b as u32)));
            }
            OpCode::Neg => {
                let a = pop_int(&mut stack)?;
                stack.push(Value::from_int(-a));
            }

            // ── Comparison ──────────────────────────────────────────
            OpCode::Eq => { binary_cmp(&mut stack, |a, b| a == b)?; }
            OpCode::Ne => { binary_cmp(&mut stack, |a, b| a != b)?; }
            OpCode::Lt => { binary_cmp(&mut stack, |a, b| a < b)?; }
            OpCode::Gt => { binary_cmp(&mut stack, |a, b| a > b)?; }
            OpCode::Le => { binary_cmp(&mut stack, |a, b| a <= b)?; }
            OpCode::Ge => { binary_cmp(&mut stack, |a, b| a >= b)?; }
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
                    return Err(Error::runtime("stack underflow", crate::error::ErrorCode::Generic));
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
                    return Err(Error::runtime("stack underflow", crate::error::ErrorCode::Generic));
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

            // ── Special ─────────────────────────────────────────────
            OpCode::EvalScript => {
                let script = stack.pop().unwrap_or_else(Value::empty);
                let result = interp.eval(script.as_str())?;
                stack.push(result);
            }
            OpCode::EvalExpr => {
                let expr = stack.pop().unwrap_or_else(Value::empty);
                let result = interp.eval_expr(expr.as_str())?;
                stack.push(result);
            }
            OpCode::CatchStart(target) => {
                // Save the PC target for catch error handling.
                // Execute instructions until CatchEnd; on error, jump to target.
                let catch_end = *target as usize;
                let result = execute_catch_block(interp, code, &mut pc, catch_end, &mut stack);
                match result {
                    Ok(()) => {
                        // No error — push code 0
                        stack.push(Value::from_int(0));
                    }
                    Err(e) => {
                        // Error — push error message, then code 1
                        stack.push(Value::from_str(&e.to_string()));
                        stack.push(Value::from_int(1));
                        pc = catch_end;
                    }
                }
            }
            OpCode::CatchEnd => {
                // Handled by CatchStart logic; if we reach here normally, just continue.
            }
            OpCode::Line(_) => {
                // Line annotation — no-op at runtime.
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
            OpCode::Nop => {}
        }
    }

    // Return the value on top of the stack (or empty).
    Ok(stack.pop().unwrap_or_else(Value::empty))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pop one integer from the stack.
fn pop_int(stack: &mut Vec<Value>) -> Result<i64> {
    let val = stack.pop().unwrap_or_else(Value::empty);
    val.as_int().ok_or_else(|| {
        Error::type_mismatch("integer", val.as_str())
    })
}

/// Binary integer arithmetic: pop two, apply `f`, push result.
fn binary_arith(stack: &mut Vec<Value>, f: impl FnOnce(i64, i64) -> i64) -> Result<()> {
    let b = pop_int(stack)?;
    let a = pop_int(stack)?;
    stack.push(Value::from_int(f(a, b)));
    Ok(())
}

/// Binary integer comparison: pop two, apply `f`, push bool.
fn binary_cmp(stack: &mut Vec<Value>, f: impl FnOnce(i64, i64) -> bool) -> Result<()> {
    let b = pop_int(stack)?;
    let a = pop_int(stack)?;
    stack.push(Value::from_bool(f(a, b)));
    Ok(())
}

/// Invoke a command through the interpreter's command table.
fn invoke_dynamic(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Ok(Value::empty());
    }
    let cmd_name = args[0].as_str();

    // Try user-defined procs first
    if let Some(proc_def) = interp.procs.get(cmd_name).cloned() {
        return interp.call_proc(&proc_def, args);
    }

    // Built-in commands
    if let Some(f) = interp.commands.get(cmd_name).cloned() {
        interp.call_depth += 1;
        let result = f(interp, args);
        interp.call_depth -= 1;
        return result;
    }

    Err(Error::invalid_command(cmd_name))
}

/// Execute instructions between a CatchStart and its matching CatchEnd,
/// capturing any errors.
fn execute_catch_block(
    interp: &mut Interp,
    code: &ByteCode,
    pc: &mut usize,
    catch_end: usize,
    stack: &mut Vec<Value>,
) -> Result<()> {
    let ops = code.ops();
    while *pc < ops.len() && *pc < catch_end {
        if matches!(ops[*pc], OpCode::CatchEnd) {
            *pc += 1;
            return Ok(());
        }
        // Execute one instruction by creating a sub-slice bytecode?
        // For simplicity, run the inner code through the main dispatcher
        // up to CatchEnd.
        let op = &ops[*pc];
        *pc += 1;

        // Mini dispatch for catch body — delegate to the interpreter for complex ops.
        match op {
            OpCode::EvalScript => {
                let script = stack.pop().unwrap_or_else(Value::empty);
                let result = interp.eval(script.as_str())?;
                stack.push(result);
            }
            OpCode::EvalExpr => {
                let expr = stack.pop().unwrap_or_else(Value::empty);
                let result = interp.eval_expr(expr.as_str())?;
                stack.push(result);
            }
            OpCode::PushConst(idx) => {
                let s = code.get_const(*idx).unwrap_or("");
                stack.push(Value::from_str(s));
            }
            OpCode::PushEmpty => stack.push(Value::empty()),
            OpCode::PushInt(n) => stack.push(Value::from_int(*n)),
            OpCode::Pop => { stack.pop(); }
            OpCode::Line(_) | OpCode::Nop => {}
            OpCode::InvokeDynamic { argc } => {
                let n = *argc as usize;
                let args: Vec<Value> = stack.drain(stack.len().saturating_sub(n)..).collect();
                let result = invoke_dynamic(interp, &args)?;
                stack.push(result);
            }
            _ => {
                // For other opcodes inside catch, fall back to eval
                // by re-winding pc and running the main execute loop.
                // This is a simplification; a full implementation would
                // handle all opcodes here.
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtcl_vm::Compiler;

    #[test]
    fn test_vm_set_and_read() {
        let mut interp = Interp::new();
        let code = Compiler::compile_script("set x 42").unwrap();
        let result = execute(&mut interp, &code).unwrap();
        // StoreGlobal leaves value on the stack
        assert!(interp.get_var("x").is_ok());
        assert_eq!(interp.get_var("x").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_vm_puts() {
        let mut interp = Interp::new();
        let code = Compiler::compile_script("puts hello").unwrap();
        // Should not error
        let _result = execute(&mut interp, &code).unwrap();
    }

    #[test]
    fn test_vm_while_loop() {
        let mut interp = Interp::new();
        // Set up initial variable
        interp.eval("set i 0").unwrap();
        // while uses EvalScript for the condition and body
        let code = Compiler::compile_script("while {$i < 5} { incr i }").unwrap();
        let _result = execute(&mut interp, &code).unwrap();
        assert_eq!(interp.get_var("i").unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_vm_if_true() {
        let mut interp = Interp::new();
        let code = Compiler::compile_script("if {1} { set x yes }").unwrap();
        let _result = execute(&mut interp, &code).unwrap();
        assert_eq!(interp.get_var("x").unwrap().as_str(), "yes");
    }

    #[test]
    fn test_vm_if_false_else() {
        let mut interp = Interp::new();
        let code = Compiler::compile_script("if {0} { set x yes } else { set x no }").unwrap();
        let _result = execute(&mut interp, &code).unwrap();
        assert_eq!(interp.get_var("x").unwrap().as_str(), "no");
    }

    #[test]
    fn test_vm_for_loop() {
        let mut interp = Interp::new();
        let code = Compiler::compile_script("for {set i 0} {$i < 3} {incr i} { set x $i }").unwrap();
        let _result = execute(&mut interp, &code).unwrap();
        assert_eq!(interp.get_var("i").unwrap().as_int(), Some(3));
    }

    #[test]
    fn test_vm_arithmetic_opcodes() {
        let mut interp = Interp::new();
        // Test that arithmetic opcodes work when the compiler eventually
        // uses them. For now, test via EvalScript fallback.
        let code = Compiler::compile_script("expr {2 + 3}").unwrap();
        let result = execute(&mut interp, &code).unwrap();
        assert_eq!(result.as_int(), Some(5));
    }
}
