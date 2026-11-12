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

use super::ast::{Node, NodeKind, DUPINF};
use super::matcher::{assert_ok, matches_at, Match, Matcher};
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
    };
    if d.node(&prog.tree, m.start, m.end, true, &mut spans) {
        Some(Captures { spans })
    } else {
        None
    }
}

struct Dissect<'a> {
    prog: &'a Prog,
    text: &'a [char],
    search_start: usize,
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
        if shorter {
            for mid in begin..=end {
                let save = caps.clone();
                if self.node(first, begin, mid, record, caps)
                    && self.concat(&parts[1..], mid, end, record, caps)
                {
                    return true;
                }
                *caps = save;
            }
        } else {
            for mid in (begin..=end).rev() {
                let save = caps.clone();
                if self.node(first, begin, mid, record, caps)
                    && self.concat(&parts[1..], mid, end, record, caps)
                {
                    return true;
                }
                *caps = save;
            }
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
        // 迭代端点优先级跟随子节点偏好：贪婪子节点取最长端点，
        // 惰性子节点（仅记录捕获时）取最短端点——tclsh
        // `(a{1,2}?){0,2}` 的末次迭代取最长剩余段。
        let ascending = record && child.flags.shorter;
        let endpoints: Vec<usize> = if ascending {
            (pos + 1..=end).collect()
        } else {
            (pos + 1..=end).rev().collect()
        };
        for ep in endpoints {
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
