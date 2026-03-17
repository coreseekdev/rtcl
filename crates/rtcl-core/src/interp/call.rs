//! Procedure call and tail-call optimisation for [`Interp`].

use super::{Interp, ProcDef};
use crate::error::{Error, Result};
use crate::value::Value;

impl Interp {
    /// Call a user-defined procedure.
    pub(crate) fn call_proc(&mut self, proc_def: &ProcDef, args: &[Value]) -> Result<Value> {
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        let mut current_params = proc_def.params.clone();
        let mut current_body = proc_def.body.clone();
        let mut current_args: Vec<Value> = args.to_vec();

        self.call_depth += 1;

        let final_result = loop {
            // ── Bind parameters ────────────────────────────────────
            let mut saved_vars = Vec::new();
            let has_args = current_params.last().map(|(p, _)| p.as_str()) == Some("args");

            let regular_params = if has_args {
                &current_params[..current_params.len() - 1]
            } else {
                &current_params[..]
            };

            for (i, (param, default)) in regular_params.iter().enumerate() {
                let value = if i + 1 < current_args.len() {
                    current_args[i + 1].clone()
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
                if let Some(old) = self.vars.get("args") {
                    saved_vars.push(("args".to_string(), Some(old.clone())));
                } else {
                    saved_vars.push(("args".to_string(), None));
                }
                self.vars.insert("args".to_string(), Value::from_str(&list_str));
            }

            // ── Execute body ───────────────────────────────────────
            let result = self.eval(&current_body);

            // ── Restore variables ──────────────────────────────────
            for (param, old_value) in saved_vars {
                if let Some(v) = old_value {
                    self.vars.insert(param, v);
                } else {
                    self.vars.remove(&param);
                }
            }

            // ── Check for tail-call signal ─────────────────────────
            match result {
                Err(e) if e.is_tail_call() => {
                    let tc_args = e.into_tail_call_args().unwrap();
                    let cmd_name = &tc_args[0];
                    if let Some(new_proc) = self.procs.get(cmd_name).cloned() {
                        // Tail-call to another proc — reuse the frame (no depth increase)
                        current_params = new_proc.params;
                        current_body = new_proc.body;
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

        self.call_depth -= 1;

        match final_result {
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
            result
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
