//! 反向引用的回溯执行（NFA/DFA 不能表达回引）。
//!
//! 只在 `Prog.has_backref` 时启用：枚举 AST 的所有匹配终点并保留
//! 捕获快照（回引依赖已捕获文本），按根偏好取整体最长/最短。
//! 预算受限（防病态模式）；语义优先保证与 Spencer 一致。

use super::ast::{Node, NodeKind, DUPINF};
use super::matcher::{assert_ok, matches_at, Match};
use super::nfa::Prog;

type Caps = Vec<Option<(usize, usize)>>;
type State = (usize, Caps);

const BUDGET: u64 = 2_000_000;
const MAX_ENDS: usize = 512;

struct Bt<'a> {
    prog: &'a Prog,
    text: &'a [char],
    search_start: usize,
    budget: u64,
}

pub(crate) fn find(prog: &Prog, text: &[char], from: usize, search_start: usize) -> Option<Match> {
    let mut bt = Bt {
        prog,
        text,
        search_start,
        budget: BUDGET,
    };
    let shortest = prog.flags.shorter;
    for start in from..=text.len() {
        let mut caps = vec![None; prog.nsubs + 1];
        let mut ends = Vec::new();
        bt.node(&prog.tree, start, true, &mut caps, &mut ends);
        if ends.is_empty() {
            continue;
        }
        let mut best = ends[0].0;
        for (e, _) in &ends[1..] {
            if (shortest && *e < best) || (!shortest && *e > best) {
                best = *e;
            }
        }
        return Some(Match {
            start,
            end: best,
        });
    }
    None
}

impl<'a> Bt<'a> {
    fn node(
        &mut self,
        n: &Node,
        pos: usize,
        record: bool,
        caps: &mut Caps,
        out: &mut Vec<State>,
    ) {
        if self.budget == 0 || out.len() >= MAX_ENDS {
            return;
        }
        self.budget -= 1;
        match &n.kind {
            NodeKind::Empty => out.push((pos, caps.clone())),
            NodeKind::Char(c) => {
                if pos < self.text.len() {
                    let ch = char::from_u32(*c).unwrap_or('\u{FFFD}');
                    if self.char_eq(self.text[pos], ch) {
                        out.push((pos + 1, caps.clone()));
                    }
                }
            }
            NodeKind::Class(set) => {
                if pos < self.text.len() && set.matches(self.text[pos], self.prog.icase) {
                    out.push((pos + 1, caps.clone()));
                }
            }
            NodeKind::Assert(k) => {
                if assert_ok(*k, pos, self.text, self.search_start) {
                    out.push((pos, caps.clone()));
                }
            }
            NodeKind::Capture { subno, node } => {
                let mut inner = Vec::new();
                self.node(node, pos, record, caps, &mut inner);
                for (e, mut c) in inner {
                    if record {
                        c[*subno] = Some((pos, e));
                    }
                    out.push((e, c));
                }
            }
            NodeKind::NoRecord(node) => self.node(node, pos, false, caps, out),
            NodeKind::Concat(parts) => self.concat(parts, pos, record, caps, out),
            NodeKind::Alt(branches) => {
                for b in branches {
                    self.node(b, pos, record, caps, out);
                    if self.budget == 0 || out.len() >= MAX_ENDS {
                        return;
                    }
                }
            }
            NodeKind::Repeat { node, min, max } => {
                self.repeat(node, *min, *max, pos, record, caps, out)
            }
            NodeKind::Backref { subno, min, max } => {
                let Some(Some((cs, ce))) = caps.get(*subno).copied() else {
                    return;
                };
                let unit: Vec<char> = self.text[cs..ce].to_vec();
                if unit.is_empty() {
                    // 空捕获重复任意次仍是空串（Spencer cbrdissect
                    // "matched trivially"）。
                    out.push((pos, caps.clone()));
                    return;
                }
                let maxc = if *max >= DUPINF { u32::MAX } else { *max };
                let mut p = pos;
                let mut count = 0u32;
                loop {
                    if count >= *min {
                        out.push((p, caps.clone()));
                    }
                    if count >= maxc || p + unit.len() > self.text.len() {
                        break;
                    }
                    if self.text[p..p + unit.len()] != unit[..] {
                        break;
                    }
                    p += unit.len();
                    count += 1;
                }
            }
            NodeKind::Look { negative, node } => {
                if let Ok(sub) = super::nfa::compile(node, self.prog.icase) {
                    let found = matches_at(&sub, self.text, pos, self.search_start);
                    if found != *negative {
                        out.push((pos, caps.clone()));
                    }
                }
            }
        }
    }

    fn concat(
        &mut self,
        parts: &[Node],
        pos: usize,
        record: bool,
        caps: &mut Caps,
        out: &mut Vec<State>,
    ) {
        let mut states: Vec<State> = vec![(pos, caps.clone())];
        for p in parts {
            let mut next = Vec::new();
            for (sp, sc) in &states {
                let mut c = sc.clone();
                self.node(p, *sp, record, &mut c, &mut next);
                if self.budget == 0 || next.len() >= MAX_ENDS {
                    break;
                }
            }
            // 去重（同一终点保留首个捕获快照）。
            let mut seen = std::collections::HashSet::new();
            next.retain(|(e, _)| seen.insert(*e));
            states = next;
            if states.is_empty() {
                break;
            }
        }
        out.extend(states);
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &mut self,
        child: &Node,
        min: u32,
        max: u32,
        pos: usize,
        record: bool,
        caps: &mut Caps,
        out: &mut Vec<State>,
    ) {
        let maxc = if max >= DUPINF { u32::MAX } else { max };
        self.repeat_from(child, min, maxc, pos, 0, record, caps, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat_from(
        &mut self,
        child: &Node,
        min: u32,
        maxc: u32,
        pos: usize,
        count: u32,
        record: bool,
        caps: &mut Caps,
        out: &mut Vec<State>,
    ) {
        if self.budget == 0 || out.len() >= MAX_ENDS {
            return;
        }
        self.budget -= 1;
        if count >= min {
            out.push((pos, caps.clone()));
        }
        if count >= maxc {
            return;
        }
        let mut inner = Vec::new();
        let mut c = caps.clone();
        if record {
            zap(child, &mut c);
        }
        self.node(child, pos, record, &mut c, &mut inner);
        inner.retain(|(e, _)| *e > pos); // 迭代必须推进（回引/空子式除外见 min）
        for (e, mut nc) in inner {
            self.repeat_from(child, min.saturating_sub(1), maxc, e, count + 1, record, &mut nc, out);
        }
    }

    fn char_eq(&self, a: char, b: char) -> bool {
        if a == b {
            return true;
        }
        if !self.prog.icase {
            return false;
        }
        a == b.to_lowercase().next().unwrap_or(b) || a == b.to_uppercase().next().unwrap_or(b)
    }
}

fn zap(n: &Node, caps: &mut Caps) {
    match &n.kind {
        NodeKind::Capture { subno, node } => {
            if *subno < caps.len() {
                caps[*subno] = None;
            }
            zap(node, caps);
        }
        NodeKind::Concat(v) | NodeKind::Alt(v) => {
            for x in v {
                zap(x, caps);
            }
        }
        NodeKind::Repeat { node, .. } | NodeKind::NoRecord(node) | NodeKind::Look { node, .. } => {
            zap(node, caps)
        }
        _ => {}
    }
}
