//! Compiled bytecode representation.
//!
//! A [`ByteCode`] object holds everything needed to execute a compiled Tcl
//! script: a constant pool, an instruction list, local-variable names, and
//! source-line mappings for diagnostics.

use crate::opcode::OpCode;
use core::fmt;
use std::rc::Rc;

/// Result of a constant-folding operation.
enum FoldResult {
    Int(i64),
    Bool(bool),
}

/// Byte span into a compilation unit's source text (see [`ByteCode::source`]).
/// Copy-only; resolving to `&str` is `span.slice(&code.source)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SrcSpan {
    pub start: u32,
    pub end: u32,
}

impl SrcSpan {
    #[inline]
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start as usize..self.end as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Source site of one compiled command — what the interpreter's errorInfo
/// harness needs per dispatched command: the command's text as written,
/// its line (already rebased to the unit's absolute numbering for inline
/// bodies), and each word's raw source (braced bodies keep delimiters, so
/// loop constructs can recover a body's line offset).  Spans resolve
/// against [`ByteCode::source`].
///
/// The three line fields mirror the tree-walk's bookkeeping exactly:
/// `line` is the unit-absolute line (harness frames add the ambient
/// offset captured at unit entry), `line_rel` is the text-relative line
/// the AST carried (`cur_cmd_line` parity — loop constructs read it to
/// rebase their bodies), and `line_delta` is what `line_offset` must be
/// installed as while this command dispatches (0 at unit top level, the
/// body's base line inside an inline-compiled body).
#[derive(Debug, Clone)]
pub struct CmdSite {
    pub text: SrcSpan,
    pub line: u32,
    pub line_rel: u32,
    pub line_delta: u32,
    pub word_srcs: Rc<Vec<SrcSpan>>,
}

/// One loop variable of a compiled foreach/lmap: the frame slot when the
/// name entered the locals table (plain name in a proc body), plus the
/// name's constant-pool index for the `set_var` fallback — degraded
/// frames (upvar/trace/array restructure, statics, unset-slot paths)
/// resolve by name against the same storage.
#[derive(Debug, Clone)]
pub struct VarTarget {
    pub slot: Option<u16>,
    pub name_idx: u16,
}

/// Compile-time resolved shape of one `foreach`/`lmap`: the var-list
/// groups (targets in iteration order) and whether the construct
/// collects body results (`lmap`).  `ForeachStart`/`ForeachNext` index
/// this table.
#[derive(Debug, Clone)]
pub struct ForeachInfo {
    pub groups: Vec<Vec<VarTarget>>,
    pub lmap: bool,
    /// True when compiled OUTSIDE a proc body (top level, uplevel,
    /// dispatched-unit evals): tclsh compiles foreach with
    /// compiledLocals only in proc contexts, and its top-level foreach
    /// keeps the DISPATCHED error shape — body errors gain the
    /// `("foreach" body line N)` exit frame and loop-var write failures
    /// gain the `(setting foreach loop variable "x")` decoration.  The
    /// compiled foreach in a framed unit reproduces both; in proc units
    /// (frameless/lexical) it does not.
    pub framed: bool,
}

/// Compiled bytecode for a single compilation unit (script / proc body).
#[derive(Debug, Clone)]
pub struct ByteCode {
    /// String constant pool — referenced by `PushConst`, `LoadGlobal`, etc.
    constants: Vec<String>,
    /// Instruction sequence.
    ops: Vec<OpCode>,
    /// Local variable names (index = slot number).
    locals: Vec<String>,
    /// Source line corresponding to each instruction (parallel to `ops`).
    line_map: Vec<u32>,
    /// foreach/lmap loop shapes, referenced by the `Foreach*` ops.
    foreach_infos: Vec<ForeachInfo>,
    /// Per-command source sites, referenced by `BeginCmd(site_idx)`.
    pub sites: Vec<CmdSite>,
    /// The compilation unit's source text — `sites` spans point into it;
    /// the executor installs it as the "current command" source.
    pub source: Rc<str>,
    /// Set when the compiler met a construct it cannot compile without
    /// changing semantics (unusual `if` shapes, `return` options, …).
    /// The consumer must fall back to AST evaluation for the whole unit.
    pub fallback: bool,
    /// Compile-time verdict of the E2 slot-gate's param/table alignment
    /// (`Some(params.len())`: every compile-time param was a slot
    /// candidate and was appended in order — the table's first
    /// `params.len()` entries ARE the params), `None` otherwise
    /// (duplicates, non-candidate names, or not a locals-mode unit).
    /// The runtime gate in `call_proc` re-checked the alignment with a
    /// zip walk on EVERY call; it is a pure function of the (params,
    /// table) pair, both fixed at compile time.
    pub params_aligned: Option<usize>,
    /// This unit was compiled in proc context (locals table active —
    /// `compile_unit_locals`): foreach/lmap inline-fold and braced catch
    /// bodies would inline (tclsh's compiledLocals units).  The executor
    /// surfaces it as the runtime proc-context signal
    /// (`Interp::in_locals_unit`) so DynCall'd commands can mirror the
    /// compiler's inline decisions.
    pub locals_mode: bool,
    /// The producer's shadow epoch at compile time (rtcl-core: any
    /// command whose leaf name matches an inline-folded Tier1 command —
    /// `set`, `if`, `while`, … — bumps it).  A consumer must only run
    /// code whose epoch is current: the folds bypass dispatch, so a
    /// later `proc set {...}` in any namespace must send the body back
    /// to the dynamically-resolving tree-walk (namespace-41.1).
    pub epoch: u64,
}

impl Default for ByteCode {
    fn default() -> Self {
        Self {
            constants: Vec::new(),
            ops: Vec::new(),
            locals: Vec::new(),
            line_map: Vec::new(),
            foreach_infos: Vec::new(),
            sites: Vec::new(),
            source: Rc::from(""),
            fallback: false,
            params_aligned: None,
            locals_mode: false,
            epoch: 0,
        }
    }
}

/// Comparison op → `cc` encoding shared by the superinstruction peepholes
/// and the pair fusions (see the `AddSlotSlot` variant docs).
fn cmp_cc_of(op: &OpCode) -> Option<u8> {
    match op {
        OpCode::Lt => Some(0),
        OpCode::Gt => Some(1),
        OpCode::Le => Some(2),
        OpCode::Ge => Some(3),
        OpCode::Eq => Some(4),
        OpCode::Ne => Some(5),
        _ => None,
    }
}

/// Inverse of [`cmp_cc_of`] — the comparison op for a `cc` value (cc is
/// compiler-generated, 0..=5; anything else degrades to `Ne`, whose only
/// reachable source is corruption the validator would have rejected).
fn cmp_op_of(cc: u8) -> OpCode {
    match cc {
        0 => OpCode::Lt,
        1 => OpCode::Gt,
        2 => OpCode::Le,
        3 => OpCode::Ge,
        4 => OpCode::Eq,
        _ => OpCode::Ne,
    }
}

/// Swapped-operand comparison encoding: `a < b` ⇔ `b > a`, `a <= b` ⇔
/// `b >= a` (eq/ne are symmetric).
fn cmp_cc_mirror(cc: u8) -> u8 {
    match cc {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        other => other,
    }
}

impl ByteCode {
    /// Create a new, empty [`ByteCode`].
    pub fn new() -> Self {
        Self::default()
    }

