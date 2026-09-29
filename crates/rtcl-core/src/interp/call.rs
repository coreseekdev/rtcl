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

        self.call_depth += 1;

        // A proc executes in the namespace where it was defined (the
        // qualifiers of its registered name), not the caller's namespace.
        let prev_namespace = self.current_namespace.clone();

        // Push a new call frame
        self.frames.push(CallFrame {
            locals: HashMap::new(),
            array_locals: HashSet::new(),
            upvars: HashMap::new(),
            ns: None,
            local_procs: Vec::new(),
            deferred_scripts: Vec::new(),
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
            let result = self.eval(&current_body);

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
                    let tc_args = e.into_tail_call_args().unwrap();
                    let cmd_name = &tc_args[0];
                    if let Some(new_proc) = self.procs.get(cmd_name).cloned() {
                        // Tail-call to another proc — reuse the frame (no depth increase)
                        current_params = new_proc.params;
                        current_body = new_proc.body;
                        current_statics = new_proc.statics;
                        current_proc_name = cmd_name.clone();
                        current_args = tc_args.into_iter().map(|s| Value::from_str(&s)).collect();
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
                let _ = self.eval(script);
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
