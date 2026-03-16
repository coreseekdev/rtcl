//! # rtcl-parser
//!
//! Tcl parser for rtcl — produces an AST from Tcl source code.
//!
//! Uses a pest PEG grammar (`tcl.pest`) to parse Tcl scripts into
//! [`Command`] / [`Word`] structures that can be compiled to bytecode
//! or interpreted directly.

use pest::iterators::Pair;
use pest::Parser;
use pest_derive::Parser;

use core::fmt;

// ---------------------------------------------------------------------------
// AST types
// ---------------------------------------------------------------------------

/// A parsed Tcl command (one line / semicolon-separated unit).
#[derive(Debug, Clone)]
pub struct Command {
    /// Command words (first word is the command name).
    pub words: Vec<Word>,
    /// Source line number (1-based).
    pub line: usize,
}

/// A word in a Tcl command.
#[derive(Debug, Clone)]
pub enum Word {
    /// Literal string — no substitution.
    Literal(String),
    /// Variable reference: `$var`, `${var}`, `$var(index)`.
    VarRef(String),
    /// Command substitution: `[cmd args...]`.
    CommandSub(String),
    /// Concatenation of multiple parts (e.g. `"hello $name"`).
    Concat(Vec<Word>),
    /// Expand syntax: `{*}word` — expands the word as multiple arguments.
    Expand(Box<Word>),
}

impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Word::Literal(s) => write!(f, "{}", s),
            Word::VarRef(n) => write!(f, "${}", n),
            Word::CommandSub(c) => write!(f, "[{}]", c),
            Word::Concat(parts) => {
                for p in parts {
                    write!(f, "{}", p)?;
                }
                Ok(())
            }
            Word::Expand(inner) => write!(f, "{{*}}{}", inner),
        }
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Parse error.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "parse error at {}:{}: {}",
            self.line, self.column, self.message
        )
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

// Convenience alias
pub type ParseResult<T> = Result<T, ParseError>;

// ---------------------------------------------------------------------------
// Pest parser
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[grammar = "tcl.pest"]
struct TclParser;

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> ParseResult<Vec<Command>> {
    let pairs = TclParser::parse(Rule::program, source).map_err(|e| ParseError {
        message: e.to_string(),
        line: 0,
        column: 0,
    })?;

    let mut commands = Vec::new();

    for pair in pairs {
        match pair.as_rule() {
            Rule::program | Rule::script => {
                for inner in pair.into_inner() {
                    if inner.as_rule() == Rule::command_or_sep {
                        for cmd_or_sep in inner.into_inner() {
                            if cmd_or_sep.as_rule() == Rule::command {
                                if let Some(cmd) = parse_command(cmd_or_sep) {
                                    commands.push(cmd);
                                }
                            }
                        }
                    } else if inner.as_rule() == Rule::command {
                        if let Some(cmd) = parse_command(inner) {
                            commands.push(cmd);
                        }
                    }
                }
            }
            Rule::command => {
                if let Some(cmd) = parse_command(pair) {
                    commands.push(cmd);
                }
            }
            _ => {}
        }
    }

    Ok(commands)
}

// ---------------------------------------------------------------------------
// Internal helpers — pest pair → AST
// ---------------------------------------------------------------------------

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
        Rule::expand => parse_expand(inner),
        Rule::braced => {
            // Use pair.as_str() to handle silent rules correctly (bug B1/B4 fix).
            let content = extract_braced_str(inner);
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
            let content = extract_cmd_sub_str(inner);
            Some(Word::CommandSub(content))
        }
        Rule::bare => {
            let text = process_bare(inner.as_str());
            Some(Word::Literal(text))
        }
        _ => None,
    }
}

fn parse_expand(pair: Pair<Rule>) -> Option<Word> {
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::word {
            if let Some(word) = parse_word(inner) {
                return Some(Word::Expand(Box::new(word)));
            }
        }
    }
    None
}

/// Extract braced content by using `pair.as_str()` and stripping the outer
/// braces.  This correctly handles silent child rules (bug B1 fix).
fn extract_braced_str(pair: Pair<Rule>) -> String {
    let raw = pair.as_str();
    // Strip outer { }
    let inner = if raw.starts_with('{') && raw.ends_with('}') {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    // Handle backslash-newline line continuation inside braces
    let mut result = String::with_capacity(inner.len());
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\n' {
            // backslash-newline-whitespace → single space
            i += 2;
            while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                i += 1;
            }
            result.push(' ');
        } else {
            result.push(chars[i]);
            i += 1;
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
                parts.push(Word::CommandSub(extract_cmd_sub_str(inner)));
            }
            Rule::escape => {
                let s = inner.as_str();
                if let Some(ch) = process_escape(s) {
                    if ch != '\0' {
                        literal.push(ch);
                    }
                }
            }
            Rule::quoted_char => {
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
            Rule::braced_var_name => {
                name.push_str(inner.as_str());
            }
            Rule::var_name_chars => {
                name.push_str(inner.as_str());
            }
            Rule::index => {
                let idx = extract_index(inner);
                name = format!("{}({})", name, idx);
            }
            _ => {}
        }
    }
    name
}

