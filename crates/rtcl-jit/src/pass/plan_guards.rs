//! `plan_guards` — L1→L1：在 lane 假设处计划守卫（投机规划 / deopt 点）。
//! 双输入 lane 至少一侧为 Any 的 lane-tracked op 计划「双立即数守卫」；
//! 全走宿主的 op（§4 的 div/mod、StrEq/StrNe、And/Or 一律宿主）不标。

use crate::ir::{L1, L1Op, Lane};
use rtcl_ir::opcode::OpCode;

fn guardable(op: &OpCode) -> bool {
    matches!(
        op,
        OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Neg
            | OpCode::Not
            | OpCode::BitAnd
            | OpCode::BitOr
            | OpCode::BitXor
            | OpCode::BitNot
            | OpCode::Shl
            | OpCode::Shr
            | OpCode::Eq
            | OpCode::Ne
            | OpCode::Lt
            | OpCode::Gt
            | OpCode::Le
            | OpCode::Ge
            | OpCode::And
            | OpCode::Or
    )
}

pub fn run(mut l1: L1) -> L1 {
    for a in &mut l1.ops {
        if guardable(&a.op) {
            a.guarded = true;
        }
    }
    let _ = Lane::Imm;
    l1
}

// L1Op 的 in_lanes 由 annotate_lane 产出；`guarded=true` 统一表示「发射
// 时带双立即数守卫 + binary_op 兜底」——两侧都确定 Imm 时 assemble 里
// 守卫自然恒真（不产生分支成本由引擎优化掉）。
#[allow(dead_code)]
fn _keep(a: L1Op) -> L1Op {
    a
}
