#[test]
fn probe_v2_body() {
    use rtcl_vm::interp::Interp;
    use rtcl_vm::interp::commands::dynmod::{DynModuleDef, DynAbi};
    let mut interp = Interp::new();
    interp.register_dyn_module_v2("binmod", DynModuleDef {
        commands: vec![("echo".into(), vec!["v2_bridge".into(), "binmod".into(), "echo".into()])],
        abi: DynAbi::V2List,
    });
    interp.eval("load binmod").unwrap();
    println!("body=[{}]", interp.eval("info body ::binmod::echo").unwrap());
    println!("args-linfo=[{}]", interp.eval("proc probe2 {args} { return [llength $args] }; probe2 x y").unwrap());
}
