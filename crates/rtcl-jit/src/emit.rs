//! ByteCode → wasm module emitter.
//!
//! M0 scope: the constant-return subset — [`OpCode::BeginCmd`],
//! [`OpCode::PushInt`], [`OpCode::Return`], [`OpCode::Nop`] and the empty
//! unit — which is enough for the full round-trip (compile → instantiate →
//! call → retrieve the result) and for the value ABI to be exercised end
//! to end.  Every op outside the subset is rejected as [`Unsupported`] and
//! the caller stays on the interpreter path.
//!
//! Emitted module (see `lib.rs` for the ABI rationale):
//!
//! ```wat
//! (module
//!   (import "rtcl" "push_int"    (func $push_int    (param i64) (result i32)))
//!   (import "rtcl" "set_result"  (func $set_result  (param i32)))
//!   (import "rtcl" "push_empty"  (func $push_empty  (result i32)))
//!   (func $run (export "run") (result i32) ...))
//! ```
//!
//! The wasm operand stack carries arena handles (i32).  `PushInt(n)`
//! lowers to `i64.const n; call $push_int`; `Return` lowers to
//! `call $set_result; i32.const 0` (TCL_OK).  Reaching the end of the op
//! stream mirrors the interpreter's stream-end discipline: the result is
//! whatever the last command left on the stack, or the empty value.

use rtcl_ir::bytecode::ByteCode;
use rtcl_ir::opcode::OpCode;
use wasm_encoder::{
    CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    ImportSection, Instruction, Module, TypeSection, ValType,
};

/// The emitter met an op it cannot compile byte-exactly.  The payload is
/// the op's display form (as printed by `disassemble`) for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl core::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "jit: unsupported opcode: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

/// Imported function indices inside the emitted module.
mod import {
    pub const PUSH_INT: u32 = 0;
    pub const SET_RESULT: u32 = 1;
    pub const PUSH_EMPTY: u32 = 2;
    /// The one locally defined function (`run`) comes after the imports.
    pub const RUN: u32 = 3;
}

/// Tcl result codes returned by `run` (tclsh's TCL_* convention).
pub mod code {
    pub const OK: i32 = 0;
}

/// Compile a [`ByteCode`] to wasm binary format.
///
/// Returns [`Unsupported`] when any instruction is outside the currently
/// compiled subset — callers must keep the interpreter path in that case;
/// nothing partial is ever emitted.
pub fn compile_module(code: &ByteCode) -> Result<Vec<u8>, Unsupported> {
    let mut body = Function::new(vec![]);
    let mut depth: usize = 0;
    for op in code.ops() {
        match op {
            // The errorInfo harness is host-side; the M0 subset cannot
            // raise, so the site marker carries no codegen.
            OpCode::BeginCmd(_) | OpCode::Nop => {}
            OpCode::PushInt(n) => {
                body.instruction(&Instruction::I64Const(*n));
                body.instruction(&Instruction::Call(import::PUSH_INT));
                depth += 1;
            }
            OpCode::Return => {
                if depth == 0 {
                    return Err(Unsupported(format!("{op} (empty stack)")));
                }
                // Pops the handle operand, records the result, returns
                // TCL_OK.
                body.instruction(&Instruction::Call(import::SET_RESULT));
                body.instruction(&Instruction::I32Const(code::OK));
                body.instruction(&Instruction::Return);
                depth -= 1;
            }
            other => return Err(Unsupported(other.to_string())),
        }
    }
    // Stream end: the interpreter's result is the last command's value
    // (top of stack), or the empty value when nothing was pushed.
    match depth {
        0 => {
            body.instruction(&Instruction::Call(import::PUSH_EMPTY));
            body.instruction(&Instruction::Call(import::SET_RESULT));
            body.instruction(&Instruction::I32Const(code::OK));
        }
        1 => {
            body.instruction(&Instruction::Call(import::SET_RESULT));
            body.instruction(&Instruction::I32Const(code::OK));
        }
        _ => {
            return Err(Unsupported(
                "stack depth > 1 at stream end".to_string(),
            ));
        }
    }
    body.instruction(&Instruction::End);

    let mut types = TypeSection::new();
    // t0: () -> i32          (run)
    types.ty().function(vec![], vec![ValType::I32]);
    // t1: (i64) -> i32       (push_int)
    types.ty().function(vec![ValType::I64], vec![ValType::I32]);
    // t2: (i32) -> ()        (set_result)
    types.ty().function(vec![ValType::I32], vec![]);
    // t3: () -> i32          (push_empty)
    types.ty().function(vec![], vec![ValType::I32]);

    let mut imports = ImportSection::new();
    imports.import("rtcl", "push_int", EntityType::Function(1));
    imports.import("rtcl", "set_result", EntityType::Function(2));
    imports.import("rtcl", "push_empty", EntityType::Function(3));

    let mut funcs = FunctionSection::new();
    funcs.function(0); // run : t0

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, import::RUN);

    let mut code_section = CodeSection::new();
    code_section.function(&body);

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&funcs);
    module.section(&exports);
    module.section(&code_section);
    Ok(module.finish())
}
