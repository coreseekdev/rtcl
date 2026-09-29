use super::*;

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

#[test]
fn test_trailing_garbage_after_close_quote_errors() {
    // tclsh: `set v "ab"cd` -> extra characters after close-quote
    let err = crate::parse("set v \"ab\"cd").unwrap_err();
    assert_eq!(err.message, "extra characters after close-quote");
}

#[test]
fn test_trailing_garbage_after_close_brace_errors() {
    // tclsh: `expr {1}junk` -> extra characters after close-brace
    let err = crate::parse("expr {1}junk").unwrap_err();
    assert_eq!(err.message, "extra characters after close-brace");
}

#[test]
fn test_no_garbage_after_delimiters_still_parses() {
    // Whitespace/;/]/end after the closer are all fine; mid-bare quotes
    // and braces stay literal.
    for src in [
        "set v \"ab\" ; set w {a} ; set y x\"y\"z",
        "set v [string cat {a}]",
        "set v \"ab\"",
        "set v {a}",
    ] {
        assert!(crate::parse(src).is_ok(), "should parse: {}", src);
    }
}
