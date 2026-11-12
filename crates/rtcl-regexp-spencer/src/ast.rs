//! Spencer `subre` 树（Rust port）。
//!
//! 节点操作符对齐 `regguts.h`：`=`（普通/DFA 节点）、`b`（反向引用）、
//! `(`（捕获）、`.`（连接）、`|`（交替）、`*`（迭代）。偏好位
//! `LONGER/SHORTER/MIXED` 与 `COMBINE/UP` 宏逐位复刻——它们决定
//! 全局 `REG_USHORTEST`（惰性量词 → 整体最短）与 dissect 的拆分序。

use super::charset::CharSet;

/// Spencer `DUPMAX`：bounded quantifier 上限。
pub const DUPMAX: u32 = 255;
/// `DUPINF`：无限迭代哨兵。
pub const DUPINF: u32 = DUPMAX + 1;

/// 子表达式偏好位（`subre.flags` 的子集）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub longer: bool,
    pub shorter: bool,
    pub mixed: bool,
    pub cap: bool,
    pub backr: bool,
    /// 来自 `[...]` 字符类（区别于 `.` / `\d` 等内部 Class 节点），
    /// 供 -about 的 UUNPORT 范围检测使用。
    pub bracket: bool,
}

impl Flags {
    pub const NONE: Flags = Flags {
        longer: false,
        shorter: false,
        mixed: false,
        cap: false,
        backr: false,
        bracket: false,
    };
    pub const LONGER: Flags = Flags {
        longer: true,
        ..Flags::NONE
    };
    pub const SHORTER: Flags = Flags {
        shorter: true,
        ..Flags::NONE
    };
    pub const CAP: Flags = Flags {
        cap: true,
        ..Flags::NONE
    };
    pub const BACKR: Flags = Flags {
        backr: true,
        ..Flags::NONE
    };
    pub const BRACKET: Flags = Flags {
        bracket: true,
        ..Flags::NONE
    };

    fn pref_bits(self) -> u8 {
        (if self.longer { 1 } else { 0 }) | (if self.shorter { 2 } else { 0 })
    }

    /// `UP(f) = (f & ~NOPROP) | (LMIX(f) & SMIX(f) & MIXED)`：
    /// 清掉偏好位；同时有 LONGER+SHORTER 时记 MIXED。
    fn up(self) -> Flags {
        Flags {
            longer: false,
            shorter: false,
            mixed: self.mixed || (self.longer && self.shorter),
            cap: self.cap,
            backr: self.backr,
            bracket: self.bracket,
        }
    }

    /// `COMBINE(f1, f2) = UP(f1|f2) | PREF2(f1,f2)`。
    pub fn combine(a: Flags, b: Flags) -> Flags {
        let both = Flags {
            longer: a.longer || b.longer,
            shorter: a.shorter || b.shorter,
            mixed: a.mixed || b.mixed,
            cap: a.cap || b.cap,
            backr: a.backr || b.backr,
            bracket: a.bracket || b.bracket,
        }
        .up();
        let pref = if a.pref_bits() != 0 {
            a.pref_bits()
        } else {
            b.pref_bits()
        };
        Flags {
            longer: pref & 1 != 0,
            shorter: pref & 2 != 0,
            ..both
        }
    }

    pub fn has_shorter(self) -> bool {
        self.shorter
    }
}

/// 零宽断言。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertKind {
    /// 真正的串首（非 newline 敏感时的 `^`）：绝对位置 0，
    /// `-start` 不为 0 时不可能命中。
    Bos,
    /// 有效串首（`\A`）：绑定 `-start` 偏移——tclsh 把搜索
    /// 起点当虚拟串首，`regexp -start 1 {\Aa} ba` 命中 1。
    Bot,
    /// 串尾（`\Z`，或非 newline 敏感时的 `$`）。
    Eos,
    /// 行首。
    Bol,
    /// 行尾。
    Eol,
    /// `\m`：词首。
    WordStart,
    /// `\M`：词尾。
    WordEnd,
    /// `\y`：词边界。
    WordB,
    /// `\Y`：非词边界。
    NotWordB,
}

