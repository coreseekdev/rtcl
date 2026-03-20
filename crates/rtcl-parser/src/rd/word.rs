//! Word-level parsing: braced, quoted, bare, variable, command substitution.

use crate::{ParseError, ParseResult, Word};
use super::cursor::Cursor;
use super::token::Token;
use super::escape::{backslash_subst, process_braced_backslash_newline};
use super::tokens::Tokens;

// ---------------------------------------------------------------------------
// Top-level word dispatch
// ---------------------------------------------------------------------------

pub fn parse_next_word(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Word> {
    match cur.peek() {
        Token::LeftBrace => {
            // Check for {*} expand syntax
            // We need to look at the raw string to detect "{*}"
            let rest = cur.rest();
            if rest.starts_with("{*}") {
                let after = rest.get(3..).and_then(|s| s.chars().next());
                // {*} followed by non-whitespace and not end = expand
                let is_expand = match after {
                    None => false,
                    Some(c) => {
                        !c.is_whitespace() && c != ';' && !(bracket_term && c == ']')
                    }
                };
                if is_expand {
                    cur.advance(); // {
                    cur.advance(); // *
                    cur.advance(); // }
                    let inner = parse_next_word(cur, bracket_term)?;
                    return Ok(Word::Expand(Box::new(inner)));
                }
            }
            parse_braced_word(cur)
        }
        Token::DoubleQuote => parse_quoted_word(cur, bracket_term),
        _ => parse_bare_word(cur, bracket_term),
    }
}

// ---------------------------------------------------------------------------
// Braced word: {text} — no substitution
// ---------------------------------------------------------------------------

fn parse_braced_word(cur: &mut Cursor) -> ParseResult<Word> {
    debug_assert!(cur.is(Token::LeftBrace));
    let err_line = cur.line();
    cur.advance(); // skip '{'
    let mut depth: u32 = 1;
    let start = cur.pos();

    while !cur.at_end() {
        match cur.peek() {
            Token::LeftBrace => {
                depth += 1;
                cur.advance();
            }
            Token::RightBrace => {
                depth -= 1;
                if depth == 0 {
                    let text = cur.slice(start);
                    cur.advance(); // skip closing '}'
                    let processed = process_braced_backslash_newline(text);
                    return Ok(Word::Literal(processed));
                }
                cur.advance();
            }
            Token::Backslash => {
                cur.advance(); // skip '\'
                if !cur.at_end() {
                    cur.advance(); // skip next token (prevents \{ or \} from affecting depth)
                }
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
        offset: cur.pos(),
    })
}

// ---------------------------------------------------------------------------
// Quoted word: "text" — substitutions active
// ---------------------------------------------------------------------------

fn parse_quoted_word(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Word> {
    debug_assert!(cur.is(Token::DoubleQuote));
    let err_line = cur.line();
    cur.advance(); // skip opening '"'

    let mut tokens = Tokens::new();
    let mut start = cur.pos();

    while !cur.at_end() {
        match cur.peek() {
            Token::DoubleQuote => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                cur.advance(); // skip closing '"'
                return Ok(tokens.take());
            }
            Token::LeftBracket => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                let cmd_text = parse_cmd_sub(cur, bracket_term)?;
                tokens.push(Word::CommandSub(cmd_text));
                start = cur.pos();
            }
            Token::Dollar => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                parse_dollar(cur, &mut tokens, bracket_term)?;
                start = cur.pos();
            }
            Token::Backslash => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                tokens.push_char(backslash_subst(cur));
                start = cur.pos();
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
        offset: cur.pos(),
    })
}

// ---------------------------------------------------------------------------
// Bare word — no delimiters, substitutions active
// ---------------------------------------------------------------------------

