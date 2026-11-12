//! Coroutines — `coroutine`/`yield`/`yieldto` (tclsh 8.6 semantics).
//!
//! Architecture note: rtcl's bytecode executor recurses per unit (one
//! Rust frame per `exec_bytecode`), so a suspension cannot freeze the
//! Rust continuation.  Instead each bytecode level that unwinds on
//! [`Error::Suspend`] freezes its executor state (`pc` mid-loop, operand
//! stack, loop/foreach frames) into [`Interp::coro_chain`]; the frames
//! above the coroutine base move into the record; the proc epilogues are
//! skipped entirely.  Resuming replays the chain from the innermost
//! level outward — each level's loop re-enters with its frozen state,
//! the resume argument becomes the suspended `yield`'s return value, and
//! a level that finishes feeds its result to the level below, exactly
//! mirroring the return path that unwound it.
//!
//! Known divergence (documented in DIVERGENCES.md): a `yield` inside a
//! nested `eval`'d AST script (an `EvalScript` fallback or the tree-walk
//! path) cannot freeze the AST continuation — it surfaces as the runtime
//! error below instead of suspending.  Bytecode-level yields (proc
//! bodies, loop bodies, `if` arms) suspend at any depth.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

/// What the suspended `yield`/`yieldto` handed across the boundary.
pub(crate) enum Pending {
    /// `yield ?value?` — the resume command's result.
    Yield(Value),
    /// `yieldto command ?arg ...?` — on suspend, `command` runs in the
    /// OUTER context; its result is the resume command's result, and the
    /// next resume argument becomes `yieldto`'s return inside the body.
    YieldTo(Vec<Value>),
}

/// Context of the coroutine currently running on the interpreter.
pub(crate) struct CoroCtx {
    pub name: String,
    /// `interp.frames.len()` when the coroutine's (or the resume's) body
    /// started — frames above belong to the coroutine and move into the
    /// record on suspension.
    pub frames_base: usize,
    pub pending: Option<Pending>,
}

/// One suspended coroutine.
pub(crate) struct Coroutine {
    /// Frames above the entry base, moved out at suspension and pushed
    /// back on resume (empty while running).
    pub frames: Vec<Box<crate::interp::CallFrame>>,
    /// Frozen bytecode levels, innermost LAST (resume pops the tail).
    pub levels: Vec<(Rc<rtcl_parser::ByteCode>, crate::interp::vm_exec::SuspendedState)>,
}

use std::rc::Rc;

/// `coroutine name cmd ?arg ...?` — create the coroutine, register the
/// resume command `name` (deleted on completion), then run the first
/// invocation: its result is the first yield value, or the body's return
/// when the body completes without suspending (the command is deleted in
/// that case, tclsh coro-2.7).
pub fn cmd_coroutine(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "coroutine",
            3,
            args.len(),
            "name cmd ?arg ...?",
        ));
    }
    let raw_name = args[1].as_str();
    let name = super::namespace::qualify(&interp.current_namespace, raw_name);
    // tclsh allows re-creating a live coroutine of the same name: the
    // new one replaces the old (its resume command is overwritten too).
    interp.coroutines.insert(
        name.clone(),
        Coroutine { frames: Vec::new(), levels: Vec::new() },
    );
    // Register the resume command (visible, like any user command).
    let resume_name = name.trim_start_matches("::").to_string();
    if interp
        .commands
        .insert(resume_name.clone(), cmd_coro_resume)
        .is_none()
    {
        interp.note_new_command(&resume_name);
    }
    interp.note_cmd_mutation();

    let mut invocation: Vec<Value> = vec![args[2].clone()];
    invocation.extend(args[3..].iter().cloned());
    match run_coro_entry(interp, &name, invocation) {
        Ok(v) => Ok(v),
        Err(e) => {
            // Body errored (or a failed dispatch): the coroutine is gone
            // — tclsh deletes it (coro-4.1: the command name vanishes).
            destroy(interp, &name);
            Err(e)
        }
    }
}

/// Run the coroutine's FIRST invocation (`coroutine`'s body start).
fn run_coro_entry(interp: &mut Interp, name: &str, invocation: Vec<Value>) -> Result<Value> {
    interp.coro_ctx = Some(CoroCtx {
        name: name.to_string(),
        frames_base: interp.frames.len(),
        pending: None,
    });
    let r = interp.dispatch_values(&invocation);
    collect_suspend(interp, name, r)
}

