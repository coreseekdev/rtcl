//! Recursive descent Tcl parser.
//!
//! Inspired by Molt (Rust) and jimtcl (C). Produces the same
//! [`Command`] / [`Word`] AST as the PEG backend.
#![allow(dead_code)]

mod cursor;
mod escape;
mod tokens;
mod word;

use crate::{Command, ParseResult};
use cursor::Cursor;
use word::parse_next_word;

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> ParseResult<Vec<Command>> {
    let mut cur = Cursor::new(source);
    parse_script(&mut cur, false)
}

fn parse_script(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Vec<Command>> {
    let mut commands = Vec::new();

    loop {
        skip_command_separators(cur);

        if cur.at_end() || (bracket_term && cur.is(b']')) {
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
        loop {
            match cur.peek() {
                Some(b' ' | b'\t' | b'\r' | 0x0c | b'\n' | b';') => cur.advance(),
                _ => break,
            }
        }
        if cur.is(b'#') {
            skip_comment(cur);
        } else {
            break;
        }
    }
}

/// Skip a comment line. `#` at command position, with `\<newline>` continuation.
fn skip_comment(cur: &mut Cursor) {
    debug_assert!(cur.is(b'#'));
    cur.advance(); // skip '#'
    loop {
        match cur.peek() {
            None => break,
            Some(b'\n') => {
                cur.advance();
                break;
            }
            Some(b'\\') => {
                cur.advance(); // skip '\'
                if cur.is(b'\n') {
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
    let line = cur.line;
    let mut words = Vec::new();

    while !cur.at_end_of_command(bracket_term) {
        words.push(parse_next_word(cur, bracket_term)?);
        cur.skip_line_whitespace();
    }

    // Consume the command terminator (newline or semicolon) — but NOT `]`
    match cur.peek() {
        Some(b'\n') | Some(b';') => cur.advance(),
        _ => {}
    }

    Ok(Command { words, line })
}
