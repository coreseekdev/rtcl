//! Bytecode execution driving the real interpreter state.
//!
//! This is the seam between [`ProcDef`]'s definition-time compilation and
//! the tree-walk: a proc body compiled once at `proc` time runs here
//! op-by-op, eliminating the per-call re-parse + AST walk.  Every op's
//! semantics are the tree-walk's — arithmetic goes through
//! `types::expr_ops`, variable reads through
//! [`Interp::eval_var_ref`], dispatches through
//! [`Interp::dispatch_values`] — so the two paths are interchangeable
//! down to errorInfo frames.
//!
//! Error-frame protocol (mirrors `eval_command` + the loop constructs):
//! each compiled command opens a site with `BeginCmd`.  A failing op is
//! frameless iff it belongs to the *body* of the current command's own
//! inline construct (`if` arm / `while` body — tracked via not-taken
//! condition jumps and [`OpCode::BodyMark`] for taken-jump else entries);
//! body errors propagate frameless exactly like the constructs' `err_fresh`
//! suppression in the tree-walk.  Everything else appends the site's
//! harness frame — through the word-error branch (`LoadVar`/`EvalScript`,
//! honouring `err_from_subst`) or the dispatch branch (everything else).

use super::{Interp, Rc};
use crate::error::{Error, Result};
use crate::value::Value;
use rtcl_parser::{ByteCode, Compiler, OpCode};

#[cfg(not(feature = "embedded"))]
use std::sync::OnceLock;

/// Compile a proc body once, at definition time.  `None` when the body
/// does not parse — the tree-walk reproduces the per-call parse error
/// exactly (`seed_parse_error` runs on every uncached `eval`).
pub(crate) fn compile_proc_body(body: &str, epoch: u64) -> Option<Rc<ByteCode>> {
    let unit = rtcl_parser::ScriptUnit::parse(body).ok()?;
    let mut code = Compiler::compile_unit(Rc::clone(&unit.source), &unit.commands);
    code.epoch = epoch;
    Some(Rc::new(code))
}

/// Bump [`Interp::tier1_epoch`] when `key`'s leaf name matches one of the
/// inline-folded Tier1 commands.  Only these ten names can be shadowed
/// out from under a compiled body — everything else in the bytecode goes
/// through dispatch (`Call`/`DynCall`) and re-resolves per call.
pub(crate) fn note_tier1_mutation(interp: &mut Interp, key: &str) {
    let leaf = key.rsplit("::").next().unwrap_or(key);
    if matches!(
        leaf,
        "set" | "if" | "while" | "for" | "expr" | "incr" | "return" | "exit" | "break"
            | "continue"
    ) {
        interp.tier1_epoch += 1;
    }
}

/// An unconditional epoch bump for a coarse mutation (namespace / object
/// tree teardown) whose affected command names aren't worth enumerating.
pub(crate) fn note_tier1_sweep(interp: &mut Interp) {
    interp.tier1_epoch += 1;
}

/// May the current call run its body through the bytecode executor?
/// Execution traces observe every dispatched command, so they pin the
/// proc to the tree-walk; `RTCL_NO_BYTECODE` is the manual escape hatch.
pub(crate) fn bytecode_applicable(interp: &Interp) -> bool {
    if !interp.exec_traces.is_empty() {
        return false;
    }
    #[cfg(not(feature = "embedded"))]
    {
        static DISABLED: OnceLock<bool> = OnceLock::new();
        !*DISABLED.get_or_init(|| std::env::var_os("RTCL_NO_BYTECODE").is_some())
    }
    #[cfg(feature = "embedded")]
    {
        let _ = interp;
        true
    }
}

// ---------------------------------------------------------------------------
// Executor state
// ---------------------------------------------------------------------------

/// Stack slot: a plain value or a `{*}`-expanded element run.
enum Entry {
    Val(Value),
    Expanded(Vec<Value>),
}

