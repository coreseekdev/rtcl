//! Runtime context trait for the VM executor.
//!
//! The [`VmContext`] trait abstracts the interpreter interface that the
//! bytecode executor needs.  This decouples the VM from the concrete
//! [`Interp`](crate) type which lives in `rtcl-core`.

use crate::error::Result;
use crate::value::Value;

/// Trait that the bytecode executor requires from its host interpreter.
///
/// `rtcl-core::Interp` implements this trait so the VM can call back into
/// the interpreter for variable access, command dispatch, and nested
/// evaluation without depending on the concrete `Interp` type.
pub trait VmContext {
    /// Read a variable by name (including `name(index)` for arrays).
    fn get_var(&self, name: &str) -> Result<Value>;

    /// Write a variable, returning the stored value.
    fn set_var(&mut self, name: &str, value: Value) -> Result<Value>;

    /// Remove a variable.  Implementations should silently ignore
    /// variables that do not exist.
    fn unset_var(&mut self, name: &str) -> Result<()>;

    /// Check if a variable exists.
    fn var_exists(&self, name: &str) -> bool;

    /// Increment a variable by `amount`, returning the new value.
    fn incr_var(&mut self, name: &str, amount: i64) -> Result<Value>;

    /// Append `value` to the named variable, returning the new value.
    fn append_var(&mut self, name: &str, value: &str) -> Result<Value>;

    /// Evaluate a Tcl script and return the result.
    fn eval_script(&mut self, script: &str) -> Result<Value>;

    /// Evaluate a Tcl expression (like `expr {…}`) and return the result.
    fn eval_expr(&mut self, expr: &str) -> Result<Value>;

    /// Invoke a command by its arguments.  `args[0]` is the command name;
    /// the remaining elements are its arguments.
    fn invoke_command(&mut self, args: &[Value]) -> Result<Value>;

    /// **ECall** — invoke a standard/language command by numeric ID.
    ///
    /// `cmd_id` corresponds to a [`StdCmdId`](rtcl_parser::StdCmdId).
    fn ecall(&mut self, cmd_id: u16, args: &[Value]) -> Result<Value>;

    /// **SysCall** — invoke an extension/platform command by numeric ID.
    ///
    /// `cmd_id` corresponds to a [`ExtCmdId`](rtcl_parser::ExtCmdId).
    fn syscall(&mut self, cmd_id: u16, args: &[Value]) -> Result<Value>;
}
