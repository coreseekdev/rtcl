//! Tcl parser - converts source code into command structures

use crate::error::{Error, Result};

/// A parsed Tcl command
#[derive(Debug, Clone)]
pub struct Command {
    /// Command words (first word is the command name)
    pub words: Vec<Word>,
    /// Source line number
    pub line: usize,
}

/// A word in a Tcl command
#[derive(Debug, Clone)]
pub enum Word {
    /// Literal string
    Literal(String),
    /// Variable reference: $var or ${var}
    VarRef(String),
    /// Command substitution: [cmd args...]
    CommandSub(String),
    /// Concatenation of multiple parts
    Concat(Vec<Word>),
}

/// Tcl parser
pub struct Parser {
    source: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Parser {
    /// Create a new parser
    pub fn new(source: &str) -> Self {
        Parser {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    /// Parse all commands in the source
    pub fn parse(&mut self) -> Result<Vec<Command>> {
        let mut commands = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if self.is_at_end() {
                break;
            }

            if self.check('\n') || self.check(';') {
                self.advance();
                continue;
            }

            let cmd = self.parse_command()?;
            commands.push(cmd);
        }

        Ok(commands)
    }

    /// Parse a single command
    fn parse_command(&mut self) -> Result<Command> {
        let start_line = self.line;
        let mut words = Vec::new();

        loop {
            self.skip_whitespace();

            if self.is_at_end() {
                break;
            }

            let ch = self.peek();
            match ch {
                '\n' | ';' | '}' => {
                    // Check for semicolon and consume it
                    if ch == ';' {
                        self.advance();
                    }
                    break;
                }
                '#' if words.is_empty() => {
                    // Comment at start of what would be a new command
                    self.skip_comment();
                    break;
                }
                _ => {
                    let word = self.parse_word()?;
                    words.push(word);
                }
            }
        }

        Ok(Command {
            words,
            line: start_line,
        })
    }

    /// Parse a single word
    fn parse_word(&mut self) -> Result<Word> {
        let mut parts = Vec::new();
        let mut literal = String::new();

        while !self.is_at_end() {
            let ch = self.peek();

            match ch {
                ' ' | '\t' | '\n' | ';' | '}' => {
                    break;
                }
                '$' => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    let var = self.parse_var_ref()?;
                    parts.push(var);
                }
                '[' => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    let cmd_sub = self.parse_command_sub()?;
                    parts.push(cmd_sub);
                }
                '\\' => {
                    let escaped = self.parse_escape()?;
                    literal.push(escaped);
                }
                '{' => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    let braced = self.parse_braced()?;
                    parts.push(Word::Literal(braced));
                }
                '"' => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    let quoted = self.parse_quoted()?;
                    parts.extend(quoted);
                }
                _ => {
                    literal.push(ch);
                    self.advance();
                }
            }
        }

        // Combine parts
        if parts.is_empty() {
            Ok(Word::Literal(literal))
        } else {
            if !literal.is_empty() {
                parts.push(Word::Literal(literal));
            }
            if parts.len() == 1 {
                Ok(parts.into_iter().next().unwrap())
            } else {
                Ok(Word::Concat(parts))
            }
        }
    }

    /// Parse a variable reference: $var or ${var}
    fn parse_var_ref(&mut self) -> Result<Word> {
        self.expect('$')?; // consume $

        let mut name = String::new();

        if self.check('{') {
            // ${var} form
            self.advance();
            while !self.is_at_end() && !self.check('}') {
                name.push(self.peek());
                self.advance();
            }
            self.expect('}')?;
        } else {
            // $var form - alphanumeric and underscore
            while !self.is_at_end() {
                let ch = self.peek();
                if ch.is_alphanumeric() || ch == '_' {
                    name.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }

            if name.is_empty() {
                // Literal $ by itself
                return Ok(Word::Literal("$".to_string()));
            }

            // Handle array reference: $var(index)
            if self.check('(') {
                self.advance();
                let mut index = String::new();
                let mut paren_depth = 1;
                while !self.is_at_end() && paren_depth > 0 {
                    let ch = self.peek();
                    match ch {
                        '(' => paren_depth += 1,
                        ')' => {
                            paren_depth -= 1;
                            if paren_depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    index.push(ch);
                    self.advance();
                }
                self.expect(')')?;
                name = format!("{}({})", name, index);
            }
        }

        Ok(Word::VarRef(name))
    }

    /// Parse command substitution: [cmd args...]
    fn parse_command_sub(&mut self) -> Result<Word> {
        self.expect('[')?;

        let mut cmd = String::new();
        let mut bracket_depth = 1;

        while !self.is_at_end() && bracket_depth > 0 {
            let ch = self.peek();
            match ch {
                '[' => {
                    bracket_depth += 1;
                    cmd.push(ch);
                }
                ']' => {
                    bracket_depth -= 1;
                    if bracket_depth > 0 {
                        cmd.push(ch);
                        self.advance();
                    }
                    // Don't advance if this is the final closing bracket
                    // The expect(']') below will consume it
                    continue;
                }
                '\\' => {
                    cmd.push(ch);
                    self.advance();
                    if !self.is_at_end() {
                        cmd.push(self.peek());
                    }
                }
                _ => {
                    cmd.push(ch);
                }
            }
            self.advance();
        }

        self.expect(']')?;
        Ok(Word::CommandSub(cmd))
    }

    /// Parse braced string: {text}
    fn parse_braced(&mut self) -> Result<String> {
        self.expect('{')?;

        let mut result = String::new();
        let mut brace_depth = 1;

        while !self.is_at_end() && brace_depth > 0 {
            let ch = self.peek();
            match ch {
                '{' => {
                    brace_depth += 1;
                    result.push(ch);
                    self.advance();
                }
                '}' => {
                    brace_depth -= 1;
                    if brace_depth > 0 {
                        result.push(ch);
                        self.advance();
                    }
                    // Don't advance if this is the final closing brace
                    // The loop will exit because brace_depth is now 0
                }
                '\\' => {
                    // In braces, backslash is only special before newline or brace
                    result.push(ch);
                    self.advance();
                    if !self.is_at_end() {
                        let next = self.peek();
                        if next == '\n' {
                            // Line continuation - skip the newline and leading whitespace
                            self.advance();
                            while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                                self.advance();
                            }
                        } else {
                            result.push(next);
                            self.advance();
                        }
                    }
                }
                _ => {
                    result.push(ch);
                    self.advance();
                }
            }
        }

        // Consume the closing '}'
        self.expect('}')?;
        Ok(result)
    }

    /// Parse double-quoted string: "text"
    fn parse_quoted(&mut self) -> Result<Vec<Word>> {
        self.expect('"')?;

        let mut parts = Vec::new();
        let mut literal = String::new();

        while !self.is_at_end() && !self.check('"') {
            let ch = self.peek();
            match ch {
                '$' => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    let var = self.parse_var_ref()?;
                    parts.push(var);
                }
                '[' => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    let cmd_sub = self.parse_command_sub()?;
                    parts.push(cmd_sub);
                }
                '\\' => {
                    let escaped = self.parse_escape()?;
                    literal.push(escaped);
                }
                _ => {
                    literal.push(ch);
                    self.advance();
                }
            }
        }

        self.expect('"')?;

        if !literal.is_empty() {
            parts.push(Word::Literal(literal));
        }

        Ok(parts)
    }

    /// Parse escape sequence
    fn parse_escape(&mut self) -> Result<char> {
        self.expect('\\')?;

        if self.is_at_end() {
            return Ok('\\');
        }

        let ch = self.peek();
        self.advance();

        let result = match ch {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'a' => '\x07',
            'b' => '\x08',
            'f' => '\x0c',
            'v' => '\x0b',
            '\\' => '\\',
            '"' => '"',
            '{' => '{',
            '}' => '}',
            '[' => '[',
            ']' => ']',
            '$' => '$',
            ' ' => ' ',
            ';' => ';',
            '\n' => {
                // Line continuation
                while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                    self.advance();
                }
                return Ok('\0'); // Signal to skip
            }
            'x' => {
                // Hex escape \xHH
                let mut hex = String::new();
                for _ in 0..2 {
                    if !self.is_at_end() {
                        let c = self.peek();
                        if c.is_ascii_hexdigit() {
                            hex.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                if hex.is_empty() {
                    return Ok('x');
                }
                return Ok(char::from(u8::from_str_radix(&hex, 16).unwrap_or(0)));
            }
            'u' => {
                // Unicode escape \uHHHH
                let mut hex = String::new();
                for _ in 0..4 {
                    if !self.is_at_end() {
                        let c = self.peek();
                        if c.is_ascii_hexdigit() {
                            hex.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                if hex.is_empty() {
                    return Ok('u');
                }
                let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
                return Ok(char::from_u32(code).unwrap_or('\0'));
            }
            c if c.is_ascii_digit() => {
                // Octal escape \OOO
                let mut oct = String::new();
                oct.push(c);
                for _ in 0..2 {
                    if !self.is_at_end() {
                        let c = self.peek();
                        if c.is_ascii_digit() && c < '8' {
                            oct.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                return Ok(char::from(u8::from_str_radix(&oct, 8).unwrap_or(0)));
            }
            c => c, // Unknown escape, keep as-is
        };

        Ok(result)
    }

    /// Skip whitespace (not newlines)
    fn skip_whitespace(&mut self) {
        while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
            self.advance();
        }
    }

    /// Skip whitespace and comments
    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip spaces and tabs
            while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                self.advance();
            }

            // Check for comment
            if self.check('#') {
                self.skip_comment();
            } else {
                break;
            }
        }
    }

    /// Skip a comment until end of line
    fn skip_comment(&mut self) {
        while !self.is_at_end() && !self.check('\n') {
            self.advance();
        }
    }

    /// Check if current character matches
    fn check(&self, ch: char) -> bool {
        !self.is_at_end() && self.peek() == ch
    }

    /// Peek at current character
    fn peek(&self) -> char {
        self.source.get(self.pos).copied().unwrap_or('\0')
    }

    /// Advance and return the character
    fn advance(&mut self) -> char {
        let ch = self.peek();
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        ch
    }

    /// Expect a specific character
    fn expect(&mut self, ch: char) -> Result<()> {
        if self.check(ch) {
            self.advance();
            Ok(())
        } else {
            Err(Error::syntax(
                format!("expected '{}', found '{}'", ch, self.peek()),
                self.line,
                self.column,
            ))
        }
    }

    /// Check if at end of source
    fn is_at_end(&self) -> bool {
        self.pos >= self.source.len()
    }
}

/// Parse Tcl source code into commands
pub fn parse(source: &str) -> Result<Vec<Command>> {
    let mut parser = Parser::new(source);
    parser.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let cmds = parse("puts hello").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 2);
    }

    #[test]
    fn test_var_ref() {
        let cmds = parse("puts $name").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::VarRef(name) => assert_eq!(name, "name"),
            _ => panic!("expected var ref"),
        }
    }

    #[test]
    fn test_command_sub() {
        let cmds = parse("puts [expr 1 + 2]").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::CommandSub(cmd) => assert_eq!(cmd, "expr 1 + 2"),
            _ => panic!("expected command sub"),
        }
    }

    #[test]
    fn test_braced_string() {
        let cmds = parse("puts {hello world}").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected literal"),
        }
    }

    #[test]
    fn test_quoted_string() {
        let cmds = parse("puts \"hello $name\"").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Concat(parts) => {
                assert_eq!(parts.len(), 2);
            }
            _ => panic!("expected concat"),
        }
    }

    #[test]
    fn test_multiple_commands() {
        let cmds = parse("set a 1\nset b 2").unwrap();
        assert_eq!(cmds.len(), 2);
    }
}
