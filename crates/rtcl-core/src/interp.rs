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
    /// Current script name (for info script)
    #[cfg(feature = "std")]
    script_name: String,
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
            #[cfg(feature = "std")]
            script_name: String::new(),
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
        self.register_builtin("switch", Self::cmd_switch);
        self.register_builtin("break", Self::cmd_break);
        self.register_builtin("continue", Self::cmd_continue);
        self.register_builtin("return", Self::cmd_return);
        self.register_builtin("exit", Self::cmd_exit);
        self.register_builtin("proc", Self::cmd_proc);
        self.register_builtin("expr", Self::cmd_expr);
        self.register_builtin("string", Self::cmd_string);
        self.register_builtin("list", Self::cmd_list);
        self.register_builtin("llength", Self::cmd_llength);
        self.register_builtin("lindex", Self::cmd_lindex);
        self.register_builtin("lappend", Self::cmd_lappend);
        self.register_builtin("lrange", Self::cmd_lrange);
        self.register_builtin("lsearch", Self::cmd_lsearch);
        self.register_builtin("lsort", Self::cmd_lsort);
        self.register_builtin("linsert", Self::cmd_linsert);
        self.register_builtin("lreplace", Self::cmd_lreplace);
        self.register_builtin("lassign", Self::cmd_lassign);
        self.register_builtin("lrepeat", Self::cmd_lrepeat);
        self.register_builtin("lreverse", Self::cmd_lreverse);
        self.register_builtin("concat", Self::cmd_concat);
        self.register_builtin("append", Self::cmd_append);
        self.register_builtin("split", Self::cmd_split);
        self.register_builtin("join", Self::cmd_join);
        self.register_builtin("subst", Self::cmd_subst);
        self.register_builtin("incr", Self::cmd_incr);
        self.register_builtin("catch", Self::cmd_catch);
        self.register_builtin("error", Self::cmd_error);
        self.register_builtin("global", Self::cmd_global);
        self.register_builtin("upvar", Self::cmd_upvar);
        self.register_builtin("unset", Self::cmd_unset);
        self.register_builtin("info", Self::cmd_info);
        self.register_builtin("rename", Self::cmd_rename);
        self.register_builtin("eval", Self::cmd_eval);
        self.register_builtin("uplevel", Self::cmd_uplevel);
        self.register_builtin("dict", Self::cmd_dict);
        self.register_builtin("array", Self::cmd_array);
        #[cfg(feature = "std")]
        self.register_builtin("source", Self::cmd_source);
        #[cfg(feature = "std")]
        self.register_builtin("file", Self::cmd_file);
        #[cfg(feature = "std")]
        self.register_builtin("format", Self::cmd_format);
        #[cfg(feature = "std")]
        self.register_builtin("glob", Self::cmd_glob);
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

    /// Set the current script name (for info script)
    #[cfg(feature = "std")]
    pub fn set_script_name(&mut self, name: &str) {
        self.script_name = name.to_string();
    }

    /// Get the script name
    pub fn script_name(&self) -> &str {
        &self.script_name
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

        // Check if last param is 'args' for variadic support
        let has_args = params.last().map(|p| p.as_str()) == Some("args");
        let regular_params = if has_args {
            &params[..params.len() - 1]
        } else {
            &params[..]
        };

        // Bind regular parameters to arguments
        for (i, param) in regular_params.iter().enumerate() {
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

        // If 'args' parameter exists, collect remaining arguments as a list
        if has_args {
            let remaining_start = regular_params.len() + 1;
            let remaining_args: Vec<&Value> = if remaining_start < args.len() {
                args[remaining_start..].iter().collect()
            } else {
                Vec::new()
            };

            // Build list string
            let list_str: String = remaining_args.iter()
                .map(|v| {
                    let s = v.as_str();
                    // Simple quoting - if contains space or is empty, wrap in braces
                    if s.is_empty() || s.contains(' ') || s.contains('\t') || s.contains('\n') {
                        format!("{{{}}}", s)
                    } else {
                        s.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");

            // Save old value of 'args' if it exists
            if let Some(old) = self.vars.get("args") {
                saved_vars.push(("args".to_string(), Some(old.clone())));
            } else {
                saved_vars.push(("args".to_string(), None));
            }

            // Set 'args' to the list of remaining arguments
            self.vars.insert("args".to_string(), Value::from_str(&list_str));
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

            // Execute next - also handle break/continue here
            match interp.eval(next) {
                Ok(_) => {}
                Err(e) => {
                    if e.is_break() {
                        break;
                    }
                    if e.is_continue() {
                        // Continue to next iteration
                    } else {
                        return Err(e);
                    }
                }
            }
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
                // TCL_OK = 0
                Ok(Value::from_int(0))
            }
            Err(e) => {
                if let Some(var) = result_var {
                    interp.set_var(var, Value::from_str(&e.to_string()))?;
                }
                // Map to Tcl return codes:
                // TCL_ERROR = 1, TCL_RETURN = 2, TCL_BREAK = 3, TCL_CONTINUE = 4
                let code = if e.is_return() {
                    2
                } else if e.is_break() {
                    3
                } else if e.is_continue() {
                    4
                } else {
                    1 // TCL_ERROR for all other errors
                };
                Ok(Value::from_int(code))
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
            "procs" => {
                let pattern = if args.len() > 2 {
                    args[2].as_str()
                } else {
                    "*"
                };
                let procs: Vec<Value> = interp.procs.keys()
                    .filter(|k| glob_match(pattern, k))
                    .map(|k| Value::from_str(k))
                    .collect();
                Ok(Value::from_list(&procs))
            }
            "level" => Ok(Value::from_int(0)), // Simple implementation
            "body" => {
                if args.len() != 3 {
                    return Err(Error::wrong_args("info body", 3, args.len()));
                }
                let name = args[2].as_str();
                if let Some(proc) = interp.procs.get(name) {
                    Ok(Value::from_str(&proc.body))
                } else {
                    Ok(Value::empty())
                }
            }
            "args" => {
                if args.len() != 3 {
                    return Err(Error::wrong_args("info args", 3, args.len()));
                }
                let name = args[2].as_str();
                if let Some(proc) = interp.procs.get(name) {
                    Ok(Value::from_list(&proc.params.iter().map(|p| Value::from_str(p)).collect::<Vec<_>>()))
                } else {
                    Ok(Value::empty())
                }
            }
            "script" => {
                #[cfg(feature = "std")]
                {
                    Ok(Value::from_str(&interp.script_name))
                }
                #[cfg(not(feature = "std"))]
                {
                    Ok(Value::empty())
                }
            }
            "version" => Ok(Value::from_str(crate::VERSION)),
            "nameofexecutable" => {
                #[cfg(feature = "std")]
                {
                    Ok(Value::from_str(""))
                }
                #[cfg(not(feature = "std"))]
                {
                    Ok(Value::empty())
                }
            }
            _ => Err(Error::runtime(
                format!("unknown info subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            ))
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
        // uplevel ?level? arg ?arg ...?
        // Simplified uplevel - just evaluates in current scope (level is ignored)
        if args.len() < 2 {
            return Err(Error::wrong_args("uplevel", 2, args.len()));
        }

        // Determine if first arg is a level specification
        // Level can be: #N (absolute), N (relative), or omitted (default 1)
        let start_idx = if args.len() > 2 {
            let first = args[1].as_str();
            // Check if it looks like a level: starts with # or is a number
            if first.starts_with('#') || first.chars().all(|c| c.is_ascii_digit() || c == '-') {
                2 // Skip the level argument
            } else {
                1 // No level, start from first arg
            }
        } else {
            1
        };

        let mut script = String::new();
        for arg in &args[start_idx..] {
            if !script.is_empty() {
                script.push(' ');
            }
            script.push_str(arg.as_str());
        }

        interp.eval(&script)
    }

    /// switch command - multi-branch conditional
    fn cmd_switch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args_with_usage("switch", 3, args.len(), "?options? string pattern body ?pattern body ...?"));
        }

        let mut i = 1;
        let mut exact_match = false;

        // Parse options
        while i < args.len() && args[i].as_str().starts_with('-') {
            match args[i].as_str() {
                "-exact" => { exact_match = true; i += 1; }
                "-glob" => { exact_match = false; i += 1; }
                "-regexp" => {
                    return Err(Error::runtime("regexp mode not supported", crate::error::ErrorCode::InvalidOp));
                }
                "--" => { i += 1; break; }
                _ => break,
            }
        }

        if i >= args.len() {
            return Err(Error::wrong_args("switch", 3, args.len()));
        }

        let string = args[i].as_str();
        i += 1;

        // Handle single body argument (list of patterns/bodies)
        let patterns: Vec<(String, String)>;
        if args.len() - i == 1 {
            // Parse pattern/body pairs from list
            let list = args[i].as_list().unwrap_or_default();
            if list.len() % 2 != 0 {
                return Err(Error::runtime("switch list must have even number of elements", crate::error::ErrorCode::InvalidOp));
            }
            patterns = list.chunks(2)
                .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
                .collect();
        } else {
            // Pattern/body pairs as arguments
            if (args.len() - i) % 2 != 0 {
                return Err(Error::runtime("switch must have even number of pattern/body pairs", crate::error::ErrorCode::InvalidOp));
            }
            patterns = args[i..].chunks(2)
                .map(|chunk| (chunk[0].as_str().to_string(), chunk[1].as_str().to_string()))
                .collect();
        }

        // Find matching pattern
        for (pattern, body) in &patterns {
            let matches = if pattern == "default" {
                true
            } else if exact_match {
                string == pattern
            } else {
                glob_match(pattern, string)
            };

            if matches {
                if body == "-" {
                    // Fall through to next body
                    continue;
                }
                return interp.eval(body);
            }
        }

        Ok(Value::empty())
    }

    /// exit command
    fn cmd_exit(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        let code = if args.len() > 1 {
            args[1].as_int().unwrap_or(0) as i32
        } else {
            0
        };
        Err(Error::exit(Some(code)))
    }

    /// upvar command - create variable link (simplified)
    fn cmd_upvar(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "level otherVar myVar ?otherVar myVar ...?"));
        }

        // Simplified: just copy the variable value
        // In a full implementation, this would create actual references
        let mut i = 1;

        // Skip level specifier if present
        if args.len() > 3 && args[1].as_str().parse::<i32>().is_ok() {
            i += 1;
        }

        // Process var pairs
        while i + 1 < args.len() {
            let other_var = args[i].as_str();
            let my_var = args[i + 1].as_str();

            if let Ok(value) = interp.get_var(other_var) {
                interp.set_var(my_var, value.clone())?;
            }
            i += 2;
        }

        Ok(Value::empty())
    }

    /// source command - load and execute a script file
    #[cfg(feature = "std")]
    fn cmd_source(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 2 {
            return Err(Error::wrong_args("source", 2, args.len()));
        }

        let filename = args[1].as_str();
        let content = std::fs::read_to_string(filename)
            .map_err(|e| Error::runtime(
                format!("can't read file \"{}\": {}", filename, e),
                crate::error::ErrorCode::Io
            ))?;

        // Save old script name and set new one
        let old_script = interp.script_name.clone();
        interp.script_name = filename.to_string();

        let result = interp.eval(&content);

        // Restore old script name
        interp.script_name = old_script;

        result
    }

    /// lrange command - get a range of list elements
    fn cmd_lrange(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 4 {
            return Err(Error::wrong_args_with_usage("lrange", 4, args.len(), "list first last"));
        }

        let list = args[1].as_list().unwrap_or_default();
        let first = args[2].as_int().unwrap_or(0) as usize;
        let last = args[3].as_str();

        let end = if last == "end" {
            list.len().saturating_sub(1)
        } else if last.starts_with("end-") {
            let offset: usize = last[4..].parse().unwrap_or(0);
            list.len().saturating_sub(1 + offset)
        } else if last.starts_with("end+") {
            let offset: usize = last[4..].parse().unwrap_or(0);
            (list.len().saturating_sub(1)).saturating_add(offset).min(list.len().saturating_sub(1))
        } else {
            last.parse::<usize>().unwrap_or(0)
        };

        if first <= end && first < list.len() {
            let result: Vec<Value> = list[first..=end.min(list.len() - 1)].to_vec();
            Ok(Value::from_list(&result))
        } else {
            Ok(Value::empty())
        }
    }

    /// lsearch command - search for element in list
    fn cmd_lsearch(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args_with_usage("lsearch", 3, args.len(), "?options? list pattern"));
        }

        let mut i = 1;
        let mut exact = false;
        let mut all = false;
        let mut inline = false;
        let mut not_match = false;

        // Parse options
        while i < args.len() && args[i].as_str().starts_with('-') {
            match args[i].as_str() {
                "-exact" => { exact = true; i += 1; }
                "-glob" => { exact = false; i += 1; }
                "-all" => { all = true; i += 1; }
                "-inline" => { inline = true; i += 1; }
                "-not" => { not_match = true; i += 1; }
                "--" => { i += 1; break; }
                _ => break,
            }
        }

        if i + 1 >= args.len() {
            return Err(Error::wrong_args("lsearch", 3, args.len()));
        }

        let list = args[i].as_list().unwrap_or_default();
        let pattern = args[i + 1].as_str();

        let matches: Vec<(usize, &Value)> = list.iter().enumerate()
            .filter(|(_, v)| {
                let m = if exact {
                    v.as_str() == pattern
                } else {
                    glob_match(pattern, v.as_str())
                };
                if not_match { !m } else { m }
            })
            .collect();

        if inline {
            let result: Vec<Value> = matches.iter().map(|(_, v)| (*v).clone()).collect();
            Ok(Value::from_list(&result))
        } else if all {
            let result: Vec<Value> = matches.iter().map(|(idx, _)| Value::from_int(*idx as i64)).collect();
            Ok(Value::from_list(&result))
        } else {
            Ok(Value::from_int(matches.first().map(|(idx, _)| *idx as i64).unwrap_or(-1)))
        }
    }

    /// lsort command - sort a list
    fn cmd_lsort(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args_with_usage("lsort", 2, args.len(), "?options? list"));
        }

        let mut i = 1;
        let mut decreasing = false;
        let mut unique = false;
        let mut nocase = false;

        // Parse options
        while i < args.len() && args[i].as_str().starts_with('-') {
            match args[i].as_str() {
                "-decreasing" => { decreasing = true; i += 1; }
                "-increasing" => { decreasing = false; i += 1; }
                "-unique" => { unique = true; i += 1; }
                "-nocase" => { nocase = true; i += 1; }
                "-ascii" | "-dictionary" | "-integer" | "-real" => { i += 1; }
                "--" => { i += 1; break; }
                _ => break,
            }
        }

        if i >= args.len() {
            return Err(Error::wrong_args("lsort", 2, args.len()));
        }

        let mut list = args[i].as_list().unwrap_or_default();
        let mut seen = std::collections::HashSet::new();

        list.sort_by(|a, b| {
            let (a_str, b_str) = if nocase {
                (a.as_str().to_lowercase(), b.as_str().to_lowercase())
            } else {
                (a.as_str().to_string(), b.as_str().to_string())
            };

            let cmp = a_str.cmp(&b_str);
            if decreasing { cmp.reverse() } else { cmp }
        });

        if unique {
            list.retain(|v| seen.insert(v.as_str().to_string()));
        }

        Ok(Value::from_list(&list))
    }

    /// linsert command - insert elements into list
    fn cmd_linsert(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args_with_usage("linsert", 3, args.len(), "list index ?element ...?"));
        }

            let list = args[1].as_list().unwrap_or_default();
        let index = args[2].as_int().unwrap_or(0) as usize;
        let index = index.min(list.len());

        let elements: Vec<Value> = args[3..].to_vec();
        let mut result = Vec::with_capacity(list.len() + elements.len());
        result.extend(list[..index].iter().cloned());
        result.extend(elements);
        result.extend(list[index..].iter().cloned());

        Ok(Value::from_list(&result))
    }

    /// lreplace command - replace elements in list
    fn cmd_lreplace(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 4 {
            return Err(Error::wrong_args_with_usage("lreplace", 4, args.len(), "list first last ?element ...?"));
        }

        let list = args[1].as_list().unwrap_or_default();
        let first = args[2].as_int().unwrap_or(0) as usize;
        let last_str = args[3].as_str();

        let last = if last_str == "end" {
            list.len().saturating_sub(1)
        } else if last_str.starts_with("end-") {
            let offset: usize = last_str[4..].parse().unwrap_or(0);
            list.len().saturating_sub(1 + offset)
        } else {
            last_str.parse::<usize>().unwrap_or(0)
        };

        let first = first.min(list.len());
        let last = last.min(list.len().saturating_sub(1));

        let mut result = Vec::with_capacity(list.len());
        result.extend(list[..first].iter().cloned());
        result.extend(args[4..].iter().cloned());
        if last + 1 < list.len() {
            result.extend(list[last + 1..].iter().cloned());
        }

        Ok(Value::from_list(&result))
    }

    /// lassign command - assign list values to variables
    fn cmd_lassign(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args_with_usage("lassign", 3, args.len(), "list varname ?varname ...?"));
        }

        let list = args[1].as_list().unwrap_or_default();
        let vars: Vec<&str> = args[2..].iter().map(|v| v.as_str()).collect();

        for (i, var) in vars.iter().enumerate() {
            let value = list.get(i).cloned().unwrap_or_else(Value::empty);
            interp.set_var(var, value)?;
        }

        // Return remaining elements
        if list.len() > vars.len() {
            Ok(Value::from_list(&list[vars.len()..]))
        } else {
            Ok(Value::empty())
        }
    }

    /// lrepeat command - create list by repeating elements
    fn cmd_lrepeat(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args_with_usage("lrepeat", 2, args.len(), "count ?element ...?"));
        }

        let count = args[1].as_int().unwrap_or(0) as usize;
        let elements: Vec<Value> = args[2..].to_vec();

        if elements.is_empty() {
            return Ok(Value::empty());
        }

        let mut result = Vec::with_capacity(count * elements.len());
        for _ in 0..count {
            result.extend(elements.iter().cloned());
        }

        Ok(Value::from_list(&result))
    }

    /// lreverse command - reverse a list
    fn cmd_lreverse(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() != 2 {
            return Err(Error::wrong_args("lreverse", 2, args.len()));
        }

        let mut list = args[1].as_list().unwrap_or_default();
        list.reverse();
        Ok(Value::from_list(&list))
    }

    /// split command - split string into list
    fn cmd_split(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(Error::wrong_args_with_usage("split", 2, args.len(), "string ?splitChars?"));
        }

        let string = args[1].as_str();

        if args.len() == 2 {
            // Default: split on whitespace
            let result: Vec<Value> = string.split_whitespace()
                .map(|s| Value::from_str(s))
                .collect();
            Ok(Value::from_list(&result))
        } else {
            let split_chars = args[2].as_str();
            if split_chars.is_empty() {
                // Split into individual characters
                let result: Vec<Value> = string.chars()
                    .map(|c| Value::from_str(&c.to_string()))
                    .collect();
                Ok(Value::from_list(&result))
            } else {
                // Split on any of the characters
                let result: Vec<Value> = string.split(|c| split_chars.contains(c))
                    .map(|s| Value::from_str(s))
                    .collect();
                Ok(Value::from_list(&result))
            }
        }
    }

    /// join command - join list into string
    fn cmd_join(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(Error::wrong_args_with_usage("join", 2, args.len(), "list ?joinString?"));
        }

        let list = args[1].as_list().unwrap_or_default();
        let sep = if args.len() == 3 { args[2].as_str() } else { " " };

        let result: String = list.iter()
            .map(|v| v.as_str())
            .collect::<Vec<&str>>()
            .join(sep);

        Ok(Value::from_str(&result))
    }

    /// subst command - perform substitutions
    fn cmd_subst(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args_with_usage("subst", 2, args.len(), "?-no?-?backs? string"));
        }

        let mut i = 1;
        let mut no_backslashes = false;
        let mut no_commands = false;
        let mut no_variables = false;

        // Parse options
        while i < args.len() && args[i].as_str().starts_with('-') {
            match args[i].as_str() {
                "-nobackslashes" => { no_backslashes = true; i += 1; }
                "-nocommands" => { no_commands = true; i += 1; }
                "-novariables" => { no_variables = true; i += 1; }
                "--" => { i += 1; break; }
                _ => break,
            }
        }

        if i >= args.len() {
            return Err(Error::wrong_args("subst", 2, args.len()));
        }

        let string = args[i].as_str();
        let mut result = String::new();
        let mut chars = string.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '\\' if !no_backslashes => {
                    if let Some(&next) = chars.peek() {
                        match next {
                            'n' => { result.push('\n'); chars.next(); }
                            't' => { result.push('\t'); chars.next(); }
                            'r' => { result.push('\r'); chars.next(); }
                            '\\' => { result.push('\\'); chars.next(); }
                            '$' => { result.push('$'); chars.next(); }
                            '[' => { result.push('['); chars.next(); }
                            _ => { result.push(c); }
                        }
                    } else {
                        result.push(c);
                    }
                }
                '$' if !no_variables => {
                    // Parse variable name
                    let mut var_name = String::new();
                    if let Some(&'{') = chars.peek() {
                        chars.next(); // consume {
                        while let Some(&ch) = chars.peek() {
                            if ch == '}' {
                                chars.next();
                                break;
                            }
                            var_name.push(ch);
                            chars.next();
                        }
                    } else {
                        while let Some(&ch) = chars.peek() {
                            if ch.is_alphanumeric() || ch == '_' {
                                var_name.push(ch);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                    if let Ok(value) = interp.get_var(&var_name) {
                        result.push_str(value.as_str());
                    }
                }
                '[' if !no_commands => {
                    // Find matching ]
                    let mut depth = 1;
                    let mut cmd = String::new();
                    while let Some(ch) = chars.next() {
                        if ch == '[' { depth += 1; }
                        else if ch == ']' { depth -= 1; if depth == 0 { break; } }
                        cmd.push(ch);
                    }
                    let value = interp.eval(&cmd)?;
                    result.push_str(value.as_str());
                }
                _ => { result.push(c); }
            }
        }

        Ok(Value::from_str(&result))
    }

    /// dict command - dictionary operations
    fn cmd_dict(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("dict", 2, args.len()));
        }

        let subcmd = args[1].as_str();

        match subcmd {
            "create" => {
                // Create a dict from key-value pairs
                if (args.len() - 2) % 2 != 0 {
                    return Err(Error::runtime("dict create requires even number of arguments", crate::error::ErrorCode::InvalidOp));
                }
                let mut dict = Vec::new();
                let mut i = 2;
                while i + 1 < args.len() {
                    dict.push(args[i].clone());
                    dict.push(args[i + 1].clone());
                    i += 2;
                }
                Ok(Value::from_list(&dict))
            }
            "get" => {
                if args.len() < 3 {
                    return Err(Error::wrong_args_with_usage("dict get", 3, args.len(), "dictVar ?key ...?"));
                }
                let dict = interp.get_var(args[2].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();
                let keys: Vec<&str> = args[3..].iter().map(|v| v.as_str()).collect();

                let mut current = dict;
                for key in &keys {
                    let mut found = false;
                    let mut i = 0;
                    while i + 1 < current.len() {
                        if current[i].as_str() == *key {
                            if keys.last() == Some(key) {
                                return Ok(current[i + 1].clone());
                            } else {
                                current = current[i + 1].as_list().unwrap_or_default();
                                found = true;
                                break;
                            }
                        }
                        i += 2;
                    }
                    if !found {
                        return Err(Error::runtime(
                            format!("key \"{}\" not known in dictionary", key),
                            crate::error::ErrorCode::InvalidOp
                        ));
                    }
                }
                Ok(Value::from_list(&current))
            }
            "set" => {
                if args.len() < 5 {
                    return Err(Error::wrong_args_with_usage("dict set", 5, args.len(), "dictVar key ?key ...? value"));
                }
                let var_name = args[2].as_str();
                let value = args.last().cloned().unwrap_or_else(Value::empty);
                let keys: Vec<&str> = args[3..args.len()-1].iter().map(|v| v.as_str()).collect();

                let dict = interp.get_var(var_name)
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();

                let result = dict_set(dict, &keys, value);
                interp.set_var(var_name, result)
            }
            "unset" => {
                if args.len() < 4 {
                    return Err(Error::wrong_args_with_usage("dict unset", 4, args.len(), "dictVar key ?key ...?"));
                }
                let var_name = args[2].as_str();
                let keys: Vec<&str> = args[3..].iter().map(|v| v.as_str()).collect();

                let dict = interp.get_var(var_name)
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();

                let result = dict_unset(dict, &keys);
                interp.set_var(var_name, result)
            }
            "keys" => {
                if args.len() < 3 {
                    return Err(Error::wrong_args_with_usage("dict keys", 3, args.len(), "dictVar ?pattern?"));
                }
                let dict = interp.get_var(args[2].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();

                let pattern = if args.len() > 3 { args[3].as_str() } else { "*" };

                let keys: Vec<Value> = dict.chunks(2)
                    .filter_map(|chunk| chunk.first())
                    .filter(|k| glob_match(pattern, k.as_str()))
                    .cloned()
                    .collect();

                Ok(Value::from_list(&keys))
            }
            "values" => {
                if args.len() < 3 {
                    return Err(Error::wrong_args_with_usage("dict values", 3, args.len(), "dictVar"));
                }
                let dict = interp.get_var(args[2].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();

                let values: Vec<Value> = dict.chunks(2)
                    .filter_map(|chunk| chunk.get(1))
                    .cloned()
                    .collect();

                Ok(Value::from_list(&values))
            }
            "haskey" => {
                if args.len() < 4 {
                    return Err(Error::wrong_args_with_usage("dict haskey", 4, args.len(), "dictVar key"));
                }
                let dict = interp.get_var(args[2].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();
                let key = args[3].as_str();

                let has_key = dict.chunks(2)
                    .any(|chunk| chunk.first().map(|k| k.as_str() == key).unwrap_or(false));

                Ok(Value::from_bool(has_key))
            }
            "size" => {
                if args.len() < 3 {
                    return Err(Error::wrong_args_with_usage("dict size", 3, args.len(), "dictVar"));
                }
                let dict = interp.get_var(args[2].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();

                Ok(Value::from_int((dict.len() / 2) as i64))
            }
            "exists" => {
                if args.len() < 4 {
                    return Err(Error::wrong_args_with_usage("dict exists", 4, args.len(), "dictVar key ?key ...?"));
                }
                let dict = interp.get_var(args[2].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();
                let keys: Vec<&str> = args[3..].iter().map(|v| v.as_str()).collect();

                let result = dict_exists(&dict, &keys);
                Ok(Value::from_bool(result))
            }
            "append" => {
                if args.len() < 5 {
                    return Err(Error::wrong_args_with_usage("dict append", 5, args.len(), "dictVar key ?key ...? value"));
                }
                let var_name = args[2].as_str();
                let value = args.last().cloned().unwrap_or_else(Value::empty);
                let keys: Vec<&str> = args[3..args.len()-1].iter().map(|v| v.as_str()).collect();

                let dict = interp.get_var(var_name)
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();

                // Get current value and append
                let current = dict_get_value(&dict, &keys).unwrap_or_default();
                let mut new_val = current.as_str().to_string();
                new_val.push_str(value.as_str());

                let result = dict_set(dict, &keys, Value::from_str(&new_val));
                interp.set_var(var_name, result)
            }
            "for" => {
                if args.len() < 5 {
                    return Err(Error::wrong_args_with_usage("dict for", 5, args.len(), "{keyVar valueVar} dictVar body"));
                }
                let vars = args[2].as_list().unwrap_or_default();
                let key_var = vars.get(0).map(|v| v.as_str()).unwrap_or("key");
                let val_var = vars.get(1).map(|v| v.as_str()).unwrap_or("value");
                let dict = interp.get_var(args[3].as_str())
                    .ok()
                    .and_then(|v| v.as_list())
                    .unwrap_or_default();
                let body = args[4].as_str();

                let mut result = Value::empty();
                let mut i = 0;
                while i + 1 < dict.len() {
                    interp.set_var(key_var, dict[i].clone())?;
                    interp.set_var(val_var, dict[i + 1].clone())?;

                    match interp.eval(body) {
                        Ok(v) => result = v,
                        Err(e) => {
                            if e.is_break() { break; }
                            if e.is_continue() { i += 2; continue; }
                            return Err(e);
                        }
                    }
                    i += 2;
                }
                Ok(result)
            }
            _ => Err(Error::runtime(
                format!("unknown dict subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    }

    /// array command - array operations
    fn cmd_array(interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args("array", 3, args.len()));
        }

        let subcmd = args[1].as_str();
        let array_name = args[2].as_str();

        match subcmd {
            "exists" => {
                // Check if array exists (any element with this name)
                let prefix = format!("{}(", array_name);
                let exists = interp.vars.keys().any(|k| k.starts_with(&prefix));
                Ok(Value::from_bool(exists))
            }
            "get" => {
                let pattern = if args.len() > 3 { args[3].as_str() } else { "*" };
                let prefix = format!("{}(", array_name);

                let result: Vec<Value> = interp.vars.iter()
                    .filter(|(k, _)| k.starts_with(&prefix))
                    .filter(|(k, _)| {
                        // Extract index and match pattern
                        if let Some(end) = k.rfind(')') {
                            let idx = &k[prefix.len()..end];
                            glob_match(pattern, idx)
                        } else {
                            false
                        }
                    })
                    .flat_map(|(k, v)| {
                        // Return index and value pairs
                        if let Some(end) = k.rfind(')') {
                            let idx = &k[prefix.len()..end];
                            vec![Value::from_str(idx), v.clone()]
                        } else {
                            vec![]
                        }
                    })
                    .collect();

                Ok(Value::from_list(&result))
            }
            "set" => {
                if args.len() < 4 {
                    return Err(Error::wrong_args_with_usage("array set", 4, args.len(), "arrayName list"));
                }
                let list = args[3].as_list().unwrap_or_default();

                if list.len() % 2 != 0 {
                    return Err(Error::runtime("list must have even number of elements", crate::error::ErrorCode::InvalidOp));
                }

                let mut i = 0;
                while i + 1 < list.len() {
                    let key = list[i].as_str();
                    let value = &list[i + 1];
                    let full_key = format!("{}({})", array_name, key);
                    interp.vars.insert(full_key, value.clone());
                    i += 2;
                }

                Ok(Value::empty())
            }
            "names" => {
                let pattern = if args.len() > 3 { args[3].as_str() } else { "*" };
                let prefix = format!("{}(", array_name);

                let names: Vec<Value> = interp.vars.keys()
                    .filter(|k| k.starts_with(&prefix))
                    .filter_map(|k| {
                        if let Some(end) = k.rfind(')') {
                            let idx = &k[prefix.len()..end];
                            if glob_match(pattern, idx) {
                                Some(Value::from_str(idx))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();

                Ok(Value::from_list(&names))
            }
            "size" => {
                let prefix = format!("{}(", array_name);
                let count = interp.vars.keys().filter(|k| k.starts_with(&prefix)).count();
                Ok(Value::from_int(count as i64))
            }
            "unset" => {
                let pattern = if args.len() > 3 { args[3].as_str() } else { "*" };
                let prefix = format!("{}(", array_name);

                let keys_to_remove: Vec<String> = interp.vars.keys()
                    .filter(|k| k.starts_with(&prefix))
                    .filter(|k| {
                        if let Some(end) = k.rfind(')') {
                            let idx = &k[prefix.len()..end];
                            glob_match(pattern, idx)
                        } else {
                            false
                        }
                    })
                    .cloned()
                    .collect();

                for key in keys_to_remove {
                    interp.vars.remove(&key);
                }

                Ok(Value::empty())
            }
            _ => Err(Error::runtime(
                format!("unknown array subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    }

    /// file command - file operations
    #[cfg(feature = "std")]
    fn cmd_file(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 3 {
            return Err(Error::wrong_args("file", 3, args.len()));
        }

        let subcmd = args[1].as_str();
        let path = args[2].as_str();

        match subcmd {
            "dirname" => {
                let path = std::path::Path::new(path);
                let parent = path.parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                Ok(Value::from_str(&parent))
            }
            "tail" => {
                let name = std::path::Path::new(path).file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(Value::from_str(&name))
            }
            "join" => {
                let mut result = std::path::PathBuf::new();
                for arg in &args[2..] {
                    result.push(arg.as_str());
                }
                Ok(Value::from_str(&result.to_string_lossy()))
            }
            "exists" => {
                Ok(Value::from_bool(std::path::Path::new(path).exists()))
            }
            "readable" => {
                Ok(Value::from_bool(std::path::Path::new(path).exists())) // Simplified
            }
            "writable" => {
                Ok(Value::from_bool(true)) // Simplified
            }
            "isfile" => {
                Ok(Value::from_bool(std::path::Path::new(path).is_file()))
            }
            "isdirectory" => {
                Ok(Value::from_bool(std::path::Path::new(path).is_dir()))
            }
            "extension" => {
                let ext = std::path::Path::new(path).extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                Ok(Value::from_str(&ext))
            }
            "rootname" => {
                let path = std::path::Path::new(path);
                let stem = path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let parent = path.parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if parent.is_empty() {
                    Ok(Value::from_str(&stem))
                } else {
                    Ok(Value::from_str(&format!("{}/{}", parent, stem)))
                }
            }
            _ => Err(Error::runtime(
                format!("unknown file subcommand: {}", subcmd),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    }

    /// format command - string formatting
    #[cfg(feature = "std")]
    fn cmd_format(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("format", 2, args.len()));
        }

        let format_str = args[1].as_str();
        let format_args: Vec<&str> = args[2..].iter().map(|v| v.as_str()).collect();

        // Simple format implementation
        let mut result = String::new();
        let mut chars = format_str.chars().peekable();
        let mut arg_idx = 0;

        while let Some(c) = chars.next() {
            if c == '%' {
                let next = chars.peek();
                match next {
                    Some('s') => {
                        chars.next();
                        if arg_idx < format_args.len() {
                            result.push_str(format_args[arg_idx]);
                            arg_idx += 1;
                        }
                    }
                    Some('d') | Some('i') => {
                        chars.next();
                        if arg_idx < format_args.len() {
                            let val: i64 = format_args[arg_idx].parse().unwrap_or(0);
                            result.push_str(&val.to_string());
                            arg_idx += 1;
                        }
                    }
                    Some('f') => {
                        chars.next();
                        if arg_idx < format_args.len() {
                            let val: f64 = format_args[arg_idx].parse().unwrap_or(0.0);
                            result.push_str(&val.to_string());
                            arg_idx += 1;
                        }
                    }
                    Some('x') => {
                        chars.next();
                        if arg_idx < format_args.len() {
                            let val: i64 = format_args[arg_idx].parse().unwrap_or(0);
                            result.push_str(&format!("{:x}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('X') => {
                        chars.next();
                        if arg_idx < format_args.len() {
                            let val: i64 = format_args[arg_idx].parse().unwrap_or(0);
                            result.push_str(&format!("{:X}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('c') => {
                        chars.next();
                        if arg_idx < format_args.len() {
                            let val: u8 = format_args[arg_idx].parse().unwrap_or(0);
                            if val > 0 {
                                result.push(val as char);
                            }
                            arg_idx += 1;
                        }
                    }
                    Some('%') => {
                        chars.next();
                        result.push('%');
                    }
                    _ => {
                        result.push(c);
                    }
                }
            } else {
                result.push(c);
            }
        }

        Ok(Value::from_str(&result))
    }

    /// glob command - file pattern matching
    #[cfg(feature = "std")]
    fn cmd_glob(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::wrong_args("glob", 2, args.len()));
        }

        let mut i = 1;
        let mut directory = ".";

        // Parse options
        while i < args.len() && args[i].as_str().starts_with('-') {
            match args[i].as_str() {
                "-directory" => {
                    i += 1;
                    if i < args.len() {
                        directory = args[i].as_str();
                    }
                    i += 1;
                }
                "-nocomplain" => {
                    i += 1;
                }
                "--" => {
                    i += 1;
                    break;
                }
                _ => {
                    i += 1;
                }
            }
        }

        // Collect patterns
        let patterns: Vec<&str> = args[i..].iter().map(|v| v.as_str()).collect();

        let mut result = Vec::new();
        let dir = std::path::Path::new(directory);

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                for pattern in &patterns {
                    if glob_match(pattern, &name) {
                        let full_path = if directory == "." {
                            name.clone()
                        } else {
                            format!("{}/{}", directory, name)
                        };
                        result.push(Value::from_str(&full_path));
                    }
                }
            }
        }

        Ok(Value::from_list(&result))
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

// Dict helper functions
fn dict_set(mut dict: Vec<Value>, keys: &[&str], value: Value) -> Value {
    if keys.is_empty() {
        return Value::from_list(&dict);
    }

    let key = keys[0];
    let mut found = false;

    let mut i = 0;
    while i + 1 < dict.len() {
        if dict[i].as_str() == key {
            if keys.len() == 1 {
                dict[i + 1] = value.clone();
            } else {
                let sub_dict = dict[i + 1].as_list().unwrap_or_default();
                dict[i + 1] = dict_set(sub_dict, &keys[1..], value.clone());
            }
            found = true;
            break;
        }
        i += 2;
    }

    if !found {
        if keys.len() == 1 {
            dict.push(Value::from_str(key));
            dict.push(value);
        } else {
            let sub_dict = dict_set(vec![], &keys[1..], value);
            dict.push(Value::from_str(key));
            dict.push(sub_dict);
        }
    }

    Value::from_list(&dict)
}

fn dict_unset(mut dict: Vec<Value>, keys: &[&str]) -> Value {
    if keys.is_empty() {
        return Value::from_list(&dict);
    }

    let key = keys[0];
    let mut i = 0;
    while i + 1 < dict.len() {
        if dict[i].as_str() == key {
            if keys.len() == 1 {
                dict.remove(i + 1);
                dict.remove(i);
            } else {
                let sub_dict = dict[i + 1].as_list().unwrap_or_default();
                dict[i + 1] = dict_unset(sub_dict, &keys[1..]);
            }
            break;
        }
        i += 2;
    }

    Value::from_list(&dict)
}

fn dict_get_value(dict: &[Value], keys: &[&str]) -> Option<Value> {
    if keys.is_empty() {
        return None;
    }

    let key = keys[0];
    let mut i = 0;
    while i + 1 < dict.len() {
        if dict[i].as_str() == key {
            if keys.len() == 1 {
                return Some(dict[i + 1].clone());
            } else {
                let sub_dict = dict[i + 1].as_list().unwrap_or_default();
                return dict_get_value(&sub_dict, &keys[1..]);
            }
        }
        i += 2;
    }
    None
}

fn dict_exists(dict: &[Value], keys: &[&str]) -> bool {
    if keys.is_empty() {
        return false;
    }

    let key = keys[0];
    let mut i = 0;
    while i + 1 < dict.len() {
        if dict[i].as_str() == key {
            if keys.len() == 1 {
                return true;
            } else {
                let sub_dict = dict[i + 1].as_list().unwrap_or_default();
                return dict_exists(&sub_dict, &keys[1..]);
            }
        }
        i += 2;
    }
    false
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
