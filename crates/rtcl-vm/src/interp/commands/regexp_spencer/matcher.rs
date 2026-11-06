//! 整体匹配（Spencer `regexec.c` 的 DFA 层语义）。
//!
//! 语义：默认 POSIX leftmost-longest；根偏好含 SHORTER（任一非贪婪
//! 量词）时整体最短。
//!
//! 性能结构（Rust 版）：
//! * **lazy DFA**（`dfa.rs`）：无断言模式用颜色类 + 转移缓存；
//!   状态超限/含断言时回退 NFA。
//! * **单趟起点跟踪**：所有起点线程同时推进，每状态保留最小起点
//!   （leftmost 优先），不逐起点重启。
//! * **零分配热路径**：代际标记 + 复用 Vec。
//! * **预过滤**：纯字面直接子串搜索；字面前缀 / 首字符集跳过不可
//!   能起点（断言可达时禁用首集，nullable 保护空匹配）。
//!
//! 捕获 dissect（M3）在后续层实现；本模块只保证整体 span。

use super::ast::{AssertKind, Flags};
use super::charset::CharSet;
use super::dfa::DfaCache;
use super::nfa::{Prog, Trans};
use crate::interp::unicode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

/// 可复用的搜索引擎（DFA 缓存在多次 `find` 间保持）。
pub struct Searcher<'a> {
    prog: &'a Prog,
    inner: Matcher,
}

/// 不借用 `Prog` 的匹配器（DFA 缓存/扫描缓冲可跨多次命令级尝试复用）。
pub struct Matcher {
    dfa: Option<DfaCache>,
    scratch: DfaScratch,
}

/// DFA 扫描的复用缓冲（跨多次 find 保持，减少密集匹配的每次分配）。
struct DfaScratch {
    stamp: Vec<u32>,
    start_of: Vec<usize>,
    cur: Vec<u32>,
    next: Vec<u32>,
    gen: u32,
}

impl DfaScratch {
    fn new() -> DfaScratch {
        DfaScratch {
            stamp: Vec::new(),
            start_of: Vec::new(),
            cur: Vec::new(),
            next: Vec::new(),
            gen: 0,
        }
    }
    fn next_gen(&mut self) -> u32 {
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.stamp.fill(0);
            self.gen = 1;
        }
        self.gen
    }
}

impl Matcher {
    pub fn new(prog: &Prog) -> Matcher {
        let dfa = if std::env::var_os("RTCL_SPENCER_NFA").is_some() {
            None
        } else {
            DfaCache::new(prog)
        };
        Matcher {
            dfa,
            scratch: DfaScratch::new(),
        }
    }

    pub fn find(
        &mut self,
        prog: &Prog,
        text: &[char],
        from: usize,
        search_start: usize,
    ) -> Option<Match> {
        let from = from.min(text.len());
        if let Some(lit) = &prog.pure_literal {
            if !prog.icase {
                if lit.is_empty() {
                    return Some(Match { start: from, end: from });
                }
                let first = lit[0];
                let mut k = from;
                while k + lit.len() <= text.len() {
                    if text[k] == first && text[k + 1..k + lit.len()] == lit[1..] {
                        return Some(Match {
                            start: k,
                            end: k + lit.len(),
                        });
                    }
                    k += 1;
                }
                return None;
            }
        }
        if !prog.literal_prefix.is_empty() && !prog.icase {
            let needle = &prog.literal_prefix;
            let first = needle[0];
            let mut k = from;
            while k + needle.len() <= text.len() {
                if text[k] == first && text[k + 1..k + needle.len()] == needle[1..] {
                    if let Some(m) = self.scan(prog, text, k, search_start) {
                        return Some(m);
                    }
                }
                k += 1;
            }
            return None;
        }
        self.scan(prog, text, from, search_start)
    }

    fn scan(
        &mut self,
        prog: &Prog,
        text: &[char],
        from: usize,
        search_start: usize,
    ) -> Option<Match> {
        if prog.has_backref {
            return super::backtrack::find(prog, text, from, search_start);
        }
        if let Some(mut dfa) = self.dfa.take() {
            match dfa_scan(prog, &mut dfa, &mut self.scratch, text, from, search_start) {
                Ok(found) => {
                    self.dfa = Some(dfa);
                    return found;
                }
                Err(()) => {
                    // 状态超限：永久回退 NFA（本 searcher 生命周期内）。
                    self.dfa = None;
                }
            }
        }
        nfa_scan(prog, text, from, search_start)
    }
}

