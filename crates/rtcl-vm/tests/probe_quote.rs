#[test]
fn probe_quote_u1() {
    let s = format!("a\u{0001}b");
    let q = rtcl_vm::value::tcl_quote(&s);
    println!("quote: {:?}", q);
    let list = rtcl_vm::Value::from_list(&[rtcl_vm::Value::from_str(&s), rtcl_vm::Value::from_str("普通文本")]);
    println!("list str: {:?}", list.to_string());
}
