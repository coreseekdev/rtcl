//! Compiler — transforms a [`crate`] AST into [`ByteCode`].
//!
//! ## Compilation strategy
//!
//! 1. **Control-flow commands** (`set`, `if`, `while`, `for`, `break`,
//!    `continue`, `return`, `exit`, `expr`, `incr`) are compiled to *native
//!    VM opcodes* — no command-table lookup at runtime.  Loop bodies and
//!    if-branches are compiled **inline** (the body string is parsed and
//!    compiled recursively), eliminating per-iteration reparsing.
//!
//! 2. **Standard and language library commands** (e.g. `string`, `list`,
//!    `foreach`, `proc`, …) are compiled to `ECall { cmd_id, argc }`.
//!    The `cmd_id` comes from [`StdCmdId`]; the VM dispatches through a
//!    function-pointer table.
//!
//! 3. **Extension / platform commands** (`puts`, `file`, `regexp`, …) are
//!    compiled to `SysCall { cmd_id, argc }`.
//!
//! 4. **Unknown / dynamic commands** fall back to `DynCall { argc }`.

use crate::{Command, Word};
use crate::bytecode::ByteCode;
use crate::opcode::{OpCode, StdCmdId, ExtCmdId};

// ---------------------------------------------------------------------------
// Loop context — tracks the active loop during compilation so that `break`
// and `continue` inside inline-compiled bodies can be resolved.
// ---------------------------------------------------------------------------

struct LoopCtx {
    /// Index of the `LoopEnter` instruction (to be patched later).
    enter_idx: usize,
    /// PC of the continue target (loop condition re-check, or "next" step).
    continue_target: u32,
    /// Indices of `Jump` instructions that need patching to the break target.
    break_patches: Vec<usize>,
}

/// Compiler state.
pub struct Compiler {
    bytecode: ByteCode,
    /// Stack of active loops (innermost at the end).
    loops: Vec<LoopCtx>,
}

impl Compiler {
    /// Compile a list of parsed commands into [`ByteCode`].
    pub fn compile(commands: &[Command]) -> ByteCode {
        let mut c = Compiler {
            bytecode: ByteCode::new(),
            loops: Vec::new(),
        };
        c.compile_commands(commands);
        c.bytecode
    }

    /// Compile a Tcl source string in one step (parse + compile).
    pub fn compile_script(source: &str) -> Result<ByteCode, crate::ParseError> {
        let commands = crate::parse(source)?;
        Ok(Self::compile(&commands))
    }

    // -----------------------------------------------------------------------
    // Internal — command-level
    // -----------------------------------------------------------------------

    fn compile_commands(&mut self, commands: &[Command]) {
        for (i, cmd) in commands.iter().enumerate() {
            if i > 0 {
                // Discard intermediate results (only the last result matters).
                self.bytecode.emit(OpCode::Pop, 0);
            }
            self.compile_command(cmd);
        }
    }

    fn compile_command(&mut self, cmd: &Command) {
        if cmd.words.is_empty() {
            return;
        }

        let line = cmd.line as u32;
        self.bytecode.emit(OpCode::Line(line), line);

        // --- Specialised codegen for known commands (first word is literal) --
        if let Word::Literal(name) = &cmd.words[0] {
            match name.as_str() {
                // ── Tier 1: compiled to native opcodes ──────────────────
                "set" if cmd.words.len() == 3 => return self.compile_set(cmd),
                "set" if cmd.words.len() == 2 => return self.compile_set_get(cmd),
                "if" => return self.compile_if(cmd),
                "while" if cmd.words.len() == 3 => return self.compile_while(cmd),
                "for" if cmd.words.len() == 5 => return self.compile_for(cmd),
                "expr" => return self.compile_expr(cmd),
                "incr" if cmd.words.len() >= 2 => return self.compile_incr(cmd),
                "break" if cmd.words.len() == 1 => {
                    self.bytecode.emit(OpCode::Break, line);
                    return;
                }
                "continue" if cmd.words.len() == 1 => {
                    self.bytecode.emit(OpCode::Continue, line);
                    return;
                }
                "return" => return self.compile_return(cmd),
                "exit" => return self.compile_exit(cmd),

                // ── Tier 2: ECall (standard / language library) ─────────
                _ if StdCmdId::from_name(name).is_some() => {
                    return self.compile_ecall(cmd, name);
                }

                // ── Tier 3: SysCall (extension / platform) ─────────────
                _ if ExtCmdId::from_name(name).is_some() => {
                    return self.compile_syscall(cmd, name);
                }

                _ => {}
            }
        }

        // ── Tier 4: DynCall (unknown / dynamic) ────────────────────────
        self.compile_dyncall(cmd);
    }

