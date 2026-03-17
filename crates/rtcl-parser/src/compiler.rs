//! Compiler — transforms a [`crate`] AST into [`ByteCode`].
//!
//! The compiler performs a single forward pass over the command list:
//!
//! 1. Each [`Command`] is translated into a sequence of push/invoke opcodes.
//! 2. Recognised built-in patterns (`set`, `if`, `while`, `for`, `expr`, …)
//!    get specialised code generation that avoids dynamic dispatch.
//! 3. Everything else falls back to `InvokeDynamic`.

use crate::{Command, Word};

use crate::bytecode::ByteCode;
use crate::opcode::OpCode;

/// Compiler state.
pub struct Compiler {
    bytecode: ByteCode,
}

impl Compiler {
    /// Compile a list of parsed commands into [`ByteCode`].
    pub fn compile(commands: &[Command]) -> ByteCode {
        let mut c = Compiler {
            bytecode: ByteCode::new(),
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
    // Internal
    // -----------------------------------------------------------------------

    fn compile_commands(&mut self, commands: &[Command]) {
        for cmd in commands {
            self.compile_command(cmd);
        }
    }

    fn compile_command(&mut self, cmd: &Command) {
        if cmd.words.is_empty() {
            return;
        }

        let line = cmd.line as u32;
        self.bytecode.emit(OpCode::Line(line), line);

        // Try specialised codegen for well-known commands whose first word is
        // a literal.
        if let Word::Literal(name) = &cmd.words[0] {
            match name.as_str() {
                "set" if cmd.words.len() == 3 => {
                    return self.compile_set(cmd);
                }
                "if" => {
                    return self.compile_if(cmd);
                }
                "while" if cmd.words.len() == 3 => {
                    return self.compile_while(cmd);
                }
                "for" if cmd.words.len() == 5 => {
                    return self.compile_for(cmd);
                }
                "expr" => {
                    return self.compile_expr(cmd);
                }
                "incr" if cmd.words.len() >= 2 => {
                    return self.compile_incr(cmd);
                }
                "break" => {
                    self.bytecode.emit(OpCode::Break, line);
                    return;
                }
                "continue" => {
                    self.bytecode.emit(OpCode::Continue, line);
                    return;
                }
                "return" => {
                    if cmd.words.len() > 1 {
                        self.compile_word(&cmd.words[1], line);
                    } else {
                        self.bytecode.emit(OpCode::PushEmpty, line);
                    }
                    self.bytecode.emit(OpCode::Return, line);
                    return;
                }
                _ => {}
            }
        }

        // Generic path: push all words, then invoke dynamically.
        self.compile_generic_command(cmd);
    }

    /// Generic command: push each word onto the stack, then `InvokeDynamic`.
    fn compile_generic_command(&mut self, cmd: &Command) {
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

        self.bytecode.emit(OpCode::InvokeDynamic { argc }, line);
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
                self.bytecode.emit(OpCode::LoadGlobal(idx), line);
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
    // Specialised codegen
    // -----------------------------------------------------------------------

    /// `set varName value`
    fn compile_set(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        // Push the value
        self.compile_word(&cmd.words[2], line);
        // Store into variable
        if let Word::Literal(name) = &cmd.words[1] {
            let idx = self.bytecode.add_const(name);
            self.bytecode.emit(OpCode::StoreGlobal(idx), line);
        } else {
            // Variable name is dynamic — fall back to generic
            self.compile_generic_command(cmd);
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
                            // Compile the else body
                            i += 1;
                            if i < cmd.words.len() {
                                self.compile_body_word(&cmd.words[i], line);
                            }
                            break;
                        }
                        _ => {
                            // implicit else body
                            self.compile_body_word(&cmd.words[i], line);
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

            // Compile the condition as an eval-script
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

            // Compile the then-body
            if i < cmd.words.len() {
                self.compile_body_word(&cmd.words[i], line);
            }
            i += 1;

            let end_jump = self.bytecode.emit(OpCode::Jump(0), line);
            end_jumps.push(end_jump);

            // Patch false jump to here
            let here = self.bytecode.current_offset();
            self.bytecode.patch_jump(false_jump, here);
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
        let loop_start = self.bytecode.current_offset();

        // Compile the test expression
        self.compile_expr_word(&cmd.words[1], line);
        let exit_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);

        // Compile the body
        self.compile_body_word(&cmd.words[2], line);
        self.bytecode.emit(OpCode::Pop, line); // discard body result

        // Jump back to loop start
        self.bytecode.emit(OpCode::Jump(loop_start), line);

        // Patch exit
        let after = self.bytecode.current_offset();
        self.bytecode.patch_jump(exit_jump, after);
    }

    /// `for start test next body`
    fn compile_for(&mut self, cmd: &Command) {
        let line = cmd.line as u32;

        // Compile "start"
        self.compile_body_word(&cmd.words[1], line);
        self.bytecode.emit(OpCode::Pop, line);

        let loop_start = self.bytecode.current_offset();

        // Compile "test"
        self.compile_expr_word(&cmd.words[2], line);
        let exit_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);

        // Compile "body"
        self.compile_body_word(&cmd.words[4], line);
        self.bytecode.emit(OpCode::Pop, line);

        // Compile "next"
        self.compile_body_word(&cmd.words[3], line);
        self.bytecode.emit(OpCode::Pop, line);

        self.bytecode.emit(OpCode::Jump(loop_start), line);

        let after = self.bytecode.current_offset();
        self.bytecode.patch_jump(exit_jump, after);
    }

    /// `expr ...` — pushes expression args as a single string for eval.
    fn compile_expr(&mut self, cmd: &Command) {
        let line = cmd.line as u32;
        // Concatenate all expression arguments
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
            // The concat result is the expression string — would need eval
            self.bytecode.emit(OpCode::EvalExpr, line);
        }
    }

    /// `incr varName ?increment?`
    fn compile_incr(&mut self, cmd: &Command) {
        let _line = cmd.line as u32;
        // Fall back to generic for now — can be optimised later with
        // LoadGlobal + PushInt + Add + StoreGlobal.
        self.compile_generic_command(cmd);
    }

    // -- helpers ------------------------------------------------------------

    /// Compile a word that represents a script body: push as const + EvalScript.
    fn compile_body_word(&mut self, word: &Word, line: u32) {
        match word {
            Word::Literal(s) => {
                let idx = self.bytecode.add_const(s);
                self.bytecode.emit(OpCode::PushConst(idx), line);
                self.bytecode.emit(OpCode::EvalScript, line);
            }
            _ => {
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
        // Should contain: Line, PushInt(10), StoreGlobal(x_idx)
        assert!(ops.iter().any(|o| matches!(o, OpCode::PushInt(10))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::StoreGlobal(_))));
    }

    #[test]
    fn test_compile_generic() {
        let bc = Compiler::compile_script("puts hello").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::InvokeDynamic { .. })));
    }

    #[test]
    fn test_compile_while() {
        let bc = Compiler::compile_script("while {$x < 10} { incr x }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Jump(_))));
    }

    #[test]
    fn test_compile_if() {
        let bc = Compiler::compile_script("if {1} { puts yes } else { puts no }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
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
