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

// Re-exports
pub use opcode::OpCode;
pub use opcode::{CmdId, StdCmdId, ExtCmdId};
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
#[derive(Debug, Clone)]
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

/// Check whether `source` is a complete Tcl script (balanced braces, quotes,
/// and brackets).  Returns `true` if the script can be parsed without needing
/// more input.  Used by `info complete` and multi-line REPL input.
pub fn is_complete(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;

    /// Scan forward until the matching close-brace, respecting nesting and
    /// backslash-escaped braces.  Returns the index *after* the `}`, or
    /// `None` if EOF is reached first.
    fn skip_braces(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start;
        let mut depth: u32 = 1;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => { depth += 1; i += 1; }
                b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 { return Some(i); }
                }
                b'\\' => {
                    i += 1; // skip backslash
                    if i < bytes.len() { i += 1; } // skip escaped char
                }
                _ => { i += 1; }
            }
        }
        None // unmatched
    }

    /// Scan forward until the closing `"`, handling backslash escapes and
    /// nested brackets / command-subs.  Returns index *after* the `"`.
    fn skip_quotes(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => { return Some(i + 1); }
                b'\\' => {
                    i += 1;
                    if i < bytes.len() { i += 1; }
                }
                b'[' => {
                    i += 1;
                    match skip_brackets(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                _ => { i += 1; }
            }
        }
        None
    }

    /// Scan forward until the matching `]`.
    fn skip_brackets(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b']' => { return Some(i + 1); }
                b'{' => {
                    i += 1;
                    match skip_braces(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                b'"' => {
                    i += 1;
                    match skip_quotes(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                b'\\' => {
                    i += 1;
                    if i < bytes.len() { i += 1; }
                }
                b'[' => {
                    i += 1;
                    match skip_brackets(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                _ => { i += 1; }
            }
        }
        None
    }

    // Main scan — top-level Tcl script
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                i += 1;
                match skip_braces(bytes, i) {
                    Some(end) => { i = end; }
                    None => { return false; }
                }
            }
            b'"' => {
                i += 1;
                match skip_quotes(bytes, i) {
                    Some(end) => { i = end; }
                    None => { return false; }
                }
            }
            b'[' => {
                i += 1;
                match skip_brackets(bytes, i) {
                    Some(end) => { i = end; }
                    None => { return false; }
                }
            }
            b'\\' => {
                i += 1;
                if i < bytes.len() { i += 1; }
            }
            _ => { i += 1; }
        }
    }
    true
}

#[cfg(test)]
mod tests;