/// One active bytecode loop (`LoopEnter` … `LoopExit`).
struct LoopFrame {
    cont: u32,
    brk: u32,
    /// `bodies` depth at `LoopEnter` — a `break`/`continue` jumping out of
    /// the loop pops every body region opened inside it.
    bodies_len: usize,
    /// Stack depth at `LoopEnter` — a `break`/`continue` can jump out from
    /// the middle of a body command whose value is still on the stack
    /// (`set y 1; break`); the tree-walk has no stack, so the jump must
    /// discard the partial command's leftovers.
    stack_len: usize,
}

/// Route a break/continue that surfaced as an *error* (it crossed a
/// dispatched command or nested-script boundary — the compiled Break /
/// Continue ops jump directly) to the innermost inline loop, mirroring
/// cmd_while/cmd_for's body-eval handling.  With no inline loop of our
/// own the signal belongs to an enclosing tree-walk construct: propagate
/// unchanged.
fn loop_signal(st: &mut VmState, e: Error) -> Result<()> {
    if e.loop_level() > 1 {
        return Err(e.with_decremented_loop_level());
    }
    let is_brk = e.is_break();
    match st.loops.last() {
        Some(l) => {
            st.bodies.truncate(l.bodies_len);
            st.stack.truncate(l.stack_len);
            st.pc = if is_brk { l.brk as usize } else { l.cont as usize };
            Ok(())
        }
        None => Err(e),
    }
}


/// Execute a proc body's compiled bytecode on `interp`.
///
/// Callers gate on [`bytecode_applicable`] and `!code.fallback`; errors
/// return identically shaped [`Error`]s to the tree-walk so `call_proc`'s
/// boundary handling (`(procedure …)` frames, return decoding, break →
/// error conversion) needs no changes.
pub(crate) fn exec_bytecode(interp: &mut Interp, code: &ByteCode) -> Result<Value> {
    // The unit is one "current script": command-substitution errors inside
    // it defer their enclosing-command frame via err_pending_top, and the
    // site lines are absolute already (entry_offset + site.line).
    let entry_offset = interp.line_offset;
    let saved_source = std::mem::replace(&mut interp.cur_source, Rc::clone(&code.source));
    let saved_text = std::mem::replace(
        &mut interp.cur_cmd_text,
        rtcl_parser::SrcSpan { start: 0, end: 0 },
    );
    let saved_line = std::mem::replace(&mut interp.cur_cmd_line, 0);
    let saved_srcs = std::mem::replace(&mut interp.cur_cmd_word_srcs, Rc::new(Vec::new()));
    // The unit "acts at" this offset: nested evals (EvalScript →
    // `Interp::eval`) must see the same base the tree-walk would have
    // left installed (e.g. a `while` body rebased by cmd_while).
    interp.line_offset = entry_offset;

    let r = exec_inner(interp, code, entry_offset);

    interp.cur_source = saved_source;
    interp.cur_cmd_text = saved_text;
    interp.cur_cmd_line = saved_line;
    interp.cur_cmd_word_srcs = saved_srcs;
    interp.line_offset = entry_offset;
    r
}

