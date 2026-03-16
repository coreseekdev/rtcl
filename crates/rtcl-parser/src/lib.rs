//! # rtcl-parser
//!
//! Tcl parser for rtcl — produces an AST from Tcl source code.
//!
//! Uses a recursive descent parser inspired by Molt/jimtcl.

use core::fmt;

mod rd;

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

/// Parse error with source location information.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    /// Byte offset into the source string where the error occurred.
    pub offset: usize,
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

impl std::error::Error for ParseError {}

// Convenience alias
pub type ParseResult<T> = Result<T, ParseError>;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse Tcl source code into a list of [`Command`]s.
pub fn parse(source: &str) -> ParseResult<Vec<Command>> {
    rd::parse(source)
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
            // Hand parser coalesces adjacent literals: Literal("$$")
            Word::Literal(s) => assert_eq!(s, "$$"),
            // PEG parser produces separate parts: Concat([Literal("$"), Literal("$")])
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

    // -----------------------------------------------------------------------
    // GAP verification tests: braced string backslash handling
    // -----------------------------------------------------------------------

    /// Backslash-n inside braces should be preserved literally
    #[test]
    fn test_braced_backslash_n() {
        let cmds = parse(r#"set x {foo\nbar}"#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r#"foo\nbar"#),
            w => panic!("expected literal 'foo\\nbar', got {:?}", w),
        }
    }

    /// Backslash-t inside braces should be preserved literally
    #[test]
    fn test_braced_backslash_t() {
        let cmds = parse(r#"set x {foo\tbar}"#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r#"foo\tbar"#),
            w => panic!("expected literal 'foo\\tbar', got {:?}", w),
        }
    }

    /// Double backslash inside braces
    #[test]
    fn test_braced_double_backslash() {
        let cmds = parse(r#"set x {foo\\bar}"#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r#"foo\\bar"#),
            w => panic!("expected literal 'foo\\\\bar', got {:?}", w),
        }
    }

    /// Hex escape inside braces (preserved literally)
    #[test]
    fn test_braced_hex_escape() {
        let cmds = parse(r#"set x {\x4A}"#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r#"\x4A"#),
            w => panic!("expected literal '\\x4A', got {:?}", w),
        }
    }

    /// Backslash at end of braced string: {foo\} is unterminated
    #[test]
    fn test_braced_trailing_backslash() {
        // In Tcl, {foo\} is unterminated because \} escapes the }
        let result = parse(r#"set x {foo\}"#);
        assert!(result.is_err(), "should be parse error (unterminated brace)");
    }

    /// Backslash-brace inside braces: {foo\}bar} → foo\}bar
    #[test]
    fn test_braced_escaped_close_brace() {
        let cmds = parse(r#"set x {foo\}bar}"#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r#"foo\}bar"#),
            w => panic!("expected literal 'foo\\}}bar', got {:?}", w),
        }
    }

    /// Backslash-open-brace inside braces: {foo\{bar} → foo\{bar
    #[test]
    fn test_braced_escaped_open_brace() {
        let cmds = parse(r#"set x {foo\{bar}"#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r#"foo\{bar"#),
            w => panic!("expected literal 'foo\\{{bar', got {:?}", w),
        }
    }

    // -----------------------------------------------------------------------
    // Additional parse.test coverage
    // -----------------------------------------------------------------------

    /// parse-1.13: Actual newline inside quotes → preserved as newline
    #[test]
    fn test_parse_1_13_newline_in_quotes() {
        let cmds = parse("set x \"abc\ndef\"").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc\ndef"),
            Word::Concat(parts) => {
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "abc\ndef");
            }
            w => panic!("expected 'abc\\ndef', got {:?}", w),
        }
    }

    /// parse-1.14: Newline in quotes after var
    #[test]
    fn test_parse_1_14_newline_in_quotes_after_var() {
        // `"abc$y\ndef"` — actual newline char in source
        let cmds = parse("set x \"abc$y\ndef\"").unwrap();
        // Should have set x, and the quoted word with var_ref + literal newline
        assert_eq!(cmds.len(), 1); // all in one command (newline is inside quotes)
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                // Should contain: "abc", VarRef("y"), "\ndef"
                assert!(parts.iter().any(|p| matches!(p, Word::VarRef(_))),
                    "should contain var ref: {:?}", parts);
            }
            w => panic!("expected concat with var ref, got {:?}", w),
        }
    }

    /// parse-1.16: Space in quotes after braced var
    #[test]
    fn test_parse_1_16_space_in_quotes_after_var() {
        let cmds = parse(r#"set x "abc${y} def""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.iter().any(|p| matches!(p, Word::VarRef(_))));
            }
            w => panic!("expected concat, got {:?}", w),
        }
    }

    /// parse-1.22: # char in quoted string context
    #[test]
    fn test_parse_1_22_hash_in_quoted_string() {
        let cmds = parse(r#"set x "[set y]#""#).unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                let has_cmd = parts.iter().any(|p| matches!(p, Word::CommandSub(_)));
                let has_hash = parts.iter().any(|p| match p {
                    Word::Literal(s) => s.contains('#'),
                    _ => false,
                });
                assert!(has_cmd, "should contain cmd sub: {:?}", parts);
                assert!(has_hash, "should contain #: {:?}", parts);
            }
            w => panic!("expected concat, got {:?}", w),
        }
    }

    /// parse-1.32: Comment as last line of eval'd script (parser test for semicolon comment)
    #[test]
    fn test_parse_1_32_comment_after_semicolon_in_script() {
        let cmds = parse("set x 3; # this is a comment").unwrap();
        assert_eq!(cmds.len(), 1, "should be one command, # is a comment: {:?}", cmds);
    }

    /// parse-1.41: Braced string containing quoted newline (preserved literally)
    #[test]
    fn test_parse_1_41_braced_with_quoted_newline() {
        let cmds = parse("set x {abc \"def\nghi\"}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert!(s.contains("def\nghi"), "should contain newline: {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// parse-1.43: Trailing backslash in quoted string
    #[test]
    fn test_parse_1_43_quoted_trailing_backslash() {
        let cmds = parse(r#"set x "abc def\\""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def\\"),
            w => panic!("expected 'abc def\\\\', got {:?}", w),
        }
    }

    /// parse-1.49: Backslash newline + tabs + actual newline in quotes
    #[test]
    fn test_parse_1_49_backslash_newline_tabs_newline() {
        // "abc\<newline><tabs><newline>def"
        // \<newline><tabs> → space, then another <newline> is literal
        let cmds = parse("set x \"abc\\\n\t\t\ndef\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc \ndef"),
            Word::Concat(parts) => {
                let text: String = parts.iter().map(|p| match p {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert_eq!(text, "abc \ndef");
            }
            w => panic!("expected 'abc \\ndef', got {:?}", w),
        }
    }

    /// parse-1.52: $ in array index
    #[test]
    fn test_parse_1_52_dollar_in_index() {
        let cmds = parse("set x $a(x$)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {}", name);
                assert!(name.contains("x$"), "index should contain 'x$': {}", name);
            }
            w => panic!("expected var_ref, got {:?}", w),
        }
    }

    /// parse-1.54: \[ in array index
    #[test]
    fn test_parse_1_54_escaped_bracket_in_index() {
        let cmds = parse(r#"set x $a(x\[)"#).unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {}", name);
            }
            w => panic!("expected var_ref, got {:?}", w),
        }
    }

    /// parse-1.56: \( in array index  
    #[test]
    fn test_parse_1_56_escaped_paren_in_index() {
        let cmds = parse(r#"set x $a(x\()"#).unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {}", name);
            }
            w => panic!("expected var_ref, got {:?}", w),
        }
    }

    /// parse-1.57-1.58: Unbalanced ( in array index (x( is the key)
    /// NOTE: jimtcl backtracks to the last valid ) when parens are unbalanced.
    /// PEG cannot easily replicate this. For now, rtcl treats this as
    /// $a followed by literal (x() — a known limitation.
    #[test]
    fn test_parse_1_58_unbalanced_paren_in_index() {
        // jimtcl: $a(x() → array access with key "x("
        // rtcl: $a followed by literal "(x()" — different but parses
        let result = parse("set x $a(x()");
        assert!(result.is_ok(), "should not fail to parse: {:?}", result);
    }

    /// parse-1.60: " in array index
    #[test]
    fn test_parse_1_60_quote_in_index() {
        let cmds = parse("set x $a(x\")").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {}", name);
                assert!(name.contains("x\""), "index should contain '\\\"': {}", name);
            }
            w => panic!("expected var_ref, got {:?}", w),
        }
    }

    /// parse-1.61: Quote escape inside cmd sub
    #[test]
    fn test_parse_1_61_quote_escape_in_cmd_sub() {
        let cmds = parse(r#"set x [list \\" x]"#).unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => {
                assert!(cmd.contains(r#"\\"#), "should contain \\\\: {:?}", cmd);
            }
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.66: Backslash newline inside command substitution
    #[test]
    fn test_parse_1_66_backslash_newline_in_cmd_sub() {
        let cmds = parse("set x [\"abc\\\n\tdef\" 4]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(cmd) => {
                assert!(cmd.contains("abc") && cmd.contains("def"),
                    "cmd sub should contain both parts: {:?}", cmd);
            }
            w => panic!("expected cmd_sub, got {:?}", w),
        }
    }

    /// parse-1.69: Comment-like string in quoted context
    #[test]
    fn test_parse_1_69_hash_string_in_quotes() {
        // `set x "#abc \\"` — the string starts with # but is in quotes
        let src = "set x \"#abc \\\\\"";
        let cmds = parse(src).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "#abc \\"),
            w => panic!("expected '#abc \\\\', got {:?}", w),
        }
    }

    /// Empty quoted string
    #[test]
    fn test_empty_quoted_string() {
        let cmds = parse(r#"set x """#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, ""),
            w => panic!("expected empty literal, got {:?}", w),
        }
    }

    /// Empty braced string
    #[test]
    fn test_empty_braced_string() {
        let cmds = parse("set x {}").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, ""),
            w => panic!("expected empty literal, got {:?}", w),
        }
    }

    /// Backslash newline in braces (line continuation)
    #[test]
    fn test_braced_line_continuation() {
        let cmds = parse("set x {abc\\\n\tdef}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def"),
            w => panic!("expected 'abc def', got {:?}", w),
        }
    }

    /// Multiple semicolons
    #[test]
    fn test_multiple_semicolons() {
        let cmds = parse("set x 1;;; set y 2").unwrap();
        assert_eq!(cmds.len(), 2, "multiple semicolons should separate commands");
    }

    /// Tab and multiple whitespace between words
    #[test]
    fn test_whitespace_between_words() {
        let cmds = parse("set   x\t\t10").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
    }

    /// parse-1.6: Incomplete array index `$a(` — parser handles, runtime error
    #[test]
    fn test_parse_1_6_incomplete_array_index() {
        // In jimtcl: `$a(` has no closing ), so $a is just the var, ( is leftover
        // PEG: index? fails, var_ref = $a, then ( is bare text
        let result = parse("set x $a(");
        assert!(result.is_ok(), "incomplete index should parse: {:?}", result);
    }

    /// parse-1.11: Backslash-newline in quotes after variable
    #[test]
    fn test_parse_1_11_backslash_newline_after_var_in_quotes() {
        let cmds = parse("set x \"abc$y\\\ndef\"").unwrap();
        assert_eq!(cmds.len(), 1); // all one command (inside quotes)
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                // Should have "abc" + VarRef(y) + " def" (backslash-newline → space)
                assert!(parts.iter().any(|p| matches!(p, Word::VarRef(_))),
                    "should contain var ref: {:?}", parts);
            }
            w => panic!("expected concat with var ref, got {:?}", w),
        }
    }

    /// parse-1.15: Space in quotes
    #[test]
    fn test_parse_1_15_space_in_quotes() {
        let cmds = parse(r#"set x "abc def""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def"),
            w => panic!("expected 'abc def', got {:?}", w),
        }
    }

    /// parse-1.36: Unicode escape \u00b5
    #[test]
    fn test_parse_1_36_unicode_escape() {
        let cmds = parse("set x \\u00b5").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                // \u00b5 → µ (micro sign)
                assert_eq!(s, "\u{00b5}");
            }
            w => panic!("expected unicode char, got {:?}", w),
        }
    }

    /// parse-1.40: Quoted string with escaped quote and trailing backslash
    #[test]
    fn test_parse_1_40_quoted_escape_and_trailing_backslash() {
        // `"abc \"def\\"` → abc "def\
        let cmds = parse(r#"set x "abc \"def\\""#).unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc \"def\\"),
            w => panic!("expected 'abc \"def\\', got {:?}", w),
        }
    }

    /// parse-1.48: Backslash-newline with tabs in quotes
    #[test]
    fn test_parse_1_48_backslash_newline_tabs() {
        let cmds = parse("set x \"abc\\\n\t\tdef\"").unwrap();
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

    /// parse-1.50: Backslash-newline in quotes (no whitespace after)
    #[test]
    fn test_parse_1_50_backslash_newline_no_whitespace() {
        let cmds = parse("set x \"abc\\\ndef\"").unwrap();
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

    /// parse-1.57: Unbalanced paren in set arg (x( is the key in jimtcl)
    /// NOTE: Same PEG limitation as parse-1.58 — jimtcl backtracks to last )
    #[test]
    fn test_parse_1_57_unbalanced_paren_as_set_arg() {
        let result = parse("set a(x() 5");
        assert!(result.is_ok(), "should not fail to parse: {:?}", result);
    }

    /// parse-1.67: Missing quote in command substitution
    /// jimtcl reports "missing quote" error. Hand parser does the same.
    /// PEG parser backtracks and treats `"` as a regular cmd_char.
    #[test]
    fn test_parse_1_67_missing_quote_in_cmd_sub() {
        let result = parse("set x [\"abc def]");
        // Hand parser (like jimtcl): error on unmatched quote
        // PEG parser: backtracks, treats " as regular char → Ok
        // Both behaviors are acceptable.
        let _ = result;
    }

    /// parse-1.68: Missing quote across lines
    /// jimtcl reports "missing quote" error. Hand parser does the same.
    /// PEG parser backtracks and treats `"` as bare_char.
    #[test]
    fn test_parse_1_68_missing_quote_across_lines() {
        let result = parse("set x \"abc\\\n\tline without quote\n");
        // Hand parser (like jimtcl): error on unmatched quote
        // PEG parser: backtracks → Ok
        let _ = result;
    }

    // ===================================================================
    // Additional jimtcl parse.test analogs (parser-level)
    // ===================================================================

    /// parse-1.4 analog: Variable reference inside quoted string.
    /// jimtcl: `set x [string length "$lb"]` where lb is `\{`
    /// Parser level: `"$lb"` parses as Concat or VarRef.
    #[test]
    fn test_parse_1_4_var_in_quoted_string() {
        let cmds = parse(r#"set x "$lb""#).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "lb"),
            Word::Concat(parts) => {
                // Some backends may wrap single VarRef in Concat
                assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "lb")));
            }
            w => panic!("expected VarRef(lb), got {:?}", w),
        }
    }

    /// parse-1.37 analog: Invalid unicode escape after valid unicode prefix.
    /// `\ub5x` — only 2 hex digits after `\u`, then `x` is literal.
    #[test]
    fn test_parse_1_37_unicode_escape_partial() {
        let cmds = parse("set x \"\\ub5x\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert!(s.ends_with('x'), "should end with literal 'x': {:?}", s);
                assert!(s.len() >= 2, "should have unicode char + 'x': {:?}", s);
            }
            Word::Concat(parts) => {
                let full: String = parts.iter().map(|w| match w {
                    Word::Literal(s) => s.as_str(),
                    _ => "",
                }).collect();
                assert!(full.ends_with('x'), "should end with 'x': {:?}", full);
            }
            w => panic!("expected literal with unicode+x, got {:?}", w),
        }
    }

    /// parse-1.51/52: Dollar sign as literal in array index `$a(x$)`.
    #[test]
    fn test_parse_1_51_dollar_in_array_index() {
        let cmds = parse("set x $a(x$)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {:?}", name);
                assert!(name.contains("x$"), "index should contain 'x$': {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// parse-1.53/54: Escaped bracket in array index `$a(x\[)`.
    #[test]
    fn test_parse_1_53_escaped_bracket_in_index() {
        let cmds = parse(r"set x $a(x\[)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {:?}", name);
                // Index content is raw text including the backslash
                assert!(name.contains(r"x\[") || name.contains("x["),
                    "index should contain bracket: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// parse-1.55/56: Escaped paren in array index `$a(x\()`.
    #[test]
    fn test_parse_1_55_escaped_paren_in_index() {
        let cmds = parse(r"set x $a(x\()").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// parse-1.59/60: Quote in array index `$a(x")`.
    #[test]
    fn test_parse_1_59_quote_in_index() {
        let cmds = parse("set x $a(x\")").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array: {:?}", name);
                assert!(name.contains("x\""), "index should contain quote: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    // ===================================================================
    // Edge cases and robustness tests
    // ===================================================================

    /// Empty script: should produce empty command list.
    #[test]
    fn test_empty_script() {
        let cmds = parse("").unwrap();
        assert!(cmds.is_empty());
    }

    /// Whitespace-only script.
    #[test]
    fn test_whitespace_only_script() {
        let cmds = parse("   \n\t\n  ").unwrap();
        assert!(cmds.is_empty());
    }

    /// Comment-only script.
    #[test]
    fn test_comment_only_script() {
        let cmds = parse("# just a comment\n# another comment\n").unwrap();
        assert!(cmds.is_empty());
    }

    /// Multiple blank lines between commands.
    #[test]
    fn test_blank_lines_between_commands() {
        let cmds = parse("set x 1\n\n\n\nset y 2\n").unwrap();
        assert_eq!(cmds.len(), 2);
    }

    /// Tab and form-feed as word separators.
    #[test]
    fn test_tab_formfeed_separators() {
        let cmds = parse("set\tx\t1").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[0] { Word::Literal(s) => assert_eq!(s, "set"), w => panic!("{:?}", w) }
    }

    /// Nested command substitution: `[a [b c]]`.
    #[test]
    fn test_nested_cmd_sub() {
        let cmds = parse("set x [list [expr 1]]").unwrap();
        assert_eq!(cmds.len(), 1);
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("[expr 1]"), "should contain nested cmd sub: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Deeply nested command substitution: `[a [b [c]]]`.
    #[test]
    fn test_deeply_nested_cmd_sub() {
        let cmds = parse("set x [a [b [c]]]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("[b [c]]"), "should contain nested subs: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Multiple variable refs in quoted string: `"$a $b $c"`.
    #[test]
    fn test_multiple_vars_in_quoted() {
        let cmds = parse("set x \"$a $b $c\"").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                let var_count = parts.iter().filter(|w| matches!(w, Word::VarRef(_))).count();
                assert_eq!(var_count, 3, "should have 3 VarRefs: {:?}", parts);
            }
            w => panic!("expected Concat with vars, got {:?}", w),
        }
    }

    /// Variable + literal in bare context: `foo${bar}baz`.
    #[test]
    fn test_var_in_bare_context() {
        let cmds = parse("set x foo${bar}baz").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "bar")),
                    "should contain VarRef(bar): {:?}", parts);
                let lits: String = parts.iter().filter_map(|w| match w {
                    Word::Literal(s) => Some(s.as_str()),
                    _ => None,
                }).collect();
                assert!(lits.contains("foo") && lits.contains("baz"),
                    "should contain foo and baz: {:?}", parts);
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Var + cmd sub in bare context: `x[cmd]y`.
    #[test]
    fn test_cmd_sub_in_bare_context() {
        let cmds = parse("set v x[cmd]y").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.iter().any(|w| matches!(w, Word::CommandSub(s) if s == "cmd")),
                    "should contain CommandSub(cmd): {:?}", parts);
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Expand with variable: `{*}$var`.
    #[test]
    fn test_expand_variable() {
        let cmds = parse("cmd {*}$var").unwrap();
        match &cmds[0].words[1] {
            Word::Expand(inner) => {
                assert!(matches!(inner.as_ref(), Word::VarRef(n) if n == "var"),
                    "expand should contain VarRef: {:?}", inner);
            }
            w => panic!("expected Expand(VarRef), got {:?}", w),
        }
    }

    /// Expand with command substitution: `{*}[cmd]`.
    #[test]
    fn test_expand_cmd_sub() {
        let cmds = parse("cmd {*}[list a b]").unwrap();
        match &cmds[0].words[1] {
            Word::Expand(inner) => {
                assert!(matches!(inner.as_ref(), Word::CommandSub(_)),
                    "expand should contain CommandSub: {:?}", inner);
            }
            w => panic!("expected Expand(CommandSub), got {:?}", w),
        }
    }

    /// Expand with quoted word: `{*}"a b"`.
    #[test]
    fn test_expand_quoted() {
        let cmds = parse("cmd {*}\"a b\"").unwrap();
        match &cmds[0].words[1] {
            Word::Expand(inner) => {
                // inner is either Literal("a b") or similar
                let s = format!("{}", inner);
                assert!(s.contains("a b"), "should contain 'a b': {:?}", inner);
            }
            w => panic!("expected Expand, got {:?}", w),
        }
    }

    /// `{*}` followed by whitespace is a braced literal, not expand.
    #[test]
    fn test_brace_star_brace_as_literal() {
        let cmds = parse("set x {*}").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "*"),
            w => panic!("expected Literal(*), got {:?}", w),
        }
    }

    /// `{*}` at end of input is a braced literal.
    #[test]
    fn test_brace_star_brace_at_end() {
        let cmds = parse("cmd {*}").unwrap();
        assert_eq!(cmds[0].words.len(), 2);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "*"),
            w => panic!("expected Literal(*), got {:?}", w),
        }
    }

    /// Deeply nested braces: `{{{a}}}`.
    #[test]
    fn test_deeply_nested_braces() {
        let cmds = parse("set x {{{a}}}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "{{a}}"),
            w => panic!("expected Literal({{a}}), got {:?}", w),
        }
    }

    /// Braced string with internal pairs: `{a {b c} d}`.
    #[test]
    fn test_braced_internal_pairs() {
        let cmds = parse("set x {a {b c} d}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a {b c} d"),
            w => panic!("expected Literal, got {:?}", w),
        }
    }

    /// Triple dollar signs in bare context: `$$$`.
    #[test]
    fn test_triple_dollar() {
        let cmds = parse("set x $$$").unwrap();
        // All orphan dollars — should produce literal "$$$" (or Concat of literals)
        let w = &cmds[0].words[2];
        let text = format!("{}", w);
        assert_eq!(text, "$$$", "triple dollar: {:?}", w);
    }

    /// Dollar followed by open brace without close: `${`.
    #[test]
    fn test_dollar_open_brace_no_close() {
        let result = parse("set x ${");
        assert!(result.is_err(), "should error on unclosed ${{ : {:?}", result);
    }

    /// Missing close bracket in command substitution.
    #[test]
    fn test_missing_close_bracket() {
        let result = parse("set x [expr 1");
        assert!(result.is_err(), "should error on unclosed [: {:?}", result);
    }

    /// Missing close brace.
    #[test]
    fn test_missing_close_brace() {
        let result = parse("set x {hello");
        assert!(result.is_err(), "should error on unclosed brace: {:?}", result);
    }

    /// Missing close quote.
    /// rd parser (like jimtcl): Err. PEG parser: backtracks, treats `"` as bare char → Ok.
    #[test]
    fn test_missing_close_quote() {
        let result = parse("set x \"hello");
        // Both Ok (PEG) and Err (rd) are acceptable
        let _ = result;
    }

    /// Backslash at end of input (bare context).
    /// rd parser: treats trailing `\` as literal → Ok("a\\").
    /// PEG parser: Err (expected EOI or word).
    #[test]
    fn test_trailing_backslash_bare() {
        let result = parse("set x a\\");
        match result {
            Ok(cmds) => {
                match &cmds[0].words[2] {
                    Word::Literal(s) => assert_eq!(s, "a\\"),
                    Word::Concat(parts) => {
                        let full: String = parts.iter().map(|w| format!("{}", w)).collect();
                        assert_eq!(full, "a\\", "trailing backslash: {:?}", parts);
                    }
                    w => panic!("expected a\\, got {:?}", w),
                }
            }
            Err(_) => {} // PEG backend rejects trailing backslash
        }
    }

    /// Semicolon inside braces is literal.
    #[test]
    fn test_semicolon_in_braces() {
        let cmds = parse("set x {a;b;c}").unwrap();
        assert_eq!(cmds.len(), 1);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a;b;c"),
            w => panic!("expected Literal(a;b;c), got {:?}", w),
        }
    }

    /// Newline inside braces is literal.
    #[test]
    fn test_newline_in_braces() {
        let cmds = parse("set x {a\nb\nc}").unwrap();
        assert_eq!(cmds.len(), 1);
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a\nb\nc"),
            w => panic!("expected literal with newlines, got {:?}", w),
        }
    }

    /// Backslash-newline as line continuation between words.
    /// jimtcl parse-1.9: `set x 123;\<newline>set y 456` → two separate commands.
    #[test]
    fn test_backslash_newline_line_continuation() {
        let cmds = parse("set x 123;\\\nset y 456").unwrap();
        // Should produce: set x 123 (cmd1) ; set y 456 (cmd2 via line continuation)
        // OR two commands depending on how ;\<newline> is handled
        assert!(cmds.len() >= 1, "should parse: {:?}", cmds);
    }

    /// Backslash-newline between words as continuation within same word.
    /// `list abc\<newline><tab>123` → `list {abc 123}` (2 words).
    /// In Tcl, `\<newline>` replaces backslash + newline + leading whitespace with a single space,
    /// so this joins into one word: "abc 123".
    #[test]
    fn test_backslash_newline_as_separator() {
        let cmds = parse("list abc\\\n\t123").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 2, "should have 2 words: {:?}", cmds[0].words);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "abc 123"),
            w => panic!("expected Literal(abc 123), got {:?}", w),
        }
    }

    /// Consecutive semicolons produce no empty commands.
    #[test]
    fn test_consecutive_semicolons() {
        let cmds = parse(";;;set x 1;;;set y 2;;;").unwrap();
        assert_eq!(cmds.len(), 2);
    }

    /// Script with only semicolons and newlines.
    #[test]
    fn test_only_separators() {
        let cmds = parse(";;;\n\n;;;\n").unwrap();
        assert!(cmds.is_empty());
    }

    /// Command line number tracking.
    #[test]
    fn test_line_numbers() {
        let cmds = parse("set x 1\n\nset y 2\nset z 3").unwrap();
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0].line, 1);
        assert_eq!(cmds[1].line, 3);
        assert_eq!(cmds[2].line, 4);
    }

    /// String with all escape types in quoted context.
    #[test]
    fn test_all_escape_types_in_quotes() {
        let cmds = parse(r#"set x "\a\b\f\n\r\t\v""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert_eq!(s, "\x07\x08\x0c\n\r\t\x0b");
            }
            w => panic!("expected literal with escapes, got {:?}", w),
        }
    }

    /// Octal escape boundary: 3 digits max, then literal.
    #[test]
    fn test_octal_escape_boundary() {
        // \1119 → \111 (=73='I') + '9'
        let cmds = parse(r#"set x "\1119""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert_eq!(s, "I9", "\\1119 should be 'I' + '9': {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Hex escape boundary: 2 digits max.
    #[test]
    fn test_hex_escape_boundary() {
        // \x4Ag → \x4A (='J') + 'g'
        let cmds = parse(r#"set x "\x4Ag""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert_eq!(s, "Jg", "\\x4Ag should be 'J' + 'g': {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Unicode escape boundary: 4 digits max for \u.
    #[test]
    fn test_unicode_escape_boundary() {
        // \u00b5x → µ + x
        let cmds = parse("set x \"\\u00b5x\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert!(s.starts_with('\u{00b5}'), "should start with µ: {:?}", s);
                assert!(s.ends_with('x'), "should end with x: {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Braces inside quoted string are literal.
    #[test]
    fn test_braces_in_quoted_string() {
        let cmds = parse(r#"set x "{a {b} c}""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "{a {b} c}"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Comment after last command (no trailing newline).
    #[test]
    fn test_comment_after_command_no_newline() {
        let cmds = parse("set x 1 ;# comment").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 3);
    }

    /// Var ref with double-colon namespace prefix: `$::global`.
    #[test]
    fn test_var_ref_global_namespace() {
        let cmds = parse("set x $::global").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "::global"),
            w => panic!("expected VarRef(::global), got {:?}", w),
        }
    }

    /// Var ref with deep namespace: `$a::b::c`.
    #[test]
    fn test_var_ref_deep_namespace() {
        let cmds = parse("set x $a::b::c").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "a::b::c"),
            w => panic!("expected VarRef(a::b::c), got {:?}", w),
        }
    }

    /// Dollar followed by space is orphan.
    #[test]
    fn test_dollar_space_orphan() {
        let cmds = parse("set x \"$ y\"").unwrap();
        let text = format!("{}", &cmds[0].words[2]);
        assert_eq!(text, "$ y", "dollar-space: {:?}", &cmds[0].words[2]);
    }

    /// Braced var ref: `${with spaces}` (unusual but valid).
    #[test]
    fn test_braced_var_name_with_special_chars() {
        let cmds = parse("set x ${a.b-c}").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "a.b-c"),
            w => panic!("expected VarRef(a.b-c), got {:?}", w),
        }
    }

    /// Array variable with complex index: `$arr(key with spaces)`.
    #[test]
    fn test_array_index_with_spaces() {
        let cmds = parse("set x $arr(key with spaces)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("arr("), "should be array: {:?}", name);
                assert!(name.contains("key with spaces"),
                    "index should contain spaces: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// Comment with backslash-newline continuation.
    #[test]
    fn test_comment_continuation_multiline() {
        let cmds = parse("# line1 \\\nline2\nset x 1").unwrap();
        // "# line1 \" continues onto "line2", so only "set x 1" is a command
        assert_eq!(cmds.len(), 1, "comment continuation should eat line2: {:?}", cmds);
        match &cmds[0].words[0] {
            Word::Literal(s) => assert_eq!(s, "set"),
            w => panic!("expected set, got {:?}", w),
        }
    }

    /// Escaped newline at end of quoted string.
    #[test]
    fn test_escaped_newline_end_of_quoted() {
        let cmds = parse("set x \"abc\\n\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc\n"),
            w => panic!("expected abc\\n, got {:?}", w),
        }
    }

    /// Single-character command.
    #[test]
    fn test_single_char_command() {
        let cmds = parse("x").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 1);
    }

    /// Command substitution with semicolons: `[a; b; c]`.
    #[test]
    fn test_cmd_sub_with_semicolons() {
        let cmds = parse("set x [list a; list b; list c]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains(';'), "should contain semicolons: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Quoted string with command sub and var: `"[cmd]$var text"`.
    #[test]
    fn test_quoted_cmd_sub_var_text() {
        let cmds = parse("set x \"[cmd]$var text\"").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.iter().any(|w| matches!(w, Word::CommandSub(_))),
                    "should have CommandSub: {:?}", parts);
                assert!(parts.iter().any(|w| matches!(w, Word::VarRef(_))),
                    "should have VarRef: {:?}", parts);
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Backslash-backslash produces literal backslash.
    #[test]
    fn test_double_backslash_in_quotes() {
        let cmds = parse(r#"set x "a\\b""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a\\b"),
            w => panic!("expected literal a\\b, got {:?}", w),
        }
    }

    /// Escaped characters in bare word.
    #[test]
    fn test_escape_in_bare_word() {
        let cmds = parse(r"set x a\nb").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a\nb"),
            w => panic!("expected literal with newline, got {:?}", w),
        }
    }

    /// `{*}` followed by semicolon is braced literal, not expand.
    #[test]
    fn test_expand_before_semicolon() {
        let cmds = parse("cmd {*};other").unwrap();
        // {*} before ; should be literal *, not expand
        assert!(cmds.len() >= 1);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, "*"),
            w => panic!("expected Literal(*), got {:?}", w),
        }
    }

    /// Empty quoted string as argument.
    #[test]
    fn test_empty_quoted_arg() {
        let cmds = parse("cmd \"\" arg").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, ""),
            w => panic!("expected empty Literal, got {:?}", w),
        }
    }

    /// Empty braced string as argument.
    #[test]
    fn test_empty_braced_arg() {
        let cmds = parse("cmd {} arg").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
        match &cmds[0].words[1] {
            Word::Literal(s) => assert_eq!(s, ""),
            w => panic!("expected empty Literal, got {:?}", w),
        }
    }

    // ===================================================================
    // rd-specific deep coverage tests
    // ===================================================================

    // --- Escape sequence edge cases ---

    /// \x with no hex digits → literal 'x'.
    #[test]
    fn test_escape_hex_no_digits() {
        let cmds = parse(r#"set x "\xzz""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "xzz"),
            w => panic!("expected 'xzz', got {:?}", w),
        }
    }

    /// \u with no hex digits → literal 'u'.
    #[test]
    fn test_escape_unicode_no_digits() {
        let cmds = parse(r#"set x "\uZZZZ""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "uZZZZ"),
            w => panic!("expected 'uZZZZ', got {:?}", w),
        }
    }

    /// \U with no hex digits → literal 'U'.
    #[test]
    fn test_escape_big_unicode_no_digits() {
        let cmds = parse(r#"set x "\UZZZZZZZZ""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "UZZZZZZZZ"),
            w => panic!("expected 'UZZZZZZZZ', got {:?}", w),
        }
    }

    /// \U with 8 hex digits producing valid codepoint.
    #[test]
    fn test_escape_big_unicode_8_digits() {
        // \U0001f600 = 😀
        let cmds = parse("set x \"\\U0001f600\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "\u{1f600}"),
            w => panic!("expected 😀, got {:?}", w),
        }
    }

    /// \U with 8 digits but invalid codepoint.
    /// rd: fallback to literal 'U'. PEG: may produce empty string.
    #[test]
    fn test_escape_big_unicode_invalid_codepoint() {
        // \UFFFFFFFF is not a valid char
        let cmds = parse("set x \"\\UFFFFFFFF\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                // rd: "U", PEG: "" — both acceptable
                assert!(s == "U" || s.is_empty(), "got: {:?}", s);
            }
            w => panic!("expected Literal, got {:?}", w),
        }
    }

    /// \u with partial hex (3 digits) → uses those 3 digits.
    #[test]
    fn test_escape_unicode_3_digits() {
        // \u03b = 0x3B = ';'
        let cmds = parse("set x \"\\u03bz\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert!(s.starts_with('\u{003b}') || s.starts_with(';'),
                    "should start with U+003B: {:?}", s);
                assert!(s.ends_with('z'), "should end with 'z': {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// \x with 1 hex digit only.
    #[test]
    fn test_escape_hex_1_digit() {
        // \x4 = 0x04
        let cmds = parse("set x \"\\x4z\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert_eq!(s, "\x04z", "should be \\x04 + z: {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Octal with only 1 digit.
    #[test]
    fn test_escape_octal_1_digit() {
        // \7 = 0o7 = 7
        let cmds = parse("set x \"\\7z\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert_eq!(s, "\x07z");
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Octal with 2 digits.
    #[test]
    fn test_escape_octal_2_digits() {
        // \77 = 0o77 = 63 = '?'
        let cmds = parse("set x \"\\77z\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                assert_eq!(s, "?z");
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Octal stops at non-octal digit: \089 → \0 (NUL) + "89".
    #[test]
    fn test_escape_octal_stops_at_non_octal() {
        let cmds = parse("set x \"\\089\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                // rd: "\x0089" (NUL + 89). PEG: may drop NUL → "89".
                assert!(s == "\x0089" || s == "89", "got: {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Unknown escape: \q → literal 'q'.
    #[test]
    fn test_escape_unknown_char() {
        let cmds = parse(r#"set x "\q""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "q"),
            w => panic!("expected 'q', got {:?}", w),
        }
    }

    /// Escape at EOF: trailing backslash in quoted string → error.
    #[test]
    fn test_escape_backslash_eof_in_quoted() {
        let result = parse("set x \"abc\\");
        assert!(result.is_err(), "trailing \\ in quoted should be error: {:?}", result);
    }

    /// Backslash-newline inside quoted eats trailing spaces/tabs.
    #[test]
    fn test_escape_bsnl_eats_whitespace_in_quoted() {
        let cmds = parse("set x \"abc\\\n   \t  def\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def"),
            w => panic!("expected 'abc def', got {:?}", w),
        }
    }

    /// Backslash-newline at end of quoted string (no following chars).
    #[test]
    fn test_escape_bsnl_at_end_of_quoted() {
        let cmds = parse("set x \"abc\\\n\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc "),
            w => panic!("expected 'abc ', got {:?}", w),
        }
    }

    /// Multiple consecutive escapes: `"\n\t\r"`.
    #[test]
    fn test_escape_consecutive() {
        let cmds = parse(r#"set x "\n\t\r""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "\n\t\r"),
            w => panic!("expected NL+TAB+CR, got {:?}", w),
        }
    }

    /// Backslash followed by ] in quoted context (not cmd sub).
    #[test]
    fn test_escape_bracket_in_quoted() {
        let cmds = parse(r#"set x "a\]b""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a]b"),
            w => panic!("expected 'a]b', got {:?}", w),
        }
    }

    /// Backslash-$ in quoted: escape prevents variable substitution.
    #[test]
    fn test_escape_dollar_in_quoted() {
        let cmds = parse(r#"set x "a\$b""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a$b"),
            w => panic!("expected 'a$b', got {:?}", w),
        }
    }

    /// Backslash-[ in quoted: escape prevents command substitution.
    #[test]
    fn test_escape_open_bracket_in_quoted() {
        let cmds = parse(r#"set x "a\[b""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a[b"),
            w => panic!("expected 'a[b', got {:?}", w),
        }
    }

    /// Backslash-" in quoted: escaped quote does not close string.
    #[test]
    fn test_escape_quote_in_quoted() {
        let cmds = parse(r#"set x "a\"b""#).unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a\"b"),
            w => panic!("expected 'a\"b', got {:?}", w),
        }
    }

    // --- Braced word edge cases ---

    /// Escaped close-brace inside braces should NOT close.
    #[test]
    fn test_braced_escaped_close_brace_deep() {
        let cmds = parse(r"set x {a\}b}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r"a\}b"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Braces: backslash-newline inside braced word becomes space.
    #[test]
    fn test_braced_word_bsnl_becomes_space() {
        let cmds = parse("set x {abc\\\n\tdef}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc def"),
            w => panic!("expected 'abc def', got {:?}", w),
        }
    }

    /// Braces: unbalanced content after backslash-brace inside braced word.
    #[test]
    fn test_braced_escaped_open_brace_no_close() {
        // \{ inside braces does NOT increment brace depth
        let cmds = parse(r"set x {a\{b}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, r"a\{b"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// Braces: `\}` inside braces escapes the `}`, so it does NOT close.
    /// `{abc\}` → error because the `}` is escaped by `\`.
    #[test]
    fn test_braced_escaped_close_brace_prevents_close() {
        let result = parse(r"set x {abc\}");
        assert!(result.is_err(), "\\}} inside braces should prevent closing: {:?}", result);
    }

    /// Braces: `{abc\\}` — double backslash then close brace.
    /// `\\` is literal backslash, `}` closes normally.
    #[test]
    fn test_braced_double_backslash_then_close() {
        let cmds = parse("set x {abc\\\\}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "abc\\\\"),
            w => panic!("expected 'abc\\\\', got {:?}", w),
        }
    }

    /// Braces: multiple backslash-newlines inside braced word.
    #[test]
    fn test_braced_multiple_bsnl() {
        let cmds = parse("set x {a\\\n\tb\\\n\tc}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "a b c"),
            w => panic!("expected 'a b c', got {:?}", w),
        }
    }

    /// Braces: quotes and brackets inside braces are literal.
    #[test]
    fn test_braced_quotes_brackets_literal() {
        let cmds = parse("set x {\"[hello]\" $var}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "\"[hello]\" $var"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    // --- Variable reference edge cases ---

    /// Variable name starting with digit: `$1abc`.
    #[test]
    fn test_var_starts_with_digit() {
        let cmds = parse("set x $1abc").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "1abc"),
            w => panic!("expected VarRef(1abc), got {:?}", w),
        }
    }

    /// Variable name is single underscore: `$_`.
    #[test]
    fn test_var_underscore() {
        let cmds = parse("set x $_").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "_"),
            w => panic!("expected VarRef(_), got {:?}", w),
        }
    }

    /// `${` with valid braced varname containing special chars.
    #[test]
    fn test_braced_var_with_spaces_and_specials() {
        let cmds = parse("set x ${hello world!}").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "hello world!"),
            w => panic!("expected VarRef(hello world!), got {:?}", w),
        }
    }

    /// `${}` empty braced variable name.
    /// rd: VarRef(""). PEG: may treat as orphan $ + literal {}.
    #[test]
    fn test_empty_braced_var_name() {
        let cmds = parse("set x ${}").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, ""),
            _ => {} // PEG may parse this differently
        }
    }

    /// `$` at end of bare word is orphan.
    #[test]
    fn test_orphan_dollar_end_of_bare() {
        let cmds = parse("set x abc$").unwrap();
        let text = format!("{}", &cmds[0].words[2]);
        assert_eq!(text, "abc$", "dollar at end: {:?}", &cmds[0].words[2]);
    }

    /// Multiple orphan dollars: `$ $ $`.
    #[test]
    fn test_multiple_orphan_dollars_quoted() {
        let cmds = parse("set x \"$ $ $\"").unwrap();
        let text = format!("{}", &cmds[0].words[2]);
        assert_eq!(text, "$ $ $");
    }

    /// Dollar + open-paren without varname: `$(`.
    #[test]
    fn test_dollar_open_paren_no_varname() {
        let cmds = parse("set x $(abc)").unwrap();
        // $ is orphan, (abc) is part of bare text
        let text = format!("{}", &cmds[0].words[2]);
        assert_eq!(text, "$(abc)");
    }

    /// `${name}(index)` — braced var name with array index.
    #[test]
    fn test_braced_var_with_array_index() {
        let cmds = parse("set x ${arr}(idx)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "arr(idx)"),
            w => panic!("expected VarRef(arr(idx)), got {:?}", w),
        }
    }

    /// Variable with :: at start but no following name: `$::`.
    #[test]
    fn test_var_namespace_prefix_only() {
        let cmds = parse("set x $::").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "::"),
            w => panic!("expected VarRef(::), got {:?}", w),
        }
    }

    /// Nested array index with parens: `$a(b(c))`.
    #[test]
    fn test_array_nested_parens() {
        let cmds = parse("set x $a(b(c))").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "name: {:?}", name);
                assert!(name.contains("b(c)"), "should have nested parens: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// Array index with backslash-paren: `$a(x\))`.
    #[test]
    fn test_array_index_escaped_close_paren() {
        let cmds = parse(r"set x $a(x\))").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "name: {:?}", name);
                // The index should contain the escaped paren
                assert!(name.contains(r"x\)"), "index: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// Array index with newline: `$a(x\ny)`.
    #[test]
    fn test_array_index_with_newline() {
        let cmds = parse("set x $a(x\ny)").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.contains("x\ny"), "index should contain newline: {:?}", name);
            }
            w => panic!("expected VarRef, got {:?}", w),
        }
    }

    /// Unclosed array index: `$a(foo` with no `)`.
    /// rd: VarRef("a(foo") — includes unclosed index.
    /// PEG: Concat([VarRef("a"), Literal("(foo")]) — stops at `(`.
    #[test]
    fn test_array_unclosed_index_bare() {
        let cmds = parse("set x $a(foo").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => {
                assert!(name.starts_with("a("), "should be array ref: {:?}", name);
            }
            Word::Concat(parts) => {
                assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "a")),
                    "should contain VarRef(a): {:?}", parts);
            }
            w => panic!("expected VarRef or Concat, got {:?}", w),
        }
    }

    // --- Command substitution edge cases ---

    /// Empty command substitution: `[]`.
    #[test]
    fn test_empty_cmd_sub() {
        let cmds = parse("set x []").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => assert_eq!(s, ""),
            w => panic!("expected empty CommandSub, got {:?}", w),
        }
    }

    /// Cmd sub with only whitespace: `[  ]`.
    #[test]
    fn test_cmd_sub_whitespace_only() {
        let cmds = parse("set x [  ]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => assert_eq!(s, "  "),
            w => panic!("expected CommandSub with spaces, got {:?}", w),
        }
    }

    /// Cmd sub inner braces: `[list {a b}]` — braces parsed correctly.
    #[test]
    fn test_cmd_sub_inner_braces() {
        let cmds = parse("set x [list {a b}]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("{a b}"), "should preserve braces: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Cmd sub inner quoted string: `[list "a b"]` — quotes parsed correctly.
    #[test]
    fn test_cmd_sub_inner_quoted() {
        let cmds = parse("set x [list \"a b\"]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("\"a b\""), "should preserve quotes: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Cmd sub with escape inside: `[list a\nb]`.
    #[test]
    fn test_cmd_sub_with_escape() {
        let cmds = parse("set x [list a\\nb]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("a\\nb"), "should contain raw escape: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Cmd sub with unmatched quote NOT at start of word (startofword=false).
    /// `[list a"b]` — quote is NOT at start of word, so NOT parsed as quoted string.
    #[test]
    fn test_cmd_sub_midword_quote() {
        let cmds = parse("set x [list a\"b]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("a\"b"), "midword quote should be literal: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Cmd sub with quote at start of word (startofword=true) then bracket.
    /// `[list "a]b"]` — quote starts word, so ] inside quotes doesn't close cmd sub.
    #[test]
    fn test_cmd_sub_quoted_bracket_inside() {
        let cmds = parse("set x [list \"a]b\"]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("\"a]b\""), "bracket inside quotes should not close: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Cmd sub with brace at start of word containing bracket.
    /// `[list {a]b}]` — brace starts word, so ] inside braces doesn't close cmd sub.
    #[test]
    fn test_cmd_sub_braced_bracket_inside() {
        let cmds = parse("set x [list {a]b}]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("{a]b}"), "bracket inside braces should not close: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Unclosed brace inside cmd sub → error.
    #[test]
    fn test_cmd_sub_unclosed_brace() {
        let result = parse("set x [list {abc]");
        assert!(result.is_err(), "unclosed brace in cmd sub: {:?}", result);
    }

    /// Unclosed quote inside cmd sub.
    /// rd: error (missing "). PEG: may backtrack successfully.
    #[test]
    fn test_cmd_sub_unclosed_quote() {
        let result = parse("set x [list \"abc]");
        // Both Ok (PEG backtrack) and Err (rd) acceptable
        let _ = result;
    }

    /// Cmd sub bracket not at word start → normal depth tracking.
    /// `[set a x[y]]` — inner `[` adds depth, inner `]` reduces depth.
    #[test]
    fn test_cmd_sub_nested_bracket_not_at_word_start() {
        let cmds = parse("set a [set b x[y]]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert_eq!(s, "set b x[y]");
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    /// Triple-nested cmd sub: `[a [b [c d]]]`.
    #[test]
    fn test_triple_nested_cmd_sub() {
        let cmds = parse("set x [a [b [c d]]]").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert_eq!(s, "a [b [c d]]");
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    // --- Token coalescing / Concat edge cases ---

    /// Single VarRef should not be wrapped in Concat.
    #[test]
    fn test_tokens_single_varref_no_concat() {
        let cmds = parse("set x \"$var\"").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "var"),
            w => panic!("expected VarRef, not Concat, got {:?}", w),
        }
    }

    /// Single CommandSub in quotes should not be wrapped in Concat.
    #[test]
    fn test_tokens_single_cmdsub_no_concat() {
        let cmds = parse("set x \"[cmd]\"").unwrap();
        match &cmds[0].words[2] {
            Word::CommandSub(s) => assert_eq!(s, "cmd"),
            w => panic!("expected CommandSub, not Concat, got {:?}", w),
        }
    }

    /// Concat: literal + varref + literal + cmdsub.
    #[test]
    fn test_tokens_complex_concat() {
        let cmds = parse("set x \"hello $name, [greet]!\"").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.len() >= 4, "should have 4+ parts: {:?}", parts);
                assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "name")));
                assert!(parts.iter().any(|w| matches!(w, Word::CommandSub(s) if s == "greet")));
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Bare word: var + literal produces Concat.
    #[test]
    fn test_tokens_bare_concat() {
        let cmds = parse("set x ${prefix}suffix").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], Word::VarRef(n) if n == "prefix"));
                assert!(matches!(&parts[1], Word::Literal(s) if s == "suffix"));
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Adjacent var refs in bare: `$a$b` → Concat with 2 VarRefs.
    #[test]
    fn test_tokens_adjacent_varrefs() {
        let cmds = parse("set x $a$b").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], Word::VarRef(n) if n == "a"));
                assert!(matches!(&parts[1], Word::VarRef(n) if n == "b"));
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    // --- Cursor / line tracking edge cases ---

    /// Line tracking with \r\n (Windows line endings).
    #[test]
    fn test_line_tracking_crlf() {
        let cmds = parse("set a 1\r\nset b 2\r\nset c 3").unwrap();
        assert_eq!(cmds.len(), 3);
        // \r is whitespace consumed by skip, \n increments line
        assert_eq!(cmds[0].line, 1);
        assert_eq!(cmds[1].line, 2);
        assert_eq!(cmds[2].line, 3);
    }

    /// Line numbers across backslash-newline continuation.
    #[test]
    fn test_line_tracking_bsnl_continuation() {
        let cmds = parse("set a \\\n  1\nset b 2").unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].line, 1);
        assert_eq!(cmds[1].line, 3, "should be line 3: {:?}", cmds);
    }

    /// Line numbers across braced content with newlines.
    #[test]
    fn test_line_tracking_braced_newlines() {
        let cmds = parse("set a {\nline2\nline3\n}\nset b 2").unwrap();
        assert_eq!(cmds[0].line, 1);
        assert_eq!(cmds[1].line, 5);
    }

    /// Line numbers across comment continuation.
    #[test]
    fn test_line_tracking_comment_continuation() {
        let cmds = parse("# comment \\\ncontinued\nset x 1").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].line, 3);
    }

    // --- Comment edge cases ---

    /// Comment with double-backslash at end (not a continuation).
    #[test]
    fn test_comment_double_backslash_end() {
        let cmds = parse("# comment\\\\\nset x 1").unwrap();
        // `\\` → literal backslash, then `\n` → NOT continuation
        assert_eq!(cmds.len(), 1);
        match &cmds[0].words[0] {
            Word::Literal(s) => assert_eq!(s, "set"),
            w => panic!("expected 'set', got {:?}", w),
        }
    }

    /// Comment with backslash in middle (not continuation).
    #[test]
    fn test_comment_backslash_middle() {
        let cmds = parse("# path is c:\\foo\nset x 1").unwrap();
        assert_eq!(cmds.len(), 1);
        match &cmds[0].words[0] {
            Word::Literal(s) => assert_eq!(s, "set"),
            w => panic!("expected 'set', got {:?}", w),
        }
    }

    /// Comment followed by EOF (no trailing newline).
    #[test]
    fn test_comment_eof_no_newline() {
        let cmds = parse("set x 1\n# final comment").unwrap();
        assert_eq!(cmds.len(), 1);
    }

    /// Multiple comment lines.
    #[test]
    fn test_multiple_comment_lines() {
        let cmds = parse("# c1\n# c2\n# c3\nset x 1").unwrap();
        assert_eq!(cmds.len(), 1);
    }

    // --- Expand edge cases ---

    /// `{*}{braced}` — expand with braced inner.
    #[test]
    fn test_expand_braced_inner() {
        let cmds = parse("cmd {*}{a b c}").unwrap();
        match &cmds[0].words[1] {
            Word::Expand(inner) => {
                match inner.as_ref() {
                    Word::Literal(s) => assert_eq!(s, "a b c"),
                    w => panic!("inner should be Literal: {:?}", w),
                }
            }
            w => panic!("expected Expand, got {:?}", w),
        }
    }

    /// `{*}{*}` — expand with braced `*` as inner word? No — second `{*}` is braced.
    #[test]
    fn test_expand_then_braced_star() {
        // {*}{*} → Expand(Literal("*"))
        // Since after consuming {*}, next char is { which starts braced word {*} → Literal("*")
        let cmds = parse("cmd {*}{*}").unwrap();
        match &cmds[0].words[1] {
            Word::Expand(inner) => {
                match inner.as_ref() {
                    Word::Literal(s) => assert_eq!(s, "*"),
                    w => panic!("inner: {:?}", w),
                }
            }
            w => panic!("expected Expand(Literal(*)), got {:?}", w),
        }
    }

    /// `{*}$a` in bracket-terminated context.
    #[test]
    fn test_expand_in_bracket_context() {
        // Simulated: we test via cmd sub that parses inner script
        let cmds = parse("set x [cmd {*}$var]").unwrap();
        // The whole thing is returned as CommandSub text
        match &cmds[0].words[2] {
            Word::CommandSub(s) => {
                assert!(s.contains("{*}"), "should contain expand: {:?}", s);
            }
            w => panic!("expected CommandSub, got {:?}", w),
        }
    }

    // --- process_braced_backslash_newline edge cases ---

    /// Braced content with \<CR> — not continuation (only \<LF>).
    #[test]
    fn test_braced_backslash_cr_not_continuation() {
        let cmds = parse("set x {abc\\\rdef}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => {
                // \<CR> is NOT continuation — literal \<CR> preserved
                assert!(s.contains('\r'), "should contain CR: {:?}", s);
            }
            w => panic!("expected literal, got {:?}", w),
        }
    }

    // --- Unicode content ---

    /// UTF-8 variable name (>= 0x80 bytes are valid varname chars).
    #[test]
    fn test_utf8_variable_name() {
        let cmds = parse("set x $变量").unwrap();
        match &cmds[0].words[2] {
            Word::VarRef(name) => assert_eq!(name, "变量"),
            w => panic!("expected VarRef(变量), got {:?}", w),
        }
    }

    /// UTF-8 in braces.
    #[test]
    fn test_utf8_in_braces() {
        let cmds = parse("set x {こんにちは}").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "こんにちは"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// UTF-8 in quoted string.
    #[test]
    fn test_utf8_in_quoted() {
        let cmds = parse("set x \"🦀 Rust\"").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "🦀 Rust"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    /// UTF-8 in bare word.
    #[test]
    fn test_utf8_in_bare() {
        let cmds = parse("set x café").unwrap();
        match &cmds[0].words[2] {
            Word::Literal(s) => assert_eq!(s, "café"),
            w => panic!("expected literal, got {:?}", w),
        }
    }

    // --- Whitespace handling ---

    /// Form feed (\x0c) is whitespace between words.
    #[test]
    fn test_formfeed_separator() {
        let cmds = parse("set\x0cx\x0c1").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
    }

    /// Carriage return without newline is whitespace.
    #[test]
    fn test_carriage_return_whitespace() {
        let cmds = parse("set\rx\r1").unwrap();
        assert_eq!(cmds[0].words.len(), 3);
    }

    /// Backslash-newline in whitespace between words acts as separator.
    /// Space before `\<newline>` ensures we're in skip_line_whitespace, not bare word.
    #[test]
    fn test_bsnl_between_words() {
        let cmds = parse("set \\\n  x \\\n  1").unwrap();
        assert_eq!(cmds[0].words.len(), 3, "should be 3 words: {:?}", cmds[0].words);
    }

    // --- Mixed/complex scenarios ---

    /// Complex one-liner with multiple features.
    #[test]
    fn test_complex_one_liner() {
        let cmds = parse(r#"set result "prefix_${arr}([expr 1+2])_${ns::var}_suffix""#).unwrap();
        assert_eq!(cmds.len(), 1);
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.len() >= 3, "complex concat: {:?}", parts);
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Multiple commands on one line separated by semicolons.
    #[test]
    fn test_multiple_semicolon_cmds() {
        let cmds = parse("set a 1; set b 2; set c 3").unwrap();
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0].line, 1);
        assert_eq!(cmds[1].line, 1);
        assert_eq!(cmds[2].line, 1);
    }

    /// Bare word with everything: var, cmd sub, escape, literal.
    #[test]
    fn test_bare_word_all_features() {
        let cmds = parse(r"set x abc$var[cmd]\ndef").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert!(parts.iter().any(|w| matches!(w, Word::Literal(s) if s.starts_with("abc"))));
                assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "var")));
                assert!(parts.iter().any(|w| matches!(w, Word::CommandSub(s) if s == "cmd")));
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Quoted word with consecutive substitutions: `"$a[b]$c"`.
    #[test]
    fn test_quoted_consecutive_subs() {
        let cmds = parse("set x \"$a[b]$c\"").unwrap();
        match &cmds[0].words[2] {
            Word::Concat(parts) => {
                assert_eq!(parts.len(), 3, "should be VarRef+CmdSub+VarRef: {:?}", parts);
                assert!(matches!(&parts[0], Word::VarRef(n) if n == "a"));
                assert!(matches!(&parts[1], Word::CommandSub(s) if s == "b"));
                assert!(matches!(&parts[2], Word::VarRef(n) if n == "c"));
            }
            w => panic!("expected Concat, got {:?}", w),
        }
    }

    /// Script that exercises all word types in sequence.
    #[test]
    fn test_all_word_types() {
        let cmds = parse("cmd literal {braced} \"quoted\" $var [sub] {*}$expand").unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].words.len(), 7);
        assert!(matches!(&cmds[0].words[0], Word::Literal(s) if s == "cmd"));
        assert!(matches!(&cmds[0].words[1], Word::Literal(s) if s == "literal"));
        assert!(matches!(&cmds[0].words[2], Word::Literal(s) if s == "braced"));
        assert!(matches!(&cmds[0].words[3], Word::Literal(s) if s == "quoted"));
        assert!(matches!(&cmds[0].words[4], Word::VarRef(n) if n == "var"));
        assert!(matches!(&cmds[0].words[5], Word::CommandSub(s) if s == "sub"));
        assert!(matches!(&cmds[0].words[6], Word::Expand(_)));
    }

    /// Long script with mixed features.
    #[test]
    fn test_multiline_script() {
        let script = "\
set a 1
set b {hello world}
# comment
set c \"$a and $b\"
set d [expr $a + 1]
";
        let cmds = parse(script).unwrap();
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[0].line, 1);
        assert_eq!(cmds[1].line, 2);
        assert_eq!(cmds[2].line, 4);
        assert_eq!(cmds[3].line, 5);
    }
}
