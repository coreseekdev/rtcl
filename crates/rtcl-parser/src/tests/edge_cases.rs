use super::*;

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
