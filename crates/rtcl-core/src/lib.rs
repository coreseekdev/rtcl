//! # rtcl-core
//!
//! Tcl interpreter core — integrates `rtcl-parser` and `rtcl-vm`.
//!
//! ## Crate organisation
//!
//! - [`rtcl_parser`] — pest-based Tcl parser (AST types)
//! - [`rtcl_vm`]     — bytecode compiler & VM (opcode definitions)
//! - `rtcl-core`     — interpreter, command implementations, value type
//!
//! ## Features
//!
//! - `std` - Enable standard library support (default)
//! - `alloc` - Enable allocation support for no-std targets
//! - `embedded` - Enable embedded mode with spin locks
//! - `debug` - Enable extra diagnostics

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Core modules
pub mod error;
pub mod value;
pub mod parser;
pub mod interp;
pub mod command;
pub mod types;
pub mod vm;

// Re-exports for convenience
pub use error::{Error, Result};
pub use value::Value;
pub use interp::Interp;
pub use parser::parse;
pub use command::CommandFunc;

// Re-export sub-crates so downstream can access them via rtcl_core
pub use rtcl_parser;
pub use rtcl_vm;

/// Prelude module for common imports
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::value::Value;
    pub use crate::interp::Interp;
    pub use crate::parser::parse;
    pub use crate::types::*;
    pub use rtcl_vm::{ByteCode, Compiler, OpCode};
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "std")]
mod platform {
    use std::collections::HashMap as Map;
    #[allow(dead_code)]
    pub type HashMap<K, V> = Map<K, V>;
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
mod platform {
    use alloc::collections::BTreeMap;
    pub type HashMap<K, V> = BTreeMap<K, V>;
}
