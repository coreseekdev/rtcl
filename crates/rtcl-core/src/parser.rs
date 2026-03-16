//! Tcl parser - re-exports from `rtcl_parser` with error type conversion.
//!
//! The actual parser lives in the [`rtcl_parser`] crate.  This module provides
//! a `parse()` function that returns `rtcl_core::Result` for backward
//! compatibility with the interpreter.

// Re-export AST types so the rest of rtcl-core can use them unchanged.
pub use rtcl_parser::{Command, Word};

use crate::error::{Error, Result};

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> Result<Vec<Command>> {
    rtcl_parser::parse(source).map_err(|e| Error::syntax(e.message, e.line, e.column))
}