    // -----------------------------------------------------------------------
    // Word compilation
    // -----------------------------------------------------------------------

    fn compile_word(&mut self, word: &Word, line: u32) {
        match word {
            Word::Literal(s) => {
                if s.is_empty() {
                    self.bytecode.emit(OpCode::PushEmpty, line);
                } else if let Ok(n) = s.parse::<i64>() {
                    self.bytecode.emit(OpCode::PushInt(n), line);
                } else {
                    let idx = self.bytecode.add_const(s);
                    self.bytecode.emit(OpCode::PushConst(idx), line);
                }
            }
            Word::VarRef(name) => {
                let idx = self.bytecode.add_const(name);
                self.bytecode.emit(OpCode::LoadVar(idx), line);
            }
            Word::CommandSub(script) => {
                let idx = self.bytecode.add_const(script);
                self.bytecode.emit(OpCode::PushConst(idx), line);
                self.bytecode.emit(OpCode::EvalScript, line);
            }
            Word::Concat(parts) => {
                let n = parts.len() as u16;
                for p in parts {
                    self.compile_word(p, line);
                }
                self.bytecode.emit(OpCode::Concat(n), line);
            }
            Word::Expand(inner) => {
                self.compile_word(inner, line);
                self.bytecode.emit(OpCode::ExpandList, line);
            }
            Word::ExprSugar(expr) => {
                let idx = self.bytecode.add_const(expr);
                self.bytecode.emit(OpCode::PushConst(idx), line);
                self.bytecode.emit(OpCode::EvalExpr, line);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tier 1 — control-flow commands (native opcodes, inline bodies)
    // -----------------------------------------------------------------------

    /// `set varName value`
    fn compile_set(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        self.compile_word(&cmd.words[2], line);
        if let Word::Literal(name) = &cmd.words[1] {
            let idx = self.bytecode.add_const(name);
            self.bytecode.emit(OpCode::StoreVar(idx), line);
        } else {
            // Dynamic var name — fall back to DynCall
            self.bytecode.emit(OpCode::Pop, line);
            self.compile_dyncall(cmd);
        }
    }

    /// `set varName` (read-only form)
    fn compile_set_get(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        if let Word::Literal(name) = &cmd.words[1] {
            let idx = self.bytecode.add_const(name);
            self.bytecode.emit(OpCode::LoadVar(idx), line);
        } else {
            self.compile_dyncall(cmd);
        }
    }

    /// `if expr body ?elseif expr body ...? ?else body?`
    fn compile_if(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        let mut end_jumps = Vec::new();
        let mut i = 1;

        while i < cmd.words.len() {
            if i > 1 {
                // "elseif" or "else" keyword
                if let Word::Literal(kw) = &cmd.words[i] {
                    match kw.as_str() {
                        "elseif" => {
                            i += 1;
                        }
                        "else" => {
                            // Compile the else body inline
                            i += 1;
                            if i < cmd.words.len() {
                                self.compile_body_inline(&cmd.words[i], line);
                            }
                            break;
                        }
                        "then" => {
                            // skip optional 'then' keyword
                            i += 1;
                            if i < cmd.words.len() {
                                self.compile_body_inline(&cmd.words[i], line);
                            }
                            break;
                        }
                        _ => {
                            // implicit else body
                            self.compile_body_inline(&cmd.words[i], line);
                            break;
                        }
                    }
                } else {
                    break;
                }
            }

            if i + 1 >= cmd.words.len() {
                break;
            }

            // Compile the condition
            self.compile_expr_word(&cmd.words[i], line);
            let false_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);
            i += 1;

            // Skip optional "then" keyword
            if i < cmd.words.len() {
                if let Word::Literal(kw) = &cmd.words[i] {
                    if kw == "then" {
                        i += 1;
                    }
                }
            }

            // Compile the then-body inline
            if i < cmd.words.len() {
                self.compile_body_inline(&cmd.words[i], line);
            }
            i += 1;

            let end_jump = self.bytecode.emit(OpCode::Jump(0), line);
            end_jumps.push(end_jump);

            // Patch false jump to here
            let here = self.bytecode.current_offset();
            self.bytecode.patch_jump(false_jump, here);
        }

        // If no branch was taken, push empty
        if end_jumps.is_empty() {
            // Simple if with no else: if condition was false, push empty
        }

        // All end-jumps converge here
        let end = self.bytecode.current_offset();
        for j in end_jumps {
            self.bytecode.patch_jump(j, end);
        }
    }

    /// `while test body`
    fn compile_while(&mut self, cmd: &Command) {
        let line = cmd.line as u32;

        // Emit LoopEnter (targets patched later)
        let loop_enter = self.bytecode.emit(
            OpCode::LoopEnter { cont: 0, brk: 0 },
            line,
        );

        // Push a loop context for break/continue resolution
        self.loops.push(LoopCtx {
            enter_idx: loop_enter,
            continue_target: 0,
            break_patches: Vec::new(),
        });

        let condition_pc = self.bytecode.current_offset();

        // Set the continue target to the condition check
        if let Some(lctx) = self.loops.last_mut() {
            lctx.continue_target = condition_pc;
        }

        // Compile the test expression
        self.compile_expr_word(&cmd.words[1], line);
        let exit_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);

        // Compile the body inline
        self.compile_body_inline(&cmd.words[2], line);
        self.bytecode.emit(OpCode::Pop, line); // discard body result

        // Jump back to loop start
        self.bytecode.emit(OpCode::Jump(condition_pc), line);

        // Break target = here
        let after_loop = self.bytecode.current_offset();
        self.bytecode.patch_jump(exit_jump, after_loop);

        // Emit LoopExit
        self.bytecode.emit(OpCode::LoopExit, line);

        // Patch LoopEnter
        self.bytecode.patch_loop(loop_enter, condition_pc, after_loop);

        // Patch any break jumps from the body
        let lctx = self.loops.pop().unwrap();
        for patch_idx in lctx.break_patches {
            self.bytecode.patch_jump(patch_idx, after_loop);
        }

        // While returns empty on normal exit
        self.bytecode.emit(OpCode::PushEmpty, line);
    }

    /// `for start test next body`
    fn compile_for(&mut self, cmd: &Command) {
        let line = cmd.line as u32;

        // Compile "start" inline
        self.compile_body_inline(&cmd.words[1], line);
        self.bytecode.emit(OpCode::Pop, line); // discard init result

        // Emit LoopEnter (targets patched later)
        let loop_enter = self.bytecode.emit(
            OpCode::LoopEnter { cont: 0, brk: 0 },
            line,
        );

        self.loops.push(LoopCtx {
            enter_idx: loop_enter,
            continue_target: 0,
            break_patches: Vec::new(),
        });

        let condition_pc = self.bytecode.current_offset();

        // Compile "test"
        self.compile_expr_word(&cmd.words[2], line);
        let exit_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);

        // Compile "body" inline
        self.compile_body_inline(&cmd.words[4], line);
        self.bytecode.emit(OpCode::Pop, line); // discard body result

        // Continue target = start of "next" step
        let next_pc = self.bytecode.current_offset();
        if let Some(lctx) = self.loops.last_mut() {
            lctx.continue_target = next_pc;
        }

        // Compile "next" inline
        self.compile_body_inline(&cmd.words[3], line);
        self.bytecode.emit(OpCode::Pop, line); // discard next result

        // Jump back to condition
        self.bytecode.emit(OpCode::Jump(condition_pc), line);

        // Break target = here
        let after_loop = self.bytecode.current_offset();
        self.bytecode.patch_jump(exit_jump, after_loop);

        // Emit LoopExit
        self.bytecode.emit(OpCode::LoopExit, line);

        // Patch LoopEnter with actual targets
        self.bytecode.patch_loop(loop_enter, next_pc, after_loop);

        let lctx = self.loops.pop().unwrap();
        for patch_idx in lctx.break_patches {
            self.bytecode.patch_jump(patch_idx, after_loop);
        }

        self.bytecode.emit(OpCode::PushEmpty, line);
    }