/// 树节点（`subre`）。
#[derive(Clone, Debug)]
pub struct Node {
    pub flags: Flags,
    pub kind: NodeKind,
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Empty,
    Char(u32),
    Class(CharSet),
    Assert(AssertKind),
    Concat(Vec<Node>),
    /// Spencer 的 `|` 是右结合链；这里用 N 元 Vec，语义等价。
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: u32,
    },
    Capture {
        subno: usize,
        node: Box<Node>,
    },
    Backref {
        subno: usize,
        min: u32,
        max: u32,
        /// 被引用捕获文本的首/末字符词性集合（-about 词约束分析）。
        first_word: u8,
        last_word: u8,
    },
    /// 前向断言（`(?=)` / `(?!)`）。内部捕获按 Tcl 规则视为非捕获。
    Look {
        negative: bool,
        node: Box<Node>,
    },
    /// 只匹配、不记录任何捕获（Spencer 编译器 `x{m-1,n-1}` 前缀等价
    /// 的扁平节点；dissect 时 record=false）。
    NoRecord(Box<Node>),
}

impl Node {
    pub fn new(kind: NodeKind, flags: Flags) -> Node {
        Node { flags, kind }
    }

    pub fn empty() -> Node {
        Node::new(NodeKind::Empty, Flags::NONE)
    }

    pub fn char(c: u32) -> Node {
        Node::new(NodeKind::Char(c), Flags::NONE)
    }

    pub fn class(set: CharSet) -> Node {
        Node::new(NodeKind::Class(set), Flags::NONE)
    }

    pub fn assert(kind: AssertKind) -> Node {
        Node::new(NodeKind::Assert(kind), Flags::NONE)
    }

    pub fn concat(mut parts: Vec<Node>) -> Node {
        if parts.is_empty() {
            return Node::empty();
        }
        let mut flags = Flags::NONE;
        for p in &parts {
            flags = Flags::combine(flags, p.flags);
        }
        if parts.len() == 1 {
            return parts.pop().unwrap();
        }
        Node::new(NodeKind::Concat(parts), flags)
    }

    pub fn alt(mut branches: Vec<Node>) -> Node {
        let mut flags = Flags::LONGER;
        for b in &branches {
            flags = Flags::combine(flags, b.flags);
        }
        if branches.len() == 1 {
            return branches.pop().unwrap();
        }
        Node::new(NodeKind::Alt(branches), flags)
    }

    /// 迭代节点：`qprefer` 来自量词（贪婪 LONGER / 惰性 SHORTER /
    /// `{m}` 为 NONE），按 `COMBINE(qprefer, operand.flags)` 合并。
    pub fn repeat(node: Node, min: u32, max: u32, qprefer: Flags) -> Node {
        let flags = Flags::combine(qprefer, node.flags);
        Node::new(
            NodeKind::Repeat {
                node: Box::new(node),
                min,
                max,
            },
            flags,
        )
    }

    pub fn capture(node: Node, subno: usize) -> Node {
        let flags = Flags::combine(Flags::CAP, node.flags);
        Node::new(
            NodeKind::Capture {
                subno,
                node: Box::new(node),
            },
            flags,
        )
    }

    pub fn backref(
        subno: usize,
        min: u32,
        max: u32,
        qprefer: Flags,
        first_word: u8,
        last_word: u8,
    ) -> Node {
        let mut flags = Flags::BACKR;
        flags = Flags::combine(qprefer, flags);
        Node::new(
            NodeKind::Backref {
                subno,
                min,
                max,
                first_word,
                last_word,
            },
            flags,
        )
    }

    pub fn no_record(node: Node) -> Node {
        let flags = node.flags;
        Node::new(NodeKind::NoRecord(Box::new(node)), flags)
    }

    pub fn look(negative: bool, node: Node) -> Node {
        // 环视是零宽约束：内部子式的偏好位不得传播到外层
        //（否则 `(?=..)[^ab]*?` 的 SHORTER 会被内层 LONGER 覆盖）。
        Node::new(
            NodeKind::Look {
                negative,
                node: Box::new(node),
            },
            Flags::NONE,
        )
    }
}

/// 编译期空匹配分析（`regexp -about` 的 REG_UEMPTYMATCH 口径）：
/// 约束算空；lookahead 不算（tclsh 对 `(?=a)` 不置位）；回引按所
/// 引用组是否可空（`(a*)\1` 可空，`(a)\1` 不可空）。
pub fn nullable(tree: &Node) -> bool {
    let mut groups: Vec<Option<bool>> = Vec::new();
    nullable_node(tree, &mut groups)
}

