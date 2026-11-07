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

#[test]
fn incremental_reader_runs_prefix_before_later_syntax_error() {
    let src = "set a 1\nputs $a\nset b {";
    let mut reader = IncrementalScript::new(src);
    let c1 = reader.next_command().unwrap().unwrap();
    assert_eq!(c1.line, 1);
    assert_eq!(c1.text.slice(src), "set a 1");
    let c2 = reader.next_command().unwrap().unwrap();
    assert_eq!(c2.line, 2);
    assert_eq!(c2.text.slice(src), "puts $a");
    let err = reader.next_command().unwrap_err();
    assert_eq!(err.line, 3);
    assert!(err.message.contains("close-brace"));
}
