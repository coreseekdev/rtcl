//! Word-level parsing: braced, quoted, bare, variable, command substitution.

use crate::{ParseError, ParseResult, Word};
use super::cursor::Cursor;
use super::escape::{backslash_subst, process_braced_backslash_newline};
use super::tokens::Tokens;

// ---------------------------------------------------------------------------
// Top-level word dispatch
// ---------------------------------------------------------------------------

pub fn parse_next_word(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Word> {
    match cur.peek() {
        Some(b'{') => {
            // Check for {*} expand syntax
            if cur.rest().starts_with("{*}") {
                let after = cur.bytes.get(cur.pos + 3).copied();
                // {*} followed by non-whitespace and not end = expand
                if after.is_some()
                    && !matches!(after, Some(b' ' | b'\t' | b'\n' | b'\r' | b';'))
                    && !(bracket_term && after == Some(b']'))
                {
                    cur.advance(); // {
                    cur.advance(); // *
                    cur.advance(); // }
                    let inner = parse_next_word(cur, bracket_term)?;
                    return Ok(Word::Expand(Box::new(inner)));
                }
            }
            parse_braced_word(cur)
        }
        Some(b'"') => parse_quoted_word(cur, bracket_term),
        _ => parse_bare_word(cur, bracket_term),
    }
}

// ---------------------------------------------------------------------------
// Braced word: {text} — no substitution
// ---------------------------------------------------------------------------

fn parse_braced_word(cur: &mut Cursor) -> ParseResult<Word> {
    debug_assert!(cur.is(b'{'));
    let err_line = cur.line;
    cur.advance(); // skip '{'
    let mut depth: u32 = 1;
    let mut text = String::new();
    let mut start = cur.pos;

    while !cur.at_end() {
        match cur.peek().unwrap() {
            b'{' => {
                depth += 1;
                cur.advance();
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    text.push_str(cur.slice(start));
                    cur.advance(); // skip closing '}'
                    let processed = process_braced_backslash_newline(&text);
                    return Ok(Word::Literal(processed));
                }
                cur.advance();
            }
            b'\\' => {
                text.push_str(cur.slice(start));
                cur.advance(); // skip '\'
                if let Some(ch) = cur.peek() {
                    if ch == b'\n' {
                        text.push('\\');
                        text.push('\n');
                        cur.advance();
                    } else {
                        text.push('\\');
                        let c = cur.advance_char().unwrap_or('\\');
                        text.push(c);
                    }
                } else {
                    text.push('\\');
                }
                start = cur.pos;
            }
            _ => {
                cur.advance();
            }
        }
    }

    Err(ParseError {
        message: "missing close-brace".into(),
        line: err_line,
        column: 0,
        offset: cur.pos,
    })
}

// ---------------------------------------------------------------------------
// Quoted word: "text" — substitutions active
// ---------------------------------------------------------------------------

fn parse_quoted_word(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Word> {
    debug_assert!(cur.is(b'"'));
    let err_line = cur.line;
    cur.advance(); // skip opening '"'

    let mut tokens = Tokens::new();
    let mut start = cur.pos;

    while !cur.at_end() {
        match cur.peek().unwrap() {
            b'"' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                cur.advance(); // skip closing '"'
                return Ok(tokens.take());
            }
            b'[' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                let cmd_text = parse_cmd_sub(cur, bracket_term)?;
                tokens.push(Word::CommandSub(cmd_text));
                start = cur.pos;
            }
            b'$' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                parse_dollar(cur, &mut tokens, bracket_term)?;
                start = cur.pos;
            }
            b'\\' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                tokens.push_char(backslash_subst(cur));
                start = cur.pos;
            }
            _ => {
                cur.advance();
            }
        }
    }

    Err(ParseError {
        message: "missing \"".into(),
        line: err_line,
        column: 0,
        offset: cur.pos,
    })
}

// ---------------------------------------------------------------------------
// Bare word — no delimiters, substitutions active
// ---------------------------------------------------------------------------

