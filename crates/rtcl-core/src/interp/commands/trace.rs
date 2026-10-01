//! The `trace` command (tclsh traceCmd.c): add / remove / info over
//! variable traces, plus stored-but-not-fired command and execution
//! traces. Legacy `trace variable|vdelete|vinfo` forms use character
//! ops (rwua).

use crate::error::{Error, Result};
use crate::interp::commands::list;
use crate::interp::{ExecStepCtx, ExecTrace, VarTrace};
use crate::interp::Interp;
use crate::value::Value;

const TRACE_SUBCMDS: &[&str] = &["add", "info", "remove", "variable", "vdelete", "vinfo"];
const VAR_OPS: &[&str] = &["array", "read", "unset", "write"];
const CMD_OPS: &[&str] = &["delete", "rename"];
const EXEC_OPS: &[&str] = &["enter", "enterstep", "leave", "leavestep"];

fn bad_option(what: &str, ops: &[&str]) -> Error {
    let joined = match ops.len() {
        1 => ops[0].to_string(),
        2 => format!("{} or {}", ops[0], ops[1]),
        _ => format!("{}, or {}", ops[..ops.len() - 1].join(", "), ops[ops.len() - 1]),
    };
    Error::runtime(
        format!("bad option \"{}\": must be {}", what, joined),
        crate::error::ErrorCode::Generic,
    )
}

/// Resolve a trace subcommand (exact wins, then unique prefix).
fn resolve_subcmd(sub: &str) -> Result<&'static str> {
    for c in TRACE_SUBCMDS {
        if *c == sub {
            return Ok(c);
        }
    }
    let mut matches = TRACE_SUBCMDS.iter().filter(|c| c.starts_with(sub));
    match (matches.next(), matches.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(bad_option(sub, TRACE_SUBCMDS)),
    }
}

/// Resolve a trace type (execution | command | variable).
fn resolve_type(ty: &str) -> Result<&'static str> {
    let types = ["execution", "command", "variable"];
    for t in types {
        if t == ty {
            return Ok(t);
        }
    }
    let mut matches = types.iter().filter(|t| t.starts_with(ty));
    match (matches.next(), matches.next()) {
        (Some(t), None) => Ok(t),
        _ => Err(bad_option(
            ty,
            &["execution", "command", "variable"],
        )),
    }
}

/// Parse an op list (words) against a vocabulary; render canonically.
fn parse_ops(interp: &mut Interp, raw: &Value, vocab: &[&str]) -> Result<Vec<String>> {
    let items = list::strict_list(interp, raw)?;
    if items.is_empty() {
        return Err(Error::runtime(
            format!(
                "bad operation list \"{}\": must be one or more of {}",
                raw.as_str(),
                format_ops_vocab(vocab)
            ),
            crate::error::ErrorCode::Generic,
        ));
    }
    let mut ops = Vec::new();
    for it in items {
        let op = it.as_str();
        if !vocab.contains(&op) {
            return Err(Error::runtime(
                format!("bad operation \"{}\": must be {}", op, format_ops_vocab(vocab)),
                crate::error::ErrorCode::Generic,
            ));
        }
        if !ops.iter().any(|o| o == op) {
            ops.push(op.to_string());
        }
    }
    // Canonical order = the vocabulary order.
    ops.sort_by_key(|o| vocab.iter().position(|v| v == o).unwrap());
    Ok(ops)
}

fn format_ops_vocab(vocab: &[&str]) -> String {
    match vocab.len() {
        1 => vocab[0].to_string(),
        2 => format!("{} or {}", vocab[0], vocab[1]),
        _ => format!("{}, or {}", vocab[..vocab.len() - 1].join(", "), vocab[vocab.len() - 1]),
    }
}

/// Split `name` into (base, Some(elem)) when it is an element reference.
fn split_elem(name: &str) -> Option<(&str, &str)> {
    let open = name.find('(')?;
    let close = name.rfind(')')?;
    if close == name.len() - 1 && open > 0 {
        Some((&name[..open], &name[open + 1..close]))
    } else {
        None
    }
}

