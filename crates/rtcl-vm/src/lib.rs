//! # rtcl-vm
//!
//! Bytecode execution engine and shared types for rtcl.
//!
//! This crate provides:
//! - [`Value`] — the Tcl value type ("everything is a string")
//! - [`Error`] / [`Result`] — error types shared across crates
//! - [`VmContext`] — the execution-hook trait the native JIT/ABI calls
//!   back into (`charge_step` and the value/command interface)
//! - [`interp`] — the interpreter itself (tree-walk + bytecode executor,
//!   command implementations, expression evaluator)
//!
//! Bytecode definitions ([`OpCode`], [`ByteCode`]) live in [`rtcl_ir`]
//! and the compiler in [`rtcl_parser`]; `rtcl-core` re-exports this crate
//! as a compatibility facade.

#![cfg_attr(feature = "embedded", no_std)]

#[cfg(feature = "embedded")]
extern crate alloc;

pub mod error;
pub mod value;
pub mod context;
pub mod command;
pub mod host;
pub mod parser;
pub mod types;
pub mod interp;
#[cfg(feature = "std")]
pub mod channel;

pub use error::{Error, Result, ErrorCode, ControlFlow};
pub use value::Value;
pub use context::VmContext;
pub use interp::Interp;

// Re-export IR types from rtcl-ir for convenience
pub use rtcl_ir::{ByteCode, OpCode, CmdId};

