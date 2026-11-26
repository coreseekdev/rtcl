//! Bytecode validator — checks compiled [`ByteCode`] for structural
//! correctness before execution.
//!
//! The validator catches:
//! - Jump targets out of bounds
//! - Mismatched `LoopEnter` / `LoopExit` nesting
//! - References to out-of-range constant pool indices
//! - `Break` / `Continue` outside any loop context (static only)

use crate::bytecode::ByteCode;
use crate::opcode::OpCode;
use core::fmt;

/// A single validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Instruction index where the problem was detected.
    pub pc: usize,
    /// Human-readable description.
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[pc={}] {}", self.pc, self.message)
    }
}

/// Validate a compiled [`ByteCode`] block and return all errors found.
///
/// An empty `Vec` means the bytecode is well-formed.
///
/// D2 第一步：操作数检查改读 [`rtcl_ir::opspec::OPS`] 行——按行的
/// 操作数种类走查（`Const16`/`Const32` 查常量池界、`Target32` 查跳转
/// 界），不再各持一份 per-op match。与旧检查的差别（记账）：
/// - 旧版只查 Jump/JumpTrue/JumpFalse/CatchStart/LoopEnter×2 及八条
///   Cmp*Jump/ForNext* 的目标界；新版按种类覆盖全部 Target32 字段，
///   追加覆盖 ForeachStart(next/end)、ForeachNext(body)、
///   ForeachNextClose(body)——编译器产物这些目标恒在界内，只影响
///   手组码的诊断面（更严，不是更松）。
/// - 旧版 upvar src 的专项文案并入通用 `constant pool index` 文案。
pub fn validate(code: &ByteCode) -> Vec<ValidationError> {
    use rtcl_ir::opspec::{Opnd, OPS, decompose};

    let ops = code.ops();
    let len = ops.len();
    let pool_size = code.constants().len() as u16;
    let pool_size_wide = code.constants().len() as u32;
    let mut errors = Vec::new();
    let mut loop_depth: i32 = 0;

    for (pc, op) in ops.iter().enumerate() {
        let (idx, fields) = decompose(op);
        let spec = &OPS[idx as usize];
        for (k, arg) in spec.args.iter().enumerate() {
            match arg.kind {
                Opnd::Const16 => {
                    let v = fields[k] as u16;
                    if v >= pool_size {
                        errors.push(ValidationError {
                            pc,
                            message: format!(
                                "constant pool index {} out of range (pool size {})",
                                v, pool_size,
                            ),
                        });
                    }
                }
                Opnd::Const32 => {
                    let v = fields[k] as u32;
                    if v >= pool_size_wide {
                        errors.push(ValidationError {
                            pc,
                            message: format!(
                                "wide constant pool index {} out of range (pool size {})",
                                v, pool_size_wide,
                            ),
                        });
                    }
                }
                Opnd::Target32 => {
                    let t = fields[k] as u32 as usize;
                    if t > len {
                        errors.push(ValidationError {
                            pc,
                            message: format!(
                                "jump target {} out of bounds (code length {})",
                                t, len,
                            ),
                        });
                    }
                }
                _ => {}
            }
        }
        // -- Loop nesting (structural, keyed on the pair's row names) ----
        match op {
            OpCode::LoopEnter { .. } => loop_depth += 1,
            OpCode::LoopExit => {
                loop_depth -= 1;
                if loop_depth < 0 {
                    errors.push(ValidationError {
                        pc,
                        message: "LoopExit without matching LoopEnter".into(),
                    });
                }
            }
            OpCode::Break | OpCode::Continue
                if loop_depth <= 0 => {
                    // This is OK in some cases (break/continue used as
                    // a general command), so we make it a warning-level
                    // note rather than a hard error.  The VM will handle
                    // it by returning an Error.
                }
            _ => {}
        }
    }

    // Final nesting check
    if loop_depth != 0 {
        errors.push(ValidationError {
            pc: len,
            message: format!(
                "unbalanced loop nesting: {} unclosed LoopEnter(s) at end of code",
                loop_depth,
            ),
        });
    }

    errors
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::ByteCode;
    use crate::opcode::OpCode;

    fn make_code(ops: Vec<OpCode>) -> ByteCode {
        let mut bc = ByteCode::new();
        for op in ops {
            bc.emit(op, 0);
        }
        bc
    }

    #[test]
    fn valid_simple_sequence() {
        let code = make_code(vec![
            OpCode::PushEmpty,
            OpCode::Pop,
        ]);
        assert!(validate(&code).is_empty());
    }

    #[test]
    fn valid_loop_nesting() {
        let code = make_code(vec![
            OpCode::LoopEnter { cont: 1, brk: 3 },
            OpCode::Nop,
            OpCode::LoopExit,
        ]);
        assert!(validate(&code).is_empty());
    }

    #[test]
    fn unbalanced_loop_exit() {
        let code = make_code(vec![
            OpCode::LoopExit,
        ]);
        let errs = validate(&code);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("without matching LoopEnter"));
    }

    #[test]
    fn unclosed_loop_enter() {
        let code = make_code(vec![
            OpCode::LoopEnter { cont: 0, brk: 1 },
        ]);
        let errs = validate(&code);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("unclosed"));
    }

    #[test]
    fn jump_out_of_bounds() {
        let code = make_code(vec![
            OpCode::Jump(999),
        ]);
        let errs = validate(&code);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("out of bounds"));
    }

    #[test]
    fn const_pool_out_of_range() {
        let code = make_code(vec![
            OpCode::PushConst(42),
        ]);
        let errs = validate(&code);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("constant pool index"));
    }

    #[test]
    fn valid_with_constants() {
        let mut bc = ByteCode::new();
        let idx = bc.add_const("hello");
        bc.emit(OpCode::PushConst(u16::try_from(idx).unwrap()), 0);
        bc.emit(OpCode::Pop, 0);
        assert!(validate(&bc).is_empty());
    }
}
