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

use super::{CmdSites, Interp, Rc};
use crate::error::{Error, Result};
use crate::value::Value;
use rtcl_parser::{ByteCode, Compiler, OpCode};

#[cfg(not(feature = "embedded"))]
use std::sync::OnceLock;

/// Compile a proc body once, at definition time.  `None` when the body
/// does not parse — the tree-walk reproduces the per-call parse error
/// exactly (`seed_parse_error` runs on every uncached `eval`).
/// `params` seed the compiled locals table (slot i = the i-th name), the
/// contract the frame-side positional binding relies on.
pub(crate) fn compile_proc_body(
    params: &[(String, Option<String>)],
    body: &str,
    epoch: u64,
) -> Option<Rc<ByteCode>> {
    let unit = rtcl_parser::ScriptUnit::parse(body).ok()?;
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let mut code =
        Compiler::compile_unit_locals(Rc::clone(&unit.source), &unit.commands, &names);
    code.epoch = epoch;
    Some(Rc::new(code))
}

/// Bump [`Interp::tier1_epoch`] when `key`'s leaf name matches one of the
/// inline-folded Tier1 commands.  Only these names can be shadowed out
/// from under a compiled body — everything else in the bytecode goes
/// through dispatch (`Call`/`DynCall`) and re-resolves per call.
pub(crate) fn note_tier1_mutation(interp: &mut Interp, key: &str) {
    let leaf = key.rsplit("::").next().unwrap_or(key);
    // KEEP IN SYNC with the compiler's Tier-1 fold list
    // (`rtcl-parser/src/compiler.rs`, the `match` on folded command
    // names): every name the compiler inline-folds must bump the epoch
    // here, or a redefinition leaves stale folds in compiled bodies
    // (`lappend` was the one gap — `compile_lappend` folds it).
    if matches!(
        leaf,
        "set" | "if" | "while" | "for" | "expr" | "incr" | "return" | "exit" | "break"
            | "continue" | "foreach" | "lmap" | "lappend"
    ) {
        interp.tier1_epoch += 1;
    }
}

/// An unconditional epoch bump for a coarse mutation (namespace / object
/// tree teardown) whose affected command names aren't worth enumerating.
pub(crate) fn note_tier1_sweep(interp: &mut Interp) {
    interp.tier1_epoch += 1;
}

/// Programmatic engine switch (the CLI's `--no-bytecode`, embedders'
/// escape hatch): same effect as the `RTCL_NO_BYTECODE` env var without
/// setting a process env var — the env var materialises into `$env(*)`
/// at startup and shows up in every variable enumeration, which
/// differential probes compare byte-exactly.  Call before the first
/// `Interp::new` (stdlib loading already consults it).
pub fn set_bytecode_disabled(disabled: bool) {
    BYTECODE_FORCED_OFF.store(disabled, core::sync::atomic::Ordering::Relaxed);
}

static BYTECODE_FORCED_OFF: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// May the current call run its body through the bytecode executor?
/// Execution traces observe every dispatched command, so they pin the
/// proc to the tree-walk; `RTCL_NO_BYTECODE` is the manual escape hatch.
pub(crate) fn bytecode_applicable(interp: &Interp) -> bool {
    if !interp.exec_traces.is_empty() {
        return false;
    }
    if BYTECODE_FORCED_OFF.load(core::sync::atomic::Ordering::Relaxed) {
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
    /// Site of the looping construct (`while`/`for`) — a `break`/`continue`
    /// truncates the body regions without their closing jumps, so the
    /// executor must re-attribute to the loop itself: the ops at the jump
    /// target (a re-checked condition, the next script) belong to it.
    owner: usize,
}

/// One active inline foreach/lmap (`ForeachStart` … `ForeachEnd`): the
/// strict-parsed data lists and the current iteration index; `collected`
/// is `Some` for an lmap (body results so far — kept across a `break`,
/// skipped by a `continue`).
struct ForeachFrame {
    lists: Vec<ForeachList>,
    idx: usize,
    /// Iteration count, fixed at start (max over groups) — ForeachNext
    /// used to recompute this per iteration over every group's list.
    iters: usize,
    collected: Option<Vec<Value>>,
    /// FRAMED unit (compiled outside a proc body): body errors gain the
    /// `("foreach"/"lmap" body line N)` exit frame and loop-var write
    /// failures gain the `(setting ... loop variable)` decoration — the
    /// dispatched foreach's tclsh shape at top level.
    framed: bool,
    lmap: bool,
}

/// One foreach varlist: borrowed from a cached list rep, or owned.
///
/// A value that already carries a list internal rep is held by reference
/// (one Rc bump — tclsh's foreach keeps a refcount on the list object and
/// reads elements in place, and so does the tree engine's `strict_list_cow`
/// fast path).  Lists materialised from a string/dict rep are owned
/// vectors, exactly what `strict_list` produced before.  A mid-loop
/// mutation of the source (`lappend` on the iterated variable) copies
/// under COW, so iteration proceeds over the start snapshot in both
/// forms — matching tclsh, whose shared list is duplicated on first
/// write.
enum ForeachList {
    Rep(Value),
    Owned(Vec<Value>),
}

impl ForeachList {
    fn len(&self) -> usize {
        match self {
            ForeachList::Rep(v) => v.as_list_ref().map_or(0, <[Value]>::len),
            ForeachList::Owned(v) => v.len(),
        }
    }

    fn get(&self, i: usize) -> Option<&Value> {
        match self {
            ForeachList::Rep(v) => v.as_list_ref().and_then(|s| s.get(i)),
            ForeachList::Owned(v) => v.get(i),
        }
    }
}

/// One inline body region currently executing — which command's construct
/// owns it (`BeginCmd` site index), and for a `for` loop's next script
/// that it IS the next region: a `continue` signal raised there escapes
/// the loop (for-8.2).
///
/// `Sub` marks an inlined `[...]` word: not a body (errors inside it are
/// NOT frameless) — it records which command's word the bracket belongs
/// to, so an error crossing it defers that command's frame to the top
/// level (the tree-walk's `eval_word` CommandSub boundary).
///
/// `Expr` marks an inlined `[...]` *operand of a compiled expression*
/// (`expr {[fib $n] + 1}`): eval_expr's bracket runs a plain nested eval
/// that sets no `err_from_subst`, so an error crossing it makes the
/// expression-owning command **append** its harness frame instead of
/// deferring.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Region {
    Body(usize),
    /// A foreach body region in a FRAMED unit (compiled outside a proc
    /// body): body errors crossing it gain the `("foreach" body line N)`
    /// exit frame — the dispatched foreach's shape, which tclsh shows at
    /// top level.  The bool is the lmap flag (the frame text differs).
    BodyFramed(usize, bool),
    Next(usize),
    Sub(usize),
    Expr(usize),
}

impl Region {
    fn site(self) -> usize {
        match self {
            Region::Body(s) | Region::BodyFramed(s, _) | Region::Next(s) | Region::Sub(s)
            | Region::Expr(s) => s,
        }
    }
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
            // A continue raised inside this loop's NEXT region escapes the
            // loop entirely (for-8.2: the compiled next script has no
            // in-loop continue target, so `eval continue` there unwinds
            // the `for` too).  A break from anywhere in the loop — body
            // or next — ends it (for-8.1).
            if !is_brk
                && st.bodies[l.bodies_len..]
                    .iter()
                    .any(|r| matches!(r, Region::Next(_)))
            {
                st.bodies.truncate(l.bodies_len);
                let stack_len = l.stack_len;
                st.stack.truncate(stack_len);
                st.loops.pop();
                // The escaped signal surfaces where the `for` command
                // itself sits (for-8.10: an inner for's next script runs
                // `eval continue`; the OUTER loop continues from its own
                // step).  Hand it to the enclosing inline loop — unless
                // the for sat in *that* loop's next region, which has no
                // continue target either: escape it too, and so on until
                // a body region or the unit boundary.
                loop {
                    match st.loops.last() {
                        Some(enc) => {
                            let in_next = st.bodies[enc.bodies_len..]
                                .iter()
                                .any(|r| matches!(r, Region::Next(_)));
                            let (bodies_len, stack_len, cont, owner) =
                                (enc.bodies_len, enc.stack_len, enc.cont, enc.owner);
                            st.bodies.truncate(bodies_len);
                            st.stack.truncate(stack_len);
                            if in_next {
                                st.loops.pop();
                                continue;
                            }
                            st.cur_site = owner;
                            st.pc = cont as usize;
                            return Ok(());
                        }
                        None => return Err(e),
                    }
                }
            }
            st.bodies.truncate(l.bodies_len);
            st.stack.truncate(l.stack_len);
            st.cur_site = l.owner;
            st.pc = if is_brk { l.brk as usize } else { l.cont as usize };
            Ok(())
        }
        None => Err(e),
    }
}


/// Deferred/appended frames for the inlined `[...]` regions an error
/// crosses on its way out of the unit: pop the region stack — a word
/// bracket (`Sub`) defers its owning command's frame to the top level
/// (the tree-walk's `eval_word` CommandSub boundary, outermost deferral
/// winning); an expression operand (`Expr`) APPENDS its owning command's
/// harness frame (eval_expr's bracket is a plain nested eval that marks
/// no `err_from_subst` — the command's frame is logged; err_harness_
/// frame's fresh-suppression branch reproduces the transparent-body
/// case, `expr {[if {1} {error x}]}`).  Body/next regions crossed add
/// nothing (their constructs log no frames of their own, the err_fresh
/// suppression).
fn unwind_subs(interp: &mut Interp, code: &ByteCode, st: &mut VmState, e: &Error) {
    while let Some(region) = st.bodies.last().copied() {
        st.bodies.pop();
        match region {
            Region::Sub(site_idx) => {
                let site = &code.sites[site_idx];
                let text = site.text.slice(&code.source);
                interp.err_pending_top = Some(Rc::from(text));
                interp.err_line = st.entry_offset + site.line as usize;
            }
            Region::Expr(site_idx) => {
                let site = &code.sites[site_idx];
                let text = site.text.slice(&code.source);
                let line = st.entry_offset + site.line as usize;
                let msg = e.message_text();
                interp.err_harness_frame(&msg, text, line);
            }
            Region::Body(_) | Region::Next(_) => {}
            Region::BodyFramed(site_idx, lmap) => {
                let site = &code.sites[site_idx];
                let text = site.text.slice(&code.source);
                let line = st.entry_offset + site.line as usize;
                let tag = if lmap { "\"lmap\" body" } else { "\"foreach\" body" };
                interp.err_exit_frame(tag);
                // The foreach command's own harness frame follows
                // immediately (`invoked from within`) — tclsh logs it
                // before the error escapes the foreach command.
                interp.err_harness_frame(e.message_text().as_str(), text, line);
            }
        }
    }
}

/// Process-level JIT hook for proc bodies (M2 seam).
///
/// [`Interp::call_proc`]'s Site A consults this after its own gates
/// (`!fallback`, epoch match, `bytecode_applicable`) pass: `Some(r)`
/// replaces the `exec_bytecode` result (ret decoding, `(procedure …)`
/// frames and break→error conversion all stay in call_proc); `None`
/// keeps the bytecode executor.  A fn pointer — no closure state, the
/// engine registers once at process start (`set_proc_body_executor`).
pub type ProcBodyExecutor =
    fn(&mut Interp, &Rc<ByteCode>) -> Option<crate::error::Result<Value>>;

static PROC_BODY_EXECUTOR: std::sync::OnceLock<Option<ProcBodyExecutor>> =
    std::sync::OnceLock::new();

/// Register the proc-body JIT hook.  First call wins (`OnceLock` — the
/// engine installs at process start; re-registration is a no-op and
/// `None` cannot unregister by design).  With no hook installed the
/// interpreter behaves exactly as before (zero-change default).
pub fn set_proc_body_executor(executor: Option<ProcBodyExecutor>) {
    let _ = PROC_BODY_EXECUTOR.set(executor);
}

/// Site A hook probe: returns the JIT executor's result when one is
/// installed and it accepts this body, `None` to run `exec_bytecode`.
pub(crate) fn proc_body_executor(
    interp: &mut Interp,
    code: &Rc<ByteCode>,
) -> Option<crate::error::Result<Value>> {
    let f = (*PROC_BODY_EXECUTOR.get().unwrap_or(&None))?;
    f(interp, code)
}

/// Execute a proc body's compiled bytecode on `interp`.
///
/// Callers gate on [`bytecode_applicable`] and `!code.fallback`; errors
/// return identically shaped [`Error`]s to the tree-walk so `call_proc`'s
/// boundary handling (`(procedure …)` frames, return decoding, break →
/// error conversion) needs no changes.
pub(crate) fn exec_bytecode(interp: &mut Interp, code: &Rc<ByteCode>) -> Result<Value> {
    // A compiled unit's DynCall'd foreach runs DISPATCHED (tclsh's
    // INST_CALL fallback): clear the lexical flag for the unit's
    // duration, restoring the enclosing unit's context afterwards.
    let saved_lexical = interp.lexical_body;
    interp.lexical_body = false;
    // Proc-context signal (tclsh's compiledLocals): a locals-mode unit
    // is where the compiler inlines braced catch bodies, so DynCall'd
    // commands inside it must see the proc context.  Non-locals units
    // (eval'd texts) clear it — their contents are fresh non-proc
    // contexts exactly like a tree-walked eval.
    let saved_inl = interp.in_locals_unit;
    interp.in_locals_unit = code.locals_mode;
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
    let saved_srcs =
        std::mem::replace(&mut interp.cur_cmd_word_srcs, super::util::WordSrcs::empty());
    // The unit "acts at" this offset: nested evals (EvalScript →
    // `Interp::eval`) must see the same base the tree-walk would have
    // left installed (e.g. a `while` body rebased by cmd_while).
    interp.line_offset = entry_offset;

    // The executor state comes from the interp's pool: a proc call pays no
    // fresh allocations for the stack/loops/bodies/scratch Vecs (the pool
    // keeps their capacity).  Depth is naturally bounded by recursion —
    // a live VmState is never in the pool, so pooling cannot alias.
    let r = {
        let mut st = interp.vm_pool.take(entry_offset);
        // Same-unit fast path: the state remembers which unit's literal
        // slice + call-site table it holds (recursion re-borrows its own
        // state).  A different unit (or a fresh state) falls to the pool
        // probe — which also lazily heals eviction (see below).
        let same = st
            .consts_code
            .as_ref()
            .map(|c| Rc::ptr_eq(c, code))
            .unwrap_or(false);
        if !same {
            // The literal pool is capped and cleared on overflow: a unit
            // whose entry was evicted used to stay evicted forever —
            // every PushConst re-materialised its literals and every
            // dispatch took the full resolution chain (the dloop
            // profile: ~35% dispatch layer).  The caller always holds
            // the `Rc`, so a miss re-inserts lazily.
            let (consts, cmd_sites) = match interp.const_pool_get(code) {
                Some((v, s)) => (Some(v), Some(s)),
                None => {
                    interp.const_pool_insert(code);
                    interp
                        .const_pool_get(code)
                        .map(|(v, s)| (Some(v), Some(s)))
                        .unwrap_or((None, None))
                }
            };
            st.consts = consts;
            st.cmd_sites = cmd_sites;
            st.consts_code = Some(Rc::clone(code));
        }
        let r = exec_inner(interp, code, &mut st);
        interp.vm_pool.give(st);
        r
    };

    interp.cur_source = saved_source;
    interp.cur_cmd_text = saved_text;
    interp.cur_cmd_line = saved_line;
    interp.cur_cmd_word_srcs = saved_srcs;
    interp.line_offset = entry_offset;
    interp.lexical_body = saved_lexical;
    interp.in_locals_unit = saved_inl;
    r
}

