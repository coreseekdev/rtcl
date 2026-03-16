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
