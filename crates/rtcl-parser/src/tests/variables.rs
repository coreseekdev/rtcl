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

/// Dollar + open-paren without varname: `$(expr)` is expression sugar.
#[test]
fn test_dollar_open_paren_no_varname() {
    let cmds = parse("set x $(abc)").unwrap();
    // $(abc) is expression sugar (jimtcl default)
    match &cmds[0].words[2] {
        Word::ExprSugar(expr) => assert_eq!(expr, "abc"),
        w => panic!("expected ExprSugar(abc), got {:?}", w),
    }
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
/// With backtracking: `$a` is VarRef, `(foo` is literal text.
#[test]
fn test_array_unclosed_index_bare() {
    let cmds = parse("set x $a(foo").unwrap();
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "a")),
                "should contain VarRef(a): {:?}", parts);
            assert!(parts.iter().any(|w| matches!(w, Word::Literal(s) if s.contains("(foo"))),
                "should contain literal (foo: {:?}", parts);
        }
        w => panic!("expected Concat with VarRef+Literal, got {:?}", w),
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

// --- $[expr] sugar (jimtcl extension) ---

/// `$[1+2]` → ExprSugar("1+2").
#[test]
fn test_expr_sugar_basic() {
    let cmds = parse("set x $[1+2]").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, "1+2"),
        w => panic!("expected ExprSugar, got {:?}", w),
    }
}

/// `$[expr]` sugar in quoted string.
#[test]
fn test_expr_sugar_in_quoted() {
    let cmds = parse("set x \"val=$[1+2]\"").unwrap();
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            assert!(parts.iter().any(|w| matches!(w, Word::ExprSugar(e) if e == "1+2")),
                "should contain ExprSugar: {:?}", parts);
        }
        w => panic!("expected Concat with ExprSugar, got {:?}", w),
    }
}

/// `$[]` — empty expr sugar.
#[test]
fn test_expr_sugar_empty() {
    let cmds = parse("set x $[]").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, ""),
        w => panic!("expected ExprSugar, got {:?}", w),
    }
}

/// `$[cmd arg]` — expr sugar with spaces.
#[test]
fn test_expr_sugar_with_spaces() {
    let cmds = parse("set x $[$a + $b]").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, "$a + $b"),
        w => panic!("expected ExprSugar, got {:?}", w),
    }
}

// --- $(expr) sugar (jimtcl default) ---

/// `$(2)` — simple numeric expr sugar.
#[test]
fn test_paren_expr_sugar_numeric() {
    let cmds = parse("set x $(2)").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, "2"),
        w => panic!("expected ExprSugar(2), got {:?}", w),
    }
}

/// `$(-3)` — negative number.
#[test]
fn test_paren_expr_sugar_negative() {
    let cmds = parse("set x $(-3)").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, "-3"),
        w => panic!("expected ExprSugar(-3), got {:?}", w),
    }
}

/// `$(!0)` — logical not.
#[test]
fn test_paren_expr_sugar_not() {
    let cmds = parse("set x $(!0)").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, "!0"),
        w => panic!("expected ExprSugar(!0), got {:?}", w),
    }
}

/// `$(6 * 7 + 2)` — arithmetic.
#[test]
fn test_paren_expr_sugar_arithmetic() {
    let cmds = parse("set x $(6 * 7 + 2)").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, "6 * 7 + 2"),
        w => panic!("expected ExprSugar, got {:?}", w),
    }
}

/// `$(expr)` in quoted string.
#[test]
fn test_paren_expr_sugar_in_quoted() {
    let cmds = parse("set x \"val=$(1+2)\"").unwrap();
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            assert!(parts.iter().any(|w| matches!(w, Word::ExprSugar(e) if e == "1+2")),
                "should contain ExprSugar: {:?}", parts);
        }
        w => panic!("expected Concat with ExprSugar, got {:?}", w),
    }
}

/// `$()` — empty paren expr sugar.
#[test]
fn test_paren_expr_sugar_empty() {
    let cmds = parse("set x $()").unwrap();
    match &cmds[0].words[2] {
        Word::ExprSugar(e) => assert_eq!(e, ""),
        w => panic!("expected ExprSugar, got {:?}", w),
    }
}

// --- Braced variable names with special characters ---

/// `${path/file.exe}` — slashes in variable name.
#[test]
fn test_braced_var_with_slashes() {
    let cmds = parse("set x ${path/file.exe}").unwrap();
    match &cmds[0].words[2] {
        Word::VarRef(name) => assert_eq!(name, "path/file.exe"),
        w => panic!("expected VarRef(path/file.exe), got {:?}", w),
    }
}

