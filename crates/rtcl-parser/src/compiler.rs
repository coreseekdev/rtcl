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
//! 2. **Known built-in commands** (e.g. `string`, `list`, `foreach`,
//!    `proc`, `puts`, …) are compiled to `Call { cmd_id, argc }`.
//!    The `cmd_id` comes from [`CmdId`]; the VM dispatches through a
//!    unified function-pointer table.
//!
//! 3. **Unknown / dynamic commands** fall back to `DynCall { argc }`.

use crate::{Command, Word};
use crate::bytecode::{ByteCode, CmdSite, SrcSpan};
use crate::opcode::{OpCode, CmdId};
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Loop context — tracks the active loop during compilation so that `break`
// and `continue` inside inline-compiled bodies can be resolved.
// ---------------------------------------------------------------------------

struct LoopCtx {
    /// Index of the `LoopEnter` instruction (to be patched later).
    #[allow(dead_code)]
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
    /// The compilation unit's source text — every emitted [`CmdSite`] span
    /// points into it.  Inline-compiled bodies rebase their (body-relative)
    /// spans by [`Compiler::span_base`] so all sites share this one string.
    source: Rc<str>,
    /// Delta added to the AST spans of the text currently being compiled
    /// (0 at unit top level; body-value start offset inside a braced body).
    span_base: u32,
    /// Line number of the current text's first line (1-based Tcl lines;
    /// 0 at top level).  Absolute line = `line_base + command.line`.
    line_base: u32,
    /// True while compiling the *next* script region of an inline `for`:
    /// a literal `continue` there escapes the loop (for-8.2), so it
    /// compiles to `LoopExit` + `Continue` instead of the plain op.  Saved
    /// and cleared around nested loop compilations — their bodies' breaks
    /// and continues belong to the nested loop.
    in_next: bool,
    /// True while compiling a *procedure body*: parameters are seeded into
    /// the locals table and plain-name variable reads/writes of table
    /// names compile to slot ops (`LoadLocal`/`StoreLocal`/`IncrLocal`).
    /// The frame-side executor binds parameters positionally into slots
    /// and consults the same table from every name-keyed variable path,
    /// so uncompiled writers (foreach vars, lappend, catch results) stay
    /// correct.  Eval-level units never set this — their variables are
    /// globals/namespace vars, never frame slots.
    locals_mode: bool,
}

/// May a name of this shape live in a proc's compiled locals table?
/// Slot names are plain: no `::` qualification, no array reference, and
/// non-empty (a parameter list may legally contain stranger names — those
/// bind through the name-keyed store instead, and a proc with any such
/// parameter forgoes slots entirely; see the frame-side gate in `call.rs`).
pub fn slot_candidate(name: &str) -> bool {
    !name.is_empty() && !name.contains('(') && !name.contains("::")
}

impl Compiler {
    /// Compile a parsed unit together with its source text.  `commands`
    /// spans must point into `source` (as [`crate::ScriptUnit::parse`]
    /// produces).  Sites referencing the source are emitted per command.
    pub fn compile_unit(source: Rc<str>, commands: &[Command]) -> ByteCode {
        Self::compile_impl(source, commands, false, &[])
    }

    /// [`Self::compile_unit`] for a procedure body: `param_names` seed the
    /// locals table (slot i = the i-th name) and plain-name variable ops
    /// of table names compile to slot form.  The frame-side caller binds
    /// parameters positionally into slots — the order here is the contract.
    pub fn compile_unit_locals(
        source: Rc<str>,
        commands: &[Command],
        param_names: &[&str],
    ) -> ByteCode {
        Self::compile_impl(source, commands, true, param_names)
    }

    fn compile_impl(
        source: Rc<str>,
        commands: &[Command],
        locals_mode: bool,
        params: &[&str],
    ) -> ByteCode {
        let mut c = Compiler {
            bytecode: ByteCode::new(),
            loops: Vec::new(),
            source: Rc::clone(&source),
            span_base: 0,
            line_base: 0,
            in_next: false,
            locals_mode,
        };
        if locals_mode {
            for p in params {
                if slot_candidate(p) {
                    c.bytecode.add_local(p);
                }
            }
        }
        c.compile_commands(commands);
        c.bytecode.peephole();
        c.bytecode.source = source;
        c.bytecode.fallback = !c.bytecode.ops().iter().all(|op| matches!(op,
            OpCode::PushConst(_) | OpCode::PushConstWide(_) | OpCode::PushEmpty
            | OpCode::PushInt(_) | OpCode::PushFloat(_) | OpCode::PushTrue | OpCode::PushFalse
            | OpCode::Pop | OpCode::Dup
            | OpCode::LoadVar(_) | OpCode::StoreVar(_) | OpCode::StoreVarPop(_)
            | OpCode::LoadLocal(_) | OpCode::StoreLocal(_) | OpCode::IncrLocal(_, _)
            | OpCode::IncrVar(_, _)
            | OpCode::Concat(_) | OpCode::ExpandList | OpCode::ExpandMark
            | OpCode::Jump(_) | OpCode::JumpTrue(_) | OpCode::JumpFalse(_)
            | OpCode::LoopEnter { .. } | OpCode::LoopExit | OpCode::Break | OpCode::Continue
            | OpCode::Return | OpCode::Exit(_)
            | OpCode::EvalScript | OpCode::EvalExpr
            | OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | OpCode::Mod | OpCode::Pow
            | OpCode::Neg
            | OpCode::Eq | OpCode::Ne | OpCode::Lt | OpCode::Gt | OpCode::Le | OpCode::Ge
            | OpCode::StrEq | OpCode::StrNe
            | OpCode::Not
            | OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor | OpCode::BitNot
            | OpCode::Shl | OpCode::Shr
            | OpCode::BeginCmd(_) | OpCode::BodyMark | OpCode::NextMark
            | OpCode::SubMark | OpCode::SubEnd | OpCode::ExprMark
            | OpCode::Call { .. } | OpCode::CallExpand { .. }
            | OpCode::DynCall { .. } | OpCode::DynCallExpand { .. }
        ));
        c.bytecode
    }

