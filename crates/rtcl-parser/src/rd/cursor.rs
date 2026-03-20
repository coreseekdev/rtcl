//! Index-based scanner over UTF-8 input.
//!
//! This cursor uses the token-based tokenizer internally, providing
//! normalized line ending handling:
//! - `\n` and `\r\n` are both treated as `Token::Newline`
//! - Standalone `\r` is treated as `Token::CarriageReturn` (whitespace)

use crate::ParseError;
use super::token::{Tokenizer, Token, Checkpoint};

pub(crate) struct Cursor<'a> {
    input: &'a str,
    tokenizer: Tokenizer<'a>,
    /// The current token (what `peek()` returns).
    current: Token,
    /// Byte offset where the current token starts.
    token_start: usize,
    /// Line number at the start of the current token (1-based).
    token_line: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(input: &'a str) -> Self {
        let mut tokenizer = Tokenizer::new(input);
        let token_start = tokenizer.pos(); // 0
        let token_line = tokenizer.line(); // 1
        let current = tokenizer.next();
        Self {
            input,
            tokenizer,
            current,
            token_start,
            token_line,
        }
    }

    #[inline]
    pub fn at_end(&self) -> bool {
        self.current == Token::Eof
    }

    /// Peek the current token.
    #[inline]
    pub fn peek(&self) -> Token {
        self.current
    }

    /// Peek the token at `offset` from the current position.
    /// `peek_at(0)` == `peek()`.
    #[inline]
    pub fn peek_at(&self, offset: usize) -> Token {
        if offset == 0 {
            return self.current;
        }
        let mut temp = self.tokenizer.clone();
        let mut tok = Token::Eof;
        for _ in 0..offset {
            tok = temp.next();
        }
        tok
    }

    /// Advance by one token.
    #[inline]
    pub fn advance(&mut self) {
        self.token_start = self.tokenizer.pos();
        self.token_line = self.tokenizer.line();
        self.current = self.tokenizer.next();
    }

    /// Advance and return the raw character at the current position.
    /// Works for any token type by reading from the original input.
    pub fn advance_char(&mut self) -> Option<char> {
        if self.at_end() {
            return None;
        }
        let ch = self.input[self.token_start..].chars().next().unwrap();
        self.advance();
        Some(ch)
    }

    /// Current token is the given token.
    #[inline]
    pub fn is(&self, tok: Token) -> bool {
        self.current == tok
    }

    /// Remaining input slice from the start of the current token.
    pub fn rest(&self) -> &'a str {
        &self.input[self.token_start..]
    }

    /// Slice from byte offset `start` to the start of the current token.
    pub fn slice(&self, start: usize) -> &'a str {
        &self.input[start..self.token_start]
    }

    /// Slice between two arbitrary byte positions in the original input.
    pub fn slice_range(&self, start: usize, end: usize) -> &'a str {
        &self.input[start..end]
    }

    /// Byte offset where the current token starts.
    #[inline]
    pub fn pos(&self) -> usize {
        self.token_start
    }

    /// Line number of the current token (1-based).
    #[inline]
    pub fn line(&self) -> usize {
        self.token_line
    }

    /// Create a checkpoint for backtracking.
    #[inline]
    pub fn checkpoint(&self) -> CursorCheckpoint {
        CursorCheckpoint {
            tokenizer: self.tokenizer.checkpoint(),
            current: self.current,
            token_start: self.token_start,
            token_line: self.token_line,
        }
    }

    /// Restore from a checkpoint.
    #[inline]
    pub fn restore(&mut self, cp: CursorCheckpoint) {
        self.tokenizer.restore(cp.tokenizer);
        self.current = cp.current;
        self.token_start = cp.token_start;
        self.token_line = cp.token_line;
    }

    /// Skip line whitespace (spaces, tabs, standalone `\r`, form feed)
    /// and backslash-newline continuations.
    pub fn skip_line_whitespace(&mut self) {
        loop {
            while self.current.is_line_whitespace() {
                self.advance();
            }
            if self.current == Token::Backslash {
                let cp = self.checkpoint();
                self.advance(); // consume backslash
                if self.current == Token::Newline {
                    // Line continuation — consume newline, then loop
                    // back to skip whitespace on the next line.
                    self.advance();
                    continue;
                } else {
                    // Not a continuation — restore and stop.
                    self.restore(cp);
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// At end of command? (newline, semicolon, EOF, or `]` if in bracket mode)
    pub fn at_end_of_command(&self, bracket_term: bool) -> bool {
        match self.current {
            Token::Eof => true,
            Token::Newline | Token::Semicolon => true,
            Token::RightBracket if bracket_term => true,
            _ => false,
        }
    }

    /// Is current token line whitespace?
    pub fn next_is_line_white(&self) -> bool {
        self.current.is_line_whitespace()
    }

    /// Is current token a valid variable-name start?
    pub fn next_is_varname_start(&self) -> bool {
        match self.current {
            Token::Other(c) if c.is_ascii_alphanumeric() || c == '_' => true,
            Token::Colon if self.peek_at(1) == Token::Colon => true,
            Token::Other(c) if (c as u32) >= 0x80 => true,
            _ => false,
        }
    }

    /// Is `ch` a valid variable-name character?
    pub fn is_varname_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_' || (ch as u32) >= 0x80
    }

    /// Make a parse error at the current position.
    pub fn error(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            message: msg.into(),
            line: self.line(),
            column: 0,
            offset: self.pos(),
        }
    }

    /// Consume a specific token or return error if not present.
    pub fn consume(&mut self, expected: Token) -> Result<(), ParseError> {
        if self.current == expected {
            self.advance();
            Ok(())
        } else {
            Err(self.error(format!("expected {:?}", expected)))
        }
    }
}

/// Checkpoint for cursor backtracking.
#[derive(Clone)]
pub(crate) struct CursorCheckpoint {
    tokenizer: Checkpoint,
    current: Token,
    token_start: usize,
    token_line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_newline_normalization() {
        let mut cur = Cursor::new("a\nb\r\nc");
        assert_eq!(cur.peek(), Token::Other('a'));
        cur.advance();
        assert_eq!(cur.peek(), Token::Newline);
        cur.advance();
        assert_eq!(cur.peek(), Token::Other('b'));
        cur.advance();
        assert_eq!(cur.peek(), Token::Newline); // \r\n normalized
        cur.advance();
        assert_eq!(cur.peek(), Token::Other('c'));
    }

    #[test]
    fn test_standalone_cr_is_whitespace() {
        let mut cur = Cursor::new("a\rb");
        assert_eq!(cur.peek(), Token::Other('a'));
        cur.advance();
        assert_eq!(cur.peek(), Token::CarriageReturn);
        assert!(cur.peek().is_line_whitespace());
    }

    #[test]
    fn test_skip_line_whitespace() {
        let mut cur = Cursor::new("  \t\r  x");
        cur.skip_line_whitespace();
        assert_eq!(cur.peek(), Token::Other('x'));
    }

    #[test]
    fn test_line_continuation() {
        let mut cur = Cursor::new("a\\\nb");
        assert_eq!(cur.peek(), Token::Other('a'));
        cur.advance();
        assert_eq!(cur.peek(), Token::Backslash);
        cur.skip_line_whitespace();
        assert_eq!(cur.peek(), Token::Other('b'));
    }

    #[test]
    fn test_crlf_continuation() {
        let mut cur = Cursor::new("a\\\r\nb");
        assert_eq!(cur.peek(), Token::Other('a'));
        cur.advance();
        assert_eq!(cur.peek(), Token::Backslash);
        cur.skip_line_whitespace();
        assert_eq!(cur.peek(), Token::Other('b'));
    }

    #[test]
    fn test_at_end_of_command() {
        let mut cur = Cursor::new("cmd\n");
        assert!(!cur.at_end_of_command(false));
        cur.advance(); // 'c'
        cur.advance(); // 'm'
        cur.advance(); // 'd'
        assert!(cur.at_end_of_command(false)); // at \n
    }

    #[test]
    fn test_checkpoint() {
        let mut cur = Cursor::new("abc");
        let cp = cur.checkpoint();
        cur.advance();
        cur.advance();
        cur.restore(cp);
        assert_eq!(cur.peek(), Token::Other('a'));
    }

    #[test]
    fn test_slice() {
        let mut cur = Cursor::new("hello world");
        let start = cur.pos();
        for _ in 0..5 {
            cur.advance();
        }
        assert_eq!(cur.slice(start), "hello");
    }

    #[test]
    fn test_pos_tracks_crlf() {
        let mut cur = Cursor::new("ab\r\ncd");
        assert_eq!(cur.pos(), 0);
        cur.advance(); // past 'a'
        assert_eq!(cur.pos(), 1);
        cur.advance(); // past 'b'
        assert_eq!(cur.pos(), 2);
        cur.advance(); // past \r\n (2 bytes, one Newline token)
        assert_eq!(cur.pos(), 4);
        cur.advance(); // past 'c'
        assert_eq!(cur.pos(), 5);
    }

    #[test]
    fn test_line_tracking() {
        let mut cur = Cursor::new("a\nb\r\nc");
        assert_eq!(cur.line(), 1);
        cur.advance(); // 'a'
        assert_eq!(cur.line(), 1); // \n is on line 1
        cur.advance(); // \n
        assert_eq!(cur.line(), 2); // 'b' is on line 2
        cur.advance(); // 'b'
        assert_eq!(cur.line(), 2); // \r\n is on line 2
        cur.advance(); // \r\n
        assert_eq!(cur.line(), 3); // 'c' is on line 3
    }
}