/// Register a variable trace; element traces on never-set elements also
/// create the phantom element (visible to `info exists` on the array,
/// invisible to names/get/size) and invalidate array searches.
fn add_variable(interp: &mut Interp, rest: &[Value]) -> Result<Value> {
    if rest.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "trace add variable",
            4,
            rest.len() + 1,
            "name opList command",
        ));
    }
    let name = rest[0].as_str().to_string();
    let ops = parse_ops(interp, &rest[1], VAR_OPS)?;
    let script = rest[2].as_str().to_string();
    let trace = VarTrace { ops, script };
    if let Some((base, elem)) = split_elem(&name) {
        let base_s = base.to_string();
        if !interp.is_array_semantic(base) {
            if interp.var_exists(base) {
                return Err(Error::runtime(
                    format!("can't trace \"{}\": variable isn't array", name),
                    crate::error::ErrorCode::Generic,
                ));
            }
            // Phantom creation: the array starts existing (size 0).
            interp.mark_array(&base_s)?;
        }
        let key = interp.array_stamp_key(&base_s);
        let elem = elem.to_string();
        let exists = interp.get_var(&name).is_ok();
        let entry = interp.elem_traces.entry(key.clone()).or_default();
        entry.entry(elem.clone()).or_default().push(trace);
        if !exists {
            interp.trace_phantoms.entry(key).or_default().insert(elem);
            // Structural change: searches die (set-old-9.10).
            interp.bump_stamp_by_name(&base_s);
        }
    } else {
        let key = interp.array_stamp_key(&name);
        interp.var_traces.entry(key).or_default().push(trace);
    }
    Ok(Value::empty())
}

fn remove_variable(interp: &mut Interp, rest: &[Value]) -> Result<Value> {
    if rest.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            "trace remove variable",
            4,
            rest.len() + 1,
            "name opList command",
        ));
    }
    let name = rest[0].as_str().to_string();
    let ops = parse_ops(interp, &rest[1], VAR_OPS)?;
    let script = rest[2].as_str().to_string();
    if let Some((base, elem)) = split_elem(&name) {
        let base = base.to_string();
        let key = interp.array_stamp_key(&base);
        let elem = elem.to_string();
        if let Some(map) = interp.elem_traces.get_mut(&key) {
            if let Some(v) = map.get_mut(&elem) {
                v.retain(|t| t.script != script || t.ops != ops);
                if v.is_empty() {
                    map.remove(&elem);
                }
            }
        }
    } else {
        let key = interp.array_stamp_key(&name);
        if let Some(v) = interp.var_traces.get_mut(&key) {
            v.retain(|t| t.script != script || t.ops != ops);
        }
    }
    Ok(Value::empty())
}

fn info_variable(interp: &mut Interp, rest: &[Value]) -> Result<Value> {
    if rest.len() != 1 {
        return Err(Error::wrong_args_with_usage(
            "trace info variable",
            3,
            rest.len() + 1,
            "name",
        ));
    }
    let name = rest[0].as_str();
    let mut out: Vec<Value> = Vec::new();
    let traces = if let Some((base, elem)) = split_elem(name) {
        let key = interp.array_stamp_key(base);
        interp
            .elem_traces
            .get(&key)
            .and_then(|m| m.get(elem))
            .cloned()
            .unwrap_or_default()
    } else {
        let key = interp.array_stamp_key(name);
        interp.var_traces.get(&key).cloned().unwrap_or_default()
    };
    for t in traces.into_iter().rev() {
        let rendered = t.ops.join(" ");
        out.push(Value::from_list(&[
            Value::from_str(&rendered),
            Value::from_str(&t.script),
        ]));
    }
    Ok(Value::from_list(&out))
}

