//! `escape` — L1 上做编译拒绝筛选（「编译拒绝」粒度，设计文档 §2）：
//! M1' 接受集之外的 op、Call 嵌在 Sub/Expr region 内（错误协议需要
//! 宿主侧累积器二分，见 exec.rs）、栈/表超界，一律整单元回解释器。

use crate::emit::Unsupported;
use crate::ir::L1;
use crate::pass::classify;
use rtcl_ir::bytecode::ByteCode;
use rtcl_ir::opcode::OpCode;

/// M1' 上限（wasm locals/内存页预算）。
pub const MAX_OPS: usize = 4_096;
pub const MAX_DEPTH: u32 = 128;
pub const MAX_BODIES: usize = 16;

pub fn run(l1: &L1, code: &ByteCode) -> Result<(), Unsupported> {
    if code.fallback {
        return Err(Unsupported("unit has fallback=true".into()));
    }
    // 数组元素引用（名字含 '('）：eval_var_ref 需做下标替换/解析，
    // read_var 公共入口不等价——编译拒绝（M2 经 LoadArrayElem 接管）。
    for (pc, a) in l1.ops.iter().enumerate() {
        let name_idx = match &a.op {
            OpCode::LoadVar(i) | OpCode::StoreVar(i) | OpCode::StoreVarPop(i)
            | OpCode::IncrVar(i, _) => Some(*i as u32),
            _ => None,
        };
        if let Some(idx) = name_idx {
            if let Some(name) = code.get_const_wide(idx) {
                if name.contains('(') {
                    return Err(Unsupported(format!(
                        "array-element var ref {name:?} at pc {pc}"
                    )));
                }
            }
        }
    }
    // i63 域外立即数：tagging 会回绕（ByteCode 可携带全 i64 的
    // PushInt/IncrVar 量）——编译拒绝，留解释器的 bignum 路径。
    let imm_ok = |n: i64| n >= -(1i64 << 62) && n < (1i64 << 62);
    for (pc, a) in l1.ops.iter().enumerate() {
        match &a.op {
            OpCode::PushInt(n) if !imm_ok(*n) => {
                return Err(Unsupported(format!(
                    "PushInt {n} outside i63 at pc {pc} (bignum)"
                )));
            }
            OpCode::IncrVar(_, n) if !imm_ok(*n) => {
                return Err(Unsupported(format!(
                    "IncrVar amount {n} outside i63 at pc {pc}"
                )));
            }
            _ => {}
        }
    }
    if l1.ops.len() > MAX_OPS {
        return Err(Unsupported(format!("{} ops > {}", l1.ops.len(), MAX_OPS)));
    }
    // 栈深模拟 + region 上下文（Call 不得在 Sub/Expr 内）。
    let mut depth: u32 = 0;
    let mut bodies: Vec<u8> = Vec::new();
    let mut loop_depth: u32 = 0;
    let mut loops: Vec<(u32, u32)> = Vec::new(); // (cont, brk)
    for (pc, a) in l1.ops.iter().enumerate() {
        let op = &a.op;
        classify(op).ok_or_else(|| Unsupported(op.to_string()))?;
        if matches!(op, OpCode::Call { .. }) && bodies.last().is_some_and(|k| *k >= 3) {
            // bodies 编码: 1=Body 2=Next 3=Sub 4=Expr（Sub/Expr ≥3）。
            return Err(Unsupported(format!(
                "Call inside Sub/Expr region at pc {pc} (M1': host-eval error \
                 accumulator split)"
            )));
        }
        match op {
            OpCode::BodyMark => bodies.push(1),
            OpCode::NextMark => bodies.push(2),
            OpCode::SubMark => bodies.push(3),
            OpCode::ExprMark => bodies.push(4),
            OpCode::SubEnd => {
                bodies.pop();
            }
            OpCode::Jump(_) => {
                // 平衡纪律: 每个普通 Jump 关闭一个内联 body region。
                bodies.pop();
            }
            OpCode::JumpTrue(_) | OpCode::JumpFalse(_) => {
                // not-taken 进入 body region——模拟保守记 push（深度上限
                // 用最大可能值）。
                if bodies.len() + 1 > MAX_BODIES {
                    return Err(Unsupported(format!("bodies depth > {MAX_BODIES} at {pc}")));
                }
                bodies.push(1);
            }
            OpCode::LoopEnter { cont, brk } => {
                loop_depth += 1;
                if loop_depth > 16 {
                    return Err(Unsupported("loop nesting > 16".into()));
                }
                loops.push((*cont, *brk));
            }
            OpCode::LoopExit => {
                loop_depth = loop_depth.saturating_sub(1);
                loops.pop();
            }
            OpCode::Break | OpCode::Continue => {
                let Some(&(cont, brk)) = loops.last() else {
                    continue; // 无循环 → 运行时错误路径（Leave 语义）
                };
                let target = if matches!(op, OpCode::Break) { brk } else { cont };
                if target <= pc as u32 {
                    // 回边（while-continue / 向后 break 不存在于本编译器，
                    // 但保守接受目标合法即可）。
                }
            }
            _ => {}
        }
        let (pops, pushes) = crate::pass::stack_effect(op);
        depth = depth - pops.min(depth) + pushes;
        if depth > MAX_DEPTH {
            return Err(Unsupported(format!("stack depth > {MAX_DEPTH} at pc {pc}")));
        }
    }
    Ok(())
}
