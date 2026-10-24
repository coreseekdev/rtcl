//! Nanopass 管线（设计文档 §7.3）。每个 pass = 纯函数、单向变换、
//! 独立 golden 测试；层间过 `check::` 校验器（cpcheck 模式）。
//!
//! ```text
//! L0 ─annotate_lane→ L1 ─escape→ L1 ─plan_guards→ L1
//!    ─alloc_locals→ L2 ─lower→ L3 ─instrument→ L3 ─assemble→ L4
//! ```

pub mod alloc_locals;
pub mod annotate_lane;
pub mod assemble;
pub mod escape;
pub mod instrument;
pub mod lower;
pub mod plan_guards;

use crate::abi;
use crate::check;
use crate::emit::Unsupported;
use crate::ir::{L2, L3};
use rtcl_ir::bytecode::ByteCode;

/// 发射配置（`jit-float-lane` 等 ABI v2 开关的载体，§8）。
#[derive(Debug, Clone, PartialEq)]
pub struct EmitConfig {
    /// f64 快车道（`jit-float-lane`，默认 off，属 C-float 里程碑）。
    /// M1' 恒 false；字段先落地让开关矩阵可接线。
    pub float_lane: bool,
    /// fuel 充值块大小（backedge 计步粒度，见 `abi::REFUEL_CHUNK`）。
    pub refuel_chunk: i64,
}

impl Default for EmitConfig {
    fn default() -> Self {
        EmitConfig { float_lane: false, refuel_chunk: abi::REFUEL_CHUNK }
    }
}

/// 一条 op 的栈效应与控制流分类（pass 间共享的纯数据视图）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// 顺序落到下一 op。
    Fall,
    /// 无条件转移到 `u32`。
    Jump(u32),
    /// 条件转移：`(target, taken_when_true)`——JumpTrue=t/t，JumpFalse=f。
    Cond(u32, bool),
    /// 离开单元（Return/Exit/无循环 Break/Continue）。
    Leave,
}

/// M1' 接受集内的 op 分类；`None` = 编译拒绝（整个单元回解释器）。
pub fn classify(op: &rtcl_ir::opcode::OpCode) -> Option<Flow> {
    use rtcl_ir::opcode::OpCode as O;
    Some(match op {
        O::Nop | O::Line(_) | O::BeginCmd(_) | O::BodyMark | O::NextMark | O::SubMark
        | O::ExprMark | O::SubEnd | O::LoopEnter { .. } | O::LoopExit => Flow::Fall,
        O::PushInt(_) | O::PushTrue | O::PushFalse | O::PushEmpty | O::PushFloat(_)
        | O::PushConst(_) | O::PushConstWide(_) | O::Pop | O::Dup | O::LoadVar(_)
        | O::StoreVar(_) | O::StoreVarPop(_) | O::IncrVar(_, _) | O::Add | O::Sub
        | O::Mul | O::Div | O::Mod | O::Pow | O::Neg | O::Not | O::And | O::Or
        | O::BitAnd | O::BitOr | O::BitXor | O::BitNot | O::Shl | O::Shr | O::Eq
        | O::Ne | O::Lt | O::Gt | O::Le | O::Ge | O::StrEq | O::StrNe
        | O::Call { .. } | O::DynCall { .. } | O::Concat(_) | O::EvalScript
        | O::EvalExpr | O::LappendVar(_)
        | O::ForeachStart { .. } | O::ForeachNext { .. } | O::ForeachCollect
        | O::ForeachEnd | O::BodyMarkFramed => Flow::Fall,
        O::Jump(t) => Flow::Jump(*t),
        O::JumpTrue(t) => Flow::Cond(*t, true),
        O::JumpFalse(t) => Flow::Cond(*t, false),
        O::Return | O::Exit(_) => Flow::Leave,
        O::Break | O::Continue => {
            // 目标由 alloc_locals 的循环栈解析（LoopEnter 的 brk/cont）；
            // 无循环时 = Leave。classify 只给类别。
            Flow::Leave
        }
        _ => return None,
    })
}

/// op 的值栈效应：`(pops, pushes)`。
pub fn stack_effect(op: &rtcl_ir::opcode::OpCode) -> (u32, u32) {
    use rtcl_ir::opcode::OpCode as O;
    match op {
        O::PushInt(_) | O::PushTrue | O::PushFalse | O::PushEmpty | O::PushFloat(_)
        | O::PushConst(_) | O::PushConstWide(_) | O::LoadVar(_) | O::IncrVar(_, _) => (0, 1),
        O::Dup => (1, 2),
        O::Pop | O::JumpTrue(_) | O::JumpFalse(_) | O::Return => (1, 0),
        O::StoreVar(_) => (1, 1),
        O::StoreVarPop(_) => (1, 0),
        O::Add | O::Sub | O::Mul | O::Div | O::Mod | O::Pow | O::And | O::Or
        | O::BitAnd | O::BitOr | O::BitXor | O::Shl | O::Shr | O::Eq | O::Ne | O::Lt
        | O::Gt | O::Le | O::Ge | O::StrEq | O::StrNe => (2, 1),
        O::Neg | O::Not | O::BitNot => (1, 1),
        O::Call { argc, .. } | O::DynCall { argc } => (*argc as u32, 1),
        O::Concat(n) => (*n as u32, 1),
        O::EvalScript | O::EvalExpr => (1, 1),
        O::ForeachCollect => (1, 0),
        O::LappendVar(_) => (1, 1),
        _ => (0, 0),
    }
}

/// 全管线：L0 → L4 字节。层间校验默认开启。
/// RTCL_JIT_PASS_TIMING=1 时打印每 pass 耗时（诊断大单元的规模病）。
pub fn run(code: &ByteCode, cfg: &EmitConfig) -> Result<(Vec<u8>, L2, L3, Option<crate::emit::FastMeta>), Unsupported> {
    // Instant::now 在 wasm32 上 panic——只在启用时取时。
    let timing = crate::env_enabled("RTCL_JIT_PASS_TIMING");
    let mut t0 = timing.then(std::time::Instant::now);
    macro_rules! mark {
        ($name:expr) => {
            if let (true, Some(t)) = (timing, t0.take()) {
                t0 = Some(std::time::Instant::now());
                eprintln!("[pass] {}: {:?}", $name, t.elapsed());
            }
        };
    }
    let mut l1 = annotate_lane::run(code);
    mark!("annotate_lane");
    alloc_locals::rewrite_slots(&mut l1, code);
    mark!("rewrite_slots");
    escape::run(&l1, code)?;
    mark!("escape");
    let l1 = plan_guards::run(l1);
    mark!("plan_guards");
    check::check_l1(&l1).map_err(|e| Unsupported(e.to_string()))?;
    mark!("check_l1");
    let l2 = alloc_locals::run(l1, code)?;
    mark!("alloc_locals");
    check::check_l2(&l2).map_err(|e| Unsupported(e.to_string()))?;
    mark!("check_l2");
    let (mut l3, bregs) = lower::run(&l2, code)?;
    mark!("lower");
    l3 = instrument::run(l3, cfg);
    mark!("instrument");
    check::check_l3(&l3, l2.max_depth + 1, bregs)
        .map_err(|e| Unsupported(e.to_string()))?;
    mark!("check_l3");
    let (bytes, fast) = assemble::run(&l3, &l2, code, cfg);
    mark!("assemble");
    let _ = t0;
    Ok((bytes, l2, l3, fast))
}