/// Shared storage handling for command/execution traces. Both require the
/// command to exist (`unknown command "X"` — trace-19.0.1, trace-28.8,
/// trace-28.9).
fn stored_command_key(interp: &Interp, name: &str) -> Result<String> {
    match super::proc::resolve_command_key(interp, name) {
        Some(k) => Ok(k),
        None => Err(Error::runtime(
            format!("unknown command \"{}\"", name),
            crate::error::ErrorCode::NotFound,
        )),
    }
}

fn store_trace(
    interp: &mut Interp,
    table: &mut HashMap<String, Vec<(Vec<String>, String)>>,
    ty: &str,
    vocab: &[&str],
    rest: &[Value],
) -> Result<Value> {
    if rest.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            format!("trace add {}", ty).as_str(),
            4,
            rest.len() + 1,
            "name opList command",
        ));
    }
    let name = rest[0].as_str().to_string();
    let ops = parse_ops(interp, &rest[1], vocab)?;
    let script = rest[2].as_str().to_string();
    let key = stored_command_key(interp, &name)?;
    table.entry(key).or_default().push((ops, script));
    Ok(Value::empty())
}

fn store_exec_trace(
    interp: &mut Interp,
    table: &mut HashMap<String, Vec<ExecTrace>>,
    ty: &str,
    vocab: &[&str],
    rest: &[Value],
) -> Result<Value> {
    if rest.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            format!("trace add {}", ty).as_str(),
            4,
            rest.len() + 1,
            "name opList command",
        ));
    }
    let name = rest[0].as_str().to_string();
    let ops = parse_ops(interp, &rest[1], vocab)?;
    let script = rest[2].as_str().to_string();
    let key = stored_command_key(interp, &name)?;
    let id = interp.next_exec_trace_id();
    table.entry(key).or_default().push(ExecTrace { id, ops, script });
    Ok(Value::empty())
}

use std::collections::HashMap;

impl Interp {
    pub(crate) fn next_exec_trace_id(&mut self) -> u64 {
        self.exec_trace_ids += 1;
        self.exec_trace_ids
    }

    /// Move command traces when their command is renamed; an empty
    /// `new_key` (delete) drops them.  Execution traces follow the same
    /// rename.
    pub(crate) fn rekey_cmd_traces(&mut self, old_key: &str, new_key: &str) {
        if let Some(v) = self.cmd_traces.remove(old_key) {
            if !new_key.is_empty() {
                self.cmd_traces.entry(new_key.to_string()).or_default().extend(v);
            }
        }
        if let Some(v) = self.exec_traces.remove(old_key) {
            if !new_key.is_empty() {
                self.exec_traces.entry(new_key.to_string()).or_default().extend(v);
            }
        }
    }

    /// `proc` redefinition silently drops the old command's stored
    /// traces (tclsh: delete+create, no rename trace fires).
    pub(crate) fn wipe_cmd_exec_traces(&mut self, key: &str) {
        self.cmd_traces.remove(key);
        self.exec_traces.remove(key);
    }

    /// Fire one batch of execution-trace records by id (most-recent
    /// first).  `extra` carries the op-specific middle arguments (result /
    /// code for leave-style ops).  When `propagate` is set the callback's
    /// error aborts the traced command; otherwise the error is discarded
    /// and the errored record is auto-removed (tclsh 8.6.17).
    fn exec_fire_ids(
        &mut self,
        key: &str,
        ids: &[u64],
        cmdtext: &str,
        op: &str,
        extra: &[&str],
        propagate: bool,
    ) -> Result<()> {
        let records: Vec<(u64, String)> = match self.exec_traces.get(key) {
            Some(v) => v
                .iter()
                .filter(|t| ids.contains(&t.id) && t.ops.iter().any(|o| o == op))
                .map(|t| (t.id, t.script.clone()))
                .collect(),
            None => return Ok(()),
        };
        let q = |v: &str| Value::from_list(&[Value::from_str(v)]).as_str().to_string();
        for (id, script) in records.into_iter().rev() {
            let mut cmd = format!("{} {}", script, q(cmdtext));
            for e in extra {
                cmd.push(' ');
                cmd.push_str(&q(e));
            }
            cmd.push(' ');
            cmd.push_str(op);
            self.exec_step_running += 1;
            let r = self.eval_isolated(&cmd);
            self.exec_step_running -= 1;
            if let Err(e) = r {
                if propagate {
                    return Err(e);
                }
                // Errored trace record is removed (tclsh).
                if let Some(v) = self.exec_traces.get_mut(key) {
                    v.retain(|t| t.id != id);
                }
            }
        }
        Ok(())
    }

