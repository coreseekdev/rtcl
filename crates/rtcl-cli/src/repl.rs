//! REPL line assembly and tclsh-parity output helpers.
//!
//! Behavioral oracle: tclsh 8.6.17 interactive mode, probed side by side in
//! `docs/REPL-TCLSH-PARITY.md`.  Summary of the decisions encoded here:
//!
//! - primary prompt `% ` (tclsh); continuation prompt `... ` is a deliberate
//!   rtcl addition — tclsh prints nothing on continuation lines, but a visible
//!   marker keeps multi-line state legible in a scrollback terminal.
//! - uncaught top-level `break`/`continue` render as the tclsh error messages
//!   (`invoked "break" outside of a loop`); `return` echoes its value like any
//!   result; `exit` stops the loop and maps to the process exit code.

use rtcl_core::Error;

pub const PRIMARY_PROMPT: &str = "% ";
pub const CONTINUATION_PROMPT: &str = "... ";

pub fn prompt_for(buffer: &str) -> &'static str {
    if buffer.is_empty() {
        PRIMARY_PROMPT
    } else {
        CONTINUATION_PROMPT
    }
}

/// Accumulates REPL lines until `rtcl_parser::is_complete` accepts the
/// buffer, mirroring tclsh's interactive read loop (which keeps reading
/// through unclosed `"`, `{`, `[` and backslash-newline).
#[derive(Default)]
pub struct ReplInput {
    buffer: String,
}

impl ReplInput {
    pub fn new() -> Self {
        ReplInput { buffer: String::new() }
    }

    /// Lines accumulated so far (empty when waiting for a fresh command).
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Drop any partial input (Ctrl-C at a continuation prompt).
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Push one line (no trailing newline).  Returns the completed script
    /// and resets, or `None` while the input is still open.  Continuation
    /// lines join with `\n`, matching tclsh.
    pub fn feed(&mut self, line: &str) -> Option<String> {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
        if rtcl_parser::is_complete(&self.buffer) {
            Some(std::mem::take(&mut self.buffer))
        } else {
            None
        }
    }
}

/// tclsh-parity rendering for an uncaught top-level control-flow error.
/// Returns `None` when the caller should treat the error as a real one
/// (plain message output), or for `exit` (handled by the loop itself).
pub fn control_flow_text(err: &Error) -> Option<String> {
    if err.is_break() {
        Some("invoked \"break\" outside of a loop".to_string())
    } else if err.is_continue() {
        Some("invoked \"continue\" outside of a loop".to_string())
    } else if err.is_return() {
        // tclsh echoes a top-level return's value like any other result;
        // a bare `return` echoes nothing.
        Some(err.message_text())
    } else {
        None
    }
}

/// Process exit code carried by a top-level `exit` (`exit` → 0,
/// `exit 7` → 7; a non-integer argument falls back to 0 like tclsh's
/// "expected integer" fallback path).
pub fn exit_code_of(err: &Error) -> i32 {
    err.message_text().trim().parse::<i32>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtcl_core::Interp;

    #[test]
    fn prompt_switches_between_primary_and_continuation() {
        assert_eq!(prompt_for(""), PRIMARY_PROMPT);
        assert_eq!(prompt_for("set x {"), CONTINUATION_PROMPT);
    }

    #[test]
    fn feed_collects_through_unclosed_brace() {
        let mut input = ReplInput::new();
        assert_eq!(input.feed("set x {a"), None);
        assert_eq!(input.buffer(), "set x {a");
        assert_eq!(input.feed("b"), None);
        assert_eq!(input.feed("c}"), Some("set x {a\nb\nc}".to_string()));
        assert!(input.is_empty());
    }

    #[test]
    fn feed_collects_through_unclosed_quote_and_bracket() {
        let mut input = ReplInput::new();
        assert_eq!(input.feed(r#"puts "unclosed"#), None);
        assert_eq!(
            input.feed(r#"more""#),
            Some("puts \"unclosed\nmore\"".to_string())
        );

        let mut input = ReplInput::new();
        assert_eq!(input.feed("set i [expr 1 +"), None);
        assert_eq!(input.feed("2]"), Some("set i [expr 1 +\n2]".to_string()));
    }

    #[test]
    fn feed_single_complete_line_passes_through() {
        let mut input = ReplInput::new();
        assert_eq!(input.feed("set a 1"), Some("set a 1".to_string()));
    }

    #[test]
    fn backslash_edges_match_tclsh_info_complete() {
        // Bare trailing backslash is complete (tclsh info complete → 1):
        // the REPL hands it straight to eval like tclsh does.
        let mut input = ReplInput::new();
        assert_eq!(input.feed("set a 1 \\"), Some("set a 1 \\".to_string()));

        // Backslash-newline inside an unclosed brace keeps the read open
        // until the brace closes (parseOld-15.2: `info complete "abc\<NL>"`
        // → 0 at end of input).
        let mut input = ReplInput::new();
        assert_eq!(input.feed("set x {a \\"), None);
        assert_eq!(input.feed("+ 2}"), Some("set x {a \\\n+ 2}".to_string()));
    }

    #[test]
    fn clear_discards_partial_input() {
        let mut input = ReplInput::new();
        input.feed("set x {");
        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.feed("set y 2"), Some("set y 2".to_string()));
    }

    #[test]
    fn control_flow_messages_match_tclsh() {
        let mut interp = Interp::new();
        let e = interp.eval("break").unwrap_err();
        assert_eq!(
            control_flow_text(&e).as_deref(),
            Some("invoked \"break\" outside of a loop")
        );
        let e = interp.eval("continue").unwrap_err();
        assert_eq!(
            control_flow_text(&e).as_deref(),
            Some("invoked \"continue\" outside of a loop")
        );
        // return echoes its value; break/continue carry no value
        let e = interp.eval("return ok").unwrap_err();
        assert!(e.is_return());
        assert_eq!(control_flow_text(&e).as_deref(), Some("ok"));
    }

    #[test]
    fn bare_return_renders_empty_and_other_errors_are_none() {
        let mut interp = Interp::new();
        let e = interp.eval("return").unwrap_err();
        assert_eq!(control_flow_text(&e).as_deref(), Some(""));

        let e = interp.eval("nosuchcmd").unwrap_err();
        assert_eq!(control_flow_text(&e), None);
    }

    #[test]
    fn exit_codes_map_from_message_value() {
        let mut interp = Interp::new();
        let e = interp.eval("exit").unwrap_err();
        assert!(e.is_exit());
        assert_eq!(exit_code_of(&e), 0);

        let e = interp.eval("exit 7").unwrap_err();
        assert_eq!(exit_code_of(&e), 7);
    }
}
