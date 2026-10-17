//! 每 pass 的 golden 测试（设计文档 §7.3「每个 pass 配 golden 测试；
//! 输入 IR 文本 → 输出 IR 文本」）。输入固定 → 输出逐行稳定；改动
//! pass 时 golden 的 diff 即评审界面。cpcheck（check::）随管线默认开启
//! （pass::run 内），此处对校验器另作直接断言。

use rtcl_jit::check;
use rtcl_jit::ir::{L1, L2};
use rtcl_jit::pass::{
    alloc_locals, annotate_lane, escape, instrument, lower, plan_guards, EmitConfig,
};
use rtcl_parser::{ByteCode, Compiler, ScriptUnit};

fn compile_unit(src: &str) -> ByteCode {
    let unit = ScriptUnit::parse(src).expect("parse");
    Compiler::compile_unit(unit.source, &unit.commands)
}

fn to_l2(src: &str) -> (ByteCode, L2) {
    let bc = compile_unit(src);
    let l1 = annotate_lane::run(&bc);
    escape::run(&l1, &bc).expect("accepted");
    let l1 = plan_guards::run(l1);
    check::check_l1(&l1).expect("l1");
    let l2 = alloc_locals::run(l1, &bc).expect("l2");
    check::check_l2(&l2).expect("l2");
    (bc, l2)
}

#[test]
fn golden_l1_annotate_lane() {
    let bc = compile_unit("set s 0\nincr s 3");
    let l1 = annotate_lane::run(&bc);
    insta_like(format!("{}", l1), &[
        "L1 (5 ops):",
        "0000 BEGIN_CMD 0",
        "0001 PUSH_INT 0",
        "0002 STORE_VAR_POP 0",
        "0003 BEGIN_CMD 1",
        "0004 INCR_VAR 0 3",
    ]);
    // plan_guards 后守卫标记落地（数值 op 标 guarded）。
    let l1g = plan_guards::run(l1);
    let g = format!("{}", l1g);
    assert!(g.contains("PUSH_INT 0") && g.contains("out=imm"));
    assert!(!g.lines().any(|l| l.contains("guard=true")), "incr 不在 guardable 集: {g}");
}

/// 行级包含断言（比全文快照宽松一档、比形状断言严一档）。
fn insta_like(text: String, needles: &[&str]) {
    for n in needles {
        assert!(
            text.lines().any(|l| l.trim_start().starts_with(n.trim_start()) && l.contains(n.trim())),
            "golden 行缺失: {n:?}\n--- 实际:\n{text}"
        );
    }
}

#[test]
fn golden_l2_alloc_locals() {
    let (_bc, l2) = to_l2("set x 1\nputs $x");
    let text = format!("{}", l2);
    let expected = "L2: 7 ops, depth 2, 1 vars, 3 fails, backedges []
0000 BEGIN_CMD 0 r=[] w=None fail=None
0001 PUSH_INT 1 r=[] w=Some(0) fail=None
0002 STORE_VAR_POP 0 r=[0] w=None fail=Some(0)
0003 BEGIN_CMD 1 r=[] w=None fail=None
0004 PUSH_CONST 1 r=[] w=Some(0) fail=None
0005 LOAD_VAR 0 r=[] w=Some(1) fail=Some(1)
0006 CALL cmd=128 argc=2 r=[0, 1] w=Some(0) fail=Some(2)
vars:
  cache=0 x (const 0)
fail[0] pc=2 site=0 word=false bodies=[]
fail[1] pc=5 site=1 word=true bodies=[]
fail[2] pc=6 site=1 word=false bodies=[]
";
    assert_eq!(text, expected);
}

#[test]
fn golden_l3_lower_and_instrument() {
    let (bc, l2) = to_l2("set i 0\nwhile {$i < 3} { incr i }");
    let (l3, bregs) = lower::run(&l2, &bc).expect("lower");
    let l3 = instrument::run(l3, &EmitConfig::default());
    check::check_l3(&l3, l2.max_depth + 1, bregs).expect("l3");
    let text = format!("{}", l3);
    // 结构骨架断言（完整 golden 过长；关键形态逐条钉死）。
    assert!(text.contains("loop hdr="), "loop node:\n{text}");
    assert!(text.contains("BackedgeFuel"), "fuel weave:\n{text}");
    assert!(text.contains("EpochCheck { at_entry: false }"), "epoch at backedge:\n{text}");
    assert!(text.contains("Cmp { cc: Lt"), "cond fast lane:\n{text}");
    assert!(text.contains("IncrVar"), "incr:\n{text}");
    assert!(!text.contains("CallSite"), "本单元无 Call，不应出现 CallSite:\n{text}");
}

#[test]
fn golden_l3_for_continue_break() {
    let (bc, l2) = to_l2(
        "for {set i 0} {$i < 9} {incr i 2} {\n  if {$i == 4} { continue }\n  if {$i == 8} { break }\n}",
    );
    let (l3, bregs) = lower::run(&l2, &bc).expect("lower");
    check::check_l3(&l3, l2.max_depth + 1, bregs).expect("l3");
    let text = format!("{}", l3);
    assert!(text.contains("block cont="), "for 形 cont block:\n{text}");
    assert!(text.contains("br 0 (backedge)"), "回边:\n{text}");
}

#[test]
fn golden_l4_module_bytes_stable() {
    let bc = compile_unit("set x 1\nputs $x");
    let m = rtcl_jit::emit::compile(&bc, &EmitConfig::default()).expect("compile");
    // 头部 + 导入数量固定（15 imports + 3 globals）。
    assert_eq!(&m.bytes[..8], b"\0asm\x01\0\0\0");
    // 版本一致性：同输入两次发射字节相同（pass 纯函数性）。
    let m2 = rtcl_jit::emit::compile(&bc, &EmitConfig::default()).expect("compile2");
    assert_eq!(m.bytes, m2.bytes);
}

#[test]
fn cpcheck_rejects_malformed() {
    let bad = L2 {
        ops: vec![],
        reads: vec![],
        writes: vec![Some(9)],
        max_depth: 2,
        vars: vec![],
        fails: vec![],
        fail_of: vec![],
        used_consts: vec![],
        edges: vec![],
        backedges: vec![],
        const_base: 3,
    };
    assert!(check::check_l2(&bad).is_err());
    let empty = L1 { ops: vec![] };
    assert!(check::check_l1(&empty).is_ok());
}

#[test]
fn assemble_section_layout() {
    // assemble 的节区顺序/全局布局由 wasmi 校验器背书（exec 端到端测试
    // 实例化成功即证明）；此处钉住发射入口的确定性。
    let bc = compile_unit("set a 5\nset b [expr {$a * 7}]\nputs $b");
    let m1 = rtcl_jit::emit::compile(&bc, &EmitConfig::default()).unwrap();
    let bc2 = compile_unit("set a 5\nset b [expr {$a * 7}]\nputs $b");
    let m2 = rtcl_jit::emit::compile(&bc2, &EmitConfig::default()).unwrap();
    assert_eq!(m1.bytes, m2.bytes);
    assert!(m1.bytes.len() < 4096);
}