fn nullable_node(n: &Node, groups: &mut Vec<Option<bool>>) -> bool {
    match &n.kind {
        NodeKind::Empty | NodeKind::Assert(_) => true,
        NodeKind::Char(_) | NodeKind::Class(_) => false,
        // lookahead 是约束，但 tclsh 的 UEMPTYMATCH 不把它算作
        // 空匹配来源。
        NodeKind::Look { .. } => false,
        NodeKind::NoRecord(inner) => nullable_node(inner, groups),
        NodeKind::Capture { subno, node } => {
            let v = nullable_node(node, groups);
            if groups.len() <= *subno {
                groups.resize(subno + 1, None);
            }
            let slot = &mut groups[*subno];
            *slot = Some(slot.unwrap_or(false) || v);
            v
        }
        NodeKind::Backref { subno, min, .. } => {
            *min == 0 || groups.get(*subno).copied().flatten().unwrap_or(false)
        }
        NodeKind::Concat(v) => v.iter().all(|n| nullable_node(n, groups)),
        NodeKind::Alt(v) => {
            let mut any = false;
            for b in v {
                if nullable_node(b, groups) {
                    any = true;
                }
            }
            any
        }
        NodeKind::Repeat { node, min, .. } => *min == 0 || nullable_node(node, groups),
    }
}

/// 节点是否**必然**消耗至少一个字符（-about UIMPOSSIBLE 分析用；
/// `a*` 可为空所以不算，`a+` 算）。
fn necessarily_consumes(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Empty | NodeKind::Assert(_) | NodeKind::Look { .. } => false,
        NodeKind::Char(_) | NodeKind::Class(_) => true,
        NodeKind::Backref { min, .. } => *min > 0,
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            necessarily_consumes(inner)
        }
        NodeKind::Concat(v) => v.iter().any(necessarily_consumes),
        NodeKind::Alt(v) => !v.is_empty() && v.iter().all(necessarily_consumes),
        NodeKind::Repeat { node, min, .. } => *min > 0 && necessarily_consumes(node),
    }
}

fn is_start_anchor(k: AssertKind) -> bool {
    matches!(k, AssertKind::Bos | AssertKind::Bot | AssertKind::Bol)
}

/// 相邻原子的"词性"（`\y \m \M` 的编译期不可命中分析）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum WordKind {
    Word,
    NonWord,
    Unknown,
}

fn char_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn wordness(n: &Node) -> WordKind {
    match &n.kind {
        NodeKind::Char(c) => match char::from_u32(*c) {
            Some(c) if char_word(c) => WordKind::Word,
            Some(_) => WordKind::NonWord,
            None => WordKind::Unknown,
        },
        NodeKind::Class(set) => {
            let (mut any_word, mut any_nonword) = (false, false);
            for b in 0u8..128 {
                if set.matches(b as char, false) {
                    if char_word(b as char) {
                        any_word = true;
                    } else {
                        any_nonword = true;
                    }
                }
            }
            match (any_word, any_nonword) {
                (true, false) => WordKind::Word,
                (false, true) => WordKind::NonWord,
                _ => WordKind::Unknown,
            }
        }
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => wordness(inner),
        NodeKind::Repeat { node, min, .. } => {
            if *min > 0 {
                wordness(node)
            } else {
                WordKind::Unknown
            }
        }
        NodeKind::Concat(v) => v.last().map(wordness).unwrap_or(WordKind::Unknown),
        NodeKind::Alt(v) => match v.first() {
            Some(first) if v.iter().all(|b| wordness(b) == wordness(first)) => wordness(first),
            _ => WordKind::Unknown,
        },
        _ => WordKind::Unknown,
    }
}

/// 捕获组首/末字符的词性集合（parser 在闭合组时记录，供回引分析）。
pub(crate) fn word_bounds(n: &Node) -> (u8, u8) {
    (first_wordness_if_consumes(n), last_wordness_if_consumes(n))
}

const WS_WORD: u8 = 1;
const WS_NON: u8 = 2;
const WS_ANY: u8 = 3;

fn word_mask(k: WordKind) -> u8 {
    match k {
        WordKind::Word => WS_WORD,
        WordKind::NonWord => WS_NON,
        WordKind::Unknown => WS_ANY,
    }
}