/// Common suspend/completion/error collector for both the entry run and
/// an adopted replay: converts [`Error::Suspend`] into the resumer's
/// result and finalizes the record on completion or error.
fn collect_suspend(interp: &mut Interp, name: &str, r: Result<Value>) -> Result<Value> {
    let ctx = interp.coro_ctx.take();
    match r {
        Err(e) if e.is_suspend() => {
            let ctx = ctx.expect("suspend without context");
            let chain = std::mem::take(&mut interp.coro_chain);
            let pending = ctx.pending;
            if chain.is_empty() {
                // No bytecode level froze (AST-eval yield): the
                // continuation is unrecoverable.
                destroy(interp, name);
                return Err(Error::runtime(
                    "yield cannot suspend across an eval boundary",
                    ErrorCode::Generic,
                ));
            }
            let record = interp.coroutines.get_mut(name).expect("record");
            // The chain unwound innermost-first; store outermost-first so
            // resume pops the INNERMOST from the tail.
            let mut chain = chain;
            chain.reverse();
            record.levels = chain;
            record.frames = interp.frames.drain(ctx.frames_base..).collect();
            match pending {
                Some(Pending::Yield(v)) => Ok(v),
                Some(Pending::YieldTo(vals)) => {
                    // tclsh: the yieldto command runs in the OUTER
                    // context right now; its result is what the
                    // invoking/resuming command returns.
                    interp.dispatch_values(&vals)
                }
                None => Ok(Value::empty()),
            }
        }
        Ok(v) => {
            // Body completed: the coroutine is done, its command deleted
            // (tclsh coro-2.7).
            destroy(interp, name);
            Ok(v)
        }
        Err(e) => {
            destroy(interp, name);
            Err(e)
        }
    }
}

/// Delete the coroutine record and its resume command.
fn destroy(interp: &mut Interp, name: &str) {
    interp.coroutines.remove(name);
    let bare = name.trim_start_matches("::");
    if interp.commands.remove(bare).is_some() {
        interp.note_cmd_mutation();
    }
}

/// The resume command installed under the coroutine's name.
pub fn cmd_coro_resume(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let word = args[0].as_str();
    let name = resolve_record(interp, word);
    let Some(name) = name else {
        return Err(Error::InvalidCommand { name: word.to_string() });
    };
    if args.len() > 2 {
        return Err(Error::wrong_args_msg(format!(
            "wrong # args: should be \"{} ?arg?\"",
            word
        )));
    }
    let resume_arg = args.get(1).cloned().unwrap_or_else(Value::empty);
    let record_frames = match interp.coroutines.get_mut(&name) {
        Some(rec) if rec.levels.is_empty() => {
            // No frozen levels: either still running (cannot happen —
            // dispatch is re-entrant only via explicit resume) or a
            // record left behind; treat as gone.
            return Err(Error::InvalidCommand { name: word.to_string() });
        }
        Some(_) => {}
        None => return Err(Error::InvalidCommand { name: word.to_string() }),
    };
    let _ = record_frames;
    resume_coro(interp, &name, resume_arg)
}

