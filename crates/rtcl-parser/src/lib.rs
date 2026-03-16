//! # rtcl-parser
//!
//! Tcl parser for rtcl — produces an AST from Tcl source code.
//!
//! Uses a recursive descent parser inspired by Molt/jimtcl.

use core::fmt;

mod rd;

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

#[cfg(test)]
mod tests;