fn exec_inner(interp: &mut Interp, code: &ByteCode, st: &mut VmState) -> Result<Value> {
    let ops = code.ops();

    let result = loop {
        // Stream end: the tree-walk's result is the last command's, which
        // the stack discipline leaves on top (`set`/`incr` fast paths and
        // dispatches all push their result).
        // One bounds check per op, then an unchecked load: the `get`
        // Option cost sits on every dispatch of every loop (the profile's
        // `get<OpCode>` ~4% of the build loop).
        if st.pc >= ops.len() {
            break st.pop_val();
        }
        // SAFETY: the check above proves `st.pc` in-bounds.
        let op = unsafe { ops.get_unchecked(st.pc) };
        st.pc += 1;
        // ── Hot lane ────────────────────────────────────────────────────
        // The ops a compiled loop body is made of run inline: the
        // `exec_op` call + its second dispatch was the exec-side profile's
        // largest single item on the loop benches.  Each arm runs the same
        // shared helper `exec_op` runs for that variant — or, for the
        // one-liners, its verbatim body — so the two dispatch paths stay
        // one semantic source; every other op, and every fallback inside a
        // helper, funnels through `exec_op` exactly as before.
        let r: Result<()> = match op {
            // ── Stack ───────────────────────────────────────────────────
            OpCode::PushConst(idx) => {
                op_push_const(st, code, *idx);
                Ok(())
            }
            OpCode::PushConstWide(idx) => {
                op_push_const_wide(st, code, *idx);
                Ok(())
            }
            OpCode::PushEmpty => {
                st.stack.push(Value::empty());
                Ok(())
            }
            OpCode::PushInt(n) => {
                st.stack.push(Value::from_int(*n));
                Ok(())
            }
            OpCode::PushTrue => {
                st.stack.push(Value::from_bool(true));
                Ok(())
            }
            OpCode::PushFalse => {
                st.stack.push(Value::from_bool(false));
                Ok(())
            }
            OpCode::Pop => {
                st.stack.pop();
                Ok(())
            }
            OpCode::Dup => {
                let v = st.top_val();
                st.stack.push(v);
                Ok(())
            }

            // ── Variables ───────────────────────────────────────────────
            OpCode::LoadVar(idx) => op_load_var(interp, code, st, *idx),
            OpCode::StoreVar(idx) => op_store_var(interp, code, st, *idx, true),
            OpCode::StoreVarPop(idx) => op_store_var(interp, code, st, *idx, false),
            OpCode::IncrVar(idx, amount) => op_incr_var(interp, code, st, *idx, *amount),
            OpCode::LoadLocal(slot) => op_load_local(interp, code, st, *slot),
            OpCode::StoreLocal(slot) => op_store_local(interp, code, st, *slot),
            OpCode::StoreLocalPop(slot) => op_store_local_pop(interp, code, st, *slot),
            OpCode::IncrLocal(slot, amount) => op_incr_local(interp, code, st, *slot, *amount),

            // ── Slot superinstructions ──────────────────────────────────
            OpCode::AddSlotSlot(a, b) => op_add_slot_slot(interp, code, st, *a, *b),
            OpCode::AddSlotImm(a, k) => op_add_slot_imm(interp, code, st, *a, *k),
            OpCode::CmpSlotSlot(a, b, cc) => op_cmp_slot_slot(interp, code, st, *a, *b, *cc),
            OpCode::CmpSlotImm(a, k, cc) => op_cmp_slot_imm(interp, code, st, *a, *k, *cc),
            OpCode::AddVarVar(a, b) => op_add_var_var(interp, code, st, *a, *b),
            OpCode::AddVarImm(a, k) => op_add_var_imm(interp, code, st, *a, *k),
            OpCode::CmpVarVar(a, b, cc) => op_cmp_var_var(interp, code, st, *a, *b, *cc),
            OpCode::CmpVarImm(a, k, cc) => op_cmp_var_imm(interp, code, st, *a, *k, *cc),

            // ── Fused slot pairs (the `fuse_slot_pairs` windows) ────────
            // One dispatch covers the producer and its consumer; the
            // helpers are the same ones `exec_op` routes to.
            OpCode::CmpSlotSlotJumpTrue(a, b, cc, t) => {
                op_cmp_ss_jump(interp, code, st, *a, *b, *cc, *t, true)
            }
            OpCode::CmpSlotSlotJumpFalse(a, b, cc, t) => {
                op_cmp_ss_jump(interp, code, st, *a, *b, *cc, *t, false)
            }
            OpCode::CmpSlotImmJumpTrue(a, k, cc, t) => {
                op_cmp_si_jump(interp, code, st, *a, *k, *cc, *t, true)
            }
            OpCode::CmpSlotImmJumpFalse(a, k, cc, t) => {
                op_cmp_si_jump(interp, code, st, *a, *k, *cc, *t, false)
            }
            OpCode::CmpVarVarJumpTrue(a, b, cc, t) => {
                op_cmp_vv_jump(interp, code, st, *a, *b, *cc, *t, true)
            }
            OpCode::CmpVarVarJumpFalse(a, b, cc, t) => {
                op_cmp_vv_jump(interp, code, st, *a, *b, *cc, *t, false)
            }
            OpCode::CmpVarImmJumpTrue(a, k, cc, t) => {
                op_cmp_vi_jump(interp, code, st, *a, *k, *cc, *t, true)
            }
            OpCode::CmpVarImmJumpFalse(a, k, cc, t) => {
                op_cmp_vi_jump(interp, code, st, *a, *k, *cc, *t, false)
            }
            OpCode::LappendSlotSlot(src, dst) => op_lappend_slot_slot(interp, code, *src, *dst),
            OpCode::LappendVarVar(src, dst) => op_lappend_var_var(interp, code, *src, *dst),
            OpCode::IncrVarPop(idx, amount) => op_incr_var_pop(interp, code, *idx, *amount),
            OpCode::BeginCmdSubMark(site) => op_begin_cmd_sub_mark(interp, code, st, *site),
            OpCode::ExprMarkBeginCmd(site) => op_expr_mark_begin_cmd(interp, code, st, *site),
            OpCode::BeginCmdPushConst(site, idx) => {
                op_begin_cmd_push_const(interp, code, st, *site, *idx)
            }
            OpCode::SubEndStoreLocalPop(slot) => {
                op_sub_end_store_local_pop(interp, code, st, *slot)
            }
            OpCode::SubEndStoreVarPop(idx) => op_sub_end_store_var_pop(interp, code, st, *idx),
            OpCode::SubEndReturn => op_sub_end_return(st),
            OpCode::SwapToNextMark => {
                op_swap_to_next_mark(st);
                Ok(())
            }
            OpCode::ForNextIncrLocal(site, slot, amount, t) => {
                op_for_next_incr_local(interp, code, st, *site, *slot, *amount, *t)
            }
            OpCode::ForNextIncrVar(site, idx, amount, t) => {
                op_for_next_incr_var(interp, code, st, *site, *idx, *amount, *t)
            }
            OpCode::NextMarkBeginCmd(site) => {
                op_next_mark_begin_cmd(interp, code, st, *site)
            }
            OpCode::BeginCmdIncrLocalPop(site, slot, amount) => {
                op_begin_cmd_incr_local_pop(interp, code, st, *site, *slot, *amount)
            }
            OpCode::BeginCmdIncrVarPop(site, idx, amount) => {
                op_begin_cmd_incr_var_pop(interp, code, st, *site, *idx, *amount)
            }
            OpCode::ForeachNextClose(info, body) => {
                op_foreach_next_close(interp, code, st, *info, *body)
            }
            OpCode::SlotSetAddSS { set_site, expr_site, a, b, dst } => op_slot_set_add_ss(
                interp, code, st, *set_site, *expr_site, *a, *b, *dst,
            ),
            OpCode::VarSetAddVV { set_site, expr_site, a, b, dst } => op_var_set_add_vv(
                interp, code, st, *set_site, *expr_site, *a, *b, *dst,
            ),
            // fib's per-call windows (×2/call) stay in the hot lane; the
            // rest of the fib family rides `exec_op` (keeping the hot-lane
            // match small measured faster for the fe/build loops).
            OpCode::PushConstSubMark(idx) => {
                op_push_const_sub_mark(st, code, *idx);
                Ok(())
            }
            OpCode::SubSlotImm(a, k) => op_sub_slot_imm(interp, code, st, *a, *k),
            OpCode::SubEndDynCall(argc) => op_sub_end_dyn_call(interp, st, *argc),
            OpCode::IncrLocalPop(slot, amount) => {
                let _ = incr_local_value(interp, code, *slot, *amount)?;
                Ok(())
            }

            // ── Arithmetic / comparison ─────────────────────────────────
            OpCode::Add => arith(st, '+'),
            OpCode::Sub => arith(st, '-'),
            OpCode::Mul => arith(st, '*'),
            OpCode::Div => arith(st, '/'),
            OpCode::Mod => op_int_mod(st),
            OpCode::Neg => {
                let a = st.pop_val();
                let v = crate::types::expr_ops::op_neg(&a)?;
                st.stack.push(v);
                Ok(())
            }
            OpCode::Eq => {
                let (a, b) = pop_pair(st);
                st.stack.push(crate::types::expr_ops::op_eq(&a, &b));
                Ok(())
            }
            OpCode::Ne => {
                let (a, b) = pop_pair(st);
                st.stack.push(crate::types::expr_ops::op_ne(&a, &b));
                Ok(())
            }
            OpCode::Lt => rel(st, "<"),
            OpCode::Gt => rel(st, ">"),
            OpCode::Le => rel(st, "<="),
            OpCode::Ge => rel(st, ">="),
            OpCode::StrEq => {
                let (a, b) = pop_pair(st);
                st.stack.push(crate::types::expr_ops::op_str_eq(&a, &b));
                Ok(())
            }
            OpCode::StrNe => {
                let (a, b) = pop_pair(st);
                st.stack.push(crate::types::expr_ops::op_str_ne(&a, &b));
                Ok(())
            }
            OpCode::Concat(n) => {
                op_concat(st, *n);
                Ok(())
            }

            // ── Control flow ────────────────────────────────────────────
            OpCode::Jump(t) => op_jump(interp, st, *t),
            OpCode::JumpTrue(t) => op_jump_if(interp, st, *t, true),
            OpCode::JumpFalse(t) => op_jump_if(interp, st, *t, false),

            // ── Loops / regions ─────────────────────────────────────────
            OpCode::LoopEnter { cont, brk } => {
                op_loop_enter(st, *cont, *brk);
                Ok(())
            }
            OpCode::LoopExit => {
                st.loops.pop();
                Ok(())
            }
            OpCode::BodyMark => {
                st.bodies.push(Region::Body(st.cur_site));
                Ok(())
            }
            OpCode::BodyMarkFramed => {
                op_body_mark_framed(st);
                Ok(())
            }
            OpCode::NextMark => {
                st.bodies.push(Region::Next(st.cur_site));
                Ok(())
            }
            OpCode::SubMark => {
                st.bodies.push(Region::Sub(st.cur_site));
                Ok(())
            }
            OpCode::ExprMark => {
                st.bodies.push(Region::Expr(st.cur_site));
                Ok(())
            }
            OpCode::SubEnd => {
                op_sub_end(st);
                Ok(())
            }
            OpCode::BeginCmd(site_idx) => op_begin_cmd(interp, code, st, *site_idx),

            // ── Inline foreach / break / continue ───────────────────────
            OpCode::ForeachStart { info, next, end } => {
                op_foreach_start(interp, code, st, *info, *next, *end)
            }
            OpCode::ForeachNext { info, body } => op_foreach_next(interp, code, st, *info, *body),
            OpCode::ForeachCollect => {
                op_foreach_collect(st);
                Ok(())
            }
            OpCode::ForeachEnd => {
                op_foreach_end(st);
                Ok(())
            }
            OpCode::Break => op_break(interp, st),
            OpCode::Continue => op_continue(interp, st),

            // ── Return / exit / meta ────────────────────────────────────
            // (Control-flow completions: the shared error handler below
            // propagates them framelessly, unchanged.)
            OpCode::Return => {
                let v = st.pop_val();
                Err(Error::ret(Some(v)))
            }
            OpCode::Exit(c) => Err(Error::exit(Some(*c))),
            OpCode::Nop | OpCode::Line(_) => Ok(()),

            // Everything else takes the table dispatch.
            _ => exec_op(interp, code, op, st),
        };
        if let Err(e) = r {
            // Control-flow completions propagate framelessly — except
            // break/continue that crossed a *dispatched command* or
            // *nested script* boundary inside an inline loop: exactly
            // what cmd_while/cmd_for catch from their body evals.
            // Level 1 belongs to this loop (jump); deeper levels
            // propagate decremented (loops.rs parity).
            if !interp.err_is_error(&e) && (e.is_break() || e.is_continue()) {
                match loop_signal(st, e) {
                    Ok(()) => continue,
                    Err(e) => return Err(e),
                }
            }
            if !interp.err_is_error(&e) {
                return Err(e);
            }
            // A unit whose FIRST `BeginCmd` failed the recursion guard
            // never opened a command: no site is current and no region is
            // open, so there is no frame to append — the error propagates
            // frameless to the call boundary (call_proc logs the procedure
            // frame, the caller's dispatch appends its harness frame),
            // exactly the tree-walk's eval_command-guard sequence.
            if st.cur_site == usize::MAX {
                return Err(e);
            }
            // A body error of the current command's own inline
            // construct propagates frameless (tree-walk: the
            // construct's err_fresh suppression).  A `for` next script
            // is one of those bodies too.
            if st
                .bodies
                .last()
                .is_some_and(|r| r.site() == st.cur_site)
            {
                unwind_subs(interp, code, st, &e);
                return Err(e);
            }
            // Everything else appends this command's harness frame —
            // word-op failures through the substitution-aware branch,
            // dispatch/expr failures through the plain one.
            let site = &code.sites[st.cur_site];
            let text = site.text.slice(&code.source);
            let line = st.entry_offset + site.line as usize;
            let is_word_op =
                matches!(op, OpCode::LoadVar(_) | OpCode::LoadLocal(_) | OpCode::EvalScript);
            if is_word_op && std::mem::take(&mut interp.err_from_subst) {
                interp.err_pending_top = Some(Rc::from(text));
                interp.err_line = line;
            } else {
                let msg = e.message_text();
                interp.err_harness_frame(&msg, text, line);
            }
            // The error then crosses every still-open inlined `[...]`
            // region: each word-bracket-owning command defers its frame to
            // the top level (the tree-walk's CommandSub word boundary, the
            // outermost deferral winning); each expression-operand owner
            // appends its harness frame.
            unwind_subs(interp, code, st, &e);
            return Err(e);
        }
    };

    interp.result = result.clone();
    Ok(result)
}

struct VmState {
    stack: Vec<Value>,
    loops: Vec<LoopFrame>,
    /// Active inline foreach/lmap frames (see [`ForeachFrame`]).
    foreaches: Vec<ForeachFrame>,
    /// Body regions currently executing, as the `BeginCmd` site index of
    /// the construct that owns each (`if` arm / `while` body / `for` next
    /// script).  Pushed by not-taken condition jumps and the region marks,
    /// popped by the plain `Jump` that ends every body.
    bodies: Vec<Region>,
    /// Reusable argument buffer for the dispatch ops: entries are MOVED
    /// off the stack into it (drain), not cloned — one buffer per executor
    /// state instead of a fresh Vec per dispatched command.
    scratch: Vec<Value>,
    /// The unit's pooled constants (see [`Interp::const_pool`]), taken at
    /// `exec_bytecode` entry — PushConst/PushConstWide then push an Rc
    /// bump (slice index + clone) instead of re-materialising the literal
    /// string per push.
    consts: Option<Rc<[Value]>>,
    /// The unit's call-site command tokens (same pool entry as `consts`):
    /// `Call`/`DynCall` ops resolve through their slot instead of the
    /// cmd_cache hash probe + dispatch chain, re-resolving on a
    /// generation/name/namespace mismatch.  Shared `Rc` — every depth
    /// executing this unit uses the one table.
    cmd_sites: Option<CmdSites>,
    /// The unit whose consts/cmd_sites are loaded (identity by the held
    /// `Rc`, `None` = none loaded).  `exec_bytecode` reuses the loaded pair
    /// when the SAME unit borrows this state again — recursion into one
    /// proc is exactly that — instead of re-hashing the const pool
    /// (fib profile: the per-call pool probe was ~7% of the bench).
    /// Correct regardless of pool residency: the state holds its own
    /// `Rc`s, and keying identity on the held `Rc` (not a borrowed
    /// address) closes the ABA window where a pool clear-all plus a
    /// same-address reallocation would alias a dead unit's tables.
    consts_code: Option<Rc<rtcl_parser::ByteCode>>,
    /// Stack base of the current command's `{*}` expansion region.
    expand_base: usize,
    /// Site index of the `BeginCmd` most recently executed.
    cur_site: usize,
    entry_offset: usize,
    pc: usize,
}

