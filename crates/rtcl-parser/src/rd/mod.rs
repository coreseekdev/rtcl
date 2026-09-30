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

use crate::{Command, ParseResult};
use cursor::Cursor;
use word::parse_next_word;

// Re-export Token for use within the crate
pub use token::Token;

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> ParseResult<Vec<Command>> {
    let mut cur = Cursor::new(source);
    parse_script(&mut cur, false)
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
        word_srcs.push(cur.slice_range(wstart, cur.pos()).to_string());
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
    // with one trailing terminator stripped.
    let mut text = cur.slice_range(start, cur.pos()).to_string();
    if text.ends_with('\n') || text.ends_with(';') {
        text.pop();
    }

    Ok(Command { words, line, text, word_srcs })
}