fn exec_inner(interp: &mut Interp, code: &ByteCode, entry_offset: usize) -> Result<Value> {
    let ops = code.ops();
    let mut st = VmState {
        stack: Vec::with_capacity(16),
        loops: Vec::new(),
        bodies: Vec::new(),
        expand_base: 0,
        cur_site: usize::MAX,
        entry_offset,
        pc: 0,
    };

    let result = loop {
        // Stream end: the tree-walk's result is the last command's, which
        // the stack discipline leaves on top (`set`/`incr` fast paths and
        // dispatches all push their result).
        let Some(op) = ops.get(st.pc) else { break st.pop_val() };
        let op = op.clone();
        st.pc += 1;
        if let Err(e) = exec_op(interp, code, &op, &mut st) {
            // Control-flow completions propagate framelessly — except
            // break/continue that crossed a *dispatched command* or
            // *nested script* boundary inside an inline loop: exactly
            // what cmd_while/cmd_for catch from their body evals.
            // Level 1 belongs to this loop (jump); deeper levels
            // propagate decremented (loops.rs parity).
            if !interp.err_is_error(&e) && (e.is_break() || e.is_continue()) {
                match loop_signal(&mut st, e) {
                    Ok(()) => continue,
                    Err(e) => return Err(e),
                }
            }
            if !interp.err_is_error(&e) {
                return Err(e);
            }
            // A body error of the current command's own inline
            // construct propagates frameless (tree-walk: the
            // construct's err_fresh suppression).
            if st.bodies.last() == Some(&st.cur_site) {
                return Err(e);
            }
            // Everything else appends this command's harness frame —
            // word-op failures through the substitution-aware branch,
            // dispatch/expr failures through the plain one.
            let site = &code.sites[st.cur_site];
            let text = site.text.slice(&code.source);
            let line = st.entry_offset + site.line as usize;
            let is_word_op = matches!(op, OpCode::LoadVar(_) | OpCode::EvalScript);
            if is_word_op && std::mem::take(&mut interp.err_from_subst) {
                interp.err_pending_top = Some(Rc::from(text));
                interp.err_line = line;
            } else {
                let msg = e.message_text();
                interp.err_harness_frame(&msg, text, line);
            }
            return Err(e);
        }
    };

    interp.result = result.clone();
    Ok(result)
}

struct VmState {
    stack: Vec<Entry>,
    loops: Vec<LoopFrame>,
    /// Body regions currently executing, as the `BeginCmd` site index of
    /// the construct that owns each (`if` arm / `while` body).  Pushed by
    /// not-taken condition jumps and `BodyMark`, popped by the plain
    /// `Jump` that ends every body.
    bodies: Vec<usize>,
    /// Stack base of the current command's `{*}` expansion region.
    expand_base: usize,
    /// Site index of the `BeginCmd` most recently executed.
    cur_site: usize,
    entry_offset: usize,
    pc: usize,
}

impl VmState {
    fn pop_val(&mut self) -> Value {
        match self.stack.pop() {
            Some(Entry::Val(v)) => v,
            Some(Entry::Expanded(vs)) => Value::from_list(&vs),
            None => Value::empty(),
        }
    }

    fn top_val(&self) -> Value {
        match self.stack.last() {
            Some(Entry::Val(v)) => v.clone(),
            Some(Entry::Expanded(vs)) => Value::from_list(vs),
            None => Value::empty(),
        }
    }

    /// Gather the current command's arguments, splicing `{*}` expansions.
    fn collect_args(&self, from: usize) -> Vec<Value> {
        let mut args = Vec::with_capacity(self.stack.len() - from);
        for entry in &self.stack[from..] {
            match entry {
                Entry::Val(v) => args.push(v.clone()),
                Entry::Expanded(vs) => args.extend(vs.iter().cloned()),
            }
        }
        args
    }
}

