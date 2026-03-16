//! Index-based scanner over UTF-8 input.

use crate::ParseError;

pub(crate) struct Cursor<'a> {
    pub(crate) input: &'a str,
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) line: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            pos: 0,
            line: 1,
        }
    }

    #[inline]
    pub fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Peek the current byte (only valid for ASCII-range dispatching).
    #[inline]
    pub fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Peek the byte at offset from current position.
    #[inline]
    pub fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    /// Advance by one byte. Tracks newlines.
    #[inline]
    pub fn advance(&mut self) {
        if self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
    }

    /// Advance by one UTF-8 character. Returns the char.
    pub fn advance_char(&mut self) -> Option<char> {
        if self.at_end() {
            return None;
        }
        let s = &self.input[self.pos..];
        let ch = s.chars().next()?;
        let len = ch.len_utf8();
        for _ in 0..len {
            self.advance();
        }
        Some(ch)
    }

    /// Current byte is the given ASCII byte.
    #[inline]
    pub fn is(&self, b: u8) -> bool {
        self.peek() == Some(b)
    }

    /// Remaining input slice from current position.
    pub fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    /// Slice from `start` to current position.
    pub fn slice(&self, start: usize) -> &'a str {
        &self.input[start..self.pos]
    }

    /// Skip ASCII whitespace on the current line (spaces, tabs, \r, \f).
    pub fn skip_line_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' | 0x0c => self.advance(),
                b'\\' if self.peek_at(1) == Some(b'\n') => {
                    // backslash-newline: line continuation is whitespace
                    self.advance(); // '\'
                    self.advance(); // '\n'
                    // consume trailing whitespace after continuation
                    while let Some(b) = self.peek() {
                        if b == b' ' || b == b'\t' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// At end of command? (newline, semicolon, end-of-input, or `]` if in bracket mode)
    pub fn at_end_of_command(&self, bracket_term: bool) -> bool {
        match self.peek() {
            None => true,
            Some(b'\n') | Some(b';') => true,
            Some(b']') if bracket_term => true,
            _ => false,
        }
    }

    /// Is next char line whitespace?
    pub fn next_is_line_white(&self) -> bool {
        matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | 0x0c))
    }

    /// Is next char valid variable name start? (alphanumeric, _, ::, or >= 0x80)
    pub fn next_is_varname_start(&self) -> bool {
        match self.peek() {
            Some(b) if b.is_ascii_alphanumeric() || b == b'_' => true,
            Some(b':') if self.peek_at(1) == Some(b':') => true,
            Some(b) if b >= 0x80 => true, // Unicode
            _ => false,
        }
    }

    /// Is byte a valid variable name character?
    pub fn is_varname_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
    }

    /// Make a parse error at the current position.
    pub fn error(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            message: msg.into(),
            line: self.line,
            column: 0,
            offset: self.pos,
        }
    }
}
