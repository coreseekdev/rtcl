//! Compiled bytecode representation.
//!
//! A [`ByteCode`] object holds everything needed to execute a compiled Tcl
//! script: a constant pool, an instruction list, local-variable names, and
//! source-line mappings for diagnostics.

use crate::opcode::OpCode;
use core::fmt;

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
    pub fn add_const(&mut self, s: &str) -> u16 {
        if let Some(idx) = self.constants.iter().position(|c| c == s) {
            idx as u16
        } else {
            let idx = self.constants.len() as u16;
            self.constants.push(s.to_string());
            idx
        }
    }

    /// Look up a constant by index.
    pub fn get_const(&self, idx: u16) -> Option<&str> {
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
    /// Patterns:
    /// - `StoreVar(x) + Pop` → `StoreVarPop(x)` + `Nop`
    /// - `PushInt(0)` (as boolean) → `PushFalse`  (when followed by JumpTrue/JumpFalse)
    /// - `PushInt(1)` (as boolean) → `PushTrue`   (when followed by JumpTrue/JumpFalse)
    pub fn peephole(&mut self) {
        let len = self.ops.len();
        if len < 2 {
            return;
        }
        let mut i = 0;
        while i + 1 < len {
            match (&self.ops[i], &self.ops[i + 1]) {
                // StoreVar(x) + Pop → StoreVarPop(x)
                (OpCode::StoreVar(idx), OpCode::Pop) => {
                    let idx = *idx;
                    self.ops[i] = OpCode::StoreVarPop(idx);
                    self.ops[i + 1] = OpCode::Nop;
                    i += 2;
                }
                // PushInt(1) before JumpFalse → PushTrue
                (OpCode::PushInt(1), OpCode::JumpFalse(_) | OpCode::JumpTrue(_)) => {
                    self.ops[i] = OpCode::PushTrue;
                    i += 1;
                }
                // PushInt(0) before JumpFalse → PushFalse
                (OpCode::PushInt(0), OpCode::JumpFalse(_) | OpCode::JumpTrue(_)) => {
                    self.ops[i] = OpCode::PushFalse;
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
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