/// Execute one instruction.  Errors carry the same shape the tree-walk
/// would produce for the same operation; the caller classifies them into
/// errorInfo frames.
fn exec_op(interp: &mut Interp, code: &ByteCode, op: &OpCode, st: &mut VmState) -> Result<()> {
    match op {
        // ── Stack ───────────────────────────────────────────────────────
        OpCode::PushConst(idx) => {
            let s = code.get_const(*idx).unwrap_or("");
            st.stack.push(Entry::Val(Value::from_str(s)));
        }
        OpCode::PushConstWide(idx) => {
            let s = code.get_const_wide(*idx).unwrap_or("");
            st.stack.push(Entry::Val(Value::from_str(s)));
        }
        OpCode::PushEmpty => st.stack.push(Entry::Val(Value::empty())),
        OpCode::PushInt(n) => st.stack.push(Entry::Val(Value::from_int(*n))),
        // float_value renders Tcl-exact (expr-52.x precision rules), not
        // Rust's default float formatting.
        OpCode::PushFloat(f) => {
            st.stack
                .push(Entry::Val(crate::types::expr_funcs::float_value(*f)));
        }
        OpCode::PushTrue => st.stack.push(Entry::Val(Value::from_bool(true))),
        OpCode::PushFalse => st.stack.push(Entry::Val(Value::from_bool(false))),
        OpCode::Pop => {
            st.stack.pop();
        }
        OpCode::Dup => {
            let v = st.top_val();
            st.stack.push(Entry::Val(v));
        }

        // ── Variables ───────────────────────────────────────────────────
        OpCode::LoadVar(idx) => {
            let name = code.get_const(*idx).unwrap_or("");
            // eval_var_ref, not a raw read: `$a($i)` words compile to
            // LoadVar with the raw reference text and need the same
            // index-substitution dance eval_word does.
            let v = interp.eval_var_ref(name)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::StoreVar(idx) | OpCode::StoreVarPop(idx) => {
            let name = code.get_const(*idx).unwrap_or("");
            let keep = matches!(op, OpCode::StoreVar(_));
            let v = if keep {
                st.top_val()
            } else {
                st.pop_val()
            };
            interp.set_var(name, v)?;
        }
        OpCode::IncrVar(idx, amount) => {
            let name = code.get_const(*idx).unwrap_or("");
            // Fast path for the success case only; any failure falls back
            // to the real `incr`, whose error framing (scalar/array
            // conflicts, missing vars) it owns.
            let fast = interp
                .get_var(name)
                .ok()
                .and_then(|v| v.as_int())
                .map(|current| Value::from_int(current + *amount));
            match fast {
                Some(new_val) => {
                    let v = interp.set_var(name, new_val)?;
                    st.stack.push(Entry::Val(v));
                }
                None => {
                    let args = [
                        Value::from_str("incr"),
                        Value::from_str(name),
                        Value::from_int(*amount),
                    ];
                    let v = super::commands::misc::cmd_incr(interp, &args)?;
                    st.stack.push(Entry::Val(v));
                }
            }
        }

        // ── Expansion ───────────────────────────────────────────────────
        OpCode::ExpandMark => {
            st.expand_base = st.stack.len();
        }
        OpCode::ExpandList => {
            let v = st.pop_val();
            match v.as_list() {
                Some(items) => st.stack.push(Entry::Expanded(items)),
                None => st.stack.push(Entry::Expanded(vec![v])),
            }
        }
        OpCode::Concat(n) => {
            let n = *n as usize;
            let from = st.stack.len() - n;
            let mut s = String::new();
            for e in &st.stack[from..] {
                match e {
                    Entry::Val(v) => s.push_str(v.as_str()),
                    Entry::Expanded(vs) => {
                        for v in vs {
                            s.push_str(v.as_str());
                        }
                    }
                }
            }
            st.stack.truncate(from);
            st.stack.push(Entry::Val(Value::from_str(&s)));
        }

        // ── Control flow ────────────────────────────────────────────────
        OpCode::Jump(t) => {
            // Every inline body ends with a plain Jump — close its region.
            st.bodies.pop();
            st.pc = *t as usize;
        }
        OpCode::JumpTrue(t) => {
            let v = st.pop_val();
            if crate::types::expr_funcs::strict_bool(&v)? {
                st.pc = *t as usize;
            } else {
                // Not taken → entering an inline body of the current
                // command's construct.
                st.bodies.push(st.cur_site);
            }
        }
        OpCode::JumpFalse(t) => {
            let v = st.pop_val();
            if !crate::types::expr_funcs::strict_bool(&v)? {
                st.pc = *t as usize;
            } else {
                st.bodies.push(st.cur_site);
            }
        }

        // ── Loops ───────────────────────────────────────────────────────
        OpCode::LoopEnter { cont, brk } => {
            st.loops.push(LoopFrame {
                cont: *cont,
                brk: *brk,
                bodies_len: st.bodies.len(),
                stack_len: st.stack.len(),
            });
        }
        OpCode::LoopExit => {
            st.loops.pop();
        }
        OpCode::Break => match st.loops.last() {
            Some(l) => {
                st.bodies.truncate(l.bodies_len);
                st.stack.truncate(l.stack_len);
                st.pc = l.brk as usize;
            }
            None => return Err(Error::brk()),
        },
        OpCode::Continue => match st.loops.last() {
            Some(l) => {
                st.bodies.truncate(l.bodies_len);
                st.stack.truncate(l.stack_len);
                st.pc = l.cont as usize;
            }
            None => return Err(Error::cont()),
        },

        // ── Return / exit ───────────────────────────────────────────────
        OpCode::Return => {
            // A `return` is the *completion*, not the value: catch reports
            // code 2, subst's bracket walker matches the ControlFlow, and
            // `call_proc` decodes it — so raise exactly what cmd_return
            // raises for a plain return.
            let v = st.pop_val();
            return Err(Error::ret(Some(v)));
        }
        OpCode::Exit(c) => {
            return Err(Error::exit(Some(*c)));
        }

        // ── Commands ────────────────────────────────────────────────────
        OpCode::BeginCmd(site_idx) => {
            if interp.call_depth > interp.max_call_depth {
                return Err(Error::runtime(
                    "maximum recursion depth exceeded",
                    crate::error::ErrorCode::StackOverflow,
                ));
            }
            let site = &code.sites[*site_idx as usize];
            interp.cur_cmd_text = site.text;
            interp.cur_cmd_line = site.line_rel as usize;
            interp.cur_cmd_word_srcs = Rc::clone(&site.word_srcs);
            interp.line_offset = st.entry_offset + site.line_delta as usize;
            st.cur_site = *site_idx as usize;
        }
        OpCode::BodyMark => {
            st.bodies.push(st.cur_site);
        }
        OpCode::Call { cmd_id, argc } => {
            let from = st.stack.len() - *argc as usize;
            let args = st.collect_args(from);
            st.stack.truncate(from);
            let v = dispatch_call(interp, args, *cmd_id)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::CallExpand { cmd_id, .. } => {
            let args = st.collect_args(st.expand_base);
            st.stack.truncate(st.expand_base);
            let v = dispatch_call(interp, args, *cmd_id)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::DynCall { argc } => {
            let from = st.stack.len() - *argc as usize;
            let args = st.collect_args(from);
            st.stack.truncate(from);
            let v = dispatch_dynamic(interp, args)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::DynCallExpand { .. } => {
            let args = st.collect_args(st.expand_base);
            st.stack.truncate(st.expand_base);
            let v = dispatch_dynamic(interp, args)?;
            st.stack.push(Entry::Val(v));
        }

        // ── Evaluation ──────────────────────────────────────────────────
        OpCode::EvalScript => {
            let script = st.pop_val();
            match interp.eval(script.as_str()) {
                Ok(v) => st.stack.push(Entry::Val(v)),
                Err(e) => {
                    // Word context: the nested eval logged its own frames;
                    // the enclosing command's frame defers to the top
                    // level (eval_word's CommandSub arm).
                    interp.err_from_subst = true;
                    return Err(e);
                }
            }
        }
        OpCode::EvalExpr => {
            let expr = st.pop_val();
            let v = interp.eval_expr(expr.as_str())?;
            st.stack.push(Entry::Val(v));
        }

        // ── Arithmetic / comparison (expr_ops = the expr parser's own
        //    semantics — overflow widening, floor division, exact error
        //    text) ──────────────────────────────────────────────────────
        OpCode::Add => arith(st, '+')?,
        OpCode::Sub => arith(st, '-')?,
        OpCode::Mul => arith(st, '*')?,
        OpCode::Div => arith(st, '/')?,
        OpCode::Mod => {
            let b = st.pop_val();
            let a = st.pop_val();
            let v = crate::types::expr_ops::int_mod(&a, &b)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::Pow => {
            let b = st.pop_val();
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_pow(a, b)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::Neg => {
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_neg(&a)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::Not => {
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_not(&a)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::BitNot => {
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_bitnot(&a)?;
            st.stack.push(Entry::Val(v));
        }
        OpCode::Eq => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(Entry::Val(crate::types::expr_ops::op_eq(&a, &b)));
        }
        OpCode::Ne => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(Entry::Val(crate::types::expr_ops::op_ne(&a, &b)));
        }
        OpCode::Lt => rel(st, "<")?,
        OpCode::Gt => rel(st, ">")?,
        OpCode::Le => rel(st, "<=")?,
        OpCode::Ge => rel(st, ">=")?,
        OpCode::StrEq => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(Entry::Val(crate::types::expr_ops::op_str_eq(&a, &b)));
        }
        OpCode::StrNe => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(Entry::Val(crate::types::expr_ops::op_str_ne(&a, &b)));
        }
        OpCode::BitAnd => bits(st, '&')?,
        OpCode::BitOr => bits(st, '|')?,
        OpCode::BitXor => bits(st, '^')?,
        OpCode::Shl => shift(st, true)?,
        OpCode::Shr => shift(st, false)?,

        // ── Meta ────────────────────────────────────────────────────────
        OpCode::Nop | OpCode::Line(_) => {}
        other => {
            // The compiler's fallback gate excludes everything else; a
            // unit reaching here is a compiler/executor contract bug, not
            // a runtime condition.
            let _ = other;
            return Err(Error::runtime(
                "bytecode op not supported by the interpreter executor",
                crate::error::ErrorCode::Generic,
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch helpers
// ---------------------------------------------------------------------------

/// `Call` — a known built-in.  With no procs/aliases/ensembles registered
/// and the interp at the global namespace, the namespace-resolution chains
/// in `dispatch_values` can only ever land on the very function the CmdId
/// names, so call it directly; anything else (or when it wouldn't) takes
/// the exact full dispatch.  (Function-pointer identity is not compared —
/// identical-code-folding can merge distinct fns to one address — the
/// invoked name must still be the canonical builtin the id names, so a
/// `rename`d command keeps the full dispatch's lookup semantics.)
fn dispatch_call(interp: &mut Interp, args: Vec<Value>, cmd_id: u16) -> Result<Value> {
    if interp.exec_traces.is_empty()
        && interp.current_namespace == "::"
        && interp.procs.is_empty()
        && interp.aliases.is_empty()
        && interp.ensembles.is_empty()
        && interp.import_aliases.is_empty()
    {
        let name = args[0].as_str();
        if rtcl_parser::CmdId::from_name(name).map(|c| c as u16) == Some(cmd_id) {
            let f = interp.commands.get(name).copied();
            if let Some(f) = f {
                interp.call_depth += 1;
                let r = f(interp, &args);
                interp.call_depth -= 1;
                let name = args[0].as_str().to_string();
                return interp.fill_wrong_args(&name, r);
            }
        }
    }
    dispatch_dynamic(interp, args)
}

/// Full dispatch — `dispatch_values` with the execution-step trace
/// bracketing `eval_command` provides.
fn dispatch_dynamic(interp: &mut Interp, args: Vec<Value>) -> Result<Value> {
    let step = interp.exec_step_begin(&args);
    let r = interp.dispatch_values(&args);
    if let Some(ctx) = step {
        match &r {
            Ok(v) => {
                let res = v.as_str().to_string();
                interp.exec_step_end(&ctx, "0", &res);
            }
            Err(e) if interp.err_is_error(e) => {
                let msg = e.message_text().to_string();
                interp.exec_step_end(&ctx, "1", &msg);
            }
            _ => {}
        }
    }
    r
}

// ---------------------------------------------------------------------------
// Arithmetic helpers
// ---------------------------------------------------------------------------

fn pop_pair(st: &mut VmState) -> (Value, Value) {
    let b = st.pop_val();
    let a = st.pop_val();
    (a, b)
}

fn arith(st: &mut VmState, op: char) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::numeric_binop(&a, &b, op)?;
    st.stack.push(Entry::Val(v));
    Ok(())
}

fn rel(st: &mut VmState, op: &'static str) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::op_rel(&a, &b, op);
    st.stack.push(Entry::Val(v));
    Ok(())
}

fn bits(st: &mut VmState, op: char) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::int_bitop(&a, &b, op)?;
    st.stack.push(Entry::Val(v));
    Ok(())
}

fn shift(st: &mut VmState, shl: bool) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::int_shift(&a, &b, shl)?;
    st.stack.push(Entry::Val(v));
    Ok(())
}
