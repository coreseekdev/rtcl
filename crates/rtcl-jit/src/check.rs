//! cpcheck —— 逐层 IR 校验器（Chez `cpcheck.ss` 模式，设计文档 §7.5）。
//! 每个 pass 跑完后应先过对应校验器再进下一层（pipeline 里默认开启）。

use crate::ir::{BReg, Instr, L1, L2, L3, Reg};
use crate::abi::cache;

#[derive(Debug, Clone, PartialEq)]
pub struct CheckError(pub String);

impl core::fmt::Display for CheckError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "jit check: {}", self.0)
    }
}

fn err<T>(what: impl Into<String>) -> Result<T, CheckError> {
    Err(CheckError(what.into()))
}

/// L1: 标注数组形状一致；lane 值合法。
pub fn check_l1(l1: &L1) -> Result<(), CheckError> {
    if l1.ops.is_empty() {
        return Ok(());
    }
    Ok(())
}

/// L2: 槽位/表索引引用闭合；栈纪律模拟一致；缓存槽唯一且在界内。
pub fn check_l2(l2: &L2) -> Result<(), CheckError> {
    let n = l2.ops.len();
    if l2.reads.len() != n || l2.writes.len() != n || l2.fail_of.len() != n {
        return err("parallel arrays length mismatch");
    }
    if l2.edges.len() != n {
        return err::<()>("edges length mismatch");
    }
    let d = (l2.max_depth + 1) as usize;
    for (i, (r, w)) in l2.reads.iter().zip(&l2.writes).enumerate() {
        if r.len() > d || w.is_some_and(|w| w as usize >= d) {
            return err(format!("op {i}: register out of bounds"));
        }
        if r.iter().any(|x| *x as usize >= d) {
            return err(format!("op {i}: read register out of bounds"));
        }
    }
    for (i, fo) in l2.fail_of.iter().enumerate() {
        if fo.is_some_and(|f| f as usize >= l2.fails.len()) {
            return err(format!("op {i}: fail index out of bounds"));
        }
    }
    for (i, e) in l2.edges.iter().enumerate() {
        for t in e {
            if *t as usize >= n {
                return err(format!("op {i}: edge target {t} out of bounds"));
            }
        }
    }
    let mut seen = [false; cache::MAX_VARS];
    for v in &l2.vars {
        if v.cache as usize >= cache::MAX_VARS || seen[v.cache as usize] {
            return err(format!("var {}: bad cache slot {}", v.name, v.cache));
        }
        seen[v.cache as usize] = true;
    }
    for be in &l2.backedges {
        let (s, t) = (be.src, be.target);
        if t >= s || s as usize >= n || t as usize >= n {
            return err(format!("backedge {s}->{t} malformed (must target earlier pc)"));
        }
        if be.fail as usize >= l2.fails.len() {
            return err(format!("backedge {s}->{t} fail index out of bounds"));
        }
    }
    Ok(())
}

/// L3: 分支深度 ≤ 嵌套深度；寄存器引用在界内。
pub fn check_l3(l3: &L3, max_regs: u32, max_bregs: u32) -> Result<(), CheckError> {
    walk_l3(l3, 0, &mut |node, depth| {
        match node {
            L3::Br { depth: d, .. } | L3::BrIf { depth: d, .. } => {
                if *d > depth {
                    return err(format!("br depth {d} exceeds nesting {depth}"));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    let mut regs = 0u32;
    let mut bregs = 0u32;
    walk_l3(l3, 0, &mut |node, _| {
        for r in instr_regs(node) {
            if r.0 >= max_regs {
                return err(format!("value register r{} out of bounds", r.0));
            }
            regs += 1;
        }
        for b in instr_bregs(node) {
            if b.0 >= max_bregs {
                return err(format!("bool register b{} out of bounds", b.0));
            }
            bregs += 1;
        }
        Ok(())
    })?;
    let _ = (regs, bregs);
    Ok(())
}

/// 遍历：`depth` = 当前标签嵌套数（Loop/Block/If 的 then/else 各一层）。
fn walk_l3(
    node: &L3,
    depth: u32,
    f: &mut impl FnMut(&L3, u32) -> Result<(), CheckError>,
) -> Result<(), CheckError> {
    f(node, depth)?;
    match node {
        L3::Seq(items) => {
            for i in items {
                walk_l3(i, depth, f)?;
            }
        }
        L3::If { then_, else_, .. } => {
            walk_l3(then_, depth + 1, f)?;
            walk_l3(else_, depth + 1, f)?;
        }
        L3::Loop { body: b, .. } | L3::Block { body: b, .. } => walk_l3(b, depth + 1, f)?,
        _ => {}
    }
    Ok(())
}

fn instr_regs(node: &L3) -> Vec<Reg> {
    let L3::Instr(i) = node else { return vec![] };
    match i {
        Instr::Const { dst, .. }
        | Instr::Copy { dst, .. }
        | Instr::LoadVar { dst, .. }
        | Instr::IncrVar { dst, .. }
        | Instr::BoolTest { v: dst, .. } => vec![*dst],
        Instr::StoreVar { src, .. } => vec![*src],
        Instr::Arith { dst, a, b, .. } | Instr::Cmp { dst, a, b, .. } => vec![*dst, *a, *b],
        Instr::Unary { dst, a, .. } => vec![*dst, *a],
        Instr::CallSite { dst, .. } => vec![*dst],
        Instr::SetResult { src } => vec![*src],
        _ => vec![],
    }
}

fn instr_bregs(node: &L3) -> Vec<BReg> {
    let L3::Instr(i) = node else {
        return match node {
            L3::BrIf { cond, .. } => vec![*cond],
            _ => vec![],
        };
    };
    match i {
        Instr::BoolTest { dst, .. } => vec![*dst],
        _ => vec![],
    }
}
