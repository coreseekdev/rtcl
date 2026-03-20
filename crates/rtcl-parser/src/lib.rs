//! # rtcl-parser
//!
//! Tcl parser and compiler for rtcl.
//!
//! This crate takes Tcl source code and produces either an AST
//! ([`Command`]/[`Word`]) or compiled [`ByteCode`] (via [`Compiler`]).
//!
//! ## Usage
//!
//! ```ignore
//! use rtcl_parser::{parse, Compiler, ByteCode, OpCode};
//!
//! // Parse to AST
//! let ast = parse("set x 10").unwrap();
//!
//! // Or compile directly to bytecode
//! let bytecode = Compiler::compile_script("set x 10").unwrap();
//! for (i, op) in bytecode.ops().iter().enumerate() {
//!     println!("{:04}: {:?}", i, op);
//! }
//! ```

use core::fmt;

mod rd;
pub mod opcode;
pub mod bytecode;
pub mod compiler;
pub mod validate;
pub mod expr_compile;
mod completeness;

// Re-exports
pub use opcode::OpCode;
pub use opcode::CmdId;
pub use bytecode::ByteCode;
pub use compiler::Compiler;

// ---------------------------------------------------------------------------
// AST types
// ---------------------------------------------------------------------------

/// A parsed Tcl command (one line / semicolon-separated unit).
#[derive(Debug, Clone)]
pub struct Command {
    /// Command words (first word is the command name).
    pub words: Vec<Word>,
    /// Source line number (1-based).
    pub line: usize,
}

/// A word in a Tcl command.
#[derive(Debug, Clone, PartialEq)]
pub enum Word {
    /// Literal string — no substitution.
    Literal(String),
    /// Variable reference: `$var`, `${var}`, `$var(index)`.
    VarRef(String),
    /// Command substitution: `[cmd args...]`.
    CommandSub(String),
    /// Concatenation of multiple parts (e.g. `"hello $name"`).
    Concat(Vec<Word>),
    /// Expand syntax: `{*}word` — expands the word as multiple arguments.
    Expand(Box<Word>),
    /// Expression sugar: `$[expr]` — evaluate content as expression (jimtcl extension).
    ExprSugar(String),
}

impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Word::Literal(s) => write!(f, "{}", s),
            Word::VarRef(n) => write!(f, "${}", n),
            Word::CommandSub(c) => write!(f, "[{}]", c),
            Word::Concat(parts) => {
                for p in parts {
                    write!(f, "{}", p)?;
                }
                Ok(())
            }
            Word::Expand(inner) => write!(f, "{{*}}{}", inner),
            Word::ExprSugar(e) => write!(f, "$[{}]", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Parse error with source location information.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    /// Byte offset into the source string where the error occurred.
    pub offset: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "parse error at {}:{}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

// Convenience alias
pub type ParseResult<T> = Result<T, ParseError>;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> ParseResult<Vec<Command>> {
    rd::parse(source)
}

// Re-export token types from rd module
pub use rd::token::{Token, Tokenizer};

/// Check whether `source` is a complete Tcl script (balanced braces, quotes,
/// and brackets).  Returns `true` if the script can be parsed without needing
/// more input.  Used by `info complete` and multi-line REPL input.
pub use completeness::is_complete;

#[cfg(test)]
mod tests;