fn parse_bare_word(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Word> {
    let mut tokens = Tokens::new();
    let mut start = cur.pos;

    while !cur.at_end_of_command(bracket_term) && !cur.next_is_line_white() {
        match cur.peek().unwrap() {
            b'[' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                let cmd_text = parse_cmd_sub(cur, bracket_term)?;
                tokens.push(Word::CommandSub(cmd_text));
                start = cur.pos;
            }
            b'$' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                parse_dollar(cur, &mut tokens, bracket_term)?;
                start = cur.pos;
            }
            b'\\' => {
                if start != cur.pos {
                    tokens.push_str(cur.slice(start));
                }
                tokens.push_char(backslash_subst(cur));
                start = cur.pos;
            }
            _ => {
                cur.advance();
            }
        }
    }

    if start != cur.pos {
        tokens.push_str(cur.slice(start));
    }

    Ok(tokens.take())
}

// ---------------------------------------------------------------------------
// Variable reference: $name, ${name}, $name(index)
// ---------------------------------------------------------------------------

fn parse_dollar(cur: &mut Cursor, tokens: &mut Tokens, _bracket_term: bool) -> ParseResult<()> {
    debug_assert!(cur.is(b'$'));
    cur.advance(); // skip '$'

    if cur.is(b'[') {
        // $[...] expr sugar (jimtcl extension): evaluate content as expression
        let cmd_text = parse_cmd_sub(cur, _bracket_term)?;
        tokens.push(Word::ExprSugar(cmd_text));
    } else if cur.is(b'{') {
        // ${var_name}
        cur.advance(); // skip '{'
        let start = cur.pos;
        while !cur.at_end() && !cur.is(b'}') {
            cur.advance();
        }
        if cur.at_end() {
            return Err(cur.error("missing close-brace for variable name"));
        }
        let name = cur.slice(start).to_string();
        cur.advance(); // skip '}'

        if cur.is(b'(') {
            if let Some(idx) = try_parse_var_index(cur) {
                tokens.push(Word::VarRef(format!("{}({})", name, idx)));
            } else {
                tokens.push(Word::VarRef(name));
            }
        } else {
            tokens.push(Word::VarRef(name));
        }
    } else if cur.next_is_varname_start() {
        // $name or $name(index) or $ns::name
        let start = cur.pos;
        loop {
            match cur.peek() {
                Some(b) if Cursor::is_varname_char(b) => cur.advance(),
                Some(b':') if cur.peek_at(1) == Some(b':') => {
                    cur.advance();
                    cur.advance();
                }
                _ => break,
            }
        }
        let name = cur.slice(start).to_string();

        if cur.is(b'(') {
            if let Some(idx) = try_parse_var_index(cur) {
                tokens.push(Word::VarRef(format!("{}({})", name, idx)));
            } else {
                tokens.push(Word::VarRef(name));
            }
        } else {
            tokens.push(Word::VarRef(name));
        }
    } else {
        // Orphan $: not followed by valid var name char
        tokens.push_char('$');
    }

    Ok(())
}

/// Try to parse an array index: `(...)`.
///
/// Returns `Some(index)` if a matching `)` was found.
/// If no `)` is found, restores the cursor to before `(` and returns `None`.
/// If parens are nested but unbalanced, backtracks to after the last `)` found
/// (jimtcl-compatible behavior).
fn try_parse_var_index(cur: &mut Cursor) -> Option<String> {
    debug_assert!(cur.is(b'('));
    let save_pos = cur.pos;
    let save_line = cur.line;
    cur.advance(); // skip '('
    let mut depth: u32 = 1;
    let content_start = cur.pos;

    // Track state right after the last ')' encountered at any depth
    let mut last_close_end: Option<usize> = None; // position of ')'
    let mut last_close_after_pos: Option<usize> = None;
    let mut last_close_after_line: Option<usize> = None;

    while !cur.at_end() && depth > 0 {
        match cur.peek().unwrap() {
            b'(' => {
                depth += 1;
                cur.advance();
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let idx = cur.slice(content_start).to_string();
                    cur.advance(); // skip closing ')'
                    return Some(idx);
                }
                let close_pos = cur.pos; // position of ')'
                cur.advance(); // advance past ')'
                last_close_end = Some(close_pos);
                last_close_after_pos = Some(cur.pos);
                last_close_after_line = Some(cur.line);
            }
            b'\\' => {
                cur.advance(); // skip '\'
                if !cur.at_end() {
                    cur.advance(); // skip escaped char
                }
            }
            _ => {
                cur.advance();
            }
        }
    }

    // Unbalanced: backtrack
    if let Some(close_pos) = last_close_end {
        // Found at least one ')' but nesting didn't balance.
        // Backtrack to just after the last ')' found.
        let idx = cur.input[content_start..close_pos].to_string();
        cur.pos = last_close_after_pos.unwrap();
        cur.line = last_close_after_line.unwrap();
        return Some(idx);
    }

    // No ')' found at all — not an array index. Restore cursor.
    cur.pos = save_pos;
    cur.line = save_line;
    None
}

