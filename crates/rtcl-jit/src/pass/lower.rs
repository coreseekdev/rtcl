//! `lower` — L2→L3：op→结构化 wasm 指令模板（指令选择）。
//!
//! 控制流结构化：rtcl 编译器发射的是规范嵌套形（if/elseif/while/for/
//! 短路），CFG 单入口可归约。算法 = 顺序扫描 + 条件点按
//! 「min-common-reachable = 汇合点」分臂递归；分支深度由 phase B 按
//! 标签栈解析（If 的 merge、Loop 的 header/exit、Block 的续体——
//! 标签 pc 由构造侧直接记录在节点上）。无法结构化的形状 → 编译拒绝。

use crate::abi::{const_handle, tag_imm, BinOp, EMPTY_HANDLE};
use crate::emit::Unsupported;
use crate::ir::{BReg, ConstWhat, CmpOp, Instr, L2, L3, Reg};
use rtcl_ir::bytecode::ByteCode;
use rtcl_ir::opcode::OpCode;
use std::collections::BTreeSet;

struct Ctx<'a> {
    l2: &'a L2,
    reachable: BTreeSet<u32>,
    placed: Vec<bool>,
    bregs: u32,
    /// (cont, brk) of enclosing loops。
    loops: Vec<(u32, u32)>,
}

pub fn run(l2: &L2, _code: &ByteCode) -> Result<(L3, u32), Unsupported> {
    let n = l2.ops.len();
    let mut ctx = Ctx {
        l2,
        reachable: BTreeSet::new(),
        placed: vec![false; n],
        bregs: 0,
        loops: Vec::new(),
    };
    // 可达集（从 0 出发沿 edges）。
    let mut work = vec![0u32];
    ctx.reachable.insert(0);
    while let Some(p) = work.pop() {
        for &t in &l2.edges[p as usize] {
            if (t as usize) < n && ctx.reachable.insert(t) {
                work.push(t);
            }
        }
    }
    let body = seq(&mut ctx, 0, n as u32)?;
    // 单元终结：流末值 = 栈顶槽（深度是 pc 的确定函数）。
    let final_depth = final_depth(l2);
    let exit_reg = l2.max_depth; // final_depth==0 时的出口专用槽
    let term = if final_depth > 0 {
        L3::Instr(Instr::SetResult { src: Reg(final_depth - 1) })
    } else {
        L3::Instr(Instr::Const {
            dst: Reg(exit_reg),
            bits: EMPTY_HANDLE,
            what: ConstWhat::Handle,
        })
    };
    let mut items = match body {
        L3::Seq(items) => items,
        other => vec![other],
    };
    if final_depth == 0 {
        items.push(term);
        items.push(L3::Instr(Instr::SetResult { src: Reg(exit_reg) }));
    } else {
        items.push(term);
    }
    items.push(L3::Instr(Instr::ReturnCode { code: crate::abi::code::OK }));
    let mut root = L3::Seq(vec![
        L3::Instr(Instr::EpochCheck { at_entry: true }),
        L3::Seq(items),
    ]);
    // phase B：分支深度解析。
    let mut labels = Vec::new();
    resolve(&mut root, &mut labels).map_err(Unsupported)?;
    Ok((root, ctx.bregs))
}

fn final_depth(l2: &L2) -> u32 {
    let mut d = 0u32;
    for op in l2.ops.iter().map(|a| &a.op) {
        let (pops, pushes) = crate::pass::stack_effect(op);
        d = d - pops.min(d) + pushes;
    }
    d
}

