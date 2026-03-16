//! Tcl interpreter — executes parsed commands.
//!
//! The interpreter is split into:
//! - This file: [`Interp`] struct, core eval loop, variable access, utilities
//! - [`commands`] submodules: individual command implementations

pub mod commands;

use crate::command::CommandFunc;
use crate::error::{Error, Result};
use crate::parser::{self, Command, Word};
use crate::value::Value;
use rtcl_vm::{ByteCode, Compiler};

#[cfg(feature = "std")]
use std::collections::HashMap;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::collections::BTreeMap as HashMap;

/// A procedure definition.
#[derive(Debug, Clone)]
pub(crate) struct ProcDef {
    pub params: Vec<(String, Option<String>)>,
    pub body: String,
}

/// Tcl interpreter.
pub struct Interp {
    /// Variables (global scope).
    pub(crate) vars: HashMap<String, Value>,
    /// Commands (built-in and registered).
    pub(crate) commands: HashMap<String, CommandFunc>,
    /// User-defined procedures.
    pub(crate) procs: HashMap<String, ProcDef>,
    /// Call stack depth (for recursion limit).
    pub(crate) call_depth: usize,
    /// Maximum call depth.
    pub(crate) max_call_depth: usize,
    /// Last result.
    pub(crate) result: Value,
    /// Bytecode cache — keyed by script source.
    pub(crate) code_cache: HashMap<String, ByteCode>,
    /// Current script name (for info script).
    #[cfg(feature = "std")]
    pub(crate) script_name: String,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    /// Create a new interpreter.
    pub fn new() -> Self {
        let mut interp = Interp {
            vars: HashMap::new(),
            commands: HashMap::new(),
            procs: HashMap::new(),
            call_depth: 0,
            max_call_depth: 1000,
            result: Value::empty(),
            code_cache: HashMap::new(),
            #[cfg(feature = "std")]
            script_name: String::new(),
        };
        interp.register_builtins();
        interp
    }

    /// Register all built-in commands.
    fn register_builtins(&mut self) {
        use commands::*;

        // Core
        self.register_builtin("set", misc::cmd_set);
        self.register_builtin("puts", io::cmd_puts);
        self.register_builtin("if", control::cmd_if);
        self.register_builtin("while", control::cmd_while);
        self.register_builtin("for", control::cmd_for);
        self.register_builtin("foreach", control::cmd_foreach);
        self.register_builtin("switch", control::cmd_switch);
        self.register_builtin("break", control::cmd_break);
        self.register_builtin("continue", control::cmd_continue);
        self.register_builtin("return", control::cmd_return);
        self.register_builtin("exit", control::cmd_exit);
        self.register_builtin("proc", proc::cmd_proc);
        self.register_builtin("expr", misc::cmd_expr);
        self.register_builtin("string", string_cmds::cmd_string);

        // List
        self.register_builtin("list", list::cmd_list);
        self.register_builtin("llength", list::cmd_llength);
        self.register_builtin("lindex", list::cmd_lindex);
        self.register_builtin("lappend", list::cmd_lappend);
        self.register_builtin("lrange", list::cmd_lrange);
        self.register_builtin("lsearch", list::cmd_lsearch);
        self.register_builtin("lsort", list::cmd_lsort);
        self.register_builtin("linsert", list::cmd_linsert);
        self.register_builtin("lreplace", list::cmd_lreplace);
        self.register_builtin("lassign", list::cmd_lassign);
        self.register_builtin("lrepeat", list::cmd_lrepeat);
        self.register_builtin("lreverse", list::cmd_lreverse);
        self.register_builtin("concat", list::cmd_concat);
        self.register_builtin("split", list::cmd_split);
        self.register_builtin("join", list::cmd_join);
        self.register_builtin("lmap", list::cmd_lmap);

        // Misc
        self.register_builtin("append", misc::cmd_append);
        self.register_builtin("subst", misc::cmd_subst);
        self.register_builtin("incr", misc::cmd_incr);
        self.register_builtin("catch", control::cmd_catch);
        self.register_builtin("error", control::cmd_error);
        self.register_builtin("global", proc::cmd_global);
        self.register_builtin("upvar", proc::cmd_upvar);
        self.register_builtin("unset", misc::cmd_unset);
        self.register_builtin("info", misc::cmd_info);
        self.register_builtin("rename", proc::cmd_rename);
        self.register_builtin("eval", proc::cmd_eval);
        self.register_builtin("uplevel", proc::cmd_uplevel);
        self.register_builtin("disassemble", misc::cmd_disassemble);

        // Dict / Array
        self.register_builtin("dict", dict::cmd_dict);
        self.register_builtin("array", array::cmd_array);

        // Format (no-std compatible)
        self.register_builtin("format", io::cmd_format);

        // std-only
        #[cfg(feature = "std")]
        {
            self.register_builtin("source", io::cmd_source);
            self.register_builtin("file", io::cmd_file);
            self.register_builtin("glob", io::cmd_glob);
        }
    }

    // -- Command registration ------------------------------------------------

    fn register_builtin(&mut self, name: &str, func: CommandFunc) {
        self.commands.insert(name.to_string(), func);
    }

    pub fn register_command(&mut self, name: &str, func: CommandFunc) {
        self.commands.insert(name.to_string(), func);
    }