    /// Compile a list of parsed commands into [`ByteCode`] (no source —
    /// sites carry empty spans).  Prefer [`Compiler::compile_unit`].
    pub fn compile(commands: &[Command]) -> ByteCode {
        Self::compile_unit(Rc::from(""), commands)
    }

    /// Compile a Tcl source string in one step (parse + compile).
    pub fn compile_script(source: &str) -> Result<ByteCode, crate::ParseError> {
        let commands = crate::parse(source)?;
        Ok(Self::compile_unit(Rc::from(source), &commands))
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

        let line = self.line_base + cmd.line as u32;

        // Record the command's source site and open its dispatch context —
        // the executor installs `sites[idx]` (text/line/word spans) as the
        // "current command" for errorInfo, replacing the tree-walk path's
        // per-command bookkeeping.
        let site_idx = self.bytecode.sites.len() as u32;
        let base = self.span_base;
        let word_srcs = if base == 0 {
            Rc::clone(&cmd.word_srcs)
        } else {
            Rc::new(
                cmd.word_srcs
                    .iter()
                    .map(|s| SrcSpan { start: s.start + base, end: s.end + base })
                    .collect(),
            )
        };
        self.bytecode.sites.push(CmdSite {
            text: SrcSpan { start: cmd.text.start + base, end: cmd.text.end + base },
            line,
            line_rel: cmd.line as u32,
            line_delta: self.line_base,
            word_srcs,
        });
        self.bytecode.emit(OpCode::BeginCmd(site_idx), line);

        // --- Specialised codegen for known commands (first word is literal) --
        // Shapes the compiler cannot express exactly degrade to Tier-2/3
        // dispatch of the real builtin (exact semantics over coverage).
        // Tier-1 folding assumes the name still resolves to the builtin at
        // runtime; the executor guards that with its invalidation epoch.
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
                    // Inside a `for` next script the continue ESCAPES the
                    // loop (for-8.2): leave this loop's context first, then
                    // hand the signal to the enclosing loop — a direct jump
                    // when one exists, the propagated error otherwise.
                    if self.in_next {
                        self.bytecode.emit(OpCode::LoopExit, line);
                    }
                    self.bytecode.emit(OpCode::Continue, line);
                    return;
                }
                "return" => return self.compile_return(cmd),
                "exit" => return self.compile_exit(cmd),

                // ── Tier 2: Call (known built-in command) ───────────────
                _ if CmdId::from_name(name).is_some() => {
                    return self.compile_call(cmd, name);
                }