/// 该节点一旦消耗字符，**最后一个**字符的词性集合。
pub(crate) fn last_wordness_if_consumes(n: &Node) -> u8 {
    match &n.kind {
        NodeKind::Char(_) | NodeKind::Class(_) => word_mask(wordness(n)),
        NodeKind::Backref { last_word, .. } => *last_word,
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            last_wordness_if_consumes(inner)
        }
        NodeKind::Concat(v) => {
            let mut acc = 0u8;
            for p in v.iter().rev() {
                if !can_consume(p) {
                    continue;
                }
                acc |= last_wordness_if_consumes(p);
                if necessarily_consumes(p) {
                    return acc;
                }
            }
            acc
        }
        NodeKind::Alt(v) => v.iter().map(last_wordness_if_consumes).fold(0, |a, b| a | b),
        NodeKind::Repeat { node, max, .. } => {
            if *max > 0 {
                last_wordness_if_consumes(node)
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// 该节点一旦消耗字符，**第一个**字符的词性集合。
pub(crate) fn first_wordness_if_consumes(n: &Node) -> u8 {
    match &n.kind {
        NodeKind::Char(_) | NodeKind::Class(_) => word_mask(wordness(n)),
        NodeKind::Backref { first_word, .. } => *first_word,
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            first_wordness_if_consumes(inner)
        }
        NodeKind::Concat(v) => {
            let mut acc = 0u8;
            for p in v {
                if !can_consume(p) {
                    continue;
                }
                acc |= first_wordness_if_consumes(p);
                if necessarily_consumes(p) {
                    return acc;
                }
            }
            acc
        }
        NodeKind::Alt(v) => v.iter().map(first_wordness_if_consumes).fold(0, |a, b| a | b),
        NodeKind::Repeat { node, max, .. } => {
            if *max > 0 {
                first_wordness_if_consumes(node)
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// 位置前一个字符的候选词性集合（可选节点可按“跳过”或“消耗”展开）。
fn prev_word_candidates(parts: &[Node], end: usize) -> u8 {
    let mut acc = 0u8;
    for p in parts[..end].iter().rev() {
        if !can_consume(p) {
            continue;
        }
        acc |= last_wordness_if_consumes(p);
        if necessarily_consumes(p) {
            return acc;
        }
    }
    // 全部可跳过时，前一个字符来自匹配之外的上下文（任一可能）。
    acc | WS_ANY
}

fn next_word_candidates(parts: &[Node], start: usize) -> u8 {
    let mut acc = 0u8;
    for p in &parts[start..] {
        if !can_consume(p) {
            continue;
        }
        acc |= first_wordness_if_consumes(p);
        if necessarily_consumes(p) {
            return acc;
        }
    }
    acc | WS_ANY
}

/// 收集同一位置上的零宽约束（穿透分组；可选重复/环视不强制）。
fn collect_zero_width_constraints(
    n: &Node,
    prev_fixed: &mut bool,
    next_fixed: &mut bool,
    asserts: &mut Vec<AssertKind>,
) {
    match &n.kind {
        NodeKind::Assert(k) => match k {
            AssertKind::Bos | AssertKind::Bot | AssertKind::Bol => *prev_fixed = true,
            AssertKind::Eos | AssertKind::Eol => *next_fixed = true,
            AssertKind::WordB
            | AssertKind::NotWordB
            | AssertKind::WordStart
            | AssertKind::WordEnd => asserts.push(*k),
        },
        NodeKind::Empty | NodeKind::Look { .. } => {}
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            collect_zero_width_constraints(inner, prev_fixed, next_fixed, asserts)
        }
        NodeKind::Concat(v) => {
            for p in v {
                collect_zero_width_constraints(p, prev_fixed, next_fixed, asserts);
            }
        }
        NodeKind::Repeat { node, min, .. } if *min > 0 => {
            collect_zero_width_constraints(node, prev_fixed, next_fixed, asserts)
        }
        _ => {}
    }
}

fn word_assert_holds(k: AssertKind, prev: u8, next: u8) -> bool {
    match k {
        AssertKind::WordB => prev != next,
        AssertKind::NotWordB => prev == next,
        AssertKind::WordStart => prev == WS_NON && next == WS_WORD,
        AssertKind::WordEnd => prev == WS_WORD && next == WS_NON,
        _ => true,
    }
}

fn word_run_feasible(asserts: &[AssertKind], prev_set: u8, next_set: u8) -> bool {
    for prev in [WS_WORD, WS_NON] {
        if prev_set & prev == 0 {
            continue;
        }
        for next in [WS_WORD, WS_NON] {
            if next_set & next == 0 {
                continue;
            }
            if asserts.iter().all(|k| word_assert_holds(*k, prev, next)) {
                return true;
            }
        }
    }
    false
}

fn is_end_anchor(k: AssertKind) -> bool {
    matches!(k, AssertKind::Eos | AssertKind::Eol)
}

/// 节点是否存在消耗字符的路径（可选重复也算）。
fn can_consume(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Empty | NodeKind::Assert(_) | NodeKind::Look { .. } => false,
        NodeKind::Char(_) | NodeKind::Class(_) | NodeKind::Backref { .. } => true,
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => can_consume(inner),
        NodeKind::Concat(v) => v.iter().any(can_consume),
        NodeKind::Alt(v) => v.iter().any(can_consume),
        NodeKind::Repeat { node, max, .. } => *max > 0 && can_consume(node),
    }
}

/// 节点是否必定以串/行首约束开始（穿透空节点与分组）。
fn starts_with_start_anchor(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Assert(k) => is_start_anchor(*k),
        NodeKind::Empty => false,
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            starts_with_start_anchor(inner)
        }
        NodeKind::Concat(v) => {
            for part in v {
                if starts_with_start_anchor(part) {
                    return true;
                }
                if necessarily_consumes(part) || !nullable(part) {
                    return false;
                }
            }
            false
        }
        NodeKind::Alt(v) => !v.is_empty() && v.iter().all(starts_with_start_anchor),
        NodeKind::Repeat { node, min, .. } => *min > 0 && starts_with_start_anchor(node),
        _ => false,
    }
}

/// 节点是否必定以串/行尾约束结束。
fn ends_with_end_anchor(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Assert(k) => is_end_anchor(*k),
        NodeKind::Empty => false,
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            ends_with_end_anchor(inner)
        }
        NodeKind::Concat(v) => {
            for part in v.iter().rev() {
                if ends_with_end_anchor(part) {
                    return true;
                }
                if necessarily_consumes(part) || !nullable(part) {
                    return false;
                }
            }
            false
        }
        NodeKind::Alt(v) => !v.is_empty() && v.iter().all(ends_with_end_anchor),
        NodeKind::Repeat { node, min, .. } => *min > 0 && ends_with_end_anchor(node),
        _ => false,
    }
}

