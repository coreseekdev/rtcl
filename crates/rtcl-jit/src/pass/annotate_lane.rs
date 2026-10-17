//! `annotate_lane` — L0→L1：lane 标注（常量类型传播）。
//! 抽象值栈上传播「确定是 tagged 立即数」的信息；`plan_guards` 据此
//! 决定哪些 op 可以省守卫。

use crate::ir::{L1, L1Op, Lane};
use rtcl_ir::bytecode::ByteCode;
use rtcl_ir::opcode::OpCode;
use std::collections::HashMap;

/// 二元/一元运算是否作用于 lane 假设（数值/逻辑域）。
fn lane_tracked(op: &OpCode) -> bool {
    matches!(
        op,
        OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Div
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Neg
            | OpCode::Not
            | OpCode::And
            | OpCode::Or
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
    )
}

pub fn run(code: &ByteCode) -> L1 {
    // 跳转目标的到达深度：值栈深度是 pc 的确定函数（编译器纪律），
    // 线性模拟即可；跳转不改深度。
    let n = code.ops().len();
    let mut depth_at = vec![0u32; n + 1];
    let mut stack: Vec<Lane> = Vec::new();
    let mut join_lane: HashMap<u32, Lane> = HashMap::new();

    let mut ops = Vec::with_capacity(n);
    for (pc, op) in code.ops().iter().enumerate() {
        depth_at[pc] = stack.len() as u32;
        let (pops, pushes) = crate::pass::stack_effect(op);
        let pops = pops as usize;
        let in_lanes = [
            stack.get(stack.len().saturating_sub(pops)).copied().unwrap_or(Lane::Any),
            stack.get(stack.len().saturating_sub(pops).saturating_add(1)).copied()
                .unwrap_or(Lane::Any),
        ];
        for _ in 0..pops {
            stack.pop();
        }
        // 常量产出 lane。
        let out_lane = match op {
            OpCode::PushInt(_) | OpCode::PushTrue | OpCode::PushFalse => Lane::Imm,
            OpCode::PushFloat(_) | OpCode::PushEmpty | OpCode::PushConst(_)
            | OpCode::PushConstWide(_) => Lane::Any,
            other if lane_tracked(other) => {
                if in_lanes == [Lane::Imm, Lane::Imm] {
                    // 双立即数输入的算术仍可能走宿主（溢出宽化），结果
                    // 保守记 Any——守卫省不掉，除非 plan_guards 常量折叠。
                    Lane::Any
                } else {
                    Lane::Any
                }
            }
            _ => Lane::Any,
        };
        for _ in 0..pushes {
            stack.push(out_lane);
        }
        // 条件跳转目标处的栈 = 弹掉 cond 后。
        match op {
            OpCode::Jump(t) => {
                join_lane.entry(*t).or_insert_with(|| Lane::Any);
            }
            OpCode::JumpTrue(t) | OpCode::JumpFalse(t) => {
                join_lane.entry(*t).or_insert_with(|| Lane::Any);
            }
            _ => {}
        }
        ops.push(L1Op { op: op.clone(), in_lanes, out_lane, guarded: false });
    }
    depth_at[n] = stack.len() as u32;
    let _ = (join_lane, depth_at);
    L1 { ops }
}