    /// `expr ...`
    fn compile_expr(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        if cmd.words.len() == 2 {
            self.compile_expr_word(&cmd.words[1], line);
        } else {
            for word in &cmd.words[1..] {
                self.compile_word(word, line);
            }
            let n = (cmd.words.len() - 1) as u16;
            if n > 1 {
                self.bytecode.emit(OpCode::Concat(n), line);
            }
            self.bytecode.emit(OpCode::EvalExpr, line);
        }
    }

    /// `incr varName ?increment?`
    fn compile_incr(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        if let Word::Literal(var_name) = &cmd.words[1] {
            let name_idx = self.bytecode.add_const(var_name);
            let amount = if cmd.words.len() >= 3 {
                if let Word::Literal(s) = &cmd.words[2] {
                    if let Ok(n) = s.parse::<i64>() {
                        n
                    } else {
                        // Dynamic increment amount — fall back to ECall
                        return self.compile_ecall_by_id(cmd, StdCmdId::Append as u16, line);
                    }
                } else {
                    // Dynamic expression for increment
                    return self.compile_dyncall(cmd);
                }
            } else {
                1
            };
            self.bytecode.emit(OpCode::IncrVar(name_idx, amount), line);
        } else {
            self.compile_dyncall(cmd);
        }
    }

    /// `return ?-code code? ?-level level? ?value?`
    fn compile_return(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        if cmd.words.len() == 1 {
            // Plain `return`
            self.bytecode.emit(OpCode::PushEmpty, line);
            self.bytecode.emit(OpCode::Return, line);
        } else if cmd.words.len() == 2 {
            // `return value` — common case
            if let Word::Literal(s) = &cmd.words[1] {
                if s == "-code" || s == "-level" {
                    // Has options — use ECall for full return handling
                    return self.compile_ecall_by_id(cmd, StdCmdId::Eval as u16, line);
                }
            }
            self.compile_word(&cmd.words[1], line);
            self.bytecode.emit(OpCode::Return, line);
        } else {
            // Complex return with options — parse -code/-level
            let mut has_code = false;
            let mut code_val: Option<i32> = None;
            let mut i = 1;
            while i < cmd.words.len() {
                if let Word::Literal(s) = &cmd.words[i] {
                    if s == "-code" && i + 1 < cmd.words.len() {
                        has_code = true;
                        if let Word::Literal(cv) = &cmd.words[i + 1] {
                            code_val = match cv.as_str() {
                                "ok" => Some(0),
                                "error" => Some(1),
                                "return" => Some(2),
                                "break" => Some(3),
                                "continue" => Some(4),
                                _ => cv.parse::<i32>().ok(),
                            };
                        }
                        i += 2;
                        continue;
                    } else if s == "-level" {
                        i += 2; // skip -level and its value
                        continue;
                    }
                }
                break;
            }
            // Remaining arg = value
            if i < cmd.words.len() {
                self.compile_word(&cmd.words[i], line);
            } else {
                self.bytecode.emit(OpCode::PushEmpty, line);
            }
            if has_code {
                if let Some(c) = code_val {
                    self.bytecode.emit(OpCode::ReturnCode(c), line);
                } else {
                    // Dynamic code — fall back to DynCall
                    self.bytecode.emit(OpCode::Pop, line);
                    self.compile_dyncall(cmd);
                }
            } else {
                self.bytecode.emit(OpCode::Return, line);
            }
        }
    }

