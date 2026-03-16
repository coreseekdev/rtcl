//! Tcl parser - converts source code into command structures
//!
//! This module provides both pest-based and manual parsers.
//! The pest-based parser is used when the "pest-parser" feature is enabled.

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

#[cfg(feature = "pest-parser")]
mod pest_parser {
    use super::*;
    use pest::iterators::Pair;
    use pest::Parser;
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "tcl.pest"]
    pub struct TclParser;

    /// Parse Tcl source code using pest
    pub fn parse(source: &str) -> Result<Vec<Command>> {
        let pairs = TclParser::parse(Rule::program, source)
            .map_err(|e| Error::syntax(e.to_string(), 0, 0))?;

        let mut commands = Vec::new();

        for pair in pairs {
            if pair.as_rule() == Rule::program {
                for inner in pair.into_inner() {
                    if inner.as_rule() == Rule::command {
                        if let Some(cmd) = parse_command(inner) {
                            commands.push(cmd);
                        }
                    }
                }
            }
        }

        Ok(commands)
    }

    fn parse_command(pair: Pair<Rule>) -> Option<Command> {
        let line = pair.as_span().start_pos().line_col().0;
        let mut words = Vec::new();

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::word {
                if let Some(word) = parse_word(inner) {
                    words.push(word);
                }
            }
        }

        if words.is_empty() {
            None
        } else {
            Some(Command { words, line })
        }
    }

    fn parse_word(pair: Pair<Rule>) -> Option<Word> {
        let inner = pair.into_inner().next()?;
        match inner.as_rule() {
            Rule::braced => {
                let content = extract_braced(inner);
                Some(Word::Literal(content))
            }
            Rule::quoted => {
                let parts = parse_quoted(inner);
                if parts.is_empty() {
                    Some(Word::Literal(String::new()))
                } else if parts.len() == 1 {
                    parts.into_iter().next()
                } else {
                    Some(Word::Concat(parts))
                }
            }
            Rule::var_ref => {
                let name = extract_var_name(inner);
                Some(Word::VarRef(name))
            }
            Rule::cmd_sub => {
                let content = extract_cmd_sub(inner);
                Some(Word::CommandSub(content))
            }
            Rule::bare => {
                let text = process_bare(inner.as_str());
                Some(Word::Literal(text))
            }
            _ => None,
        }
    }

    fn extract_braced(pair: Pair<Rule>) -> String {
        let mut result = String::new();
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::braced_inner => {
                    result.push_str(inner.as_str());
                }
                Rule::braced => {
                    result.push('{');
                    result.push_str(&extract_braced(inner));
                    result.push('}');
                }
                _ => {}
            }
        }
        result
    }

    fn parse_quoted(pair: Pair<Rule>) -> Vec<Word> {
        let mut parts = Vec::new();
        let mut literal = String::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::var_ref => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    parts.push(Word::VarRef(extract_var_name(inner)));
                }
                Rule::cmd_sub => {
                    if !literal.is_empty() {
                        parts.push(Word::Literal(literal.clone()));
                        literal.clear();
                    }
                    parts.push(Word::CommandSub(extract_cmd_sub(inner)));
                }
                Rule::escape => {
                    let ch = process_escape(inner.as_str());
                    if ch != '\0' {
                        literal.push(ch);
                    }
                }
                Rule::quoted_inner => {
                    literal.push_str(inner.as_str());
                }
                _ => {}
            }
        }

        if !literal.is_empty() {
            parts.push(Word::Literal(literal));
        }
        parts
    }

    fn extract_var_name(pair: Pair<Rule>) -> String {
        let mut name = String::new();
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::name => name.push_str(inner.as_str()),
                Rule::index => {
                    let idx: String = inner.into_inner().map(|p| p.as_str()).collect();
                    name = format!("{}({})", name, idx);
                }
                _ => {}
            }
        }
        name
    }

    fn extract_cmd_sub(pair: Pair<Rule>) -> String {
        let mut result = String::new();
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::cmd_inner => result.push_str(inner.as_str()),
                Rule::cmd_sub => {
                    result.push('[');
                    result.push_str(&extract_cmd_sub(inner));
                    result.push(']');
                }
                _ => {}
            }
        }
        result
    }

    fn process_bare(s: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let escape_str: String = chars[i..].iter().collect();
                let ch = process_escape(&escape_str);
                if ch != '\0' {
                    result.push(ch);
                }
                // Move past the escape sequence
                i += 2;
                // Handle multi-char escapes
                if i >= 2 && chars[i - 2] == '\\' {
                    match chars.get(i - 1) {
                        Some('x') | Some('u') => {
                            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                                i += 1;
                            }
                        }
                        Some(c) if c.is_ascii_digit() => {
                            while i < chars.len() && chars[i].is_ascii_digit() && chars[i] < '8' {
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    fn process_escape(s: &str) -> char {
        if !s.starts_with('\\') || s.len() < 2 {
            return s.chars().next().unwrap_or('\0');
        }
        let chars: Vec<char> = s.chars().collect();
        match chars[1] {
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
            '\n' => '\0', // Line continuation
            'x' => {
                let hex: String = chars[2..].iter().take(2).collect();
                if hex.is_empty() { 'x' } else { char::from(u8::from_str_radix(&hex, 16).unwrap_or(0)) }
            }
            'u' => {
                let hex: String = chars[2..].iter().take(4).collect();
                if hex.is_empty() { 'u' } else { char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0)).unwrap_or('\0') }
            }
            c if c.is_ascii_digit() => {
                let oct: String = chars[1..].iter().take(3).filter(|c| c.is_ascii_digit()).collect();
                char::from(u8::from_str_radix(&oct, 8).unwrap_or(0))
            }
            c => c,
        }
    }
}

#[cfg(feature = "pest-parser")]
pub use pest_parser::parse;

#[cfg(not(feature = "pest-parser"))]
mod manual_parser {
    use super::*;

    pub struct Parser {
        source: Vec<char>,
        pos: usize,
        line: usize,
        column: usize,
    }

    impl Parser {
        pub fn new(source: &str) -> Self {
            Parser {
                source: source.chars().collect(),
                pos: 0,
                line: 1,
                column: 1,
            }
        }

        pub fn parse(&mut self) -> Result<Vec<Command>> {
            let mut commands = Vec::new();
            while !self.is_at_end() {
                self.skip_whitespace_and_comments();
                if self.is_at_end() { break; }
                if self.check('\n') || self.check(';') {
                    self.advance();
                    continue;
                }
                let cmd = self.parse_command()?;
                commands.push(cmd);
            }
            Ok(commands)
        }

        fn parse_command(&mut self) -> Result<Command> {
            let start_line = self.line;
            let mut words = Vec::new();
            loop {
                self.skip_whitespace();
                if self.is_at_end() { break; }
                let ch = self.peek();
                match ch {
                    '\n' | ';' | '}' => {
                        if ch == ';' { self.advance(); }
                        break;
                    }
                    '#' if words.is_empty() => {
                        self.skip_comment();
                        break;
                    }
                    _ => {
                        let word = self.parse_word()?;
                        words.push(word);
                    }
                }
            }
            Ok(Command { words, line: start_line })
        }

        fn parse_word(&mut self) -> Result<Word> {
            let mut parts = Vec::new();
            let mut literal = String::new();
            while !self.is_at_end() {
                let ch = self.peek();
                match ch {
                    ' ' | '\t' | '\n' | ';' | '}' => break,
                    '$' => {
                        if !literal.is_empty() {
                            parts.push(Word::Literal(literal.clone()));
                            literal.clear();
                        }
                        parts.push(self.parse_var_ref()?);
                    }
                    '[' => {
                        if !literal.is_empty() {
                            parts.push(Word::Literal(literal.clone()));
                            literal.clear();
                        }
                        parts.push(self.parse_command_sub()?);
                    }
                    '\\' => literal.push(self.parse_escape()?),
                    '{' => {
                        if !literal.is_empty() {
                            parts.push(Word::Literal(literal.clone()));
                            literal.clear();
                        }
                        parts.push(Word::Literal(self.parse_braced()?));
                    }
                    '"' => {
                        if !literal.is_empty() {
                            parts.push(Word::Literal(literal.clone()));
                            literal.clear();
                        }
                        parts.extend(self.parse_quoted()?);
                    }
                    _ => {
                        literal.push(ch);
                        self.advance();
                    }
                }
            }
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

        fn parse_var_ref(&mut self) -> Result<Word> {
            self.expect('$')?;
            let mut name = String::new();
            if self.check('{') {
                self.advance();
                while !self.is_at_end() && !self.check('}') {
                    name.push(self.peek());
                    self.advance();
                }
                self.expect('}')?;
            } else {
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
                    return Ok(Word::Literal("$".to_string()));
                }
                if self.check('(') {
                    self.advance();
                    let mut index = String::new();
                    let mut depth = 1;
                    while !self.is_at_end() && depth > 0 {
                        let ch = self.peek();
                        match ch {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 { break; }
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

        fn parse_command_sub(&mut self) -> Result<Word> {
            self.expect('[')?;
            let mut cmd = String::new();
            let mut depth = 1;
            while !self.is_at_end() && depth > 0 {
                let ch = self.peek();
                match ch {
                    '[' => {
                        depth += 1;
                        cmd.push(ch);
                    }
                    ']' => {
                        depth -= 1;
                        if depth > 0 {
                            cmd.push(ch);
                            self.advance();
                        }
                        continue;
                    }
                    '\\' => {
                        cmd.push(ch);
                        self.advance();
                        if !self.is_at_end() {
                            cmd.push(self.peek());
                        }
                    }
                    _ => cmd.push(ch),
                }
                self.advance();
            }
            self.expect(']')?;
            Ok(Word::CommandSub(cmd))
        }

        fn parse_braced(&mut self) -> Result<String> {
            self.expect('{')?;
            let mut result = String::new();
            let mut depth = 1;
            while !self.is_at_end() && depth > 0 {
                let ch = self.peek();
                match ch {
                    '{' => {
                        depth += 1;
                        result.push(ch);
                        self.advance();
                    }
                    '}' => {
                        depth -= 1;
                        if depth > 0 {
                            result.push(ch);
                            self.advance();
                        }
                    }
                    '\\' => {
                        result.push(ch);
                        self.advance();
                        if !self.is_at_end() {
                            if self.peek() == '\n' {
                                self.advance();
                                while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                                    self.advance();
                                }
                            } else {
                                result.push(self.peek());
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
            self.expect('}')?;
            Ok(result)
        }

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
                        parts.push(self.parse_var_ref()?);
                    }
                    '[' => {
                        if !literal.is_empty() {
                            parts.push(Word::Literal(literal.clone()));
                            literal.clear();
                        }
                        parts.push(self.parse_command_sub()?);
                    }
                    '\\' => literal.push(self.parse_escape()?),
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

        fn parse_escape(&mut self) -> Result<char> {
            self.expect('\\')?;
            if self.is_at_end() { return Ok('\\'); }
            let ch = self.peek();
            self.advance();
            Ok(match ch {
                'n' => '\n', 't' => '\t', 'r' => '\r',
                'a' => '\x07', 'b' => '\x08', 'f' => '\x0c', 'v' => '\x0b',
                '\\' => '\\', '"' => '"', '{' => '{', '}' => '}',
                '[' => '[', ']' => ']', '$' => '$', ' ' => ' ', ';' => ';',
                '\n' => {
                    while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                        self.advance();
                    }
                    return Ok('\0');
                }
                'x' => {
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if !self.is_at_end() && self.peek().is_ascii_hexdigit() {
                            hex.push(self.peek());
                            self.advance();
                        }
                    }
                    return Ok(if hex.is_empty() { 'x' } else { char::from(u8::from_str_radix(&hex, 16).unwrap_or(0)) });
                }
                'u' => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if !self.is_at_end() && self.peek().is_ascii_hexdigit() {
                            hex.push(self.peek());
                            self.advance();
                        }
                    }
                    return Ok(if hex.is_empty() { 'u' } else { char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0)).unwrap_or('\0') });
                }
                c if c.is_ascii_digit() => {
                    let mut oct = String::new();
                    oct.push(c);
                    for _ in 0..2 {
                        if !self.is_at_end() && self.peek().is_ascii_digit() && self.peek() < '8' {
                            oct.push(self.peek());
                            self.advance();
                        }
                    }
                    return Ok(char::from(u8::from_str_radix(&oct, 8).unwrap_or(0)));
                }
                c => c,
            })
        }

        fn skip_whitespace(&mut self) {
            while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                self.advance();
            }
        }

        fn skip_whitespace_and_comments(&mut self) {
            loop {
                while !self.is_at_end() && (self.peek() == ' ' || self.peek() == '\t') {
                    self.advance();
                }
                if self.check('#') {
                    self.skip_comment();
                } else {
                    break;
                }
            }
        }

        fn skip_comment(&mut self) {
            while !self.is_at_end() && !self.check('\n') {
                self.advance();
            }
        }

        fn check(&self, ch: char) -> bool { !self.is_at_end() && self.peek() == ch }
        fn peek(&self) -> char { self.source.get(self.pos).copied().unwrap_or('\0') }
        fn advance(&mut self) -> char {
            let ch = self.peek();
            self.pos += 1;
            if ch == '\n' { self.line += 1; self.column = 1; } else { self.column += 1; }
            ch
        }
        fn expect(&mut self, ch: char) -> Result<()> {
            if self.check(ch) { self.advance(); Ok(()) }
            else { Err(Error::syntax(format!("expected '{}', found '{}'", ch, self.peek()), self.line, self.column)) }
        }
        fn is_at_end(&self) -> bool { self.pos >= self.source.len() }
    }

    pub fn parse(source: &str) -> Result<Vec<Command>> {
        let mut parser = Parser::new(source);
        parser.parse()
    }
}

#[cfg(not(feature = "pest-parser"))]
pub use manual_parser::parse;

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
        // Quoted string with variable should be a Concat
        match &cmds[0].words[1] {
            Word::Concat(_) | Word::Literal(_) => {}
            _ => panic!("expected concat or literal"),
        }
    }

    #[test]
    fn test_multiple_commands() {
        let cmds = parse("set a 1\nset b 2").unwrap();
        assert_eq!(cmds.len(), 2);
    }
}
