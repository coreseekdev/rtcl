use super::*;

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

// --- Unicode content ---

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
