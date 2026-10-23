//! `alloc_locals` — L1→L2：slot→wasm-local 分配决策（寄存器分配前置；
//! 实际寄存器分配委托引擎，§7.1 启示 3）。一次性静态模拟产出：
//! 每条 op 的读写槽（值栈槽 = 寄存器，路径无关——编译器栈纪律保证
//! 深度是 pc 的确定函数）、CFG 边与回边、变量缓存槽、宿主失败点表
//! （cur_site/bodies 在结构化 ByteCode 上路径不变，直接烘焙为常量表）。

use crate::abi;
use crate::emit::Unsupported;
use crate::ir::{Backedge, BodyKind, FailInfo, FailKind, L1, L2, VarSlot};
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
        4 => BodyKind::Expr,
        5 => BodyKind::BodyFramed(false),
        _ => BodyKind::BodyFramed(true),
    }
}

/// slot 系 op → 名键重写（proc 体 JIT 的关键开放）。须在 escape 之前
/// 调用（escape 的拒绝名单不含 slot 系的模板出口）。
pub fn rewrite_slots(l1: &mut L1, code: &ByteCode) {
    // ── slot 系 op → 名键重写 ───────────────────────────────────────
    // slot 系（LoadLocal/StoreLocal/IncrLocal/LappendLocal/AddSlotSlot/
    // AddSlotImm/CmpSlotSlot/CmpSlotImm）此前 classify 通过但 lower 无
    // 模板——静默无操作（proc_scope 的 counter incr c 丢失 → 0/1）。
    // 语义等价：slot 名 = code.locals()[slot]（非退化帧下 slot 表与
    // 名字路径别名同一存储——宿主 read_var/set_var/incr_var 路由）。
    // 名字句柄 = 伪常量槽（consts.len()+slot，执行器 arena 预驻留
    // 追加 locals 名——同公式）。融合超指令展开为规范窗口。
    {
        let base = code.constants().len() as u32;
        let pseudo = |slot: u16| base + slot as u32;
        let mut rewritten: Vec<OpCode> = Vec::with_capacity(l1.ops.len() * 2);
        // 多 op 展开（Cmp/AddSlot*）让后续 op 的 pc 平移——**所有**
        // 跳转/迭代目标必须按 old→new 映射重编，否则目标错位
        //（fib 的 `if {$n<2}`：CmpSlotImm 展开 +2 后 JumpFalse(7)
        // 指向了 Return——栈纪律检查器拦下整单元拒绝，proc 体静默
        // 回落解释器，每次调用重编译——fib .13× 的真正根因）。
        let mut map: Vec<u32> = Vec::with_capacity(l1.ops.len());
        for l1op in l1.ops.drain(..) {
            map.push(rewritten.len() as u32);
            let op = l1op.op;
            match op {
                OpCode::LoadLocal(s) => rewritten.push(OpCode::LoadVar(pseudo(s) as u16)),
                OpCode::StoreLocal(s) => rewritten.push(OpCode::StoreVar(pseudo(s) as u16)),
                OpCode::IncrLocal(s, n) => rewritten.push(OpCode::IncrVar(pseudo(s) as u16, n)),
                OpCode::LappendLocal(s) => rewritten.push(OpCode::LappendVar(pseudo(s) as u16)),
                OpCode::AddSlotSlot(a, b) => {
                    rewritten.push(OpCode::LoadVar(pseudo(a) as u16));
                    rewritten.push(OpCode::LoadVar(pseudo(b) as u16));
                    rewritten.push(OpCode::Add);
                }
                OpCode::AddSlotImm(a, k) => {
                    rewritten.push(OpCode::LoadVar(pseudo(a) as u16));
                    rewritten.push(OpCode::PushInt(k));
                    rewritten.push(OpCode::Add);
                }
                OpCode::CmpSlotSlot(a, b, cc) => {
                    rewritten.push(OpCode::LoadVar(pseudo(a) as u16));
                    rewritten.push(OpCode::LoadVar(pseudo(b) as u16));
                    rewritten.push(match cc {
                        0 => OpCode::Lt,
                        1 => OpCode::Gt,
                        2 => OpCode::Le,
                        3 => OpCode::Ge,
                        4 => OpCode::Eq,
                        _ => OpCode::Ne,
                    });
                }
                OpCode::CmpSlotImm(a, k, cc) => {
                    rewritten.push(OpCode::LoadVar(pseudo(a) as u16));
                    rewritten.push(OpCode::PushInt(k));
                    rewritten.push(match cc {
                        0 => OpCode::Lt,
                        1 => OpCode::Gt,
                        2 => OpCode::Le,
                        3 => OpCode::Ge,
                        4 => OpCode::Eq,
                        _ => OpCode::Ne,
                    });
                }
                other => rewritten.push(other),
            }
            // lane 字段随 op 丢弃——重写后的规范窗口由 plan_guards 的
            // guardable 集覆盖（annotate_lane 的 lane 信息对 slot 重写
            // 产物不敏感：数值 op 语义同 M1' 路径）。
        }
        // pc 目标重映射（映射表按 old op 边界建——目标都指向 op 起点）。
        let at = |map: &[u32], t: &mut u32| {
            if let Some(n) = map.get(*t as usize) {
                *t = *n;
            }
        };
        for op in rewritten.iter_mut() {
            match op {
                OpCode::Jump(t) | OpCode::JumpTrue(t) | OpCode::JumpFalse(t) => at(&map, t),
                OpCode::LoopEnter { cont, brk } => {
                    at(&map, cont);
                    at(&map, brk);
                }
                OpCode::ForeachStart { next, end, .. } => {
                    at(&map, next);
                    at(&map, end);
                }
                OpCode::ForeachNext { body, .. } => at(&map, body),
                OpCode::CatchStart(t) => at(&map, t),
                OpCode::CmpSlotSlotJumpTrue(_, _, _, t)
                | OpCode::CmpSlotSlotJumpFalse(_, _, _, t)
                | OpCode::CmpSlotImmJumpTrue(_, _, _, t)
                | OpCode::CmpSlotImmJumpFalse(_, _, _, t)
                | OpCode::CmpVarVarJumpTrue(_, _, _, t)
                | OpCode::CmpVarVarJumpFalse(_, _, _, t)
                | OpCode::CmpVarImmJumpTrue(_, _, _, t)
                | OpCode::CmpVarImmJumpFalse(_, _, _, t) => at(&map, t),
                OpCode::ForNextIncrLocal(_, _, _, t) | OpCode::ForNextIncrVar(_, _, _, t) => {
                    at(&map, t)
                }
                OpCode::ForeachNextClose(_, b) => at(&map, b),
                _ => {}
            }
        }
        l1.ops = rewritten.into_iter().map(|op| crate::ir::L1Op {
            op,
            in_lanes: [crate::ir::Lane::Any, crate::ir::Lane::Any],
            out_lane: crate::ir::Lane::Any,
            guarded: false,
        }).collect();
    }
}

