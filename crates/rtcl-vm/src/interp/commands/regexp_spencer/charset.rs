//! 字符集/谓词（Spencer colormap 的语义替代）。
//!
//! 不复刻 byte-tree 颜色表（那只是性能结构）；每个字符类用
//! "范围 + Tcl Unicode 谓词" 表示，匹配时按成员关系判定。大小写
//! 不敏感用简单折叠（与既有引擎的 `İ→i` 口径一致）。

use crate::interp::unicode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pred {
    Digit,
    NotDigit,
    Space,
    NotSpace,
    Word,
    NotWord,
    Alnum,
    Alpha,
    Blank,
    Cntrl,
    Graph,
    Lower,
    Print,
    Punct,
    Upper,
    XDigit,
}

impl Pred {
    pub fn matches(self, c: char) -> bool {
        match self {
            Pred::Digit => unicode::is_digit(c),
            Pred::NotDigit => !unicode::is_digit(c),
            Pred::Space => unicode::is_space(c),
            Pred::NotSpace => !unicode::is_space(c),
            Pred::Word => unicode::is_word(c),
            Pred::NotWord => !unicode::is_word(c),
            Pred::Alnum => unicode::is_alnum(c),
            Pred::Alpha => unicode::is_alpha(c),
            Pred::Blank => unicode::is_blank(c),
            Pred::Cntrl => unicode::is_control(c),
            Pred::Graph => unicode::is_graph(c),
            Pred::Lower => c.is_lowercase(),
            Pred::Print => unicode::is_print_re(c),
            Pred::Punct => unicode::is_punct(c),
            Pred::Upper => c.is_uppercase(),
            Pred::XDigit => unicode::is_xdigit(c),
        }
    }

    /// POSIX 类名（小写）→ 谓词。
    pub fn from_posix(name: &str) -> Option<Pred> {
        Some(match name {
            "digit" => Pred::Digit,
            "space" => Pred::Space,
            "word" => Pred::Word,
            "alnum" => Pred::Alnum,
            "alpha" => Pred::Alpha,
            "blank" => Pred::Blank,
            "cntrl" => Pred::Cntrl,
            "graph" => Pred::Graph,
            "lower" => Pred::Lower,
            "print" => Pred::Print,
            "punct" => Pred::Punct,
            "upper" => Pred::Upper,
            "xdigit" => Pred::XDigit,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Range(u32, u32),
    Pred(Pred),
}

/// 字符类。`neg` 为整体否定；`no_newline` 对应 Tcl `REG_NEWLINE`
/// 下 `[^...]` 不匹配换行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharSet {
    pub neg: bool,
    pub no_newline: bool,
    pub items: Vec<Item>,
}

impl CharSet {
    pub fn any() -> CharSet {
        CharSet {
            neg: false,
            no_newline: false,
            items: vec![Item::Range(0, 0x10FFFF)],
        }
    }

    pub fn dot(newline_stop: bool) -> CharSet {
        CharSet {
            neg: false,
            no_newline: newline_stop,
            items: vec![Item::Range(0, 0x10FFFF)],
        }
    }

    pub fn from_char(c: char) -> CharSet {
        let u = c as u32;
        CharSet {
            neg: false,
            no_newline: false,
            items: vec![Item::Range(u, u)],
        }
    }

    pub fn matches(&self, c: char, icase: bool) -> bool {
        if self.no_newline && c == '\n' {
            return false;
        }
        let mut hit = self.items.iter().any(|it| item_matches(it, c));
        if !hit && icase {
            // 简单折叠：同时试单字符 lower/upper 形式。
            let mut variants = [None, None];
            let mut it = c.to_lowercase();
            variants[0] = it.next();
            let mut it = c.to_uppercase();
            variants[1] = it.next();
            for v in variants.into_iter().flatten() {
                if v != c && self.items.iter().any(|it| item_matches(it, v)) {
                    hit = true;
                    break;
                }
            }
        }
        if self.neg {
            !hit
        } else {
            hit
        }
    }
}

fn item_matches(it: &Item, c: char) -> bool {
    match it {
        Item::Range(lo, hi) => {
            let u = c as u32;
            *lo <= u && u <= *hi
        }
        Item::Pred(p) => p.matches(c),
    }
}
