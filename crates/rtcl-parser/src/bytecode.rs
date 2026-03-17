//! Compiled bytecode representation.
//!
//! A [`ByteCode`] object holds everything needed to execute a compiled Tcl
//! script: a constant pool, an instruction list, local-variable names, and
//! source-line mappings for diagnostics.

use crate::opcode::OpCode;
use core::fmt;

/// Result of a constant-folding operation.
enum FoldResult {
    Int(i64),
    Bool(bool),
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
}

impl ByteCode {
    /// Create a new, empty [`ByteCode`].
    pub fn new() -> Self {
        ByteCode {
            constants: Vec::new(),
            ops: Vec::new(),
            locals: Vec::new(),
            line_map: Vec::new(),
        }
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
        let len = self.ops.len();
        if len < 2 {
            return false;
        }
        let mut changed = false;

        // --- 3-op constant folding ---
        if len >= 3 {
            let mut i = 0;
            while i + 2 < len {
                if let (OpCode::PushInt(a), OpCode::PushInt(b)) = (&self.ops[i], &self.ops[i + 1]) {
                    let a = *a;
                    let b = *b;
                    let folded = match &self.ops[i + 2] {
                        OpCode::Add => Some(FoldResult::Int(a.wrapping_add(b))),
                        OpCode::Sub => Some(FoldResult::Int(a.wrapping_sub(b))),
                        OpCode::Mul => Some(FoldResult::Int(a.wrapping_mul(b))),
                        OpCode::Div if b != 0 => Some(FoldResult::Int(a / b)),
                        OpCode::Mod if b != 0 => Some(FoldResult::Int(a % b)),
                        OpCode::Pow if b >= 0 && b <= u32::MAX as i64 => {
                            Some(FoldResult::Int(a.wrapping_pow(b as u32)))
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
                        OpCode::Shl => Some(FoldResult::Int(a.wrapping_shl((b & 63) as u32))),
                        OpCode::Shr => Some(FoldResult::Int(a.wrapping_shr((b & 63) as u32))),
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
        for i in 0..len {
            new_index[i] = offset;
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

impl Default for ByteCode {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Pretty-print
// ---------------------------------------------------------------------------

impl fmt::Display for ByteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== ByteCode ===")?;
        if !self.constants.is_empty() {
            writeln!(f, "Constants:")?;
            for (i, c) in self.constants.iter().enumerate() {
                writeln!(f, "  {:4}: {:?}", i, c)?;
            }
        }
        if !self.locals.is_empty() {
            writeln!(f, "Locals:")?;
            for (i, l) in self.locals.iter().enumerate() {
                writeln!(f, "  {:4}: {}", i, l)?;
            }
        }
        writeln!(f, "Instructions:")?;
        for (i, op) in self.ops.iter().enumerate() {
            let line = self.line_map.get(i).copied().unwrap_or(0);
            writeln!(f, "  {:04} [L{:>3}] {}", i, line, op)?;
        }
        Ok(())
    }
}