/// Concat 是否被串首/串尾约束强制成空匹配：串首约束之前只有可跳过
/// 节点，串首与串尾之间没有可消耗节点（`^...$` 路径只能为空）。
fn concat_forced_empty(parts: &[Node]) -> bool {
    for s in 0..parts.len() {
        if !matches!(
            &parts[s].kind,
            NodeKind::Assert(AssertKind::Bos | AssertKind::Bot)
        ) {
            continue;
        }
        // 串首约束前若必然消耗，模式另有 impossible 判定。
        if parts[..s].iter().any(necessarily_consumes) {
            continue;
        }
        let Some(e) = (s + 1..parts.len()).find(|&i| {
            matches!(&parts[i].kind, NodeKind::Assert(AssertKind::Eos))
        }) else {
            continue;
        };
        if !parts[s + 1..e].iter().any(can_consume) {
            return true;
        }
    }
    false
}

/// 编译期不可命中分析（`regexp -about` 的 REG_UIMPOSSIBLE）：
/// 串首/行首约束前有消耗、串尾/行尾约束后有消耗、`^...$` 强制空串下
/// 的词约束，或所有分支都不可命中。`empty` 表示当前处于强制空匹配
/// 上下文（不能消耗任何字符）。
pub fn impossible(tree: &Node) -> bool {
    impossible_ctx(tree, false) || word_solver_says_impossible(tree)
}

fn impossible_ctx(n: &Node, empty: bool) -> bool {
    // 强制空串上下文里不能出现任何必然消耗的节点。
    if empty && necessarily_consumes(n) {
        return true;
    }
    match &n.kind {
        NodeKind::Assert(k) => {
            empty
                && matches!(
                    k,
                    AssertKind::WordB | AssertKind::WordStart | AssertKind::WordEnd
                )
        }
        NodeKind::Concat(v) => {
            let forced = concat_forced_empty(v);
            let child_empty = empty || forced;
            for (i, part) in v.iter().enumerate() {
                if impossible_ctx(part, child_empty) {
                    return true;
                }
                if starts_with_start_anchor(part) && v[..i].iter().any(necessarily_consumes) {
                    return true;
                }
                if ends_with_end_anchor(part) && v[i + 1..].iter().any(necessarily_consumes) {
                    return true;
                }
            }
            if !child_empty {
                // 同一位置上的零宽约束（\y \Y \m \M + 锚点）联立
                // 求解：前/后字符词性候选集合中不存在满足全部约束的
                // 组合 → 不可命中。
                let mut i = 0usize;
                while i < v.len() {
                    if can_consume(&v[i]) {
                        i += 1;
                        continue;
                    }
                    let start = i;
                    let mut asserts = Vec::new();
                    let mut prev_fixed = false;
                    let mut next_fixed = false;
                    while i < v.len() && !can_consume(&v[i]) {
                        collect_zero_width_constraints(
                            &v[i],
                            &mut prev_fixed,
                            &mut next_fixed,
                            &mut asserts,
                        );
                        i += 1;
                    }
                    if !asserts.is_empty() {
                        let mut prev_set = prev_word_candidates(v, start);
                        let mut next_set = next_word_candidates(v, i);
                        if prev_fixed {
                            prev_set = WS_NON;
                        }
                        if next_fixed {
                            next_set = WS_NON;
                        }
                        if !word_run_feasible(&asserts, prev_set, next_set) {
                            return true;
                        }
                    }
                }
            }
            false
        }
        NodeKind::Alt(v) => !v.is_empty() && v.iter().all(|b| impossible_ctx(b, empty)),
        NodeKind::Repeat { node, min, .. } => *min > 0 && impossible_ctx(node, empty),
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            impossible_ctx(inner, empty)
        }
        // lookahead 内部消耗不占匹配长度；负环视在空串上还可成立。
        NodeKind::Look { .. } => false,
        _ => false,
    }
}

