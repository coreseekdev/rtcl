//! 捕获 dissect（Spencer `regexec.c` 的 `cdissect` 家族）。
//!
//! 已知整体 span 后，按 subre 树递归分配子匹配：
//! * concat：左侧尽量长（左侧子树 SHORTER 时尽量短），右侧验证到 end；
//! * alternation：按分支书写序，整段匹配且可 dissect 者胜；
//! * iteration：贪婪端点（子节点最长），重复组捕获取最后一次迭代
//!   （每次迭代前 zap 子树的捕获）；
//! * 编译器改写 `x{m,n}`（m>0）= `NoRecord(x{m-1,n-1}) x`，因此
//!   `(a*)+` 的组是尾部空迭代。
//!
//! lookahead/回引的完整执行属 M4；回引按已捕获文本做等长比较，lookahead
//! 暂不支持（返回 false）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::ast::{Node, NodeKind, DUPINF};
use super::matcher::{assert_ok, matches_at, prefix_ends, Match, Matcher};
use super::nfa::{self, Prog};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captures {
    /// 0 = 整体匹配；1..=nsubs = 捕获组（未参与为 None）。
    pub spans: Vec<Option<(usize, usize)>>,
}

/// 执行一次匹配并返回捕获 span（无匹配 → None）。
pub fn exec(
    prog: &Prog,
    text: &[char],
    from: usize,
    search_start: usize,
) -> Option<Captures> {
    let mut m = Matcher::new(prog);
    exec_with(prog, text, from, search_start, &mut m)
}

/// 复用 `Matcher`（DFA 缓存/扫描缓冲）执行捕获匹配——命令级 `-all`
/// 循环用，避免每个起点重建 DFA 缓存。
pub fn exec_with(
    prog: &Prog,
    text: &[char],
    from: usize,
    search_start: usize,
    matcher: &mut Matcher,
) -> Option<Captures> {
    let m: Match = matcher.find(prog, text, from, search_start)?;
    let mut spans = vec![None; prog.nsubs + 1];
    spans[0] = Some((m.start, m.end));
    let d = Dissect {
        prog,
        text,
        search_start,
        depth: core::cell::Cell::new(0),
        progs: RefCell::new(HashMap::new()),
    };
    let found = d.node(&prog.tree, m.start, m.end, true, &mut spans);
    if super::is_over_budget() {
        return None;
    }
    if found {
        Some(Captures { spans })
    } else {
        None
    }
}

/// Levels of the generic iteration search (a repeat whose operand has no
/// fixed length). Each level is a few native frames; a repeat of a
/// variable-length operand iterating thousands of times reaches it, and
/// the match is reported as unfinished (see
/// [`super::take_over_budget`]) instead of running the stack out.
const MAX_DISSECT_DEPTH: u32 = 1000;

/// Span up to which split points are tried directly (see
/// [`Dissect::splits`]).
const DIRECT_SPLIT_SPAN: usize = 64;

struct Dissect<'a> {
    prog: &'a Prog,
    text: &'a [char],
    search_start: usize,
    depth: core::cell::Cell<u32>,
    /// NFA programs of the backreference-free subtrees, by node address
    /// (see [`Dissect::split_points`]).
    progs: RefCell<HashMap<usize, Rc<Prog>>>,
}

