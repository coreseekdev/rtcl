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

    /// A1 步数预算：每条命令分派/每次循环回边计一步；超限抛
    /// `ErrorCode::Timeout`。缺省 no-op——未武装预算的宿主零语义变化。
    fn charge_step(&mut self) -> Result<()> {
        Ok(())
    }

    /// **Call** — invoke a built-in command by numeric ID.
    ///
    /// `cmd_id` corresponds to a [`CmdId`](rtcl_parser::CmdId).
    fn call(&mut self, cmd_id: u16, args: &[Value]) -> Result<Value>;
}