    // -- constant pool -------------------------------------------------------

    /// Add a string to the constant pool and return its index.
    /// If the string already exists, reuse the existing index.
    /// Returns `u16` for backward compatibility (panics if pool > u16::MAX).
    pub fn add_const(&mut self, s: &str) -> u16 {
        let idx = self.add_const_wide(s);
        idx as u16
    }

    /// Add a string to the constant pool and return its wide (u32) index.
    /// If the string already exists, reuse the existing index.
    pub fn add_const_wide(&mut self, s: &str) -> u32 {
        if let Some(idx) = self.constants.iter().position(|c| c == s) {
            idx as u32
        } else {
            let idx = self.constants.len() as u32;
            self.constants.push(s.to_string());
            idx
        }
    }

    /// Emit a `PushConst` or `PushConstWide` instruction for the given string.
    pub fn emit_push_const(&mut self, s: &str, line: u32) -> usize {
        let idx = self.add_const_wide(s);
        if idx <= u16::MAX as u32 {
            self.emit(OpCode::PushConst(idx as u16), line)
        } else {
            self.emit(OpCode::PushConstWide(idx), line)
        }
    }

    /// Look up a constant by index.
    pub fn get_const(&self, idx: u16) -> Option<&str> {
        self.constants.get(idx as usize).map(|s| s.as_str())
    }

    /// Look up a constant by wide index.
    pub fn get_const_wide(&self, idx: u32) -> Option<&str> {
        self.constants.get(idx as usize).map(|s| s.as_str())
    }

    /// Read-only view of the constant pool.
    pub fn constants(&self) -> &[String] {
        &self.constants
    }

    // -- instruction list ----------------------------------------------------

    /// Append an instruction and return its index.
    pub fn emit(&mut self, op: OpCode, line: u32) -> usize {
        let idx = self.ops.len();
        self.ops.push(op);
        self.line_map.push(line);
        idx
    }

    /// Roll emitted instructions back to `len` (a previously observed
    /// `ops().len()`).  The expression compiler emits speculatively and can
    /// fail mid-expression, leaving orphan stack pushes behind — harmless
    /// at unit level (only the final stack value is read) but fatal when
    /// the same expression compiles inline inside a word.
    pub fn truncate_ops(&mut self, len: usize) {
        self.ops.truncate(len);
        self.line_map.truncate(len);
    }

    /// Patch the operand of a jump instruction at `idx`.
    pub fn patch_jump(&mut self, idx: usize, target: u32) {
        match &mut self.ops[idx] {
            OpCode::Jump(off) => *off = target,
            OpCode::JumpTrue(off) => *off = target,
            OpCode::JumpFalse(off) => *off = target,
            OpCode::CatchStart(off) => *off = target,
            _ => panic!("patch_jump on non-jump instruction at {}", idx),
        }
    }

    /// Patch a `LoopEnter` instruction's continue and break targets.
    pub fn patch_loop(&mut self, idx: usize, cont: u32, brk: u32) {
        match &mut self.ops[idx] {
            OpCode::LoopEnter { cont: c, brk: b } => {
                *c = cont;
                *b = brk;
            }
            _ => panic!("patch_loop on non-LoopEnter instruction at {}", idx),
        }
    }

    /// Patch a `ForeachStart` instruction's jump targets (continue/loop
    /// end — the same roles `LoopEnter`'s targets play for its loop frame).
    pub fn patch_foreach_start(&mut self, idx: usize, next: u32, end: u32) {
        match &mut self.ops[idx] {
            OpCode::ForeachStart { next: n, end: e, .. } => {
                *n = next;
                *e = end;
            }
            _ => panic!("patch_foreach_start on non-ForeachStart instruction at {}", idx),
        }
    }

    /// Number of emitted instructions.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Read-only view of the instruction list.
    pub fn ops(&self) -> &[OpCode] {
        &self.ops
    }

    /// Current offset (= next instruction index).
    pub fn current_offset(&self) -> u32 {
        self.ops.len() as u32
    }

    // -- locals --------------------------------------------------------------

    /// Register a local variable name and return its slot index.
    pub fn add_local(&mut self, name: &str) -> u16 {
        if let Some(idx) = self.locals.iter().position(|n| n == name) {
            idx as u16
        } else {
            let idx = self.locals.len() as u16;
            self.locals.push(name.to_string());
            idx
        }
    }

    /// Look up a local by name.
    pub fn find_local(&self, name: &str) -> Option<u16> {
        self.locals.iter().position(|n| n == name).map(|i| i as u16)
    }

    /// Read-only view of local names.
    pub fn locals(&self) -> &[String] {
        &self.locals
    }

    // -- foreach shapes ------------------------------------------------------

    /// Register a foreach/lmap loop shape; returns its table index.
    pub fn add_foreach_info(&mut self, info: ForeachInfo) -> u32 {
        let idx = self.foreach_infos.len() as u32;
        self.foreach_infos.push(info);
        idx
    }

    /// Look up a foreach/lmap shape by index.
    pub fn foreach_info(&self, idx: u32) -> Option<&ForeachInfo> {
        self.foreach_infos.get(idx as usize)
    }

    // -- line map ------------------------------------------------------------

    /// Source line for instruction at `idx`.
    pub fn line_at(&self, idx: usize) -> u32 {
        self.line_map.get(idx).copied().unwrap_or(0)
    }

    /// Run the optimization pipeline on the bytecode — the driver of the
    /// nanopass phase (each pass below is a pure function over the op
    /// vector; `strip_nops` compacts between rounds and remaps every
    /// target-carrying op, so passes may rewrite a window to `Nop` +
    /// fused head without touching control flow).
    ///
    /// Patterns (2-op):
    /// - `StoreVar(x) + Pop` → `StoreVarPop(x)` + `Nop`
    /// - `PushInt(0/1)` before `JumpTrue/JumpFalse` → `PushFalse/PushTrue`
    /// - `Not + Not` → `Nop + Nop` (double negation elimination)
    ///
    /// Patterns (3-op constant folding):
    /// - `PushInt(a) + PushInt(b) + ArithOp` → `PushInt(result)` + `Nop + Nop`
    /// - `PushInt(a) + PushInt(b) + CmpOp` → `PushTrue/PushFalse` + `Nop + Nop`
    ///
    /// Producer/consumer pair fusions (see [`Self::fuse_pairs`]) run in
    /// the same round.
    ///
    /// Runs iteratively until no further changes are made.
    pub fn peephole(&mut self) {
        // Iterate: fold → strip nops → fold again (for chained constant
        // expressions).
        for _ in 0..8 {
            let changed = self.peephole_pass() | self.fuse_pairs();
            self.strip_nops();
            if !changed {
                break;
            }
        }
    }

