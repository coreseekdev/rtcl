//! Evaluation methods on [`Interp`] — script parsing and word expansion.

use super::Interp;
use crate::error::{Error, Result};
use crate::parser::{self, Command, Word};
use crate::value::Value;
use rtcl_parser::Compiler;

impl Interp {
    pub fn eval(&mut self, script: &str) -> Result<Value> {
        let commands = parser::parse(script)?;
        self.eval_commands(&commands)
    }

    /// Compile a script to bytecode (caching it) and execute via the VM.
    pub fn eval_compiled(&mut self, script: &str) -> Result<Value> {
        let code = if let Some(cached) = self.code_cache.get(script) {
            cached.clone()
        } else {
            let compiled = Compiler::compile_script(script)
                .map_err(|e| Error::syntax(e.to_string(), 0, 0))?;
            self.code_cache.insert(script.to_string(), compiled.clone());
            compiled
        };
        rtcl_vm::execute(self, &code)
    }

    pub fn eval_commands(&mut self, commands: &[Command]) -> Result<Value> {
        let mut result = Value::empty();
        for cmd in commands {
            result = self.eval_command(cmd)?;
        }
        self.result = result.clone();
        Ok(result)
    }

    fn eval_command(&mut self, cmd: &Command) -> Result<Value> {
        if cmd.words.is_empty() {
            return Ok(Value::empty());
        }
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        // Evaluate all words, handling {*} expand
        let mut args = Vec::with_capacity(cmd.words.len());
        for word in &cmd.words {
            if let Word::Expand(inner) = word {
                let value = self.eval_word(inner)?;
                if let Some(items) = value.as_list() {
                    for item in items {
                        args.push(item);
                    }
                } else {
                    args.push(value);
                }
            } else {
                let value = self.eval_word(word)?;
                args.push(value);
            }
        }

        let cmd_name = args[0].as_str();

        // User-defined procs first
        if let Some(proc_def) = self.procs.get(cmd_name).cloned() {
            return self.call_proc(&proc_def, &args);
        }

        // Built-in commands
        let func = self.commands.get(cmd_name).cloned();
        match func {
            Some(f) => {
                self.call_depth += 1;
                let result = f(self, &args);
                self.call_depth -= 1;
                result
            }
            None => Err(Error::invalid_command(cmd_name)),
        }
    }

    /// Evaluate a word to get its value.
    pub(crate) fn eval_word(&mut self, word: &Word) -> Result<Value> {
        match word {
            Word::Literal(s) => Ok(Value::from_str(s)),
            Word::VarRef(name) => self.get_var(name).cloned(),
            Word::CommandSub(cmd) => self.eval(cmd),
            Word::Concat(parts) => {
                let mut result = String::new();
                for part in parts {
                    let value = self.eval_word(part)?;
                    result.push_str(value.as_str());
                }
                Ok(Value::from_str(&result))
            }
            Word::Expand(inner) => self.eval_word(inner),
            Word::ExprSugar(expr) => self.eval_expr(expr),
        }
    }

    /// Evaluate an expression.
    pub fn eval_expr(&mut self, expr: &str) -> Result<Value> {
        crate::types::expr::eval_expr(self, expr)
    }
}