fn extract_index(pair: Pair<Rule>) -> String {
    let mut result = String::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::index => {
                result.push('(');
                result.push_str(&extract_index(inner));
                result.push(')');
            }
            Rule::index_char => {
                result.push_str(inner.as_str());
            }
            Rule::index_escape => {
                result.push_str(inner.as_str());
            }
            _ => {}
        }
    }
    result
}

/// Extract command substitution content using `pair.as_str()` and stripping
/// the outer `[ ]`.
fn extract_cmd_sub_str(pair: Pair<Rule>) -> String {
    let raw = pair.as_str();
    if raw.starts_with('[') && raw.ends_with(']') {
        raw[1..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
}

fn process_bare(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' {
            let escape_start = i;
            i += 1;
            if i < chars.len() {
                match chars[i] {
                    'x' => {
                        i += 1;
                        let mut count = 0;
                        while i < chars.len() && chars[i].is_ascii_hexdigit() && count < 2 {
                            i += 1;
                            count += 1;
                        }
                    }
                    'u' => {
                        i += 1;
                        let mut count = 0;
                        while i < chars.len() && chars[i].is_ascii_hexdigit() && count < 4 {
                            i += 1;
                            count += 1;
                        }
                    }
                    'U' => {
                        i += 1;
                        let mut count = 0;
                        while i < chars.len() && chars[i].is_ascii_hexdigit() && count < 8 {
                            i += 1;
                            count += 1;
                        }
                    }
                    '0'..='7' => {
                        let mut count = 0;
                        while i < chars.len() && chars[i] >= '0' && chars[i] <= '7' && count < 3 {
                            i += 1;
                            count += 1;
                        }
                    }
                    '\n' => {
                        i += 1;
                        while i < chars.len()
                            && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == '\r' || chars[i] == '\n')
                        {
                            i += 1;
                        }
                        result.push(' ');
                        continue;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let escape_str: String = chars[escape_start..i].iter().collect();
            if let Some(ch) = process_escape(&escape_str) {
                if ch != '\0' {
                    result.push(ch);
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Process a `\X` escape sequence and return the resulting character.
/// Returns `'\0'` as a sentinel for line continuations that produce no output.
fn process_escape(s: &str) -> Option<char> {
    if !s.starts_with('\\') || s.len() < 2 {
        return s.chars().next();
    }
    let chars: Vec<char> = s.chars().collect();
    match chars[1] {
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        'a' => Some('\x07'),
        'b' => Some('\x08'),
        'f' => Some('\x0c'),
        'v' => Some('\x0b'),
        '\\' => Some('\\'),
        '"' => Some('"'),
        '{' => Some('{'),
        '}' => Some('}'),
        '[' => Some('['),
        ']' => Some(']'),
        '$' => Some('$'),
        '#' => Some('#'),
        ' ' => Some(' '),
        ';' => Some(';'),
        '\n' => Some('\0'), // line continuation sentinel
        'x' => {
            let hex: String = chars[2..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .take(2)
                .collect();
            if hex.is_empty() {
                Some('x')
            } else {
                Some(char::from(u8::from_str_radix(&hex, 16).unwrap_or(0)))
            }
        }
        'u' => {
            let hex: String = chars[2..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .take(4)
                .collect();
            if hex.is_empty() {
                Some('u')
            } else {
                char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0))
            }
        }
        'U' => {
            let hex: String = chars[2..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .take(8)
                .collect();
            if hex.is_empty() {
                Some('U')
            } else {
                char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0))
            }
        }
        c @ '0'..='7' => {
            let oct: String = chars[1..]
                .iter()
                .take_while(|c| **c >= '0' && **c <= '7')
                .take(3)
                .collect();
            Some(char::from(u8::from_str_radix(&oct, 8).unwrap_or(0)))
        }
        c => Some(c),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn test_braced_string() {
        let cmds = parse("puts {hello world}").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "hello world"),
            _ => panic!("expected literal"),
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
    fn test_multiple_commands() {
        let cmds = parse("set x 10\nputs $x").unwrap();
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn test_expand() {
        let cmds = parse("cmd {*}$args").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Expand(_) => {}
            _ => panic!("expected expand"),
        }
    }
}