impl<'a> Dissect<'a> {
    fn node(
        &self,
        n: &Node,
        begin: usize,
        end: usize,
        record: bool,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> bool {
        match &n.kind {
            NodeKind::Empty => begin == end,
            NodeKind::Char(c) => {
                if end != begin + 1 {
                    return false;
                }
                let ch = char::from_u32(*c).unwrap_or('\u{FFFD}');
                self.char_eq(self.text[begin], ch)
            }
            NodeKind::Class(set) => {
                end == begin + 1 && set.matches(self.text[begin], self.prog.icase)
            }
            NodeKind::Assert(k) => begin == end && assert_ok(*k, begin, self.text, self.search_start),
            NodeKind::Capture { subno, node } => {
                if !self.node(node, begin, end, record, caps) {
                    return false;
                }
                if record {
                    caps[*subno] = Some((begin, end));
                }
                true
            }
            NodeKind::NoRecord(node) => self.node(node, begin, end, false, caps),
            NodeKind::Concat(parts) => self.concat(parts, begin, end, record, caps),
            NodeKind::Alt(branches) => {
                for b in branches {
                    let save = caps.clone();
                    if self.node(b, begin, end, record, caps) {
                        return true;
                    }
                    *caps = save;
                }
                false
            }
            NodeKind::Repeat { node, min, max } => {
                self.repeat(node, *min, *max, begin, end, record, caps)
            }
            NodeKind::Backref { subno, min, max, .. } => self.backref(*subno, *min, *max, begin, end, caps),
            NodeKind::Look { negative, node } => {
                let Ok(sub) = nfa::compile(node, self.prog.icase) else {
                    return false;
                };
                let found = matches_at(&sub, self.text, begin, self.search_start);
                begin == end && found != *negative
            }
        }
    }

    /// Split points worth trying for `node` spanning `[pos, limit]`, in
    /// search order.
    ///
    /// Up to [`DIRECT_SPLIT_SPAN`] characters every offset is tried (a
    /// handful; the per-call check is a character test), which costs less
    /// than the NFA run's fixed overhead. Beyond it the candidates come
    /// from [`Dissect::split_points`].
    fn splits(&self, node: &Node, pos: usize, limit: usize, ascending: bool) -> Vec<usize> {
        // A fixed-length part can end at exactly one offset (a literal
        // separator, a bounded class): the separator in `(\w+)-(\d+)`
        // was tried at all 28 offsets of its span.
        if let Some(len) = super::ast::fixed_len(node).filter(|l| *l > 0) {
            return if pos + len <= limit {
                vec![pos + len]
            } else {
                Vec::new()
            };
        }
        let span = limit - pos;
        if span <= DIRECT_SPLIT_SPAN || node.flags.backr {
            let all: Vec<usize> = (pos..=limit).collect();
            return if ascending { all } else { all.into_iter().rev().collect() };
        }
        self.split_points(node, pos, limit, ascending)
            .unwrap_or_else(|| (pos..=limit).collect())
    }

    /// The offsets where an anchored NFA run of the node really matches.
    /// The dissection used to try every offset —
    /// `regexp -inline {(a+)(b+)}` over 400 characters took 20ms (O(n³))
    /// against tclsh's microseconds, and never finished over 800. The
    /// candidate set is exact (no feasible split is lost) for a subtree
    /// without a backreference; with one the NFA is a placeholder and
    /// `None` sends the caller back to every offset. The span is always
    /// past [`DIRECT_SPLIT_SPAN`] here.
    fn split_points(&self, node: &Node, pos: usize, limit: usize, ascending: bool) -> Option<Vec<usize>> {
        if node.flags.backr {
            return None;
        }
        let key = node as *const Node as usize;
        // The lookup's borrow ends before the miss arm takes the map
        // mutably.
        let cached = self.progs.borrow().get(&key).cloned();
        let prog = match cached {
            Some(p) => p,
            None => {
                let p = Rc::new(nfa::compile(node, self.prog.icase).ok()?);
                self.progs.borrow_mut().insert(key, Rc::clone(&p));
                p
            }
        };
        let mut ends = prefix_ends(&prog, self.text, pos, self.search_start, limit);
        if !ascending {
            ends.reverse();
        }
        Some(ends)
    }

    fn char_eq(&self, a: char, b: char) -> bool {
        if a == b {
            return true;
        }
        if !self.prog.icase {
            return false;
        }
        let lo = b.to_lowercase().next();
        let up = b.to_uppercase().next();
        a == lo.unwrap_or(b) || a == up.unwrap_or(b)
    }

    fn concat(
        &self,
        parts: &[Node],
        begin: usize,
        end: usize,
        record: bool,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> bool {
        if parts.is_empty() {
            return begin == end;
        }
        if parts.len() == 1 {
            return self.node(&parts[0], begin, end, record, caps);
        }
        let first = &parts[0];
        let shorter = first.flags.shorter;
        // Spencer ccondissect/crevcondissect：左子树最长优先（SHORTER
        // 时最短优先），右侧必须精确匹配到 end。
        for mid in self.splits(first, begin, end, shorter) {
            let save = caps.clone();
            if self.node(first, begin, mid, record, caps)
                && self.concat(&parts[1..], mid, end, record, caps)
            {
                return true;
            }
            *caps = save;
        }
        false
    }

    /// 迭代节点（min/max）。`record=false` 时为纯语言成员检查（Spencer
    /// 前缀扁平节点）；否则取最后一次迭代的捕获。
    fn repeat(
        &self,
        child: &Node,
        min: u32,
        max: u32,
        begin: usize,
        end: usize,
        record: bool,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> bool {
        self.repeat_from(child, min, max, begin, end, begin, 0, record, caps)
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat_from(
        &self,
        child: &Node,
        min: u32,
        max: u32,
        begin: usize,
        end: usize,
        pos: usize,
        count: u32,
        record: bool,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> bool {
        let depth = self.depth.get();
        if depth >= MAX_DISSECT_DEPTH {
            super::over_budget();
            return false;
        }
        self.depth.set(depth + 1);
        let r = self.repeat_from_at(child, min, max, begin, end, pos, count, record, caps);
        self.depth.set(depth);
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat_from_at(
        &self,
        child: &Node,
        min: u32,
        max: u32,
        begin: usize,
        end: usize,
        pos: usize,
        count: u32,
        record: bool,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> bool {
        let max_count = if max >= DUPINF { u32::MAX } else { max };
        if pos == end {
            if count >= min {
                return true;
            }
            // 空的剩余迭代（仅当子节点可空且还需要凑 min）。
            if count < max_count {
                let save = caps.clone();
                if record {
                    self.zap(child, caps);
                }
                if self.node(child, pos, pos, record, caps)
                    && self.repeat_from(child, min, max, begin, end, pos, count + 1, record, caps)
                {
                    return true;
                }
                *caps = save;
            }
            return false;
        }
        if count >= max_count {
            return false;
        }
        // A fixed-length operand partitions the span at multiples of its
        // length: the only possible split, so count the iterations
        // instead of searching for endpoints (ast::fixed_len). The last
        // one leaves the captures, as a recorded repeat must.
        if let Some(len) = super::ast::fixed_len(child).filter(|l| *l > 0) {
            let span = end - pos;
            if span % len != 0 {
                return false;
            }
            let iters = (span / len) as u32;
            if count.saturating_add(iters) > max_count {
                return false;
            }
            let save = caps.clone();
            for k in 0..iters as usize {
                if record {
                    self.zap(child, caps);
                }
                if !self.node(child, pos + k * len, pos + (k + 1) * len, record, caps) {
                    *caps = save;
                    return false;
                }
            }
            // A non-empty operand cannot add the empty iterations that
            // would make up a shortfall against `min`.
            if count.saturating_add(iters) < min {
                *caps = save;
                return false;
            }
            return true;
        }
        // 迭代端点优先级跟随子节点偏好：贪婪子节点取最长端点，
        // 惰性子节点（仅记录捕获时）取最短端点——tclsh
        // `(a{1,2}?){0,2}` 的末次迭代取最长剩余段。
        let ascending = record && child.flags.shorter;
        // Only where the child can end at all; an iteration consumes at
        // least one character.
        let endpoints: Vec<usize> = self
            .splits(child, pos, end, ascending)
            .into_iter()
            .filter(|&ep| ep > pos)
            .collect();
        let mut try_endpoint = |ep: usize| -> bool {
            let save = caps.clone();
            if record {
                self.zap(child, caps);
            }
            if self.node(child, pos, ep, record, caps)
                && self.repeat_from(child, min, max, begin, end, ep, count + 1, record, caps)
            {
                return true;
            }
            *caps = save;
            false
        };
        for ep in endpoints {
            if try_endpoint(ep) {
                return true;
            }
        }
        // 必要时允许零长迭代凑 min（不会推进 pos）。
        if count < min {
            let save = caps.clone();
            if record {
                self.zap(child, caps);
            }
            if self.node(child, pos, pos, record, caps)
                && self.repeat_from(child, min, max, begin, end, pos, count + 1, record, caps)
            {
                return true;
            }
            *caps = save;
        }
        false
    }

    /// Spencer `zaptreesubs`：清除子树内所有捕获（每次迭代前）。
    fn zap(&self, n: &Node, caps: &mut Vec<Option<(usize, usize)>>) {
        match &n.kind {
            NodeKind::Capture { subno, node } => {
                if *subno < caps.len() {
                    caps[*subno] = None;
                }
                self.zap(node, caps);
            }
            NodeKind::Concat(v) | NodeKind::Alt(v) => {
                for x in v {
                    self.zap(x, caps);
                }
            }
            NodeKind::Repeat { node, .. } => self.zap(node, caps),
            NodeKind::NoRecord(node) => self.zap(node, caps),
            NodeKind::Look { node, .. } => self.zap(node, caps),
            _ => {}
        }
    }

    fn backref(
        &self,
        subno: usize,
        min: u32,
        max: u32,
        begin: usize,
        end: usize,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> bool {
        let Some(Some((cs, ce))) = caps.get(subno).copied() else {
            return false;
        };
        let unit = &self.text[cs..ce];
        let ulen = unit.len();
        if ulen == 0 {
            // 空捕获重复任意次仍是空串（Spencer "matched trivially"）。
            return begin == end;
        }
        let span = end - begin;
        if span % ulen != 0 {
            return false;
        }
        let count = (span / ulen) as u32;
        let max_count = if max >= DUPINF { u32::MAX } else { max };
        if count < min || count > max_count {
            return false;
        }
        for i in 0..count as usize {
            if &self.text[begin + i * ulen..begin + (i + 1) * ulen] != unit {
                return false;
            }
        }
        true
    }
}
