//! Recursive descent Tcl parser.
//!
//! Inspired by Molt (Rust) and jimtcl (C). Produces the same
//! [`Command`] / [`Word`] AST as the PEG backend.
#![allow(dead_code)]

mod cursor;
mod escape;
pub mod token;  // Public for re-export in lib.rs
mod tokens;
mod word;

use crate::{Command, ParseResult, SrcSpan};
use cursor::Cursor;
use word::parse_next_word;

// Re-export Token for use within the crate
pub use token::Token;

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> ParseResult<Vec<Command>> {
    let mut cur = Cursor::new(source);
    parse_script(&mut cur, false)
}

/// Incremental command reader, matching tclsh's eval model: each command is
/// parsed only when the previous one has finished executing, so an earlier
/// command runs (and prints) before a later syntax error is reported.
///
/// Spans and line numbers are absolute in the original `source`.
pub struct IncrementalScript<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
}

impl<'a> IncrementalScript<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { source, offset: 0, line: 1 }
    }

    /// Byte offset of the next unparsed command (after the last returned one).
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Parse and return the next command, or `None` at end of input.
    pub fn next_command(&mut self) -> ParseResult<Option<Command>> {
        if self.offset >= self.source.len() {
            return Ok(None);
        }
        let mut cur = Cursor::new_at(self.source, self.offset, self.line);
        skip_command_separators(&mut cur);
        if cur.at_end() {
            self.offset = self.source.len();
            return Ok(None);
        }
        let cmd = parse_command(&mut cur, false)?;
        let next = cur.pos();
        self.line += self.source[self.offset..next]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        self.offset = next;
        Ok(Some(cmd))
    }
}

fn parse_script(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Vec<Command>> {
    let mut commands = Vec::new();

    loop {
        skip_command_separators(cur);

        if cur.at_end() || (bracket_term && cur.is(Token::RightBracket)) {
            break;
        }

        let cmd = parse_command(cur, bracket_term)?;
        if !cmd.words.is_empty() {
            commands.push(cmd);
        }
    }

    Ok(commands)
}

/// Skip whitespace (including newlines), semicolons, and comments between commands.
fn skip_command_separators(cur: &mut Cursor) {
    loop {
        while cur.peek().is_any_whitespace() || cur.is(Token::Semicolon) {
            cur.advance();
        }
        if cur.is(Token::Hash) {
            skip_comment(cur);
        } else {
            break;
        }
    }
}

/// Skip a comment line. `#` at command position, with `\<newline>` continuation.
/// The tokenizer normalizes line endings, so we just look for Token::Newline.
fn skip_comment(cur: &mut Cursor) {
    debug_assert!(cur.is(Token::Hash));
    cur.advance(); // skip '#'
    loop {
        match cur.peek() {
            Token::Eof => break,
            Token::Newline => {
                cur.advance();
                break;
            }
            Token::Backslash => {
                cur.advance(); // skip '\'
                // Check for line continuation
                if cur.peek() == Token::Newline {
                    cur.advance(); // continuation: comment extends to next line
                } else if !cur.at_end() {
                    cur.advance(); // skip the char after backslash
                }
            }
            _ => {
                cur.advance();
            }
        }
    }
}

fn parse_command(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Command> {
    let line = cur.line();
    let start = cur.pos();
    let mut words = Vec::new();
    let mut word_srcs = Vec::new();

    while !cur.at_end_of_command(bracket_term) {
        let wstart = cur.pos();
        words.push(parse_next_word(cur, bracket_term)?);
        word_srcs.push(SrcSpan { start: wstart as u32, end: cur.pos() as u32 });
        cur.skip_line_whitespace();
    }

    // Consume the command terminator (newline or semicolon) — but NOT `]`
    // The tokenizer has already normalized line endings.
    match cur.peek() {
        Token::Newline | Token::Semicolon => {
            cur.advance();
        }
        _ => {}
    }

    // Source text as tclsh quotes it in errorInfo frames: the range as
    // written (leading whitespace already skipped by the separator skip),
    // with one trailing terminator stripped.  Offsets only — consumers
    // slice the owning ScriptUnit's source.
    let mut end = cur.pos();
    if end > start {
        let last = cur.byte(end - 1);
        if last == b'\n' || last == b';' {
            end -= 1;
        }
    }

    Ok(Command {
        words,
        line,
        text: SrcSpan { start: start as u32, end: end as u32 },
        word_srcs: std::rc::Rc::new(word_srcs),
    })
}
