//! Procedure call and tail-call optimisation for [`Interp`].

use super::{CallFrame, Interp, ProcDef};
use crate::error::{Error, Result};
use crate::value::Value;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

#[cfg(not(feature = "embedded"))]
use std::collections::HashSet;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeSet as HashSet;

impl Interp {
    /// Call a user-defined procedure.
    ///
    /// `ns_override` pins the frame's namespace for `apply {p b ::ns}`
    /// (the lambda runs in `::ns` regardless of its synthetic name).
    pub(crate) fn call_proc(&mut self, proc_def: &ProcDef, args: &[Value], proc_name: &str, ns_override: Option<String>) -> Result<Value> {
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        let mut current_params = proc_def.params.clone();
        let mut current_body = proc_def.body.clone();
        let mut current_args: Vec<Value> = args.to_vec();
        let mut current_statics: HashMap<String, Value> = proc_def.statics.clone();
        let mut current_proc_name = proc_name.to_string();
        let mut ns_override = ns_override;

        // Execution traces (tclsh 8.6.17): `enter` fires before the frame
        // exists and its callback's error REPLACES this invocation
        // (trace-40.1); `leave` fires after the frame is gone.  The
        // resolved key doubles as this frame's enterstep context (the
        // body's commands resolve their enterstep traces through it).
        let exec_key = super::commands::proc::resolve_command_key(self, proc_name)
            .unwrap_or_else(|| proc_name.to_string());
        let exec_cmdtext = if self.exec_traces.is_empty() {
            String::new()
        } else {
            Value::from_list(args).as_str().to_string()
        };
        if !self.exec_traces.is_empty() {
            self.exec_fire_enter(&exec_key, &exec_cmdtext)?;
        }
        self.exec_step_stack.push(exec_key.clone());

        self.call_depth += 1;

        // A proc executes in the namespace where it was defined (the
        // qualifiers of its registered name), not the caller's namespace.
        let prev_namespace = self.current_namespace.clone();
        // Apply's one-shot `info level 0` word (consumed even when the
        // arity check fails, so it can't leak into a later call).
        let level0_override = self.frame_level0_args.take();

        // Push a new call frame
        self.frames.push(CallFrame {
            locals: HashMap::new(),
            array_locals: HashSet::new(),
            upvars: HashMap::new(),
            ns: None,
            call_ns: Some(prev_namespace.clone()),
            local_procs: Vec::new(),
            deferred_scripts: Vec::new(),
            tailcall: None,
            level0: String::new(),
            ns_depth: self.ns_level0.len(),
        });

        let final_result = loop {
            // ── Enter the definition namespace (also on tail-call switch) ──
            // The override (apply's ::ns element) applies to the first frame
            // only; tail-call targets recompute from their own names.
            {
                let def_ns = ns_override
                    .take()
                    .unwrap_or_else(|| ns_of_qualified(&current_proc_name));
                self.current_namespace = def_ns.clone();
                let frame = self.frames.last_mut().unwrap();
                frame.ns = Some(def_ns);
                frame.locals.clear();
                frame.array_locals.clear();
                frame.upvars.clear();
                // `info level 0` shows the invocation as dispatched: the
                // as-typed command word plus the evaluated arguments
                // (tclsh 47.1: `ns a b c` → `::ns::a b c`).  `apply`
                // overrides with `apply {<term>} <args...>`.
                frame.level0 = match &level0_override {
                    Some(l0) => Value::from_list(l0).as_str().to_string(),
                    None => {
                        Value::from_list(&current_args.to_vec()).as_str().to_string()
                    }
                };
            }

            let has_args = current_params.last().map(|(p, _)| p.as_str()) == Some("args");

            let regular_params = if has_args {
                &current_params[..current_params.len() - 1]
            } else {
                &current_params[..]
            };

            // ── Arity check (Tcl: wrong # args) ────────────────────
            // Tcl binds positionally: any parameter left without an
            // argument must have a default (or be the trailing `args`).
            {
                let num_args = current_args.len().saturating_sub(1);
                let mut arity_ok = has_args || num_args <= regular_params.len();
                if arity_ok {
                    for (i, (_, d)) in regular_params.iter().enumerate() {
                        if i >= num_args && d.is_none() {
                            arity_ok = false;
                            break;
                        }
                    }
                }
                if !arity_ok {
                    // Tcl list-quotes the command name ("{}", "{a b  c}")
                    // but appends parameter descriptors verbatim ("?arg ...?").
                    let name_word = if current_proc_name == "apply lambdaExpr" {
                        // Synthetic apply frame: Tcl renders this prefix literally.
                        current_proc_name.clone()
                    } else {
                        crate::value::tcl_quote(&current_proc_name)
                    };
                    let mut usage = name_word;
                    for (i, (p, d)) in current_params.iter().enumerate() {
                        usage.push(' ');
                        if has_args && i == current_params.len() - 1 {
                            usage.push_str("?arg ...?");
                        } else if d.is_some() {
                            usage.push('?');
                            usage.push_str(p);
                            usage.push('?');
                        } else {
                            usage.push_str(p);
                        }
                    }
                    break Err(Error::Msg(format!(
                        "wrong # args: should be \"{}\"",
                        usage
                    )));
                }
            }

            for (i, (param, default)) in regular_params.iter().enumerate() {
                let value = if i + 1 < current_args.len() {
                    current_args[i + 1].clone()
                } else if let Some(d) = default {
                    Value::from_str(d)
                } else {
                    Value::empty()
                };
                self.frames.last_mut().unwrap().locals.insert(param.clone(), value);
            }

            if has_args {
                let remaining_start = regular_params.len() + 1;
                let remaining_args: Vec<&Value> = if remaining_start < current_args.len() {
                    current_args[remaining_start..].iter().collect()
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
                self.frames.last_mut().unwrap().locals.insert("args".to_string(), Value::from_str(&list_str));
            }

            // ── Inject static variables into the frame ─────────────
            for (sname, sval) in &current_statics {
                self.frames.last_mut().unwrap().locals.insert(sname.clone(), sval.clone());
            }

            // ── Execute body ───────────────────────────────────────
            let mut result = self.eval(&current_body);

            // A tailcall whose completion an inner catch consumed leaves
            // the frame marker armed: it still fires at body end and
            // replaces the result (tclsh tailcall-12.3a1:
            // `catch [list tailcall foo]; tailcall` → {}).
            if !matches!(&result, Err(e) if e.is_tail_call()) {
                if let Some(frame) = self.frames.last_mut() {
                    if let Some(tc_args) = frame.tailcall.take() {
                        if !tc_args.is_empty() {
                            result = Err(Error::tail_call(tc_args));
                        }
                    }
                }
            }

            // ── Check for tail-call signal ─────────────────────────
            match result {
                Err(e) if e.is_tail_call() => {
                    // Write back statics before switching to tail-call target
                    if !current_statics.is_empty() {
                        if let Some(frame) = self.frames.last() {
                            let snames: Vec<String> = current_statics.keys().cloned().collect();
                            for sname in &snames {
                                if let Some(val) = frame.locals.get(sname) {
                                    current_statics.insert(sname.clone(), val.clone());
                                }
                            }
                        }
                        if let Some(pdef) = self.procs.get_mut(&current_proc_name) {
                            pdef.statics.clone_from(&current_statics);
                        }
                        current_statics = HashMap::new();
                    }
                    let tc_args = match self.frames.last_mut().unwrap().tailcall.take() {
                        Some(a) => a,
                        None => e.into_tail_call_args().unwrap(),
                    };
                    if tc_args.is_empty() {
                        // `tailcall` with no command: the frame yields the
                        // empty completion value.
                        break Ok(Value::empty());
                    }
                    let cmd_name = tc_args[0].clone();
                    if let Some(new_proc) = self.procs.get(&cmd_name).cloned() {
                        // Tail-call to another proc — reuse the frame (no depth increase)
                        current_params = new_proc.params;
                        current_body = new_proc.body;
                        current_statics = new_proc.statics;
                        current_proc_name = cmd_name.clone();
                        current_args = tc_args.into_iter().map(|s| Value::from_str(&s)).collect();
                        // The reused frame's enterstep context now names the
                        // tail-call target (corpus-free: no extra enter/leave
                        // fires for the switched-to command).
                        let new_key = super::commands::proc::resolve_command_key(self, &cmd_name)
                            .unwrap_or_else(|| cmd_name.clone());
                        if let Some(top) = self.exec_step_stack.last_mut() {
                            *top = new_key;
                        }
                        continue;
                    } else {
                        // Target is a built-in — evaluate and return
                        let tc_values: Vec<Value> =
                            tc_args.iter().map(|s| Value::from_str(s)).collect();
                        break self.invoke_builtin_or_eval(&tc_values);
                    }
                }
                other => break other,
            }
        };

        // `(procedure "name" line N)` frame when the body errored — the
        // line is the erroring command's line within the body, recorded by
        // the body's script harness.  Lambda bodies (apply) get the
        // `(lambda term ...)` tag from cmd_apply instead.
        if let Err(e) = &final_result {
            if self.err_is_error(e) && current_proc_name != "apply lambdaExpr" {
                let tag = format!("procedure \"{}\"", current_proc_name);
                self.err_exit_frame(&tag);
            }
        }

        // Write back static variables to the proc definition
        if !current_statics.is_empty() {
            if let Some(frame) = self.frames.last() {
                let mut updated = current_statics;
                for sname in updated.keys().cloned().collect::<Vec<_>>() {
                    if let Some(val) = frame.locals.get(&sname) {
                        updated.insert(sname, val.clone());
                    }
                }
                if let Some(pdef) = self.procs.get_mut(&current_proc_name) {
                    pdef.statics = updated;
                }
            }
        }

        // Execute deferred scripts (from `defer` command) in reverse order
        if let Some(frame) = self.frames.last() {
            let scripts: Vec<String> = frame.deferred_scripts.clone();
            for script in scripts.iter().rev() {
                let _ = self.eval_isolated(script);
            }
        }

        // Clean up local procs (created by `local` command)
        if let Some(frame) = self.frames.last() {
            let procs_to_delete: Vec<String> = frame.local_procs.clone();
            for name in &procs_to_delete {
                self.procs.remove(name);
                self.commands.remove(name);
                self.aliases.remove(name);
            }
        }

        // Pop the frame and leave the definition namespace
        self.frames.pop();
        // Frame-local trace tables keyed F{idx}:... must not leak into the
        // next call that reuses this frame index.
        let fkey = format!("F{}:", self.frames.len());
        self.var_traces.retain(|k, _| !k.starts_with(&fkey));
        self.elem_traces.retain(|k, _| !k.starts_with(&fkey));
        self.trace_phantoms.retain(|k, _| !k.starts_with(&fkey));
        self.current_namespace = prev_namespace;
        self.call_depth -= 1;

        // Execution traces: the frame is gone; `leave` fires with the
        // completion code and result (errors are background errors).
        self.exec_step_stack.pop();
        if !self.exec_traces.is_empty() {
            let pair = match &final_result {
                Ok(v) => Some(("0", v.as_str().to_string())),
                Err(e) if self.err_is_error(e) => {
                    Some(("1", e.message_text().to_string()))
                }
                _ => None,
            };
            if let Some((code, res)) = pair {
                self.exec_fire_leave(&exec_key, &exec_cmdtext, code, &res);
            }
        }

        match final_result {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_return() {
                    match &e {
                        Error::ControlFlow { level, value, error_info, error_code, .. } => {
                            // Propagate -errorinfo / -errorcode to global variables
                            if let Some(info) = error_info {
                                let _ = self.set_var("::errorInfo", Value::from_str(info));
                            }
                            if let Some(code) = error_code {
                                let _ = self.set_var("::errorCode", Value::from_str(code));
                                self.err_code_raised = true;
                            }
                            let val = value.clone().unwrap_or_default();
                            match *level {
                                0 => {
                                    // Plain return (level=0 means just return the value)
                                    Ok(val)
                                }
                                n if n < 0 => {
                                    // Explicit -level N encoded as -(N+1): any of
                                    // these leaves this proc with its value
                                    // (rtcl has no multi-level proc return).
                                    Ok(val)
                                }
                                1 => {
                                    // return -code error "msg" → propagate as error,
                                    // preserving any -errorinfo / -errorcode options.
                                    Err(Error::ControlFlow {
                                        kind: crate::error::ControlFlow::Error,
                                        value: Some(val),
                                        level: 1,
                                        error_info: error_info.clone(),
                                        error_code: error_code.clone(),
                                    })
                                }
                                3 => {
                                    // return -code break
                                    Err(Error::brk())
                                }
                                4 => {
                                    // return -code continue
                                    Err(Error::cont())
                                }
                                _ => Ok(val),
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

    /// Invoke a command from tail-call args. Tries builtins first, falls back to eval.
    fn invoke_builtin_or_eval(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Ok(Value::empty());
        }
        let cmd_name = args[0].as_str();
        if let Some(f) = self.commands.get(cmd_name).cloned() {
            self.call_depth += 1;
            let result = f(self, args);
            self.call_depth -= 1;
            self.fill_wrong_args(cmd_name, result)
        } else {
            // Fallback: build script string and eval
            let script: String = args
                .iter()
                .map(|a| {
                    let s = a.as_str();
                    if s.is_empty() || s.contains(' ') || s.contains('\t') || s.contains('\n') {
                        format!("{{{}}}", s)
                    } else {
                        s.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            self.eval(&script)
        }
    }
}

/// The namespace part of a qualified proc name: `::foo::p` → `::foo`,
/// `::p` → `::`, bare `p` → `::`.
fn ns_of_qualified(name: &str) -> String {
    match name.rfind("::") {
        Some(0) => "::".to_string(),
        Some(pos) => format!("::{}", name[..pos].trim_start_matches("::")),
        None => "::".to_string(),
    }
}