    /// Nanopass fusion of the compiler's producer/consumer windows —
    /// one pure rewrite per window family, jump-target aware: a fusion is
    /// refused when any op *inside* the window (everything but the head)
    /// is a jump target, since the fused op replaces the window as a unit
    /// and an entry point on a tail op would lose its producers.
    ///
    /// Window table (3-op windows first so the 2-op branch windows see
    /// the superinstructions in the next round; the driver iterates to
    /// convergence with `strip_nops` between rounds):
    ///
    /// | window | fused op |
    /// |---|---|
    /// | `[LoadVar a, LoadVar b, ADD/CMP]` | `AddVarVar` / `CmpVarVar` |
    /// | `[LoadVar a, PushInt k, ADD/CMP]` | `AddVarImm` / `CmpVarImm` |
    /// | `[PushInt k, LoadVar b, CMP]` | `CmpVarImm(b, k, mirrored cc)` |
    /// | `[LoadVar s, LappendVar d, Pop]` | `LappendVarVar` |
    /// | `[LoadVar s, LappendLocal d, Pop]` | `LappendSlotSlot` |
    /// | `[LoadLocal s, LappendLocal d, Pop]` | `LappendSlotSlot` |
    /// | `[IncrVar x, Pop]` / `[IncrLocal x, Pop]` | `IncrVarPop` / `IncrLocalPop` |
    /// | `[StoreLocal x, Pop]` | `StoreLocalPop` |
    /// | `[CmpSlotSlot/Imm, JumpTrue/False]` | `CmpSlotSlot/Imm...JumpTrue/False` |
    /// | `[CmpVarVar/Imm, JumpTrue/False]` | `CmpVarVar/Imm...JumpTrue/False` |
    ///
    /// The slot families only match locals-mode codegen (`LoadLocal` and
    /// the slot superinstructions appear nowhere else), the var families
    /// match eval-level codegen — every unit runs the pass.  The fused
    /// ops are interpreter-native windows; the JIT's accepted-op delta
    /// they create is chronicled in `docs/JIT-OPCODE-DELTA.md` so its
    /// accept set / lowering can follow.
    fn fuse_pairs(&mut self) -> bool {
        if self.ops.len() < 2 {
            return false;
        }
        let len = self.ops.len();
        let targets = self.collect_jump_targets();
        // Class-A fusions ([Jump trampoline, NextMark/ForeachNext]) move
        // the consumed op's semantics *into the head* (the fused executor
        // helper normalizes both entries, so entry at the tail is
        // equivalent).  Targets that pointed at the consumed op must
        // follow it to the head — `strip_nops`' forward map would land
        // them *past* the fused op (the op after it).  The retarget is
        // step-charge faithful: for every just-before continue the
        // comparison `cont < pc` keeps its verdict (the head index is
        // exactly the consumed op's index minus one, and both sides shift
        // with the same compaction).
        let mut retargets: Vec<(u32, u32)> = Vec::new();
        let mut changed = false;
        let mut i = 0;
        while i + 1 < len {
            // Window tails must not be jump targets.
            let tail_free = |w: usize| (1..w).all(|k| i + k < len && !targets[i + k]);
            let fused: Option<(OpCode, usize)> = match (&self.ops[i], &self.ops[i + 1]) {
                // ── fused comparison branches ───────────────────────────
                (OpCode::CmpSlotSlot(a, b, cc), OpCode::JumpTrue(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpSlotSlotJumpTrue(*a, *b, *cc, *t), 2))
                }
                (OpCode::CmpSlotSlot(a, b, cc), OpCode::JumpFalse(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpSlotSlotJumpFalse(*a, *b, *cc, *t), 2))
                }
                (OpCode::CmpSlotImm(a, k, cc), OpCode::JumpTrue(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpSlotImmJumpTrue(*a, *k, *cc, *t), 2))
                }
                (OpCode::CmpSlotImm(a, k, cc), OpCode::JumpFalse(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpSlotImmJumpFalse(*a, *k, *cc, *t), 2))
                }
                (OpCode::CmpVarVar(a, b, cc), OpCode::JumpTrue(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpVarVarJumpTrue(*a, *b, *cc, *t), 2))
                }
                (OpCode::CmpVarVar(a, b, cc), OpCode::JumpFalse(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpVarVarJumpFalse(*a, *b, *cc, *t), 2))
                }
                (OpCode::CmpVarImm(a, k, cc), OpCode::JumpTrue(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpVarImmJumpTrue(*a, *k, *cc, *t), 2))
                }
                (OpCode::CmpVarImm(a, k, cc), OpCode::JumpFalse(t)) if !targets[i + 1] => {
                    Some((OpCode::CmpVarImmJumpFalse(*a, *k, *cc, *t), 2))
                }
                // ── name-keyed superinstructions ────────────────────────
                (OpCode::LoadVar(a), OpCode::PushInt(k)) if tail_free(3) => {
                    match self.ops[i + 2].clone() {
                        OpCode::Add => Some((OpCode::AddVarImm(*a, *k), 3)),
                        op => cmp_cc_of(&op).map(|cc| (OpCode::CmpVarImm(*a, *k, cc), 3)),
                    }
                }
                (OpCode::PushInt(k), OpCode::LoadVar(b)) if tail_free(3) => {
                    cmp_cc_of(&self.ops[i + 2])
                        .map(|cc| (OpCode::CmpVarImm(*b, *k, cmp_cc_mirror(cc)), 3))
                }
                (OpCode::LoadVar(a), OpCode::LoadVar(b)) if tail_free(3) => {
                    match self.ops[i + 2].clone() {
                        OpCode::Add => Some((OpCode::AddVarVar(*a, *b), 3)),
                        op => cmp_cc_of(&op).map(|cc| (OpCode::CmpVarVar(*a, *b, cc), 3)),
                    }
                }
                // ── name-keyed lappend statement ────────────────────────
                (OpCode::LoadVar(s), OpCode::LappendVar(d)) if tail_free(3) => {
                    match self.ops[i + 2] {
                        OpCode::Pop => Some((OpCode::LappendVarVar(*s, *d), 3)),
                        _ => None,
                    }
                }
                // ── slot lappend statement ──────────────────────────────
                (OpCode::LoadLocal(s), OpCode::LappendLocal(d)) if tail_free(3) => {
                    match self.ops[i + 2] {
                        OpCode::Pop => Some((OpCode::LappendSlotSlot(*s, *d), 3)),
                        _ => None,
                    }
                }
                // ── meta-op pairs (site/region bookkeeping) ──────────────
                (OpCode::BeginCmd(site), OpCode::SubMark) if !targets[i + 1] => {
                    Some((OpCode::BeginCmdSubMark(*site), 2))
                }
                (OpCode::ExprMark, OpCode::BeginCmd(site)) if !targets[i + 1] => {
                    Some((OpCode::ExprMarkBeginCmd(*site), 2))
                }
                (OpCode::BeginCmd(site), OpCode::PushConst(idx)) if !targets[i + 1] => {
                    Some((OpCode::BeginCmdPushConst(*site, *idx), 2))
                }
                (OpCode::SubEnd, OpCode::StoreLocalPop(slot)) if !targets[i + 1] => {
                    Some((OpCode::SubEndStoreLocalPop(*slot), 2))
                }
                (OpCode::SubEnd, OpCode::StoreVarPop(idx)) if !targets[i + 1] => {
                    Some((OpCode::SubEndStoreVarPop(*idx), 2))
                }
                (OpCode::SubEnd, OpCode::Return) if !targets[i + 1] => {
                    Some((OpCode::SubEndReturn, 2))
                }
                // ── loop-control meta pairs ─────────────────────────────
                // Trampoline tabs: the jump's target is the mark itself.
                // The fused op normalizes the region stack to the loop
                // frame's entry depth (the fused executor helper), so a
                // `continue` landing on the same (consumed) instruction
                // arrives with the same state either way — no target guard
                // is needed here, unlike the value-window fusions.
                (OpCode::Jump(t), OpCode::NextMark) if *t as usize == i + 1 => {
                    retargets.push(((i + 1) as u32, i as u32));
                    Some((OpCode::SwapToNextMark, 2))
                }
                (OpCode::Jump(t), OpCode::ForeachNext { info, body }) if *t as usize == i + 1 => {
                    retargets.push(((i + 1) as u32, i as u32));
                    Some((OpCode::ForeachNextClose(*info, *body), 2))
                }
                (OpCode::NextMark, OpCode::BeginCmd(site)) if !targets[i + 1] => {
                    Some((OpCode::NextMarkBeginCmd(*site), 2))
                }
                (OpCode::BeginCmd(site), OpCode::IncrLocalPop(x, k)) if !targets[i + 1] => {
                    Some((OpCode::BeginCmdIncrLocalPop(*site, *x, *k), 2))
                }
                (OpCode::BeginCmd(site), OpCode::IncrVarPop(x, k)) if !targets[i + 1] => {
                    Some((OpCode::BeginCmdIncrVarPop(*site, *x, *k), 2))
                }
                // ── statement-result discards ───────────────────────────
                (OpCode::IncrVar(x, k), OpCode::Pop) if !targets[i + 1] => {
                    Some((OpCode::IncrVarPop(*x, *k), 2))
                }
                (OpCode::IncrLocal(x, k), OpCode::Pop) if !targets[i + 1] => {
                    Some((OpCode::IncrLocalPop(*x, *k), 2))
                }
                (OpCode::StoreLocal(x), OpCode::Pop) if !targets[i + 1] => {
                    Some((OpCode::StoreLocalPop(*x), 2))
                }
                _ => None,
            };
            if let Some((op, width)) = fused {
                self.ops[i] = op;
                for slot in self.ops[i + 1..i + width].iter_mut() {
                    *slot = OpCode::Nop;
                }
                changed = true;
                i += width;
            } else {
                i += 1;
            }
        }
        if !retargets.is_empty() {
            for op in self.ops.iter_mut() {
                match op {
                    OpCode::Jump(t)
                    | OpCode::JumpTrue(t)
                    | OpCode::JumpFalse(t)
                    | OpCode::CatchStart(t)
                    | OpCode::CmpSlotSlotJumpTrue(_, _, _, t)
                    | OpCode::CmpSlotSlotJumpFalse(_, _, _, t)
                    | OpCode::CmpSlotImmJumpTrue(_, _, _, t)
                    | OpCode::CmpSlotImmJumpFalse(_, _, _, t)
                    | OpCode::CmpVarVarJumpTrue(_, _, _, t)
                    | OpCode::CmpVarVarJumpFalse(_, _, _, t)
                    | OpCode::CmpVarImmJumpTrue(_, _, _, t)
                    | OpCode::CmpVarImmJumpFalse(_, _, _, t) => {
                        for &(from, to) in &retargets {
                            if *t == from {
                                *t = to;
                            }
                        }
                    }
                    OpCode::LoopEnter { cont, brk } => {
                        for f in [cont, brk] {
                            for &(from, to) in &retargets {
                                if *f == from {
                                    *f = to;
                                }
                            }
                        }
                    }
                    OpCode::ForeachStart { next, end, .. } => {
                        for f in [next, end] {
                            for &(from, to) in &retargets {
                                if *f == from {
                                    *f = to;
                                }
                            }
                        }
                    }
                    OpCode::ForeachNext { body, .. } => {
                        for &(from, to) in &retargets {
                            if *body == from {
                                *body = to;
                            }
                        }
                    }
                    OpCode::ForeachNextClose(_, body) => {
                        for &(from, to) in &retargets {
                            if *body == from {
                                *body = to;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        changed
    }

    /// Does the unit contain any fused superinstruction (see
    /// [`Self::fuse_pairs`])?
    pub fn has_fused_ops(&self) -> bool {
        self.ops.iter().any(|op| {
            matches!(
                op,
                OpCode::CmpSlotSlotJumpTrue(..)
                    | OpCode::CmpSlotSlotJumpFalse(..)
                    | OpCode::CmpSlotImmJumpTrue(..)
                    | OpCode::CmpSlotImmJumpFalse(..)
                    | OpCode::CmpVarVarJumpTrue(..)
                    | OpCode::CmpVarVarJumpFalse(..)
                    | OpCode::CmpVarImmJumpTrue(..)
                    | OpCode::CmpVarImmJumpFalse(..)
                    | OpCode::LappendSlotSlot(..)
                    | OpCode::LappendVarVar(..)
                    | OpCode::IncrLocalPop(..)
                    | OpCode::IncrVarPop(..)
                    | OpCode::StoreLocalPop(..)
                    | OpCode::CmpVarImm(..)
                    | OpCode::CmpVarVar(..)
                    | OpCode::AddVarImm(..)
                    | OpCode::AddVarVar(..)
                    | OpCode::BeginCmdSubMark(..)
                    | OpCode::ExprMarkBeginCmd(..)
                    | OpCode::BeginCmdPushConst(..)
                    | OpCode::SubEndStoreLocalPop(..)
                    | OpCode::SubEndStoreVarPop(..)
                    | OpCode::SubEndReturn
                    | OpCode::SwapToNextMark
                    | OpCode::NextMarkBeginCmd(..)
                    | OpCode::BeginCmdIncrLocalPop(..)
                    | OpCode::BeginCmdIncrVarPop(..)
                    | OpCode::ForeachNextClose(..)
            )
        })
    }

    /// Expand every fused superinstruction back to its canonical window —
    /// the exact op sequence this unit would have had with fusion disabled
    /// (`fuse_pairs` is a pure rewrite, so this is its inverse; jump
    /// targets are remapped through the expansion shift).  Consumers whose
    /// accepted-op set must not meet the fused families (the JIT's
    /// analysis boundary, `rtcl-vm`) call this once at their compile
    /// entry — see `docs/JIT-OPCODE-DELTA.md`.
    pub fn unfused(&self) -> ByteCode {
        /// Expand one fused op into `out`; `false` = not fused (the
        /// caller copies it verbatim).
        fn expand_into(op: &OpCode, out: &mut Vec<OpCode>) -> bool {
            match op {
                OpCode::CmpSlotSlotJumpTrue(a, b, cc, t) => {
                    out.push(OpCode::CmpSlotSlot(*a, *b, *cc));
                    out.push(OpCode::JumpTrue(*t));
                }
                OpCode::CmpSlotSlotJumpFalse(a, b, cc, t) => {
                    out.push(OpCode::CmpSlotSlot(*a, *b, *cc));
                    out.push(OpCode::JumpFalse(*t));
                }
                OpCode::CmpSlotImmJumpTrue(a, k, cc, t) => {
                    out.push(OpCode::CmpSlotImm(*a, *k, *cc));
                    out.push(OpCode::JumpTrue(*t));
                }
                OpCode::CmpSlotImmJumpFalse(a, k, cc, t) => {
                    out.push(OpCode::CmpSlotImm(*a, *k, *cc));
                    out.push(OpCode::JumpFalse(*t));
                }
                // The name-keyed families predate the fusion only as raw
                // windows — expand straight through (a jump fusion's
                // canonical form is the four-op chain; re-fusing it via
                // `fuse_pairs` is exact, see the round-trip test).
                OpCode::CmpVarVarJumpTrue(a, b, cc, t) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::LoadVar(*b));
                    out.push(cmp_op_of(*cc));
                    out.push(OpCode::JumpTrue(*t));
                }
                OpCode::CmpVarVarJumpFalse(a, b, cc, t) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::LoadVar(*b));
                    out.push(cmp_op_of(*cc));
                    out.push(OpCode::JumpFalse(*t));
                }
                OpCode::CmpVarImmJumpTrue(a, k, cc, t) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::PushInt(*k));
                    out.push(cmp_op_of(*cc));
                    out.push(OpCode::JumpTrue(*t));
                }
                OpCode::CmpVarImmJumpFalse(a, k, cc, t) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::PushInt(*k));
                    out.push(cmp_op_of(*cc));
                    out.push(OpCode::JumpFalse(*t));
                }
                OpCode::LappendSlotSlot(src, dst) => {
                    out.push(OpCode::LoadLocal(*src));
                    out.push(OpCode::LappendLocal(*dst));
                    out.push(OpCode::Pop);
                }
                OpCode::LappendVarVar(src, dst) => {
                    out.push(OpCode::LoadVar(*src));
                    out.push(OpCode::LappendVar(*dst));
                    out.push(OpCode::Pop);
                }
                OpCode::IncrLocalPop(x, k) => {
                    out.push(OpCode::IncrLocal(*x, *k));
                    out.push(OpCode::Pop);
                }
                OpCode::IncrVarPop(x, k) => {
                    out.push(OpCode::IncrVar(*x, *k));
                    out.push(OpCode::Pop);
                }
                OpCode::StoreLocalPop(x) => {
                    out.push(OpCode::StoreLocal(*x));
                    out.push(OpCode::Pop);
                }
                OpCode::BeginCmdSubMark(site) => {
                    out.push(OpCode::BeginCmd(*site));
                    out.push(OpCode::SubMark);
                }
                OpCode::ExprMarkBeginCmd(site) => {
                    out.push(OpCode::ExprMark);
                    out.push(OpCode::BeginCmd(*site));
                }
                OpCode::BeginCmdPushConst(site, idx) => {
                    out.push(OpCode::BeginCmd(*site));
                    out.push(OpCode::PushConst(*idx));
                }
                OpCode::SubEndStoreLocalPop(slot) => {
                    // Straight to the pre-fusion canonical form: a
                    // `StoreLocalPop` here would still be a fused op the
                    // boundary must not emit.
                    out.push(OpCode::SubEnd);
                    out.push(OpCode::StoreLocal(*slot));
                    out.push(OpCode::Pop);
                }
                OpCode::SubEndStoreVarPop(idx) => {
                    out.push(OpCode::SubEnd);
                    out.push(OpCode::StoreVarPop(*idx));
                }
                OpCode::SubEndReturn => {
                    out.push(OpCode::SubEnd);
                    out.push(OpCode::Return);
                }
                OpCode::SwapToNextMark => {
                    out.push(OpCode::Nop);
                    out.push(OpCode::NextMark);
                }
                OpCode::NextMarkBeginCmd(site) => {
                    out.push(OpCode::NextMark);
                    out.push(OpCode::BeginCmd(*site));
                }
                OpCode::BeginCmdIncrLocalPop(site, x, k) => {
                    out.push(OpCode::BeginCmd(*site));
                    out.push(OpCode::IncrLocal(*x, *k));
                    out.push(OpCode::Pop);
                }
                OpCode::BeginCmdIncrVarPop(site, x, k) => {
                    out.push(OpCode::BeginCmd(*site));
                    out.push(OpCode::IncrVar(*x, *k));
                    out.push(OpCode::Pop);
                }
                OpCode::ForeachNextClose(info, body) => {
                    out.push(OpCode::Nop);
                    out.push(OpCode::ForeachNext { info: *info, body: *body });
                }
                // The name-keyed superinstructions themselves predate the
                // fusion only as windows — expand them back too.
                OpCode::CmpVarImm(a, k, cc) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::PushInt(*k));
                    out.push(cmp_op_of(*cc));
                }
                OpCode::CmpVarVar(a, b, cc) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::LoadVar(*b));
                    out.push(cmp_op_of(*cc));
                }
                OpCode::AddVarImm(a, k) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::PushInt(*k));
                    out.push(OpCode::Add);
                }
                OpCode::AddVarVar(a, b) => {
                    out.push(OpCode::LoadVar(*a));
                    out.push(OpCode::LoadVar(*b));
                    out.push(OpCode::Add);
                }
                _ => return false,
            }
            true
        }

        if !self.has_fused_ops() {
            return self.clone();
        }
        // old index → new index (expansion heads are the only jump targets
        // a fusion may have consumed; `fuse_pairs` refuses windows with
        // interior targets).
        let mut map = Vec::with_capacity(self.ops.len());
        let mut ops = Vec::with_capacity(self.ops.len() + self.ops.len() / 4);
        let mut line_map = Vec::with_capacity(ops.capacity());
        for (i, op) in self.ops.iter().enumerate() {
            map.push(ops.len() as u32);
            let line = self.line_at(i);
            let before = ops.len();
            if !expand_into(op, &mut ops) {
                ops.push(op.clone());
            }
            for _ in before..ops.len() {
                line_map.push(line);
            }
        }
        let n_ops = ops.len() as u32;
        let remap = |t: &mut u32| {
            *t = map.get(*t as usize).copied().unwrap_or(n_ops);
        };
        for op in ops.iter_mut() {
            match op {
                OpCode::Jump(t)
                | OpCode::JumpTrue(t)
                | OpCode::JumpFalse(t)
                | OpCode::CatchStart(t) => remap(t),
                OpCode::LoopEnter { cont, brk } => {
                    remap(cont);
                    remap(brk);
                }
                OpCode::ForeachStart { next, end, .. } => {
                    remap(next);
                    remap(end);
                }
                OpCode::ForeachNext { body, .. } => remap(body),
                _ => {}
            }
        }
        ByteCode {
            ops,
            line_map,
            ..self.clone()
        }
    }

    /// Indices that are jump targets of any target-carrying op — the
    /// complete set `strip_nops` remaps (keep the two in agreement).
    fn collect_jump_targets(&self) -> Vec<bool> {
        let mut targets = vec![false; self.ops.len() + 1];
        let mut mark = |t: u32| {
            if (t as usize) < targets.len() {
                targets[t as usize] = true;
            }
        };
        for op in &self.ops {
            match op {
                OpCode::Jump(t)
                | OpCode::JumpTrue(t)
                | OpCode::JumpFalse(t)
                | OpCode::CatchStart(t)
                | OpCode::CmpSlotSlotJumpTrue(_, _, _, t)
                | OpCode::CmpSlotSlotJumpFalse(_, _, _, t)
                | OpCode::CmpSlotImmJumpTrue(_, _, _, t)
                | OpCode::CmpSlotImmJumpFalse(_, _, _, t)
                | OpCode::CmpVarVarJumpTrue(_, _, _, t)
                | OpCode::CmpVarVarJumpFalse(_, _, _, t)
                | OpCode::CmpVarImmJumpTrue(_, _, _, t)
                | OpCode::CmpVarImmJumpFalse(_, _, _, t) => mark(*t),
                OpCode::LoopEnter { cont, brk } => {
                    mark(*cont);
                    mark(*brk);
                }
                OpCode::ForeachStart { next, end, .. } => {
                    mark(*next);
                    mark(*end);
                }
                OpCode::ForeachNext { body, .. } => mark(*body),
                OpCode::ForeachNextClose(_, body) => mark(*body),
                _ => {}
            }
        }
        targets
    }

    /// Single round of pattern matching. Returns `true` if any change was made.
    fn peephole_pass(&mut self) -> bool {
        // --- slot superinstructions (loop unrolling) ---
        //
        // [LoadLocal a, LoadLocal b, ADD/CMP]      -> AddSlotSlot/CmpSlotSlot
        // [LoadLocal a, PushInt k, ADD/CMP]        -> AddSlotImm(a, k, cc)
        // [PushInt k, LoadLocal b, CMP]            -> CmpSlotImm(b, k, cc')
        //
        // The accumulator/condition shapes of every arithmetic loop, INCLUDING
        // the copies inside `[expr ...]` bracket words (the window needs no
        // StoreLocal suffix, so brackets don't block it).  cc' mirrors the
        // comparison for the swapped operands (Lt<->Gt, Le<->Ge).  The fused
        // ops replicate LoadLocal's fallback (`eval_var_ref` on the table
        // name) inside themselves, so unset/linked slots and non-numeric
        // operands produce the identical values and errors.
        let mut slot_fused = false;
        {
        let cc_of = cmp_cc_of;
        let mirror = cmp_cc_mirror;
        let len = self.ops.len();
        let _changed = false;
        if len >= 3 {
            let mut i = 0;
            while i + 2 < len {
                let fused: Option<OpCode> =
                    match (&self.ops[i], &self.ops[i + 1], &self.ops[i + 2]) {
                        (OpCode::LoadLocal(a), OpCode::LoadLocal(b), op) => {
                            if matches!(op, OpCode::Add) {
                                Some(OpCode::AddSlotSlot(*a, *b))
                            } else {
                                cc_of(op).map(|cc| OpCode::CmpSlotSlot(*a, *b, cc))
                            }
                        }
                        (OpCode::LoadLocal(a), OpCode::PushInt(k), op) => {
                            if matches!(op, OpCode::Add) {
                                Some(OpCode::AddSlotImm(*a, *k))
                            } else {
                                cc_of(op).map(|cc| OpCode::CmpSlotImm(*a, *k, cc))
                            }
                        }
                        (OpCode::PushInt(k), OpCode::LoadLocal(b), op) => cc_of(op)
                            .map(|cc| OpCode::CmpSlotImm(*b, *k, mirror(cc))),
                        _ => None,
                    };
                if let Some(f) = fused {
                    self.ops[i] = f;
                    self.ops[i + 1] = OpCode::Nop;
                    self.ops[i + 2] = OpCode::Nop;
                    slot_fused = true;
                    i += 3;
                } else {
                    i += 1;
                }
            }
        }
        if slot_fused {
            self.strip_nops();
        }
        }
        let _len = self.ops.len();
        let len = self.ops.len();
        if len < 2 {
            return false;
        }
        let mut changed = false;

        // --- 3-op constant folding ---
        //
        // Fold ONLY when the folded result is exactly what runtime
        // evaluation would produce: Tcl widens i64 overflow to bignum,
        // floors `/`/`%` (== trunc only in the a≥0, b>0 domain), errors
        // on negative shift counts, and errors on huge exponents.  When
        // a fold doesn't apply, leave the ops — the executor's shared
        // expr_ops helpers compute the exact semantics.
        if len >= 3 {
            let mut i = 0;
            while i + 2 < len {
                if let (OpCode::PushInt(a), OpCode::PushInt(b)) = (&self.ops[i], &self.ops[i + 1]) {
                    let a = *a;
                    let b = *b;
                    let folded = match &self.ops[i + 2] {
                        OpCode::Add => a.checked_add(b).map(FoldResult::Int),
                        OpCode::Sub => a.checked_sub(b).map(FoldResult::Int),
                        OpCode::Mul => a.checked_mul(b).map(FoldResult::Int),
                        // Floor division == truncating only for a≥0, b>0
                        OpCode::Div if a >= 0 && b > 0 => Some(FoldResult::Int(a / b)),
                        OpCode::Mod if a >= 0 && b > 0 => Some(FoldResult::Int(a % b)),
                        // checked_pow matches runtime exactly (unit cases
                        // included); huge exponents error at runtime.
                        OpCode::Pow if (0..(1 << 28)).contains(&b) => {
                            a.checked_pow(b as u32).map(FoldResult::Int)
                        }
                        OpCode::Eq  => Some(FoldResult::Bool(a == b)),
                        OpCode::Ne  => Some(FoldResult::Bool(a != b)),
                        OpCode::Lt  => Some(FoldResult::Bool(a < b)),
                        OpCode::Gt  => Some(FoldResult::Bool(a > b)),
                        OpCode::Le  => Some(FoldResult::Bool(a <= b)),
                        OpCode::Ge  => Some(FoldResult::Bool(a >= b)),
                        OpCode::BitAnd => Some(FoldResult::Int(a & b)),
                        OpCode::BitOr  => Some(FoldResult::Int(a | b)),
                        OpCode::BitXor => Some(FoldResult::Int(a ^ b)),
                        // checked_shl only validates the COUNT (Rust:
                        // shifts are defined wrapping); a positive shift
                        // can still overflow i64, and the runtime widens
                        // to bignum — fold only when the result fits
                        // (expr-24.10: 500000000000000<<28).
                        OpCode::Shl if (0..64).contains(&b) => {
                            let fits = if a >= 0 {
                                b == 0 || a <= (i64::MAX >> b)
                            } else {
                                b == 0 || a >= (i64::MIN >> b)
                            };
                            if fits { Some(FoldResult::Int(a << b)) } else { None }
                        }
                        // Arithmetic >> saturates past the width, matching Tcl
                        OpCode::Shr if b >= 64 => {
                            Some(FoldResult::Int(if a < 0 { -1 } else { 0 }))
                        }
                        OpCode::Shr if b >= 0 => Some(FoldResult::Int(a >> b)),
                        _ => None,
                    };
                    if let Some(result) = folded {
                        match result {
                            FoldResult::Int(n) => self.ops[i] = OpCode::PushInt(n),
                            FoldResult::Bool(true) => self.ops[i] = OpCode::PushTrue,
                            FoldResult::Bool(false) => self.ops[i] = OpCode::PushFalse,
                        }
                        self.ops[i + 1] = OpCode::Nop;
                        self.ops[i + 2] = OpCode::Nop;
                        changed = true;
                        continue;
                    }
                }
                i += 1;
            }
        }

        // --- 2-op patterns ---
        let len = self.ops.len();
        let mut i = 0;
        while i + 1 < len {
            match (&self.ops[i], &self.ops[i + 1]) {
                // StoreVar(x) + Pop → StoreVarPop(x)
                (OpCode::StoreVar(idx), OpCode::Pop) => {
                    let idx = *idx;
                    self.ops[i] = OpCode::StoreVarPop(idx);
                    self.ops[i + 1] = OpCode::Nop;
                    changed = true;
                    i += 2;
                }
                // PushInt(1) before JumpFalse/JumpTrue → PushTrue
                (OpCode::PushInt(1), OpCode::JumpFalse(_) | OpCode::JumpTrue(_)) => {
                    self.ops[i] = OpCode::PushTrue;
                    changed = true;
                    i += 1;
                }
                // PushInt(0) before JumpFalse/JumpTrue → PushFalse
                (OpCode::PushInt(0), OpCode::JumpFalse(_) | OpCode::JumpTrue(_)) => {
                    self.ops[i] = OpCode::PushFalse;
                    changed = true;
                    i += 1;
                }
                // Double negation elimination
                (OpCode::Not, OpCode::Not) => {
                    self.ops[i] = OpCode::Nop;
                    self.ops[i + 1] = OpCode::Nop;
                    changed = true;
                    i += 2;
                }
                _ => {
                    i += 1;
                }
            }
        }

        changed
    }

    /// Remove all `Nop` instructions, adjusting jump targets accordingly.
    fn strip_nops(&mut self) {
        let len = self.ops.len();
        if len == 0 {
            return;
        }

        // Build a mapping: old_index → new_index
        let mut new_index = vec![0u32; len];
        let mut offset = 0u32;
        for (i, idx) in new_index.iter_mut().enumerate() {
            *idx = offset;
            if !matches!(self.ops[i], OpCode::Nop) {
                offset += 1;
            }
        }
        let new_len = offset as usize;
        if new_len == len {
            return; // nothing to strip
        }

        // Remap jump targets
        // Jump targets point to instruction indices — map them through new_index.
        // If a jump target pointed at a Nop, map it to the next real instruction.
        // Build a "forward" table: for any old index, what's the next non-Nop new index?
        let mut forward = vec![new_len as u32; len + 1];
        // Process backwards so forward[i] is the new index of the first non-Nop at or after old i.
        {
            let mut next = new_len as u32;
            for i in (0..len).rev() {
                if !matches!(self.ops[i], OpCode::Nop) {
                    next = new_index[i];
                }
                forward[i] = next;
            }
            forward[len] = new_len as u32;
        }

        for op in self.ops.iter_mut() {
            match op {
                OpCode::Jump(t) => *t = forward[*t as usize],
                OpCode::JumpTrue(t) => *t = forward[*t as usize],
                OpCode::JumpFalse(t) => *t = forward[*t as usize],
                OpCode::CmpSlotSlotJumpTrue(_, _, _, t)
                | OpCode::CmpSlotSlotJumpFalse(_, _, _, t)
                | OpCode::CmpSlotImmJumpTrue(_, _, _, t)
                | OpCode::CmpSlotImmJumpFalse(_, _, _, t)
                | OpCode::CmpVarVarJumpTrue(_, _, _, t)
                | OpCode::CmpVarVarJumpFalse(_, _, _, t)
                | OpCode::CmpVarImmJumpTrue(_, _, _, t)
                | OpCode::CmpVarImmJumpFalse(_, _, _, t) => *t = forward[*t as usize],
                OpCode::LoopEnter { cont, brk } => {
                    *cont = forward[*cont as usize];
                    *brk = forward[*brk as usize];
                }
                OpCode::CatchStart(t) => *t = forward[*t as usize],
                // The inline foreach's targets bake pre-compaction
                // indices: ForeachNext jumps back to the body,
                // ForeachStart's next/end land on the iteration step and
                // the exit.  Without this remap, ANY nop strip (a
                // constant fold) earlier in the unit shifted the loop's
                // entry by the strip count — latent since the inline
                // foreach landed (proc bodies never folded ahead of a
                // loop; the global-scope inline hit it on day one).
                OpCode::ForeachStart { next, end, .. } => {
                    *next = forward[*next as usize];
                    *end = forward[*end as usize];
                }
                OpCode::ForeachNext { body, .. } => {
                    *body = forward[*body as usize];
                }
                OpCode::ForeachNextClose(_, body) => {
                    *body = forward[*body as usize];
                }
                _ => {}
            }
        }

        // Compact ops and line_map
        let mut write = 0;
        for read in 0..len {
            if !matches!(self.ops[read], OpCode::Nop) {
                self.ops[write] = self.ops[read].clone();
                self.line_map[write] = self.line_map[read];
                write += 1;
            }
        }
        self.ops.truncate(new_len);
        self.line_map.truncate(new_len);
    }
}