impl<'a> Searcher<'a> {
    pub fn new(prog: &'a Prog) -> Searcher<'a> {
        Searcher {
            prog,
            inner: Matcher::new(prog),
        }
    }

    pub fn find(&mut self, text: &[char], from: usize, search_start: usize) -> Option<Match> {
        self.inner.find(self.prog, text, from, search_start)
    }
}

pub fn find(prog: &Prog, text: &[char], from: usize) -> Option<Match> {
    find_from(prog, text, from, from)
}

/// `from` = 搜索起点；`search_start` = 锚点（`\A`）绑定的偏移。
pub fn find_from(
    prog: &Prog,
    text: &[char],
    from: usize,
    search_start: usize,
) -> Option<Match> {
    let mut s = Searcher::new(prog);
    s.find(text, from, search_start)
}

pub fn is_match(prog: &Prog, text: &[char], from: usize) -> bool {
    find(prog, text, from).is_some()
}

// ── DFA 扫描 ─────────────────────────────────────────────────────

fn dfa_scan(
    prog: &Prog,
    dfa: &mut DfaCache,
    scratch: &mut DfaScratch,
    text: &[char],
    from: usize,
    _search_start: usize,
) -> Result<Option<Match>, ()> {
    let shortest = prog.flags.has_shorter();
    let start_id = dfa.start();
    if scratch.stamp.len() < dfa.state_count() {
        scratch.stamp.resize(dfa.state_count(), 0);
        scratch.start_of.resize(dfa.state_count(), 0);
    }
    scratch.cur.clear();
    scratch.next.clear();
    let mut best: Option<Match> = None;

    for p in from..=text.len() {
        // 注入起点线程。若 start 状态仍在 cur 中，保留其更早起点
        // （DFA 状态合并后 start 状态可能由更早线程到达）。
        if best.map_or(true, |b| p < b.start) && inject_allowed(prog, text, p) {
            if scratch.start_of.len() < dfa.state_count() {
                scratch.stamp.resize(dfa.state_count(), 0);
                scratch.start_of.resize(dfa.state_count(), 0);
            }
            if !scratch.cur.contains(&start_id) {
                scratch.start_of[start_id as usize] = p;
                scratch.cur.push(start_id);
            }
        }
        if scratch.cur.is_empty() {
            continue;
        }

        for &id in &scratch.cur {
            if dfa.is_accept(id) {
                let st = scratch.start_of[id as usize];
                best = Some(match best {
                    None => Match { start: st, end: p },
                    Some(b) if st < b.start => Match { start: st, end: p },
                    Some(b) if st == b.start => {
                        if shortest {
                            b
                        } else {
                            Match {
                                start: b.start,
                                end: b.end.max(p),
                            }
                        }
                    }
                    Some(b) => b,
                });
            }
        }

        if p == text.len() {
            break;
        }
        let ch = text[p];
        let color = dfa.color(ch);
        let gen = scratch.next_gen();
        scratch.next.clear();
        for &id in &scratch.cur {
            let st = scratch.start_of[id as usize];
            if let Some(b) = best {
                if st > b.start || (shortest && st == b.start) {
                    continue;
                }
            }
            match dfa.step(prog, id, color)? {
                Some(ns) => {
                    if scratch.stamp.len() < dfa.state_count() {
                        scratch.stamp.resize(dfa.state_count(), 0);
                        scratch.start_of.resize(dfa.state_count(), 0);
                    }
                    let ni = ns as usize;
                    if scratch.stamp[ni] != gen {
                        scratch.stamp[ni] = gen;
                        scratch.start_of[ni] = st;
                        scratch.next.push(ns);
                    } else if st < scratch.start_of[ni] {
                        scratch.start_of[ni] = st;
                    }
                }
                None => {}
            }
        }
        scratch.cur.clear();
        std::mem::swap(&mut scratch.cur, &mut scratch.next);
        if let Some(b) = best {
            let useful = scratch
                .cur
                .iter()
                .any(|&id| scratch.start_of[id as usize] < b.start
                    || (!shortest && scratch.start_of[id as usize] == b.start));
            if !useful {
                break;
            }
        }
    }
    Ok(best)
}

fn inject_allowed(prog: &Prog, text: &[char], p: usize) -> bool {
    match &prog.first {
        None => true,
        Some(sets) => {
            if prog.nullable {
                true
            } else if p < text.len() {
                sets.iter().any(|s| s.matches(text[p], prog.icase))
            } else {
                false
            }
        }
    }
}

// ── NFA 回退扫描 ─────────────────────────────────────────────────

fn nfa_scan(prog: &Prog, text: &[char], from: usize, search_start: usize) -> Option<Match> {
    let shortest = prog.flags.has_shorter();
    let mut sc = Scanner::new(prog, text, search_start);
    let mut best: Option<Match> = None;
    let mut carried: Vec<(usize, usize)> = Vec::new();
    let mut seeds: Vec<(usize, usize)> = Vec::new();

    for p in from..=text.len() {
        seeds.clear();
        seeds.extend(carried.drain(..));
        if best.map_or(true, |b| p < b.start) && inject_allowed(prog, text, p) {
            seeds.push((prog.start, p));
        }
        if seeds.is_empty() {
            continue;
        }
        sc.closure(&seeds, p);
        for &s in &sc.cur {
            if s != prog.accept {
                continue;
            }
            let st = sc.start_of[s];
            best = Some(match best {
                None => Match { start: st, end: p },
                Some(b) if st < b.start => Match { start: st, end: p },
                Some(b) if st == b.start => {
                    if shortest {
                        b
                    } else {
                        Match {
                            start: b.start,
                            end: b.end.max(p),
                        }
                    }
                }
                Some(b) => b,
            });
        }
        if p == text.len() {
            break;
        }
        let ch = text[p];
        for &s in &sc.cur {
            let st = sc.start_of[s];
            if let Some(b) = best {
                if st > b.start || (shortest && st == b.start) {
                    continue;
                }
            }
            for t in &prog.states[s].trans {
                if let Trans::Char(set, to) = t {
                    if set.matches(ch, prog.icase) {
                        carried.push((*to, st));
                    }
                }
            }
        }
        if let Some(b) = best {
            let useful = carried
                .iter()
                .any(|&(_, st)| st < b.start || (!shortest && st == b.start));
            if !useful {
                break;
            }
        }
    }
    best
}

/// lookahead：子 Prog 是否在 `pos` 起有 anchored 匹配（不要求到串尾）。
pub(crate) fn matches_at(prog: &Prog, text: &[char], pos: usize, search_start: usize) -> bool {
    let mut sc = Scanner::new(prog, text, search_start);
    let mut seeds = vec![(prog.start, 0usize)];
    sc.closure(&seeds, pos);
    if sc.cur.contains(&prog.accept) {
        return true;
    }
    let mut p = pos;
    while p < text.len() {
        let ch = text[p];
        seeds.clear();
        for &s in &sc.cur {
            for t in &prog.states[s].trans {
                if let Trans::Char(set, to) = t {
                    if set.matches(ch, prog.icase) {
                        seeds.push((*to, 0));
                    }
                }
            }
        }
        p += 1;
        if seeds.is_empty() {
            return false;
        }
        sc.closure(&seeds, p);
        if sc.cur.contains(&prog.accept) {
            return true;
        }
    }
    false
}

struct Scanner<'a> {
    prog: &'a Prog,
    text: &'a [char],
    search_start: usize,
    stamp: Vec<u32>,
    start_of: Vec<usize>,
    gen: u32,
    stack: Vec<(usize, usize)>,
    pub cur: Vec<usize>,
}