                _ => {}
            }
        }

        // ── Tier 3: DynCall (unknown / dynamic) ────────────────────────
        self.compile_dyncall(cmd);
    }

    /// Absolute (unit-relative) 1-based line of `cmd`, accounting for any
    /// inline body being compiled.
    fn abs_line(&self, cmd: &Command) -> u32 {
        self.line_base + cmd.line as u32
    }

    // -----------------------------------------------------------------------
    // Word compilation
    // -----------------------------------------------------------------------

    /// Emit a read of variable `name`: the slot op when this unit's locals
    /// table holds the name (proc bodies only), the name-keyed op otherwise
    /// (the executor's name path consults the slot table at runtime, so
    /// both forms observe the same variable).
    fn emit_var_read(&mut self, name: &str, line: u32) {
        if self.locals_mode && slot_candidate(name) {
            if let Some(slot) = self.bytecode.find_local(name) {
                self.bytecode.emit(OpCode::LoadLocal(slot), line);
                return;
            }
        }
        let idx = self.bytecode.add_const(name);
        self.bytecode.emit(OpCode::LoadVar(idx), line);
    }

    /// Emit a scalar store to `name`.  Writes *discover*: a plain-name
    /// `set` target in a proc body gains a slot here (params seed the
    /// first slots; `set` targets append in compilation order — the same
    /// order the tree-walk would insert them at runtime).
    fn emit_var_store(&mut self, name: &str, line: u32) {
        if self.locals_mode && slot_candidate(name) {
            let slot = self.bytecode.add_local(name);
            self.bytecode.emit(OpCode::StoreLocal(slot), line);
            return;
        }
        let idx = self.bytecode.add_const(name);
        self.bytecode.emit(OpCode::StoreVar(idx), line);
    }

    fn compile_word(&mut self, word: &Word, line: u32) {
        match word {
            Word::Literal(s) => {
                if s.is_empty() {
                    self.bytecode.emit(OpCode::PushEmpty, line);
                } else if let Ok(n) = s.parse::<i64>() {
                    // PushInt(n) renders as `n.to_string()`, so only fold
                    // canonical decimal literals — `007` must stay "007",
                    // `+5` stays "+5" (tclsh set/return are string-exact).
                    if n.to_string() == *s {
                        self.bytecode.emit(OpCode::PushInt(n), line);
                    } else {
                        self.bytecode.emit_push_const(s, line);
                    }
                } else {
                    self.bytecode.emit_push_const(s, line);
                }
            }
            Word::VarRef(name) => {
                self.emit_var_read(name, line);
            }
            Word::CommandSub(script) => {
                self.bytecode.emit_push_const(script, line);
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
                self.bytecode.emit_push_const(expr, line);
                self.bytecode.emit(OpCode::EvalExpr, line);
            }
        }
    }

    /// Compile a word whose source span is known (`cmd.words[word_idx]`,
    /// rebased by the current [`Compiler::span_base`]).  This is the entry
    /// point that inlines `[...]` command substitutions: the bracket's
    /// script compiles into the unit between `SubMark`/`SubEnd`, exactly
    /// what tclsh's compiler does (no nested eval per execution).  Only a
    /// whole-word bracket can inline — the span must locate the content,
    /// so brackets embedded in concatenations keep `EvalScript`.
    fn compile_word_spanned(&mut self, word: &Word, line: u32, cmd: &Command, word_idx: usize) {
        if let Word::CommandSub(script) = word {
            if self.try_compile_sub(script, cmd, word_idx) {
                return;
            }
        }
        self.compile_word(word, line);
    }

    /// Inline a `[script]` word.  Returns false (compiling nothing) when
    /// the word is not verbatim bracket source — then the caller falls
    /// back to `PushConst` + `EvalScript`, whose nested `Interp::eval`
    /// owns the exact semantics (parse errors with their own frames,
    /// transparent bodies, traces).
    fn try_compile_sub(&mut self, script: &str, cmd: &Command, word_idx: usize) -> bool {
        let Some(ws) = cmd.word_srcs.get(word_idx) else {
            return false;
        };
        let abs = SrcSpan { start: ws.start + self.span_base, end: ws.end + self.span_base };
        let src = abs.slice(&self.source);
        if !(src.len() >= 2
            && src.starts_with('[')
            && src.ends_with(']')
            && &src[1..src.len() - 1] == script)
        {
            return false;
        }
        let Ok(commands) = crate::parse(script) else {
            return false;
        };
        let content_start = abs.start + 1;
        let saved = (self.span_base, self.line_base);
        self.span_base = content_start;
        // line_base is inherited unchanged: the tree-walk's nested eval
        // runs with the enclosing offset still installed, so a command on
        // the bracket's first line reports the same line either way.
        self.bytecode.emit(OpCode::SubMark, self.abs_line(cmd));
        if commands.is_empty() {
            self.bytecode.emit(OpCode::PushEmpty, self.abs_line(cmd));
        } else {
            self.compile_commands(&commands);
        }
        self.bytecode.emit(OpCode::SubEnd, self.abs_line(cmd));
        self.span_base = saved.0;
        self.line_base = saved.1;
        true
    }

    // -----------------------------------------------------------------------
    // Tier 1 — control-flow commands (native opcodes, inline bodies)
    // -----------------------------------------------------------------------

    /// `set varName value`
    fn compile_set(&mut self, cmd: &Command) {
        let line = self.abs_line(cmd);
        if let Word::Literal(name) = &cmd.words[1] {
            self.compile_word_spanned(&cmd.words[2], line, cmd, 2);
            self.emit_var_store(name, line);
        } else {
            // Dynamic var name — dispatch the real `set`
            self.compile_dyncall(cmd);
        }
    }

    /// `set varName` (read-only form)
    fn compile_set_get(&mut self, cmd: &Command) {
        let line = self.abs_line(cmd);
        if let Word::Literal(name) = &cmd.words[1] {
            self.emit_var_read(name, line);
        } else {
            self.compile_dyncall(cmd);
        }
    }

    /// `if expr ?then? body ?elseif expr ?then? body ...? ?else? ?body?`
    ///
    /// Compiled inline only when the word sequence matches the Tcl grammar
    /// exactly; any other shape (dangling `elseif`/`else`, unknown or
    /// dynamic keyword, missing bodies, words after the else body) is
    /// dispatched as a DynCall of the real `if` — tclsh's behavior for
    /// malformed `if` is a command-level arity/syntax error, which only
    /// `cmd_if` reproduces.  (The tree-walk path's old "implicit else"
    /// reading of unknown keywords was a deliberate divergence from tcl.)
    fn compile_if(&mut self, cmd: &Command) {
        // Structural validation: collect (expr_idx, body_idx) arms and an
        // optional else-body index.  Any deviation → DynCall.
        let n = cmd.words.len();
        let mut arms: Vec<(usize, usize)> = Vec::new();
        let mut else_body: Option<usize> = None;
        let mut i = 1usize;
        loop {
            if i >= n {
                return self.compile_dyncall(cmd); // dangling keyword / no args
            }
            let expr_idx = i;
            i += 1;
            if let Some(Word::Literal(kw)) = cmd.words.get(i) {
                if kw == "then" {
                    i += 1;
                }
            }
            if i >= n {
                return self.compile_dyncall(cmd); // condition without body
            }
            let body_idx = i;
            arms.push((expr_idx, body_idx));
            i += 1;
            match cmd.words.get(i) {
                None => break, // no else — false path pushes empty below
                Some(Word::Literal(kw)) if kw == "elseif" => {
                    i += 1;
                }
                Some(Word::Literal(kw)) if kw == "else" => {
                    i += 1;
                    if i + 1 != n {
                        return self.compile_dyncall(cmd); // missing body / trailing words
                    }
                    else_body = Some(i);
                    break;
                }
                Some(_) => return self.compile_dyncall(cmd), // not a keyword
            }
        }

        // Every arm body must inline verbatim; otherwise the real `if`
        // evaluates them (transparent-body error framing and all).
        for (_, body_idx) in &arms {
            if !self.body_inlinable(cmd, *body_idx) {
                return self.compile_dyncall(cmd);
            }
        }
        if let Some(bi) = else_body {
            if !self.body_inlinable(cmd, bi) {
                return self.compile_dyncall(cmd);
            }
        }

        let line = self.abs_line(cmd);
        let mut end_jumps = Vec::new();
        for (expr_idx, body_idx) in &arms {
            self.compile_expr_word(&cmd.words[*expr_idx], line, cmd, *expr_idx);
            let false_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);
            self.compile_body_inline(&cmd.words[*body_idx], cmd, *body_idx);
            let end_jump = self.bytecode.emit(OpCode::Jump(0), line);
            end_jumps.push(end_jump);
            let here = self.bytecode.current_offset();
            self.bytecode.patch_jump(false_jump, here);
        }
        match else_body {
            Some(bi) => {
                // The else body is entered by a *taken* jump (the last
                // arm's JumpFalse lands here), so the executor can't use
                // its not-taken rule to open the body region — mark it.
                // The trailing Jump gives the region the uniform plain-Jump
                // terminator every other inline body has.
                self.bytecode.emit(OpCode::BodyMark, line);
                self.compile_body_inline(&cmd.words[bi], cmd, bi);
                let skip = self.bytecode.emit(OpCode::Jump(0), line);
                let end = self.bytecode.current_offset();
                self.bytecode.patch_jump(skip, end);
            }
            // No else: the false path must still leave a value on the stack
            None => {
                self.bytecode.emit(OpCode::PushEmpty, line);
            }
        }
        let end = self.bytecode.current_offset();
        for j in end_jumps {
            self.bytecode.patch_jump(j, end);
        }
    }

    /// `while test body`
    ///
    /// Compiled inline only when the body is a braced verbatim literal
    /// that parses (the hot case); any other shape dispatches the real
    /// `while`, whose per-iteration eval — including its line-offset
    /// rebasing and frameless body errors — is the exact tree-walk
    /// semantics the executor's ops cannot express for opaque bodies.
    fn compile_while(&mut self, cmd: &Command) {
        // The condition word must be a verbatim literal: tclsh substitutes
        // the argument ONCE at dispatch, then re-evaluates the *result*
        // every iteration (compile-7.1: `while [expr {$i < 3}] {...}` runs
        // on the substituted "1" forever).  An inline EvalScript of a
        // substitution word would re-substitute per iteration.
        if !self.body_inlinable(cmd, 2) || !matches!(cmd.words[1], Word::Literal(_)) {
            return self.compile_dyncall(cmd);
        }
        // A loop owns the breaks/continues of its own scripts: clear any
        // enclosing next-region context for the whole compilation.
        let saved_next = self.in_next;
        self.in_next = false;
        let line = self.abs_line(cmd);

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
        self.compile_expr_word(&cmd.words[1], line, cmd, 1);
        let exit_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);

        // Compile the body inline (pre-checked above)
        self.compile_body_inline(&cmd.words[2], cmd, 2);
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

        self.in_next = saved_next;

        // While returns empty on normal exit
        self.bytecode.emit(OpCode::PushEmpty, line);
    }

    /// `for start test next body`
    ///
    /// Compiled inline like `while` when start/next/body are inlinable
    /// braced scripts and the test is a literal (same verbatim/parse
    /// gates) — tclsh compiles `for` inline the same way, and the
    /// dispatched `cmd_for` re-parses the test through the expression
    /// parser on *every* iteration, which is exactly the gap this closes.
    ///
    /// Layout:
    ///
    /// ```text
    ///   ExprMark                       ; opens the START harness region
    ///   <start>                        ; errors append the for's frame
    ///   SubEnd                         ; …and restore the for as current
    ///   LoopEnter { cont: NEXT, brk: END }
    /// COND:  <test> ; JumpFalse END        ; not-taken opens the body region
    ///   <body> ; Pop ; Jump NEXT
    /// NEXT:  NextMark                        ; opens the NEXT region
    ///   <next> ; Pop ; Jump COND
    /// END:   LoopExit ; PushEmpty
    /// ```
    ///
    /// The start script sits in a harness region (ExprMark) because
    /// cmd_for evaluates it with a plain eval whose errors cross the
    /// command's boundary as an APPENDED frame (`for {nosuch} {1} {} {}`
    /// logs the `for …` frame after `nosuch`'s), and the condition ops
    /// after it must attribute to the `for` itself, not to the start's
    /// last command.
    ///
    /// `continue` from the body jumps to NEXT (cont); from the next script
    /// it escapes (compiled `LoopExit`+`Continue` there, and the executor's
    /// loop-signal routing reads the NextMark region for signal-form
    /// continues, e.g. `eval continue`); `break` from either ends the loop.
    fn compile_for(&mut self, cmd: &Command) {
        if !matches!(cmd.words[2], Word::Literal(_))
            || !self.body_inlinable(cmd, 1)
            || !self.body_inlinable(cmd, 3)
            || !self.body_inlinable(cmd, 4)
        {
            return self.compile_dyncall(cmd);
        }
        // A loop owns the breaks/continues of its own scripts: clear any
        // enclosing next-region context for the whole compilation.
        let saved_next = self.in_next;
        self.in_next = false;
        let line = self.abs_line(cmd);

        // The start script runs before the loop context exists — its own
        // break/continue belong to the *enclosing* loop — inside a harness
        // region: its errors append the `for`'s frame on the way out
        // (cmd_for's plain start-eval boundary) and its close restores the
        // `for` as the current command for the condition that follows.
        self.bytecode.emit(OpCode::ExprMark, line);
        self.compile_body_inline(&cmd.words[1], cmd, 1);
        self.bytecode.emit(OpCode::SubEnd, line);
        self.bytecode.emit(OpCode::Pop, line);

        let loop_enter = self.bytecode.emit(OpCode::LoopEnter { cont: 0, brk: 0 }, line);
        self.loops.push(LoopCtx {
            enter_idx: loop_enter,
            continue_target: 0,
            break_patches: Vec::new(),
        });

        let condition_pc = self.bytecode.current_offset();

        self.compile_expr_word(&cmd.words[2], line, cmd, 2);
        let exit_jump = self.bytecode.emit(OpCode::JumpFalse(0), line);

        self.compile_body_inline(&cmd.words[4], cmd, 4);
        self.bytecode.emit(OpCode::Pop, line);
        let body_end_jump = self.bytecode.emit(OpCode::Jump(0), line);

        let next_pc = self.bytecode.current_offset();
        if let Some(lctx) = self.loops.last_mut() {
            lctx.continue_target = next_pc;
        }
        self.bytecode.emit(OpCode::NextMark, line);
        self.in_next = true;
        self.compile_body_inline(&cmd.words[3], cmd, 3);
        self.in_next = false;
        self.bytecode.emit(OpCode::Pop, line);
        self.bytecode.emit(OpCode::Jump(condition_pc), line);

        let end_pc = self.bytecode.current_offset();
        self.bytecode.patch_jump(exit_jump, end_pc);
        self.bytecode.patch_jump(body_end_jump, next_pc);
        self.bytecode.patch_loop(loop_enter, next_pc, end_pc);
        self.bytecode.emit(OpCode::LoopExit, line);

        let lctx = self.loops.pop().unwrap();
        for patch_idx in lctx.break_patches {
            self.bytecode.patch_jump(patch_idx, end_pc);
        }

        self.in_next = saved_next;

        // Tcl: loop commands always return the empty string
        self.bytecode.emit(OpCode::PushEmpty, line);
    }

    /// `expr ...`
    ///
    /// Only the single-argument form is folded (`expr $e`); multi-argument
    /// `expr a + b` joins its words with a space separator (cmd_expr), which
    /// `Concat` does not reproduce — dispatch the real `expr` instead.
    fn compile_expr(&mut self, cmd: &Command) {
        if cmd.words.len() == 2 {
            let line = self.abs_line(cmd);
            self.compile_expr_word(&cmd.words[1], line, cmd, 1);
        } else {
            self.compile_dyncall(cmd);
        }
    }

    /// `incr varName ?increment?`
    fn compile_incr(&mut self, cmd: &Command) {
        let line = self.abs_line(cmd);
        if cmd.words.len() > 3 {
            return self.compile_dyncall(cmd); // too many args — cmd_incr errors
        }
        if let Word::Literal(var_name) = &cmd.words[1] {
            let amount = if cmd.words.len() == 3 {
                if let Word::Literal(s) = &cmd.words[2] {
                    // Round-trip gate: `incr x 007` must add 7 (octal),
                    // `incr x +1` renders "+1" — fold only canonical ints.
                    match s.parse::<i64>() {
                        Ok(n) if n.to_string() == *s => n,
                        _ => return self.compile_dyncall(cmd),
                    }
                } else {
                    // Dynamic increment amount — dispatch the real `incr`
                    return self.compile_dyncall(cmd);
                }
            } else {
                1
            };
            // A plain-name `incr` target in a proc body takes a slot too
            // (writes discover, like `set`): the hot loop counter then
            // mutates the slot's int rep in place, no name probe at all.
            if self.locals_mode && slot_candidate(var_name) {
                let slot = self.bytecode.add_local(var_name);
                self.bytecode.emit(OpCode::IncrLocal(slot, amount), line);
            } else {
                let name_idx = self.bytecode.add_const(var_name);
                self.bytecode.emit(OpCode::IncrVar(name_idx, amount), line);
            }
        } else {
            self.compile_dyncall(cmd);
        }
    }

    /// `return ?-code code? ?-level level? ?value?`
    ///
    /// Only plain `return` / `return value` become the `Return` op; every
    /// option-bearing shape dispatches the real `return` — cmd_return's
    /// `-code`/`-level` semantics (level stripping, catch interaction) are
    /// far subtler than a `ReturnCode` opcode can express.
    fn compile_return(&mut self, cmd: &Command) {
        let line = self.abs_line(cmd);
        if cmd.words.len() == 1 {
            // Plain `return`
            self.bytecode.emit(OpCode::PushEmpty, line);
            self.bytecode.emit(OpCode::Return, line);
        } else if cmd.words.len() == 2 {
            // `return value` — unless the value is an option word
            // (`return -code` with a missing value must reach cmd_return
            // to produce its exact error).
            if let Word::Literal(s) = &cmd.words[1] {
                if s.starts_with('-') {
                    return self.compile_dyncall(cmd);
                }
            }
            self.compile_word(&cmd.words[1], line);
            self.bytecode.emit(OpCode::Return, line);
        } else {
            self.compile_dyncall(cmd);
        }
    }

    /// `exit ?code?`
    fn compile_exit(&mut self, cmd: &Command) {
        let line = self.abs_line(cmd);
        if cmd.words.len() <= 2 {
            // Round-trip gate: tcl_get_int accepts `exit 0x2` (2) and
            // `exit 010` (8); Exit(b) is the raw process code.  Fold only
            // canonical decimal literals.
            let code = if cmd.words.len() == 2 {
                match &cmd.words[1] {
                    Word::Literal(s) => match s.parse::<i32>() {
                        Ok(n) if n.to_string() == *s => n,
                        _ => return self.compile_dyncall(cmd),
                    },
                    _ => return self.compile_dyncall(cmd),
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
    // Tier 2 — Call (known built-in command, unified dispatch)
    // -----------------------------------------------------------------------

    /// Compile as a Call (known built-in command by [`CmdId`]).
    fn compile_call(&mut self, cmd: &Command, name: &str) {
        let line = self.abs_line(cmd);
        let cmd_id = CmdId::from_name(name).unwrap() as u16;
        self.compile_call_by_id(cmd, cmd_id, line);
    }

    fn compile_call_by_id(&mut self, cmd: &Command, cmd_id: u16, line: u32) {
        let argc = cmd.words.len() as u16;
        let has_expand = cmd.words.iter().any(|w| matches!(w, Word::Expand(_)));

        if has_expand {
            self.bytecode.emit(OpCode::ExpandMark, line);
        }

        for (word_idx, word) in cmd.words.iter().enumerate() {
            match word {
                Word::Expand(inner) => {
                    self.compile_word(inner, line);
                    self.bytecode.emit(OpCode::ExpandList, line);
                }
                _ => self.compile_word_spanned(word, line, cmd, word_idx),
            }
        }
        if has_expand {
            self.bytecode.emit(OpCode::CallExpand { cmd_id, argc }, line);
        } else {
            self.bytecode.emit(OpCode::Call { cmd_id, argc }, line);
        }
    }

    // -----------------------------------------------------------------------
    // Tier 3 — DynCall (fully dynamic)
    // -----------------------------------------------------------------------

    fn compile_dyncall(&mut self, cmd: &Command) {
        let line = self.abs_line(cmd);
        let argc = cmd.words.len() as u16;
        let has_expand = cmd.words.iter().any(|w| matches!(w, Word::Expand(_)));

        if has_expand {
            self.bytecode.emit(OpCode::ExpandMark, line);
        }

        for (word_idx, word) in cmd.words.iter().enumerate() {
            match word {
                Word::Expand(inner) => {
                    self.compile_word(inner, line);
                    self.bytecode.emit(OpCode::ExpandList, line);
                }
                _ => self.compile_word_spanned(word, line, cmd, word_idx),
            }
        }
        if has_expand {
            self.bytecode.emit(OpCode::DynCallExpand { argc }, line);
        } else {
            self.bytecode.emit(OpCode::DynCall { argc }, line);
        }
    }

    // -----------------------------------------------------------------------
    // Body / expression compilation helpers
    // -----------------------------------------------------------------------

    /// Can this body word be compiled inline?  True for empty literals and
    /// for braced verbatim literals whose text parses — the shapes the
    /// executor reproduces exactly.  Everything else keeps the construct on
    /// the DynCall path (`cmd_while`/`cmd_if` evaluate the value with the
    /// full tree-walk semantics, including transparent-body error framing).
    fn body_inlinable(&self, cmd: &Command, idx: usize) -> bool {
        match &cmd.words[idx] {
            Word::Literal(s) => {
                s.is_empty()
                    || (self.is_verbatim_braced(cmd, idx, s) && crate::parse(s).is_ok())
            }
            _ => false,
        }
    }

    /// Compile a word that represents a script body **inline**.
    ///
    /// For braced `Word::Literal` bodies (the common case) the body value is
    /// verbatim — identical to its source text between the braces — so it is
    /// parsed and compiled recursively with spans/lines rebased into the
    /// enclosing unit ([`Compiler::span_base`]/[`Compiler::line_base`]),
    /// keeping every emitted site pointing into the one shared source.
    ///
    /// Callers gate with [`Compiler::body_inlinable`]; non-verbatim bodies
    /// fall back to `EvalScript` here only defensively.
    fn compile_body_inline(&mut self, word: &Word, cmd: &Command, word_idx: usize) {
        if let Word::Literal(s) = word {
            if s.is_empty() {
                self.bytecode.emit(OpCode::PushEmpty, self.abs_line(cmd));
                return;
            }
            if self.is_verbatim_braced(cmd, word_idx, s) {
                if let Ok(commands) = crate::parse(s) {
                    if commands.is_empty() {
                        self.bytecode.emit(OpCode::PushEmpty, self.abs_line(cmd));
                    } else {
                        // Rebase into the unit: the body value starts just
                        // after the opening brace; its first line is the
                        // source line holding that byte.
                        let body_start =
                            self.span_base + cmd.word_srcs[word_idx].start + 1;
                        let saved = (self.span_base, self.line_base);
                        self.span_base = body_start;
                        self.line_base = count_lines(&self.source, body_start as usize);
                        self.compile_commands(&commands);
                        self.span_base = saved.0;
                        self.line_base = saved.1;
                    }
                    return;
                }
                // Parse failed — runtime eval reproduces the exact parse
                // error and its errorInfo frames.
            }
        }
        // Dynamic / non-verbatim / unparseable body — eval at runtime.
        let line = self.abs_line(cmd);
        self.compile_word(word, line);
        self.bytecode.emit(OpCode::EvalScript, line);
    }

    /// True when `cmd.words[word_idx]` was written as a braced literal whose
    /// value equals its source text minus the braces — the condition under
    /// which body spans can be rebased into the enclosing unit.
    fn is_verbatim_braced(&self, cmd: &Command, word_idx: usize, value: &str) -> bool {
        match cmd.word_srcs.get(word_idx) {
            Some(ws) => {
                let abs = SrcSpan {
                    start: ws.start + self.span_base,
                    end: ws.end + self.span_base,
                };
                let src = abs.slice(&self.source);
                src.len() >= 2
                    && src.starts_with('{')
                    && src.ends_with('}')
                    && &src[1..src.len() - 1] == value
            }
            None => false,
        }
    }

    /// Compile a word that is an expression — evaluates via `eval_expr`.
    ///
    /// For `Word::Literal` expressions that contain only variables, integers,
    /// and basic operators, compiles to native comparison/arithmetic opcodes.
    /// Falls back to `PushConst + EvalExpr` for complex expressions.
    fn compile_expr_word(&mut self, word: &Word, line: u32, cmd: &Command, word_idx: usize) {
        match word {
            Word::Literal(s) => {
                // Try inline compilation first.  The expression compiler
                // emits speculatively and can fail mid-expression (`1 && 0`:
                // PushInt lands, then `&&` is rejected) — roll the unit back
                // before the fallback, or the orphan value misaligns the
                // stack (harmless when the expression is a whole unit;
                // fatal when it compiles inline inside a word).  Inlining a
                // `[...]` operand also pushes sites for its inner commands,
                // so the rollback covers both ops and sites.
                let ops_mark = self.bytecode.ops().len();
                let sites_mark = self.bytecode.sites.len();
                // expr_base: the expression text's byte offset within the
                // unit — `Some(_)` only for a verbatim-braced word, the
                // condition under which a bracket operand's content span
                // can rebase into the unit.
                let expr_base = self
                    .is_verbatim_braced(cmd, word_idx, s)
                    .then(|| self.span_base + cmd.word_srcs[word_idx].start + 1);
                if crate::expr_compile::try_compile_expr(self, s, line, expr_base) {
                    return;
                }
                self.bytecode.truncate_ops(ops_mark);
                self.bytecode.sites.truncate(sites_mark);
                // Fallback: runtime eval
                self.bytecode.emit_push_const(s, line);
                self.bytecode.emit(OpCode::EvalExpr, line);
            }
            _ => {
                self.compile_word_spanned(word, line, cmd, word_idx);
                self.bytecode.emit(OpCode::EvalExpr, line);
            }
        }
    }
}

impl crate::expr_compile::ExprSink for Compiler {
    fn emit(&mut self, op: OpCode, line: u32) {
        self.bytecode.emit(op, line);
    }

    fn add_const(&mut self, name: &str) -> u16 {
        self.bytecode.add_const(name)
    }

    /// A `$var` operand of a compiled expression reads through the slot
    /// when the locals table holds it — the loop-condition counter case.
    fn var_read(&mut self, name: &str, line: u32) {
        self.emit_var_read(name, line)
    }

    /// Inline a `[...]` operand of the expression being compiled:
    /// `abs_start` is the bracket content's absolute start in the unit
    /// source.  Returns false (compiling nothing) when the script does not
    /// parse — the caller abandons the whole expression to the runtime
    /// evaluator, whose nested eval owns the parse-error framing.
    fn emit_sub(&mut self, script: &str, abs_start: u32, line: u32) -> bool {
        let Ok(commands) = crate::parse(script) else {
            return false;
        };
        let saved = self.span_base;
        self.span_base = abs_start;
        // line_base is inherited unchanged — same rule as whole-word
        // brackets: the tree-walk's nested eval runs with the enclosing
        // offset still installed.
        self.bytecode.emit(OpCode::ExprMark, line);
        if commands.is_empty() {
            self.bytecode.emit(OpCode::PushEmpty, line);
        } else {
            self.compile_commands(&commands);
        }
        self.bytecode.emit(OpCode::SubEnd, line);
        self.span_base = saved;
        true
    }
}

/// Number of newlines before byte offset `off` — the 0-based index of the
/// line holding `off` (Tcl lines are 1-based, so the line number is this + 1).
fn count_lines(src: &str, off: usize) -> u32 {
    src.as_bytes()[..off].iter().filter(|&&b| b == b'\n').count() as u32
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
    fn test_compile_puts_call() {
        let bc = Compiler::compile_script("puts hello").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Call { .. })));
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
        // Condition is compiled inline — LoadVar + PushInt(10) + Lt
        assert!(ops.iter().any(|o| matches!(o, OpCode::Lt)));
        // Should NOT have EvalExpr (expression was compiled inline)
        assert!(!ops.iter().any(|o| matches!(o, OpCode::EvalExpr)));
    }

    #[test]
    fn test_compile_for_inline() {
        // `for` compiles inline like `while` (tclsh does the same): the
        // test expression becomes native ops (no per-iteration ExprParser
        // run), the next script opens a NEXT_MARK region so a `continue`
        // there escapes the loop (for-8.2), and a literal continue inside
        // that region emits LoopExit+Continue.
        let bc = Compiler::compile_script("for {set i 0} {$i < 10} {incr i} { set x $i }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoopEnter { .. })));
        assert!(ops.iter().any(|o| matches!(o, OpCode::LoopExit)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::NextMark)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::IncrVar(_, 1))));
        // Condition compiled inline — no EvalExpr fallback.
        assert!(!ops.iter().any(|o| matches!(o, OpCode::EvalExpr)));
        assert!(!ops.iter().any(|o| matches!(o, OpCode::DynCall { .. })));

        // Non-inlinable shapes (unbraced next script) still dispatch the
        // real cmd_for.
        let bc = Compiler::compile_script("for {set i 0} {$i < 10} $next { set x $i }").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));

        // A literal `continue` in the next script leaves the loop first.
        let bc = Compiler::compile_script("for {set i 0} {$i < 10} {continue} { set x $i }").unwrap();
        let ops = bc.ops();
        let cont = ops.iter().position(|o| matches!(o, OpCode::Continue)).unwrap();
        assert!(
            ops[..cont].iter().any(|o| matches!(o, OpCode::LoopExit)),
            "continue in a next script must be preceded by LoopExit"
        );
    }

    #[test]
    fn test_compile_if_inline() {
        let bc = Compiler::compile_script("if {1} { puts yes } else { puts no }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
        // Bodies should be compiled inline (Call for puts)
        let call_count = ops.iter().filter(|o| matches!(o, OpCode::Call { .. })).count();
        assert_eq!(call_count, 2, "expected 2 Call for puts yes / puts no");
    }

    #[test]
    fn test_compile_call_string() {
        let bc = Compiler::compile_script("string length hello").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::Call { .. })));
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

    #[test]
    fn test_push_const_wide() {
        // Verify emit_push_const uses PushConst for small indices
        let mut bc = crate::ByteCode::new();
        bc.emit_push_const("hello", 1);
        assert!(matches!(bc.ops()[0], OpCode::PushConst(0)));
        assert_eq!(bc.get_const(0), Some("hello"));

        // Verify add_const_wide returns correct wide index
        let mut bc2 = crate::ByteCode::new();
        let idx = bc2.add_const_wide("test");
        assert_eq!(idx, 0);
        assert_eq!(bc2.get_const_wide(0), Some("test"));
    }

    // -- exactness gates ----------------------------------------------------

    #[test]
    fn test_if_no_else_pushes_empty() {
        // `if 0 { set y 1 }` — false path must leave a value on the stack.
        let bc = Compiler::compile_script("if 0 { set y 1 }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
        // The converge region after the arm's Jump pushes a value.
        let jump_pos = ops.iter().position(|o| matches!(o, OpCode::Jump(_))).unwrap();
        assert!(
            ops[jump_pos + 1..].iter().any(|o| matches!(o, OpCode::PushEmpty)),
            "expected PushEmpty after the then-arm Jump"
        );
    }

    #[test]
    fn test_if_implicit_else_degrades_to_dyncall() {
        // tclsh does NOT read an unknown word as an implicit else body.
        let bc = Compiler::compile_script("if 1 { puts a } { puts b }").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
        assert!(!bc.ops().iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
    }

    #[test]
    fn test_if_dangling_else_degrades_to_dyncall() {
        let bc = Compiler::compile_script("if 1 { puts a } else").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    #[test]
    fn test_if_trailing_words_after_else_degrade_to_dyncall() {
        let bc = Compiler::compile_script("if 1 { puts a } else { puts b } { puts c }").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    #[test]
    fn test_if_dangling_elseif_degrades_to_dyncall() {
        let bc = Compiler::compile_script("if 0 { puts a } elseif").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    #[test]
    fn test_if_then_keyword_compiles() {
        let bc = Compiler::compile_script("if 1 then { puts a } else { puts b }").unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::JumpFalse(_))));
        assert!(!ops.iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    #[test]
    fn test_if_dynamic_keyword_degrades_to_dyncall() {
        // `$kw` could substitute to "else" at runtime — only the real
        // command handles that.
        let bc = Compiler::compile_script("if 0 { puts a } $kw { puts b }").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    #[test]
    fn test_return_options_degrade_to_dyncall() {
        for src in ["return -code error foo", "return -code", "return -level 2 x"] {
            let bc = Compiler::compile_script(src).unwrap();
            assert!(
                bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })),
                "{src} should dispatch the real return"
            );
            assert!(
                !bc.ops().iter().any(|o| matches!(o, OpCode::Return)),
                "{src} must not use the Return op"
            );
        }
    }

    #[test]
    fn test_exit_literal_gates() {
        let bc = Compiler::compile_script("exit 2").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::Exit(2))));

        // Non-canonical integer forms reach cmd_exit (tcl_get_int: 0x2 → 2,
        // 010 → 8, and an empty/unparseable code has its own error).
        for src in ["exit 0x2", "exit 010", "exit +2"] {
            let bc = Compiler::compile_script(src).unwrap();
            assert!(
                !bc.ops().iter().any(|o| matches!(o, OpCode::Exit(_))),
                "{src} must not fold the exit code"
            );
        }
    }

    #[test]
    fn test_incr_literal_gates() {
        let bc = Compiler::compile_script("incr x 2").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::IncrVar(_, 2))));

        // 007 is octal 7 to tcl_get_int — must not fold as decimal 7.
        let bc = Compiler::compile_script("incr x 007").unwrap();
        assert!(!bc.ops().iter().any(|o| matches!(o, OpCode::IncrVar(_, _))));
    }

    #[test]
    fn test_non_canonical_int_literal_stays_const() {
        let bc = Compiler::compile_script("set x 007").unwrap();
        assert!(
            !bc.ops().iter().any(|o| matches!(o, OpCode::PushInt(_))),
            "007 must not become PushInt(7)"
        );
        let bc = Compiler::compile_script("set x +5").unwrap();
        assert!(
            !bc.ops().iter().any(|o| matches!(o, OpCode::PushInt(_))),
            "+5 must not become PushInt(5)"
        );
        let bc = Compiler::compile_script("set x 10").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::PushInt(10))));
    }

    #[test]
    fn test_multi_word_expr_degrades_to_dyncall() {
        // `expr 1 + 2` joins with spaces (cmd_expr); Concat has no separator.
        let bc = Compiler::compile_script("expr 1 + 2").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
        let bc = Compiler::compile_script("expr {1 + 2}").unwrap();
        assert!(!bc.ops().iter().any(|o| matches!(o, OpCode::DynCall { .. })));
    }

    // -- sites ---------------------------------------------------------------

    #[test]
    fn test_sites_emitted_per_command() {
        let src = "set x 10\nputs $x";
        let bc = Compiler::compile_script(src).unwrap();
        assert_eq!(bc.sites.len(), 2);
        assert_eq!(bc.sites[0].line, 1);
        assert_eq!(bc.sites[1].line, 2);
        assert_eq!(bc.sites[0].text.slice(&bc.source), "set x 10");
        assert_eq!(bc.sites[1].text.slice(&bc.source), "puts $x");
        // Word spans are shared (base 0) and slice against the unit source.
        assert_eq!(bc.sites[0].word_srcs[0].slice(&bc.source), "set");
    }

    #[test]
    fn test_compile_sub_inline() {
        // A whole-word bracket compiles inline: SubMark/SubEnd bracket the
        // script's own compiled commands, and the inner command's site
        // points back into the unit source (rebased spans).
        let src = "set s [expr {6 * 7}]";
        let bc = Compiler::compile_script(src).unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::SubMark)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::SubEnd)));
        assert!(!ops.iter().any(|o| matches!(o, OpCode::EvalScript)));
        let expr_site = bc
            .sites
            .iter()
            .find(|s| s.text.slice(&bc.source) == "expr {6 * 7}")
            .expect("expr site with unit-resolvable text");
        assert_eq!(expr_site.line, 1);

        // Multi-line bracket: the inner command keeps its physical line.
        let src = "set x [\n    nosuch a\n]";
        let bc = Compiler::compile_script(src).unwrap();
        let site = bc
            .sites
            .iter()
            .find(|s| s.text.slice(&bc.source) == "nosuch a")
            .expect("inner site");
        assert_eq!(site.line, 2);

        // Brackets embedded in a concatenation have no locatable span —
        // they keep the runtime EvalScript.
        let bc = Compiler::compile_script("set x a[foo]b").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::EvalScript)));

    }

    #[test]
    fn test_compile_expr_bracket_inline() {
        // Brackets as operands of a compiled expression inline too
        // (ExprMark/SubEnd) — the classic fib recursion shape.
        let src = "set s [expr {[f 1] + [f 2]}]";
        let bc = Compiler::compile_script(src).unwrap();
        let ops = bc.ops();
        assert!(ops.iter().any(|o| matches!(o, OpCode::ExprMark)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::Add)));
        assert!(!ops.iter().any(|o| matches!(o, OpCode::EvalExpr)));
        assert!(!ops.iter().any(|o| matches!(o, OpCode::EvalScript)));

        // Inner command sites rebase into the unit source.
        let f_site = bc
            .sites
            .iter()
            .find(|s| s.text.slice(&bc.source) == "f 1")
            .expect("rebased inner site");
        assert_eq!(f_site.line, 1);

        // A bracket inside a multi-line expression: the collected script
        // starts at `[` (no leading newline), so the inner command's line
        // follows the enclosing command's line — exactly what the
        // tree-walk's nested eval produces (line_base inherited).
        let src = "set x [expr {\n    [nosuch a]\n    + 1}]";
        let bc = Compiler::compile_script(src).unwrap();
        let site = bc
            .sites
            .iter()
            .find(|s| s.text.slice(&bc.source) == "nosuch a")
            .expect("inner site");
        assert_eq!(site.line, 1);

        // Failure mid-expression rolls back the inlined bracket AND its
        // sites before the EvalExpr fallback (the `&&` rejection leaves
        // no orphan ops or sites behind).
        let src = "set q [expr {[f 1] && 0}]";
        let bc = Compiler::compile_script(src).unwrap();
        let ops = bc.ops();
        assert!(!ops.iter().any(|o| matches!(o, OpCode::ExprMark)));
        assert!(ops.iter().any(|o| matches!(o, OpCode::EvalExpr)));
        assert!(!bc.sites.iter().any(|s| s.text.slice(&bc.source) == "f 1"));

        // A non-verbatim expr word (substitution) keeps the runtime eval.
        let bc = Compiler::compile_script("set e {1 + 1}; expr $e").unwrap();
        assert!(bc.ops().iter().any(|o| matches!(o, OpCode::EvalExpr)));
    }

    #[test]
    fn test_inline_body_spans_rebased() {
        let src = "while {$i < 2} {\n  incr i\n}";
        let bc = Compiler::compile_script(src).unwrap();
        // The inlined `incr i` command's site must point back into the unit
        // source at its original position, with the original line number.
        let incr_site = bc
            .sites
            .iter()
            .find(|s| s.text.slice(&bc.source) == "incr i")
            .expect("incr site with unit-resolvable text");
        assert_eq!(incr_site.line, 2);
        assert_eq!(incr_site.word_srcs[0].slice(&bc.source), "incr");
        // The while command's own site keeps its top-level identity.
        assert_eq!(bc.sites[0].line, 1);
        assert_eq!(bc.sites[0].text.slice(&bc.source), "while {$i < 2} {\n  incr i\n}");
    }
}
