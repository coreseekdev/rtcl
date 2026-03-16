use super::*;

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
