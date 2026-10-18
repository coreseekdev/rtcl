//! # rtcl-vm
//!
//! Bytecode execution engine and shared types for rtcl.
//!
//! This crate provides:
//! - [`Value`] — the Tcl value type ("everything is a string")
//! - [`Error`] / [`Result`] — error types shared across crates
//! - [`VmContext`] — the execution-hook trait the native JIT/ABI calls
//!   back into (`charge_step` and the value/command interface)
//!
//! Bytecode definitions ([`OpCode`], [`ByteCode`]) live in [`rtcl_ir`],
//! the compiler in [`rtcl_parser`], and the interpreter (this VM) in
//! `rtcl-core`'s `interp` module.

pub mod error;
pub mod value;
pub mod context;

pub use error::{Error, Result, ErrorCode, ControlFlow};
pub use value::Value;
pub use context::VmContext;

// Re-export IR types from rtcl-ir for convenience
pub use rtcl_ir::{ByteCode, OpCode, CmdId};

