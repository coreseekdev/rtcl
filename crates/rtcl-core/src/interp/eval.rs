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

    /// Run `script` with errorInfo accumulation isolated: internal
    /// side-evals (deferred scripts, trace callbacks, package indexes)
    /// must not append frames to an error propagating through the caller.
    pub(crate) fn eval_isolated(&mut self, script: &str) -> Result<Value> {
        let saved_info = self.err_info.take();
        let saved_fresh = std::mem::take(&mut self.err_fresh);
        let saved_subst = std::mem::take(&mut self.err_from_subst);
        let saved_pending = self.err_pending_top.take();
        let r = self.eval(script);
        self.err_info = saved_info;
        self.err_fresh = saved_fresh;
        self.err_from_subst = saved_subst;
        self.err_pending_top = saved_pending;
        r
    }

    /// Script-harness frame append after a dispatched command returned an
    /// error (tclsh's TclEvalEx logging): the first append starts the
    /// accumulated info with the message and `while executing`; later
    /// appends add `invoked from within`.  `fresh` suppresses exactly one
    /// append.  Non-error completions (return/break/continue/exit) never
    /// reach this.
    pub(crate) fn err_harness_frame(&mut self, msg: &str, text: &str, line: usize) {
        match (&mut self.err_info, self.err_fresh) {
            (Some(_), true) => {
                self.err_fresh = false;
            }
            (Some(info), false) => {
                info.push_str(&format!("\n    invoked from within\n\"{}\"", text));
            }
            (None, _) => {
                self.err_info = Some(format!(
                    "{}\n    while executing\n\"{}\"",
                    msg, text
                ));
            }
        }
        self.err_line = line;
        // A normal harness log means the substitution boundary's
        // deferred enclosing-command frame was superseded (tclsh's
        // ERR_ALREADY_LOGGED is consumed by one level).
        self.err_pending_top = None;
    }

    /// Construct-exit frame append — `(procedure "x" line N)`,
    /// `(in namespace eval "::n" script line N)`, `("uplevel" body line
    /// N)`, `(file "p" line N)`: pass the tag without the `line N` tail.
    /// No-op when no error info is accumulating.
    pub(crate) fn err_exit_frame(&mut self, tag: &str) {
        if let Some(info) = &mut self.err_info {
            info.push_str(&format!("\n    ({} line {})", tag, self.err_line));
        }
    }

    /// Does this error participate in errorInfo frame accumulation?
    /// Control-flow completions (return/break/continue/exit, tail-call
    /// requests) propagate framelessly.
    pub(crate) fn err_is_error(&self, e: &Error) -> bool {
        !(e.is_return() || e.is_break() || e.is_continue() || e.is_exit() || e.is_tail_call())
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

        // Evaluate all words, handling {*} expand.  A failure here
        // propagates frameless when it came from a command substitution
        // (the nested eval already logged; the enclosing command's frame
        // is deferred to the top-level report), but a plain variable-read
        // or word-parse failure logs this command's frame (tclsh:
        // `lindex $q [boomq]` with $q unset shows "lindex $q [boomq]").
        let mut args = Vec::with_capacity(cmd.words.len());
        for word in &cmd.words {
            let value = match self.eval_word(word) {
                Ok(v) => v,
                Err(e) => {
                    if self.err_is_error(&e) {
                        if std::mem::take(&mut self.err_from_subst) {
                            self.err_pending_top = Some(cmd.text.clone());
                            // The construct-exit tags report the
                            // enclosing command's line, not the
                            // substitution-internal line.
                            self.err_line = cmd.line;
                        } else {
                            let msg = e.message_text();
                            self.err_harness_frame(&msg, &cmd.text, cmd.line);
                        }
                    }
                    return Err(e);
                }
            };
            if let Word::Expand(_) = word {
                if let Some(items) = value.as_list() {
                    for item in items {
                        args.push(item);
                    }
                } else {
                    args.push(value);
                }
            } else {
                args.push(value);
            }
        }

        // Expose the invocation for constructs that need the raw source
        // (`info level 0` inside `namespace eval`).
        self.cur_cmd_text = cmd.text.clone();
        let r = self.dispatch_values(&args);
        if let Err(e) = &r {
            if self.err_is_error(e) {
                let msg = e.message_text();
                self.err_harness_frame(&msg, &cmd.text, cmd.line);
            }
        }
        r
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
            })
            .or_else(|| {
                // Global fallback: after the current namespace misses, an
                // unqualified name reaches the global namespace's `::name`
                // proc (tclsh 52.2: `foo` inside ::bar::jim finds ::foo).
                if !cmd_name.starts_with("::") {
                    let qualified = format!("::{}", cmd_name);
                    self.procs.get(&qualified).cloned().map(|p| (p, qualified))
                } else {
                    None
                }
            })
            .or_else(|| {
                // `::pp` — an absolute simple name reaches the global
                // namespace's bare-keyed proc (tclsh: `::pp q r` calls pp).
                // The frame name is as invoked (tclsh `(procedure "::pp")`).
                if cmd_name.starts_with("::") && !cmd_name[2..].contains("::") {
                    let bare = &cmd_name[2..];
                    return self.procs.get(bare).cloned().map(|p| (p, cmd_name.to_string()));
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
                // ::tcl::unsupported::representation — internal-rep debug
                // view (expr-52.1 string-matches "*no string
                // representation*" for values whose string form is not
                // yet generated, e.g. lists).
                if cmd_name
                    .strip_prefix("::tcl::unsupported::")
                    .or_else(|| cmd_name.strip_prefix("tcl::unsupported::"))
                    == Some("representation")
                {
                    if args.len() != 2 {
                        return Err(Error::wrong_args(
                            "::tcl::unsupported::representation value",
                            1,
                            args.len().saturating_sub(1),
                        ));
                    }
                    let v = &args[1];
                    let ptr = &*v as *const _ as usize;
                    let tn = v.type_name();
                    let text = if tn == "list" || tn == "dict" {
                        format!(
                            "value is a {tn} with a refcount of 2, object pointer at 0x{ptr:x}, internal representation: {tn}, no string representation"
                        )
                    } else {
                        let sv = v.as_str();
                        let head: String = sv.chars().take(40).collect();
                        let dots = if sv.chars().count() > 40 { "..." } else { "" };
                        format!(
                            "value is a pure string with a refcount of 2, object pointer at 0x{ptr:x}, string representation \"{head}{dots}\""
                        )
                    };
                    return Ok(Value::from_str(&text));
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
                // Try "unknown" handler (if defined as a proc or ensemble);
                // the rename dance of namespace-52.6 keeps it under either
                // the bare or the `::`-qualified key.  tclsh dispatches the
                // handler INLINE (no harness frame naming "unknown ...") and
                // with the qualified word `::unknown` — the original
                // command's frame comes from the outer harness.
                if cmd_name != "unknown" {
                    let has_unknown = self.procs.contains_key("unknown")
                        || self.procs.contains_key("::unknown")
                        || self.ensembles.contains_key("unknown")
                        || self.ensembles.contains_key("::unknown");
                    if has_unknown {
                        let mut unknown_args = vec![Value::from_str("::unknown")];
                        unknown_args.extend(args.iter().cloned());
                        return self.dispatch_values(&unknown_args);
                    }
                }
                Err(Error::invalid_command(cmd_name))
            }
        }
    }

    /// Full uncaught-error report for the `-f`/`-c` top level: the
    /// accumulated errorInfo plus the `(file "..." line N)` frame tclsh
    /// appends as the script unwinds, or the bare message when no frames
    /// accumulated.  Prints verbatim on stderr (tclsh-compatible).
    pub fn error_report(&mut self, e: &Error, file: &str) -> String {
        if self.err_is_error(e) {
            let tag = format!("file \"{}\"", file);
            // A word-substitution error that escaped every enclosing
            // construct frameless gets its enclosing command's frame
            // here (tclsh logs it at the top level, after the skip).
            if let Some(text) = self.err_pending_top.take() {
                if let Some(info) = &mut self.err_info {
                    info.push_str(&format!("\n    invoked from within\n\"{}\"", text));
                }
            }
            self.err_exit_frame(&tag);
            if let Some(info) = self.err_info.take() {
                return info;
            }
        }
        e.to_string()
    }

    /// Evaluate a word to get its value.
    pub(crate) fn eval_word(&mut self, word: &Word) -> Result<Value> {
        match word {
            Word::Literal(s) => Ok(Value::from_str(s)),
            Word::VarRef(name) => self.read_var(name),
            Word::CommandSub(cmd) => match self.eval(cmd) {
                Ok(v) => Ok(v),
                Err(e) => {
                    // Mark the boundary: the enclosing command must not
                    // log its own frame (the nested eval just did).
                    self.err_from_subst = true;
                    Err(e)
                }
            },
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