/// 顺序结构化 `[start, end)` 内的可达且未放置的 op。
fn seq(ctx: &mut Ctx, start: u32, end: u32) -> Result<L3, Unsupported> {
    let mut items: Vec<L3> = Vec::new();
    let mut pc = start;
    while pc < end {
        if !ctx.reachable.contains(&pc) || ctx.placed[pc as usize] {
            pc += 1;
            continue;
        }
        let op = ctx.l2.ops[pc as usize].op.clone();
        match op {
            OpCode::LoopEnter { cont, brk } => {
                ctx.placed[pc as usize] = true;
                ctx.loops.push((cont, brk));
                let body = seq(ctx, pc + 1, brk)?;
                ctx.loops.pop();
                if (brk as usize) < ctx.l2.ops.len() {
                    ctx.placed[brk as usize] = true; // LoopExit
                }
                items.push(L3::Loop {
                    header: pc + 1,
                    exit: brk,
                    body: Box::new(body),
                });
                pc = brk + 1;
            }
            OpCode::Jump(t) => {
                ctx.placed[pc as usize] = true;
                if t == pc + 1 || t >= end {
                    // 自然落下的跳转：无操作。
                } else if t <= pc {
                    let fail = backedge_fail(ctx, pc, t);
                    items.push(L3::Br { target: t, depth: 0, backedge: true, fail });
                } else {
                    // 前向跳过 [pc+1, t)：该区域若可达，属于此前条件点的
                    // 臂（那次递归已结构化）；这里包 Block 提供目标标签。
                    let inner = seq(ctx, pc + 1, t)?;
                    items.push(L3::Block {
                        cont: t,
                        body: Box::new(L3::Seq(vec![
                            L3::Br { target: t, depth: 0, backedge: false, fail: 0 },
                            inner,
                        ])),
                    });
                }
                pc += 1;
            }
            OpCode::JumpTrue(t) | OpCode::JumpFalse(t) => {
                if t <= pc {
                    return Err(Unsupported(format!(
                        "conditional backedge at pc {pc} (M1' shape)"
                    )));
                }
                ctx.placed[pc as usize] = true;
                let v = ctx.l2.reads[pc as usize][0];
                let breg = BReg(ctx.bregs);
                ctx.bregs += 1;
                let fail = ctx.l2.fail_of[pc as usize].expect("cond fail");
                let taken_when_true = matches!(op, OpCode::JumpTrue(_));
                let j = merge_point(ctx, t, pc + 1).unwrap_or(end).min(end);
                let arm_target = seq(ctx, t, j)?;
                let arm_fall = seq(ctx, pc + 1, j)?;
                let (then_, else_) = if taken_when_true {
                    (arm_target, arm_fall)
                } else {
                    (arm_fall, arm_target)
                };
                items.push(L3::Seq(vec![
                    L3::Instr(Instr::BoolTest { dst: breg, v: Reg(v), fail }),
                    L3::If {
                        cond: breg,
                        j,
                        then_: Box::new(then_),
                        else_: Box::new(else_),
                    },
                ]));
                pc = j;
            }
            OpCode::Break | OpCode::Continue => {
                ctx.placed[pc as usize] = true;
                let Some(&edge) = ctx.l2.edges[pc as usize].first() else {
                    items.push(L3::Instr(Instr::ReturnCode {
                        code: if matches!(op, OpCode::Break) {
                            crate::abi::code::BREAK
                        } else {
                            crate::abi::code::CONTINUE
                        },
                    }));
                    pc += 1;
                    continue;
                };
                let fail = if edge <= pc { backedge_fail(ctx, pc, edge) } else { 0 };
                items.push(L3::Br {
                    target: edge,
                    depth: 0,
                    backedge: edge <= pc,
                    fail,
                });
                pc += 1;
            }
            OpCode::Return => {
                ctx.placed[pc as usize] = true;
                let v = ctx.l2.reads[pc as usize][0];
                items.push(L3::Instr(Instr::SetResult { src: Reg(v) }));
                items.push(L3::Instr(Instr::ReturnCode { code: crate::abi::code::RETURN }));
                pc += 1;
            }
            other => {
                ctx.placed[pc as usize] = true;
                if let Some(instr) = template(ctx, pc, &other)? {
                    items.push(L3::Instr(instr));
                }
                pc += 1;
            }
        }
    }
    Ok(L3::Seq(items))
}