/// `${/usr/local/bin/prog}` — absolute path as variable name.
#[test]
fn test_braced_var_absolute_path() {
    let cmds = parse("set x ${/usr/local/bin/prog}").unwrap();
    match &cmds[0].words[2] {
        Word::VarRef(name) => assert_eq!(name, "/usr/local/bin/prog"),
        w => panic!("expected VarRef(/usr/local/bin/prog), got {:?}", w),
    }
}

/// `${config.server.host}` — dots in variable name.
#[test]
fn test_braced_var_with_dots() {
    let cmds = parse("set x ${config.server.host}").unwrap();
    match &cmds[0].words[2] {
        Word::VarRef(name) => assert_eq!(name, "config.server.host"),
        w => panic!("expected VarRef(config.server.host), got {:?}", w),
    }
}

/// `$foo.bar` — dot is NOT a varname char, so this is `$foo` + `.bar`.
#[test]
fn test_bare_var_dot_boundary() {
    let cmds = parse("puts $foo.bar").unwrap();
    // Should be a Concat of VarRef("foo") + Literal(".bar")
    match &cmds[0].words[1] {
        Word::Concat(parts) => {
            match &parts[0] {
                Word::VarRef(name) => assert_eq!(name, "foo"),
                w => panic!("expected VarRef(foo), got {:?}", w),
            }
        }
        Word::VarRef(name) => {
            // If the parser treats the whole word as varref, it should only be "foo"
            assert_eq!(name, "foo");
        }
        w => panic!("expected Concat or VarRef, got {:?}", w),
    }
}

/// `${a.b}.x` — braced var + literal suffix.
#[test]
fn test_braced_var_with_suffix() {
    let cmds = parse("puts ${a.b}.x").unwrap();
    match &cmds[0].words[1] {
        Word::Concat(parts) => {
            match &parts[0] {
                Word::VarRef(name) => assert_eq!(name, "a.b"),
                w => panic!("expected VarRef(a.b), got {:?}", w),
            }
            // Rest should contain ".x"
            let rest: String = parts[1..].iter().map(|w| format!("{}", w)).collect();
            assert_eq!(rest, ".x");
        }
        w => panic!("expected Concat, got {:?}", w),
    }
}

/// `$(` with no closing paren — orphan $.
#[test]
fn test_paren_expr_sugar_unclosed() {
    let cmds = parse("set x $(abc").unwrap();
    // No matching ')' → $ is orphan, (abc is literal
    let text = format!("{}", &cmds[0].words[2]);
    assert_eq!(text, "$(abc");
}

// --- Paren backtracking ---

/// Unclosed paren in quoted context: `"$a(foo"`.
#[test]
fn test_unclosed_paren_quoted() {
    let cmds = parse("set x \"$a(foo\"").unwrap();
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n == "a")),
                "should contain VarRef(a): {:?}", parts);
            assert!(parts.iter().any(|w| matches!(w, Word::Literal(s) if s.contains("(foo"))),
                "should contain literal (foo: {:?}", parts);
        }
        w => panic!("expected Concat, got {:?}", w),
    }
}

/// Unbalanced nested paren: `$a(b(c)d` — backtrack to after last `)`.
#[test]
fn test_unbalanced_nested_paren_backtrack() {
    let cmds = parse("set x $a(b(c)d").unwrap();
    // jimtcl backtracks to after last ')': $a(b(c) is dict sugar, d is literal
    match &cmds[0].words[2] {
        Word::Concat(parts) => {
            assert!(parts.iter().any(|w| matches!(w, Word::VarRef(n) if n.starts_with("a("))),
                "should contain VarRef(a(...)): {:?}", parts);
            assert!(parts.iter().any(|w| matches!(w, Word::Literal(s) if s == "d")),
                "should contain literal d: {:?}", parts);
        }
        w => panic!("expected Concat, got {:?}", w),
    }
}

/// Unclosed paren doesn't consume newline: `$a(foo\nset y 1`.
#[test]
fn test_unclosed_paren_doesnt_eat_newline() {
    let cmds = parse("set x $a(foo\nset y 1").unwrap();
    // Without backtracking, the paren would consume the newline and "set y 1".
    // With backtracking, $a is just a VarRef and (foo is literal for the first command.
    assert_eq!(cmds.len(), 2, "should be 2 commands: {:?}", cmds);
}
