//! [`VmContext`] implementation — bridges the VM executor and the interpreter.

use super::Interp;
use crate::error::{Error, Result};
use crate::value::Value;
use rtcl_vm::VmContext;

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
        let current = match Interp::get_var(self, name) {
            Ok(v) => v.clone(),
            Err(_) => Value::from_int(0),
        };
        let int_val = current.as_int().ok_or_else(|| {
            Error::type_mismatch("integer", current.as_str())
        })?;
        let new_val = Value::from_int(int_val + amount);
        Interp::set_var(self, name, new_val.clone())?;
        Ok(new_val)
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
            return self.call_proc(&proc_def, args);
        }

        // Built-in commands
        if let Some(f) = self.commands.get(cmd_name).cloned() {
            self.call_depth += 1;
            let result = f(self, args);
            self.call_depth -= 1;
            return result;
        }

        Err(Error::invalid_command(cmd_name))
    }

    fn call(&mut self, cmd_id: u16, args: &[Value]) -> Result<Value> {
        let func = self.resolve_cmd(cmd_id);
        match func {
            Some(f) => {
                self.call_depth += 1;
                let result = f(self, args);
                self.call_depth -= 1;
                result
            }
            None => Err(Error::runtime(
                format!("unknown command id {}", cmd_id),
                crate::error::ErrorCode::Generic,
            )),
        }
    }
}
