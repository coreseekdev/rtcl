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

    /// Run a peephole optimization pass on the bytecode.
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
    /// Runs iteratively until no further changes are made.
    pub fn peephole(&mut self) {
        // Iterate: fold → strip nops → fold again (for chained constant expressions)
        for _ in 0..8 {
            let changed = self.peephole_pass();
            self.strip_nops();
            if !changed {
                break;
            }
        }
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
        let cc_of = |op: &OpCode| -> Option<u8> {
            match op {
                OpCode::Lt => Some(0),
                OpCode::Gt => Some(1),
                OpCode::Le => Some(2),
                OpCode::Ge => Some(3),
                OpCode::Eq => Some(4),
                OpCode::Ne => Some(5),
                _ => None,
            }
        };
        let mirror = |cc: u8| match cc {
            0 => 1u8,
            1 => 0u8,
            2 => 3u8,
            3 => 2u8,
            other => other,
        };
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