    /// `exit ?code?`
    fn compile_exit(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        if cmd.words.len() <= 2 {
            let code = if cmd.words.len() == 2 {
                if let Word::Literal(s) = &cmd.words[1] {
                    s.parse::<i32>().unwrap_or(0)
                } else {
                    // Dynamic exit code — fall back
                    return self.compile_dyncall(cmd);
                }
            } else {
                0
            };
            self.bytecode.emit(OpCode::Exit(code), line);
        } else {
            self.compile_dyncall(cmd);
        }
    }

    // -----------------------------------------------------------------------
    // Tier 2 & 3 — ECall / SysCall
    // -----------------------------------------------------------------------

    /// Compile as an ECall (standard / language library command).
    fn compile_ecall(&mut self, cmd: &Command, name: &str) {
        let line = cmd.line as u32;
        let cmd_id = StdCmdId::from_name(name).unwrap() as u16;
        self.compile_ecall_by_id(cmd, cmd_id, line);
    }

    fn compile_ecall_by_id(&mut self, cmd: &Command, cmd_id: u16, line: u32) {
        let argc = cmd.words.len() as u16;
        for word in &cmd.words {
            match word {
                Word::Expand(inner) => {
                    self.compile_word(inner, line);
                    self.bytecode.emit(OpCode::ExpandList, line);
                }
                _ => self.compile_word(word, line),
            }
        }
        self.bytecode.emit(OpCode::ECall { cmd_id, argc }, line);
    }

