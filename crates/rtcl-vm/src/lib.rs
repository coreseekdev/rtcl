//! # rtcl-vm
//!
//! Bytecode execution engine and shared types for rtcl.
//!
//! This crate provides:
//! - [`Value`] — the Tcl value type ("everything is a string")
//! - [`Error`] / [`Result`] — error types shared across crates
//! - [`VmContext`] — trait abstracting the interpreter for the VM
//! - [`execute`] — runs [`ByteCode`] against a [`VmContext`]
//!
//! Bytecode definitions ([`OpCode`], [`ByteCode`]) live in [`rtcl_ir`].
//! The [`Compiler`] lives in [`rtcl_parser`].
//!
//! ## Usage
//!
//! ```ignore
//! use rtcl_ir::{ByteCode, OpCode};
//! use rtcl_parser::Compiler;
//! use rtcl_vm::{Value, execute, VmContext};
//!
//! // Compile a script
//! let bytecode = Compiler::compile_script("set x 10").unwrap();
//!
//! // Execute using a VmContext implementation
//! let result = execute(&mut my_interp, &bytecode).unwrap();
//! ```

pub mod error;
pub mod value;
pub mod context;
pub mod execute;

pub use error::{Error, Result, ErrorCode, ControlFlow};
pub use value::Value;
pub use context::VmContext;
pub use execute::execute;

// Re-export IR types from rtcl-ir for convenience
pub use rtcl_ir::{ByteCode, OpCode, CmdId};

