#[test]
fn probe_u1_list_parse() {
    let s = "a\u{1}b 普通文本";
    let v = rtcl_vm::Value::from_str(s);
    let items = v.as_list_strict().unwrap();
    println!("parse n={} items={:?}", items.len(), items.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    println!("chars of first: {:?}", items.first().map(|x| x.to_string().chars().collect::<Vec<_>>()));
}
