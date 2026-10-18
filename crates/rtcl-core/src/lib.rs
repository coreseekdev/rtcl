//! # rtcl-core
//!
//! Tcl interpreter core — integrates `rtcl-parser` and `rtcl-vm`.
//!
//! ## Crate organisation
//!
//! - [`rtcl_parser`] — parser & compiler (AST types, bytecode, opcodes)
//! - [`rtcl_vm`]     — execution engine & shared types (Value, Error, VmContext)
//! - `rtcl-core`     — interpreter, command implementations, expression evaluator
//!
//! ## Features
//!
//! - `std` - Enable standard library support (default)
//! - `alloc` - Enable allocation support for no-std targets
//! - `embedded` - Enable embedded mode with spin locks
//! - `debug` - Enable extra diagnostics

#![cfg_attr(feature = "embedded", no_std)]

#[cfg(feature = "embedded")]
extern crate alloc;

// Core modules — error and value re-export from rtcl-vm
// The interpreter and its supporting modules live in `rtcl-vm`; this
// crate is the compatibility facade (same paths, same items).
pub mod error;
pub mod value;

pub use rtcl_vm::host;
pub use rtcl_vm::parser;
pub use rtcl_vm::interp;
pub use rtcl_vm::command;
pub use rtcl_vm::types;
#[cfg(feature = "std")]
pub use rtcl_vm::channel;

// Re-exports for convenience
pub use error::{Error, Result};
pub use value::Value;
pub use rtcl_vm::interp::Interp;
pub use rtcl_vm::parser::parse;
pub use rtcl_vm::command::{CommandFunc, CommandMeta};

// Re-export sub-crates so downstream can access them via rtcl_core
pub use rtcl_parser;
pub use rtcl_vm;

/// Prelude module for common imports
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::value::Value;
    pub use rtcl_vm::interp::Interp;
    pub use rtcl_vm::parser::parse;
    pub use rtcl_vm::types::*;
    pub use rtcl_parser::{ByteCode, Compiler, OpCode};
    pub use rtcl_vm::VmContext;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(not(feature = "embedded"))]
mod platform {
    use std::collections::HashMap as Map;
    #[allow(dead_code)]
    pub type HashMap<K, V> = Map<K, V>;
}

#[cfg(feature = "embedded")]
mod platform {
    use alloc::collections::BTreeMap;
    pub type HashMap<K, V> = BTreeMap<K, V>;
}
