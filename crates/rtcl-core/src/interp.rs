//! Tcl interpreter - executes parsed commands

use crate::command::CommandFunc;
use crate::error::{Error, Result};
use crate::parser::{self, Command, Word};
use crate::value::Value;

#[cfg(feature = "std")]
use std::collections::HashMap;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
use alloc::collections::BTreeMap as HashMap;

/// A procedure definition
#[derive(Debug, Clone)]
struct ProcDef {
    /// Parameter names
    params: Vec<String>,
    /// Procedure body
    body: String,
}

/// Tcl interpreter
pub struct Interp {
    /// Variables (global scope)
    vars: HashMap<String, Value>,
    /// Commands (built-in and registered)
    commands: HashMap<String, CommandFunc>,
    /// User-defined procedures
    procs: HashMap<String, ProcDef>,
    /// Call stack depth (for recursion limit)
    call_depth: usize,
    /// Maximum call depth
    max_call_depth: usize,
    /// Last result
    result: Value,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    /// Create a new interpreter
    pub fn new() -> Self {
        let mut interp = Interp {
            vars: HashMap::new(),
            commands: HashMap::new(),
            procs: HashMap::new(),
            call_depth: 0,
            max_call_depth: 1000,
            result: Value::empty(),
        };

        // Register built-in commands
        interp.register_builtins();
        interp
    }

    /// Register all built-in commands
    fn register_builtins(&mut self) {
        // Core commands
        self.register_builtin("set", Self::cmd_set);
        self.register_builtin("puts", Self::cmd_puts);
        self.register_builtin("if", Self::cmd_if);
        self.register_builtin("while", Self::cmd_while);
        self.register_builtin("for", Self::cmd_for);
        self.register_builtin("foreach", Self::cmd_foreach);
        self.register_builtin("break", Self::cmd_break);
        self.register_builtin("continue", Self::cmd_continue);
        self.register_builtin("return", Self::cmd_return);
        self.register_builtin("proc", Self::cmd_proc);
        self.register_builtin("expr", Self::cmd_expr);
        self.register_builtin("string", Self::cmd_string);
        self.register_builtin("list", Self::cmd_list);
        self.register_builtin("llength", Self::cmd_llength);
        self.register_builtin("lindex", Self::cmd_lindex);
        self.register_builtin("lappend", Self::cmd_lappend);
        self.register_builtin("concat", Self::cmd_concat);
        self.register_builtin("append", Self::cmd_append);
        self.register_builtin("incr", Self::cmd_incr);
        self.register_builtin("catch", Self::cmd_catch);
        self.register_builtin("error", Self::cmd_error);
        self.register_builtin("global", Self::cmd_global);
        self.register_builtin("unset", Self::cmd_unset);
        self.register_builtin("info", Self::cmd_info);
        self.register_builtin("rename", Self::cmd_rename);
        self.register_builtin("eval", Self::cmd_eval);
        self.register_builtin("uplevel", Self::cmd_uplevel);
    }

    /// Register a built-in command
    fn register_builtin(&mut self, name: &str, func: CommandFunc) {
        self.commands.insert(name.to_string(), func);
    }

    /// Register a custom command
    pub fn register_command(&mut self, name: &str, func: CommandFunc) {
        self.commands.insert(name.to_string(), func);
    }

    /// Delete a command
    pub fn delete_command(&mut self, name: &str) -> Result<()> {
        if self.commands.remove(name).is_none() {
            return Err(Error::invalid_command(name));
        }
        Ok(())
    }

