//! 整体匹配（Spencer `regexec.c` 的 DFA 层语义）。
//!
//! 用 NFA 集合模拟实现 leftmost-longest / SHORTER：
//! * 默认：同一左端位置取最长可接受终点（POSIX leftmost-longest）；
//! * 根偏好含 SHORTER：取最短（Spencer `REG_USHORTEST`）。
//!
//! 捕获的 POSIX 消歧（dissect）在 M3 补上；本模块先保证整体 span。

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
    let from = from.min(text.len());
    for start in from..=text.len() {
        if let Some(end) = run_at(prog, text, start) {
            return Some(Match { start, end });
        }
    }
    None
}

/// `is_match` 语义（regexp 无捕获返回布尔）。
pub fn is_match(prog: &Prog, text: &[char], from: usize) -> bool {
    find(prog, text, from).is_some()
}

fn run_at(prog: &Prog, text: &[char], start: usize) -> Option<usize> {
    let shortest = prog.flags.has_shorter();
    let mut cur = closure(prog, &[prog.start], start, text);
    let mut best: Option<usize> = None;
    if cur.contains(&prog.accept) {
        best = Some(start);
        if shortest {
            return best;
        }
    }
    let mut pos = start;
    while pos < text.len() {
        let ch = text[pos];
        let mut seeds = Vec::new();
        for &s in &cur {
            for t in &prog.states[s].trans {
                if let Trans::Char(set, to) = t {
                    if set.matches(ch, prog.icase) {
                        seeds.push(*to);
                    }
                }
            }
        }
        pos += 1;
        if seeds.is_empty() {
            break;
        }
        cur = closure(prog, &seeds, pos, text);
        if cur.contains(&prog.accept) {
            best = Some(pos);
            if shortest {
                return best;
            }
        }
    }
    best
}

fn closure(prog: &Prog, seeds: &[usize], pos: usize, text: &[char]) -> Vec<usize> {
    let mut seen = vec![false; prog.states.len()];
    let mut stack: Vec<usize> = seeds.to_vec();
    let mut out = Vec::new();
    while let Some(s) = stack.pop() {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        out.push(s);
        for t in &prog.states[s].trans {
            match t {
                Trans::Eps(to) => stack.push(*to),
                Trans::Assert(k, to) => {
                    if assert_ok(*k, pos, text) {
                        stack.push(*to);
                    }
                }
                Trans::Char(_, _) => {}
            }
        }
    }
    out
}

fn is_word(c: char) -> bool {
    unicode::is_word(c)
}

fn word_at(text: &[char], pos: usize) -> bool {
    text.get(pos).map(|&c| is_word(c)).unwrap_or(false)
}

fn assert_ok(k: AssertKind, pos: usize, text: &[char]) -> bool {
    match k {
        AssertKind::Bos => pos == 0,
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