    /// Compile as a SysCall (extension / platform command).
    fn compile_syscall(&mut self, cmd: &Command, name: &str) {
        let line = cmd.line as u32;
        let cmd_id = ExtCmdId::from_name(name).unwrap() as u16;
        let argc = cmd.words.len() as u16;
        for word in &cmd.words {
            match word {
                Word::Expand(inner) => {
                    self.compile_word(inner, line);
                    self.bytecode.emit(OpCode::ExpandList, line);
                }
                _ => self.compile_word(word, line),
            }
        }
        self.bytecode.emit(OpCode::SysCall { cmd_id, argc }, line);
    }

    // -----------------------------------------------------------------------
    // Tier 4 — DynCall (fully dynamic)
    // -----------------------------------------------------------------------

    fn compile_dyncall(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        let argc = cmd.words.len() as u16;
        for word in &cmd.words {
            match word {
                Word::Expand(inner) => {
                    self.compile_word(inner, line);
                    self.bytecode.emit(OpCode::ExpandList, line);
                }
                _ => self.compile_word(word, line),
            }
        }
        self.bytecode.emit(OpCode::DynCall { argc }, line);
    }

    // -----------------------------------------------------------------------
    // Body / expression compilation helpers
    // -----------------------------------------------------------------------

    /// Compile a word that represents a script body **inline**.
    ///
    /// For `Word::Literal` bodies (the common brace-quoted case), the
    /// string is parsed into commands and compiled recursively — the body
    /// runs as native opcodes instead of being re-parsed at runtime.
    ///
    /// For dynamic bodies, falls back to `EvalScript`.
    fn compile_body_inline(&mut self, word: &Word, line: u32) {
        match word {
            Word::Literal(s) => {
                if let Ok(commands) = crate::parse(s) {
                    if commands.is_empty() {
                        self.bytecode.emit(OpCode::PushEmpty, line);
                    } else {
                        self.compile_commands(&commands);
                    }
                } else {
                    // Parse failed — fall back to dynamic eval
                    let idx = self.bytecode.add_const(s);
                    self.bytecode.emit(OpCode::PushConst(idx), line);
                    self.bytecode.emit(OpCode::EvalScript, line);
                }
            }
            _ => {
                // Dynamic body — must eval at runtime
                self.compile_word(word, line);
                self.bytecode.emit(OpCode::EvalScript, line);
            }
        }
    }

