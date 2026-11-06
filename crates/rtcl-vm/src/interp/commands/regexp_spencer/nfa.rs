//! Thompson NFA 构建（Spencer `regc_nfa.c` 的语义子集）。
//!
//! 不复刻 color/byte-tree 与 arc 内存池；每个状态携带出弧列表，弧类型
//! 为 epsilon / 字符集 / 零宽断言。颜色只影响性能，不影响可观察语义。

use super::ast::{AssertKind, Flags, Node, NodeKind, DUPINF};
use super::charset::CharSet;
use super::Error;

#[derive(Clone, Debug)]
pub enum Trans {
    Eps(usize),
    Char(CharSet, usize),
    Assert(AssertKind, usize),
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub trans: Vec<Trans>,
}

#[derive(Clone, Debug)]
pub struct Prog {
    pub states: Vec<State>,
    pub start: usize,
    pub accept: usize,
    /// 根节点偏好位（决定整体最长/最短）。
    pub flags: Flags,
    pub icase: bool,
    /// 每个匹配都必须以此字面前缀开头（空 = 无）；用于跳过不可能起点。
    pub literal_prefix: Vec<char>,
    /// 首字符集（断言可达时为 None——首字符依赖位置）。
    pub first: Option<Vec<CharSet>>,
    /// 空匹配可能（仅 eps 可达 accept）。
    pub nullable: bool,
    /// 纯字面模式（无任何结构）：可直接子串搜索。
    pub pure_literal: Option<Vec<char>>,
}

struct Builder {
    states: Vec<State>,
}

impl Builder {
    fn new_state(&mut self) -> usize {
        let id = self.states.len();
        self.states.push(State::default());
        id
    }

    /// 把 `node` 编译成 "entry state → next" 的片段，返回 entry。
    fn compile(&mut self, node: &Node, next: usize) -> Result<usize, Error> {
        match &node.kind {
            NodeKind::Empty => Ok(next),
            NodeKind::Char(c) => {
                let st = self.new_state();
                self.states[st]
                    .trans
                    .push(Trans::Char(CharSet::from_char(char::from_u32(*c).unwrap_or('\u{FFFD}')), next));
                Ok(st)
            }
            NodeKind::Class(set) => {
                let st = self.new_state();
                self.states[st]
                    .trans
                    .push(Trans::Char(set.clone(), next));
                Ok(st)
            }
            NodeKind::Assert(k) => {
                let st = self.new_state();
                self.states[st].trans.push(Trans::Assert(*k, next));
                Ok(st)
            }
            NodeKind::Concat(parts) => {
                let mut cur = next;
                for p in parts.iter().rev() {
                    cur = self.compile(p, cur)?;
                }
                Ok(cur)
            }
            NodeKind::Alt(branches) => {
                let st = self.new_state();
                for b in branches {
                    let e = self.compile(b, next)?;
                    self.states[st].trans.push(Trans::Eps(e));
                }
                Ok(st)
            }
            NodeKind::Capture { node, .. } => self.compile(node, next),
            NodeKind::Repeat { node, min, max } => {
                self.compile_repeat(node, *min, *max, next)
            }
            NodeKind::Backref { .. } | NodeKind::Look { .. } => Err(Error::new(
                "RTCL_SPENCER_UNSUPPORTED",
                "backref/lookahead not implemented yet",
            )),
        }
    }

    fn compile_repeat(
        &mut self,
        node: &Node,
        min: u32,
        max: u32,
        next: usize,
    ) -> Result<usize, Error> {
        if max >= DUPINF {
            // star 循环：loop --ε--> body / next；min 份强制副本在 loop 前。
            let loop_state = self.new_state();
            let body = self.compile(node, loop_state)?;
            self.states[loop_state].trans.push(Trans::Eps(body));
            self.states[loop_state].trans.push(Trans::Eps(next));
            let mut entry = loop_state;
            for _ in 0..min {
                entry = self.compile(node, entry)?;
            }
            Ok(entry)
        } else {
            // 先建 min 份强制尾巴，再在前面加 (max-min) 个可跳过副本。
            let mut cur = next;
            for _ in 0..min {
                cur = self.compile(node, cur)?;
            }
            for _ in 0..(max - min) {
                let body = self.compile(node, cur)?;
                let st = self.new_state();
                self.states[st].trans.push(Trans::Eps(body));
                self.states[st].trans.push(Trans::Eps(cur));
                cur = st;
            }
            Ok(cur)
        }
    }
}

