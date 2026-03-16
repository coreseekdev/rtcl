//! # rtcl-core
//!
//! A lightweight, no-std compatible Tcl interpreter core.
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

// Re-exports for convenience
pub use error::{Error, Result};
pub use value::Value;
pub use interp::Interp;
pub use parser::parse;
pub use command::CommandFunc;

/// Prelude module for common imports
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::value::Value;
    pub use crate::interp::Interp;
    pub use crate::parser::parse;
    pub use crate::types::*;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "std")]
mod platform {
    use std::collections::HashMap as Map;
    pub type HashMap<K, V> = Map<K, V>;
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
mod platform {
    use alloc::collections::BTreeMap;
    pub type HashMap<K, V> = BTreeMap<K, V>;
}