    /// Fire the `enter` execution traces for `key`; the callback's error
    /// replaces the command's own invocation (trace-40.1).
    pub(crate) fn exec_fire_enter(&mut self, key: &str, cmdtext: &str) -> Result<()> {
        if self.exec_step_running > 0 || self.exec_traces.is_empty() {
            return Ok(());
        }
        let ids: Vec<u64> = match self.exec_traces.get(key) {
            Some(v) => v.iter().map(|t| t.id).collect(),
            None => return Ok(()),
        };
        self.exec_fire_ids(key, &ids, cmdtext, "enter", &[], true)
    }

    /// Fire the `leave` execution traces for `key`; errors are background
    /// errors (discarded; errored record removed).
    pub(crate) fn exec_fire_leave(&mut self, key: &str, cmdtext: &str, code: &str, result: &str) {
        if self.exec_step_running > 0 || self.exec_traces.is_empty() {
            return;
        }
        let ids: Vec<u64> = match self.exec_traces.get(key) {
            Some(v) => v.iter().map(|t| t.id).collect(),
            None => return,
        };
        let _ = self.exec_fire_ids(key, &ids, cmdtext, "leave", &[code, result], false);
    }

    /// Enterstep half of a command about to dispatch inside a traced
    /// proc's body: snapshot the caller's enterstep/leavestep records,
    /// fire enterstep (errors discarded).  `None` when nothing can fire —
    /// no step bookkeeping needed.
    pub(crate) fn exec_step_begin(&mut self, args: &[Value]) -> Option<ExecStepCtx> {
        if self.exec_step_running > 0 || self.exec_traces.is_empty() {
            return None;
        }
        let key = self.exec_step_stack.last()?.clone();
        let snapshot: Vec<u64> = match self.exec_traces.get(&key) {
            Some(v) => v.iter().map(|t| t.id).collect(),
            None => return None,
        };
        let cmdtext = Value::from_list(args).as_str().to_string();
        let _ = self.exec_fire_ids(&key, &snapshot, &cmdtext, "enterstep", &[], false);
        Some(ExecStepCtx { key, cmdtext, snapshot })
    }

    /// Leavestep half: fire the snapshot's records that are *still
    /// registered* under the caller's key (trace-34.1: a record removed
    /// and re-added inside the enterstep callback must NOT fire — identity
    /// is the registration, not the tuple).
    pub(crate) fn exec_step_end(&mut self, ctx: &ExecStepCtx, code: &str, result: &str) {
        if self.exec_step_running > 0 {
            return;
        }
        let _ = self.exec_fire_ids(
            &ctx.key,
            &ctx.snapshot,
            &ctx.cmdtext,
            "leavestep",
            &[code, result],
            false,
        );
    }

    /// Fire command traces for `op` ("rename" | "delete"): each callback
    /// script gets ` oldName newName op` appended with `::`-qualified
    /// names (tclsh: `lappend got` sees `::foo ::bar rename`); errors are
    /// background errors and discarded. Delete traces die with the
    /// command.
    pub(crate) fn fire_cmd_traces(&mut self, key: &str, old: &str, new: &str, op: &str) {
        let scripts: Vec<String> = self
            .cmd_traces
            .get(key)
            .map(|v| {
                v.iter()
                    .filter(|(ops, _)| ops.iter().any(|o| o == op))
                    .map(|(_, s)| s.clone())
                    .collect()
            })
            .unwrap_or_default();
        let qual = |v: &str| {
            if v.is_empty() {
                String::new()
            } else {
                super::namespace::qualify("::", v)
            }
        };
        let q =
            |v: &str| Value::from_list(&[Value::from_str(v)]).as_str().to_string();
        let (oldq, newq) = (qual(old), qual(new));
        for s in scripts.into_iter().rev() {
            let cmd = format!("{} {} {} {}", s, q(&oldq), q(&newq), op);
            let _ = self.eval_isolated(&cmd);
        }
        if op == "delete" {
            self.cmd_traces.remove(key);
        }
    }
}

