//! `alloc_locals` — L1→L2：slot→wasm-local 分配决策（寄存器分配前置；
//! 实际寄存器分配委托引擎，§7.1 启示 3）。一次性静态模拟产出：
//! 每条 op 的读写槽（值栈槽 = 寄存器，路径无关——编译器栈纪律保证
//! 深度是 pc 的确定函数）、CFG 边与回边、变量缓存槽、宿主失败点表
//! （cur_site/bodies 在结构化 ByteCode 上路径不变，直接烘焙为常量表）。

use crate::abi;
use crate::emit::Unsupported;
use crate::ir::{Backedge, BodyKind, FailInfo, L1, L2, VarSlot};
use crate::pass::{classify, stack_effect, Flow};
use rtcl_ir::bytecode::ByteCode;
use rtcl_ir::opcode::OpCode;
use std::collections::HashMap;

/// 常量句柄基址：0=UNSET 永不分配，1=错误哨兵保留，2=空值，
/// 常量池 i → `CONST_BASE + i`（emit 与 exec 两侧同一公式）。
pub const CONST_BASE: u32 = 3;

fn body_kind(kind: u8) -> BodyKind {
    match kind {
        1 => BodyKind::Body,
        2 => BodyKind::Next,
        3 => BodyKind::Sub,
        _ => BodyKind::Expr,
    }
}

