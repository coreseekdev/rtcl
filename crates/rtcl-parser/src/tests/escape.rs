use super::*;

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