pub fn cmd_trace(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("trace", 2, args.len(), "option ?arg ...?"));
    }
    let sub = resolve_subcmd(args[1].as_str())?;
    match sub {
        "add" => {
            if args.len() < 3 {
                return Err(Error::wrong_args_with_usage(
                    "trace add",
                    3,
                    args.len(),
                    "type ?arg ...?",
                ));
            }
            match resolve_type(args[2].as_str())? {
                "variable" => add_variable(interp, &args[3..]),
                "command" => {
                    let mut t = std::mem::take(&mut interp.cmd_traces);
                    let r = store_trace(interp, &mut t, "command", CMD_OPS, &args[3..]);
                    interp.cmd_traces = t;
                    r
                }
                "execution" => {
                    let mut t = std::mem::take(&mut interp.exec_traces);
                    let r = store_exec_trace(interp, &mut t, "execution", EXEC_OPS, &args[3..]);
                    interp.exec_traces = t;
                    r
                }
                _ => unreachable!(),
            }
        }
        "remove" => {
            if args.len() < 3 {
                return Err(Error::wrong_args_with_usage(
                    "trace remove",
                    3,
                    args.len(),
                    "type ?arg ...?",
                ));
            }
            match resolve_type(args[2].as_str())? {
                "variable" => remove_variable(interp, &args[3..]),
                "command" => {
                    let mut t = std::mem::take(&mut interp.cmd_traces);
                    let r = remove_stored(interp, &mut t, CMD_OPS, &args[3..], "command");
                    interp.cmd_traces = t;
                    r
                }
                "execution" => {
                    let mut t = std::mem::take(&mut interp.exec_traces);
                    let r = remove_exec_trace(interp, &mut t, EXEC_OPS, &args[3..], "execution");
                    interp.exec_traces = t;
                    r
                }
                _ => unreachable!(),
            }
        }
        "info" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage("trace info", 3, args.len(), "type name"));
            }
            match resolve_type(args[2].as_str())? {
                "variable" => info_variable(interp, &args[3..]),
                "command" => {
                    let t = std::mem::take(&mut interp.cmd_traces);
                    let r = info_stored(interp, &t, &args[3..]);
                    interp.cmd_traces = t;
                    r
                }
                "execution" => {
                    let t = std::mem::take(&mut interp.exec_traces);
                    let r = info_exec_trace(interp, &t, &args[3..]);
                    interp.exec_traces = t;
                    r
                }
                _ => unreachable!(),
            }
        }
        // Legacy character-op forms (rwua); rare, corpus-free, but parse.
        "variable" | "vdelete" | "vinfo" => {
            legacy_trace(interp, sub, &args[2..])
        }
        _ => unreachable!(),
    }
}

fn remove_stored(
    interp: &mut Interp,
    table: &mut HashMap<String, Vec<(Vec<String>, String)>>,
    vocab: &[&str],
    rest: &[Value],
    ty: &str,
) -> Result<Value> {
    if rest.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            format!("trace remove {}", ty).as_str(),
            4,
            rest.len() + 1,
            "name opList command",
        ));
    }
    let name = rest[0].as_str().to_string();
    let ops = parse_ops(interp, &rest[1], vocab)?;
    let script = rest[2].as_str().to_string();
    // Mirror the store-time key resolution; a missing command errors
    // (trace-27.2, trace-27.3).
    let key = stored_command_key(interp, &name)?;
    if let Some(v) = table.get_mut(&key) {
        v.retain(|(o, s)| *o != ops || *s != script);
    }
    Ok(Value::empty())
}

