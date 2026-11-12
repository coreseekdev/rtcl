//! 字符集/谓词（Spencer colormap 的语义替代）。
//!
//! 不复刻 byte-tree 颜色表（那只是性能结构）；每个字符类用
//! "范围 + Tcl Unicode 谓词" 表示，匹配时按成员关系判定。大小写
//! 不敏感用简单折叠（与既有引擎的 `İ→i` 口径一致）。

use rtcl_unicode as unicode;

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
    /// ASCII 0..127 的成员位图（性能结构：常见文本热路径免逐项扫描）。
    ascii: [u64; 4],
}

impl CharSet {
    pub fn new(neg: bool, no_newline: bool, items: Vec<Item>) -> CharSet {
        let mut ascii = [0u64; 4];
        for u in 0..128u32 {
            let c = char::from_u32(u).unwrap();
            if items.iter().any(|it| item_matches(it, c)) {
                ascii[(u >> 6) as usize] |= 1u64 << (u & 63);
            }
        }
        CharSet {
            neg,
            no_newline,
            items,
            ascii,
        }
    }

    pub fn any() -> CharSet {
        CharSet::new(false, false, vec![Item::Range(0, 0x10FFFF)])
    }

    pub fn dot(newline_stop: bool) -> CharSet {
        CharSet::new(false, newline_stop, vec![Item::Range(0, 0x10FFFF)])
    }

    pub fn from_char(c: char) -> CharSet {
        let u = c as u32;
        CharSet::new(false, false, vec![Item::Range(u, u)])
    }

    pub fn matches(&self, c: char, icase: bool) -> bool {
        if self.no_newline && c == '\n' {
            return false;
        }
        let u = c as u32;
        if u < 128 {
            let mut hit = self.ascii[(u >> 6) as usize] & (1u64 << (u & 63)) != 0;
            if !hit && icase {
                for v in fold_variants(c) {
                    if v != c && self.ascii[((v as u32) >> 6) as usize]
                        & (1u64 << ((v as u32) & 63))
                        != 0
                    {
                        hit = true;
                        break;
                    }
                }
            }
            return if self.neg { !hit } else { hit };
        }
        let mut hit = self.items.iter().any(|it| item_matches(it, c));
        if !hit && icase {
            // 简单折叠：同时试单字符 lower/upper 形式。
            for v in fold_variants(c) {
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

fn fold_variants(c: char) -> impl Iterator<Item = char> {
    let lo = c.to_lowercase().next();
    let up = c.to_uppercase().next();
    [lo, up].into_iter().flatten()
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
