use rtcl_parser::{Compiler, ScriptUnit};
use std::rc::Rc;
fn main() {
    let _ = &std::env::args().nth(1);
    let body = "\n\trename crash {}\n\tset a \"x y z\"\n\tset b \"\"\n\tforeach aa $a bb $b { set x \"aa = $aa bb = $bb\" }\n".to_string();

    let binding = std::env::args().nth(2).unwrap_or_default();
    let params: Vec<&str> = binding.split(',').collect();
    let body2 = body.clone();
    let unit = ScriptUnit::parse(&body2).unwrap();
    let bc = Compiler::compile_unit_locals(Rc::clone(&unit.source), &unit.commands, &params).unfused();
    let l1 = rtcl_jit::pass::annotate_lane::run(&bc);
    let mut l1 = l1;
    rtcl_jit::pass::alloc_locals::rewrite_slots(&mut l1, &bc);
    let l2 = match rtcl_jit::pass::alloc_locals::run(l1, &bc) {
        Ok(l2) => l2,
        Err(e) => {
            println!("REJECT: {e}");
            return;
        }
    };
    match rtcl_jit::emit::compile(&bc, &Default::default()) {
        Ok(_) => println!("ACCEPT"),
        Err(e) => println!("REJECT-at-emit: {e}"),
    }
}