    /// Check if a command exists
    pub fn command_exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// Get a variable value
    pub fn get_var(&self, name: &str) -> Result<&Value> {
        // Handle array references
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.get(&full_key).ok_or_else(|| Error::var_not_found(name))
        } else {
            self.vars.get(name).ok_or_else(|| Error::var_not_found(name))
        }
    }

    /// Set a variable value
    pub fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        // Handle array references
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.insert(full_key, value.clone());
            // Also mark the array name as existing
            if !self.vars.contains_key(array_name) {
                self.vars.insert(array_name.to_string(), Value::empty());
            }
        } else {
            self.vars.insert(name.to_string(), value.clone());
        }
        Ok(value)
    }

    /// Unset a variable
    pub fn unset_var(&mut self, name: &str) -> Result<()> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.remove(&full_key);
        } else {
            self.vars.remove(name);
        }
        Ok(())
    }

    /// Check if a variable exists
    pub fn var_exists(&self, name: &str) -> bool {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.contains_key(&full_key)
        } else {
            self.vars.contains_key(name)
        }
    }

    /// Get the last result
    pub fn result(&self) -> &Value {
        &self.result
    }

    /// Evaluate a string of Tcl code
    pub fn eval(&mut self, script: &str) -> Result<Value> {
        let commands = parser::parse(script)?;
        self.eval_commands(&commands)
    }

    /// Evaluate parsed commands
    pub fn eval_commands(&mut self, commands: &[Command]) -> Result<Value> {
        let mut result = Value::empty();

        for cmd in commands {
            result = self.eval_command(cmd)?;
        }

        self.result = result.clone();
        Ok(result)
    }

    /// Evaluate a single command
    fn eval_command(&mut self, cmd: &Command) -> Result<Value> {
        if cmd.words.is_empty() {
            return Ok(Value::empty());
        }

        // Recursion check
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        // Evaluate all words
        let mut args = Vec::with_capacity(cmd.words.len());
        for word in &cmd.words {
            let value = self.eval_word(word)?;
            args.push(value);
        }

        // Get command name
        let cmd_name = args[0].as_str();

        // First check if it's a user-defined proc
        if let Some(proc) = self.procs.get(cmd_name).cloned() {
            return self.call_proc(&proc, &args);
        }

        // Then check built-in commands
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

    /// Call a user-defined procedure
    fn call_proc(&mut self, proc: &ProcDef, args: &[Value]) -> Result<Value> {
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        // Save current variables that might be shadowed
        let mut saved_vars = Vec::new();

        // Clone params to avoid borrow issues
        let params = proc.params.clone();

        // Bind parameters to arguments
        for (i, param) in params.iter().enumerate() {
            let value = if i + 1 < args.len() {
                args[i + 1].clone()
            } else {
                Value::empty()
            };

            // Save old value if it exists
            if let Some(old) = self.vars.get(param) {
                saved_vars.push((param.clone(), Some(old.clone())));
            } else {
                saved_vars.push((param.clone(), None));
            }

            // Set new value
            self.vars.insert(param.clone(), value);
        }

        // Execute the body
        self.call_depth += 1;
        let result = self.eval(&proc.body);
        self.call_depth -= 1;

        // Restore saved variables
        for (param, old_value) in saved_vars {
            if let Some(v) = old_value {
                self.vars.insert(param, v);
            } else {
                self.vars.remove(&param);
            }
        }
        // Handle return control flow
        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_return() {
                    // Extract return value
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

    /// Evaluate a word to get its value
    fn eval_word(&mut self, word: &Word) -> Result<Value> {
        match word {
            Word::Literal(s) => Ok(Value::from_str(s)),
            Word::VarRef(name) => {
                self.get_var(name).cloned()
            }
            Word::CommandSub(cmd) => {
                self.eval(cmd)
            }
            Word::Concat(parts) => {
                let mut result = String::new();
                for part in parts {
                    let value = self.eval_word(part)?;
                    result.push_str(value.as_str());
                }
                Ok(Value::from_str(&result))
            }
        }
    }

    // ==================== Built-in Commands ====================

    /// set command - get or set a variable
    fn cmd_set(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        match args.len() {
            2 => {
                // Get variable
                interp.get_var(args[1].as_str()).cloned()
            }
            3 => {
                // Set variable
                interp.set_var(args[1].as_str(), args[2].clone())
            }
            _ => Err(Error::wrong_args_with_usage("set", 2, args.len(), "varName ?newValue?")),
        }
    }

    /// puts command - output a string
    #[cfg(feature = "std")]
    fn cmd_puts(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        let mut i = 1;
        let mut newline = true;
        let mut channel = "stdout";

        // Handle -nonewline flag
        if args.len() > 1 && args[1].as_str() == "-nonewline" {
            newline = false;
            i += 1;
        }

        // Check for channel argument
        if args.len() > i + 1 {
            channel = args[i].as_str();
            i += 1;
        }

        if i >= args.len() {
            return Err(Error::wrong_args("puts", 2, args.len()));
        }

        let text = args[i].as_str();

        match channel {
            "stdout" => {
                if newline {
                    println!("{}", text);
                } else {
                    print!("{}", text);
                }
                std::io::Write::flush(&mut std::io::stdout()).ok();
            }
            "stderr" => {
                if newline {
                    eprintln!("{}", text);
                } else {
                    eprint!("{}", text);
                }
                std::io::Write::flush(&mut std::io::stderr()).ok();
            }
            _ => {
                return Err(Error::runtime(
                    format!("unknown channel: {}", channel),
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
        }

        Ok(Value::empty())
    }

    #[cfg(not(feature = "std"))]
    fn cmd_puts(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
        // In no-std, we can't actually output
        Ok(Value::empty())
    }

    /// if command
    fn cmd_if(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args("if", 3, args.len()));
        }

        let expr = args[1].as_str();
        let cond = interp.eval_expr(expr)?;

        if cond.is_true() {
            return interp.eval(args[2].as_str());
        }

        // Check for elseif/else
        let mut i = 3;
        while i < args.len() {
            let word = args[i].as_str();
            match word {
                "elseif" => {
                    if i + 2 >= args.len() {
                        return Err(Error::wrong_args("elseif", 2, args.len() - i));
                    }
                    let expr = args[i + 1].as_str();
                    let cond = interp.eval_expr(expr)?;
                    if cond.is_true() {
                        return interp.eval(args[i + 2].as_str());
                    }
                    i += 3;
                }
                "else" => {
                    if i + 1 >= args.len() {
                        return Err(Error::wrong_args("else", 1, args.len() - i));
                    }
                    return interp.eval(args[i + 1].as_str());
                }
                _ => {
                    // Assume it's a script (implicit else)
                    return interp.eval(word);
                }
            }
        }

        Ok(Value::empty())
    }

    /// while command
    fn cmd_while(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 3 {
            return Err(Error::wrong_args_with_usage("while", 3, args.len(), "test body"));
        }

        let test = args[1].as_str();
        let body = args[2].as_str();
        let mut result = Value::empty();

        loop {
            let cond = interp.eval_expr(test)?;
            if !cond.is_true() {
                break;
            }

            match interp.eval(body) {
                Ok(v) => result = v,
                Err(e) => {
                    if e.is_break() {
                        break;
                    }
                    if e.is_continue() {
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Ok(result)
    }

    /// for command
    fn cmd_for(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 5 {
            return Err(Error::wrong_args_with_usage("for", 5, args.len(), "start test next body"));
        }

        let start = args[1].as_str();
        let test = args[2].as_str();
        let next = args[3].as_str();
        let body = args[4].as_str();

        // Execute start
        interp.eval(start)?;

        let mut result = Value::empty();

        loop {
            let cond = interp.eval_expr(test)?;
            if !cond.is_true() {
                break;
            }

            match interp.eval(body) {
                Ok(v) => result = v,
                Err(e) => {
                    if e.is_break() {
                        break;
                    }
                    if e.is_continue() {
                        // Fall through to execute next
                    } else {
                        return Err(e);
                    }
                }
            }

            // Execute next
            interp.eval(next)?;
        }

        Ok(result)
    }

    /// foreach command
    fn cmd_foreach(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 4 || args.len() % 2 != 0 {
            return Err(Error::wrong_args_with_usage(
                "foreach",
                4,
                args.len(),
                "varname list body ?varname list body ...?",
            ));
        }

        let body = args[args.len() - 1].as_str();
        let mut result = Value::empty();

        // Parse var-list pairs
        let mut var_lists: Vec<(&str, Vec<Value>)> = Vec::new();
        let mut i = 1;
        while i < args.len() - 1 {
            let var = args[i].as_str();
            let list = args[i + 1].as_list().unwrap_or_default();
            var_lists.push((var, list));
            i += 2;
        }

        // Find maximum list length
        let max_len = var_lists.iter().map(|(_, l)| l.len()).max().unwrap_or(0);

        // Iterate
        for idx in 0..max_len {
            for (var, list) in &var_lists {
                let value = list.get(idx).cloned().unwrap_or_else(Value::empty);
                interp.set_var(var, value)?;
            }

            match interp.eval(body) {
                Ok(v) => result = v,
                Err(e) => {
                    if e.is_break() {
                        break;
                    }
                    if e.is_continue() {
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Ok(result)
    }

    /// break command
    fn cmd_break(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
        Err(Error::brk())
    }

    /// continue command
    fn cmd_continue(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
        Err(Error::cont())
    }

    /// return command
    fn cmd_return(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        let value = if args.len() > 1 {
            args[1].clone()
        } else {
            Value::empty()
        };
        Err(Error::ret(Some(value.as_str().to_string())))
    }

    /// proc command - define a procedure
    fn cmd_proc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 4 {
            return Err(Error::wrong_args_with_usage("proc", 4, args.len(), "name args body"));
        }

        let name = args[1].as_str().to_string();
        let params_str = args[2].as_str();
        let body = args[3].as_str().to_string();

        // Parse parameter list
        let params: Vec<String> = params_str
            .split_whitespace()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Store the procedure
        interp.procs.insert(name, ProcDef { params, body });

        Ok(Value::empty())
    }

    /// expr command - evaluate an expression
    fn cmd_expr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("expr", 2, args.len()));
        }

        // Concatenate all arguments
        let mut expr_str = String::new();
        for arg in &args[1..] {
            if !expr_str.is_empty() {
                expr_str.push(' ');
            }
            expr_str.push_str(arg.as_str());
        }

        interp.eval_expr(&expr_str)
    }

    /// Evaluate an expression
    fn eval_expr(&mut self, expr: &str) -> Result<Value> {
        crate::types::expr::eval_expr(self, expr)
    }

    /// string command - string operations
    fn cmd_string(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args("string", 3, args.len()));
        }

        let subcmd = args[1].as_str();
        let str_val = args[2].as_str();

        match subcmd {
            "length" => Ok(Value::from_int(str_val.len() as i64)),
            "tolower" => Ok(Value::from_str(&str_val.to_lowercase())),
            "toupper" => Ok(Value::from_str(&str_val.to_uppercase())),
            "trim" => {
                let chars = if args.len() > 3 {
                    args[3].as_str()
                } else {
                    " \t\n\r"
                };
                Ok(Value::from_str(str_val.trim_matches(|c| chars.contains(c))))
            }
            "trimleft" => {
                let chars = if args.len() > 3 {
                    args[3].as_str()
                } else {
                    " \t\n\r"
                };
                Ok(Value::from_str(str_val.trim_start_matches(|c| chars.contains(c))))
            }
            "trimright" => {
                let chars = if args.len() > 3 {
                    args[3].as_str()
                } else {
                    " \t\n\r"
                };
                Ok(Value::from_str(str_val.trim_end_matches(|c| chars.contains(c))))
            }
            "range" => {
                if args.len() != 5 {
                    return Err(Error::wrong_args("string range", 5, args.len()));
                }
                let start: usize = args[3].as_int().unwrap_or(0) as usize;
                let end: usize = args[4].as_int().unwrap_or(str_val.len() as i64) as usize;
                let end = end.min(str_val.len() - 1);
                if start <= end && start < str_val.len() {
                    Ok(Value::from_str(&str_val[start..=end]))
                } else {
                    Ok(Value::empty())
                }
            }
            "index" => {
                if args.len() != 4 {
                    return Err(Error::wrong_args("string index", 4, args.len()));
                }
                let idx: usize = args[3].as_int().unwrap_or(-1) as usize;
                if idx < str_val.len() {
                    Ok(Value::from_str(&str_val[idx..idx + 1]))
                } else {
                    Ok(Value::empty())
                }
            }
            "equal" => {
                if args.len() != 4 {
                    return Err(Error::wrong_args("string equal", 4, args.len()));
                }
                let other = args[3].as_str();
                Ok(Value::from_bool(str_val == other))
            }
            "match" => {
                if args.len() != 4 {
                    return Err(Error::wrong_args("string match", 4, args.len()));
                }
                let pattern = args[3].as_str();
                Ok(Value::from_bool(glob_match(pattern, str_val)))
            }
            "first" => {
                if args.len() < 4 {
                    return Err(Error::wrong_args("string first", 4, args.len()));
                }
                let needle = args[3].as_str();
                let start = if args.len() > 4 {
                    args[4].as_int().unwrap_or(0) as usize
                } else {
                    0
                };
                let pos = str_val[start..].find(needle)
                    .map(|i| (i + start) as i64)
                    .unwrap_or(-1);
                Ok(Value::from_int(pos))
            }
            "last" => {
                if args.len() < 4 {
                    return Err(Error::wrong_args("string last", 4, args.len()));
                }
                let needle = args[3].as_str();
                let pos = str_val.rfind(needle)
                    .map(|i| i as i64)
                    .unwrap_or(-1);
                Ok(Value::from_int(pos))
            }
            _ => Err(Error::runtime(
                format!("unknown string subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    }

    /// list command
    fn cmd_list(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        Ok(Value::from_list(&args[1..]))
    }

    /// llength command
    fn cmd_llength(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 2 {
            return Err(Error::wrong_args("llength", 2, args.len()));
        }
        let list = args[1].as_list().unwrap_or_default();
        Ok(Value::from_int(list.len() as i64))
    }

    /// lindex command
    fn cmd_lindex(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args("lindex", 3, args.len()));
        }
        let list = args[1].as_list().unwrap_or_default();
        let idx: usize = args[2].as_int().unwrap_or(-1) as usize;
        if idx < list.len() {
            Ok(list[idx].clone())
        } else {
            Ok(Value::empty())
        }
    }

    /// lappend command
    fn cmd_lappend(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("lappend", 2, args.len()));
        }

        let var_name = args[1].as_str();
        let mut list = interp.get_var(var_name)
            .ok()
            .and_then(|v| v.as_list())
            .unwrap_or_default();

        for arg in &args[2..] {
            list.push(arg.clone());
        }

        let result = Value::from_list(&list);
        interp.set_var(var_name, result.clone())
    }

    /// concat command
    fn cmd_concat(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        let mut result = String::new();
        for arg in &args[1..] {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(arg.as_str());
        }
        Ok(Value::from_str(&result))
    }

    /// append command
    fn cmd_append(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("append", 2, args.len()));
        }

        let var_name = args[1].as_str();
        let mut current = interp.get_var(var_name)
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();

        for arg in &args[2..] {
            current.push_str(arg.as_str());
        }

        let result = Value::from_str(&current);
        interp.set_var(var_name, result.clone())
    }

    /// incr command
    fn cmd_incr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(Error::wrong_args_with_usage("incr", 2, args.len(), "varName ?increment?"));
        }

        let var_name = args[1].as_str();
        let increment = if args.len() == 3 {
            args[2].as_int().ok_or_else(|| Error::type_mismatch("integer", "value"))?
        } else {
            1
        };

        let current = interp.get_var(var_name)
            .ok()
            .and_then(|v| v.as_int())
            .unwrap_or(0);

        let new_value = current + increment;
        interp.set_var(var_name, Value::from_int(new_value))
    }

    /// catch command
    fn cmd_catch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("catch", 2, args.len()));
        }

        let script = args[1].as_str();
        let result_var = if args.len() > 2 {
            Some(args[2].as_str())
        } else {
            None
        };

        match interp.eval(script) {
            Ok(v) => {
                if let Some(var) = result_var {
                    interp.set_var(var, v)?;
                }
                Ok(Value::from_int(0))
            }
            Err(e) => {
                if let Some(var) = result_var {
                    interp.set_var(var, Value::from_str(&e.to_string()))?;
                }
                Ok(Value::from_int(e.code() as i64))
            }
        }
    }

    /// error command
    fn cmd_error(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("error", 2, args.len()));
        }
        Err(Error::Msg(args[1].as_str().to_string()))
    }

    /// global command
    fn cmd_global(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
        // In our simple implementation, all variables are global
        Ok(Value::empty())
    }

    /// unset command
    fn cmd_unset(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("unset", 2, args.len()));
        }

        for arg in &args[1..] {
            interp.unset_var(arg.as_str())?;
        }

        Ok(Value::empty())
    }

    /// info command
    fn cmd_info(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("info", 2, args.len()));
        }

        let subcmd = args[1].as_str();

        match subcmd {
            "exists" => {
                if args.len() != 3 {
                    return Err(Error::wrong_args("info exists", 3, args.len()));
                }
                let var = args[2].as_str();
                Ok(Value::from_bool(interp.var_exists(var)))
            }
            "vars" => {
                let pattern = if args.len() > 2 {
                    args[2].as_str()
                } else {
                    "*"
                };
                let vars: Vec<Value> = interp.vars.keys()
                    .filter(|k| !k.contains('(')) // Skip array elements
                    .filter(|k| glob_match(pattern, k))
                    .map(|k| Value::from_str(k))
                    .collect();
                Ok(Value::from_list(&vars))
            }
            "commands" => {
                let pattern = if args.len() > 2 {
                    args[2].as_str()
                } else {
                    "*"
                };
                let cmds: Vec<Value> = interp.commands.keys()
                    .filter(|k| glob_match(pattern, k))
                    .map(|k| Value::from_str(k))
                    .collect();
                Ok(Value::from_list(&cmds))
            }
            "level" => Ok(Value::from_int(0)), // Simple implementation
            "body" => {
                if args.len() != 3 {
                    return Err(Error::wrong_args("info body", 3, args.len()));
                }
                // Would need to track proc bodies
                Ok(Value::empty())
            }
            "args" => {
                if args.len() != 3 {
                    return Err(Error::wrong_args("info args", 3, args.len()));
                }
                // Would need to track proc args
                Ok(Value::empty())
            }
            "version" => Ok(Value::from_str(crate::VERSION)),
            _ => Err(Error::runtime(
                format!("unknown info subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    }

    /// rename command
    fn cmd_rename(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(Error::wrong_args_with_usage("rename", 2, args.len(), "oldName ?newName?"));
        }

        let old_name = args[1].as_str();

        if args.len() == 2 {
            // Delete command
            interp.delete_command(old_name)?;
            Ok(Value::empty())
        } else {
            // Rename command
            let new_name = args[2].as_str();
            let func = interp.commands.remove(old_name)
                .ok_or_else(|| Error::invalid_command(old_name))?;
            interp.commands.insert(new_name.to_string(), func);
            Ok(Value::empty())
        }
    }

    /// eval command
    fn cmd_eval(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("eval", 2, args.len()));
        }

        let mut script = String::new();
        for arg in &args[1..] {
            if !script.is_empty() {
                script.push(' ');
            }
            script.push_str(arg.as_str());
        }

        interp.eval(&script)
    }

    /// uplevel command
    fn cmd_uplevel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        // Simplified uplevel - just evaluates in current scope
        if args.len() < 2 {
            return Err(Error::wrong_args("uplevel", 2, args.len()));
        }

        let mut script = String::new();
        for arg in &args[1..] {
            if !script.is_empty() {
                script.push(' ');
            }
            script.push_str(arg.as_str());
        }

        interp.eval(&script)
    }
}

/// Split an array reference into (array_name, index)
fn split_array_ref(name: &str) -> Option<(&str, &str)> {
    let paren = name.find('(')?;
    let end_paren = name.rfind(')')?;
    if end_paren > paren {
        Some((&name[..paren], &name[paren + 1..end_paren]))
    } else {
        None
    }
}

/// Simple glob pattern matching
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    fn match_helper(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                // Try matching * with nothing, or with one+ characters
                match_helper(&pattern[1..], text) ||
                    (!text.is_empty() && match_helper(pattern, &text[1..]))
            }
            (Some('?'), Some(_)) => {
                match_helper(&pattern[1..], &text[1..])
            }
            (Some(p), Some(t)) if *p == *t => {
                match_helper(&pattern[1..], &text[1..])
            }
            (Some(p), None) if *p == '*' => {
                match_helper(&pattern[1..], text)
            }
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
