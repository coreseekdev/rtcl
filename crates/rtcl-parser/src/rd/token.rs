//! Unified token system for Tcl parsing.
//!
//! This module provides a token-based abstraction that normalizes line endings:
//! - `\n` and `\r\n` are both tokenized as `Token::Newline`
//! - Standalone `\r` is tokenized as `Token::CarriageReturn` (whitespace)
//!
//! This eliminates the need for manual `\r\n` handling throughout the parser.

/// A single token from the Tcl source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// End of input
    Eof,

    /// Newline - normalized from `\n` or `\r\n`
    Newline,

    /// Standalone `\r` (not followed by `\n`) - treated as whitespace in Tcl
    CarriageReturn,

    /// Whitespace: space, tab, form feed
    Whitespace,

    /// Backslash `\`
    Backslash,

    /// Dollar sign `$`
    Dollar,

    /// Opening brace `{`
    LeftBrace,

    /// Closing brace `}`
    RightBrace,

    /// Opening bracket `[`
    LeftBracket,

    /// Closing bracket `]`
    RightBracket,

    /// Double quote `"`
    DoubleQuote,

    /// Semicolon `;`
    Semicolon,

    /// Hash `#` (comment start)
    Hash,

    /// Opening parenthesis `(`
    LeftParen,

    /// Closing parenthesis `)`
    RightParen,

    /// Colon `:`
    Colon,

    /// Asterisk `*`
    Asterisk,

    /// Any other character (use the char value)
    Other(char),
}

impl Token {
    /// Returns true if this token is line whitespace (space, tab, standalone \r, form feed)
    pub fn is_line_whitespace(self) -> bool {
        matches!(
            self,
            Token::Whitespace | Token::CarriageReturn
        )
    }

    /// Returns true if this token can end a command
    pub fn is_command_end(self) -> bool {
        matches!(
            self,
            Token::Newline | Token::Semicolon | Token::Eof
        )
    }

    /// Returns true if this token is whitespace (any kind)
    pub fn is_any_whitespace(self) -> bool {
        matches!(
            self,
            Token::Whitespace | Token::CarriageReturn | Token::Newline
        )
    }

    /// Returns the character value for `Token::Other`, or '\0' for other tokens
    pub fn as_char(self) -> char {
        match self {
            Token::Other(c) => c,
            _ => '\0',
        }
    }
}

/// Tokenizer that converts a string slice into a stream of tokens.
///
/// The tokenizer handles line ending normalization:
/// - `\n` -> `Token::Newline`
/// - `\r\n` -> `Token::Newline`
/// - `\r` (not followed by `\n`) -> `Token::CarriageReturn`
///
/// # Example
///
/// ```
/// use rtcl_parser::rd::token::{Tokenizer, Token};
///
/// let input = "hello\r\nworld\rgoodbye";
/// let mut tokenizer = Tokenizer::new(input);
///
/// assert!(matches!(tokenizer.next(), Some(Token::Other('h'))));
/// // ... more tokens ...
/// assert!(matches!(tokenizer.next(), Some(Token::Newline))); // from \r\n
/// assert!(matches!(tokenizer.next(), Some(Token::CarriageReturn))); // from standalone \r
/// ```
pub struct Tokenizer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            pos: 0,
            line: 1,
        }
    }

    /// Peek the current byte without consuming it.
    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Peek at offset from current position.
    #[inline]
    fn peek_byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    /// Get the current UTF-8 character without consuming.
    fn current_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Consume one byte and return it.
    #[inline]
    fn consume_byte(&mut self) -> Option<u8> {
        if self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    /// Get the next token without consuming it.
    pub fn peek(&self) -> Token {
        if self.pos >= self.bytes.len() {
            return Token::Eof;
        }
        let mut temp = self.clone();
        temp.next()
    }

    /// Get the token at offset from current position.
    pub fn peek_at(&self, offset: usize) -> Token {
        let mut temp = self.clone();
        for _ in 0..offset {
            if temp.pos >= temp.bytes.len() {
                return Token::Eof;
            }
            temp.next();
        }
        temp.next()
    }

    /// Get the next token and advance.
    pub fn next(&mut self) -> Token {
        if self.pos >= self.bytes.len() {
            return Token::Eof;
        }

        let b = self.bytes[self.pos];

        match b {
            // Handle line endings with normalization
            b'\n' => {
                self.pos += 1;
                self.line += 1;
                Token::Newline
            }
            b'\r' => {
                // Check if this is \r\n (CRLF)
                if self.peek_byte_at(1) == Some(b'\n') {
                    self.pos += 2;
                    self.line += 1;
                    Token::Newline
                } else {
                    // Standalone \r - whitespace
                    self.pos += 1;
                    Token::CarriageReturn
                }
            }

            // Single-character tokens
            b' ' | b'\t' | 0x0c => {
                self.pos += 1;
                Token::Whitespace
            }
            b'\\' => {
                self.pos += 1;
                Token::Backslash
            }
            b'$' => {
                self.pos += 1;
                Token::Dollar
            }
            b'{' => {
                self.pos += 1;
                Token::LeftBrace
            }
            b'}' => {
                self.pos += 1;
                Token::RightBrace
            }
            b'[' => {
                self.pos += 1;
                Token::LeftBracket
            }
            b']' => {
                self.pos += 1;
                Token::RightBracket
            }
            b'"' => {
                self.pos += 1;
                Token::DoubleQuote
            }
            b';' => {
                self.pos += 1;
                Token::Semicolon
            }
            b'#' => {
                self.pos += 1;
                Token::Hash
            }
            b'(' => {
                self.pos += 1;
                Token::LeftParen
            }
            b')' => {
                self.pos += 1;
                Token::RightParen
            }
            b':' => {
                self.pos += 1;
                Token::Colon
            }
            b'*' => {
                self.pos += 1;
                Token::Asterisk
            }

            // Any other character - need to handle UTF-8
            _ => {
                let ch = self.current_char().unwrap_or('\0');
                self.pos += ch.len_utf8();
                Token::Other(ch)
            }
        }
    }

    /// Consume tokens while the predicate returns true.
    /// Returns the number of tokens consumed.
    pub fn consume_while<F>(&mut self, mut pred: F) -> usize
    where
        F: FnMut(Token) -> bool,
    {
        let mut count = 0;
        loop {
            let tok = self.peek();
            if !pred(tok) || tok == Token::Eof {
                break;
            }
            self.next();
            count += 1;
        }
        count
    }

    /// Current line number (1-based).
    pub fn line(&self) -> usize {
        self.line
    }

    /// Current byte position.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Remaining input from current position.
    pub fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    /// Slice from `start` to current position.
    pub fn slice(&self, start: usize) -> &'a str {
        &self.input[start..self.pos]
    }

    /// Get a slice from the original input between two positions.
    pub fn slice_range(&self, start: usize, end: usize) -> &'a str {
        &self.input[start..end]
    }

    /// Check if we're at the end of input.
    pub fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Reset to a previous position.
    pub fn reset_to(&mut self, pos: usize, line: usize) {
        self.pos = pos;
        self.line = line;
    }

    /// Create a checkpoint of the current position.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            pos: self.pos,
            line: self.line,
        }
    }

    /// Restore from a checkpoint.
    pub fn restore(&mut self, checkpoint: Checkpoint) {
        self.pos = checkpoint.pos;
        self.line = checkpoint.line;
    }
}