fn remove_exec_trace(
    interp: &mut Interp,
    table: &mut HashMap<String, Vec<ExecTrace>>,
    vocab: &[&str],
    rest: &[Value],
    ty: &str,
) -> Result<Value> {
    if rest.len() != 3 {
        return Err(Error::wrong_args_with_usage(
            format!("trace remove {}", ty).as_str(),
            4,
            rest.len() + 1,
            "name opList command",
        ));
    }
    let name = rest[0].as_str().to_string();
    let ops = parse_ops(interp, &rest[1], vocab)?;
    let script = rest[2].as_str().to_string();
    let key = stored_command_key(interp, &name)?;
    if let Some(v) = table.get_mut(&key) {
        v.retain(|t| t.ops != ops || t.script != script);
    }
    Ok(Value::empty())
}

fn info_stored(
    interp: &mut Interp,
    table: &HashMap<String, Vec<(Vec<String>, String)>>,
    rest: &[Value],
) -> Result<Value> {
    if rest.len() != 1 {
        return Err(Error::wrong_args_with_usage(
            "trace info",
            3,
            rest.len() + 1,
            "name",
        ));
    }
    let name = rest[0].as_str();
    let mut out: Vec<Value> = Vec::new();
    // A missing command errors (trace-27.3 shape).
    let key = stored_command_key(interp, name)?;
    if let Some(v) = table.get(&key) {
        for (ops, script) in v {
            out.push(Value::from_list(&[
                Value::from_str(&ops.join(" ")),
                Value::from_str(script),
            ]));
        }
    }
    let _ = interp;
    Ok(Value::from_list(&out))
}

fn info_exec_trace(
    interp: &mut Interp,
    table: &HashMap<String, Vec<ExecTrace>>,
    rest: &[Value],
) -> Result<Value> {
    if rest.len() != 1 {
        return Err(Error::wrong_args_with_usage(
            "trace info",
            3,
            rest.len() + 1,
            "name",
        ));
    }
    let name = rest[0].as_str();
    let mut out: Vec<Value> = Vec::new();
    let key = stored_command_key(interp, name)?;
    if let Some(v) = table.get(&key) {
        for t in v {
            out.push(Value::from_list(&[
                Value::from_str(&t.ops.join(" ")),
                Value::from_str(&t.script),
            ]));
        }
    }
    let _ = interp;
    Ok(Value::from_list(&out))
}

