use super::*;

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
