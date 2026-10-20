use rtcl_parser::{Compiler, ScriptUnit};
use std::rc::Rc;
fn main() {
    let body = "\tlmap a {1 2 3} b {4 5 6} {\n\t    list $b $a\n\t}\n    ";
    let unit = ScriptUnit::parse(body).unwrap();
    let bc = Compiler::compile_unit_locals(Rc::clone(&unit.source), &unit.commands, &[]).unfused();
    let l1 = rtcl_jit::pass::annotate_lane::run(&bc);
    let mut l1 = l1;
    rtcl_jit::pass::alloc_locals::rewrite_slots(&mut l1, &bc);
    let l2 = rtcl_jit::pass::alloc_locals::run(l1, &bc).unwrap();
    println!("ops:");
    for (i, a) in l2.ops.iter().enumerate() {
        println!("{:03} {:<26} r={:?} w={:?}", i, a.op, l2.reads[i], l2.writes[i]);
    }
}