impl fmt::Display for ByteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Instructions:")?;
        for (i, op) in self.ops.iter().enumerate() {
            let line = self.line_at(i);
            writeln!(f, "{:04} L{:<4} {}", i, line, op)?;
        }
        if !self.constants.is_empty() {
            writeln!(f, "--- constants ---")?;
            for (i, c) in self.constants.iter().enumerate() {
                writeln!(f, "  [{}] {:?}", i, c)?;
            }
        }
        if !self.locals.is_empty() {
            writeln!(f, "--- locals ---")?;
            for (i, name) in self.locals.iter().enumerate() {
                writeln!(f, "  [{}] {}", i, name)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bc(ops: &[OpCode], locals_mode: bool) -> ByteCode {
        let mut c = ByteCode::new();
        for op in ops {
            c.emit(op.clone(), 0);
        }
        c.locals_mode = locals_mode;
        c
    }

    /// The producer/consumer windows collapse to one op each, and the
    /// `strip_nops` compaction leaves no residue.
    #[test]
    fn fuses_slot_pair_windows() {
        let mut code = bc(
            &[
                OpCode::CmpSlotSlot(1, 2, 0),
                OpCode::JumpFalse(8),
                OpCode::Nop,
                OpCode::LoadLocal(1),
                OpCode::LappendLocal(2),
                OpCode::Pop,
                OpCode::IncrLocal(3, 1),
                OpCode::Pop,
                OpCode::StoreLocal(4),
                OpCode::Pop,
                OpCode::Return,
            ],
            true,
        );
        code.peephole();
        assert_eq!(
            code.ops(),
            &[
                OpCode::CmpSlotSlotJumpFalse(1, 2, 0, 3),
                OpCode::LappendSlotSlot(1, 2),
                OpCode::IncrLocalPop(3, 1),
                OpCode::StoreLocalPop(4),
                OpCode::Return,
            ]
        );
    }

    /// A jump targeting the consumer inside the window refuses the
    /// fusion: the fused op replaces the whole window, and an entry
    /// point on the consumer would lose its producer.
    #[test]
    fn refuses_fusion_when_target_inside_window() {
        let mut code = bc(
            &[
                OpCode::JumpTrue(2),
                OpCode::LoadLocal(1),
                OpCode::LappendLocal(2),
                OpCode::Pop,
                OpCode::Return,
            ],
            true,
        );
        code.peephole();
        assert!(code
            .ops()
            .iter()
            .any(|o| matches!(o, OpCode::LappendLocal(_))));
        assert!(code.ops().iter().any(|o| matches!(o, OpCode::Pop)));
    }

    /// Name-keyed (eval-level) windows fuse too — the chain
    /// `[LoadVar, PushInt, Lt, JumpFalse]` becomes one op over two rounds
    /// of the driver.  The op-set delta this creates for the JIT's accept
    /// set is chronicled in `docs/JIT-OPCODE-DELTA.md`.
    #[test]
    fn fuses_name_keyed_windows() {
        let mut code = bc(
            &[
                OpCode::LoadVar(1),
                OpCode::PushInt(200000),
                OpCode::Lt,
                OpCode::JumpFalse(5),
                OpCode::Nop,
                OpCode::Return,
            ],
            false,
        );
        code.peephole();
        assert_eq!(
            code.ops(),
            &[
                OpCode::CmpVarImmJumpFalse(1, 200000, 0, 1),
                OpCode::Return,
            ]
        );
    }

    /// `unfused()` is the exact inverse of the fusions — expansion
    /// re-creates the canonical windows, jump targets included — and
    /// re-fusing the result yields the identical unit (round trip).  The
    /// JIT/`rtcl-vm` boundary relies on this to keep its accepted set
    /// untouched (see `docs/JIT-OPCODE-DELTA.md`).
    #[test]
    fn unfused_expands_windows() {
        let mut code = bc(
            &[
                OpCode::LoadVar(1),
                OpCode::PushInt(200000),
                OpCode::Lt,
                OpCode::JumpFalse(5),
                OpCode::Nop,
                OpCode::Return,
            ],
            false,
        );
        code.peephole();
        assert_eq!(code.ops()[0], OpCode::CmpVarImmJumpFalse(1, 200000, 0, 1));
        let back = code.unfused();
        assert!(!back.has_fused_ops());
        assert_eq!(
            back.ops(),
            &[
                OpCode::LoadVar(1),
                OpCode::PushInt(200000),
                OpCode::Lt,
                OpCode::JumpFalse(4),
                OpCode::Return,
            ]
        );
        let mut again = back;
        again.peephole();
        assert_eq!(again.ops(), code.ops());
    }

    /// Every fused/superinstruction family expands — `unfused()` leaves no
    /// op outside the pre-fusion set (the JIT/`rtcl-vm` boundary contract).
    #[test]
    fn unfused_covers_every_family() {
        let mut code = bc(
            &[
                OpCode::CmpSlotSlotJumpTrue(1, 2, 0, 12),
                OpCode::CmpSlotSlotJumpFalse(1, 2, 0, 12),
                OpCode::CmpSlotImmJumpTrue(1, 9, 0, 12),
                OpCode::CmpSlotImmJumpFalse(1, 9, 0, 12),
                OpCode::CmpVarVarJumpTrue(1, 2, 0, 12),
                OpCode::CmpVarVarJumpFalse(1, 2, 0, 12),
                OpCode::CmpVarImmJumpTrue(1, 9, 0, 12),
                OpCode::CmpVarImmJumpFalse(1, 9, 0, 12),
                OpCode::LappendSlotSlot(1, 2),
                OpCode::LappendVarVar(1, 2),
                OpCode::IncrLocalPop(1, 1),
                OpCode::IncrVarPop(1, 1),
                OpCode::StoreLocalPop(1),
                OpCode::CmpVarImm(1, 9, 0),
                OpCode::CmpVarVar(1, 2, 0),
                OpCode::AddVarImm(1, 9),
                OpCode::AddVarVar(1, 2),
                OpCode::BeginCmdSubMark(1),
                OpCode::ExprMarkBeginCmd(1),
                OpCode::BeginCmdPushConst(1, 2),
                OpCode::SubEndStoreLocalPop(1),
                OpCode::SubEndStoreVarPop(1),
                OpCode::SubEndReturn,
                OpCode::SwapToNextMark,
                OpCode::NextMarkBeginCmd(1),
                OpCode::BeginCmdIncrLocalPop(1, 2, 1),
                OpCode::BeginCmdIncrVarPop(1, 2, 1),
                OpCode::ForeachNextClose(1, 2),
                OpCode::Return,
            ],
            false,
        );
        assert!(code.has_fused_ops());
        let back = code.unfused();
        assert!(!back.has_fused_ops());
        assert_eq!(back.ops().last(), Some(&OpCode::Return));
    }

    /// `strip_nops` remaps the fused op's own jump target like the
    /// dispatched jump ops' (a compaction ahead of the window must not
    /// shift the branch landing).
    #[test]
    fn strip_remaps_fused_targets() {
        let mut code = bc(
            &[
                OpCode::Nop,
                OpCode::CmpSlotSlot(1, 2, 0),
                OpCode::JumpFalse(4),
                OpCode::Nop,
                OpCode::Return,
            ],
            true,
        );
        code.peephole();
        let target = code
            .ops()
            .iter()
            .find_map(|o| match o {
                OpCode::CmpSlotSlotJumpFalse(_, _, _, t) => Some(*t),
                _ => None,
            })
            .expect("fused branch present");
        assert_eq!(target as usize, code.ops().len() - 1);
    }
}