pub fn run(l1: L1, code: &ByteCode) -> Result<L2, Unsupported> {
    let n = l1.ops.len();
    let mut depth = 0u32;
    let mut max_depth = 0u32;
    let mut depth_at = vec![0u32; n];
    let mut after_at = vec![0u32; n];
    let mut reads: Vec<Vec<u32>> = Vec::with_capacity(n);
    let mut writes: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut edges: Vec<Vec<u32>> = Vec::with_capacity(n);
    let mut backedges: Vec<Backedge> = Vec::new();
    let mut pending_backedges: Vec<(u32, u32)> = Vec::new();

    // 上下文模拟状态（与 vm_exec 的 bodies/cur_site 纪律逐条对齐）。
    let mut cur_site: u32 = u32::MAX;
    let mut bodies: Vec<(u8, u32)> = Vec::new();
    let mut loops: Vec<(u32, u32, usize, u32)> = Vec::new(); // (cont, brk, bodies_len, owner)
    let mut foreaches: Vec<(u32, bool)> = Vec::new(); // (info_idx, lmap)
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
        kind: FailKind,
        fails: &mut Vec<FailInfo>,
    ) -> u32 {
        let id = fails.len() as u32;
        fails.push(FailInfo {
            pc: pc as u32,
            site: cur_site,
            bodies: bodies.iter().map(|(k, s)| (body_kind(*k), *s)).collect(),
            word_op,
            kind,
        });
        id
    }

    for (pc, a) in l1.ops.iter().enumerate() {
        let op = &a.op;
        depth_at[pc] = depth;
        // `for` next 脚本里的**分派命令**（for-8.2/8.8/8.10 的
        // `incr i; eval continue`——eval 编译为 Call，非 EvalScript）：
        // 分派返回的 break/continue 按 vm_exec loop_signal 须**逃逸**
        // 本循环（unwind + 交外层/边界转错）——wasm 的 CallSite kind
        // 分派只能内跳最近循环的 cont/brk 标签，静态结构表达不了
        // 「跳出一个循环再信号外层」。kind-5 逃逸通道是后续工程；
        // 此前拒绝（回落解释器 = 语义正确；与 pc 重映射修复前这些
        // 单元被整体拒绝的行为一致）。op 级的 incr/set/lappend
        // （IncrVar/StoreVar/LappendVar）不受影响。
        if matches!(op, OpCode::Call { .. } | OpCode::DynCall { .. })
            && bodies.iter().any(|(k, _)| *k == 2)
        {
            return Err(Unsupported("dispatched command inside for-next script".to_string()));
        }
        let (pops, pushes) = match op {
            // ngroups 由 info 表决定（stack_effect 无 ByteCode 访问）。
            OpCode::ForeachStart { info, .. } => (
                code.foreach_info(*info).map(|f| f.groups.len() as u32).unwrap_or(0),
                0,
            ),
            OpCode::ForeachEnd => (0, 1),
            other => stack_effect(other),
        };
        let base = depth.saturating_sub(pops);

        // 读写槽。
        let (r, w): (Vec<u32>, Option<u32>) = match op {
            OpCode::Dup => (vec![depth - 1], Some(depth)),
            OpCode::StoreVar(_) => (vec![depth - 1], None),
            OpCode::LappendVar(_) => (vec![depth - 1], Some(depth - 1)),
            OpCode::Neg | OpCode::Not | OpCode::BitNot => (vec![depth - 1], Some(depth - 1)),
            OpCode::EvalScript | OpCode::EvalExpr => (vec![depth - 1], Some(depth - 1)),
            OpCode::Call { .. } | OpCode::DynCall { .. } => {
                ((base..depth).collect(), Some(base))
            }
            OpCode::Concat(_) => ((base..depth).collect(), Some(base)),
            OpCode::ForeachStart { .. } => ((base..depth).collect(), None),
            OpCode::ForeachEnd => (vec![], Some(depth)),
            OpCode::JumpTrue(_) | OpCode::JumpFalse(_) | OpCode::Return => (vec![depth - 1], None),
            _ if pops == 0 && pushes == 1 => (vec![], Some(depth)),
            // break/continue 之后的死代码：线性深度下溢——读集置空
            //（op 不可达，寄存器引用无意义）。
            _ if pops > depth => (vec![], None),
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
            // 伪常量槽（>= consts.len()）：slot 重写产出的 locals 名，
            // 执行器 arena 预驻留时按序追加（同一公式）。
            let name = if (idx as usize) >= code.constants().len() {
                let slot = idx - code.constants().len() as u32;
                code.locals()
                    .get(slot as usize)
                    .cloned()
                    .ok_or_else(|| Unsupported(format!("local {slot} missing")))?
            } else {
                code.get_const_wide(idx)
                    .ok_or_else(|| Unsupported(format!("const {idx} missing")))?
                    .to_string()
            };
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
            | OpCode::IncrVar(i, _) | OpCode::LappendVar(i) => {
                intern_var(*i as u32, &mut vars, &mut var_of_const)?
            }
            OpCode::PushConst(i) => used_consts.push(*i as u32),
            OpCode::PushConstWide(i) => used_consts.push(*i),
            _ => {}
        }

        // 失败点（可失败的宿主交互）。
        let fail = match op {
            OpCode::LoadVar(_) => Some(snap(pc, cur_site, &bodies, true, FailKind::Cold, &mut fails)),
            OpCode::StoreVar(_) | OpCode::StoreVarPop(_) | OpCode::IncrVar(_, _) => {
                Some(snap(pc, cur_site, &bodies, false, FailKind::Cold, &mut fails))
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
            | OpCode::JumpFalse(_) => Some(snap(pc, cur_site, &bodies, false, FailKind::Cold, &mut fails)),
            OpCode::Call { .. } | OpCode::DynCall { .. } => Some(snap(
                pc,
                cur_site,
                &bodies,
                false,
                FailKind::Call,
                &mut fails,
            )),
            OpCode::EvalScript => Some(snap(pc, cur_site, &bodies, true, FailKind::EvalScript, &mut fails)),
            OpCode::LappendVar(_) => Some(snap(pc, cur_site, &bodies, false, FailKind::Cold, &mut fails)),
            OpCode::EvalExpr => Some(snap(pc, cur_site, &bodies, false, FailKind::EvalExpr, &mut fails)),
            OpCode::ForeachStart { .. } | OpCode::ForeachNext { .. } => Some(snap(
                pc,
                cur_site,
                &bodies,
                false,
                FailKind::Cold,
                &mut fails,
            )),
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
                // 回边 = fuel 计步点：失败上下文在上下文推进之后快照
                // （vm_exec 的 charge 在 Jump 弹 body 之后——帧态一致）。
                pending_backedges.push((pc as u32, t));
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
            OpCode::BodyMarkFramed => {
                let lmap = foreaches.last().map(|(_, l)| *l).unwrap_or(false);
                bodies.push((if lmap { 6 } else { 5 }, cur_site));
            }
            OpCode::ForeachStart { info, next, end, .. } => {
                let lmap = code.foreach_info(*info).map(|f| f.lmap).unwrap_or(false);
                foreaches.push((*info, lmap));
                loops.push((*next, *end, bodies.len(), cur_site));
            }
            // FOREACH_NEXT：vm_exec 的 op_foreach_next(_close) 回跳循环
            // 头时 bodies 截断到循环入口（LoopFrame.bodies_len）——PR#2
            // 起 FOREACH_NEXT_CLOSE 吸收了体闭合 JUMP，unfused 展开为
            // NOP，BodyFramed 的 region 失去弹出点；此处按同一语义
            // 截断（body 首经时 push 的条目已写入 fail 表，不受影响）。
            OpCode::ForeachNext { .. } => {
                if let Some(l) = loops.last() {
                    let (_, _, blen, owner) = *l;
                    bodies.truncate(blen);
                    cur_site = owner;
                }
            }
            OpCode::ForeachEnd => {
                foreaches.pop();
                loops.pop();
            }
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

        for &(src, t) in &pending_backedges {
            if src == pc as u32 {
                let bf = snap(pc, cur_site, &bodies, false, FailKind::Cold, &mut fails);
                backedges.push(Backedge { src, target: t, fail: bf });
            }
        }
        pending_backedges.retain(|(src, _)| *src != pc as u32);
        depth = base + pushes;
        after_at[pc] = depth;
        max_depth = max_depth.max(depth);
        reads.push(r);
        writes.push(w);
    }

    // 栈纪律校验：线性模拟深度与边目标一致。两类编译器习语放宽：
    // (a) 目标更深（then 臂跳过 else 的 PUSH_EMPTY 直达 POP）——差值
    //     必须由目标处起连续的 POP 吸收（读到的旧槽即被丢弃）；
    // (b) Break/Continue 截断到循环入口深度（vm_exec 的 stack_len 纪律）。
    for (pc, e) in edges.iter().enumerate() {
        let after = after_at[pc];
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
        depth_after: after_at,
        vars,
        fails,
        fail_of,
        used_consts,
        edges,
        backedges,
        const_base: CONST_BASE,
    })
}