pub fn run(l1: L1, code: &ByteCode) -> Result<L2, Unsupported> {
    let n = l1.ops.len();
    let mut depth = 0u32;
    let mut max_depth = 0u32;
    let mut depth_at = vec![0u32; n];
    let mut reads: Vec<Vec<u32>> = Vec::with_capacity(n);
    let mut writes: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut edges: Vec<Vec<u32>> = Vec::with_capacity(n);
    let mut backedges: Vec<Backedge> = Vec::new();

    // 上下文模拟状态（与 vm_exec 的 bodies/cur_site 纪律逐条对齐）。
    let mut cur_site: u32 = u32::MAX;
    let mut bodies: Vec<(u8, u32)> = Vec::new();
    let mut loops: Vec<(u32, u32, usize, u32)> = Vec::new(); // (cont, brk, bodies_len, owner)
    let mut vars: Vec<VarSlot> = Vec::new();
    let mut var_of_const: HashMap<u32, u32> = HashMap::new();
    let mut fails: Vec<FailInfo> = Vec::new();
    let mut fail_of: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut used_consts: Vec<u32> = Vec::new();

    fn snap(
        pc: usize,
        cur_site: u32,
        bodies: &[(u8, u32)],
        word_op: bool,
        fails: &mut Vec<FailInfo>,
    ) -> u32 {
        let id = fails.len() as u32;
        fails.push(FailInfo {
            pc: pc as u32,
            site: cur_site,
            bodies: bodies.iter().map(|(k, s)| (body_kind(*k), *s)).collect(),
            word_op,
        });
        id
    }

    for (pc, a) in l1.ops.iter().enumerate() {
        let op = &a.op;
        depth_at[pc] = depth;
        let (pops, pushes) = stack_effect(op);
        let base = depth.saturating_sub(pops);

        // 读写槽。
        let (r, w): (Vec<u32>, Option<u32>) = match op {
            OpCode::Dup => (vec![depth - 1], Some(depth)),
            OpCode::StoreVar(_) => (vec![depth - 1], None),
            OpCode::Neg | OpCode::Not | OpCode::BitNot => (vec![depth - 1], Some(depth - 1)),
            OpCode::Call { .. } => ((base..depth).collect(), Some(base)),
            OpCode::JumpTrue(_) | OpCode::JumpFalse(_) | OpCode::Return => (vec![depth - 1], None),
            _ if pops == 0 && pushes == 1 => (vec![], Some(depth)),
            _ if pops == 2 && pushes == 1 => (vec![depth - 2, depth - 1], Some(depth - 2)),
            _ if pops == 1 && pushes == 0 => (vec![depth - 1], None),
            _ => (vec![], None),
        };

        // 变量槽与常量引用。
        let intern_var = |idx: u32,
                              vars: &mut Vec<VarSlot>,
                              var_of_const: &mut HashMap<u32, u32>|
         -> Result<(), Unsupported> {
            if var_of_const.contains_key(&idx) {
                return Ok(());
            }
            let name = code
                .get_const_wide(idx)
                .ok_or_else(|| Unsupported(format!("const {idx} missing")))?;
            if vars.len() >= abi::cache::MAX_VARS {
                return Err(Unsupported(format!("> {} distinct vars", abi::cache::MAX_VARS)));
            }
            let cache = vars.len() as u32;
            vars.push(VarSlot { const_idx: idx, name: name.to_string(), cache });
            var_of_const.insert(idx, cache);
            Ok(())
        };
        match op {
            OpCode::LoadVar(i) | OpCode::StoreVar(i) | OpCode::StoreVarPop(i)
            | OpCode::IncrVar(i, _) => intern_var(*i as u32, &mut vars, &mut var_of_const)?,
            OpCode::PushConst(i) => used_consts.push(*i as u32),
            OpCode::PushConstWide(i) => used_consts.push(*i),
            _ => {}
        }

        // 失败点（可失败的宿主交互）。
        let fail = match op {
            OpCode::LoadVar(_) => Some(snap(pc, cur_site, &bodies, true, &mut fails)),
            OpCode::StoreVar(_) | OpCode::StoreVarPop(_) | OpCode::IncrVar(_, _) => {
                Some(snap(pc, cur_site, &bodies, false, &mut fails))
            }
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
            | OpCode::StrEq
            | OpCode::StrNe
            | OpCode::JumpTrue(_)
            | OpCode::JumpFalse(_) => Some(snap(pc, cur_site, &bodies, false, &mut fails)),
            OpCode::Call { .. } => Some(snap(pc, cur_site, &bodies, false, &mut fails)),
            _ => None,
        };
        fail_of.push(fail);

        // 控制流边。
        let flow = match op {
            OpCode::Break => match loops.last() {
                Some(l) => Flow::Jump(l.1),
                None => Flow::Leave,
            },
            OpCode::Continue => match loops.last() {
                Some(l) => Flow::Jump(l.0),
                None => Flow::Leave,
            },
            other => classify(other).ok_or_else(|| Unsupported(other.to_string()))?,
        };
        let edge: Vec<u32> = match flow {
            Flow::Fall => {
                if pc + 1 < n {
                    vec![(pc + 1) as u32]
                } else {
                    vec![]
                }
            }
            Flow::Jump(t) | Flow::Cond(t, _) => {
                let mut e = vec![t];
                if matches!(flow, Flow::Cond(..)) && pc + 1 < n {
                    e.push((pc + 1) as u32);
                }
                e
            }
            Flow::Leave => vec![],
        };
        for &t in &edge {
            if t <= pc as u32 {
                // 回边 = fuel 计步点：失败上下文同本 op（追加表项）。
                let bf = snap(pc, cur_site, &bodies, false, &mut fails);
                backedges.push(Backedge { src: pc as u32, target: t, fail: bf });
            }
        }
        edges.push(edge);

        // 上下文推进。
        match op {
            OpCode::BeginCmd(k) => cur_site = *k,
            OpCode::BodyMark => bodies.push((1, cur_site)),
            OpCode::NextMark => bodies.push((2, cur_site)),
            OpCode::SubMark => bodies.push((3, cur_site)),
            OpCode::ExprMark => bodies.push((4, cur_site)),
            OpCode::SubEnd => {
                if let Some((_, s)) = bodies.pop() {
                    cur_site = s;
                }
            }
            OpCode::Jump(_) => {
                if let Some((_, s)) = bodies.pop() {
                    cur_site = s;
                }
            }
            OpCode::JumpTrue(_) | OpCode::JumpFalse(_) => bodies.push((1, cur_site)),
            OpCode::LoopEnter { cont, brk } => {
                loops.push((*cont, *brk, bodies.len(), cur_site));
            }
            OpCode::LoopExit => {
                loops.pop();
            }
            OpCode::Break | OpCode::Continue => {
                if let Some(l) = loops.last() {
                    let (_, _, blen, owner) = *l;
                    bodies.truncate(blen);
                    cur_site = owner;
                }
            }
            _ => {}
        }

        depth = base + pushes;
        max_depth = max_depth.max(depth);
        reads.push(r);
        writes.push(w);
    }

    // 栈纪律校验：线性模拟深度与边目标一致。两类编译器习语放宽：
    // (a) 目标更深（then 臂跳过 else 的 PUSH_EMPTY 直达 POP）——差值
    //     必须由目标处起连续的 POP 吸收（读到的旧槽即被丢弃）；
    // (b) Break/Continue 截断到循环入口深度（vm_exec 的 stack_len 纪律）。
    for (pc, e) in edges.iter().enumerate() {
        let after = depth_after(&l1.ops[pc].op, depth_at[pc]);
        let is_bc = matches!(l1.ops[pc].op, OpCode::Break | OpCode::Continue);
        for &t in e {
            let Some(tu) = (t as usize).checked_sub(0).filter(|x| *x < n) else {
                continue;
            };
            let target_d = depth_at[tu];
            if target_d == after || (is_bc && target_d < after) {
                continue;
            }
            if target_d > after {
                let mut absorb = target_d - after;
                let mut q = tu;
                while absorb > 0 && q < n {
                    if matches!(l1.ops[q].op, OpCode::Pop) {
                        absorb -= 1;
                        q += 1;
                    } else {
                        break;
                    }
                }
                if absorb == 0 {
                    continue;
                }
            }
            return Err(Unsupported(format!(
                "stack discipline broken at pc {pc} -> {t}: {target_d} != {after}"
            )));
        }
    }

    Ok(L2 {
        ops: l1.ops,
        reads,
        writes,
        max_depth,
        vars,
        fails,
        fail_of,
        used_consts,
        edges,
        backedges,
        const_base: CONST_BASE,
    })
}

fn depth_after(op: &OpCode, before: u32) -> u32 {
    let (pops, pushes) = stack_effect(op);
    before - pops.min(before) + pushes
}