/// 直线 op 的模板（控制流 op 除外）。
fn template(ctx: &Ctx, pc: u32, op: &OpCode) -> Result<Option<Instr>, Unsupported> {
    let r = &ctx.l2.reads[pc as usize];
    let w = ctx.l2.writes[pc as usize];
    let fail = || ctx.l2.fail_of[pc as usize].expect("fail id");
    let var = |ctx: &Ctx, pc: u32| -> u32 {
        let idx = match &ctx.l2.ops[pc as usize].op {
            OpCode::LoadVar(i) | OpCode::StoreVar(i) | OpCode::StoreVarPop(i)
            | OpCode::IncrVar(i, _) => *i as u32,
            _ => 0,
        };
        ctx.l2.vars.iter().find(|v| v.const_idx == idx).map(|v| v.cache).unwrap_or(0)
    };
    let dst = || Reg(w.expect("value-producing op"));
    let instr = match op {
        OpCode::PushInt(n) => Instr::Const { dst: dst(), bits: tag_imm(*n), what: ConstWhat::Imm },
        OpCode::PushTrue => Instr::Const { dst: dst(), bits: tag_imm(1), what: ConstWhat::Imm },
        OpCode::PushFalse => Instr::Const { dst: dst(), bits: tag_imm(0), what: ConstWhat::Imm },
        OpCode::PushEmpty => Instr::Const { dst: dst(), bits: EMPTY_HANDLE, what: ConstWhat::Handle },
        OpCode::PushFloat(f) => Instr::BoxFloat { dst: dst(), v: *f },
        OpCode::PushConst(i) => Instr::Const {
            dst: dst(),
            bits: const_handle(*i as u32),
            what: ConstWhat::Handle,
        },
        OpCode::PushConstWide(i) => Instr::Const {
            dst: dst(),
            bits: const_handle(*i),
            what: ConstWhat::Handle,
        },
        OpCode::BeginCmd(k) => Instr::SetSite(*k),
        OpCode::Dup => Instr::Copy { dst: dst(), src: Reg(r[0]) },
        OpCode::LoadVar(_) => Instr::LoadVar { dst: dst(), var: var(ctx, pc), fail: fail() },
        OpCode::StoreVar(_) => Instr::StoreVar { src: Reg(r[0]), var: var(ctx, pc), keep: true, fail: fail() },
        OpCode::StoreVarPop(_) => Instr::StoreVar { src: Reg(r[0]), var: var(ctx, pc), keep: false, fail: fail() },
        OpCode::IncrVar(_, n) => Instr::IncrVar { dst: dst(), var: var(ctx, pc), amount: *n, fail: fail() },
        OpCode::Call { cmd_id, argc } => {
            let (cont, brk) = match ctx.loops.last() {
                Some(&(c, b)) => (Some(c), Some(b)),
                None => (None, None),
            };
            Instr::CallSite {
                dst: dst(),
                cmd_id: *cmd_id,
                argc: *argc,
                fail: fail(),
                site: ctx.l2.fails[fail() as usize].site,
                brk,
                cont,
                cont_back: cont.is_some_and(|c| c <= pc),
                brk_depth: None,
                cont_depth: None,
            }
        }
        OpCode::Lt | OpCode::Gt | OpCode::Le | OpCode::Ge => {
            let cc = match op {
                OpCode::Lt => CmpOp::Lt,
                OpCode::Gt => CmpOp::Gt,
                OpCode::Le => CmpOp::Le,
                _ => CmpOp::Ge,
            };
            Instr::Cmp { cc, dst: dst(), a: Reg(r[0]), b: Reg(r[1]), fail: fail() }
        }
        OpCode::Neg | OpCode::Not | OpCode::BitNot => {
            let kind = match op {
                OpCode::Neg => BinOp::Neg,
                OpCode::Not => BinOp::Not,
                _ => BinOp::BitNot,
            };
            Instr::Unary { kind, dst: dst(), a: Reg(r[0]), fail: fail() }
        }
        OpCode::Exit(c) => Instr::Exit { code: *c },
        other => {
            let kind = match other {
                OpCode::Add => BinOp::Add,
                OpCode::Sub => BinOp::Sub,
                OpCode::Mul => BinOp::Mul,
                OpCode::Div => BinOp::Div,
                OpCode::Mod => BinOp::Mod,
                OpCode::Pow => BinOp::Pow,
                OpCode::And => BinOp::And,
                OpCode::Or => BinOp::Or,
                OpCode::BitAnd => BinOp::BitAnd,
                OpCode::BitOr => BinOp::BitOr,
                OpCode::BitXor => BinOp::BitXor,
                OpCode::Shl => BinOp::Shl,
                OpCode::Shr => BinOp::Shr,
                OpCode::Eq => BinOp::Eq,
                OpCode::Ne => BinOp::Ne,
                OpCode::StrEq => BinOp::StrEq,
                OpCode::StrNe => BinOp::StrNe,
                _ => return Ok(None), // Nop/Line/marks：无代码
            };
            Instr::Arith { kind, dst: dst(), a: Reg(r[0]), b: Reg(r[1]), fail: fail() }
        }
    };
    Ok(Some(instr))
}