/// Legacy `trace variable name opChars command` (also vdelete/vinfo).
fn legacy_trace(interp: &mut Interp, sub: &str, rest: &[Value]) -> Result<Value> {
    match sub {
        "vinfo" => {
            if rest.len() != 1 {
                return Err(Error::wrong_args_with_usage(
                    "trace vinfo",
                    3,
                    rest.len(),
                    "name",
                ));
            }
            let name = rest[0].as_str();
            // Render whole-var traces with character ops (tclsh legacy).
            let key = interp.array_stamp_key(name);
            let traces = interp.var_traces.get(&key).cloned().unwrap_or_default();
            let mut out: Vec<Value> = Vec::new();
            for t in traces {
                let chars: String = t
                    .ops
                    .iter()
                    .map(|o| match o.as_str() {
                        "read" => 'r',
                        "write" => 'w',
                        "unset" => 'u',
                        _ => 'a',
                    })
                    .collect();
                out.push(Value::from_list(&[
                    Value::from_str(&chars),
                    Value::from_str(&t.script),
                ]));
            }
            Ok(Value::from_list(&out))
        }
        _ => {
            if rest.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    format!("trace {}", sub).as_str(),
                    4,
                    rest.len(),
                    "name ops command",
                ));
            }
            let name = rest[0].as_str().to_string();
            let raw = rest[1].as_str();
            let mut ops: Vec<String> = Vec::new();
            for c in raw.chars() {
                let op = match c {
                    'r' => "read",
                    'w' => "write",
                    'u' => "unset",
                    'a' => "array",
                    _ => {
                        return Err(Error::runtime(
                            format!("bad operation \"{}\": must be array, read, unset, or write", c),
                            crate::error::ErrorCode::Generic,
                        ))
                    }
                };
                if !ops.iter().any(|o| o == op) {
                    ops.push(op.to_string());
                }
            }
            ops.sort_by_key(|o| VAR_OPS.iter().position(|v| v == o).unwrap());
            let script = rest[2].as_str().to_string();
            if sub == "variable" {
                let key = interp.array_stamp_key(&name);
                interp
                    .var_traces
                    .entry(key)
                    .or_default()
                    .push(VarTrace { ops, script });
            } else if let Some(v) = interp.var_traces.get_mut(&interp.array_stamp_key(&name)) {
                v.retain(|t| t.script != script || t.ops != ops);
            }
            Ok(Value::empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    fn eval_err(script: &str) -> String {
        Interp::new().eval(script).unwrap_err().to_string()
    }

    #[test]
    fn test_trace_add_info_remove() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("proc cb {a b c} {}; trace add variable x {read write} cb; trace info variable x")
                .unwrap()
                .as_str(),
            "{{read write} cb}"
        );
        interp
            .eval("trace remove variable x {write read} cb")
            .unwrap();
        assert_eq!(
            interp.eval("trace info variable x").unwrap().as_str(),
            ""
        );
    }

    #[test]
    fn test_trace_write_read_fire_order() {
        let mut interp = Interp::new();
        let out = interp
            .eval(
                "set log {}\n\
                 proc cb {args} {lappend ::log [list [lindex $args 0] [lindex $args 1] [lindex $args 2]]}\n\
                 catch {unset x}\n\
                 trace add variable x {read write} cb\n\
                 set x 5\n\
                 set x\n\
                 set log",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(out, "{x {} write} {x {} read}");
    }

    #[test]
    fn test_trace_write_error_propagates() {
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {unset x}; trace add variable x write {error boom;#}; catch {set x 6} m; list $m [info exists x]")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "{can't set \"x\": boom} 1");
    }

    #[test]
    fn test_trace_ops_errors() {
        let mut interp = Interp::new();
        for (script, want) in [
            ("catch {trace add variable y rw cb} m; set m", "bad operation \"rw\": must be array, read, unset, or write"),
            ("catch {trace add variable y {} cb} m; set m", "bad operation list \"\": must be one or more of array, read, unset, or write"),
            ("catch {trace add variable z} m; set m", "wrong # args: should be \"trace add variable name opList command\""),
        ] {
            assert_eq!(interp.eval(script).unwrap().as_str(), want, "{}", script);
        }
    }

    #[test]
    fn test_trace_element_phantom() {
        // set-old-8.19/8.25: traced-but-unset element invisible to
        // names/get/size; array starts existing.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "catch {unset a}; set a(x) 3; trace add var a(y) write {}; \
                 list [array get a] [array size a] [info exists a] [catch {set a(y)} m] $m",
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "{x 3} 1 1 1 {can't read \"a(y)\": no such element in array}");
    }

    #[test]
    fn test_trace_phantom_invalidates_searches() {
        // set-old-9.10 vs 9.11: phantom element trace kills searches,
        // existing-element trace does not.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "catch {unset a}; set a(a) 1; set x [array startsearch a]; \
                 trace add var a(b) read {}; \
                 list [catch {array next a $x} m] $m"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "1 {couldn't find search \"s-1-a\"}");
        let r2 = interp
            .eval(
                "catch {unset b2}; set b2(a) 1; set x [array startsearch b2]; \
                 trace add var b2(a) read {}; array next b2 $x"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r2, "a");
    }

    #[test]
    fn test_trace_array_op_fires_on_creation() {
        // tclsh: 'array' traces fire when `array set` creates the array.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set log {}; proc at {n1 n2 op} {lappend ::log $op}; \
                 trace add variable g7 array at; array set g7 {k 1}; set log"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "array");
    }

    #[test]
    fn test_trace_unset_fires_after_delete() {
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set log {}; proc u {n1 n2 op} {lappend ::log [list $n1 $n2 $op]}; \
                 set g8 5; trace add variable g8 unset u; unset g8; \
                 list $log [info exists g8] [catch {unset g8} m] $m"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "{{g8 {} unset}} 0 1 {can't unset \"g8\": no such variable}");
    }

    #[test]
    fn test_cmd_trace_fires_on_rename_and_delete() {
        // tclsh: rename/delete traces fire after the operation; errors
        // in the callback are background errors (discarded).
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set log {}; \
                 proc foo {} {}; \
                 trace add command foo rename {append ::log R;#}; \
                 trace add command foo delete {error boom}; \
                 rename foo bar; rename bar {}; \
                 list $log [info commands bar]"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "R {}");
    }

    #[test]
    fn cmd_traces_follow_rename() {
        // Traces attach to the command record: rename foo bar, then
        // renaming bar again still fires; info moves to the new name.
        // (tclsh 8.6.17: `trace info command` on a name that no longer
        // resolves — renamed away or never defined — is
        // `unknown command "<as-typed>"`.)
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set log {}; \
                 proc foo {} {}; \
                 trace add command foo rename {append ::log R;#}; \
                 rename foo bar; rename bar baz; \
                 list $log [trace info command baz]"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "RR {{rename {append ::log R;#}}}");
        assert_eq!(
            eval_err("trace info command neverexisted"),
            "unknown command \"neverexisted\""
        );
    }

    #[test]
    fn cmd_trace_callback_receives_old_and_new_names() {
        // tclsh probe: callback sees `::old ::new rename` / `::old {} delete`.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set got {}; \
                 proc foo {} {}; \
                 trace add command foo rename {lappend ::got}; \
                 trace add command foo delete {lappend ::got}; \
                 rename foo bar; rename bar {}; \
                 set got"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "::foo ::bar rename ::bar {} delete");
    }

    #[test]
    fn test_trace_most_recent_fires_first() {
        // tclsh: two write traces fire most-recent-first (T2 T1).
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set log {}; catch {unset x}; \
                 trace add variable x write {append ::log T1;#}; \
                 trace add variable x write {append ::log T2;#}; \
                 set x 1; set log"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "T2T1");
    }

    #[test]
    fn test_trace_info_reverse_order() {
        // trace-14.16: trace info lists most recent first.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "catch {unset x}; \
                 trace add variable x write {traceTag 1}; \
                 trace add variable x write traceProc; \
                 trace add variable x write {traceTag 2}; \
                 trace info variable x"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(
            r,
            "{write {traceTag 2}} {write traceProc} {write {traceTag 1}}"
        );
    }

    #[test]
    fn test_trace_names_on_missing_fires_array() {
        // trace-5.8: array names on a missing array fires 'array' traces.
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "catch {unset x}; \
                 trace add variable x array {set x(foo) 1 ;#}; \
                 set res \"names: [array names x]\"; set res"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "names: foo");
    }

    #[test]
    fn test_trace_add_command_requires_existing() {
        // trace-19.0.1: command traces need the command to exist.
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {trace add command nosuchname rename tc} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "unknown command \"nosuchname\"");
    }

    #[test]
    fn test_trace_read_fires_on_missing_scalar_not_elem() {
        let mut interp = Interp::new();
        let r = interp
            .eval(
                "set log {}; proc rf {a b c} {lappend ::log $a}; \
                 catch {unset g5}; trace add variable g5 read rf; \
                 catch {set g5}; set a1 $log; set log {}; \
                 catch {set g5(3)}; list $a1 $log [catch {info exists g5} x]"
            )
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "g5 {} 0");
    }
}
