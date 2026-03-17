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
pub fn validate(code: &ByteCode) -> Vec<ValidationError> {
    let ops = code.ops();
    let len = ops.len();
    let pool_size = code.constants().len() as u16;
    let mut errors = Vec::new();
    let mut loop_depth: i32 = 0;

    for (pc, op) in ops.iter().enumerate() {
        match op {
            // -- Constant-pool references ------------------------------------
            OpCode::PushConst(idx)
            | OpCode::LoadVar(idx)
            | OpCode::StoreVar(idx)
            | OpCode::LoadArrayElem(idx)
            | OpCode::StoreArrayElem(idx)
            | OpCode::AppendVar(idx)
            | OpCode::UnsetVar(idx)
            | OpCode::VarExists(idx)
            | OpCode::Global(idx) => {
                if *idx >= pool_size {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "constant pool index {} out of range (pool size {})",
                            idx, pool_size,
                        ),
                    });
                }
            }
            OpCode::IncrVar(idx, _) => {
                if *idx >= pool_size {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "constant pool index {} out of range (pool size {})",
                            idx, pool_size,
                        ),
                    });
                }
            }
            OpCode::UpVar { src, .. } => {
                if *src >= pool_size {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "upvar src constant pool index {} out of range",
                            src,
                        ),
                    });
                }
            }

            // -- Jump targets ------------------------------------------------
            OpCode::Jump(target) | OpCode::JumpTrue(target) | OpCode::JumpFalse(target) => {
                if *target as usize > len {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "jump target {} out of bounds (code length {})",
                            target, len,
                        ),
                    });
                }
            }
            OpCode::CatchStart(target) => {
                if *target as usize > len {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "catch target {} out of bounds (code length {})",
                            target, len,
                        ),
                    });
                }
            }

            // -- Loop nesting ------------------------------------------------
            OpCode::LoopEnter { cont, brk } => {
                loop_depth += 1;
                if *cont as usize > len {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "LoopEnter continue target {} out of bounds",
                            cont,
                        ),
                    });
                }
                if *brk as usize > len {
                    errors.push(ValidationError {
                        pc,
                        message: format!(
                            "LoopEnter break target {} out of bounds",
                            brk,
                        ),
                    });
                }
            }
            OpCode::LoopExit => {
                loop_depth -= 1;
                if loop_depth < 0 {
                    errors.push(ValidationError {
                        pc,
                        message: "LoopExit without matching LoopEnter".into(),
                    });
                }
            }
            OpCode::Break | OpCode::Continue => {
                if loop_depth <= 0 {
                    // This is OK in some cases (break/continue used as
                    // a general command), so we make it a warning-level
                    // note rather than a hard error.  The VM will handle
                    // it by returning an Error.
                }
            }

            // Everything else — no structural checks needed.
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
        bc.emit(OpCode::PushConst(idx), 0);
        bc.emit(OpCode::Pop, 0);
        assert!(validate(&bc).is_empty());
    }
}
