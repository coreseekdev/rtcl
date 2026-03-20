//! Backslash escape processing.

use super::cursor::Cursor;

/// Process a backslash escape at the cursor. Consumes the `\` and the escape
/// sequence, returns the substituted character.
pub fn backslash_subst(cur: &mut Cursor) -> char {
    debug_assert!(cur.is(b'\\'));
    cur.advance(); // skip '\'

    match cur.peek() {
        None => '\\',
        Some(b'a') => { cur.advance(); '\x07' }
        Some(b'b') => { cur.advance(); '\x08' }
        Some(b'f') => { cur.advance(); '\x0c' }
        Some(b'n') => { cur.advance(); '\n' }
        Some(b'r') => { cur.advance(); '\r' }
        Some(b't') => { cur.advance(); '\t' }
        Some(b'v') => { cur.advance(); '\x0b' }
        Some(b'\r') => {
            // CRLF line continuation: \<cr><lf><whitespace> → single space
            cur.advance(); // skip '\r'
            if cur.is(b'\n') { cur.advance(); } // skip '\n' if present
            while let Some(b) = cur.peek() {
                if b == b' ' || b == b'\t' {
                    cur.advance();
                } else {
                    break;
                }
            }
            ' '
        }
        Some(b'\n') => {
            // LF line continuation: \<newline><whitespace> → single space
            cur.advance(); // skip newline
            while let Some(b) = cur.peek() {
                if b == b' ' || b == b'\t' {
                    cur.advance();
                } else {
                    break;
                }
            }
            ' '
        }
        Some(b'x') => {
            cur.advance(); // skip 'x'
            let start = cur.pos;
            let mut count = 0;
            while count < 2 {
                match cur.peek() {
                    Some(b) if b.is_ascii_hexdigit() => { cur.advance(); count += 1; }
                    _ => break,
                }
            }
            if count == 0 {
                'x'
            } else {
                let hex = cur.slice(start);
                u8::from_str_radix(hex, 16).map(|v| v as char).unwrap_or('x')
            }
        }
        Some(b'u') => {
            cur.advance(); // skip 'u'
            let start = cur.pos;
            let mut count = 0;
            while count < 4 {
                match cur.peek() {
                    Some(b) if b.is_ascii_hexdigit() => { cur.advance(); count += 1; }
                    _ => break,
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
        Some(b'U') => {
            cur.advance(); // skip 'U'
            let start = cur.pos;
            let mut count = 0;
            while count < 8 {
                match cur.peek() {
                    Some(b) if b.is_ascii_hexdigit() => { cur.advance(); count += 1; }
                    _ => break,
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
        Some(b @ b'0'..=b'7') => {
            let _ = b;
            let start = cur.pos;
            let mut count = 0;
            while count < 3 {
                match cur.peek() {
                    Some(b'0'..=b'7') => { cur.advance(); count += 1; }
                    _ => break,
                }
            }
            let oct = cur.slice(start);
            u8::from_str_radix(oct, 8).map(|v| v as char).unwrap_or('\0')
        }
        Some(_) => {
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