impl VmState {
    fn new(entry_offset: usize) -> Self {
        VmState {
            stack: Vec::with_capacity(16),
            loops: Vec::new(),
            foreaches: Vec::new(),
            bodies: Vec::new(),
            scratch: Vec::new(),
            consts: None,
            cmd_sites: None,
            consts_code: None,
            expand_base: 0,
            cur_site: usize::MAX,
            entry_offset,
            pc: 0,
        }
    }

    fn pop_val(&mut self) -> Value {
        self.stack.pop().unwrap_or_else(Value::empty)
    }

    fn top_val(&self) -> Value {
        self.stack.last().cloned().unwrap_or_else(Value::empty)
    }

    /// Move the current command's arguments off the stack into `scratch`
    /// (splicing `{*}` expansions), leaving the stack truncated at
    /// `from`.  Borrowing, not owning: the dispatch helpers take the
    /// buffer by reference and the buffer survives for the next command.
    fn collect_args_into_scratch(&mut self, from: usize) {
        self.scratch.clear();
        self.scratch.extend(self.stack.drain(from..));
    }
}

/// Recycled [`VmState`]s, owned by the interp: every `exec_bytecode` call
/// borrows one and gives it back emptied, so proc calls stop paying the
/// four Vec allocations.  Boxed: the struct is ~168 bytes and pool-pop/
/// pool-push moved it whole (a memcpy each, visible in the fib profile);
/// the box rides the pool instead, so only a fresh recursion level ever
/// allocates one.  The type is opaque outside this module (the pool
/// methods are the only API); spare depth is capped — deeper recursion
/// churn simply drops the excess states.
pub(crate) struct VmPool {
    // Deliberate Box (see struct doc): pool pop/push must move an
    // 8-byte pointer, not the ~168-byte state.
    #[allow(clippy::vec_box)]
    spare: Vec<Box<VmState>>,
}

impl VmPool {
    pub(crate) fn new() -> Self {
        VmPool { spare: Vec::new() }
    }

    fn take(&mut self, entry_offset: usize) -> Box<VmState> {
        match self.spare.pop() {
            Some(mut st) => {
                st.expand_base = 0;
                st.cur_site = usize::MAX;
                st.entry_offset = entry_offset;
                st.pc = 0;
                st
            }
            None => Box::new(VmState::new(entry_offset)),
        }
    }

    fn give(&mut self, mut st: Box<VmState>) {
        st.stack.clear();
        st.loops.clear();
        st.foreaches.clear();
        st.bodies.clear();
        st.scratch.clear();
        // consts/cmd_sites/consts_code deliberately stay: the next
        // execution of the SAME unit reuses them without touching the
        // const pool (the recursion hot path).
        if self.spare.len() < 4 {
            self.spare.push(st);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared op bodies
// ---------------------------------------------------------------------------
//
// The ops that make up a compiled loop's body are dispatched from two call
// sites — `exec_inner`'s inline hot lane and the `exec_op` table — and both
// run these bodies: one semantic source per op, `#[inline]` so the split is
// a code-shape device, not an abstraction boundary.

#[inline]
fn op_push_const(st: &mut VmState, code: &ByteCode, idx: u16) {
    let v = st
        .consts
        .as_deref()
        .and_then(|vs| vs.get(idx as usize))
        .cloned()
        .unwrap_or_else(|| Value::from_str(code.get_const(idx).unwrap_or("")));
    st.stack.push(v);
}

#[inline]
fn op_push_const_wide(st: &mut VmState, code: &ByteCode, idx: u32) {
    let v = st
        .consts
        .as_deref()
        .and_then(|vs| vs.get(idx as usize))
        .cloned()
        .unwrap_or_else(|| Value::from_str(code.get_const_wide(idx).unwrap_or("")));
    st.stack.push(v);
}

#[inline]
fn op_store_var(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    idx: u16,
    keep: bool,
) -> Result<()> {
    let name = code.get_const(idx).unwrap_or("");
    let v = if keep { st.top_val() } else { st.pop_val() };
    interp.set_var(name, v)?;
    Ok(())
}

#[inline]
fn op_incr_var(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    idx: u16,
    amount: i64,
) -> Result<()> {
    let name = code.get_const(idx).unwrap_or("");
    // Fast path for the success case only; any failure (missing var,
    // non-integer value, overflow, traces, qualified/array names) falls
    // back to the real `incr`, whose errors it owns.
    match interp.incr_var_fast(name, amount) {
        Some(v) => st.stack.push(v),
        None => {
            let args = [
                Value::from_str("incr"),
                Value::from_str(name),
                Value::from_int(amount),
            ];
            // dispatch's builtin arm resets first (fresh error state);
            // the fallback owns its errors the same way.
            interp.err_code_raised = false;
            let v = super::commands::misc::cmd_incr(interp, &args)?;
            st.stack.push(v);
        }
    }
    Ok(())
}

/// `LoadLocal`'s value, without the stack push — the fused
/// `LoadLocal + JumpTrue/JumpFalse` and `LoadLocal + LappendLocal` arms
/// consume it directly.
#[inline]
fn load_local_value(interp: &mut Interp, code: &ByteCode, slot: u16) -> Result<Value> {
    let slot = slot as usize;
    match interp.frame_slot_value(slot) {
        Some(v) => Ok(v),
        None => {
            let name = code.locals().get(slot).map(String::as_str).unwrap_or("");
            interp.eval_var_ref(name)
        }
    }
}

#[inline]
fn op_load_local(interp: &mut Interp, code: &ByteCode, st: &mut VmState, slot: u16) -> Result<()> {
    let v = load_local_value(interp, code, slot)?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn op_store_local(interp: &mut Interp, code: &ByteCode, st: &mut VmState, slot: u16) -> Result<()> {
    let slot = slot as usize;
    // `set` keeps its result: the value stays on the stack.
    let v = st.top_val();
    if !interp.frame_slot_write(slot, &v) {
        let name = code.locals().get(slot).map(String::as_str).unwrap_or("");
        interp.set_var(name, v)?;
    }
    Ok(())
}

/// `StoreLocalPop`'s body: the result is discarded, so the popped value
/// moves into the slot (no clone) — the same delegation ladder as
/// [`op_store_local`], one stack round-trip shorter.
#[inline]
fn op_store_local_pop(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    slot: u16,
) -> Result<()> {
    let slot = slot as usize;
    let v = st.pop_val();
    if let Err(v) = interp.frame_slot_write_move(slot, v) {
        let name = code.locals().get(slot).map(String::as_str).unwrap_or("");
        interp.set_var(name, v)?;
    }
    Ok(())
}

/// `SubEnd`'s region close (see [`op_sub_end`]) followed by a store —
/// the `set x [...]` statement tail in one dispatch.  The steps, their
/// order and every delegation are the pair's verbatim.
#[inline]
fn op_sub_end_store_local_pop(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    slot: u16,
) -> Result<()> {
    op_sub_end(st);
    op_store_local_pop(interp, code, st, slot)
}

/// [`op_sub_end_store_local_pop`]'s name-keyed twin (`[SubEnd,
/// StoreVarPop]`).
#[inline]
fn op_sub_end_store_var_pop(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    idx: u16,
) -> Result<()> {
    op_sub_end(st);
    op_store_var(interp, code, st, idx, false)
}

/// `[SubEnd, Return]` — `return [...]`: close the bracket region, then
/// return the value (the `Return` arm's exact completion).
#[inline]
fn op_sub_end_return(st: &mut VmState) -> Result<()> {
    op_sub_end(st);
    let v = st.pop_val();
    Err(Error::ret(Some(v)))
}

/// `[BeginCmd s, SubMark]`: set the site, then open the bracket region of
/// the just-set site.
#[inline]
fn op_begin_cmd_sub_mark(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
) -> Result<()> {
    op_begin_cmd(interp, code, st, site)?;
    st.bodies.push(Region::Sub(st.cur_site));
    Ok(())
}

/// `[ExprMark, BeginCmd s]`: open the expression-operand region, then set
/// the site.
#[inline]
fn op_expr_mark_begin_cmd(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
) -> Result<()> {
    st.bodies.push(Region::Expr(st.cur_site));
    op_begin_cmd(interp, code, st, site)
}

/// `[BeginCmd s, PushConst i]`: set the site, then push the constant (the
/// dispatch shape's name push).
#[inline]
fn op_begin_cmd_push_const(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
    idx: u16,
) -> Result<()> {
    op_begin_cmd(interp, code, st, site)?;
    op_push_const(st, code, idx);
    Ok(())
}

/// `[Jump(t), NextMark]` (only fused when `t` is the mark itself — the
/// region-closing trampoline): normalize the region stack to the loop's
/// entry depth and open the next-script region of the loop's owner site.
/// Two entries reach this op: the body-end trampoline (the jump popped the
/// body region; `cur_site` is the body's site = the owner) and a compiled
/// `continue` (which already truncated to the same depth and set the
/// owner) — [`op_continue`]'s normalization covers both exactly.  No
/// backward-step charge: the trampoline's target is one past the jump,
/// and `continue` charged its own edge.
#[inline]
fn op_swap_to_next_mark(st: &mut VmState) {
    match st.loops.last() {
        Some(l) => {
            st.bodies.truncate(l.bodies_len);
            st.cur_site = l.owner;
        }
        None => {
            if let Some(region) = st.bodies.pop() {
                st.cur_site = region.site();
            }
        }
    }
    st.bodies.push(Region::Next(st.cur_site));
}

/// The `for` next-script tail `[SwapToNextMark, BeginCmdIncrLocalPop,
/// Jump]` in one dispatch: the swap's region normalization + next-region
/// open, the site set, the increment-and-discard, then the jump's region
/// close, step charge and transfer — the verbatim sequence, so entries at
/// the head (a compiled `continue`) and errors at any step see identical
/// state.  The window's Jump sat one past the fused head's next op, hence
/// `jump_pc = st.pc + 1` for the backward-charge parity.
#[allow(clippy::too_many_arguments)] // interp/code/st + the window's four operands
#[inline]
fn op_for_next_incr_local(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
    slot: u16,
    amount: i32,
    t: u32,
) -> Result<()> {
    op_swap_to_next_mark(st);
    op_begin_cmd(interp, code, st, site)?;
    let _ = incr_local_value(interp, code, slot, amount as i64)?;
    op_jump_from(interp, st, t, st.pc + 1)
}

/// The name-keyed twin of [`op_for_next_incr_local`].
#[allow(clippy::too_many_arguments)] // interp/code/st + the window's four operands
#[inline]
fn op_for_next_incr_var(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
    idx: u16,
    amount: i32,
    t: u32,
) -> Result<()> {
    op_swap_to_next_mark(st);
    op_begin_cmd(interp, code, st, site)?;
    op_incr_var_pop(interp, code, idx, amount as i64)?;
    op_jump_from(interp, st, t, st.pc + 1)
}

/// `[NextMark, BeginCmd s]`: open the next-script region, then set the
/// site.
#[inline]
fn op_next_mark_begin_cmd(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
) -> Result<()> {
    st.bodies.push(Region::Next(st.cur_site));
    op_begin_cmd(interp, code, st, site)
}

/// `[BeginCmd s, IncrLocalPop x k]` — the `for` next-script's `incr`
/// statement in one dispatch.
#[inline]
fn op_begin_cmd_incr_local_pop(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
    slot: u16,
    amount: i64,
) -> Result<()> {
    op_begin_cmd(interp, code, st, site)?;
    let _ = incr_local_value(interp, code, slot, amount)?;
    Ok(())
}

/// `[BeginCmd s, IncrVarPop i k]` — the name-keyed twin.
#[inline]
fn op_begin_cmd_incr_var_pop(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
    idx: u16,
    amount: i64,
) -> Result<()> {
    op_begin_cmd(interp, code, st, site)?;
    op_incr_var_pop(interp, code, idx, amount)
}

/// The `set <slot> [expr {$a + $b}]` statement window, in one dispatch —
/// the exact step sequence of `[BeginCmdSubMark, BeginCmd, AddSlotSlot,
/// SubEndStoreLocalPop]` (see the opcode's docs).
#[allow(clippy::too_many_arguments)] // interp/code/st + the window's five operands
#[inline]
fn op_slot_set_add_ss(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    set_site: u32,
    expr_site: u32,
    a: u16,
    b: u16,
    dst: u16,
) -> Result<()> {
    op_begin_cmd(interp, code, st, set_site)?;
    st.bodies.push(Region::Sub(st.cur_site));
    op_begin_cmd(interp, code, st, expr_site)?;
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = slot_read_or_name(interp, code, b as usize)?;
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '+')?;
    op_sub_end(st);
    let dst = dst as usize;
    if let Err(v) = interp.frame_slot_write_move(dst, v) {
        let name = code.locals().get(dst).map(String::as_str).unwrap_or("");
        interp.set_var(name, v)?;
    }
    Ok(())
}

/// The name-keyed twin (`set <var> [expr {$a + $b}]`).
#[allow(clippy::too_many_arguments)] // interp/code/st + the window's five operands
#[inline]
fn op_var_set_add_vv(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    set_site: u32,
    expr_site: u32,
    a: u16,
    b: u16,
    dst: u16,
) -> Result<()> {
    op_begin_cmd(interp, code, st, set_site)?;
    st.bodies.push(Region::Sub(st.cur_site));
    op_begin_cmd(interp, code, st, expr_site)?;
    let va = var_read(interp, code, a)?;
    let vb = var_read(interp, code, b)?;
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '+')?;
    op_sub_end(st);
    let name = code.get_const(dst).unwrap_or("");
    interp.set_var(name, v)?;
    Ok(())
}

/// `[PushConst i, SubMark]` — the name push plus the bracket region open.
#[inline]
fn op_push_const_sub_mark(st: &mut VmState, code: &ByteCode, idx: u16) {
    op_push_const(st, code, idx);
    st.bodies.push(Region::Sub(st.cur_site));
}

/// `[SubMark, BeginCmd s]` — the argument-bracket entry.
#[inline]
fn op_sub_mark_begin_cmd(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
) -> Result<()> {
    st.bodies.push(Region::Sub(st.cur_site));
    op_begin_cmd(interp, code, st, site)
}

/// `[ExprMark, BeginCmd s, PushConst i]` — an expression operand's region
/// open, command site and name push.
#[inline]
fn op_expr_mark_begin_push_const(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site: u32,
    idx: u16,
) -> Result<()> {
    st.bodies.push(Region::Expr(st.cur_site));
    op_begin_cmd(interp, code, st, site)?;
    op_push_const(st, code, idx);
    Ok(())
}

/// `[LoadLocal a, PushInt k, Sub]` — slot minus immediate.
#[inline]
fn op_sub_slot_imm(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = Value::from_int(k);
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '-')?;
    st.stack.push(v);
    Ok(())
}

/// `[LoadLocal a, LoadLocal b, Sub]` — slot minus slot.
#[inline]
fn op_sub_slot_slot(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = slot_read_or_name(interp, code, b as usize)?;
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '-')?;
    st.stack.push(v);
    Ok(())
}

/// `[SubEnd, DynCall argc]` — close the bracket region, then dispatch the
/// bracket-result call (the `DynCall` arm's exact body; the site cache
/// keys on `st.pc - 1`, which now names this fused op — the first
/// execution backfills it, every later one hits).
#[inline]
fn op_sub_end_dyn_call(
    interp: &mut Interp,
    st: &mut VmState,
    argc: u16,
) -> Result<()> {
    op_sub_end(st);
    let from = st.stack.len() - argc as usize;
    st.collect_args_into_scratch(from);
    let v = dispatch_site(interp, st, dispatch_dynamic)?;
    st.stack.push(v);
    Ok(())
}

