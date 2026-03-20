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
            let start = cur.pos();
            let mut count = 0;
            while count < 4 {
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
                'u'
            } else {
                let hex = cur.slice(start);
                u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .unwrap_or('u')
            }
        }
        Token::Other('U') => {
            cur.advance(); // skip 'U'
            let start = cur.pos();
            let mut count = 0;
            while count < 8 {
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
                'U'
            } else {
                let hex = cur.slice(start);
                u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .unwrap_or('U')
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