/// 空路径上所有必执行零宽约束（忽略可消耗节点）是否可同时成立：
/// 空匹配时全部消耗节点都跳过，词约束落在同一位置。
fn nullable_path_constraints_feasible(v: &[Node]) -> bool {
    let mut asserts = Vec::new();
    let mut prev_fixed = false;
    let mut next_fixed = false;
    for p in v {
        collect_zero_width_constraints(p, &mut prev_fixed, &mut next_fixed, &mut asserts);
    }
    if asserts.is_empty() {
        return true;
    }
    let prev_set = if prev_fixed { WS_NON } else { WS_ANY };
    let next_set = if next_fixed { WS_NON } else { WS_ANY };
    word_run_feasible(&asserts, prev_set, next_set)
}

/// `-about` 的 REG_UEMPTYMATCH 口径：结构可空且该空路径不是
/// 不可命中的（剔除 word 约束在 `^...$` 强制空串下必然失败的路径）。
pub fn nullable_matchable(tree: &Node) -> bool {
    if impossible(tree) {
        return false;
    }
    let mut groups: Vec<Option<bool>> = Vec::new();
    nullable_m(tree, &mut groups)
}

/// 空路径上必然经过、且在空串处必然失败的词约束。
fn mandatory_empty_word_fails(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Assert(
            AssertKind::WordB | AssertKind::WordStart | AssertKind::WordEnd,
        ) => true,
        NodeKind::Concat(v) => v.iter().any(mandatory_empty_word_fails),
        NodeKind::Alt(v) => !v.is_empty() && v.iter().all(mandatory_empty_word_fails),
        NodeKind::Repeat { node, min, .. } => *min > 0 && mandatory_empty_word_fails(node),
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            mandatory_empty_word_fails(inner)
        }
        _ => false,
    }
}

fn concat_has_abs_start_and_eos(v: &[Node]) -> bool {
    v.iter()
        .any(|n| matches!(&n.kind, NodeKind::Assert(AssertKind::Bos | AssertKind::Bot)))
        && v.iter()
            .any(|n| matches!(&n.kind, NodeKind::Assert(AssertKind::Eos)))
}

fn nullable_m(n: &Node, groups: &mut Vec<Option<bool>>) -> bool {
    if impossible_ctx(n, false) {
        return false;
    }
    match &n.kind {
        NodeKind::Empty | NodeKind::Assert(_) => true,
        NodeKind::Char(_) | NodeKind::Class(_) => false,
        NodeKind::Look { .. } => false,
        NodeKind::NoRecord(inner) => nullable_m(inner, groups),
        NodeKind::Capture { subno, node } => {
            let v = nullable_m(node, groups);
            if groups.len() <= *subno {
                groups.resize(subno + 1, None);
            }
            let slot = &mut groups[*subno];
            *slot = Some(slot.unwrap_or(false) || v);
            v
        }
        NodeKind::Backref { subno, min, .. } => {
            *min == 0 || groups.get(*subno).copied().flatten().unwrap_or(false)
        }
        NodeKind::Concat(v) => {
            let all = v.iter().all(|p| nullable_m(p, groups));
            if all && !nullable_path_constraints_feasible(v) {
                return false;
            }
            if all
                && concat_has_abs_start_and_eos(v)
                && !v.iter().any(necessarily_consumes)
                && v.iter().any(mandatory_empty_word_fails)
            {
                return false;
            }
            all
        }
        NodeKind::Alt(v) => {
            let mut any = false;
            for b in v {
                if nullable_m(b, groups) {
                    any = true;
                }
            }
            any
        }
        NodeKind::Repeat { node, min, .. } => *min == 0 || nullable_m(node, groups),
    }
}