/// `[SubEnd, Call { cmd_id, argc }]` — the cached-builtin twin.
#[inline]
fn op_sub_end_call(
    interp: &mut Interp,
    st: &mut VmState,
    argc: u16,
) -> Result<()> {
    op_sub_end(st);
    let from = st.stack.len() - argc as usize;
    st.collect_args_into_scratch(from);
    let v = dispatch_site(interp, st, dispatch_call)?;
    st.stack.push(v);
    Ok(())
}

/// `[SubEnd, Add]` — close the bracket region, then add.
#[inline]
fn op_sub_end_add(st: &mut VmState) -> Result<()> {
    op_sub_end(st);
    arith(st, '+')
}

/// `[Jump(t), ForeachNext]` (trampoline) with the body-mark folded in:
/// normalize the region stack to the foreach's entry depth (the body-end
/// jump popped the body region; a compiled `continue` truncated the same
/// way through its loop frame), then the iterator advance/bind (the taken
/// edge owns [`op_foreach_next`]'s step charge) and the body region open —
/// `body` already points *past* the branch's `BODY_MARK`, which only the
/// first iteration (entered by `ForeachStart`'s fall-through) executes.
/// The region kind mirrors [`op_body_mark_framed`].
#[inline]
fn op_foreach_next_close(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    info: u32,
    body: u32,
) -> Result<()> {
    match st.loops.last() {
        Some(l) => {
            st.bodies.truncate(l.bodies_len);
            st.cur_site = l.owner;
        }
        None => {
            if let Some(region) = st.bodies.pop() {
                st.cur_site = region.site();
            }
        }
    }
    let info_ref = code.foreach_info(info).expect("foreach info index");
    let (idx, max) = {
        let ff = st.foreaches.last().expect("foreach frame");
        (ff.idx + 1, ff.iters)
    };
    if idx < max {
        interp.charge_step()?;
        st.foreaches.last_mut().unwrap().idx = idx;
        foreach_bind(interp, code, info_ref, st, idx)?;
        let (framed, lmap) = {
            let ff = st.foreaches.last().expect("foreach frame");
            (ff.framed, ff.lmap)
        };
        if framed {
            st.bodies.push(Region::BodyFramed(st.cur_site, lmap));
        } else {
            st.bodies.push(Region::Body(st.cur_site));
        }
        st.pc = body as usize;
    }
    // done → fall through to the closing ForeachEnd
    Ok(())
}

/// `IncrLocal`'s value, without the stack push — the fused
/// `IncrLocal + Pop` arm consumes it directly.
#[inline]
fn incr_local_value(
    interp: &mut Interp,
    code: &ByteCode,
    slot: u16,
    amount: i64,
) -> Result<Value> {
    let slot = slot as usize;
    // Same fallback ladder as IncrVar: slot fast path, then the name fast
    // path (degraded frames), then the real `incr`, which owns creation
    // and the exact errors.
    let name = || code.locals().get(slot).map(String::as_str).unwrap_or("");
    match interp.frame_slot_incr(slot, amount) {
        Some(v) => Ok(v),
        None => match interp.incr_var_fast(name(), amount) {
            Some(v) => Ok(v),
            None => {
                let args = [
                    Value::from_str("incr"),
                    Value::from_str(name()),
                    Value::from_int(amount),
                ];
                // dispatch's builtin arm resets first (fresh error state);
                // the fallback owns its errors the same way.
                interp.err_code_raised = false;
                super::commands::misc::cmd_incr(interp, &args)
            }
        },
    }
}

#[inline]
fn op_incr_local(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    slot: u16,
    amount: i64,
) -> Result<()> {
    let v = incr_local_value(interp, code, slot, amount)?;
    st.stack.push(v);
    Ok(())
}

// The fused [LoadLocal, LoadLocal/PushInt, ARITH-or-CMP] superinstructions'
// bodies (see the executor arm docs for the fallback contract).

#[inline]
fn op_add_slot_slot(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = slot_read_or_name(interp, code, b as usize)?;
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '+')?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn op_add_slot_imm(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = Value::from_int(k);
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '+')?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn op_cmp_slot_slot(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
    cc: u8,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = slot_read_or_name(interp, code, b as usize)?;
    st.stack.push(cmp_values(&va, &vb, cc));
    Ok(())
}

#[inline]
fn op_cmp_slot_imm(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
    cc: u8,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = Value::from_int(k);
    st.stack.push(cmp_values(&va, &vb, cc));
    Ok(())
}

/// `LoadVar`'s read for the name-keyed superinstructions — the exact
/// `eval_var_ref` path (array references, link indirection, canonical
/// errors) the unfused window's `LoadVar` would take.
#[inline]
fn var_read(interp: &mut Interp, code: &ByteCode, idx: u16) -> Result<Value> {
    let name = code.get_const(idx).unwrap_or("");
    interp.eval_var_ref(name)
}