impl<'a> Clone for Tokenizer<'a> {
    fn clone(&self) -> Self {
        Self {
            input: self.input,
            bytes: self.bytes,
            pos: self.pos,
            line: self.line,
        }
    }
}

/// Checkpoint for saving/restoring tokenizer position.
#[derive(Debug, Clone, Copy)]
pub struct Checkpoint {
    pos: usize,
    line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_newline_normalization() {
        let mut tz = Tokenizer::new("a\nb\r\nc");
        assert_eq!(tz.next(), Token::Other('a'));
        assert_eq!(tz.next(), Token::Newline);
        assert_eq!(tz.next(), Token::Other('b'));
        assert_eq!(tz.next(), Token::Newline);  // \r\n -> Newline
        assert_eq!(tz.next(), Token::Other('c'));
        assert_eq!(tz.next(), Token::Eof);
    }

    #[test]
    fn test_standalone_cr() {
        let mut tz = Tokenizer::new("a\rb");
        assert_eq!(tz.next(), Token::Other('a'));
        assert_eq!(tz.next(), Token::CarriageReturn);  // standalone \r
        assert_eq!(tz.next(), Token::Other('b'));
    }

    #[test]
    fn test_cr_followed_by_non_lf() {
        let mut tz = Tokenizer::new("a\r b");
        assert_eq!(tz.next(), Token::Other('a'));
        assert_eq!(tz.next(), Token::CarriageReturn);  // \r followed by space
        assert_eq!(tz.next(), Token::Whitespace);
        assert_eq!(tz.next(), Token::Other('b'));
    }

    #[test]
    fn test_basic_tokens() {
        let mut tz = Tokenizer::new("{ $var } [cmd]");
        assert_eq!(tz.next(), Token::LeftBrace);
        assert_eq!(tz.next(), Token::Whitespace);
        assert_eq!(tz.next(), Token::Dollar);
        assert_eq!(tz.next(), Token::Other('v'));
        assert_eq!(tz.next(), Token::Other('a'));
        assert_eq!(tz.next(), Token::Other('r'));
        assert_eq!(tz.next(), Token::Whitespace);
        assert_eq!(tz.next(), Token::RightBrace);
        assert_eq!(tz.next(), Token::Whitespace);
        assert_eq!(tz.next(), Token::LeftBracket);
    }

    #[test]
    fn test_line_tracking() {
        let mut tz = Tokenizer::new("a\nb\r\nc\nd");
        assert_eq!(tz.line(), 1);
        tz.next();
        tz.next();  // consume \n
        assert_eq!(tz.line(), 2);
        tz.next();  // consume 'b'
        tz.next();  // consume \r\n
        assert_eq!(tz.line(), 3);
        tz.next();
        tz.next();  // consume \n
        assert_eq!(tz.line(), 4);
    }

    #[test]
    fn test_peek() {
        let tz = Tokenizer::new("abc");
        assert_eq!(tz.peek(), Token::Other('a'));
        assert_eq!(tz.peek_at(1), Token::Other('b'));
        assert_eq!(tz.peek_at(2), Token::Other('c'));
    }

    #[test]
    fn test_checkpoint() {
        let mut tz = Tokenizer::new("abc");
        let cp = tz.checkpoint();
        tz.next();
        tz.next();
        tz.restore(cp);
        assert_eq!(tz.next(), Token::Other('a'));
    }
}
