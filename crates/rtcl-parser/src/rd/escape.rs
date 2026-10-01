//! Backslash escape processing.

use super::cursor::Cursor;
use super::token::Token;

/// Process a backslash escape at the cursor. Consumes the `\` and the escape
/// sequence, returns the substituted character.
pub fn backslash_subst(cur: &mut Cursor) -> char {
    debug_assert!(cur.is(Token::Backslash));
    cur.advance(); // skip '\'

    match cur.peek() {
        Token::Eof => '\\',
        Token::Other('a') => { cur.advance(); '\x07' }
        Token::Other('b') => { cur.advance(); '\x08' }
        Token::Other('f') => { cur.advance(); '\x0c' }
        Token::Other('n') => { cur.advance(); '\n' }
        Token::Other('r') => { cur.advance(); '\r' }
        Token::Other('t') => { cur.advance(); '\t' }
        Token::Other('v') => { cur.advance(); '\x0b' }
        Token::Newline => {
            // Line continuation: \<newline><whitespace> → single space
            // The tokenizer normalizes \r\n to Newline, so we just handle Newline
            cur.advance(); // skip newline
            while cur.peek().is_line_whitespace() || cur.peek() == Token::Whitespace {
                cur.advance();
            }
            ' '
        }
        Token::Other('x') => {
            cur.advance(); // skip 'x'
            let start = cur.pos();
            let mut count = 0;
            while count < 2 {
                let ch = match cur.peek() {
                    Token::Other(c) => c,
                    _ => break,
                };
                if ch.is_ascii_hexdigit() {
                    cur.advance();
                    count += 1;
                } else {
                    break;
                }
            }
            if count == 0 {
                'x'
            } else {
                let hex = cur.slice(start);
                u8::from_str_radix(hex, 16).map(|v| v as char).unwrap_or('x')
            }
        }
        Token::Other('u') => {
            cur.advance(); // skip 'u'
            match parse_hex(cur, 4) {
                Some(value) => escape_char(value),
                None => 'u',
            }
        }
        Token::Other('U') => {
            cur.advance(); // skip 'U'
            match parse_hex(cur, 8) {
                Some(value) => escape_char(value),
                None => 'U',
            }
        }
        Token::Other(c) if c.is_ascii_digit() && c <= '7' => {
            let start = cur.pos();
            let mut count = 0;
            while count < 3 {
                match cur.peek() {
                    Token::Other(d) if d.is_ascii_digit() && d <= '7' => {
                        cur.advance();
                        count += 1;
                    }
                    _ => break,
                }
            }
            let oct = cur.slice(start);
            u8::from_str_radix(oct, 8).map(|v| v as char).unwrap_or('\0')
        }
        _ => {
            // Unknown escape: return the character literally
            cur.advance_char().unwrap_or('\\')
        }
    }
}

/// ParseHex (tclParse.c): consume up to `max` hex digits, stopping early once
/// the accumulator exceeds 0x10FFF — digits past that stay in the stream and
/// surface as literal characters ("\UFFFFFFFF" is U+FFFD followed by "FFF",
/// "\U00110000" is U+FFFD followed by "0"). Returns None when no hex digit
/// was consumed (the escape letter is then returned literally).
fn parse_hex(cur: &mut Cursor, max: usize) -> Option<u32> {
    let mut result = 0u32;
    let mut count = 0;
    while count < max {
        let ch = match cur.peek() {
            Token::Other(c) => c,
            _ => break,
        };
        let digit = match ch.to_digit(16) {
            Some(d) => d,
            None => break,
        };
        if result > 0x10FFF {
            break;
        }
        cur.advance();
        result = (result << 4) | digit;
        count += 1;
    }
    if count == 0 {
        None
    } else {
        Some(result)
    }
}

/// Encode a \u/\U value. tclsh's default 8.6 build (TCL_UTF_MAX < 4) clamps
/// values above 0xFFFF to U+FFFD, and Rust chars cannot hold unpaired
/// surrogates at all (tclsh keeps them internally, but they never survive a
/// round-trip through a script), so both become U+FFFD. This makes
/// "\UD842" and "\uD842" compare equal, as in tclsh.
fn escape_char(value: u32) -> char {
    if value > 0xFFFF || (0xD800..=0xDFFF).contains(&value) {
        '\u{FFFD}'
    } else {
        char::from_u32(value).unwrap_or('\u{FFFD}')
    }
}

/// Handle `\<newline><whitespace>` → single space inside braced content.
/// Supports both LF (\n) and CRLF (\r\n) line endings.
pub fn process_braced_backslash_newline(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            // Check for CRLF: \r\n
            if bytes[i + 1] == b'\r' && i + 2 < bytes.len() && bytes[i + 2] == b'\n' {
                // backslash-CRLF-whitespace → single space
                i += 3;
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                result.push(' ');
                continue;
            }
            // Check for LF: \n
            if bytes[i + 1] == b'\n' {
                // backslash-newline-whitespace → single space
                i += 2;
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                result.push(' ');
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}
