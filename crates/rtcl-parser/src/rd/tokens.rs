//! Token accumulator — Molt-style string coalescing.

use crate::Word;

pub(crate) struct Tokens {
    parts: Vec<Word>,
    string: String,
    has_string: bool,
}

impl Tokens {
    pub fn new() -> Self {
        Self {
            parts: Vec::new(),
            string: String::new(),
            has_string: false,
        }
    }

    /// Push a non-literal word (VarRef, CommandSub, etc.). Flushes any
    /// accumulated string first.
    pub fn push(&mut self, word: Word) {
        self.flush_string();
        self.parts.push(word);
    }

    /// Append text to the current string accumulator.
    pub fn push_str(&mut self, s: &str) {
        self.string.push_str(s);
        self.has_string = true;
    }

    /// Append a single char to the current string accumulator.
    pub fn push_char(&mut self, ch: char) {
        self.string.push(ch);
        self.has_string = true;
    }

    fn flush_string(&mut self) {
        if self.has_string {
            let s = std::mem::take(&mut self.string);
            self.parts.push(Word::Literal(s));
            self.has_string = false;
        }
    }

    /// Take the accumulated tokens as a single Word.
    pub fn take(mut self) -> Word {
        self.flush_string();
        match self.parts.len() {
            0 => Word::Literal(String::new()),
            1 => self.parts.pop().unwrap(),
            _ => Word::Concat(self.parts),
        }
    }
}