/// `regexp -about` 的 REG_UUNPORT：字符类里存在真正的范围
/// （`[a-c]`；`[ab]`/`[a\-c]` 不算，后者由方言决定）。
pub fn has_bracket_range(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Class(set) => {
            n.flags.bracket
                && set.items.iter().any(|it| match it {
                    super::charset::Item::Range(lo, hi) => lo < hi,
                    _ => false,
                })
        }
        NodeKind::Concat(v) | NodeKind::Alt(v) => v.iter().any(has_bracket_range),
        NodeKind::Repeat { node, .. }
        | NodeKind::NoRecord(node)
        | NodeKind::Capture { node, .. }
        | NodeKind::Look { node, .. } => has_bracket_range(node),
        _ => false,
    }
}

// ── 词性序列可满足性（-about REG_UIMPOSSIBLE 的补充证明）───────────
//
// 把 AST 的“字符词性语言”建成小 NFA（字母表 {W,N}），再把
// `\m \M \y \Y ^ $ \A \Z` 编码为边界谓词，搜索是否存在一条词性
// 序列满足全部约束。字符类/回引按**过近似**处理（允许更多词性），
// 因此“无见证 ⇒ 必然不可匹配”是可靠的；找到见证则不做结论。

const WB_W: u8 = 1;
const WB_N: u8 = 2;
const WB_ANY: u8 = 3;
const WB_MAX_STATES: usize = 8192;

#[derive(Clone, Copy)]
enum WTrans {
    Eps(usize),
    Sym(u8, usize),
    Assert(AssertKind, usize),
}

struct WBuilder {
    states: Vec<Vec<WTrans>>,
    overflow: bool,
}

impl WBuilder {
    fn new_state(&mut self) -> Option<usize> {
        if self.states.len() >= WB_MAX_STATES {
            self.overflow = true;
            return None;
        }
        let id = self.states.len();
        self.states.push(Vec::new());
        Some(id)
    }

    fn push(&mut self, st: usize, t: WTrans) {
        self.states[st].push(t);
    }

    fn compile(&mut self, node: &Node, next: usize) -> Option<usize> {
        if self.overflow {
            return None;
        }
        match &node.kind {
            NodeKind::Empty => Some(next),
            NodeKind::Char(c) => {
                let c = char::from_u32(*c)?;
                let mask = if char_word(c) { WB_W } else { WB_N };
                let st = self.new_state()?;
                self.push(st, WTrans::Sym(mask, next));
                Some(st)
            }
            NodeKind::Class(set) => {
                let mask = charset_word_mask(set);
                let st = self.new_state()?;
                self.push(st, WTrans::Sym(mask, next));
                Some(st)
            }
            NodeKind::Assert(k) => {
                let st = self.new_state()?;
                self.push(st, WTrans::Assert(*k, next));
                Some(st)
            }
            NodeKind::Concat(v) => {
                let mut cur = next;
                for p in v.iter().rev() {
                    cur = self.compile(p, cur)?;
                }
                Some(cur)
            }
            NodeKind::Alt(v) => {
                let st = self.new_state()?;
                for b in v {
                    let entry = self.compile(b, next)?;
                    self.push(st, WTrans::Eps(entry));
                }
                Some(st)
            }
            NodeKind::Repeat { node, min, max } => {
                self.compile_repeat(node, *min, *max, next)
            }
            NodeKind::Capture { node, .. } | NodeKind::NoRecord(node) => {
                self.compile(node, next)
            }
            // 环视不消耗字符：忽略其内部约束（过近似）。
            NodeKind::Look { .. } => Some(next),
            // 回引：按任意长度的任意词性序列过近似（捕获文本可空）。
            NodeKind::Backref { .. } => {
                let loop_st = self.new_state()?;
                self.push(loop_st, WTrans::Eps(next));
                self.push(loop_st, WTrans::Sym(WB_ANY, loop_st));
                Some(loop_st)
            }
        }
    }

    fn compile_repeat(
        &mut self,
        node: &Node,
        min: u32,
        max: u32,
        next: usize,
    ) -> Option<usize> {
        if max >= DUPINF {
            let loop_st = self.new_state()?;
            let body = self.compile(node, loop_st)?;
            self.push(loop_st, WTrans::Eps(body));
            self.push(loop_st, WTrans::Eps(next));
            let mut entry = loop_st;
            for _ in 0..min {
                entry = self.compile(node, entry)?;
            }
            Some(entry)
        } else {
            let mut cur = next;
            for _ in 0..min {
                cur = self.compile(node, cur)?;
            }
            for _ in 0..(max - min) {
                let body = self.compile(node, cur)?;
                let st = self.new_state()?;
                self.push(st, WTrans::Eps(body));
                self.push(st, WTrans::Eps(cur));
                cur = st;
            }
            Some(cur)
        }
    }
}

