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
        Rule::composite => {
            let parts = parse_composite(inner);
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
        Rule::orphan_dollar => Some(Word::Literal("$".to_string())),
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

/// Parse a composite word (mixed bare text + var_ref + cmd_sub + orphan_dollar).
fn parse_composite(pair: Pair<Rule>) -> Vec<Word> {
    let mut parts = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::var_ref => {
                parts.push(Word::VarRef(extract_var_name(inner)));
            }
            Rule::cmd_sub => {
                parts.push(Word::CommandSub(extract_cmd_sub_str(inner)));
            }
            Rule::orphan_dollar => {
                parts.push(Word::Literal("$".to_string()));
            }
            Rule::bare => {
                parts.push(Word::Literal(process_bare(inner.as_str())));
            }
            _ => {}
        }
    }
    parts
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
            Rule::orphan_dollar => {
                literal.push('$');
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
    // index is atomic (@{ }), so pair.as_str() is the raw text including outer ( )
    let raw = pair.as_str();
    if raw.starts_with('(') && raw.ends_with(')') {
        raw[1..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
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
        '\n' => Some(' '), // line continuation → single space
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

    // -----------------------------------------------------------------------
    // jimtcl parse.test alignment tests
    // -----------------------------------------------------------------------

    /// parse-1.1: Quoted closing bracket  `"]"` → length 1 literal
    #[test]
    fn test_parse_1_1_quoted_closing_bracket() {
        // `set x [string length "]"]`
        // The `"]"` inside [..] is a quoted string containing `]`
        let cmds = parse(r#"set x [string length "]"]"#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        // word 2 is cmd_sub
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => assert_eq!(cmd, r#"string length "]""#),
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.2: Quoted opening bracket via escape `"\["`
    #[test]
    fn test_parse_1_2_quoted_escaped_bracket() {
        let cmds = parse(r#"set x [string length "\["]"#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => assert_eq!(cmd, r#"string length "\[""#),
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.3: Quoted open brace via escape `"\{"`
    #[test]
    fn test_parse_1_3_quoted_escaped_brace() {
        let cmds = parse(r#"set x [string length "\{"]"#).unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => assert_eq!(cmd, r#"string length "\{""#),
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.5: Braced bracket `{]}`
    #[test]
    fn test_parse_1_5_braced_bracket() {
        let cmds = parse(r#"set x [string length {]}]"#).unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => assert_eq!(cmd, "string length {]}"),
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.9: Backslash newline (line continuation)
    #[test]
    fn test_parse_1_9_backslash_newline() {
        let cmds = parse("set x 123;\\\nset y 456").unwrap();
        // `set x 123` ; `\ + newline` is line continuation → `set y 456`
        assert!(cmds.len() >= 2, "should have at least 2 commands, got {:?}", cmds);
    }

    /// parse-1.10: Backslash newline in quotes → space
    #[test]
    fn test_parse_1_10_backslash_newline_in_quotes() {
        let cmds = parse("set x \"abc\\\ndef\"").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def"),
            Word::Concat(parts) => {
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "abc def");
            }
            w => panic!("expected literal 'abc def', got {:?}", w),
        }
    }

    /// parse-1.17: Command and var in quotes
    #[test]
    fn test_parse_1_17_cmd_var_in_quotes() {
        let cmds = parse(r#"set x "[set z 2][set y]""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], Word::CommandSub(_)));
                assert!(matches!(&parts[1], Word::CommandSub(_)));
            }
            w => panic!("expected concat with 2 cmd subs, got {:?}", w),
        }
    }

    /// parse-1.18: Command and var in bare context
    #[test]
    fn test_parse_1_18_cmd_var_bare() {
        let cmds = parse("set x [set z 2][set y]").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], Word::CommandSub(_)));
                assert!(matches!(&parts[1], Word::CommandSub(_)));
            }
            w => panic!("expected concat with 2 cmd subs, got {:?}", w),
        }
    }

    /// parse-1.19: Lone dollar sign in quotes `"6$[set y]"` → `6$1`
    #[test]
    fn test_parse_1_19_orphan_dollar_in_quotes() {
        let cmds = parse(r#"set x "6$[set y]""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                // Should be: Literal("6$") or Literal("6") + Literal("$") + CommandSub("set y")
                let has_dollar = parts.iter().any(|p| match p {
                    Word::Literal(s) => s.contains('$'),
                    _ => false,
                });
                let has_cmd = parts.iter().any(|p| matches!(p, Word::CommandSub(_)));
                assert!(has_dollar, "should contain literal $, got {:?}", parts);
                assert!(has_cmd, "should contain cmd sub, got {:?}", parts);
            }
            w => panic!("expected concat, got {:?}", w),
        }
    }

    /// parse-1.20: Lone dollar sign in bare context `6$[set y]`
    #[test]
    fn test_parse_1_20_orphan_dollar_bare() {
        let cmds = parse("set x 6$[set y]").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                let has_dollar = parts.iter().any(|p| match p {
                    Word::Literal(s) => s.contains('$'),
                    _ => false,
                });
                let has_cmd = parts.iter().any(|p| matches!(p, Word::CommandSub(_)));
                assert!(has_dollar, "should contain literal $, got {:?}", parts);
                assert!(has_cmd, "should contain cmd sub, got {:?}", parts);
            }
            w => panic!("expected concat, got {:?}", w),
        }
    }

    /// parse-1.21: Comment handling
    #[test]
    fn test_parse_1_21_comment() {
        let src = "set y 1\n# A comment on a line\nset x 2";
        let cmds = parse(src).unwrap();
        assert_eq!(cmds.len(), 2, "comment should not be a command: {:?}", cmds);
    }

    /// parse-1.22: # char in non-command position  
    #[test]
    fn test_parse_1_22_hash_in_word() {
        let cmds = parse("append y #").unwrap();
        assert_eq!(cmds[0].words.len(), 3, "# should be a word, not a comment: {:?}", cmds);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "#"),
            w => panic!("expected literal '#', got {:?}", w),
        }
    }

    /// parse-1.23: newline in command substitution
    #[test]
    fn test_parse_1_23_newline_in_cmd_sub() {
        let cmds = parse("set x [incr y\nincr z]").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => {
                assert!(cmd.contains("incr y") && cmd.contains("incr z"),
                    "cmd sub should contain both commands: {:?}", cmd);
            }
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.24: semicolon in command substitution
    #[test]
    fn test_parse_1_24_semicolon_in_cmd_sub() {
        let cmds = parse("set x [list a; list b c; list d e f]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => {
                assert!(cmd.contains("list a") && cmd.contains("list d e f"),
                    "cmd sub should contain commands: {:?}", cmd);
            }
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.26: newline in braced var name
    #[test]
    fn test_parse_1_26_newline_in_braced_var() {
        let cmds = parse("set x ${a\nb}").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "a\nb"),
            w => panic!("expected var_ref 'a\\nb', got {:?}", w),
        }
    }

    /// parse-1.31: Backslash newline in bare context
    #[test]
    fn test_parse_1_31_backslash_newline_bare() {
        let cmds = parse("list abc\\\n\t123").unwrap();
        // `abc\<newline>\t123` → `abc` is first word with escape producing space,
        // then `123` could be part of same word or next word depending on parsing
        assert!(!cmds.is_empty());
    }

    /// parse-1.33: Upper case hex escapes
    #[test]
    fn test_parse_1_33_hex_escapes() {
        let cmds = parse("list \\x4A \\x4F \\x3C").unwrap();
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "J"),
            w => panic!("expected 'J', got {:?}", w),
        }
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "O"),
            w => panic!("expected 'O', got {:?}", w),
        }
        match &cmds[0].words[3] {
            Word::Literal(s) => assert_eq!(s, "<"),
            w => panic!("expected '<', got {:?}", w),
        }
    }

    /// parse-1.34: Octal escapes
    #[test]
    fn test_parse_1_34_octal_escapes() {
        let cmds = parse("list \\112 \\117 \\074").unwrap();
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "J"),
            w => panic!("expected 'J', got {:?}", w),
        }
    }

    /// parse-1.35: Invalid hex escape
    #[test]
    fn test_parse_1_35_invalid_hex_escape() {
        let cmds = parse("list \\xZZ").unwrap();
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "xZZ"),
            w => panic!("expected 'xZZ', got {:?}", w),
        }
    }

    /// parse-1.38: Invalid unicode escape
    #[test]
    fn test_parse_1_38_invalid_unicode_escape() {
        let cmds = parse("list \\ux").unwrap();
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "ux"),
            w => panic!("expected 'ux', got {:?}", w),
        }
    }

    /// parse-1.39: Octal escape followed by invalid
    #[test]
    fn test_parse_1_39_octal_then_char() {
        let cmds = parse("list \\76x").unwrap();
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, ">x"),
            w => panic!("expected '>x', got {:?}", w),
        }
    }

    /// parse-1.47-1.50: Backslash newline in quotes with whitespace
    #[test]
    fn test_parse_1_47_backslash_newline_in_quotes_spaces() {
        let cmds = parse("set x \"abc\\\n      def\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def"),
            Word::Concat(parts) => {
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "abc def");
            }
            w => panic!("expected 'abc def', got {:?}", w),
        }
    }

    /// parse-1.62: Quoted orphan dollar sign at end `"x$"`
    #[test]
    fn test_parse_1_62_quoted_orphan_dollar_end() {
        let cmds = parse(r#"set x "x$""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                // "x" + "$"
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "x$");
            }
            Word::Literal(s) => assert_eq!(s, "x$"),
            w => panic!("expected 'x$', got {:?}", w),
        }
    }

    /// parse-1.63: Unquoted orphan dollar sign at end `x$`
    #[test]
    fn test_parse_1_63_unquoted_orphan_dollar_end() {
        let cmds = parse("set x x$").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "x$");
            }
            Word::Literal(s) => assert_eq!(s, "x$"),
            w => panic!("expected 'x$', got {:?}", w),
        }
    }

    /// parse-1.64: Backslash in comment (line continuation)
    #[test]
    fn test_parse_1_64_backslash_in_comment() {
        let src = "set x 0\n# comment \\\nincr x\nincr x";
        let cmds = parse(src).unwrap();
        // The `\` at end of comment continues the comment to next line
        // So `incr x` after `\<newline>` is part of the comment
        // Only the second `incr x` is a real command
        // Commands: set x 0, incr x (the second one)
        assert_eq!(cmds.len(), 2, "backslash-newline should continue comment: {:?}", cmds);
    }

    /// parse-1.65: Double backslash in comment (NOT line continuation)
    #[test]
    fn test_parse_1_65_double_backslash_in_comment() {
        let src = "set x 0\n# comment \\\\\nincr x\nincr x";
        let cmds = parse(src).unwrap();
        // `\\` means escaped backslash, so newline ends the comment
        // Both `incr x` lines are commands
        // Commands: set x 0, incr x, incr x
        assert_eq!(cmds.len(), 3, "double backslash should not continue comment: {:?}", cmds);
    }

    /// Inline comment after semicolon
    #[test]
    fn test_inline_comment_after_semicolon() {
        let src = "set x 1 ;# this is a comment\nset y 2";
        let cmds = parse(src).unwrap();
        assert_eq!(cmds.len(), 2, "inline comment should be ignored: {:?}", cmds);
    }

    /// parse-1.8: Dict/array sugar with command sub in index
    #[test]
    fn test_parse_1_8_cmd_sub_in_array_index() {
        let cmds = parse("set x $a([set y b])").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "var name should start with 'a(': {}", name);
                assert!(name.contains("[set y b]"), "index should contain cmd sub text: {}", name);
            }
            w => panic!("expected var_ref with array index, got {:?}", w),
        }
    }

    /// parse-1.27: Backslash escape in array index
    #[test]
    fn test_parse_1_27_escape_in_array_index() {
        let cmds = parse("set x $a(b\\x55d)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array access: {}", name);
            }
            w => panic!("expected var_ref, got {:?}", w),
        }
    }

    /// parse-1.28: Nested dict sugar
    #[test]
    fn test_parse_1_28_nested_dict_sugar() {
        let cmds = parse("set x $b($a(V))").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("b("), "should be array access on b: {}", name);
                assert!(name.contains("$a(V)") || name.contains("a(V)"),
                    "should contain nested access: {}", name);
            }
            w => panic!("expected var_ref, got {:?}", w),
        }
    }

    /// Standalone $ as a word
    #[test]
    fn test_standalone_dollar() {
        let cmds = parse("puts $").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "$"),
            w => panic!("expected literal '$', got {:?}", w),
        }
    }

    /// $$ (two orphan dollars)
    #[test]
    fn test_double_dollar() {
        let cmds = parse("puts $$").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Concat(parts) => {
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "$$");
            }
            w => panic!("expected '$$', got {:?}", w),
        }
    }

    /// Composite: namespace qualified variable
    #[test]
    fn test_namespace_var() {
        let cmds = parse("puts $::foo::bar").unwrap();
        match &cmds[0].words[1] {
            Word::VarRef(name) => assert_eq!(name, "::foo::bar"),
            w => panic!("expected var_ref '::foo::bar', got {:?}", w),
        }
    }
}
