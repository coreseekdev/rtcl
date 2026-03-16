//! Error handling for rtcl-core

use core::fmt;
use core::result;

/// Result type alias for rtcl operations
pub type Result<T> = result::Result<T, Error>;

/// Main error type for rtcl
#[derive(Debug, Clone)]
pub enum Error {
    /// Syntax error during parsing
    Syntax {
        message: String,
        line: usize,
        column: usize,
    },

    /// Runtime error during execution
    Runtime {
        message: String,
        code: ErrorCode,
    },

    /// Invalid command name
    InvalidCommand {
        name: String,
    },

    /// Wrong number of arguments
    WrongNumArgs {
        command: String,
        expected: usize,
        actual: usize,
        usage: Option<String>,
    },

    /// Variable not found
    VarNotFound {
        name: String,
    },

    /// Type mismatch
    TypeMismatch {
        expected: String,
        actual: String,
    },

    /// Division by zero
    DivisionByZero,

    /// Control flow (return, break, continue)
    ControlFlow {
        kind: ControlFlow,
        value: Option<String>,
    },

    /// Custom error with message
    Msg(String),
}

/// Error codes for runtime errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Generic error
    Generic = 1,
    /// Invalid operation
    InvalidOp = 2,
    /// Stack overflow
    StackOverflow = 3,
    /// Timeout
    Timeout = 4,
    /// IO error (when std is available)
    Io = 5,
    /// Not found
    NotFound = 6,
}

/// Control flow types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlow {
    Return,
    Break,
    Continue,
    Error,
    Exit,
}

impl Error {
    /// Create a syntax error
    pub fn syntax(msg: impl Into<String>, line: usize, col: usize) -> Self {
        Error::Syntax {
            message: msg.into(),
            line,
            column: col,
        }
    }

    /// Create a runtime error
    pub fn runtime(msg: impl Into<String>, code: ErrorCode) -> Self {
        Error::Runtime {
            message: msg.into(),
            code,
        }
    }

    /// Create an invalid command error
    pub fn invalid_command(name: impl Into<String>) -> Self {
        Error::InvalidCommand { name: name.into() }
    }

    /// Create a wrong number of arguments error
    pub fn wrong_args(cmd: impl Into<String>, expected: usize, actual: usize) -> Self {
        Error::WrongNumArgs {
            command: cmd.into(),
            expected,
            actual,
            usage: None,
        }
    }

    /// Create a wrong number of arguments error with usage hint
    pub fn wrong_args_with_usage(
        cmd: impl Into<String>,
        expected: usize,
        actual: usize,
        usage: impl Into<String>,
    ) -> Self {
        Error::WrongNumArgs {
            command: cmd.into(),
            expected,
            actual,
            usage: Some(usage.into()),
        }
    }

    /// Create a variable not found error
    pub fn var_not_found(name: impl Into<String>) -> Self {
        Error::VarNotFound { name: name.into() }
    }

    /// Create a type mismatch error
    pub fn type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Error::TypeMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create a return control flow
    pub fn ret(value: Option<String>) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Return,
            value,
        }
    }

    /// Create a break control flow
    pub fn brk() -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Break,
            value: None,
        }
    }

    /// Create a continue control flow
    pub fn cont() -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Continue,
            value: None,
        }
    }

    /// Create an exit control flow
    pub fn exit(code: Option<i32>) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Exit,
            value: code.map(|c| c.to_string()),
        }
    }

    /// Check if this is a control flow error
    pub fn is_control_flow(&self) -> bool {
        matches!(self, Error::ControlFlow { .. })
    }

    /// Check if this is a return
    pub fn is_return(&self) -> bool {
        matches!(self, Error::ControlFlow { kind: ControlFlow::Return, .. })
    }

    /// Check if this is a break
    pub fn is_break(&self) -> bool {
        matches!(self, Error::ControlFlow { kind: ControlFlow::Break, .. })
    }

    /// Check if this is a continue
    pub fn is_continue(&self) -> bool {
        matches!(self, Error::ControlFlow { kind: ControlFlow::Continue, .. })
    }

    /// Check if this is an exit
    pub fn is_exit(&self) -> bool {
        matches!(self, Error::ControlFlow { kind: ControlFlow::Exit, .. })
    }

    /// Get error code (for Tcl compatibility)
    pub fn code(&self) -> i32 {
        match self {
            Error::Syntax { .. } => -1,
            Error::Runtime { code, .. } => *code as i32,
            Error::InvalidCommand { .. } => -2,
            Error::WrongNumArgs { .. } => -3,
            Error::VarNotFound { .. } => -4,
            Error::TypeMismatch { .. } => -5,
            Error::DivisionByZero => -6,
            Error::ControlFlow { kind, .. } => *kind as i32,
            Error::Msg(_) => -99,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax { message, line, column } => {
                write!(f, "syntax error at {}:{}: {}", line, column, message)
            }
            Error::Runtime { message, code } => {
                write!(f, "runtime error (code {}): {}", *code as i32, message)
            }
            Error::InvalidCommand { name } => {
                write!(f, "invalid command name \"{}\"", name)
            }
            Error::WrongNumArgs { command, expected, actual, usage } => {
                write!(
                    f,
                    "wrong # args: should be \"{} {}\"",
                    command,
                    usage.as_deref().unwrap_or(&format!("{} args", expected))
                )?;
                if usage.is_some() {
                    write!(f, "\n  expected {} arguments, got {}", expected, actual)?;
                }
                Ok(())
            }
            Error::VarNotFound { name } => {
                write!(f, "can't read \"{}\": no such variable", name)
            }
            Error::TypeMismatch { expected, actual } => {
                write!(f, "expected {}, got {}", expected, actual)
            }
            Error::DivisionByZero => {
                write!(f, "divide by zero")
            }
            Error::ControlFlow { kind, value } => {
                match kind {
                    ControlFlow::Return => write!(f, "return"),
                    ControlFlow::Break => write!(f, "break"),
                    ControlFlow::Continue => write!(f, "continue"),
                    ControlFlow::Error => write!(f, "error"),
                    ControlFlow::Exit => write!(f, "exit"),
                }?;
                if let Some(v) = value {
                    write!(f, " with value: {}", v)?;
                }
                Ok(())
            }
            Error::Msg(s) => write!(f, "{}", s),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