fn parse_bare_word(cur: &mut Cursor, bracket_term: bool) -> ParseResult<Word> {
    let mut tokens = Tokens::new();
    let mut start = cur.pos();

    while !cur.at_end_of_command(bracket_term) && !cur.next_is_line_white() {
        match cur.peek() {
            Token::LeftBracket => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                let cmd_text = parse_cmd_sub(cur, bracket_term)?;
                tokens.push(Word::CommandSub(cmd_text));
                start = cur.pos();
            }
            Token::Dollar => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                parse_dollar(cur, &mut tokens, bracket_term)?;
                start = cur.pos();
            }
            Token::Backslash => {
                if cur.pos() != start {
                    tokens.push_str(cur.slice(start));
                }
                tokens.push_char(backslash_subst(cur));
                start = cur.pos();
            }
            _ => {
                cur.advance();
            }
        }
    }

    if cur.pos() != start {
        tokens.push_str(cur.slice(start));
    }

    Ok(tokens.take())
}

// ---------------------------------------------------------------------------
// Variable reference: $name, ${name}, $name(index)
// ---------------------------------------------------------------------------

fn parse_dollar(cur: &mut Cursor, tokens: &mut Tokens, _bracket_term: bool) -> ParseResult<()> {
    debug_assert!(cur.is(Token::Dollar));
    cur.advance(); // skip '$'

    match cur.peek() {
        Token::LeftBracket => {
            // $[...] expr sugar (jimtcl extension): evaluate content as expression
            let cmd_text = parse_cmd_sub(cur, _bracket_term)?;
            tokens.push(Word::ExprSugar(cmd_text));
        }
        Token::LeftBrace => {
            // ${var_name}
            cur.advance(); // skip '{'
            let start = cur.pos();
            while !cur.at_end() && !cur.is(Token::RightBrace) {
                cur.advance();
            }
            if cur.at_end() {
                return Err(cur.error("missing close-brace for variable name"));
            }
            let name = cur.slice(start).to_string();
            cur.advance(); // skip '}'

            if cur.is(Token::LeftParen) {
                if let Some(idx) = try_parse_var_index(cur) {
                    tokens.push(Word::VarRef(format!("{}({})", name, idx)));
                } else {
                    tokens.push(Word::VarRef(name));
                }
            } else {
                tokens.push(Word::VarRef(name));
            }
        }
        Token::LeftParen => {
            // $(...) expr sugar (jimtcl default): evaluate content as expression
            // Only when $ is NOT followed by a variable name char
            if let Some(expr) = try_parse_expr_sugar(cur) {
                tokens.push(Word::ExprSugar(expr));
            } else {
                // No closing ')' found — treat $ as orphan
                tokens.push_char('$');
            }
        }
        t => {
            let is_var_start = match t {
                Token::Other(c) if c.is_ascii_alphanumeric() || c == '_' => true,
                Token::Colon if cur.peek_at(1) == Token::Colon => true,
                Token::Other(c) if (c as u32) >= 0x80 => true,
                _ => false,
            };
            if is_var_start {
                // $name or $name(index) or $ns::name
                let start = cur.pos();
                loop {
                    match cur.peek() {
                        Token::Other(c) if Cursor::is_varname_char(c) => cur.advance(),
                        Token::Colon if cur.peek_at(1) == Token::Colon => {
                            cur.advance();
                            cur.advance();
                        }
                        _ => break,
                    }
                }
                let name = cur.slice(start).to_string();

                if cur.is(Token::LeftParen) {
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
        }
    }

    Ok(())
}

/// Try to parse `$(expr)` sugar: when `$` is followed by `(`, parse balanced
/// parens and return the expression content. Returns `None` if no closing `)` found.
fn try_parse_expr_sugar(cur: &mut Cursor) -> Option<String> {
    debug_assert!(cur.is(Token::LeftParen));
    let save = cur.checkpoint();
    cur.advance(); // skip '('
    let content_start = cur.pos();
    let mut depth: u32 = 1;

    while !cur.at_end() && depth > 0 {
        match cur.peek() {
            Token::LeftParen => {
                depth += 1;
                cur.advance();
            }
            Token::RightParen => {
                depth -= 1;
                if depth == 0 {
                    let expr = cur.slice(content_start).to_string();
                    cur.advance(); // skip closing ')'
                    return Some(expr);
                }
                cur.advance();
            }
            Token::Backslash => {
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

    // No balanced close — restore cursor
    cur.restore(save);
    None
}

/// Try to parse an array index: `(...)`.
///
/// Returns `Some(index)` if a matching `)` was found.
/// If no `)` is found, restores the cursor to before `(` and returns `None`.
/// If parens are nested but unbalanced, backtracks to after the last `)` found
/// (jimtcl-compatible behavior).
fn try_parse_var_index(cur: &mut Cursor) -> Option<String> {
    debug_assert!(cur.is(Token::LeftParen));
    let save = cur.checkpoint();
    cur.advance(); // skip '('
    let mut depth: u32 = 1;
    let content_start = cur.pos();

    // Track state right after the last ')' encountered at any depth
    let mut last_close_end: Option<usize> = None;
    let mut last_close_checkpoint: Option<super::cursor::CursorCheckpoint> = None;

    while !cur.at_end() && depth > 0 {
        match cur.peek() {
            Token::LeftParen => {
                depth += 1;
                cur.advance();
            }
            Token::RightParen => {
                depth -= 1;
                if depth == 0 {
                    let idx = cur.slice(content_start).to_string();
                    cur.advance(); // skip closing ')'
                    return Some(idx);
                }
                let close_pos = cur.pos();
                cur.advance(); // advance past ')'
                last_close_end = Some(close_pos);
                last_close_checkpoint = Some(cur.checkpoint());
            }
            Token::Backslash => {
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
        let idx = cur.slice_range(content_start, close_pos).to_string();
        if let Some(cp) = last_close_checkpoint {
            cur.restore(cp);
        }
        return Some(idx);
    }

    // No ')' found at all — not an array index. Restore cursor.
    cur.restore(save);
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
    debug_assert!(cur.is(Token::LeftBracket));
    let err_line = cur.line();
    cur.advance(); // skip '['
    let start = cur.pos();
    let mut depth: u32 = 1;
    let mut startofword = true;

    while !cur.at_end() && depth > 0 {
        match cur.peek() {
            Token::LeftBracket => {
                depth += 1;
                startofword = true;
                cur.advance();
            }
            Token::RightBracket => {
                depth -= 1;
                if depth == 0 {
                    let content = cur.slice(start).to_string();
                    cur.advance(); // skip ']'
                    return Ok(content);
                }
                startofword = false;
                cur.advance();
            }
            Token::LeftBrace if startofword => {
                skip_braced(cur)?;
                startofword = false;
            }
            Token::DoubleQuote if startofword => {
                skip_quoted_in_cmd(cur)?;
                startofword = false;
            }
            Token::Whitespace | Token::CarriageReturn | Token::Newline | Token::Semicolon => {
                startofword = true;
                cur.advance();
            }
            Token::Backslash => {
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
        offset: cur.pos(),
    })
}

/// Skip a `{...}` block inside command substitution (find matching `}`).
fn skip_braced(cur: &mut Cursor) -> ParseResult<()> {
    debug_assert!(cur.is(Token::LeftBrace));
    let err_line = cur.line();
    cur.advance();
    let mut depth: u32 = 1;

    while !cur.at_end() && depth > 0 {
        match cur.peek() {
            Token::LeftBrace => {
                depth += 1;
                cur.advance();
            }
            Token::RightBrace => {
                depth -= 1;
                cur.advance();
            }
            Token::Backslash => {
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
            offset: cur.pos(),
        })
    } else {
        Ok(())
    }
}

/// Skip a `"..."` string inside command substitution (find matching `"`).
fn skip_quoted_in_cmd(cur: &mut Cursor) -> ParseResult<()> {
    debug_assert!(cur.is(Token::DoubleQuote));
    let err_line = cur.line();
    cur.advance(); // skip opening '"'

    while !cur.at_end() {
        match cur.peek() {
            Token::DoubleQuote => {
                cur.advance();
                return Ok(());
            }
            Token::Backslash => {
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
        offset: cur.pos(),
    })
}