    /// Compile a word that is an expression — evaluates via `eval_expr`.
    fn compile_expr_word(&mut self, word: &Word, line: u32) {
        match word {
            Word::Literal(s) => {
                let idx = self.bytecode.add_const(s);
                self.bytecode.emit(OpCode::PushConst(idx), line);
                self.bytecode.emit(OpCode::EvalExpr, line);
            }
            _ => {
                self.compile_word(word, line);
                self.bytecode.emit(OpCode::EvalExpr, line);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcode::OpCode;

    #[test]
    fn test_compile_set() {
        let bc = Compiler::compile_script("set x 10").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(10))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::StoreVar(_))));
    }

    #[test]
    fn test_compile_puts_syscall() {
        let bc = Compiler::compile_script("puts hello").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::SysCall { .. })));
    }

    #[test]
    fn test_compile_while_inline() {
        let bc = Compiler::compile_script("while {$x < 10} { incr x }").unwrap();
        let ops = bc.ops();
        // Should have LoopEnter/LoopExit instead of EvalScript for body
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoopEnter { .. })));
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoopExit)));
        // Body is compiled inline — incr becomes IncrVar
        assert!(ops.iter().any(|o| matches!(o, OpCode::IncrVar(_, 1))));
        // Condition still uses EvalExpr (for now)
        assert!(ops.iter().any(|o| matches!(o, OpCode::EvalExpr)));
    }

    #[test]
    fn test_compile_for_inline() {
        let bc = Compiler::compile_script("for {set i 0} {$i < 10} {incr i} { set x $i }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoopEnter { .. })));
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoopExit)));
    }

    #[test]
    fn test_compile_if_inline() {
        let bc = Compiler::compile_script("if {1} { puts yes } else { puts no }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
        // Bodies should be compiled inline (SysCall for puts)
        let syscall_count = ops.iter().filter(|o| matches!(o, OpCode::SysCall { .. })).count();
        assert_eq!(syscall_count, 2, "expected 2 SysCall for puts yes / puts no");
    }

    #[test]
    fn test_compile_ecall() {
        let bc = Compiler::compile_script("string length hello").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::ECall { .. })));
    }

    #[test]
    fn test_compile_dyncall() {
        let bc = Compiler::compile_script("$cmd arg1 arg2").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    #[test]
    fn test_compile_incr() {
        let bc = Compiler::compile_script("incr x").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::IncrVar(_, 1))));
    }

    #[test]
    fn test_bytecode_display() {
        let bc = Compiler::compile_script("set x 10\nputs $x").unwrap();
        let display = format!("{}", bc);
        assert!(display.contains("Instructions:"));
        assert!(display.contains("PUSH_INT 10"));
    }

    #[test]
    fn test_compile_break_continue() {
        let bc = Compiler::compile_script("break").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::Break)));

        let bc = Compiler::compile_script("continue").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::Continue)));
    }

    #[test]
    fn test_compile_return() {
        let bc = Compiler::compile_script("return 42").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::PushInt(42))));
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::Return)));
    }

    #[test]
    fn test_constant_dedup() {
        let bc = Compiler::compile_script("puts hello\nputs hello").unwrap();
        // "hello" and "puts" should each appear only once in the constant pool
        let hello_count = bc.constants().iter().filter(|c| *c == "hello").count();
        assert_eq!(hello_count, 1);
    }
}
