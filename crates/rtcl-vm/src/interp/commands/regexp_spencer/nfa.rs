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
    Ok(Prog {
        states: b.states,
        start,
        accept,
        flags: node.flags,
        icase,
    })
}
