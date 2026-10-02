//! Error handling for rtcl

use core::fmt;
use core::result;

use crate::value::Value;

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

    /// Wrong-args error whose message is already in Tcl's exact format,
    /// for commands whose arity errors are not of the
    /// `wrong # args: should be "cmd arg ..."` form (e.g. `if` clauses).
    WrongArgsMsg(String),

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
        value: Option<Value>,
        /// Tcl return code: 0=ok, 1=error, 2=return, 3=break, 4=continue
        level: i32,
        /// Optional `-errorinfo` string (for stack trace).
        error_info: Option<String>,
        /// Optional `-errorcode` list (e.g. "POSIX ENOENT {no such file}").
        error_code: Option<String>,
    },

    /// Tail-call request — signals that `tailcall` wants the
    /// current proc frame to be replaced with a new command.
    /// `args[0]` is the command name, `args[1..]` are arguments.
    TailCall {
        args: Vec<String>,
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

    /// Create a wrong-args error with a pre-formatted Tcl message
    /// (for arity errors that are not of the `should be "cmd arg ..."` form).
    pub fn wrong_args_msg(msg: impl Into<String>) -> Self {
        Error::WrongArgsMsg(msg.into())
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
    pub fn ret(value: Option<Value>) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Return,
            value,
            level: 0,
            error_info: None,
            error_code: None,
        }
    }

    /// Create a `return -level N`: negative encodes the explicit level as
    /// −(N+1) so it cannot collide with the -code completion codes and so
    /// −1 (i.e. `-level 0`) can be recognized by script boundaries.
    pub fn ret_level(level: i32, value: Option<Value>) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Return,
            value,
            level: -(level + 1),
            error_info: None,
            error_code: None,
        }
    }

    /// Create a return with explicit code (for `return -code`)
    /// Always uses Return kind so proc boundary catches it, but level carries the target code.
    pub fn return_with_code(code: i32, value: Option<Value>) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Return,
            value,
            level: code,
            error_info: None,
            error_code: None,
        }
    }

    /// Create a return with full options (for `return -code -errorinfo -errorcode`).
    pub fn return_with_options(
        code: i32,
        value: Option<Value>,
        error_info: Option<String>,
        error_code: Option<String>,
    ) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Return,
            value,
            level: code,
            error_info,
            error_code,
        }
    }

    /// Create a break control flow
    pub fn brk() -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Break,
            value: None,
            level: 1,
            error_info: None,
            error_code: None,
        }
    }

    /// Create a multi-level break: `break N` breaks out of N nested loops.
    pub fn brk_level(n: i32) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Break,
            value: None,
            level: n,
            error_info: None,
            error_code: None,
        }
    }

    /// Create a continue control flow
    pub fn cont() -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Continue,
            value: None,
            level: 1,
            error_info: None,
            error_code: None,
        }
    }

    /// Create a multi-level continue: `continue N` skips N nested loops.
    pub fn cont_level(n: i32) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Continue,
            value: None,
            level: n,
            error_info: None,
            error_code: None,
        }
    }

    /// Create an exit control flow
    pub fn exit(code: Option<i32>) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Exit,
            value: code.map(|c| Value::from_int(c as i64)),
            level: 0,
            error_info: None,
            error_code: None,
        }
    }

    /// Create a tail-call signal.
    /// `args[0]` is the target command name; the rest are its arguments.
    pub fn tail_call(args: Vec<String>) -> Self {
        Error::TailCall { args }
    }

    /// Check if this is a tail-call request.
    pub fn is_tail_call(&self) -> bool {
        matches!(self, Error::TailCall { .. })
    }

    /// Consume the error and return the tail-call arguments,
    /// or `None` if this is not a `TailCall`.
    pub fn into_tail_call_args(self) -> Option<Vec<String>> {
        match self {
            Error::TailCall { args } => Some(args),
            _ => None,
        }
    }

    /// Create an error control flow (for `return -code error`)
    pub fn error_flow(msg: String) -> Self {
        Error::ControlFlow {
            kind: ControlFlow::Error,
            value: Some(Value::from_str(&msg)),
            level: 1,
            error_info: None,
            error_code: None,
        }
    }

    /// Get the Tcl return code for this error
    pub fn return_code(&self) -> i32 {
        match self {
            Error::ControlFlow { kind: ControlFlow::Return, .. } => 2,
            Error::ControlFlow { kind: ControlFlow::Break, .. } => 3,
            Error::ControlFlow { kind: ControlFlow::Continue, .. } => 4,
            Error::ControlFlow { kind: ControlFlow::Exit, .. } => 5,
            Error::ControlFlow { kind: ControlFlow::Error, .. } => 1,
            _ => 1, // all other errors are code 1
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

    /// For break/continue: get the loop level (number of remaining loops to exit).
    /// Returns 1 for a simple break/continue; >1 for multi-level.
    pub fn loop_level(&self) -> i32 {
        match self {
            Error::ControlFlow { kind: ControlFlow::Break | ControlFlow::Continue, level, .. } => {
                if *level <= 0 { 1 } else { *level }
            }
            _ => 1,
        }
    }

    /// Return a copy with the loop level decremented by 1 (for propagation).
    pub fn with_decremented_loop_level(self) -> Self {
        match self {
            Error::ControlFlow { kind, value, level, error_info, error_code } => {
                Error::ControlFlow { kind, value, level: level - 1, error_info, error_code }
            }
            other => other,
        }
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
            Error::WrongArgsMsg(_) => -3,
            Error::VarNotFound { .. } => -4,
            Error::TypeMismatch { .. } => -5,
            Error::DivisionByZero => -6,
            Error::ControlFlow { kind, .. } => *kind as i32,
            Error::TailCall { .. } => -7,
            Error::Msg(_) => -99,
        }
    }

    /// The payload a Tcl script should see for this error — what `catch`'s
    /// result variable receives (e.g. `return hi` → `hi`, `break` → ``).
    pub fn message_text(&self) -> String {
        match self {
            Error::ControlFlow { value: Some(v), .. } => v.as_str().to_string(),
            Error::ControlFlow { value: None, .. } => String::new(),
            other => other.to_string(),
        }
    }

    /// Tcl `-errorcode` list for this error (e.g. `NONE`,
    /// `ARITH DIVZERO {divide by zero}`).
    pub fn tcl_error_code(&self) -> String {
        match self {
            Error::ControlFlow { error_code: Some(c), .. } => c.clone(),
            Error::DivisionByZero => "ARITH DIVZERO {divide by zero}".to_string(),
            Error::Runtime { message, .. } if message.starts_with("domain error") => {
                "ARITH DOMAIN {domain error: argument not in valid range}".to_string()
            }
            Error::Runtime { message, .. } if message.starts_with("can't read \"") => {
                // tclsh read errors: a missing element in an existing
                // array is bare `TCL READ VARNAME` (no name); a read
                // against a non-array (`$b(0)`) or an array read as a
                // scalar blames `TCL LOOKUP VARNAME` with the base name.
                if message.contains("no such element in array") {
                    "TCL READ VARNAME".to_string()
                } else if let Some(q0) = message.find('"') {
                    let rest = &message[q0 + 1..];
                    let name = rest.split('"').next().unwrap_or(rest);
                    let base = name.split('(').next().unwrap_or(name);
                    format!("TCL LOOKUP VARNAME {}", base)
                } else {
                    "NONE".to_string()
                }
            }
            Error::InvalidCommand { name } => {
                format!("TCL LOOKUP COMMAND {}", crate::value::tcl_quote(name))
            }
            Error::VarNotFound { name } => {
                // An element read of a wholly-missing array (`$zz(5)`)
                // blames the base name: `TCL LOOKUP VARNAME zz`.
                let base = name.split('(').next().unwrap_or(name);
                format!("TCL LOOKUP VARNAME {}", crate::value::tcl_quote(base))
            }
            Error::WrongNumArgs { .. } | Error::WrongArgsMsg(_) => {
                "TCL WRONGARGS".to_string()
            }
            _ => "NONE".to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax { message, .. } => {
                // tclsh renders parser errors bare ("missing close-bracket").
                write!(f, "{}", message)
            }
            Error::Runtime { message, .. } => {
                write!(f, "{}", message)
            }
            Error::InvalidCommand { name } => {
                write!(f, "invalid command name \"{}\"", name)
            }
            Error::WrongNumArgs { command, expected, usage, .. } => {
                // Tcl renders arity errors as a single line:
                //   wrong # args: should be "cmd arg ..."
                match usage {
                    Some(u) if u.is_empty() => {
                        write!(f, "wrong # args: should be \"{}\"", command)
                    }
                    Some(u) => {
                        write!(f, "wrong # args: should be \"{} {}\"", command, u)
                    }
                    None => {
                        write!(f, "wrong # args: should be \"{} {} args\"", command, expected)
                    }
                }
            }
            Error::WrongArgsMsg(message) => write!(f, "{}", message),
            Error::VarNotFound { name } => {
                write!(f, "can't read \"{}\": no such variable", name)
            }
            Error::TypeMismatch { expected, actual } => {
                write!(f, "expected {}, got {}", expected, actual)
            }
            Error::DivisionByZero => {
                write!(f, "divide by zero")
            }
            Error::ControlFlow { kind, value, .. } => {
                match kind {
                    ControlFlow::Return => write!(f, "return")?,
                    ControlFlow::Break => write!(f, "break")?,
                    ControlFlow::Continue => write!(f, "continue")?,
                    // An error flow's message is its value (like the `error` command).
                    ControlFlow::Error => {
                        if let Some(v) = value {
                            return write!(f, "{}", v.as_str());
                        }
                        write!(f, "error")?;
                    }
                    ControlFlow::Exit => write!(f, "exit")?,
                }
                if let Some(v) = value {
                    write!(f, " with value: {}", v.as_str())?;
                }
                Ok(())
            }
            Error::TailCall { args } => {
                write!(f, "tailcall {}", args.join(" "))
            }
            Error::Msg(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for Error {}
