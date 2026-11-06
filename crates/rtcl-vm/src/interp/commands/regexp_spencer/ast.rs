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
}

impl Flags {
    pub const NONE: Flags = Flags {
        longer: false,
        shorter: false,
        mixed: false,
        cap: false,
        backr: false,
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

    pub fn backref(subno: usize, min: u32, max: u32, qprefer: Flags) -> Node {
        let mut flags = Flags::BACKR;
        flags = Flags::combine(qprefer, flags);
        Node::new(
            NodeKind::Backref { subno, min, max },
            flags,
        )
    }

    pub fn no_record(node: Node) -> Node {
        let flags = node.flags;
        Node::new(NodeKind::NoRecord(Box::new(node)), flags)
    }

    pub fn look(negative: bool, node: Node) -> Node {
        let flags = node.flags;
        Node::new(
            NodeKind::Look {
                negative,
                node: Box::new(node),
            },
            flags,
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

/// 跳过零宽节点，取相邻原子词性；串边界视为 Unknown。
fn neighbor_wordness(parts: &[Node], from: usize, forward: bool) -> WordKind {
    let mut i = from;
    loop {
        let idx = if forward {
            if i >= parts.len() {
                return WordKind::Unknown;
            }
            i
        } else {
            if i == 0 {
                return WordKind::Unknown;
            }
            i - 1
        };
        let n = &parts[idx];
        if matches!(
            &n.kind,
            NodeKind::Empty | NodeKind::Assert(_) | NodeKind::Look { .. }
        ) {
            if forward {
                i += 1;
            } else {
                i -= 1;
            }
            continue;
        }
        return wordness(n);
    }
}

/// `\y \Y \m \M` 与相邻已知词性冲突 → 不可命中。
fn word_assert_impossible(k: AssertKind, prev: WordKind, next: WordKind) -> bool {
    match k {
        AssertKind::WordB => prev != WordKind::Unknown && prev == next,
        AssertKind::NotWordB => {
            prev != WordKind::Unknown
                && next != WordKind::Unknown
                && prev != next
        }
        AssertKind::WordStart => prev == WordKind::Word || next == WordKind::NonWord,
        AssertKind::WordEnd => prev == WordKind::NonWord || next == WordKind::Word,
        _ => false,
    }
}

fn is_end_anchor(k: AssertKind) -> bool {
    matches!(k, AssertKind::Eos | AssertKind::Eol)
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

/// 编译期不可命中分析（`regexp -about` 的 REG_UIMPOSSIBLE）：
/// 串首/行首约束前有消耗、串尾/行尾约束后有消耗，或所有分支都不可命中。
pub fn impossible(tree: &Node) -> bool {
    match &tree.kind {
        NodeKind::Concat(v) => {
            for (i, part) in v.iter().enumerate() {
                if impossible(part) {
                    return true;
                }
                if starts_with_start_anchor(part) && v[..i].iter().any(necessarily_consumes) {
                    return true;
                }
                if ends_with_end_anchor(part) && v[i + 1..].iter().any(necessarily_consumes) {
                    return true;
                }
                if let NodeKind::Assert(k) = part.kind {
                    if word_assert_impossible(
                        k,
                        neighbor_wordness(v, i, false),
                        neighbor_wordness(v, i + 1, true),
                    ) {
                        return true;
                    }
                }
            }
            false
        }
        NodeKind::Alt(v) => !v.is_empty() && v.iter().all(impossible),
        NodeKind::Repeat { node, min, .. } => *min > 0 && impossible(node),
        NodeKind::NoRecord(inner) | NodeKind::Capture { node: inner, .. } => {
            impossible(inner)
        }
        NodeKind::Look { node, .. } => impossible(node),
        _ => false,
    }
}
