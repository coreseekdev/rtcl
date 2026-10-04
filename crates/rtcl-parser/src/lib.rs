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

pub mod rd;
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
pub use compiler::split_varlist_simple;

// ---------------------------------------------------------------------------
// AST types
// ---------------------------------------------------------------------------

/// Byte span into the source text a [`Command`] was parsed from.
///
/// Spans are Copy: resolving to `&str` needs the owning [`ScriptUnit`]'s
/// `source` (`span.slice(&unit.source)`), so the interpreter's
/// per-dispatch save/restore of "current command" state is plain moves /
/// refcount bumps on the one shared source — no per-command heap copies.
/// Defined in `rtcl-ir` so [`ByteCode`] sites can share the type.
pub use rtcl_ir::SrcSpan;

/// A parsed script: the command list plus the source text the spans of
/// each [`Command`] (`text`, `word_srcs`) point into.  Keeping the two
/// together lets the AST carry `offset + size` instead of string copies —
/// the whole unit is one `String` + one `Vec<Command>` (quickjs keeps
/// function source the same way).
#[derive(Debug, Clone)]
pub struct ScriptUnit {
    pub source: std::rc::Rc<str>,
    pub commands: Vec<Command>,
}

impl ScriptUnit {
    /// Parse `script`, keeping the source alive for span resolution.
    pub fn parse(script: &str) -> Result<Self, ParseError> {
        Ok(ScriptUnit {
            source: std::rc::Rc::from(script),
            commands: parse(script)?,
        })
    }
}

/// A parsed Tcl command (one line / semicolon-separated unit).
#[derive(Debug, Clone)]
pub struct Command {
    /// Command words (first word is the command name).
    pub words: Vec<Word>,
    /// Source line number (1-based).
    pub line: usize,
    /// Byte span of the command's source text as written (leading
    /// whitespace skipped, one trailing terminator `\n`/`;` stripped at
    /// parse time by adjusting `end`) — tclsh errorInfo frames quote this
    /// verbatim (`while executing "set a 1 "`).
    pub text: SrcSpan,
    /// Byte span of each word's raw source, aligned with `words` (braced
    /// words keep their braces) — used for `eval`-style errorInfo frames
    /// and loop-body line rebasing.
    pub word_srcs: std::rc::Rc<Vec<SrcSpan>>,
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
