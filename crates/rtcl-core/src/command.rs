//! Command registration, categories, and types.
//!
//! Commands are divided into three categories:
//!
//! - **Language** — core Tcl language primitives that are inseparable from the
//!   interpreter semantics (`set`, `if`, `while`, `proc`, `return`, …).
//!   These *cannot* be meaningfully removed.
//!
//! - **Standard** — data-manipulation commands that ship with every Tcl
//!   distribution (`string`, `list`, `dict`, `expr`, `format`, …).
//!   They do not affect the interpreter's control-flow machinery and could
//!   technically be implemented as an extension library, but they are so
//!   universally expected that they are registered by default.
//!
//! - **Extension** — platform-dependent or optional commands that may be
//!   omitted in constrained environments (`puts`, `source`, `file`, `glob`,
//!   `regexp`/`regsub`, `disassemble`, …).

use crate::error::Result;
use crate::interp::Interp;
use crate::value::Value;

/// Command function type
pub type CommandFunc = fn(&mut Interp, &[Value]) -> Result<Value>;

/// Category describing how tightly a command is bound to the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandCategory {
    /// Core language primitives — inseparable from the interpreter.
    Language,
    /// Standard library — shipped by default, can theoretically be external.
    Standard,
    /// Extension / platform — optional, can be omitted in no-std / embedded.
    Extension,
}

impl core::fmt::Display for CommandCategory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CommandCategory::Language => write!(f, "language"),
            CommandCategory::Standard => write!(f, "standard"),
            CommandCategory::Extension => write!(f, "extension"),
        }
    }
}

/// Information about a registered command (name + category).
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// Command name
    pub name: String,
    /// Category
    pub category: CommandCategory,
}
