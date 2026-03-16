use super::*;

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
