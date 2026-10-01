//! [`VmContext`] implementation — bridges the VM executor and the interpreter.

use super::Interp;
use crate::error::{Error, Result};
use crate::value::Value;
use rtcl_vm::VmContext;

impl Interp {
    /// Tcl renders every arity error with the command's usage text; fill it
    /// in from the registry when a builtin raised a bare WrongNumArgs
    /// (tclsh prints `should be "set varName ?newValue?"`, never
    /// `set 2 args`).
    pub(crate) fn fill_wrong_args(&self, cmd_name: &str, result: Result<Value>) -> Result<Value> {
        match result {
            Err(Error::WrongNumArgs { command, expected, actual, usage: None }) => {
                let usage = self.command_meta.get(cmd_name).and_then(|meta| {
                    if meta.usage.is_empty() {
                        None
                    } else {
                        Some(meta.usage.to_string())
                    }
                });
                Err(Error::WrongNumArgs { command, expected, actual, usage })
            }
            other => other,
        }
    }
}

impl VmContext for Interp {
    fn get_var(&self, name: &str) -> Result<Value> {
        Interp::get_var(self, name).cloned()
    }

    fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        Interp::set_var(self, name, value)
    }

    fn unset_var(&mut self, name: &str) -> Result<()> {
        Interp::unset_var(self, name)
    }

    fn var_exists(&self, name: &str) -> bool {
        Interp::var_exists(self, name)
    }

    fn incr_var(&mut self, name: &str, amount: i64) -> Result<Value> {
        Interp::incr_var(self, name, amount)
    }

    fn append_var(&mut self, name: &str, value: &str) -> Result<Value> {
        let current = match Interp::get_var(self, name) {
            Ok(v) => v.clone(),
            Err(_) => Value::empty(),
        };
        let mut s = current.as_str().to_string();
        s.push_str(value);
        let new_val = Value::from_str(&s);
        Interp::set_var(self, name, new_val.clone())?;
        Ok(new_val)
    }

    fn eval_script(&mut self, script: &str) -> Result<Value> {
        self.eval(script)
    }

    fn eval_expr(&mut self, expr: &str) -> Result<Value> {
        Interp::eval_expr(self, expr)
    }

    fn invoke_command(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Ok(Value::empty());
        }
        let cmd_name = args[0].as_str();

        // Try user-defined procs first
        if let Some(proc_def) = self.procs.get(cmd_name).cloned() {
            return self.call_proc(&proc_def, args, cmd_name, None);
        }

        // Built-in commands
        if let Some(f) = self.commands.get(cmd_name).cloned() {
            self.call_depth += 1;
            let result = f(self, args);
            self.call_depth -= 1;
            return self.fill_wrong_args(cmd_name, result);
        }

        // An unknown command raises a fresh `TCL LOOKUP COMMAND` errorCode
        // (tclsh: `set errorCode` after `nosuchcmd` → `TCL LOOKUP COMMAND
        // nosuchcmd`).
        let name = cmd_name.to_string();
        super::commands::list::set_error_code(
            self,
            &format!("TCL LOOKUP COMMAND {}", name),
        );
        Err(Error::invalid_command(name))
    }

    fn call(&mut self, cmd_id: u16, args: &[Value]) -> Result<Value> {
        let func = self.resolve_cmd(cmd_id);
        match func {
            Some(f) => {
                self.call_depth += 1;
                let result = f(self, args);
                self.call_depth -= 1;
                let name = args.first().map(|a| a.as_str()).unwrap_or("");
                self.fill_wrong_args(name, result)
            }
            None => Err(Error::runtime(
                format!("unknown command id {}", cmd_id),
                crate::error::ErrorCode::Generic,
            )),
        }
    }
}
