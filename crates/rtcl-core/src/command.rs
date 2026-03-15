//! Command registration and types

use crate::error::Result;
use crate::interp::Interp;
use crate::value::Value;

/// Command function type
pub type CommandFunc = fn(&mut Interp, &[Value]) -> Result<Value>;

/// Built-in command marker
pub trait BuiltinCmd {
    fn name(&self) -> &'static str;
    fn execute(&self, interp: &mut Interp, args: &[Value]) -> Result<Value>;
}

/// Command trait for custom commands
pub trait Command: Send + Sync {
    /// Get the command name
    fn name(&self) -> &str;

    /// Execute the command
    fn execute(&self, interp: &mut Interp, args: &[Value]) -> Result<Value>;

    /// Get help text
    fn help(&self) -> Option<&str> {
        None
    }
}

/// Information about a command
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// Command name
    pub name: String,
    /// Number of arguments (None for variable)
    pub min_args: usize,
    pub max_args: Option<usize>,
    /// Help text
    pub help: Option<String>,
}

impl CommandInfo {
    pub fn new(name: impl Into<String>) -> Self {
        CommandInfo {
            name: name.into(),
            min_args: 0,
            max_args: None,
            help: None,
        }
    }

    pub fn args(mut self, min: usize, max: impl Into<Option<usize>>) -> Self {
        self.min_args = min;
        self.max_args = max.into();
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}
