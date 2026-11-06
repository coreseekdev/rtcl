//! Tcl ARE 词法/语法（`regc_lex.c` + `regcomp.c` 的 parse 部分，Rust port）。
//!
//! 只构建 `subre` 树；NFA/DFA 与匹配在 `nfa.rs`/`matcher.rs`。错误
//! 消息与 `REG_*` 码按 tclsh 8.6.17 实测钉死（见 D-19 与
//! `specs/regex-engine-eval.md`）。

use super::ast::{AssertKind, Flags, Node, DUPINF, DUPMAX};
use super::charset::{CharSet, Item, Pred};
use super::Error;

/// 编译选项（对应 Tcl `cflags` 的可观察子集）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub icase: bool,
    /// `REG_NEWLINE`：`^ $ . [^]` 全部新行敏感（含 nlstop+nlanch）。
    pub newline: bool,
    /// `REG_NLSTOP`：`.` 与 `[^...]` 不匹配换行。
    pub nlstop: bool,
    /// `REG_NLANCH`：`^ $` 按行。
    pub nlanch: bool,
    /// `REG_EXPANDED`：忽略空白与 `#` 注释。
    pub expanded: bool,
    /// `REG_QUOTE`：`***=` 字面模式。
    pub quote: bool,
    /// `***:`/`(?e)`/`(?b)` 记录（M4 才完整区分 BRE/ERE 语法）。
    pub ere: bool,
    pub bre: bool,
}

impl Options {
    fn dot_stops_newline(self) -> bool {
        self.newline || self.nlstop
    }
    fn anchors_by_line(self) -> bool {
        self.newline || self.nlanch
    }
    fn class_stops_newline(self) -> bool {
        self.newline || self.nlstop
    }
}

#[derive(Debug)]
enum ClassAtom {
    Char(char),
    Pred(Pred),
}

pub struct Parser {
    chars: Vec<char>,
    i: usize,
    opts: Options,
    /// 已闭合的捕获组（1-based）。
    closed: Vec<bool>,
    /// 下一个捕获组编号。
    next_sub: usize,
    /// lookahead 嵌套层数：内部括号非捕获，回引非法。
    lacon: usize,
}

impl Parser {
    pub fn new(pattern: &str, opts: Options) -> Parser {
        Parser {
            chars: pattern.chars().collect(),
            i: 0,
            opts,
            closed: vec![false],
            next_sub: 1,
            lacon: 0,
        }
    }

