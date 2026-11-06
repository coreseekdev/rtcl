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
    /// 串首（`\A`，或非 newline 敏感时的 `^`）。
    Bos,
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