fn resolve_record(interp: &Interp, word: &str) -> Option<String> {
    for cand in [
        word.to_string(),
        format!("::{}", word.trim_start_matches("::")),
    ] {
        if interp.coroutines.contains_key(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Replay the frozen chain: run the innermost level with the resume
/// argument as the suspended `yield`'s return; a finishing level feeds
/// its result to the level below.
fn resume_coro(interp: &mut Interp, name: &str, resume_arg: Value) -> Result<Value> {
    let levels = interp.coroutines.get(name).map(|r| r.levels.len()).unwrap_or(0);
    if levels == 0 {
        return Err(Error::InvalidCommand { name: name.to_string() });
    }
    // Push ALL frozen frames back; the base sits below the whole stack
    // (taken BEFORE the extend — frames above it belong to the coroutine).
    let frames = interp
        .coroutines
        .get_mut(name)
        .unwrap()
        .frames
        .drain(..)
        .collect::<Vec<_>>();
    let frames_base = interp.frames.len();
    interp.frames.extend(frames);
    interp.coro_ctx = Some(CoroCtx {
        name: name.to_string(),
        frames_base,
        pending: None,
    });
    let mut feed = resume_arg;
    loop {
        // Adopt the innermost frozen level.
        let level = interp
            .coroutines
            .get_mut(name)
            .unwrap()
            .levels
            .pop();
        let Some((code, mut state)) = level else {
            // Nothing frozen left but the body never signalled
            // completion — impossible (the innermost run either
            // suspends, errors, or completes).  Treat as done-empty.
            interp.coro_ctx = None;
            destroy(interp, name);
            return Ok(Value::empty());
        };
        // The resume argument is the suspended command's return value.
        state.stack_push(feed);
        let r = crate::interp::vm_exec::exec_bytecode_adopt(interp, &code, state);
        // Adopted replays have no proc machinery above them: decode the
        // proc-boundary completions here (return -code → Ok; the plain
        // error path stays an error).
        let r = match r {
            Err(e) if e.is_return() => {
                let v = match e {
                    Error::ControlFlow { value, .. } => {
                        value.unwrap_or_else(Value::empty)
                    }
                    other => Value::from_str(other.message_text().as_str()),
                };
                Ok(v)
            }
            other => other,
        };
        match r {
            Err(e) if e.is_suspend() => {
                // Suspended again: collect, hand the pending value (or
                // the yieldto command's result) to our resumer.
                let ctx = interp.coro_ctx.take();
                let ctx = ctx.expect("suspend without context");
                let chain = std::mem::take(&mut interp.coro_chain);
                let pending = ctx.pending;
                if chain.is_empty() {
                    destroy(interp, name);
                    return Err(Error::runtime(
                        "yield cannot suspend across an eval boundary",
                        ErrorCode::Generic,
                    ));
                }
                let record = interp.coroutines.get_mut(name).expect("record");
                let mut chain = chain;
                chain.reverse();
                record.levels.extend(chain);
                record.frames = interp.frames.drain(ctx.frames_base..).collect();
                return match pending {
                    Some(Pending::Yield(v)) => Ok(v),
                    Some(Pending::YieldTo(vals)) => interp.dispatch_values(&vals),
                    None => Ok(Value::empty()),
                };
            }
            Ok(v) => {
                // This level finished: its result is the pending call's
                // return for the level below.  Adopted levels have no
                // proc epilogue above them — pop the finished level's
                // frame here (the proc machinery below the coroutine
                // base owns the rest).
                interp.frames.pop();
                if interp.coroutines.get(name).map(|r| r.levels.is_empty()).unwrap_or(true) {
                    interp.coro_ctx = None;
                    destroy(interp, name);
                    return Ok(v);
                }
                feed = v;
            }
            Err(e) => {
                // Error inside the coroutine: it is done; the restored
                // frames unwind through their own epilogues.
                interp.coro_ctx = None;
                destroy(interp, name);
                return Err(e);
            }
        }
    }
}

/// `yield ?value?` — suspend; returns the next resume's single argument.
pub fn cmd_yield(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let value = match args.len() {
        1 => Value::empty(),
        2 => args[1].clone(),
        _ => {
            return Err(Error::wrong_args_with_usage("yield", 1, args.len(), "?value?"));
        }
    };
    match interp.coro_ctx.as_mut() {
        Some(ctx) => {
            ctx.pending = Some(Pending::Yield(value));
            Err(Error::Suspend)
        }
        None => Err(Error::runtime(
            "yield can only be called in a coroutine",
            ErrorCode::Generic,
        )),
    }
}

/// `yieldto command ?arg ...?` — suspend; `command` runs in the outer
/// context (its result is the resume command's result) and the next
/// resume argument becomes `yieldto`'s return value.
pub fn cmd_yieldto(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "yieldto",
            2,
            args.len(),
            "command ?arg ...?",
        ));
    }
    let vals: Vec<Value> = args[1..].to_vec();
    match interp.coro_ctx.as_mut() {
        Some(ctx) => {
            ctx.pending = Some(Pending::YieldTo(vals));
            Err(Error::Suspend)
        }
        None => Err(Error::runtime(
            "yieldto can only be called in a coroutine",
            ErrorCode::Generic,
        )),
    }
}