#[inline]
fn op_add_var_var(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
) -> Result<()> {
    let va = var_read(interp, code, a)?;
    let vb = var_read(interp, code, b)?;
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '+')?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn op_add_var_imm(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
) -> Result<()> {
    let va = var_read(interp, code, a)?;
    let vb = Value::from_int(k);
    let v = crate::types::expr_ops::numeric_binop(&va, &vb, '+')?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn op_cmp_var_var(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
    cc: u8,
) -> Result<()> {
    let va = var_read(interp, code, a)?;
    let vb = var_read(interp, code, b)?;
    st.stack.push(cmp_values(&va, &vb, cc));
    Ok(())
}

#[inline]
fn op_cmp_var_imm(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
    cc: u8,
) -> Result<()> {
    let va = var_read(interp, code, a)?;
    st.stack.push(cmp_values(&va, &Value::from_int(k), cc));
    Ok(())
}

/// Fused `[CmpVarVar, JumpTrue/JumpFalse]` — the name-keyed twin of
/// [`op_cmp_ss_jump`].
#[inline]
#[allow(clippy::too_many_arguments)] // interp/code/st + the op's five operands
fn op_cmp_vv_jump(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
    cc: u8,
    t: u32,
    want: bool,
) -> Result<()> {
    let va = var_read(interp, code, a)?;
    let vb = var_read(interp, code, b)?;
    let cond = crate::types::expr_ops::cmp_bool(&va, &vb, cc);
    jump_if_bool(interp, st, cond, t, want, st.pc)?;
    Ok(())
}

/// Fused `[CmpVarImm, JumpTrue/JumpFalse]` — [`op_cmp_vv_jump`] with the
/// immediate operand.
#[inline]
#[allow(clippy::too_many_arguments)] // interp/code/st + the op's five operands
fn op_cmp_vi_jump(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
    cc: u8,
    t: u32,
    want: bool,
) -> Result<()> {
    let va = var_read(interp, code, a)?;
    let cond = crate::types::expr_ops::cmp_bool(&va, &Value::from_int(k), cc);
    jump_if_bool(interp, st, cond, t, want, st.pc)?;
    Ok(())
}

/// `lappend <name> <value>`'s shared body (the `LappendVar` arm and the
/// `LappendVarVar` fusion): take → in-place append → store through
/// `take_var_fast` (its own guards own traces/links/arrays/qualifiers), or
/// the dispatched `cmd_lappend` with the assembled arguments for
/// creation, strict parse and exact errors.  Returns the command's
/// result — the caller pushes it or discards it.
#[inline]
fn lappend_var_impl(
    interp: &mut Interp,
    code: &ByteCode,
    name_idx: u16,
    value: Value,
) -> Result<Value> {
    let name = code.get_const(name_idx).unwrap_or("");
    if let Some(mut v) = interp.take_var_fast(name) {
        if let Some(items) = v.as_list_mut() {
            items.push(value);
            let r = interp.set_var(name, v.clone());
            // The take removed the variable; a failure here would silently
            // drop it.  The guards (traces/links/arrays/qualifiers) make
            // this unreachable — assert it stays that way.
            debug_assert!(r.is_ok(), "lappend fast-path store failed for {name:?}");
            return Ok(v);
        }
        interp.store_var(name, v);
    }
    let args = [
        Value::from_str("lappend"),
        Value::from_str(name),
        value,
    ];
    // dispatch's builtin arm resets first (fresh error state); the
    // fallback owns its errors the same way.
    interp.err_code_raised = false;
    super::commands::list::cmd_lappend(interp, &args)
}

/// Fused `[LoadVar src, LappendVar dst, Pop]` — the name-keyed
/// `lappend <var> $value` statement: read the source (LoadVar's exact
/// path), append through [`lappend_var_impl`], discard the result.
#[inline]
fn op_lappend_var_var(interp: &mut Interp, code: &ByteCode, src: u16, dst: u16) -> Result<()> {
    let value = var_read(interp, code, src)?;
    let _ = lappend_var_impl(interp, code, dst, value)?;
    Ok(())
}

/// Fused `[IncrVar idx, Pop]` — [`op_incr_var`]'s ladder with the
/// discarded result.
#[inline]
fn op_incr_var_pop(interp: &mut Interp, code: &ByteCode, idx: u16, amount: i64) -> Result<()> {
    let name = code.get_const(idx).unwrap_or("");
    match interp.incr_var_fast(name, amount) {
        Some(_) => Ok(()),
        None => {
            let args = [
                Value::from_str("incr"),
                Value::from_str(name),
                Value::from_int(amount),
            ];
            // dispatch's builtin arm resets first (fresh error state);
            // the fallback owns its errors the same way.
            interp.err_code_raised = false;
            let _ = super::commands::misc::cmd_incr(interp, &args)?;
            Ok(())
        }
    }
}

/// Fused `[CmpSlotSlot, JumpTrue/JumpFalse]`: the comparison branches
/// directly through [`jump_if_value`] — no boolean is materialized.  The
/// operand read replicates [`op_cmp_slot_slot`]'s exact fallback.  At
/// entry `st.pc` points one past this op — the index the pair's jump op
/// would have had — which is the `jump_pc` the backward-edge step charge
/// tests against; the not-taken path falls through to `jump_pc + 1`.
#[inline]
#[allow(clippy::too_many_arguments)] // interp/code/st + the op's five operands
fn op_cmp_ss_jump(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    b: u16,
    cc: u8,
    t: u32,
    want: bool,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let vb = slot_read_or_name(interp, code, b as usize)?;
    // The comparison's truth directly — no boolean `Value` round-trip
    // (see `cmp_bool`).
    let cond = crate::types::expr_ops::cmp_bool(&va, &vb, cc);
    // The fused op compacted the pair into one slot: `st.pc` already
    // points at the op after the pair, so the not-taken fall-through is
    // simply "leave `st.pc` alone" (the charge parity below still uses
    // `st.pc + 1`, the pair's original jump-op index).
    jump_if_bool(interp, st, cond, t, want, st.pc)?;
    Ok(())
}

/// Fused `[CmpSlotImm, JumpTrue/JumpFalse]` — [`op_cmp_ss_jump`] with the
/// immediate operand.
#[inline]
#[allow(clippy::too_many_arguments)] // interp/code/st + the op's five operands
fn op_cmp_si_jump(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    a: u16,
    k: i64,
    cc: u8,
    t: u32,
    want: bool,
) -> Result<()> {
    let va = slot_read_or_name(interp, code, a as usize)?;
    let cond = crate::types::expr_ops::cmp_bool(&va, &Value::from_int(k), cc);
    jump_if_bool(interp, st, cond, t, want, st.pc)?;
    Ok(())
}

/// Fused `[LoadLocal src, LappendLocal dst, Pop]` — the `lappend <var>
/// $value` statement shape: same fast/fallback ladder as the pair (the
/// `LappendLocal` arm's take/append/restore fast path; the dispatched
/// `cmd_lappend` fallback with identical arguments otherwise), without
/// the stack round-trip or the discarded result.
#[inline]
fn op_lappend_slot_slot(interp: &mut Interp, code: &ByteCode, src: u16, dst: u16) -> Result<()> {
    let value = load_local_value(interp, code, src)?;
    let dst = dst as usize;
    if interp.lappend_slot_fast(dst, &value) {
        return Ok(());
    }
    let name = code.locals().get(dst).map(String::as_str).unwrap_or("");
    let args = [
        Value::from_str("lappend"),
        Value::from_str(name),
        value,
    ];
    // dispatch's builtin arm resets first (fresh error state); the
    // fallback owns its errors the same way.
    interp.err_code_raised = false;
    let _ = super::commands::list::cmd_lappend(interp, &args)?;
    Ok(())
}

#[inline]
fn op_jump(interp: &mut Interp, st: &mut VmState, t: u32) -> Result<()> {
    // `st.pc` is the jump op's own index + 1 (the dispatch incremented).
    op_jump_from(interp, st, t, st.pc - 1)
}

/// [`op_jump`] with the jump's *original* op index supplied: a fused
/// loop-tail op compacts several ops into the head, and the backward-step
/// charge tests against where the jump op sits (`jump_pc + 1`), exactly as
/// the dispatched `Jump` did.
#[inline]
fn op_jump_from(interp: &mut Interp, st: &mut VmState, t: u32, jump_pc: usize) -> Result<()> {
    // Every inline body ends with a plain Jump — close its region and
    // restore the owning construct as the current command: the words/bodies
    // inside may have opened commands of their own (inlined brackets), and
    // the construct's remaining ops (a re-checked loop condition, the
    // result push) must attribute to the construct, not to the last inner
    // command.
    if let Some(region) = st.bodies.pop() {
        st.cur_site = region.site();
    }
    // A1 step budget: backward jump = loop back-edge — pure control flow
    // (`while {1} {}`) dispatches zero commands, only the back-edge can
    // charge it.
    if i64::from(t) < (jump_pc + 1) as i64 {
        interp.charge_step()?;
    }
    st.pc = t as usize;
    Ok(())
}

/// The branch semantic shared by `JumpTrue`/`JumpFalse` and the fused
/// `[cmp, jcc]` superinstructions: branch on `v` directly, without the
/// stack round-trip the unfused pair performs.  `jump_pc` is the index of
/// the *jump op* being realised — the backward-edge step charge
/// (`op_jump`'s loop-back parity) tests against it exactly as the
/// dispatched `op_jump_if` did with `st.pc == jump_pc + 1`.  Returns
/// whether the branch was taken; a not-taken branch leaves `st.pc`
/// untouched (callers fall through past the consumed jump) and opens the
/// construct's body region, exactly as the stack round-trip would.
#[inline]
fn jump_if_value(
    interp: &mut Interp,
    st: &mut VmState,
    v: &Value,
    t: u32,
    want: bool,
    jump_pc: usize,
) -> Result<bool> {
    jump_if_bool(
        interp,
        st,
        crate::types::expr_funcs::strict_bool(v)?,
        t,
        want,
        jump_pc,
    )
}

/// [`jump_if_value`] for a caller that already holds the truth value (the
/// fused comparison branches compute it directly —
/// [`crate::types::expr_ops::cmp_bool`] — instead of materializing a
/// boolean `Value` and re-coercing it).
#[inline]
fn jump_if_bool(
    interp: &mut Interp,
    st: &mut VmState,
    cond: bool,
    t: u32,
    want: bool,
    jump_pc: usize,
) -> Result<bool> {
    if cond == want {
        if (t as i64) < (jump_pc + 1) as i64 {
            interp.charge_step()?;
        }
        st.pc = t as usize;
        Ok(true)
    } else {
        // Not taken → entering an inline body of the current command's
        // construct.
        st.bodies.push(Region::Body(st.cur_site));
        Ok(false)
    }
}

#[inline]
fn op_jump_if(interp: &mut Interp, st: &mut VmState, t: u32, want: bool) -> Result<()> {
    let v = st.pop_val();
    // The dispatch incremented past the jump op: its own index is
    // `st.pc - 1`.
    jump_if_value(interp, st, &v, t, want, st.pc - 1)?;
    Ok(())
}

#[inline]
fn op_loop_enter(st: &mut VmState, cont: u32, brk: u32) {
    st.loops.push(LoopFrame {
        cont,
        brk,
        bodies_len: st.bodies.len(),
        stack_len: st.stack.len(),
        owner: st.cur_site,
    });
}

#[inline]
fn op_foreach_start(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    info: u32,
    next: u32,
    end: u32,
) -> Result<()> {
    let info_ref = code.foreach_info(info).expect("foreach info index");
    let ngroups = info_ref.groups.len();
    let mut lists = Vec::with_capacity(ngroups);
    // Pop the group data values (last group on top), strict-parse each
    // exactly like cmd_foreach — same errors, same errorCode.  A value
    // already carrying a list rep iterates by reference (zero-copy); other
    // reps materialise an owned Vec through the strict parse.  A list-rep
    // value never fails the strict parse, so the error surface is identical
    // to the always-owned form.
    for _ in 0..ngroups {
        let v = st.pop_val();
        if v.as_list_ref().is_some() {
            lists.push(ForeachList::Rep(v));
        } else {
            lists.push(ForeachList::Owned(super::commands::list::strict_list(
                interp, &v,
            )?));
        }
    }
    lists.reverse();
    let max = info_ref
        .groups
        .iter()
        .zip(&lists)
        .map(|(g, l)| l.len().div_ceil(g.len().max(1)))
        .max()
        .unwrap_or(0);
    let collected = info_ref.lmap.then(Vec::new);
    st.foreaches.push(ForeachFrame {
        lists,
        idx: 0,
        iters: max,
        collected,
        framed: info_ref.framed,
        lmap: info_ref.lmap,
    });
    // The loop frame rides the ordinary machinery: compiled Break/Continue
    // and error-form signals route through it.
    st.loops.push(LoopFrame {
        cont: next,
        brk: end,
        bodies_len: st.bodies.len(),
        stack_len: st.stack.len(),
        owner: st.cur_site,
    });
    if max == 0 {
        st.pc = end as usize;
    } else {
        foreach_bind(interp, code, info_ref, st, 0)?;
    }
    Ok(())
}

#[inline]
fn op_foreach_next(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    info: u32,
    body: u32,
) -> Result<()> {
    let info_ref = code.foreach_info(info).expect("foreach info index");
    let (idx, max) = {
        let ff = st.foreaches.last().expect("foreach frame");
        (ff.idx + 1, ff.iters)
    };
    if idx < max {
        // A1 step budget: taken iteration edge = one step.
        interp.charge_step()?;
        st.foreaches.last_mut().unwrap().idx = idx;
        foreach_bind(interp, code, info_ref, st, idx)?;
        st.pc = body as usize;
    }
    // done → fall through to the closing ForeachEnd
    Ok(())
}

#[inline]
fn op_foreach_collect(st: &mut VmState) {
    let v = st.pop_val();
    if let Some(ff) = st.foreaches.last_mut() {
        if let Some(c) = &mut ff.collected {
            c.push(v);
        }
    }
}

#[inline]
fn op_foreach_end(st: &mut VmState) {
    st.loops.pop();
    let ff = st.foreaches.pop();
    let v = match ff.and_then(|f| f.collected) {
        Some(c) => Value::from_list(&c),
        None => Value::empty(),
    };
    st.stack.push(v);
}

#[inline]
fn op_break(interp: &mut Interp, st: &mut VmState) -> Result<()> {
    match st.loops.last() {
        Some(l) => {
            st.bodies.truncate(l.bodies_len);
            st.stack.truncate(l.stack_len);
            st.cur_site = l.owner;
            if l.brk < st.pc as u32 {
                interp.charge_step()?;
            }
            st.pc = l.brk as usize;
            Ok(())
        }
        None => Err(Error::brk()),
    }
}

#[inline]
fn op_continue(interp: &mut Interp, st: &mut VmState) -> Result<()> {
    match st.loops.last() {
        Some(l) => {
            st.bodies.truncate(l.bodies_len);
            st.stack.truncate(l.stack_len);
            st.cur_site = l.owner;
            // A1 step budget: continue jumps back to the loop's condition —
            // a backward transfer IS the iteration edge when the body ends
            // in `continue` (no plain Jump runs).
            if l.cont < st.pc as u32 {
                interp.charge_step()?;
            }
            st.pc = l.cont as usize;
            Ok(())
        }
        None => Err(Error::cont()),
    }
}

#[inline]
fn op_begin_cmd(
    interp: &mut Interp,
    code: &ByteCode,
    st: &mut VmState,
    site_idx: u32,
) -> Result<()> {
    if interp.call_depth > interp.max_call_depth {
        return Err(Error::runtime(
            "maximum recursion depth exceeded",
            crate::error::ErrorCode::StackOverflow,
        ));
    }
    let site = &code.sites[site_idx as usize];
    interp.cur_cmd_text = site.text;
    interp.cur_cmd_line = site.line_rel as usize;
    interp.cur_cmd_word_srcs = super::util::WordSrcs::from_rc(&site.word_srcs);
    interp.line_offset = st.entry_offset + site.line_delta as usize;
    st.cur_site = site_idx as usize;
    Ok(())
}

#[inline]
fn op_body_mark_framed(st: &mut VmState) {
    // The compiler only emits this op for framed units, so `framed` is
    // known true here — `lmap` is the only per-iteration input.
    let lmap = st.foreaches.last().map(|ff| ff.lmap).unwrap_or(false);
    st.bodies.push(Region::BodyFramed(st.cur_site, lmap));
}

#[inline]
fn op_sub_end(st: &mut VmState) {
    // Close the bracket region and restore its owning command as current:
    // the bracket's inner commands clobbered cur_site, and the owner's
    // remaining words / its own dispatch must attribute to the owner (e.g.
    // `nosuch [list a]` logs `"nosuch [list a]"`, not `"list a"`).
    if let Some(region) = st.bodies.pop() {
        st.cur_site = region.site();
    }
}

#[inline]
fn op_load_var(interp: &mut Interp, code: &ByteCode, st: &mut VmState, idx: u16) -> Result<()> {
    let name = code.get_const(idx).unwrap_or("");
    // eval_var_ref, not a raw read: `$a($i)` words compile to LoadVar with
    // the raw reference text and need the same index-substitution dance
    // eval_word does.
    let v = interp.eval_var_ref(name)?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn op_concat(st: &mut VmState, n: u16) {
    let n = n as usize;
    let from = st.stack.len() - n;
    let mut s = String::new();
    for v in &st.stack[from..] {
        s.push_str(v.as_str());
    }
    st.stack.truncate(from);
    st.stack.push(Value::from_str(&s));
}

#[inline]
fn op_int_mod(st: &mut VmState) -> Result<()> {
    let b = st.pop_val();
    let a = st.pop_val();
    let v = crate::types::expr_ops::int_mod(&a, &b)?;
    st.stack.push(v);
    Ok(())
}

/// Execute one instruction.  Errors carry the same shape the tree-walk
/// would produce for the same operation; the caller classifies them into
/// errorInfo frames.
fn exec_op(interp: &mut Interp, code: &ByteCode, op: &OpCode, st: &mut VmState) -> Result<()> {
    match op {
        // ── Stack ───────────────────────────────────────────────────────
        // Literals come from the unit's pooled constants (one Rc bump);
        // a pool miss re-materialises from the string table, which is
        // exactly what the pool was built from — same bytes either way.
        OpCode::PushConst(idx) => op_push_const(st, code, *idx),
        OpCode::PushConstWide(idx) => op_push_const_wide(st, code, *idx),
        OpCode::PushEmpty => st.stack.push(Value::empty()),
        OpCode::PushInt(n) => st.stack.push(Value::from_int(*n)),
        // float_value renders Tcl-exact (expr-52.x precision rules), not
        // Rust's default float formatting.
        OpCode::PushFloat(f) => {
            st.stack
                .push(crate::types::expr_funcs::float_value(*f));
        }
        OpCode::PushTrue => st.stack.push(Value::from_bool(true)),
        OpCode::PushFalse => st.stack.push(Value::from_bool(false)),
        OpCode::Pop => {
            st.stack.pop();
        }
        OpCode::Dup => {
            let v = st.top_val();
            st.stack.push(v);
        }

        // ── Variables ───────────────────────────────────────────────────
        OpCode::LoadVar(idx) => op_load_var(interp, code, st, *idx)?,
        OpCode::StoreVar(idx) | OpCode::StoreVarPop(idx) => {
            let keep = matches!(op, OpCode::StoreVar(_));
            op_store_var(interp, code, st, *idx, keep)?
        }
        OpCode::IncrVar(idx, amount) => op_incr_var(interp, code, st, *idx, *amount)?,

        // ── Slot locals (E2) ────────────────────────────────────────────
        // The slot ops only fire on slot-compiled frames; every fallback
        // goes through the name-keyed path, which consults the same table
        // (degraded, statics, unset cells, traces) — so the two storage
        // forms alias one variable set either way.
        OpCode::LoadLocal(slot) => op_load_local(interp, code, st, *slot)?,
        OpCode::StoreLocal(slot) => op_store_local(interp, code, st, *slot)?,
        OpCode::StoreLocalPop(slot) => op_store_local_pop(interp, code, st, *slot)?,
        // ── Folded lappend ──────────────────────────────────────────────
        // `lappend <literal-var> <value>`: the fast path appends through
        // the variable's canonical slot storage (cmd_lappend's take/
        // append/restore, slot-indexed — no name probes).  EVERY
        // divergence risk (traces, aliased/linked slots, non-list rep
        // needing the strict parse, creation) delegates to the real
        // `cmd_lappend` with the assembled [cmd, name, value] args —
        // identical errors, identical frames.
        OpCode::LappendLocal(slot) => {
            let slot = *slot as usize;
            let value = st.pop_val();
            if interp.lappend_slot_fast(slot, &value) {
                let result = interp
                    .frames
                    .last()
                    .and_then(|f| f.slots.get(slot))
                    .cloned()
                    .unwrap_or_else(Value::empty);
                st.stack.push(result);
            } else {
                // The name is only needed on the fallback path — keep the
                // hot path free of the per-iteration String materialization.
                let name = code.locals().get(slot).map(String::as_str).unwrap_or("");
                let args = [
                    Value::from_str("lappend"),
                    Value::from_str(name),
                    value,
                ];
                let v = super::commands::list::cmd_lappend(interp, &args)?;
                st.stack.push(v);
            }
        }
        OpCode::LappendVar(name_idx) => {
            // Fast path: take -> append in place -> store (cmd_lappend's
            // own fast body, minus the arg assembly + command prologue).
            // take_var_fast owns every guard (traces, links, arrays, ns
            // qualifiers); a non-list rep is restored and the full
            // command runs (creation, strict parse, exact errors).
            let value = st.pop_val();
            let v = lappend_var_impl(interp, code, *name_idx, value)?;
            st.stack.push(v);
        }
        OpCode::IncrLocal(slot, amount) => op_incr_local(interp, code, st, *slot, *amount)?,

        // ── Expansion ───────────────────────────────────────────────────
        OpCode::ExpandMark => {
            st.expand_base = st.stack.len();
        }
        OpCode::ExpandList => {
            let v = st.pop_val();
            match v.as_list() {
                // Flatten: the elements join the stack individually —
                // the call ops collect the whole `expand_base..` range,
                // so per-expansion grouping is unnecessary.
                Some(items) => st.stack.extend(items),
                None => st.stack.push(v),
            }
        }
        OpCode::Concat(n) => op_concat(st, *n),

        // ── Control flow ────────────────────────────────────────────────
        OpCode::Jump(t) => op_jump(interp, st, *t)?,
        OpCode::JumpTrue(t) => op_jump_if(interp, st, *t, true)?,
        OpCode::JumpFalse(t) => op_jump_if(interp, st, *t, false)?,

        // ── Loops ───────────────────────────────────────────────────────
        OpCode::LoopEnter { cont, brk } => op_loop_enter(st, *cont, *brk),
        OpCode::LoopExit => {
            st.loops.pop();
        }

        // ── Inline foreach/lmap (D) ────────────────────────────────────
        OpCode::ForeachStart { info, next, end } => {
            op_foreach_start(interp, code, st, *info, *next, *end)?
        }
        OpCode::ForeachNext { info, body } => op_foreach_next(interp, code, st, *info, *body)?,
        OpCode::ForeachCollect => op_foreach_collect(st),
        OpCode::ForeachEnd => op_foreach_end(st),
        OpCode::Break => op_break(interp, st)?,
        OpCode::Continue => op_continue(interp, st)?,

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
        OpCode::BeginCmd(site_idx) => op_begin_cmd(interp, code, st, *site_idx)?,
        OpCode::BodyMark => {
            st.bodies.push(Region::Body(st.cur_site));
        }
        OpCode::BodyMarkFramed => op_body_mark_framed(st),
        OpCode::NextMark => {
            st.bodies.push(Region::Next(st.cur_site));
        }
        OpCode::SubMark => {
            st.bodies.push(Region::Sub(st.cur_site));
        }
        OpCode::ExprMark => {
            st.bodies.push(Region::Expr(st.cur_site));
        }
        OpCode::SubEnd => op_sub_end(st),
        OpCode::Call { argc, .. } => {
            let from = st.stack.len() - *argc as usize;
            st.collect_args_into_scratch(from);
            let v = dispatch_site(interp, st, dispatch_call)?;
            st.stack.push(v);
        }
        OpCode::CallExpand { .. } => {
            st.collect_args_into_scratch(st.expand_base);
            let v = dispatch_site(interp, st, dispatch_call)?;
            st.stack.push(v);
        }
        OpCode::DynCall { argc } => {
            let from = st.stack.len() - *argc as usize;
            st.collect_args_into_scratch(from);
            let v = dispatch_site(interp, st, dispatch_dynamic)?;
            st.stack.push(v);
        }
        OpCode::DynCallExpand { .. } => {
            st.collect_args_into_scratch(st.expand_base);
            let v = dispatch_site(interp, st, dispatch_dynamic)?;
            st.stack.push(v);
        }

        // ── Evaluation ──────────────────────────────────────────────────
        OpCode::EvalScript => {
            let script = st.pop_val();
            match interp.eval(script.as_str()) {
                Ok(v) => st.stack.push(v),
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
            st.stack.push(v);
        }

        // ── Arithmetic / comparison (expr_ops = the expr parser's own
        //    semantics — overflow widening, floor division, exact error
        //    text) ──────────────────────────────────────────────────────
        OpCode::Add => arith(st, '+')?,
        // ── Slot superinstructions (peephole loop unrolling) ────────────
        // The fused [LoadLocal, LoadLocal/PushInt, ARITH-or-CMP] triples.
        // Operand reads replicate LoadLocal exactly: the slot cell when
        // set, `eval_var_ref` on the table name otherwise (linked slots,
        // globals fallback, canonical errors — all owned by the same
        // name path).  Comparisons run through op_rel/op_eq so string
        // operands and error shapes are identical to the unfused ops.
        OpCode::AddSlotSlot(a, b) => op_add_slot_slot(interp, code, st, *a, *b)?,
        OpCode::AddSlotImm(a, k) => op_add_slot_imm(interp, code, st, *a, *k)?,
        OpCode::CmpSlotSlot(a, b, cc) => op_cmp_slot_slot(interp, code, st, *a, *b, *cc)?,
        OpCode::CmpSlotImm(a, k, cc) => op_cmp_slot_imm(interp, code, st, *a, *k, *cc)?,
        OpCode::CmpSlotSlotJumpTrue(a, b, cc, t) => {
            op_cmp_ss_jump(interp, code, st, *a, *b, *cc, *t, true)?
        }
        OpCode::CmpSlotSlotJumpFalse(a, b, cc, t) => {
            op_cmp_ss_jump(interp, code, st, *a, *b, *cc, *t, false)?
        }
        OpCode::CmpSlotImmJumpTrue(a, k, cc, t) => {
            op_cmp_si_jump(interp, code, st, *a, *k, *cc, *t, true)?
        }
        OpCode::CmpSlotImmJumpFalse(a, k, cc, t) => {
            op_cmp_si_jump(interp, code, st, *a, *k, *cc, *t, false)?
        }
        OpCode::LappendSlotSlot(src, dst) => op_lappend_slot_slot(interp, code, *src, *dst)?,
        OpCode::LappendVarVar(src, dst) => op_lappend_var_var(interp, code, *src, *dst)?,
        OpCode::IncrVarPop(idx, amount) => op_incr_var_pop(interp, code, *idx, *amount)?,
        OpCode::BeginCmdSubMark(site) => op_begin_cmd_sub_mark(interp, code, st, *site)?,
        OpCode::ExprMarkBeginCmd(site) => op_expr_mark_begin_cmd(interp, code, st, *site)?,
        OpCode::BeginCmdPushConst(site, idx) => {
            op_begin_cmd_push_const(interp, code, st, *site, *idx)?
        }
        OpCode::SubEndStoreLocalPop(slot) => {
            op_sub_end_store_local_pop(interp, code, st, *slot)?
        }
        OpCode::SubEndStoreVarPop(idx) => op_sub_end_store_var_pop(interp, code, st, *idx)?,
        OpCode::SubEndReturn => op_sub_end_return(st)?,
        OpCode::SwapToNextMark => op_swap_to_next_mark(st),
        OpCode::ForNextIncrLocal(site, slot, amount, t) => {
            op_for_next_incr_local(interp, code, st, *site, *slot, *amount, *t)?
        }
        OpCode::ForNextIncrVar(site, idx, amount, t) => {
            op_for_next_incr_var(interp, code, st, *site, *idx, *amount, *t)?
        }
        OpCode::NextMarkBeginCmd(site) => op_next_mark_begin_cmd(interp, code, st, *site)?,
        OpCode::BeginCmdIncrLocalPop(site, slot, amount) => {
            op_begin_cmd_incr_local_pop(interp, code, st, *site, *slot, *amount)?
        }
        OpCode::BeginCmdIncrVarPop(site, idx, amount) => {
            op_begin_cmd_incr_var_pop(interp, code, st, *site, *idx, *amount)?
        }
        OpCode::ForeachNextClose(info, body) => {
            op_foreach_next_close(interp, code, st, *info, *body)?
        }
        OpCode::SlotSetAddSS { set_site, expr_site, a, b, dst } => {
            op_slot_set_add_ss(interp, code, st, *set_site, *expr_site, *a, *b, *dst)?
        }
        OpCode::VarSetAddVV { set_site, expr_site, a, b, dst } => {
            op_var_set_add_vv(interp, code, st, *set_site, *expr_site, *a, *b, *dst)?
        }
        OpCode::PushConstSubMark(idx) => op_push_const_sub_mark(st, code, *idx),
        OpCode::SubMarkBeginCmd(site) => op_sub_mark_begin_cmd(interp, code, st, *site)?,
        OpCode::ExprMarkBeginPushConst(site, idx) => {
            op_expr_mark_begin_push_const(interp, code, st, *site, *idx)?
        }
        OpCode::SubSlotImm(a, k) => op_sub_slot_imm(interp, code, st, *a, *k)?,
        OpCode::SubSlotSlot(a, b) => op_sub_slot_slot(interp, code, st, *a, *b)?,
        OpCode::SubEndDynCall(argc) => op_sub_end_dyn_call(interp, st, *argc)?,
        OpCode::SubEndCall { argc, .. } => op_sub_end_call(interp, st, *argc)?,
        OpCode::SubEndAdd => op_sub_end_add(st)?,
        OpCode::AddVarVar(a, b) => op_add_var_var(interp, code, st, *a, *b)?,
        OpCode::AddVarImm(a, k) => op_add_var_imm(interp, code, st, *a, *k)?,
        OpCode::CmpVarVar(a, b, cc) => op_cmp_var_var(interp, code, st, *a, *b, *cc)?,
        OpCode::CmpVarImm(a, k, cc) => op_cmp_var_imm(interp, code, st, *a, *k, *cc)?,
        OpCode::CmpVarVarJumpTrue(a, b, cc, t) => {
            op_cmp_vv_jump(interp, code, st, *a, *b, *cc, *t, true)?
        }
        OpCode::CmpVarVarJumpFalse(a, b, cc, t) => {
            op_cmp_vv_jump(interp, code, st, *a, *b, *cc, *t, false)?
        }
        OpCode::CmpVarImmJumpTrue(a, k, cc, t) => {
            op_cmp_vi_jump(interp, code, st, *a, *k, *cc, *t, true)?
        }
        OpCode::CmpVarImmJumpFalse(a, k, cc, t) => {
            op_cmp_vi_jump(interp, code, st, *a, *k, *cc, *t, false)?
        }
        OpCode::IncrLocalPop(slot, amount) => {
            let _ = incr_local_value(interp, code, *slot, *amount)?;
        }
        OpCode::Sub => arith(st, '-')?,
        OpCode::Mul => arith(st, '*')?,
        OpCode::Div => arith(st, '/')?,
        OpCode::Mod => op_int_mod(st)?,
        OpCode::Pow => {
            let b = st.pop_val();
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_pow(a, b)?;
            st.stack.push(v);
        }
        OpCode::Neg => {
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_neg(&a)?;
            st.stack.push(v);
        }
        OpCode::Not => {
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_not(&a)?;
            st.stack.push(v);
        }
        OpCode::BitNot => {
            let a = st.pop_val();
            let v = crate::types::expr_ops::op_bitnot(&a)?;
            st.stack.push(v);
        }
        OpCode::Eq => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(crate::types::expr_ops::op_eq(&a, &b));
        }
        OpCode::Ne => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(crate::types::expr_ops::op_ne(&a, &b));
        }
        OpCode::Lt => rel(st, "<")?,
        OpCode::Gt => rel(st, ">")?,
        OpCode::Le => rel(st, "<=")?,
        OpCode::Ge => rel(st, ">=")?,
        OpCode::StrEq => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(crate::types::expr_ops::op_str_eq(&a, &b));
        }
        OpCode::StrNe => {
            let (a, b) = pop_pair(st);
            st.stack
                .push(crate::types::expr_ops::op_str_ne(&a, &b));
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
// Inline foreach helpers
// ---------------------------------------------------------------------------

/// Bind iteration `idx`'s values of every group: the slot when the
/// current frame still holds one for the target (unset cells recreate —
/// tclsh clears the compiledLocal's Var, the table entry stays), the
/// name-keyed `set_var` otherwise (degraded frames, qualified/array
/// names, statics).  Compiled-context var-write failures are PLAIN — no
/// `(setting foreach loop variable)` decoration — but ::errorCode still
/// installs TCL WRITE VARNAME and the error still appends the enclosing
/// command's harness frame through the ordinary op-error path (probed
/// tclsh 8.6.17: `can't set "a": …` + `while executing "foreach …"` +
/// `(procedure "p" line N)`).
fn foreach_bind(
    interp: &mut Interp,
    code: &ByteCode,
    info: &rtcl_parser::bytecode::ForeachInfo,
    st: &VmState,
    idx: usize,
) -> Result<()> {
    let ff = st.foreaches.last().expect("foreach frame");
    // The dominant shape — one group, one variable, slot target — binds
    // through the frame slot directly, skipping the nested iteration (and
    // the empty-singleton's thread-local lookup, which only the
    // out-of-range branches need).
    if let [g] = info.groups.as_slice() {
        if let [t] = g.as_slice() {
            if let (Some(slot), [list]) = (t.slot, ff.lists.as_slice()) {
                let v = match list.get(idx) {
                    Some(v) => v,
                    None => &Value::empty(),
                };
                if interp.frame_slot_write(slot as usize, v) {
                    return Ok(());
                }
            }
        }
    }
    // Out-of-range targets (multi-group lists of uneven length) bind the
    // empty value — one shared instance per bind, not per variable.
    let empty = Value::empty();
    for (g, list) in info.groups.iter().zip(&ff.lists) {
        let n = g.len();
        for (vi, t) in g.iter().enumerate() {
            let v: &Value = list.get(idx * n + vi).unwrap_or(&empty);
            match t.slot {
                Some(s) if interp.frame_slot_write(s as usize, v) => {}
                _ => {
                    let name = code.get_const(t.name_idx).unwrap_or("");
                    if let Err(e) = interp.set_var(name, v.clone()) {
                        if interp.err_is_error(&e) {
                            super::commands::list::set_error_code(
                                interp,
                                "TCL WRITE VARNAME",
                            );
                            if info.framed {
                                // FRAMED unit: the dispatched foreach's
                                // decoration rides between the message
                                // and the command frame (tclsh top-level
                                // shape).
                                let what = if info.lmap { "lmap" } else { "foreach" };
                                let deco = format!(
                                    "\n    (setting {} loop variable \"{}\")",
                                    what,
                                    name
                                );
                                if interp.err_info.is_none() {
                                    interp.err_info = Some(e.message_text());
                                }
                                if let Some(info_s) = &mut interp.err_info {
                                    info_s.push_str(&deco);
                                }
                            }
                        }
                        return Err(e);
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch helpers
// ---------------------------------------------------------------------------

/// `Call` — a known built-in.  The resolution cache already holds the
/// winner `dispatch_values`'s full chain would produce for this exact
/// invocation name (a proc/ensemble/import shadowing the builtin simply
/// caches a different variant or nothing), so a cached `Builtin` IS the
/// Dispatch `st.scratch` for the call op at `st.pc - 1`: through the
/// unit's call-site token when it is still valid, else the full resolver
/// `full` — then backfill the token from what the full path cached.
///
/// The token is the fib profile's missing tclsh step: every compiled
/// `Call`/`DynCall` paid `cmd_cache_get` inside `dispatch_call` AND again
/// inside `dispatch_values` (two Fx hash probes) plus the
/// `dispatch_dynamic`/`exec_step_begin` layers before `call_proc`.  A
/// valid token skips all of it: the builtin fn / proc key sits on the op
/// (indexed by op pc, no hashing), gated by the command-table generation,
/// the invocation name, and the resolving namespace — the exact key
/// `cmd_cache` uses.  Ensemble/unknown/unresolved names never cache, so
/// they keep the full path (as before); execution traces route through
/// `dispatch_dynamic`'s bracketing.
fn dispatch_site(
    interp: &mut Interp,
    st: &mut VmState,
    full: fn(&mut Interp, &[Value]) -> Result<Value>,
) -> Result<Value> {
    // A1 step budget: the single choke point for every compiled command
    // dispatch (builtin fn-pointer fast path, cached proc token, and the
    // `full` fallback all pass through here — one charge per dispatch,
    // never doubled: `dispatch_values`/`call_proc` don't route through
    // the tree-walk's `eval_command`).
    interp.charge_step()?;
    let pc = (st.pc - 1) as u32;
    if interp.exec_traces.is_empty() {
        if let Some(slots) = st.cmd_sites.clone() {
            let name = st.scratch[0].as_str();
            let (hit, site_def) = {
                let slots = slots.borrow();
                match slots.binary_search_by_key(&pc, |(p, _)| *p) {
                    Ok(i) => {
                        let s = &slots[i].1;
                        (
                            (s.gen == interp.cmd_generation
                                && s.name.as_ref() == name
                                && (std::rc::Rc::ptr_eq(&s.ns, &interp.current_namespace)
                                    || s.ns.as_ref() == interp.current_namespace.as_ref()))
                                .then(|| s.target.clone()),
                            s.def.clone(),
                        )
                    }
                    Err(_) => (None, None),
                }
            };
            match hit {
                Some(super::ResolvedCmd::Builtin(f)) => {
                    // dispatch_values' prologue + dispatch_call's builtin
                    // arm, verbatim: fresh error state, depth bracket,
                    // wrong-#-args post-fill.
                    interp.err_code_raised = false;
                    interp.call_depth += 1;
                    let r = f(interp, &st.scratch);
                    interp.call_depth -= 1;
                    return interp.fill_wrong_args(name, r);
                }
                Some(super::ResolvedCmd::Proc(key)) => {
                    // The def rides the token: every `procs` mutation —
                    // the statics write-backs included, which was the one
                    // unbumped gap — increments `cmd_generation`, and this
                    // site's gen check already ran above, so the carried
                    // def IS the live map entry.
                    if let Some(def) = site_def {
                        interp.err_code_raised = false;
                        return interp.call_proc(&def, &st.scratch, &key, None);
                    }
                    // A token from the pre-def transition window: fetch
                    // once, and the next backfill rewrites the slot.
                    if let Some(def) = interp.procs.get(key.as_ref()).cloned() {
                        interp.err_code_raised = false;
                        return interp.call_proc(&def, &st.scratch, &key, None);
                    }
                }
                None => {}
            }
        }
    }
    let name = st.scratch[0].as_str();
    let r = full(interp, &st.scratch);
    // Backfill the token from the resolution the full path just cached
    // (proc/builtin wins only — matching cmd_cache's policy).
    if interp.exec_traces.is_empty() {
        if let Some(target) = interp.cmd_cache_get(name) {
            // A proc verdict carries its def: the probe runs on the miss
            // path only (cold), while every hit skips it forever.
            let def = match &target {
                super::ResolvedCmd::Proc(key) => interp.procs.get(key.as_ref()).cloned(),
                _ => None,
            };
            if let Some(slots) = &st.cmd_sites {
                let site = super::CmdSite {
                    gen: interp.cmd_generation,
                    name: Rc::from(name),
                    ns: Rc::clone(&interp.current_namespace),
                    target,
                    def,
                };
                let mut v = slots.borrow_mut();
                match v.binary_search_by_key(&pc, |(p, _)| *p) {
                    Ok(i) => v[i].1 = site,
                    Err(i) => v.insert(i, (pc, site)),
                }
            }
        }
    }
    r
}

/// dispatch outcome: call it directly.  Everything else — cache miss
/// (first call, ensemble/unknown resolution), a cached `Proc`, or
/// execution traces needing `dispatch_dynamic`'s bracketing — takes the
/// exact full dispatch.  `cmd_id` is no longer consulted: the cache is
/// keyed by the invoked name, a strictly stronger identity check than
/// the `CmdId` string match it replaced.
fn dispatch_call(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if interp.exec_traces.is_empty() {
        let hit = match interp.cmd_cache_get(args[0].as_str()) {
            Some(super::ResolvedCmd::Builtin(f)) => Some(f),
            _ => None,
        };
        if let Some(f) = hit {
            let name = args[0].as_str();
            interp.call_depth += 1;
            let r = f(interp, args);
            interp.call_depth -= 1;
            return interp.fill_wrong_args(name, r);
        }
    }
    dispatch_dynamic(interp, args)
}

/// Full dispatch — `dispatch_values` with the execution-step trace
/// bracketing `eval_command` provides.
fn dispatch_dynamic(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let step = interp.exec_step_begin(args);
    let r = interp.dispatch_values(args);
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

/// Read slot `slot` with LoadLocal's exact fallback chain: the cell when
/// set, `eval_var_ref` on the table name otherwise (globals fallback,
/// linked-slot resolution, canonical errors — all owned by the name path).
#[inline(always)]
fn slot_read_or_name(interp: &mut Interp, code: &ByteCode, slot: usize) -> Result<Value> {
    if let Some(f) = interp.frames.last() {
        if !(f.has_aliased && f.slot_aliased.get(slot).copied().unwrap_or(false)) {
            if let Some(v) = f.slots.get(slot) {
                if v.is_unset() {
                    // Unset cell: LoadLocal falls through to the name path —
                    // which for an unset table name errors identically.
                    let name = code.locals().get(slot).map(String::as_str).unwrap_or("");
                    return interp.eval_var_ref(name);
                }
                return Ok(v.clone());
            }
        }
    }
    let name = code.locals().get(slot).map(String::as_str).unwrap_or("");
    interp.eval_var_ref(name)
}

/// The comparison superinstruction's semantic: `op_rel` for the four
/// relational operators (numeric-then-string, NaN rules included), plus
/// eq/ne (numeric equality, then string equality — op_eq/op_ne's own
/// contract).
#[inline(always)]
fn cmp_values(a: &Value, b: &Value, cc: u8) -> Value {
    match cc {
        0 => crate::types::expr_ops::op_rel(a, b, "<"),
        1 => crate::types::expr_ops::op_rel(a, b, ">"),
        2 => crate::types::expr_ops::op_rel(a, b, "<="),
        3 => crate::types::expr_ops::op_rel(a, b, ">="),
        4 => crate::types::expr_ops::op_eq(a, b),
        _ => crate::types::expr_ops::op_ne(a, b),
    }
}

#[inline]
fn pop_pair(st: &mut VmState) -> (Value, Value) {
    let b = st.pop_val();
    let a = st.pop_val();
    (a, b)
}

#[inline]
fn arith(st: &mut VmState, op: char) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::numeric_binop(&a, &b, op)?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn rel(st: &mut VmState, op: &'static str) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::op_rel(&a, &b, op);
    st.stack.push(v);
    Ok(())
}

#[inline]
fn bits(st: &mut VmState, op: char) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::int_bitop(&a, &b, op)?;
    st.stack.push(v);
    Ok(())
}

#[inline]
fn shift(st: &mut VmState, shl: bool) -> Result<()> {
    let (a, b) = pop_pair(st);
    let v = crate::types::expr_ops::int_shift(&a, &b, shl)?;
    st.stack.push(v);
    Ok(())
}

#[cfg(test)]
mod sub_inline_tests {
    use crate::interp::Interp;

    fn ev(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    #[test]
    fn test_sub_inline_values() {
        // Whole-word brackets compile inline (SubMark/SubEnd) — values,
        // nesting and empties all agree with the tree-walk.
        assert_eq!(ev("set x [expr {6 * 7}]; set x"), "42");
        // tclsh: an empty bracket substitutes to the empty command name.
        assert_eq!(
            ev("catch {set x [[]]} m; set m"),
            "invalid command name \"\""
        );
        assert_eq!(ev("set x [list]; set x"), "");
        assert_eq!(
            ev("proc f n { expr {$n + 1} }; set x [f [f [f 0]]]; set x"),
            "3"
        );
        // The bracket result is the LAST command's value; earlier results
        // are discarded like any script.
        assert_eq!(ev("set x [list a; list b]; set x"), "b");
    }

    /// The fused `[cmp, jcc]` superinstruction must not skip the op that
    /// follows the pair: the fused op occupies the pair's head slot, so
    /// the not-taken fall-through is `st.pc` itself (a stray `pc + 1` once
    /// swallowed the body's first `BeginCmd`, making every body error look
    /// like the construct's own — the inner command frames vanished from
    /// errorInfo; caught by judge gen/gen_proc-old).
    #[test]
    fn fused_branch_fallthrough_keeps_site() {
        let mut interp = Interp::new();
        interp
            .eval(
                "proc t {} {\n    set x 44\n    while {$x < 100} {\n        error \"boom\"\n    }\n}",
            )
            .unwrap();
        interp.eval("catch {t} m").unwrap();
        let info = interp
            .eval("set ::errorInfo")
            .unwrap()
            .as_str()
            .to_string();
        assert!(info.contains("error \"boom\""), "inner frame missing: {info}");
        assert!(
            info.contains("(procedure \"t\" line 4)"),
            "procedure frame missing: {info}"
        );
    }

    #[test]
    fn test_sub_inline_expr_partial_rollback() {
        // `1 && 0` makes the expression compiler emit PushInt(1) and THEN
        // bail on `&&`.  Inline in a word, the orphan would misalign the
        // stack (invalid command name "1") — the compiler must roll back
        // to the EvalExpr fallback.
        assert_eq!(ev("set q [expr {1 && 0}]; set q"), "0");
        // Tcl logical ops normalise to 1/0.
        assert_eq!(ev("set q [expr {0 || 7}]; set q"), "1");
        assert_eq!(ev("set q [expr {1 && 0}]; set q"), "0");
    }

    #[test]
    fn test_sub_inline_error_framing() {
        // An error inside an inlined bracket gets the failing command's
        // frame plus the deferral of the bracket-owning command — the
        // tree-walk's CommandSub word boundary.
        let mut interp = Interp::new();
        let r = interp.eval("set x [nosuch a]");
        assert!(r.is_err());
        let info = interp.err_info.take().unwrap_or_default();
        assert!(info.contains("\"nosuch a\""), "inner frame in {info}");
        assert_eq!(interp.err_pending_top.as_deref(), Some("set x [nosuch a]"));

        // Nested: the outermost bracket's deferral wins.
        let mut interp = Interp::new();
        let r = interp.eval("set x [string length [nosuch a]]");
        assert!(r.is_err());
        assert_eq!(
            interp.err_pending_top.as_deref(),
            Some("set x [string length [nosuch a]]")
        );
    }

    #[test]
    fn test_expr_inline_values() {
        // Brackets as expression operands compile inline — values agree
        // with the tree-walk, including nesting and the fib recursion.
        assert_eq!(ev("set a [expr {[expr {6 * 7}] + 1}]; set a"), "43");
        assert_eq!(ev("proc f x { expr {$x + 10} }; set d [expr {[f 5] + [f 6]}]; set d"), "31");
        assert_eq!(ev("proc f x { expr {$x + 10} }; set e [expr {-[f 6]}]; set e"), "-16");
        assert_eq!(
            ev("proc h {} { list 1 2 }; set i [expr {[llength [h]] == 2}]; set i"),
            "1"
        );
        assert_eq!(
            ev("proc fib n { if {$n < 2} { return $n }; expr {[fib [expr {$n - 1}]] + [fib [expr {$n - 2}]]} }; fib 12"),
            "144"
        );
        // A condition's brackets re-run every iteration (while/if).
        assert_eq!(
            ev("set L {}; set n 0; while {[llength $L] < 3} { lappend L [incr n] }; set L"),
            "1 2 3"
        );
        // Control flow through expr-embedded brackets.
        assert_eq!(
            ev("set hits 0; for {set i 0} {$i < 3} {incr i} { set t [expr {[continue] + 1}]; incr hits }; set hits"),
            "0"
        );
        assert_eq!(ev("proc r {} { expr {1 + [return 42]} }; r"), "42");
    }

    #[test]
    fn test_expr_inline_error_framing() {
        // An error crossing an expr-operand bracket APPENDS the owning
        // command's frame (eval_expr's bracket is a plain nested eval —
        // no err_from_subst), unlike a word bracket, which defers.
        let mut interp = Interp::new();
        let r = interp.eval("expr {[nosuch a]}");
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().message_text(), "invalid command name \"nosuch\"");
        let info = interp.err_info.take().unwrap_or_default();
        assert!(info.contains("\"nosuch a\""), "inner frame in {info}");
        assert!(info.contains("\"expr {[nosuch a]}\""), "appended expr frame in {info}");

        // Mixed: word bracket defers, expr bracket inside it appends.
        let mut interp = Interp::new();
        let r = interp.eval("set x [expr {[nosuch a]}]");
        assert!(r.is_err());
        let info = interp.err_info.take().unwrap_or_default();
        assert!(info.contains("\"nosuch a\""), "inner frame in {info}");
        assert!(info.contains("\"expr {[nosuch a]}\""), "expr frame in {info}");
        assert_eq!(interp.err_pending_top.as_deref(), Some("set x [expr {[nosuch a]}]"));

        // Transparent-body suppression: `if`'s body error adds no if frame;
        // the expr frame still appends (both engines agree, verified vs
        // tclsh modulo the recorded expr-frame divergence).
        let mut interp = Interp::new();
        let r = interp.eval("expr {1 + [if {1} {error mid}]}");
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().message_text(), "mid");
        let info = interp.err_info.take().unwrap_or_default();
        assert!(info.contains("\"error mid\""), "inner frame in {info}");
        assert!(info.contains("\"expr {1 + [if {1} {error mid}]}\""), "expr frame in {info}");
        assert!(!info.contains("\"if {1} {error mid}\""), "no if frame: {info}");
    }

    #[test]
    fn test_if_arm_body_line_rebase() {
        // tclsh numbers an if arm against the enclosing script's single
        // line table: with the `if` on the proc body's line 2, an else-
        // body error reports `(procedure "b1" line 2)` — not the body
        // text's own line 1.
        let mut interp = Interp::new();
        let r = interp.eval(
            "proc b1 {} {\n    if {0} { puts a } else { nosuch }\n}\ncatch {b1} m\nset ::errorInfo",
        );
        let info = r.unwrap().as_str().to_string();
        assert!(info.contains("(procedure \"b1\" line 2)"), "line 2 in {info}");
        assert!(info.contains("\"nosuch\""), "inner frame in {info}");
    }

    #[test]
    fn test_control_flow_does_not_leak_fresh() {
        // A control-flow completion out of an if arm (the `continue`) must
        // not arm the harness-frame suppression: the NEXT error in the
        // interp keeps all of its frames — here both expr frames of the
        // nested-bracket error (expr_bracket_control's `deep` case).
        let mut interp = Interp::new();
        let r = interp.eval(concat!(
            "set out {}\n",
            "for {set i 0} {$i < 4} {incr i} {\n",
            "    if {[expr {$i % 2}] == 0} { lappend out even-$i ; continue }\n",
            "    lappend out odd-$i\n",
            "}\n",
            "proc deep n { expr {[expr {[nosuch $n]}] + 1} }\n",
            "catch {deep 3} m\nset ::errorInfo\n",
        ));
        let info = r.unwrap().as_str().to_string();
        assert!(
            info.contains("\"expr {[nosuch $n]}\""),
            "inner expr frame present: {info}"
        );
        assert!(
            info.contains("\"expr {[expr {[nosuch $n]}] + 1} "),
            "outer expr frame present: {info}"
        );
    }

    #[test]
    fn test_incr_fast_paths() {
        // incr mutates the int rep in place; the string rendering follows.
        assert_eq!(ev("set x 5; incr x; set x"), "6");
        assert_eq!(ev("set x 1000000; incr x 41; set x"), "1000041");
        // Undefined variable starts at 0 (real incr via fallback).
        assert_eq!(ev("incr fresh; set fresh"), "1");
        // Non-integer value: the real incr's error.
        assert_eq!(
            ev("set s abc; catch {incr s} m; set m"),
            "expected integer but got \"abc\""
        );
        // Array element: qualified shape, fallback to the real incr.
        assert_eq!(ev("set a(1) 7; incr a(1) 2; set a(1)"), "9");
        // Lazy ints still render exactly.
        assert_eq!(ev("set x [expr {1000000 + 2}]; set x"), "1000002");
        assert_eq!(ev("set x -5; set x"), "-5");
    }
}

#[cfg(test)]
mod foreach_inline_tests {
    //! Item D — foreach/lmap inlined into compiled proc bodies
    //! (tclsh's compiledLocals shape).  The tree-walk twin (lexical
    //! arm) runs under RTCL_NO_BYTECODE in the external sweep; these
    //! in-process cases pin the bytecode engine's compiled shape.

    use crate::interp::Interp;

    fn ev(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    fn caught(script: &str) -> (String, String, String) {
        // (message, errorCode, errorInfo) of the caught error.
        let mut interp = Interp::new();
        let r = interp
            .eval(&format!("catch {{{script}}} m; list $m $::errorCode $::errorInfo"))
            .unwrap();
        let list = r.as_list().unwrap();
        (
            list[0].as_str().to_string(),
            list[1].as_str().to_string(),
            list[2].as_str().to_string(),
        )
    }

    #[test]
    fn test_foreach_inline_values() {
        assert_eq!(
            ev("proc p {} { set r {}; foreach i {a b c} { lappend r $i }; return $r }; p"),
            "a b c",
        );
        // Multi-var groups and ragged tails (missing elements read empty).
        assert_eq!(
            ev("proc p {} { set r {}; foreach {x y} {1 2 3} { lappend r $x-$y }; return $r }; p"),
            "1-2 3-",
        );
        // Two groups in lockstep.
        assert_eq!(
            ev("proc p {} { set r {}; foreach a {1 2} b {x y} { lappend r $a$b }; return $r }; p"),
            "1x 2y",
        );
        // Empty data list: zero iterations, empty result.
        assert_eq!(
            ev("proc p {} { set hit 0; foreach i {} { incr hit }; return $hit }; p"),
            "0",
        );
        // Empty body word inlines too.
        assert_eq!(
            ev("proc p {} { foreach i {1 2} {}; return done }; p"),
            "done",
        );
        // The loop variable binds a slot the rest of the body reads.
        assert_eq!(
            ev("proc p {} { set s 0; foreach i {1 2 3 4} { incr s $i }; return $s }; p"),
            "10",
        );
        // Nested inline foreach.
        assert_eq!(
            ev("proc p {} { set r {}; foreach a {1 2} { foreach b {x y} { lappend r $a$b } }; return $r }; p"),
            "1x 1y 2x 2y",
        );
    }

    #[test]
    fn test_foreach_inline_break_continue() {
        assert_eq!(
            ev("proc p {} { set r {}; foreach i {1 2 3 4} { if {$i == 3} break; lappend r $i }; return $r }; p"),
            "1 2",
        );
        assert_eq!(
            ev("proc p {} { set r {}; foreach i {1 2 3 4} { if {$i == 2} continue; lappend r $i }; return $r }; p"),
            "1 3 4",
        );
        // Level-2 break passes the loop through: catch reports code 2 and
        // the result variable stays empty (probed tclsh: p=2, resc=,,).
        assert_eq!(
            ev("proc p {} { set c [catch {foreach i {1} { return -code break -level 2 }}]; return $c }; p"),
            "2",
        );
        assert_eq!(
            ev("proc q {} { catch {foreach i {1} { return -code break -level 2 }} c; return ,$c, }; q"),
            ",,",
        );
        // return unwinds the proc with its value.
        assert_eq!(
            ev("proc p {} { foreach i {1 2} { return $i } }; p"),
            "1",
        );
    }

    #[test]
    fn test_lmap_inline_values() {
        assert_eq!(
            ev("proc p {} { lmap i {1 2 3} { expr {$i * 2} } }; p"),
            "2 4 6",
        );
        // break keeps the results collected so far; continue skips one.
        assert_eq!(
            ev("proc p {} { lmap i {1 2 3 4} { if {$i == 2} break; expr {$i * 10} } }; p"),
            "10",
        );
        assert_eq!(
            ev("proc p {} { lmap i {1 2 3} { if {$i == 2} continue; expr {$i * 10} } }; p"),
            "10 30",
        );
        // return -level 0 completes as an ordinary command: collected.
        assert_eq!(
            ev("proc p {} { lmap i {1 2} { return -level 0 v$i } }; p"),
            "v1 v2",
        );
        // lmap with a multi-var group.
        assert_eq!(
            ev("proc p {} { lmap {a b} {1 2 3 4} { expr {$a + $b} } }; p"),
            "3 7",
        );
    }

    #[test]
    fn test_foreach_inline_error_framing() {
        // Compiled shape (probed tclsh): the body error names no foreach
        // frame — innermost command frame + absolute procedure line.
        let (m, _c, info) = caught(
            "proc p {} {\n    foreach x {1 2 3} {\n        nosuchcmd\n    }\n}\np",
        );
        assert_eq!(m, "invalid command name \"nosuchcmd\"");
        assert!(info.contains("\"nosuchcmd\""), "inner frame in {info}");
        assert!(info.contains("(procedure \"p\" line 3)"), "absolute line in {info}");
        assert!(!info.contains("\"foreach\" body"), "no body frame: {info}");
        assert!(!info.contains("\"foreach x"), "no foreach frame: {info}");

        // Var-write failure in compiled context: PLAIN (no decoration) but
        // with the foreach command's harness frame + TCL WRITE VARNAME.
        let (m, c, info) = caught(
            "proc p {} {\n    set a(0) 44\n    foreach a {1 2 3} { set x 1 }\n}\np",
        );
        assert_eq!(m, "can't set \"a\": variable is array");
        assert_eq!(c, "TCL WRITE VARNAME");
        assert!(info.contains("\"foreach a {1 2 3} { set x 1 }\""), "cmd frame in {info}");
        assert!(info.contains("(procedure \"p\" line 3)"), "line 3 in {info}");
        assert!(!info.contains("(setting foreach loop variable"), "no decoration: {info}");
    }

    #[test]
    fn test_foreach_dispatched_error_framing() {
        // Top level (no compiledLocals): the dispatched shape — body exit
        // frame + decoration, body-relative lines (foreach-1.14 shape).
        let (m, c, info) = caught("unset -nocomplain a; set a(0) 44; foreach a {1 2 3} {}");
        assert_eq!(m, "can't set \"a\": variable is array");
        assert_eq!(c, "TCL WRITE VARNAME");
        assert!(info.contains("(setting foreach loop variable \"a\")"), "decoration in {info}");

        let (m, _c, info) = caught("foreach i {1 2 3} { nosuchcmd }");
        assert_eq!(m, "invalid command name \"nosuchcmd\"");
        assert!(info.contains("(\"foreach\" body line 1)"), "exit frame in {info}");
        assert!(info.contains("\"foreach i {1 2 3} { nosuchcmd }\""), "harness in {info}");
    }

    #[test]
    fn test_lmap_dispatched_error_framing() {
        // The dispatched-lmap fixes: decoration + errorCode, body exit
        // frame, loop-level handling (probed tclsh 8.6.17).
        let (m, c, info) = caught("unset -nocomplain a; set a(0) 44; lmap a {1 2 3} {set x 1}");
        assert_eq!(m, "can't set \"a\": variable is array");
        assert_eq!(c, "TCL WRITE VARNAME");
        assert!(info.contains("(setting lmap loop variable \"a\")"), "decoration in {info}");

        let (m, _c, info) = caught("lmap i {1 2 3} { nosuchcmd }");
        assert_eq!(m, "invalid command name \"nosuchcmd\"");
        assert!(info.contains("(\"lmap\" body line 1)"), "exit frame in {info}");
        assert!(info.contains("\"lmap i {1 2 3} { nosuchcmd }\""), "harness in {info}");

        // A level-2 break escapes the dispatched lmap (catch sees 2), and
        // the multi-line body frame counts body-relative lines.
        let (_m, _c, info) = caught("lmap i {1 2} {\n    set t $i\n    nosuchcmd2\n}");
        assert!(info.contains("(\"lmap\" body line 3)"), "body-relative line in {info}");
    }

    #[test]
    fn test_foreach_slots_and_degrade() {
        // The loop variable lands in a slot the compiled body reads; a
        // later `set` of the same name sees the same storage.
        assert_eq!(
            ev("proc p {} { set last 0; foreach i {5 6 7} { set last $i }; return $last }; p"),
            "7",
        );
        // upvar to a loop variable degrades the frame: iteration still
        // writes through the name path.
        assert_eq!(
            ev("proc inner {n} { upvar 1 $n s; set s 99 }\nproc p {} { set r {}; foreach i {1 2} { inner i; lappend r $i }; return $r }; p"),
            "99 99",
        );
        // unset mid-loop recreates the variable on the next binding.
        assert_eq!(
            ev("proc p {} { set r {}; foreach i {1 2 3} { lappend r [info exists i]; unset i }; return $r }; p"),
            "1 1 1",
        );
        // Non-candidate loop names (qualified) bind through set_var.
        assert_eq!(
            ev("proc p {} { set r {}; foreach ::g {1 2} { lappend r $::g }; return $r }; p"),
            "1 2",
        );
    }

    #[test]
    fn test_foreach_in_noninline_shapes_stay_dispatched() {
        // Quoted body word: the compiler requires braces-only verbatim —
        // dispatched, so the body error gains the exit frame.
        let mut interp = Interp::new();
        let r = interp
            .eval("proc p {} { foreach i {1} \"nosuchcmd\" }\ncatch {p} m\nset ::errorInfo")
            .unwrap();
        let info = r.as_str();
        assert!(info.contains("(\"foreach\" body line 1)"), "dispatched shape: {info}");
        // A body from a variable: dispatched as well.
        let mut interp = Interp::new();
        let r = interp
            .eval("proc p {b} { foreach i {1} $b }\ncatch {p nosuchcmd} m\nset ::errorInfo")
            .unwrap();
        let info = r.as_str();
        assert!(info.contains("(\"foreach\" body line 1)"), "var body dispatched: {info}");
    }

    #[test]
    fn test_catch_body_shapes_by_context() {
        // tclsh inlines a braces-only catch body into proc-context units
        // (procs, lambdas) and leaves it a dispatched unit everywhere
        // else.  Probed shapes:
        // - dispatched: `(setting lmap loop variable …)` + `invoked
        //   from within`;
        // - inlined: plain write + `while executing` (the compiled
        //   var-write frame).
        assert_eq!(
            ev("set a(0) 44; catch {lmap a {1 2 3} {}} m; set ::errorInfo"),
            "can't set \"a\": variable is array\n    (setting lmap loop variable \"a\")\n    invoked from within\n\"lmap a {1 2 3} {}\"",
        );
        assert_eq!(
            ev("proc p {} { set a(0) 44; catch {lmap a {1 2 3} {}} m; set ::errorInfo }; p"),
            "can't set \"a\": variable is array\n    while executing\n\"lmap a {1 2 3} {}\"",
        );
        // Apply bodies compile like proc bodies — same inlined shape.
        assert_eq!(
            ev("apply {{} { set a(0) 44; catch {lmap a {1 2 3} {}} m; set ::errorInfo }}"),
            "can't set \"a\": variable is array\n    while executing\n\"lmap a {1 2 3} {}\"",
        );
        // An eval'd string inside a proc is a fresh unit — the catch
        // body inside it is dispatched again.
        assert_eq!(
            ev("proc p {} { set a(0) 44; eval {catch {lmap a {1 2 3} {}} m}; set ::errorInfo }; p"),
            "can't set \"a\": variable is array\n    (setting lmap loop variable \"a\")\n    invoked from within\n\"lmap a {1 2 3} {}\"",
        );
        // A non-braces-only body word (quoted / from a variable) stays
        // dispatched even inside a proc.
        assert_eq!(
            ev("proc p {} { set a(0) 44; catch \"lmap a {1 2 3} {}\" m; set ::errorInfo }; p"),
            "can't set \"a\": variable is array\n    (setting lmap loop variable \"a\")\n    invoked from within\n\"lmap a {1 2 3} {}\"",
        );
        assert_eq!(
            ev("proc p {b} { set a(0) 44; catch $b m; set ::errorInfo }; p {lmap a {1 2 3} {}}"),
            "can't set \"a\": variable is array\n    (setting lmap loop variable \"a\")\n    invoked from within\n\"lmap a {1 2 3} {}\"",
        );
    }

    #[test]
    fn test_apply_lambda_compiles() {
        // tclsh compiles lambda bodies like proc bodies; the inline
        // foreach/lmap now applies to them (defaults, args, slots).
        assert_eq!(
            ev("apply {{x} { set r {}; foreach i $x { lappend r [expr {$i * 2}] }; return $r }} {1 2 3}"),
            "2 4 6",
        );
        assert_eq!(ev("apply {{} { lmap i {1 2 3} { expr {$i * $i} } }}"), "1 4 9");
        assert_eq!(ev("apply {{a {b 10}} { expr {$a + $b} }} 5"), "15");
        assert_eq!(ev("apply {{args} { llength $args }} a b c"), "3");
        // The lambda boundary frame survives compilation.
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {apply {{} { nosuchcmd }}} m\nset ::errorInfo")
            .unwrap();
        let info = r.as_str().to_string();
        assert!(info.contains("\"nosuchcmd\""), "inner frame in {info}");
        assert!(
            info.contains("(lambda term \"{} { nosuchcmd }\" line 1)"),
            "lambda frame in {info}"
        );
        // A non-parsing lambda body keeps the per-call parse-error path.
        assert_eq!(
            ev("catch {apply {{} { set x \"unterm}} } m; set m"),
            "missing \""
        );
    }

    #[test]
    fn test_foreach_inside_loop_constructs() {
        // A foreach in an inlined while/if body of a compiled proc is part
        // of the same unit (compiled shape); in a while with a non-literal
        // condition the body is a nested unit (dispatched shape).
        let mut interp = Interp::new();
        let r = interp
            .eval(concat!(
                "proc a {} { set c 1; while $c { foreach i {1} { nosuchcmd } ; set c 0 } }\n",
                "catch {a} m\nset ::errorInfo\n",
            ))
            .unwrap();
        assert!(r.as_str().contains("(\"foreach\" body line 1)"), "nested-unit dispatched: {}", r.as_str());

        let mut interp = Interp::new();
        let r = interp
            .eval(concat!(
                "proc b {} { if {1} { foreach i {1} { nosuchcmd } } }\n",
                "catch {b} m\nset ::errorInfo\n",
            ))
            .unwrap();
        let info = r.as_str();
        assert!(info.contains("\"nosuchcmd\""), "inner frame in {info}");
        assert!(!info.contains("\"foreach\" body"), "same-unit compiled: {info}");
        assert!(info.contains("(procedure \"b\" line 1)"), "abs line in {info}");
    }

}

#[cfg(test)]
mod slot_locals_tests {
    //! E2 slot-resolved proc locals: slots are the canonical store of the
    //! compiled locals table's names; every name-keyed path aliases them.

    use crate::interp::Interp;

    fn ev(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    fn assert_both(script: &str, want: &str) {
        // (named assert_both for the historical two-engine form; the
        // tree-walk half lives in the external sweep — env-var toggling
        // here would race the OnceLock under parallel tests)
        assert_eq!(ev(script), want, "{script}");
    }

    #[test]
    fn test_slot_basics() {
        assert_both("proc p {a b} { set c [expr {$a + $b}] ; incr c ; return $c }; p 1 2", "4");
        assert_both("proc q {} { set x 5; set x }; q", "5");
        assert_both("proc r {a} { set a 9; return $a }; r 1", "9");
        assert_both("proc s {x args} { return $x|[llength $args] }; s 1 2 3", "1|2");
        assert_both("proc d {a {b 7}} { return $a+$b }; d 1", "1+7");
    }

    #[test]
    fn test_slot_use_before_set_error() {
        assert_both(
            "proc p {} { puts $never }; catch p m; set m",
            "can't read \"never\": no such variable",
        );
        assert_both(
            "proc p {a} { unset a; set a }; catch {p 1} m; set m",
            "can't read \"a\": no such variable",
        );
    }

    #[test]
    fn test_slot_uncompiled_writer_aliasing() {
        // foreach writes a table name through the name path.
        assert_both(
            "proc f {} { set total 0; foreach i {1 2 3} { incr total $i }; return $total }; f",
            "6",
        );
        // catch stores its result into a table name.
        assert_both(
            "proc c {} { set msg old; catch {error boom} msg; return $msg }; c",
            "boom",
        );
        // lappend mutates a table name.
        assert_both("proc l {} { set v a; lappend v b; return $v }; l", "a b");
        // append / unset / recreate cycles.
        assert_both("proc a {} { set v 1; unset v; set v 2; return $v }; a", "2");
        // info exists sees slot state.
        assert_both("proc e {x} { return [info exists x] }; e 1", "1");
        assert_both(
            "proc e2 {x} { unset x; return [info exists x] }; e2 1",
            "0",
        );
        // expr reads the slot (loop-condition shape).
        assert_both(
            "proc w {} { set i 0; while {$i < 4} { incr i }; return $i }; w",
            "4",
        );
    }

    #[test]
    fn test_slot_degrade_upvar() {
        // upvar to a caller local: both frames observe one variable.
        assert_both(
            "proc inner {n} { upvar 1 $n s; incr s }; proc outer {} { set shared 41; inner shared; return $shared }; outer",
            "42",
        );
        // upvar onto an existing local name errors (exists check consults
        // the migrated map).
        assert_both(
            "proc p {} { set x 1; upvar 0 y x }; catch p m; set m",
            "variable \"x\" already exists",
        );
    }

    #[test]
    fn test_slot_degrade_global() {
        assert_both(
            "set g 10; proc p {} { global g; incr g; return $g }; p; set g",
            "11",
        );
        // A global'd name that was already written compiled: the degrade
        // migrates the prior slot value into the link-visible store.
        assert_both(
            "proc p {} { set g2 3; global g2; return $g2 }; p",
            "3",
        );
    }

    #[test]
    fn test_slot_degrade_array() {
        assert_both(
            "proc p {} { set a(0) x; return [array exists a] }; p",
            "1",
        );
        // Element read/write through the map after array-ification.
        assert_both("proc p {} { set a(0) 1; set a(1) 2; return $a(0)$a(1) }; p", "12");
    }

    #[test]
    fn test_slot_degrade_trace() {
        assert_both(
            "proc p {} { set t 1; trace add variable t write {apply {args {set ::hit 1}}}; set t 2; return [list $t [info exists ::hit]] }; p",
            "2 1",
        );
    }

    #[test]
    fn test_slot_enumeration_degrades() {
        assert_both(
            "proc p {a b} { set loc1 1; set loc2 2; lsort [info locals] }; p 1 2",
            "a b loc1 loc2",
        );
        assert_both(
            "proc p {z} { set w 9; lsort [info vars] }; p 5",
            "w z",
        );
        assert_both(
            "proc p {q} { set inner 3; info frame 0 }; p 9",
            "type proc level 0 cmd {} locals {q inner}",
        );
    }

    #[test]
    fn test_slot_tailcall_rebind() {
        // The tailcall loop re-gates per iteration; slot binding follows
        // the new proc's table.
        assert_both(
            "proc tc {n acc} { if {$n <= 0} { return $acc }; tailcall tc [expr {$n - 1}] [expr {$n + $acc}] }; tc 5 0",
            "15",
        );
        assert_both(
            "proc a {n} { if {$n <= 0} { return a }; tailcall b [expr {$n - 1}] }; proc b {n} { if {$n <= 0} { return b }; tailcall a [expr {$n - 1}] }; a 4",
            "a",
        );
    }

    #[test]
    fn test_slot_epoch_fallback() {
        // Shadowing an inline-folded command bumps the epoch: the compiled
        // body goes stale and the call runs tree-walked, where the frame
        // is name-keyed (slot gate consults the same epoch) and the body's
        // slot ops take their name fallbacks.  The shadow forwards to the
        // renamed builtin, so the observable result is unchanged.
        let script = "proc p {a} { set b [expr {$a + 1}]; return $b }; \
rename set myset; \
proc set {args} { uplevel 1 [linsert $args 0 myset] }; \
p 1";
        assert_eq!(ev(script), "2");
    }

    #[test]
    fn test_slot_statics_stay_name_keyed() {
        assert_both(
            "proc p {} { set c 5; return $c }; p",
            "5",
        );
    }

    #[test]
    fn test_slot_duplicate_param_names_stay_name_keyed() {
        // Duplicate parameter names dedup in the compiled locals table
        // (params {old - -} → table len 2), so the positional slot binding
        // would write past the table: the slot gate must detect the
        // misalignment and keep the frame name-keyed (tclsh rejects such
        // procs at definition; rtcl accepts them with map semantics).
        assert_both(
            "proc cb {old - -} { return [list $old [info level 0]] }; cb a b c",
            "a {cb a b c}",
        );
        // rtcl's map binding is last-wins for a readable duplicate (tclsh
        // 8.6.17 is first-wins — a recorded divergence class, not slot
        // business); both engines must agree on it.
        assert_both("proc dq {x x} { return $x }; dq 7 8", "8");
        // (Non-candidate formal names — qualified, array-element, empty —
        // are rejected at `proc` definition in both rtcl and tclsh, so
        // duplicates are the only reachable misalignment; the gate's zip
        // alignment also covers them defensively, e.g. for apply lambdas.)
    }

    #[test]
    fn test_slot_exotic_param_names() {
        // Non-candidate parameter names bind through the map; the frame
        // forgoes slots entirely and stays correct.
        assert_both("proc p {a b} { return [list $a $b] }; p 1 2", "1 2");
        assert_both(
            "proc p {n} { set n [expr {$n * 2}]; return $n }; p 21",
            "42",
        );
    }

    #[test]
    fn test_for_tail_fusion_continue_landing() {
        // The for-tail fusion (ForNextIncrLocal/Var) owns the next-script
        // region head — a `continue` in the body now lands ON the fused
        // op, which must run the next script and take the backedge with
        // the same step-count charging as the 3-op form.
        assert_both(
            "proc p {} { set acc {}; for {set i 0} {$i < 5} {incr i} { \
             if {$i == 1 || $i == 3} continue; lappend acc $i }; return $acc }; p",
            "0 2 4",
        );
        // Same shape with a variable-name next script (top-level loop →
        // name-keyed incr → ForNextIncrVar).
        assert_both(
            "set acc {}; for {set i 0} {$i < 4} {incr i} { if {$i == 2} continue; lappend acc $i }; set acc",
            "0 1 3",
        );
    }
}