    pub fn parse(mut self) -> Result<Node, Error> {
        self.prefixes()?;
        if self.opts.quote {
            let mut parts = Vec::new();
            while let Some(c) = self.peek() {
                self.i += 1;
                parts.push(Node::char(c as u32));
            }
            return Ok(Node::concat(parts));
        }
        let node = self.parse_alt()?;
        if self.i < self.chars.len() {
            // 顶层多余的 ')' 或未消费字符。
            return Err(Error::new(
                "REG_EPAREN",
                "parentheses () not balanced",
            ));
        }
        Ok(node)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn at(&self, off: usize) -> Option<char> {
        self.chars.get(self.i + off).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn err(code: &'static str, msg: &str) -> Error {
        Error::new(code, msg)
    }

    // ── 前缀：***= / ***: / 内嵌选项 ─────────────────────────────

    fn prefixes(&mut self) -> Result<(), Error> {
        // "***" 前缀。
        if self.at(0) == Some('*') && self.at(1) == Some('*') && self.at(2) == Some('*') {
            match self.at(3) {
                Some('?') => {
                    return Err(Self::err("REG_BADPAT", "invalid regexp (reg version 0.8)"));
                }
                Some('=') => {
                    self.opts.quote = true;
                    self.opts.expanded = false;
                    self.opts.newline = false;
                    self.i += 4;
                    return Ok(());
                }
                Some(':') => {
                    self.i += 4;
                }
                _ => {
                    return Err(Self::err("REG_BADRPT", "quantifier operand invalid"));
                }
            }
        }
        // 内嵌选项（仅 ARE）：`(?letters)`。
        if self.at(0) == Some('(')
            && self.at(1) == Some('?')
            && self.at(2).map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        {
            self.i += 2;
            while let Some(c) = self.peek() {
                if !c.is_ascii_alphabetic() {
                    break;
                }
                self.i += 1;
                match c {
                    'b' => {
                        self.opts.bre = true;
                        self.opts.ere = false;
                        self.opts.quote = false;
                    }
                    'c' => self.opts.icase = false,
                    'e' => {
                        self.opts.ere = true;
                        self.opts.bre = false;
                        self.opts.quote = false;
                    }
                    'i' => self.opts.icase = true,
                    'm' | 'n' => {
                        self.opts.newline = true;
                        self.opts.nlstop = true;
                        self.opts.nlanch = true;
                    }
                    'p' => {
                        self.opts.nlstop = true;
                        self.opts.nlanch = false;
                    }
                    'q' => {
                        self.opts.quote = true;
                        self.opts.expanded = false;
                        self.opts.newline = false;
                    }
                    's' => {
                        self.opts.newline = false;
                        self.opts.nlstop = false;
                        self.opts.nlanch = false;
                    }
                    't' => self.opts.expanded = false,
                    'w' => {
                        self.opts.nlstop = false;
                        self.opts.nlanch = true;
                    }
                    'x' => self.opts.expanded = true,
                    _ => {
                        return Err(Self::err("REG_BADOPT", "invalid embedded option"));
                    }
                }
            }
            if !self.eat(')') {
                return Err(Self::err("REG_BADOPT", "invalid embedded option"));
            }
        }
        Ok(())
    }

    fn skip_ws(&mut self) {
        if !self.opts.expanded {
            return;
        }
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => self.i += 1,
                Some('#') => {
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    // ── 分支/交替 ────────────────────────────────────────────────

    fn parse_alt(&mut self) -> Result<Node, Error> {
        let mut branches = vec![self.parse_branch()?];
        loop {
            self.skip_ws();
            if self.peek() == Some('|') {
                self.i += 1;
                branches.push(self.parse_branch()?);
            } else {
                break;
            }
        }
        Ok(Node::alt(branches))
    }

    fn parse_branch(&mut self) -> Result<Node, Error> {
        let mut parts = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => parts.push(self.parse_quantified()?),
            }
        }
        Ok(Node::concat(parts))
    }

    /// 一个原子 + 可选量词（Spencer `parseqatom`）。
    fn parse_quantified(&mut self) -> Result<Node, Error> {
        let atom = self.parse_atom()?;
        self.skip_ws();
        let (min, max, qprefer) = match self.peek() {
            Some('*') => {
                self.i += 1;
                (0, DUPINF, Flags::LONGER)
            }
            Some('+') => {
                self.i += 1;
                (1, DUPINF, Flags::LONGER)
            }
            Some('?') => {
                self.i += 1;
                (0, 1, Flags::LONGER)
            }
            Some('{') if self.at(1).map(|c| c.is_ascii_digit()).unwrap_or(false) => {
                let (m, n, comma) = self.scan_bound()?;
                let pref = if comma { Flags::LONGER } else { Flags::NONE };
                (m, n, pref)
            }
            _ => return Ok(atom),
        };
        // 惰性后缀。
        let qprefer = if self.peek() == Some('?') {
            self.i += 1;
            match qprefer {
                Flags::LONGER => Flags::SHORTER,
                other => other,
            }
        } else {
            qprefer
        };
        // 连续量词（a**）→ BADRPT。
        self.skip_ws();
        if matches!(self.peek(), Some('*') | Some('+') | Some('?'))
            || (self.peek() == Some('{')
                && self.at(1).map(|c| c.is_ascii_digit()).unwrap_or(false))
        {
            return Err(Self::err("REG_BADRPT", "quantifier operand invalid"));
        }
        if let super::ast::NodeKind::Backref { subno, .. } = atom.kind {
            // 量化回引：把 min/max 放进回引节点本身（Spencer 特例）。
            return Ok(Node::backref(subno, min, max, qprefer));
        }
        if min >= 1 {
            // Spencer 编译器改写（regcomp.c）：x{m,n}（m>0，无回引）
            // = x{m-1,n-1} x —— 最后一个 x 是独立的"最后迭代"，
            // 捕获取它（`(a*)+` 的组是尾部空迭代）。
            let pmax = if max >= super::ast::DUPINF {
                super::ast::DUPINF
            } else {
                max - 1
            };
            let prefix = Node::no_record(Node::repeat(atom.clone(), min - 1, pmax, qprefer));
            return Ok(Node::concat(vec![prefix, atom]));
        }
        Ok(Node::repeat(atom, min, max, qprefer))
    }

    /// `{m}` / `{m,}` / `{m,n}`；调用方已确认 `{` 后是数字。
    fn scan_bound(&mut self) -> Result<(u32, u32, bool), Error> {
        let bad = || Self::err("REG_BADBR", "invalid repetition count(s)");
        self.i += 1; // '{'
        let mut m = 0u32;
        while let Some(c) = self.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            m = m.saturating_mul(10).saturating_add(c as u32 - '0' as u32);
            self.i += 1;
        }
        let mut comma = false;
        let mut n = m;
        if self.eat(',') {
            comma = true;
            if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                let mut v = 0u32;
                while let Some(c) = self.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    v = v.saturating_mul(10).saturating_add(c as u32 - '0' as u32);
                    self.i += 1;
                }
                n = v;
            } else {
                n = DUPINF;
            }
        }
        if !self.eat('}') {
            return Err(bad());
        }
        if m > DUPMAX || n > DUPMAX || m > n {
            return Err(bad());
        }
        Ok((m, n, comma))
    }

    // ── 原子 ─────────────────────────────────────────────────────

    fn parse_atom(&mut self) -> Result<Node, Error> {
        let c = match self.peek() {
            Some(c) => c,
            None => return Ok(Node::empty()),
        };
        match c {
            '(' => self.parse_group(),
            '[' => self.parse_class(),
            '.' => {
                self.i += 1;
                Ok(Node::class(CharSet::dot(self.opts.dot_stops_newline())))
            }
            '^' => {
                self.i += 1;
                Ok(Node::assert(if self.opts.anchors_by_line() {
                    AssertKind::Bol
                } else {
                    AssertKind::Bos
                }))
            }
            '$' => {
                self.i += 1;
                Ok(Node::assert(if self.opts.anchors_by_line() {
                    AssertKind::Eol
                } else {
                    AssertKind::Eos
                }))
            }
            '\\' => {
                self.i += 1;
                self.parse_escape_atom()
            }
            '*' | '+' | '?' => Err(Self::err("REG_BADRPT", "quantifier operand invalid")),
            '{' if self.at(1).map(|c| c.is_ascii_digit()).unwrap_or(false) => {
                Err(Self::err("REG_BADRPT", "quantifier operand invalid"))
            }
            ')' => Err(Self::err("REG_EPAREN", "parentheses () not balanced")),
            _ => {
                self.i += 1;
                Ok(Node::char(c as u32))
            }
        }
    }

    fn parse_group(&mut self) -> Result<Node, Error> {
        self.i += 1; // '('
        if self.peek() == Some('?') {
            self.i += 1;
            match self.peek() {
                Some(':') => {
                    self.i += 1;
                    let inner = self.parse_alt()?;
                    if !self.eat(')') {
                        return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
                    }
                    Ok(inner)
                }
                Some('=') | Some('!') => {
                    let negative = self.peek() == Some('!');
                    self.i += 1;
                    self.lacon += 1;
                    let inner = self.parse_alt();
                    self.lacon -= 1;
                    let inner = inner?;
                    if !self.eat(')') {
                        return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
                    }
                    Ok(Node::look(negative, inner))
                }
                Some('<') => Err(Self::err("REG_BADRPT", "quantifier operand invalid")),
                _ => Err(Self::err("REG_BADOPT", "invalid embedded option")),
            }
        } else {
            let subno = self.next_sub;
            self.next_sub += 1;
            if self.lacon > 0 {
                // lookahead 内所有括号非捕获。
                let inner = self.parse_alt()?;
                if !self.eat(')') {
                    return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
                }
                return Ok(inner);
            }
            let inner = self.parse_alt()?;
            if !self.eat(')') {
                return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
            }
            if self.closed.len() <= subno {
                self.closed.resize(subno + 1, false);
            }
            self.closed[subno] = true;
            Ok(Node::capture(inner, subno))
        }
    }

    fn parse_escape_atom(&mut self) -> Result<Node, Error> {
        let c = match self.peek() {
            Some(c) => c,
            None => return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence")),
        };
        self.i += 1;
        match c {
            'd' => Ok(Node::class(CharSet::new(false, false, vec![Item::Pred(Pred::Digit)]))),
            'D' => Ok(Node::class(CharSet::new(false, false, vec![Item::Pred(Pred::NotDigit)]))),
            's' => Ok(Node::class(CharSet::new(false, false, vec![Item::Pred(Pred::Space)]))),
            'S' => Ok(Node::class(CharSet::new(false, false, vec![Item::Pred(Pred::NotSpace)]))),
            'w' => Ok(Node::class(CharSet::new(false, false, vec![Item::Pred(Pred::Word)]))),
            'W' => Ok(Node::class(CharSet::new(false, false, vec![Item::Pred(Pred::NotWord)]))),
            'm' => Ok(Node::assert(AssertKind::WordStart)),
            'M' => Ok(Node::assert(AssertKind::WordEnd)),
            'y' => Ok(Node::assert(AssertKind::WordB)),
            'Y' => Ok(Node::assert(AssertKind::NotWordB)),
            'A' => Ok(Node::assert(AssertKind::Bos)),
            'Z' => Ok(Node::assert(AssertKind::Eos)),
            'z' => Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence")),
            'b' => Ok(Node::char(0x08)),
            'B' => Ok(Node::char('\\' as u32)),
            'a' => Ok(Node::char(0x07)),
            'e' => Ok(Node::char(0x1b)),
            'f' => Ok(Node::char(0x0c)),
            'n' => Ok(Node::char('\n' as u32)),
            'r' => Ok(Node::char('\r' as u32)),
            't' => Ok(Node::char('\t' as u32)),
            'v' => Ok(Node::char(0x0b)),
            'c' => {
                let x = self.peek().ok_or_else(|| {
                    Self::err("REG_EESCAPE", "invalid escape \\ sequence")
                })?;
                self.i += 1;
                Ok(Node::char(x as u32 & 0x1f))
            }
            'u' | 'U' | 'x' => self.hex_escape_atom(c),
            '0'..='9' => self.digit_escape_atom(),
            c if c.is_ascii_alphanumeric() => {
                Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"))
            }
            c => Ok(Node::char(c as u32)),
        }
    }

    fn hex_escape_atom(&mut self, kind: char) -> Result<Node, Error> {
        let max = match kind {
            'u' => 4,
            'U' => 8,
            _ => 2,
        };
        if kind == 'x' && self.peek() == Some('{') {
            return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"));
        }
        let mut v = 0u32;
        let mut n = 0;
        while n < max {
            match self.peek().and_then(|c| c.to_digit(16)) {
                Some(d) => {
                    v = v * 16 + d;
                    self.i += 1;
                    n += 1;
                }
                None => break,
            }
        }
        if n == 0 {
            return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"));
        }
        Ok(Node::char(v))
    }

    /// 八进制/反向引用启发式（Spencer）：整个数字串作为一个数。
    fn digit_escape_atom(&mut self) -> Result<Node, Error> {
        // 首个数字已消费。
        let first = self.chars[self.i - 1];
        let mut run = String::new();
        run.push(first);
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                run.push(c);
                self.i += 1;
            } else {
                break;
            }
        }
        let digits: Vec<char> = run.chars().collect();
        let mut val = 0usize;
        let mut overflow = false;
        for c in &digits {
            val = val.saturating_mul(10).saturating_add(*c as usize - '0' as usize);
            if val == usize::MAX {
                overflow = true;
            }
        }
        if first != '0' {
            let single = digits.len() == 1;
            let is_backref = single || (!overflow && val >= 1 && val <= self.closed.len() - 1);
            if is_backref {
                if self.lacon > 0 {
                    return Err(Self::err(
                        "REG_ESUBREG",
                        "invalid backreference number",
                    ));
                }
                let closed = self
                    .closed
                    .get(val)
                    .copied()
                    .unwrap_or(false);
                if !closed {
                    return Err(Self::err(
                        "REG_ESUBREG",
                        "invalid backreference number",
                    ));
                }
                return Ok(Node::backref(val, 1, 1, Flags::NONE));
            }
            if !(first as u8).is_ascii_octal() {
                return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"));
            }
        }
        // 八进制：最多 3 位（0-7），其余字面。
        let mut oct = 0u32;
        let mut used = 0;
        for c in &digits {
            if used >= 3 || !(*c as u8).is_ascii_octal() {
                break;
            }
            oct = oct * 8 + (*c as u32 - '0' as u32);
            used += 1;
        }
        let mut parts = vec![Node::char(oct)];
        for c in &digits[used..] {
            parts.push(Node::char(*c as u32));
        }
        Ok(Node::concat(parts))
    }

    // ── 字符类 ───────────────────────────────────────────────────

    fn parse_class(&mut self) -> Result<Node, Error> {
        self.i += 1; // '['
        let neg = self.eat('^');
        let mut items: Vec<Item> = Vec::new();
        // 首个 ']' 是字面。
        if self.peek() == Some(']') {
            items.push(Item::Range(']' as u32, ']' as u32));
            self.i += 1;
        }
        loop {
            match self.peek() {
                None => return Err(Self::err("REG_EBRACK", "brackets [] not balanced")),
                Some(']') => {
                    self.i += 1;
                    break;
                }
                Some('[') if matches!(self.at(1), Some(':') | Some('.') | Some('=')) => {
                    if let Some(item) = self.parse_posix_or_collate()? {
                        items.push(item);
                    }
                    continue;
                }
                _ => {}
            }
            let lo = self.parse_class_char()?;
            // 范围：Char '-' Char（'-' 在末尾/开头时是字面）。
            let lo_char = match lo {
                ClassAtom::Char(c) => Some(c),
                ClassAtom::Pred(_) => None,
            };
            if let Some(lo_c) = lo_char {
                if self.peek() == Some('-')
                    && self.at(1).is_some()
                    && self.at(1) != Some(']')
                {
                    self.i += 1; // '-'
                    match self.parse_class_char()? {
                        ClassAtom::Char(hi_c) => {
                            if (lo_c as u32) > (hi_c as u32) {
                                return Err(Self::err(
                                    "REG_ERANGE",
                                    "invalid range in brackets",
                                ));
                            }
                            items.push(Item::Range(lo_c as u32, hi_c as u32));
                        }
                        ClassAtom::Pred(p) => {
                            // Tcl 对 [a-\d] 的处理：'-' 与谓词分别字面/谓词。
                            items.push(Item::Range(lo_c as u32, lo_c as u32));
                            items.push(Item::Range('-' as u32, '-' as u32));
                            items.push(Item::Pred(p));
                        }
                    }
                    continue;
                }
            }
            match lo {
                ClassAtom::Char(c) => items.push(Item::Range(c as u32, c as u32)),
                ClassAtom::Pred(p) => items.push(Item::Pred(p)),
            }
        }
        Ok(Node::class(CharSet::new(
            neg,
            neg && self.opts.class_stops_newline(),
            items,
        )))
    }

    fn parse_posix_or_collate(&mut self) -> Result<Option<Item>, Error> {
        self.i += 1; // '['
        let kind = self.peek().unwrap(); // ':' '.' '='
        self.i += 1;
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c == kind && self.at(1) == Some(']') {
                self.i += 2;
                return match kind {
                    ':' => match Pred::from_posix(&name) {
                        Some(p) => Ok(Some(Item::Pred(p))),
                        None => Err(Self::err("REG_ECTYPE", "invalid character class")),
                    },
                    _ => {
                        // 排序/等价元素：取首个字符作为字面（M4 细化）。
                        match name.chars().next() {
                            Some(c) => Ok(Some(Item::Range(c as u32, c as u32))),
                            None => Ok(None),
                        }
                    }
                };
            }
            name.push(c);
            self.i += 1;
        }
        // 未闭合：把 '[' 当字面处理。
        Err(Self::err("REG_EBRACK", "brackets [] not balanced"))
    }

    fn parse_class_char(&mut self) -> Result<ClassAtom, Error> {
        let c = match self.peek() {
            Some(c) => c,
            None => return Err(Self::err("REG_EBRACK", "brackets [] not balanced")),
        };
        self.i += 1;
        if c != '\\' {
            return Ok(ClassAtom::Char(c));
        }
        let e = match self.peek() {
            Some(e) => e,
            None => return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence")),
        };
        self.i += 1;
        match e {
            'd' => Ok(ClassAtom::Pred(Pred::Digit)),
            'D' => Ok(ClassAtom::Pred(Pred::NotDigit)),
            's' => Ok(ClassAtom::Pred(Pred::Space)),
            'S' => Ok(ClassAtom::Pred(Pred::NotSpace)),
            'w' => Ok(ClassAtom::Pred(Pred::Word)),
            'W' => Ok(ClassAtom::Pred(Pred::NotWord)),
            'b' => Ok(ClassAtom::Char('\u{8}')),
            'B' => Ok(ClassAtom::Char('\\')),
            'a' => Ok(ClassAtom::Char('\u{7}')),
            'e' => Ok(ClassAtom::Char('\u{1b}')),
            'f' => Ok(ClassAtom::Char('\u{c}')),
            'n' => Ok(ClassAtom::Char('\n')),
            'r' => Ok(ClassAtom::Char('\r')),
            't' => Ok(ClassAtom::Char('\t')),
            'v' => Ok(ClassAtom::Char('\u{b}')),
            'c' => {
                let x = self.peek().ok_or_else(|| {
                    Self::err("REG_EESCAPE", "invalid escape \\ sequence")
                })?;
                self.i += 1;
                Ok(ClassAtom::Char(char::from_u32(x as u32 & 0x1f).unwrap_or('\0')))
            }
            'u' | 'U' | 'x' => {
                let max = match e {
                    'u' => 4,
                    'U' => 8,
                    _ => 2,
                };
                if e == 'x' && self.peek() == Some('{') {
                    return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"));
                }
                let mut v = 0u32;
                let mut n = 0;
                while n < max {
                    match self.peek().and_then(|c| c.to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            self.i += 1;
                            n += 1;
                        }
                        None => break,
                    }
                }
                if n == 0 {
                    return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"));
                }
                Ok(ClassAtom::Char(char::from_u32(v).unwrap_or('\u{FFFD}')))
            }
            '0'..='9' => {
                // 类内：单个非零数字非法（回引）；多位按八进制（首位
                // 必须 0-7）；见 D-19。
                let first = e;
                let mut run = vec![first];
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        run.push(c);
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                if first != '0'
                    && (run.len() == 1 || !(first as u8).is_ascii_octal())
                {
                    return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"));
                }
                let mut oct = 0u32;
                let mut used = 0;
                for c in &run {
                    if used >= 3 || !(*c as u8).is_ascii_octal() {
                        break;
                    }
                    oct = oct * 8 + (*c as u32 - '0' as u32);
                    used += 1;
                }
                // 剩余数字作为字面：这里只能返回一个 atom；多字符的
                // 八进制尾巴在类内极罕见，取第一个字符（M4 再细化）。
                Ok(ClassAtom::Char(char::from_u32(oct).unwrap_or('\0')))
            }
            'm' | 'M' | 'y' | 'Y' | 'A' | 'Z' | 'z' => {
                Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"))
            }
            c if c.is_ascii_alphanumeric() => {
                Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence"))
            }
            c => Ok(ClassAtom::Char(c)),
        }
    }
}

trait AsciiOctal {
    fn is_ascii_octal(self) -> bool;
}

impl AsciiOctal for u8 {
    fn is_ascii_octal(self) -> bool {
        (b'0'..=b'7').contains(&self)
    }
}

/// 解析入口。
pub fn parse(pattern: &str, opts: Options) -> Result<Node, Error> {
    Parser::new(pattern, opts).parse()
}
