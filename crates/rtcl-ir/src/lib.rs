//! # rtcl-ir
//!
//! Intermediate representation types shared between the compiler (`rtcl-parser`)
//! and the execution engine (`rtcl-vm`).
//!
//! This crate defines:
//! - [`OpCode`] — the virtual machine instruction set
//! - [`CmdId`] — numeric identifiers for built-in commands
//! - [`ByteCode`] — compiled bytecode (constant pool, instructions, locals, line map)
//!
//! Zero dependencies — this crate is the shared bridge between the front-end
//! (parser/compiler) and the back-end (VM executor).

pub mod opcode;
pub mod bytecode;

pub use opcode::{OpCode, CmdId};
pub use bytecode::ByteCode;
