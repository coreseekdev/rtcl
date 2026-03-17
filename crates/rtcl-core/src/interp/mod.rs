//! Tcl interpreter — executes parsed commands.
//!
//! The interpreter is split into:
//! - This file: [`Interp`] struct, core eval loop, variable access, utilities
//! - [`commands`] submodules: individual command implementations

pub mod commands;

use crate::command::{CommandFunc, CommandCategory};
use crate::error::{Error, Result};
use crate::parser::{self, Command, Word};
use crate::value::Value;
use rtcl_parser::{ByteCode, Compiler, CmdId};
use rtcl_vm::VmContext;

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
    /// Command category metadata.
    pub(crate) command_categories: HashMap<String, CommandCategory>,
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
            command_categories: HashMap::new(),
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
        use CommandCategory::*;

        // -- Language builtins: core Tcl language primitives ------------------
        self.register_categorized("set", misc::cmd_set, Language);
        self.register_categorized("if", control::cmd_if, Language);
        self.register_categorized("while", control::cmd_while, Language);
        self.register_categorized("for", control::cmd_for, Language);
        self.register_categorized("foreach", control::cmd_foreach, Language);
        self.register_categorized("switch", control::cmd_switch, Language);
        self.register_categorized("break", control::cmd_break, Language);
        self.register_categorized("continue", control::cmd_continue, Language);
        self.register_categorized("return", control::cmd_return, Language);
        self.register_categorized("exit", control::cmd_exit, Language);
        self.register_categorized("proc", proc::cmd_proc, Language);
        self.register_categorized("rename", proc::cmd_rename, Language);
        self.register_categorized("eval", proc::cmd_eval, Language);
        self.register_categorized("apply", proc::cmd_apply, Language);
        self.register_categorized("uplevel", proc::cmd_uplevel, Language);
        self.register_categorized("upvar", proc::cmd_upvar, Language);
        self.register_categorized("global", proc::cmd_global, Language);
        self.register_categorized("unset", misc::cmd_unset, Language);
        self.register_categorized("expr", misc::cmd_expr, Language);
        self.register_categorized("catch", control::cmd_catch, Language);
        self.register_categorized("error", control::cmd_error, Language);
        self.register_categorized("try", control::cmd_try, Language);
        self.register_categorized("tailcall", control::cmd_tailcall, Language);
        self.register_categorized("subst", misc::cmd_subst, Language);
        self.register_categorized("incr", misc::cmd_incr, Language);
        self.register_categorized("append", misc::cmd_append, Language);
        self.register_categorized("info", misc::cmd_info, Language);

        // -- Standard library: data manipulation commands ---------------------
        self.register_categorized("string", string_cmds::cmd_string, Standard);
        self.register_categorized("list", list::cmd_list, Standard);
        self.register_categorized("llength", list::cmd_llength, Standard);
        self.register_categorized("lindex", list::cmd_lindex, Standard);
        self.register_categorized("lappend", list::cmd_lappend, Standard);
        self.register_categorized("lrange", list::cmd_lrange, Standard);
        self.register_categorized("lsearch", list::cmd_lsearch, Standard);
        self.register_categorized("lsort", list::cmd_lsort, Standard);
        self.register_categorized("linsert", list::cmd_linsert, Standard);
        self.register_categorized("lreplace", list::cmd_lreplace, Standard);
        self.register_categorized("lassign", list::cmd_lassign, Standard);
        self.register_categorized("lrepeat", list::cmd_lrepeat, Standard);
        self.register_categorized("lreverse", list::cmd_lreverse, Standard);
        self.register_categorized("concat", list::cmd_concat, Standard);
        self.register_categorized("split", list::cmd_split, Standard);
        self.register_categorized("join", list::cmd_join, Standard);
        self.register_categorized("lmap", list::cmd_lmap, Standard);
        self.register_categorized("lset", list::cmd_lset, Standard);
        self.register_categorized("dict", dict::cmd_dict, Standard);
        self.register_categorized("array", array::cmd_array, Standard);
        self.register_categorized("format", io::cmd_format, Standard);
        self.register_categorized("scan", misc::cmd_scan, Standard);
        self.register_categorized("range", control::cmd_range, Standard);
        self.register_categorized("time", control::cmd_time, Standard);
        self.register_categorized("timerate", control::cmd_timerate, Standard);

        // -- Extension: platform / optional commands -------------------------
        self.register_categorized("puts", io::cmd_puts, Extension);
        self.register_categorized("disassemble", misc::cmd_disassemble, Extension);

        #[cfg(feature = "std")]
        {
            self.register_categorized("source", io::cmd_source, Extension);
            self.register_categorized("file", io::cmd_file, Extension);
            self.register_categorized("glob", io::cmd_glob, Extension);
            self.register_categorized("regexp", regexp_cmds::cmd_regexp, Extension);
            self.register_categorized("regsub", regexp_cmds::cmd_regsub, Extension);
        }
    }

    // -- Command registration ------------------------------------------------

    fn register_categorized(&mut self, name: &str, func: CommandFunc, cat: CommandCategory) {
        self.commands.insert(name.to_string(), func);
        self.command_categories.insert(name.to_string(), cat);
    }

    /// Register an external command (always categorised as Extension).
    pub fn register_command(&mut self, name: &str, func: CommandFunc) {
        self.register_categorized(name, func, CommandCategory::Extension);
    }

    pub fn delete_command(&mut self, name: &str) -> Result<()> {
        if self.commands.remove(name).is_none() {
            return Err(Error::invalid_command(name));
        }
        self.command_categories.remove(name);
        Ok(())
    }

    pub fn command_exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    // -- Call dispatch (by numeric CmdId) ------------------------------------

    /// Map a `CmdId` (u16) to the corresponding command function.
    fn resolve_cmd(&self, cmd_id: u16) -> Option<CommandFunc> {
        use commands::*;
        Some(match cmd_id {
            // -- Language commands (standard) --
            x if x == CmdId::Foreach   as u16 => control::cmd_foreach,
            x if x == CmdId::Switch    as u16 => control::cmd_switch,
            x if x == CmdId::Try       as u16 => control::cmd_try,
            x if x == CmdId::Catch     as u16 => control::cmd_catch,
            x if x == CmdId::Proc      as u16 => proc::cmd_proc,
            x if x == CmdId::Rename    as u16 => proc::cmd_rename,
            x if x == CmdId::Eval      as u16 => proc::cmd_eval,
            x if x == CmdId::Apply     as u16 => proc::cmd_apply,
            x if x == CmdId::Uplevel   as u16 => proc::cmd_uplevel,
            x if x == CmdId::Upvar     as u16 => proc::cmd_upvar,
            x if x == CmdId::Global    as u16 => proc::cmd_global,
            x if x == CmdId::Unset     as u16 => misc::cmd_unset,
            x if x == CmdId::Subst     as u16 => misc::cmd_subst,
            x if x == CmdId::Info      as u16 => misc::cmd_info,
            x if x == CmdId::Error     as u16 => control::cmd_error,
            x if x == CmdId::Tailcall  as u16 => control::cmd_tailcall,
            x if x == CmdId::Append    as u16 => misc::cmd_append,
            x if x == CmdId::StringCmd as u16 => string_cmds::cmd_string,
            x if x == CmdId::List      as u16 => list::cmd_list,
            x if x == CmdId::Llength   as u16 => list::cmd_llength,
            x if x == CmdId::Lindex    as u16 => list::cmd_lindex,
            x if x == CmdId::Lappend   as u16 => list::cmd_lappend,
            x if x == CmdId::Lrange    as u16 => list::cmd_lrange,
            x if x == CmdId::Lsearch   as u16 => list::cmd_lsearch,
            x if x == CmdId::Lsort     as u16 => list::cmd_lsort,
            x if x == CmdId::Linsert   as u16 => list::cmd_linsert,
            x if x == CmdId::Lreplace  as u16 => list::cmd_lreplace,
            x if x == CmdId::Lassign   as u16 => list::cmd_lassign,
            x if x == CmdId::Lrepeat   as u16 => list::cmd_lrepeat,
            x if x == CmdId::Lreverse  as u16 => list::cmd_lreverse,
            x if x == CmdId::Concat    as u16 => list::cmd_concat,
            x if x == CmdId::Split     as u16 => list::cmd_split,
            x if x == CmdId::Join      as u16 => list::cmd_join,
            x if x == CmdId::Lmap      as u16 => list::cmd_lmap,
            x if x == CmdId::Lset      as u16 => list::cmd_lset,
            x if x == CmdId::Dict      as u16 => dict::cmd_dict,
            x if x == CmdId::Array     as u16 => array::cmd_array,
            x if x == CmdId::Format    as u16 => io::cmd_format,
            x if x == CmdId::Scan      as u16 => misc::cmd_scan,
            x if x == CmdId::Range     as u16 => control::cmd_range,
            x if x == CmdId::Time      as u16 => control::cmd_time,
            x if x == CmdId::Timerate  as u16 => control::cmd_timerate,
            // -- Extension commands --
            x if x == CmdId::Puts        as u16 => io::cmd_puts,
            x if x == CmdId::Disassemble as u16 => misc::cmd_disassemble,
            #[cfg(feature = "std")]
            x if x == CmdId::Source as u16 => io::cmd_source,
            #[cfg(feature = "std")]
            x if x == CmdId::File   as u16 => io::cmd_file,
            #[cfg(feature = "std")]
            x if x == CmdId::Glob   as u16 => io::cmd_glob,
            #[cfg(feature = "std")]
            x if x == CmdId::Regexp as u16 => regexp_cmds::cmd_regexp,
            #[cfg(feature = "std")]
            x if x == CmdId::Regsub as u16 => regexp_cmds::cmd_regsub,
            _ => return None,
        })
    }

    /// Return the category for a command, if it exists.
    pub fn command_category(&self, name: &str) -> Option<CommandCategory> {
        self.command_categories.get(name).copied()
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
                    match &e {
                        Error::ControlFlow { level, value, .. } => {
                            let val_str = value.clone().unwrap_or_default();
                            match *level {
                                0 => {
                                    // Plain return (level=0 means just return the value)
                                    Ok(Value::from_str(&val_str))
                                }
                                1 => {
                                    // return -code error "msg" → propagate as error
                                    Err(Error::Msg(val_str))
                                }
                                3 => {
                                    // return -code break
                                    Err(Error::brk())
                                }
                                4 => {
                                    // return -code continue
                                    Err(Error::cont())
                                }
                                _ => Ok(Value::from_str(&val_str)),
                            }
                        }
                        _ => Ok(Value::empty()),
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
// VmContext implementation — bridges the VM executor and the interpreter
// ---------------------------------------------------------------------------

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
        let current = self.vars.get(name).cloned().unwrap_or_else(|| Value::from_int(0));
        let int_val = current.as_int().ok_or_else(|| {
            Error::type_mismatch("integer", current.as_str())
        })?;
        let new_val = Value::from_int(int_val + amount);
        self.vars.insert(name.to_string(), new_val.clone());
        Ok(new_val)
    }

    fn append_var(&mut self, name: &str, value: &str) -> Result<Value> {
        let current = self.vars.get(name).cloned().unwrap_or_else(Value::empty);
        let mut s = current.as_str().to_string();
        s.push_str(value);
        let new_val = Value::from_str(&s);
        self.vars.insert(name.to_string(), new_val.clone());
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
