//! # rtcl-vm
//!
//! Bytecode virtual machine for rtcl.
//!
//! This crate provides:
//! - [`OpCode`] — the instruction set
//! - [`ByteCode`] — compiled bytecode (constants + instructions)
//! - [`Compiler`] — compiles [`rtcl_parser::Command`] AST to [`ByteCode`]
//! - [`Vm`] — executes [`ByteCode`]
//!
//! ## Usage
//!
//! ```ignore
//! use rtcl_parser::parse;
//! use rtcl_vm::{Compiler, ByteCode, OpCode};
//!
//! let ast = parse("set x 10").unwrap();
//! let bytecode = Compiler::compile(&ast);
//! // Inspect opcodes
//! for (i, op) in bytecode.ops().iter().enumerate() {
//!     println!("{:04}: {:?}", i, op);
//! }
//! ```

pub mod opcode;
pub mod compiler;
pub mod bytecode;

pub use opcode::OpCode;
pub use bytecode::ByteCode;
pub use compiler::Compiler;
