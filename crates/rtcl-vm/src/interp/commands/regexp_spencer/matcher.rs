//! 整体匹配（Spencer `regexec.c` 的 DFA 层语义）。
//!
//! 性能结构（Rust 版）：
//! * **单趟扫描**：所有起点线程同时推进，每个 NFA 状态只保留最小起点
//!   （leftmost 优先）；不再对每个起点重启模拟（旧版 O(n²·m)）。
//! * **零分配热路径**：bitset/代际标记 + 复用的状态/种子/栈 Vec；
//!   每个位置只做一次 epsilon/断言闭包。
//! * **ASCII 位图**：`CharSet` 对 0..127 用 4×u64 成员表，常见文本
//!   不做逐项谓词扫描。
//!
//! 语义：默认 POSIX leftmost-longest；根偏好含 SHORTER（任一非贪婪
//! 量词）时整体最短。捕获 dissect（M3）在后续层实现。

use super::ast::{AssertKind, Flags};
use super::charset::CharSet;
use super::nfa::{Prog, Trans};
use crate::interp::unicode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
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
    let from = from.min(text.len());
    if let Some(lit) = &prog.pure_literal {
        if prog.icase {
            // 大小写不敏感：回退 NFA（字面折叠由 CharSet 处理）。
        } else if lit.is_empty() {
            return Some(Match {
                start: from,
                end: from,
            });
        } else {
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
        // 字面前缀预过滤：匹配必须从该前缀开始。
        let needle = &prog.literal_prefix;
        let first = needle[0];
        let mut k = from;
        while k + needle.len() <= text.len() {
            // 首字符过滤 + 剩余比较（Vec<char> 无 memchr，先做廉价首字符）。
            if text[k] == first && text[k + 1..k + needle.len()] == needle[1..] {
                if let Some(m) = scan(prog, text, k, search_start) {
                    return Some(m);
                }
            }
            k += 1;
        }
        return None;
    }
    scan(prog, text, from, search_start)
}

fn scan(prog: &Prog, text: &[char], from: usize, search_start: usize) -> Option<Match> {
    let shortest = prog.flags.has_shorter();
    let mut sc = Scanner::new(prog, text, search_start);
    let mut best: Option<Match> = None;
    let mut carried: Vec<(usize, usize)> = Vec::new();
    let mut seeds: Vec<(usize, usize)> = Vec::new();

    for p in from..=text.len() {
        seeds.clear();
        seeds.extend(carried.drain(..));
        // 新起点线程：一旦已有匹配，后续起点不可能更左。
        if best.map_or(true, |b| p < b.start) {
            let inject = match &prog.first {
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
            };
            if inject {
                seeds.push((prog.start, p));
            }
        }
        if seeds.is_empty() {
            continue;
        }
        sc.closure(&seeds, p);

        // 接受：leftmost 优先；同起点按全局偏好取最长/最短。
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
        // 推进一个字符：丢弃不可能改善 leftmost 的线程。
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

/// `is_match` 语义（regexp 无捕获返回布尔）。
pub fn is_match(prog: &Prog, text: &[char], from: usize) -> bool {
    find(prog, text, from).is_some()
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

    /// epsilon/断言闭包。同一状态保留**最小起点**；起点变小时重新展开
    /// 其出弧（下游可能因此拿到更左的起点）。
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

fn assert_ok(k: AssertKind, pos: usize, text: &[char], search_start: usize) -> bool {
    match k {
        // D-21：\A 绑定 -start 偏移（搜索起点），不是绝对串首。
        AssertKind::Bos => pos == search_start,
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

/// 供后续 dissect 复用的字符集入口。
pub fn class_matches(set: &CharSet, c: char, icase: bool) -> bool {
    set.matches(c, icase)
}

/// 根偏好（调试/测试）。
pub fn root_flags(prog: &Prog) -> Flags {
    prog.flags
}
