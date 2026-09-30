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

        self.dispatch_values(&args)
    }

    /// Dispatch an already-evaluated argument vector: procs, ensembles,
    /// builtins, expr functions, then unknown handlers.  This is the tail
    /// of [`eval_command`] shared with ensemble/unknown re-dispatch.
    pub(crate) fn dispatch_values(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Ok(Value::empty());
        }
        let cmd_name = args[0].as_str();

        // Namespace-aware command lookup:
        // 1. Try the name as-is (handles fully-qualified "::ns::cmd" and global commands)
        // 2. If in a non-global namespace, try qualifying the name in the current namespace
        // 3. Fall back to global unqualified name

        // User-defined procs (including `namespace import` aliases, which
        // dispatch to the origin's *current* body and namespace)
        let proc_lookup = self.procs.get(cmd_name).cloned().map(|p| (p, cmd_name.to_string()))
            .or_else(|| {
                if self.current_namespace != "::" && !cmd_name.starts_with("::") {
                    let qualified = crate::interp::commands::namespace::qualify(
                        &self.current_namespace, cmd_name,
                    );
                    self.procs.get(&qualified).cloned().map(|p| (p, qualified))
                } else {
                    None
                }
            })
            .or_else(|| {
                // `foo::p` at global scope is the fully-qualified `::foo::p`
                if !cmd_name.starts_with("::") && cmd_name.contains("::") {
                    let qualified = format!("::{}", cmd_name);
                    self.procs.get(&qualified).cloned().map(|p| (p, qualified))
                } else {
                    None
                }
            })
            .or_else(|| {
                // Ancestor namespaces: a command inherited from an
                // enclosing namespace resolves unqualified (tclsh 52.2:
                // ::bar::jim::test sees ::bar's `foo`).
                let mut anc = self.current_namespace.clone();
                loop {
                    if anc == "::" {
                        return None;
                    }
                    anc = crate::interp::commands::namespace::parent_of(&anc);
                    let q = crate::interp::commands::namespace::qualify(
                        &anc, cmd_name,
                    );
                    if let Some(p) = self.procs.get(&q).cloned() {
                        return Some((p, q));
                    }
                }
            })
            .or_else(|| {
                // Import aliases: follow the chain to the origin so the
                // proc runs in its definition namespace with its current
                // body (tclsh: redefining the source is visible).
                let found = crate::interp::commands::namespace::lookup_command_key(
                    self, cmd_name,
                )?;
                let origin = crate::interp::commands::namespace::origin_of(self, &found)?;
                self.procs.get(&origin).cloned().map(|p| (p, origin))
            })
            .or_else(|| {
                // Colon runs collapse in command names too: `p1:::g`
                // dispatches to the proc registered as `::p1::g`.
                if cmd_name.contains("::") {
                    let norm = crate::interp::commands::namespace::normalise(cmd_name);
                    if norm != cmd_name {
                        return self.procs.get(&norm).cloned().map(|p| (p, norm));
                    }
                }
                None
            });
        if let Some((proc_def, resolved_name)) = proc_lookup {
            return self.call_proc(&proc_def, &args, &resolved_name, None);
        }

        // Namespace ensembles: the command itself, or an import alias
        // whose origin chain lands on one.
        if let Some((ens_key, ens_def)) =
            crate::interp::commands::namespace::find_ensemble(self, cmd_name)
        {
            return crate::interp::commands::namespace::dispatch_ensemble(
                self, &ens_key, ens_def, &args,
            );
        }

        // Built-in commands
        let func = self.commands.get(cmd_name).cloned().or_else(|| {
            if self.current_namespace != "::" && !cmd_name.starts_with("::") {
                let qualified = crate::interp::commands::namespace::qualify(
                    &self.current_namespace, cmd_name,
                );
                self.commands.get(&qualified).cloned()
            } else {
                None
            }
        }).or_else(|| {
            if !cmd_name.starts_with("::") && cmd_name.contains("::") {
                self.commands.get(&format!("::{}", cmd_name)).cloned()
            } else {
                None
            }
        }).or_else(|| {
            // `::set` — a `::`-qualified builtin resolves through its
            // unqualified registration key (tclsh 8.6.17)
            if cmd_name.starts_with("::") {
                let stripped = cmd_name.trim_start_matches(':');
                if !stripped.contains("::") {
                    return self.commands.get(stripped).cloned();
                }
            }
            None
        }).or_else(|| {
            // Colon runs collapse in builtin names as well.
            if cmd_name.contains("::") {
                let norm = crate::interp::commands::namespace::normalise(cmd_name);
                if norm != cmd_name {
                    return self.commands.get(&norm).cloned();
                }
            }
            None
        });
        match func {
            Some(f) => {
                self.call_depth += 1;
                let result = f(self, &args);
                self.call_depth -= 1;
                self.fill_wrong_args(cmd_name, result)
            }
            None => {
                // Expr functions double as commands: `::tcl::mathfunc::abs -0`
                // (expr-38.5). The tail after tcl::mathfunc:: is the function.
                let mf = cmd_name
                    .strip_prefix("::tcl::mathfunc::")
                    .or_else(|| cmd_name.strip_prefix("tcl::mathfunc::"));
                if let Some(fname) = mf {
                    let seed = self.call_depth;
                    return crate::types::expr_funcs::call_math_func(
                        fname,
                        args[1..].to_vec(),
                        seed,
                    );
                }
                // `namespace unknown` handler: the current namespace's
                // own handler first, then ancestors' (tclsh 52.7 walks up
                // to ::).  An explicitly-set value stops the walk even
                // when it parses to an empty prefix.
                if cmd_name != "unknown" {
                    let mut anc = self.current_namespace.clone();
                    let handler = loop {
                        if let Some(v) = self.ns_unknown.get(&anc) {
                            let words: Vec<String> =
                                crate::value::Value::from_str(v.as_str())
                                    .as_list()
                                    .map(|l| {
                                        l.iter().map(|w| w.as_str().to_string()).collect()
                                    })
                                    .unwrap_or_default();
                            break if words.is_empty() { None } else { Some(words) };
                        }
                        if anc == "::" {
                            break None;
                        }
                        anc = crate::interp::commands::namespace::parent_of(&anc);
                    };
                    if let Some(words) = handler {
                        let mut call: Vec<Value> = words
                            .iter()
                            .map(|w| Value::from_str(w.as_str()))
                            .collect();
                        call.extend(args.iter().cloned());
                        return self.dispatch_values(&call);
                    }
                }
                // Try "unknown" handler (if defined as a proc or command)
                if cmd_name != "unknown" {
                    let has_unknown = self.procs.contains_key("unknown")
                        || self.commands.contains_key("unknown");
                    if has_unknown {
                        let mut unknown_args = vec![Value::from_str("unknown")];
                        unknown_args.extend(args.iter().cloned());
                        // Recurse through eval_command to dispatch "unknown"
                        let unknown_cmd = crate::parser::Command {
                            words: unknown_args.iter().map(|v| {
                                crate::parser::Word::Literal(v.as_str().to_string())
                            }).collect(),
                            line: 0,
                        };
                        return self.eval_command(&unknown_cmd);
                    }
                }
                Err(Error::invalid_command(cmd_name))
            }
        }
    }

    /// Evaluate a word to get its value.
    pub(crate) fn eval_word(&mut self, word: &Word) -> Result<Value> {
        match word {
            Word::Literal(s) => Ok(Value::from_str(s)),
            Word::VarRef(name) => self.read_var(name),
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
