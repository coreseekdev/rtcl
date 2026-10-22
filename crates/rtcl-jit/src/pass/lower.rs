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
    /// 直接后支配者（虚拟出口 = 单元末尾）。
    idom: Vec<u32>,
    /// foreach 计数器序号（i/iters 对，local = foreach_base + 2n）。
    fcounters: u32,
}

pub fn run(l2: &L2, _code: &ByteCode) -> Result<(L3, u32), Unsupported> {
    FOREACH_COUNTERS.with(|m| m.borrow_mut().clear());
    let n = l2.ops.len();
    // 空单元（`proc p {}` 的体）：无 CFG 可结构化——直接出最小模块
    // （置空结果 + OK）。可达集种子会摸 edges[0]，空表越界。
    if n == 0 {
        let root = L3::Seq(vec![
            L3::Instr(Instr::EpochCheck { at_entry: true }),
            L3::Instr(Instr::Const {
                dst: Reg(0),
                bits: crate::abi::EMPTY_HANDLE,
                what: ConstWhat::Handle,
            }),
            L3::Instr(Instr::SetResult { src: Reg(0) }),
            L3::Instr(Instr::ReturnCode { code: crate::abi::code::OK }),
        ]);
        return Ok((root, 0));
    }
    let idom = post_dominate(l2, n as u32);
    let mut ctx = Ctx {
        l2,
        reachable: BTreeSet::new(),
        placed: vec![false; n],
        bregs: 0,
        loops: Vec::new(),
        idom: idom.clone(),
        fcounters: 0,
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
                let (header, body) = loop_body(ctx, pc + 1, cont, brk)?;
                ctx.loops.pop();
                if (brk as usize) < ctx.l2.ops.len() {
                    ctx.placed[brk as usize] = true; // LoopExit
                }
                items.push(L3::Loop {
                    header,
                    exit: brk,
                    body: Box::new(body),
                });
                pc = brk + 1;
            }
            OpCode::ForeachStart { info, next, end } => {
                ctx.placed[pc as usize] = true;
                ctx.loops.push((next, end));
                let ng = ctx
                    .l2
                    .reads[pc as usize]
                    .len() as u16;
                let counter = ctx.fcounters;
                ctx.fcounters += 1;
                FOREACH_COUNTERS.with(|m| m.borrow_mut().push((info, counter)));
                let (header, body) = loop_body(ctx, pc + 1, next, end)?;
                ctx.loops.pop();
                if (end as usize) < ctx.l2.ops.len() {
                    ctx.placed[end as usize] = true; // ForeachEnd 由模板发射
                    // —— 标记已放置，pc 跳过。
                }
                let ebreg = BReg(ctx.bregs);
                ctx.bregs += 1;
                items.push(L3::Seq(vec![
                    L3::Instr(Instr::ForeachStart {
                        info,
                        ngroups: ng,
                        counter,
                        srcs: ctx.l2.reads[pc as usize].iter().map(|x| Reg(*x)).collect(),
                        fail: ctx.l2.fail_of[pc as usize].unwrap_or(0),
                    }),
                    L3::Loop {
                        header,
                        exit: end,
                        body: Box::new(L3::Seq(vec![
                            L3::Instr(Instr::ForeachEntry { counter, b: ebreg }),
                            L3::BrIf {
                                target: end,
                                depth: 0,
                                cond: ebreg,
                                when_zero: true,
                                backedge: false,
                                fail: 0,
                            },
                            body,
                        ])),
                    },
                    // ForeachEnd（brk 出口与正常出口都在此汇合）。
                    L3::Instr(Instr::ForeachEnd {
                        // 槽号 = 深度-1（depth_after 是执行后的深度计数）。
                        dst: Reg(depth_at_end(ctx, end).saturating_sub(1)),
                    }),
                ]));
                pc = end + 1;
            }
            OpCode::ForeachNext { info, body: body_t } => {
                ctx.placed[pc as usize] = true;
                // 展开：Block(done=end){ step(i+1,i<iters→b);
                // br_if done when b==0; bind(i); fuel; br header }。
                let breg = BReg(ctx.bregs);
                ctx.bregs += 1;
                let fail = ctx.l2.fail_of[pc as usize].unwrap_or(0);
                // done → br 到循环出口（= ForeachEnd 处的 exit 标签）；
                // 继续迭代 → bind + br header（fuel 由 instrument 织入）。
                let end_t = ctx.loops.last().map(|(_, b)| *b).unwrap_or(pc + 1);
                let counter = foreach_counter_of(ctx, info);
                // 回跳目标 = 对应 FOREACH_START 的落点（header）而非
                // body 目标：PR#2 起 unfused 形状的 FOREACH_NEXT body
                // 指向 BeginCmd（跳过 BODY_MARK_FRAMED）——JIT 的
                // region 栈是编译期烘焙（BODY_MARK_FRAMED 无代码），
                // 跳 header 与 vm_exec 跳 body 语义等价，且 header 有
                // Loop 标签可解析（PR#2 前两者重合，回跳目标是 body
                // 还是 header 无差别）。
                let start_pc = ctx
                    .l2
                    .ops
                    .iter()
                    .take(pc as usize)
                    .position(|a| matches!(&a.op, OpCode::ForeachStart { info: i, .. } if *i == info))
                    .map(|p| p as u32 + 1)
                    .unwrap_or(body_t);
                items.push(L3::Seq(vec![
                    L3::Instr(Instr::ForeachStep { counter, b: breg }),
                    L3::BrIf {
                        target: end_t,
                        depth: 0,
                        cond: breg,
                        when_zero: true,
                        backedge: false,
                        fail: 0,
                    },
                    L3::Instr(Instr::ForeachBind { info, counter, fail }),
                    L3::Br {
                        target: start_pc,
                        depth: 0,
                        backedge: true,
                        fail,
                    },
                ]));
                pc += 1;
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
                // 产生者分析：值的写入者是「输出恒为 tagged 0/1」的 op
                // → 布尔转换纯化（免宿主兜底路径）。
                let pure_bool = {
                    let mut producer = None;
                    for back in (0..pc).rev() {
                        if ctx.l2.writes[back as usize] == Some(v) {
                            producer = Some(&ctx.l2.ops[back as usize].op);
                            break;
                        }
                    }
                    matches!(
                        producer,
                        Some(
                            OpCode::Lt
                                | OpCode::Gt
                                | OpCode::Le
                                | OpCode::Ge
                                | OpCode::Eq
                                | OpCode::Ne
                                | OpCode::And
                                | OpCode::Or
                                | OpCode::Not
                                | OpCode::StrEq
                                | OpCode::StrNe
                        )
                    )
                };
                let test = if pure_bool {
                    L3::Instr(Instr::BoolOfBool { dst: breg, v: Reg(v) })
                } else {
                    L3::Instr(Instr::BoolTest { dst: breg, v: Reg(v), fail })
                };
                let taken_when_true = matches!(op, OpCode::JumpTrue(_));
                // ipdom = 真实汇合点；臂区间不得越过当前 range（越界臂
                // 以 Br 到祖先标签收尾——如 for 的 cond-false → 循环出口，
                // 落到 range 末尾会错进 next 区域）。
                let j_real = {
                    let ip = ctx.idom[pc as usize];
                    let j = if ip == u32::MAX { end } else { ip };
                    if j > pc { j } else { end }
                };
                let (arm_target, arm_fall, cont_pc) = if t >= end {
                    // 转移目标在 range 外（典型：cond-false → 循环出口）。
                    let fail = if t <= pc { backedge_fail(ctx, pc, t) } else { 0 };
                    (
                        L3::Br { target: t, depth: 0, backedge: t <= pc, fail },
                        seq(ctx, pc + 1, end)?,
                        end,
                    )
                } else if j_real > end {
                    (seq(ctx, t, end)?, seq(ctx, pc + 1, end)?, end)
                } else {
                    (seq(ctx, t, j_real)?, seq(ctx, pc + 1, j_real)?, j_real)
                };
                let (then_, else_) = if taken_when_true {
                    (arm_target, arm_fall)
                } else {
                    (arm_fall, arm_target)
                };
                items.push(L3::Seq(vec![
                    test,
                    L3::If {
                        cond: breg,
                        // wasm 级汇合位置（= cont_pc）：真实 ipdom 越界时
                        // 若用 j_real 做标签会遮蔽同 pc 的祖先标签
                        // （循环出口），cond-false 分支会错误折回 if 末尾。
                        j: cont_pc,
                        then_: Box::new(then_),
                        else_: Box::new(else_),
                    },
                ]));
                pc = cont_pc;
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

/// pc 处（含）执行后的值栈深度——查 alloc_locals 的模拟表（重放需
/// 复刻 ForeachStart 的 ngroups 特判，查表免重复；lmap 列表 dst 曾因
/// 重放高估 1 槽而错位——puts 读到残留的末次迭代值）。
fn depth_at_end(ctx: &Ctx, end: u32) -> u32 {
    ctx.l2.depth_after[end as usize]
}

/// 循环体结构（for/foreach 共用）：cont 介于 header/brk 时 body 包
/// Block(cont)——continue 落到 next/ForeachNext 区。
fn loop_body(ctx: &mut Ctx, header: u32, cont: u32, brk: u32) -> Result<(u32, L3), Unsupported> {
    let body = if cont > header && cont < brk {
        Ok(L3::Seq(vec![
            L3::Block {
                cont,
                body: Box::new(seq(ctx, header, cont)?),
            },
            seq(ctx, cont, brk)?,
        ]))
    } else {
        seq(ctx, header, brk)
    };
    Ok((header, body?))
}

/// 该 info 的计数器序号（ForeachStart 分配序与发射序一致：按 info idx
/// 稳定——ForeachNext 查询同表）。
fn foreach_counter_of(_ctx: &Ctx, info: u32) -> u32 {
    // ForeachStart 顺序处理，counter 按出现序分配；info 与 counter 的
    // 对应存于侧表（thread_local 会破坏纯函数性——改在 run() 里收集）。
    FOREACH_COUNTERS.with(|m| m.borrow().iter().find(|(i, _)| *i == info).map(|(_, c)| *c).unwrap_or(0))
}

thread_local! {
    static FOREACH_COUNTERS: std::cell::RefCell<Vec<(u32, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// 直线 op 的模板（控制流 op 除外）。
fn template(ctx: &Ctx, pc: u32, op: &OpCode) -> Result<Option<Instr>, Unsupported> {
    let r = &ctx.l2.reads[pc as usize];
    let w = ctx.l2.writes[pc as usize];
    let fail = || ctx.l2.fail_of[pc as usize].expect("fail id");
    let var = |ctx: &Ctx, pc: u32| -> u32 {
        let idx = match &ctx.l2.ops[pc as usize].op {
            OpCode::LoadVar(i) | OpCode::StoreVar(i) | OpCode::StoreVarPop(i)
            | OpCode::IncrVar(i, _) | OpCode::LappendVar(i) => *i as u32,
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
        OpCode::IncrVar(_, n) => Instr::IncrVar {
            dst: dst(),
            var: var(ctx, pc),
            amount: *n,
            fail: fail(),
            site: ctx.l2.fails[fail() as usize].site,
        },
        OpCode::Concat(_) => Instr::Concat {
            dst: dst(),
            srcs: r.iter().map(|x| Reg(*x)).collect(),
            fail: 0,
        },
        OpCode::ForeachCollect => Instr::ForeachCollect { src: Reg(r[0]) },
        OpCode::ForeachEnd => Instr::ForeachEnd { dst: dst() },
        OpCode::EvalScript => Instr::EvalScript {
            dst: dst(),
            src: Reg(r[0]),
            fail: fail(),
        },
        OpCode::EvalExpr => Instr::EvalExpr {
            dst: dst(),
            src: Reg(r[0]),
            fail: fail(),
        },
        OpCode::LappendVar(_) => Instr::LappendVar {
            dst: dst(),
            src: Reg(r[0]),
            var: var(ctx, pc),
            fail: fail(),
        },
        OpCode::DynCall { argc } => {
            let (cont, brk) = match ctx.loops.last() {
                Some(&(c, b)) => (Some(c), Some(b)),
                None => (None, None),
            };
            Instr::CallSite {
                dst: dst(),
                cmd_id: 0,
                argc: *argc,
                fail: fail(),
                site: ctx.l2.fails[fail() as usize].site,
                srcs: r.iter().map(|x| Reg(*x)).collect(),
                brk,
                cont,
                cont_back: cont.is_some_and(|c| c <= pc),
                brk_depth: None,
                cont_depth: None,
            }
        }
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
                srcs: r.iter().map(|x| Reg(*x)).collect(),
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

/// 分支的汇合点 = 直接后支配者（ipdom）。循环内两臂经回边彼此可达，
/// 可达集求交会被污染（把 header 当 merge）——后支配树是正确工具：
/// break-if 的 ipdom = 循环出口，普通 if-else 的 ipdom = 真实 merge。
/// n = 单元出口哨兵（虚拟出口吸收全部终端）。
fn post_dominate(l2: &L2, n: u32) -> Vec<u32> {
    // preds[x]：x 的前驱（含虚拟出口的逆向根）。
    let total = n as usize + 1;
    let mut preds: Vec<Vec<u32>> = vec![vec![]; total];
    for (pc, e) in l2.edges.iter().enumerate() {
        for &t in e {
            if (t as usize) < total {
                preds[t as usize].push(pc as u32);
            }
        }
    }
    // 逆向图（根 = 出口 n）：逆向后继 = 前驱。迭代支配集。
    // dom(v) on reverse graph from root n。
    let all: std::collections::BTreeSet<u32> = (0..=n).collect();
    let mut dom: Vec<std::collections::BTreeSet<u32>> =
        (0..total).map(|i| {
            let mut s = std::collections::BTreeSet::new();
            s.insert(i as u32);
            s
        })
        .collect();
    // 逆向图的根 = n（虚拟出口）。其他点初值 = 全集。
    for d in dom.iter_mut().take(total) {
        if !preds_n_root(d) {
            *d = all.clone();
        }
    }
    dom[n as usize].clear();
    dom[n as usize].insert(n);
    let mut changed = true;
    while changed {
        changed = false;
        for v in 0..total {
            if v == n as usize {
                continue;
            }
            // 逆向图中 v 的前驱 = 正向的后继（edges[v] + 出口边）。
            let mut ps: Vec<u32> = l2.edges[v].clone();
            if l2.edges[v].is_empty() {
                // 终端（无后继）→ 逆向图连虚拟出口。
                ps.push(n);
            }
            if ps.is_empty() {
                continue;
            }
            let mut inter: Option<std::collections::BTreeSet<u32>> = None;
            for p in ps {
                if (p as usize) < total {
                    inter = Some(match inter {
                        None => dom[p as usize].clone(),
                        Some(cur) => cur
                            .intersection(&dom[p as usize])
                            .cloned()
                            .collect(),
                    });
                }
            }
            if let Some(mut inter) = inter {
                inter.insert(v as u32);
                if inter != dom[v] {
                    dom[v] = inter;
                    changed = true;
                }
            }
        }
    }
    // idom(v) = dom(v)\{v} 中 |dom| 最大者。
    let mut idom = vec![u32::MAX; total];
    for v in 0..total {
        let others: Vec<u32> = dom[v].iter().copied().filter(|x| *x != v as u32).collect();
        if let Some(best) = others
            .iter()
            .max_by_key(|c| dom[**c as usize].len())
            .copied()
        {
            idom[v] = best;
        }
    }
    idom
}

fn preds_n_root(_d: &std::collections::BTreeSet<u32>) -> bool {
    false
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