impl<'a> Scanner<'a> {
    fn new(prog: &'a Prog, text: &'a [char], search_start: usize) -> Scanner<'a> {
        let n = prog.states.len();
        Scanner {
            prog,
            text,
            search_start,
            stamp: vec![0; n],
            start_of: vec![0; n],
            gen: 0,
            stack: Vec::new(),
            cur: Vec::new(),
        }
    }

    fn next_gen(&mut self) {
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.stamp.fill(0);
            self.gen = 1;
        }
    }

    fn closure(&mut self, seeds: &[(usize, usize)], pos: usize) {
        self.next_gen();
        self.cur.clear();
        self.stack.clear();
        self.stack.extend_from_slice(seeds);
        while let Some((s, start)) = self.stack.pop() {
            if self.stamp[s] != self.gen {
                self.stamp[s] = self.gen;
                self.start_of[s] = start;
                self.cur.push(s);
            } else if start < self.start_of[s] {
                self.start_of[s] = start;
            } else {
                continue;
            }
            for t in &self.prog.states[s].trans {
                match t {
                    Trans::Eps(to) => self.stack.push((*to, start)),
                    Trans::Assert(k, to) => {
                        if assert_ok(*k, pos, self.text, self.search_start) {
                            self.stack.push((*to, start));
                        }
                    }
                    Trans::Lacon { negative, prog, to } => {
                        let ok = matches_at(
                            &self.prog.lacons[*prog],
                            self.text,
                            pos,
                            self.search_start,
                        );
                        if ok != *negative {
                            self.stack.push((*to, start));
                        }
                    }
                    Trans::Char(_, _) => {}
                }
            }
        }
    }
}

fn is_word(c: char) -> bool {
    unicode::is_word(c)
}

fn word_at(text: &[char], pos: usize) -> bool {
    text.get(pos).map(|&c| is_word(c)).unwrap_or(false)
}

pub(crate) fn assert_ok(k: AssertKind, pos: usize, text: &[char], search_start: usize) -> bool {
    match k {
        // `^`（非 newline 模式）锚绝对串首；`\A` 锚 -start 偏移
        // （tclsh 把搜索起点当虚拟串首）。
        AssertKind::Bos => pos == 0,
        AssertKind::Bot => pos == search_start,
        AssertKind::Eos => pos == text.len(),
        AssertKind::Bol => pos == 0 || text.get(pos - 1) == Some(&'\n'),
        AssertKind::Eol => pos == text.len() || text.get(pos) == Some(&'\n'),
        AssertKind::WordStart => word_at(text, pos) && (pos == 0 || !word_at(text, pos - 1)),
        AssertKind::WordEnd => pos > 0 && word_at(text, pos - 1) && !word_at(text, pos),
        AssertKind::WordB => {
            let before = pos > 0 && word_at(text, pos - 1);
            let after = word_at(text, pos);
            before != after
        }
        AssertKind::NotWordB => {
            let before = pos > 0 && word_at(text, pos - 1);
            let after = word_at(text, pos);
            before == after
        }
    }
}

pub fn class_matches(set: &CharSet, c: char, icase: bool) -> bool {
    set.matches(c, icase)
}

pub fn root_flags(prog: &Prog) -> Flags {
    prog.flags
}
