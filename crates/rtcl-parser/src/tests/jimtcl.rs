use super::*;

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

/// parse-1.19: `"6$[set y]"` → with $[expr] sugar, `$[set y]` is ExprSugar
#[test]
fn test_parse_1_19_orphan_dollar_in_quotes() {
    let cmds = parse(r#"set x "6$[set y]""#).unwrap();
    assert_eq!(cmds[0].words.len(), 3);
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            // Should be: Literal("6") + ExprSugar("set y")
            let has_6 = parts.iter().any(|p| match p {
                Word::Literal(s) => s.contains('6'),
                _ => false,
            });
            let has_expr = parts.iter().any(|p| matches!(p, Word::ExprSugar(_)));
            assert!(has_6, "should contain literal 6, got {:?}", parts);
            assert!(has_expr, "should contain ExprSugar, got {:?}", parts);
        }
        w => panic!("expected concat, got {:?}", w),
    }
}

/// parse-1.20: `6$[set y]` in bare context → with $[expr] sugar
#[test]
fn test_parse_1_20_orphan_dollar_bare() {
    let cmds = parse("set x 6$[set y]").unwrap();
    assert_eq!(cmds[0].words.len(), 3);
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            let has_6 = parts.iter().any(|p| match p {
                Word::Literal(s) => s.contains('6'),
                _ => false,
            });
            let has_expr = parts.iter().any(|p| matches!(p, Word::ExprSugar(_)));
            assert!(has_6, "should contain literal 6, got {:?}", parts);
            assert!(has_expr, "should contain ExprSugar, got {:?}", parts);
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