    pub fn delete_command(&mut self, name: &str) -> Result<()> {
        if self.commands.remove(name).is_none() {
            return Err(Error::invalid_command(name));
        }
        Ok(())
    }

    pub fn command_exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    // -- Variable access -----------------------------------------------------

    pub fn get_var(&self, name: &str) -> Result<&Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.get(&full_key).ok_or_else(|| Error::var_not_found(name))
        } else {
            self.vars.get(name).ok_or_else(|| Error::var_not_found(name))
        }
    }

    pub fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.insert(full_key, value.clone());
            if !self.vars.contains_key(array_name) {
                self.vars.insert(array_name.to_string(), Value::empty());
            }
        } else {
            self.vars.insert(name.to_string(), value.clone());
        }
        Ok(value)
    }

    pub fn unset_var(&mut self, name: &str) -> Result<()> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.remove(&full_key);
        } else {
            self.vars.remove(name);
        }
        Ok(())
    }

    pub fn var_exists(&self, name: &str) -> bool {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.contains_key(&full_key)
        } else {
            self.vars.contains_key(name)
        }
    }

    pub fn result(&self) -> &Value {
        &self.result
    }

    #[cfg(feature = "std")]
    pub fn set_script_name(&mut self, name: &str) {
        self.script_name = name.to_string();
    }

    #[cfg(feature = "std")]
    pub fn script_name(&self) -> &str {
        &self.script_name
    }

    // -- Eval ----------------------------------------------------------------

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
                .map_err(|e| Error::syntax(&e.to_string(), 0, 0))?;
            self.code_cache.insert(script.to_string(), compiled.clone());
            compiled
        };
        crate::vm::execute(self, &code)
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

    /// Call a user-defined procedure.
    pub(crate) fn call_proc(&mut self, proc_def: &ProcDef, args: &[Value]) -> Result<Value> {
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        let mut saved_vars = Vec::new();
        let params = proc_def.params.clone();
        let has_args = params.last().map(|(p, _)| p.as_str()) == Some("args");
        let regular_params = if has_args {
            &params[..params.len() - 1]
        } else {
            &params[..]
        };

        for (i, (param, default)) in regular_params.iter().enumerate() {
            let value = if i + 1 < args.len() {
                args[i + 1].clone()
            } else if let Some(d) = default {
                Value::from_str(d)
            } else {
                Value::empty()
            };
            if let Some(old) = self.vars.get(param.as_str()) {
                saved_vars.push((param.clone(), Some(old.clone())));
            } else {
                saved_vars.push((param.clone(), None));
            }
            self.vars.insert(param.clone(), value);
        }

        if has_args {
            let remaining_start = regular_params.len() + 1;
            let remaining_args: Vec<&Value> = if remaining_start < args.len() {
                args[remaining_start..].iter().collect()
            } else {
                Vec::new()
            };
            let list_str: String = remaining_args
                .iter()
                .map(|v| {
                    let s = v.as_str();
                    if s.is_empty() || s.contains(' ') || s.contains('\t') || s.contains('\n') {
                        format!("{{{}}}", s)
                    } else {
                        s.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(old) = self.vars.get("args") {
                saved_vars.push(("args".to_string(), Some(old.clone())));
            } else {
                saved_vars.push(("args".to_string(), None));
            }
            self.vars.insert("args".to_string(), Value::from_str(&list_str));
        }

        self.call_depth += 1;
        let result = self.eval(&proc_def.body);
        self.call_depth -= 1;

        // Restore
        for (param, old_value) in saved_vars {
            if let Some(v) = old_value {
                self.vars.insert(param, v);
            } else {
                self.vars.remove(&param);
            }
        }

        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_return() {
                    if let Error::ControlFlow { value: Some(v), .. } = e {
                        Ok(Value::from_str(&v))
                    } else {
                        Ok(Value::empty())
                    }
                } else {
                    Err(e)
                }
            }
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

// ---------------------------------------------------------------------------
// Utility functions (used by command modules)
// ---------------------------------------------------------------------------

/// Split `name(index)` into `(name, index)`.
pub(crate) fn split_array_ref(name: &str) -> Option<(&str, &str)> {
    let paren = name.find('(')?;
    let end_paren = name.rfind(')')?;
    if end_paren > paren {
        Some((&name[..paren], &name[paren + 1..end_paren]))
    } else {
        None
    }
}

/// Simple glob pattern matching.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    fn match_helper(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                match_helper(&pattern[1..], text)
                    || (!text.is_empty() && match_helper(pattern, &text[1..]))
            }
            (Some('?'), Some(_)) => match_helper(&pattern[1..], &text[1..]),
            (Some(p), Some(t)) if *p == *t => match_helper(&pattern[1..], &text[1..]),
            (Some(p), None) if *p == '*' => match_helper(&pattern[1..], text),
            _ => false,
        }
    }

    match_helper(&pattern, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_var() {
        let mut interp = Interp::new();
        interp.eval("set x 42").unwrap();
        assert_eq!(interp.get_var("x").unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_expr() {
        let mut interp = Interp::new();
        let result = interp.eval("expr 1 + 2").unwrap();
        assert_eq!(result.as_int(), Some(3));
    }

    #[test]
    fn test_while() {
        let mut interp = Interp::new();
        interp.eval("set i 0").unwrap();
        interp.eval("while {$i < 5} { incr i }").unwrap();
        assert_eq!(interp.get_var("i").unwrap().as_int(), Some(5));
    }
}