/// 回边 (pc -> t) 的 fuel 失败表索引。
fn backedge_fail(ctx: &Ctx, pc: u32, t: u32) -> u32 {
    ctx.l2
        .backedges
        .iter()
        .find(|b| b.src == pc && b.target == t)
        .map(|b| b.fail)
        .unwrap_or(0)
}

/// 从 `from` 出发的可达集。
fn reach(ctx: &Ctx, from: u32) -> BTreeSet<u32> {
    let mut seen = BTreeSet::new();
    let mut work = vec![from];
    seen.insert(from);
    while let Some(p) = work.pop() {
        for &t in &ctx.l2.edges[p as usize] {
            if seen.insert(t) {
                work.push(t);
            }
        }
    }
    seen
}

/// 两臂的最小公共可达 pc（> lower_bound）。
fn merge_point(ctx: &Ctx, a: u32, b: u32) -> Option<u32> {
    let ra = reach(ctx, a);
    let rb = reach(ctx, b);
    let lower = a.min(b);
    ra.intersection(&rb).copied().filter(|p| *p > lower).min()
}

// ── phase B：分支深度解析 ─────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Label {
    pc: u32,
}

fn resolve(node: &mut L3, labels: &mut Vec<Label>) -> Result<(), String> {
    match node {
        L3::Seq(items) => {
            for i in items.iter_mut() {
                resolve(i, labels)?;
            }
        }
        L3::If { j, then_, else_, .. } => {
            labels.push(Label { pc: *j });
            resolve(then_, labels)?;
            resolve(else_, labels)?;
            labels.pop();
        }
        L3::Loop { header, exit, body } => {
            labels.push(Label { pc: *exit });
            labels.push(Label { pc: *header });
            resolve(body, labels)?;
            labels.pop();
            labels.pop();
        }
        L3::Block { cont, body } => {
            labels.push(Label { pc: *cont });
            resolve(body, labels)?;
            labels.pop();
        }
        L3::Br { target, depth, backedge: _, fail: _ } => {
            let t = *target;
            let Some(idx) = labels.iter().rposition(|l| l.pc == t) else {
                return Err(format!("br target {t} has no enclosing label"));
            };
            *depth = (labels.len() - 1 - idx) as u32;
        }
        L3::BrIf { target, depth, cond: _, when_zero: _, backedge: _, fail: _ } => {
            let t = *target;
            let Some(idx) = labels.iter().rposition(|l| l.pc == t) else {
                return Err(format!("br_if target {t} has no enclosing label"));
            };
            *depth = (labels.len() - 1 - idx) as u32;
        }
        L3::Instr(Instr::CallSite {
            brk, cont, brk_depth, cont_depth, ..
        }) => {
            // 目标 pc → 分支深度（调用现场到目标循环标签）。解析失败
            // （目标不是祖先标签）= 出口深度 None → return 码路径。
            *brk_depth = brk.and_then(|t| resolve_target_opt(labels, t));
            *cont_depth = cont.and_then(|t| resolve_target_opt(labels, t));
        }
        L3::Instr(_) => {}
    }
    Ok(())
}

fn resolve_target_opt(labels: &[Label], t: u32) -> Option<u32> {
    labels
        .iter()
        .rposition(|l| l.pc == t)
        .map(|idx| (labels.len() - 1 - idx) as u32)
}