/// 字符类可能的词性集合（**过近似**：不确定时返回 {W,N}）。
fn charset_word_mask(set: &CharSet) -> u8 {
    if set.neg {
        return WB_ANY;
    }
    let mut mask = 0u8;
    for it in &set.items {
        let m = match it {
            super::charset::Item::Range(lo, hi) if lo == hi => {
                match char::from_u32(*lo) {
                    Some(c) if char_word(c) => WB_W,
                    Some(_) => WB_N,
                    None => WB_ANY,
                }
            }
            // 多字符范围可能跨越词/非词，保守取全集。
            super::charset::Item::Range(_, _) => WB_ANY,
            super::charset::Item::Pred(p) => pred_word_mask(*p),
        };
        mask |= m;
    }
    if mask == 0 {
        // 实际不匹配任何字符（例如类为空）：给一个死符号。
        return 0;
    }
    mask
}

fn pred_word_mask(p: super::charset::Pred) -> u8 {
    use super::charset::Pred::*;
    match p {
        Word | Alnum | Alpha | Lower | Upper | Digit | XDigit => WB_W,
        NotWord | Space | Blank | Punct | Cntrl => WB_N,
        NotDigit | NotSpace | Graph | Print => WB_ANY,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct WState {
    node: u32,
    prev: u8,
    consumed: bool,
    /// 后续字符必须允许的词性（3=无约束，0=不允许任何字符）。
    next_mask: u8,
    /// 已经过 $/\Z：后面不允许再消耗字符。
    at_end: bool,
}

fn word_solver_says_impossible(tree: &Node) -> bool {
    let mut b = WBuilder {
        states: Vec::new(),
        overflow: false,
    };
    let accept = match b.new_state() {
        Some(s) => s,
        None => return false,
    };
    let start = match b.compile(tree, accept) {
        Some(s) => s,
        None => return false,
    };
    if b.overflow {
        return false;
    }

    // 匹配起点前的字符词性未知（可在词/非词之后开始）；串首锚点
    // 会通过 prev==N 约束排除词后起点。
    let mut stack = vec![
        WState {
            node: start as u32,
            prev: WB_N,
            consumed: false,
            next_mask: WB_ANY,
            at_end: false,
        },
        WState {
            node: start as u32,
            prev: WB_W,
            consumed: false,
            next_mask: WB_ANY,
            at_end: false,
        },
    ];
    let mut seen: std::collections::HashSet<WState> = std::collections::HashSet::new();
    while let Some(st) = stack.pop() {
        if !seen.insert(st) {
            continue;
        }
        let n = st.node as usize;
        if n == accept {
            // 未经过串尾锚点时，匹配后的下一个字符词性未知（W/N
            // 均可）；经过 $/\Z 时只能是串尾虚拟非词。
            let ok = if st.at_end {
                st.next_mask & WB_N != 0
            } else {
                st.next_mask != 0
            };
            if ok {
                return false;
            }
            continue;
        }
        for t in &b.states[n] {
            match t {
                WTrans::Eps(to) => stack.push(WState {
                    node: *to as u32,
                    ..st
                }),
                WTrans::Sym(mask, to) => {
                    if st.at_end {
                        continue;
                    }
                    for w in [WB_W, WB_N] {
                        if mask & w == 0 {
                            continue;
                        }
                        if st.next_mask & w == 0 {
                            continue;
                        }
                        stack.push(WState {
                            node: *to as u32,
                            prev: w,
                            consumed: true,
                            next_mask: WB_ANY,
                            at_end: false,
                        });
                    }
                }
                WTrans::Assert(k, to) => {
                    let mut ns = st;
                    let ok = match k {
                        AssertKind::Bos | AssertKind::Bot => {
                            !st.consumed && st.prev == WB_N
                        }
                        AssertKind::Bol => st.prev == WB_N,
                        AssertKind::Eos => {
                            ns.at_end = true;
                            true
                        }
                        AssertKind::Eol => {
                            ns.next_mask &= WB_N;
                            true
                        }
                        AssertKind::WordStart => {
                            if st.prev != WB_N {
                                false
                            } else {
                                ns.next_mask &= WB_W;
                                true
                            }
                        }
                        AssertKind::WordEnd => {
                            if st.prev != WB_W {
                                false
                            } else {
                                ns.next_mask &= WB_N;
                                true
                            }
                        }
                        AssertKind::WordB => {
                            ns.next_mask &= if st.prev == WB_W { WB_N } else { WB_W };
                            true
                        }
                        AssertKind::NotWordB => {
                            ns.next_mask &= st.prev;
                            true
                        }
                    };
                    if ok && ns.next_mask != 0 {
                        ns.node = *to as u32;
                        stack.push(ns);
                    }
                }
            }
        }
    }
    true
}