/// 编译 AST 为 NFA；`icase` 用于匹配时折叠。
pub fn compile(node: &Node, icase: bool) -> Result<Prog, Error> {
    let mut b = Builder {
        states: Vec::new(),
    };
    let accept = b.new_state();
    let start = b.compile(node, accept)?;
    let literal_prefix = literal_prefix(node);
    let first = first_sets(&b.states, start, accept);
    let nullable = nullable_eps(&b.states, start, accept);
    let pure_literal = pure_literal(node);
    Ok(Prog {
        states: b.states,
        start,
        accept,
        flags: node.flags,
        icase,
        literal_prefix,
        first,
        nullable,
        pure_literal,
    })
}

/// 匹配必须开始于的字面前缀：连续 `Char`（穿过 Capture）。
fn literal_prefix(node: &Node) -> Vec<char> {
    fn walk(n: &Node, out: &mut Vec<char>, started: &mut bool) {
        match &n.kind {
            NodeKind::Char(c) => {
                out.push(char::from_u32(*c).unwrap_or('\u{FFFD}'));
                *started = true;
            }
            NodeKind::Capture { node, .. } => walk(node, out, started),
            NodeKind::Concat(parts) => {
                for p in parts {
                    let before = out.len();
                    walk(p, out, started);
                    if out.len() == before {
                        break;
                    }
                    if !matches!(p.kind, NodeKind::Char(_) | NodeKind::Capture { .. }) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut started = false;
    walk(node, &mut out, &mut started);
    out
}

/// 首字符集：从 start 做 epsilon 闭包，收集首条字符弧；断言可达 → None。
fn first_sets(states: &[State], start: usize, accept: usize) -> Option<Vec<CharSet>> {
    let mut seen = vec![false; states.len()];
    let mut stack = vec![start];
    let mut sets: Vec<CharSet> = Vec::new();
    while let Some(s) = stack.pop() {
        if s == accept || seen[s] {
            continue;
        }
        seen[s] = true;
        for t in &states[s].trans {
            match t {
                Trans::Eps(to) => stack.push(*to),
                // 断言依赖位置，首字符集不可静态判定。
                Trans::Assert(_, _) => return None,
                Trans::Char(set, _) => {
                    if !sets.iter().any(|x| x == set) {
                        sets.push(set.clone());
                    }
                }
            }
        }
    }
    if sets.is_empty() {
        None
    } else {
        Some(sets)
    }
}

/// 仅 epsilon 可达 accept（字符/断言阻挡）。
fn nullable_eps(states: &[State], start: usize, accept: usize) -> bool {
    let mut seen = vec![false; states.len()];
    let mut stack = vec![start];
    while let Some(s) = stack.pop() {
        if s == accept {
            return true;
        }
        if seen[s] {
            continue;
        }
        seen[s] = true;
        for t in &states[s].trans {
            if let Trans::Eps(to) = t {
                stack.push(*to);
            }
        }
    }
    false
}

/// 纯字面（Concat/Capture 里全是 Char）→ 可走子串搜索。
fn pure_literal(node: &Node) -> Option<Vec<char>> {
    fn walk(n: &Node, out: &mut Vec<char>) -> bool {
        match &n.kind {
            NodeKind::Char(c) => {
                out.push(char::from_u32(*c).unwrap_or('\u{FFFD}'));
                true
            }
            NodeKind::Capture { node, .. } => walk(node, out),
            NodeKind::Concat(parts) => parts.iter().all(|p| walk(p, out)),
            _ => false,
        }
    }
    let mut out = Vec::new();
    if walk(node, &mut out) {
        Some(out)
    } else {
        None
    }
}