// ---------------------------------------------------------------------------
// Command substitution: [script]
// ---------------------------------------------------------------------------

/// Parse `[...]` and return the inner script text.
///
/// Uses jimtcl-style `startofword` tracking so that `"` and `{` only
/// trigger sub-parsing when they appear at word boundaries.
pub fn parse_cmd_sub(cur: &mut Cursor, _bracket_term: bool) -> ParseResult<String> {
    debug_assert!(cur.is(b'['));
    let err_line = cur.line;
    cur.advance(); // skip '['
    let start = cur.pos;
    let mut depth: u32 = 1;
    let mut startofword = true;

    while !cur.at_end() && depth > 0 {
        match cur.peek().unwrap() {
            b'[' => {
                depth += 1;
                startofword = true;
                cur.advance();
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let content = cur.slice(start).to_string();
                    cur.advance(); // skip ']'
                    return Ok(content);
                }
                startofword = false;
                cur.advance();
            }
            b'{' if startofword => {
                skip_braced(cur)?;
                startofword = false;
            }
            b'"' if startofword => {
                skip_quoted_in_cmd(cur)?;
                startofword = false;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b';' => {
                startofword = true;
                cur.advance();
            }
            b'\\' => {
                cur.advance(); // skip '\'
                if !cur.at_end() {
                    cur.advance(); // skip escaped char
                }
                startofword = false;
            }
            _ => {
                startofword = false;
                cur.advance();
            }
        }
    }

    Err(ParseError {
        message: "missing close-bracket".into(),
        line: err_line,
        column: 0,
        offset: cur.pos,
    })
}

/// Skip a `{...}` block inside command substitution (find matching `}`).
fn skip_braced(cur: &mut Cursor) -> ParseResult<()> {
    debug_assert!(cur.is(b'{'));
    let err_line = cur.line;
    cur.advance();
    let mut depth: u32 = 1;

    while !cur.at_end() && depth > 0 {
        match cur.peek().unwrap() {
            b'{' => {
                depth += 1;
                cur.advance();
            }
            b'}' => {
                depth -= 1;
                cur.advance();
            }
            b'\\' => {
                cur.advance();
                if !cur.at_end() {
                    cur.advance();
                }
            }
            _ => {
                cur.advance();
            }
        }
    }

    if depth > 0 {
        Err(ParseError {
            message: "missing close-brace".into(),
            line: err_line,
            column: 0,
            offset: cur.pos,
        })
    } else {
        Ok(())
    }
}

/// Skip a `"..."` string inside command substitution (find matching `"`).
fn skip_quoted_in_cmd(cur: &mut Cursor) -> ParseResult<()> {
    debug_assert!(cur.is(b'"'));
    let err_line = cur.line;
    cur.advance(); // skip opening '"'

    while !cur.at_end() {
        match cur.peek().unwrap() {
            b'"' => {
                cur.advance();
                return Ok(());
            }
            b'\\' => {
                cur.advance();
                if !cur.at_end() {
                    cur.advance();
                }
            }
            _ => {
                cur.advance();
            }
        }
    }

    Err(ParseError {
        message: "missing \"".into(),
        line: err_line,
        column: 0,
        offset: cur.pos,
    })
}
