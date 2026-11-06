//! Tcl ARE 词法/语法（`regc_lex.c` + `regcomp.c` 的 parse 部分，Rust port）。
//!
//! 只构建 `subre` 树；NFA/DFA 与匹配在 `nfa.rs`/`matcher.rs`。错误
//! 消息与 `REG_*` 码按 tclsh 8.6.17 实测钉死（见 D-19 与
//! `specs/regex-engine-eval.md`）。

use super::ast::{AssertKind, Flags, Node, NodeKind, DUPINF, DUPMAX};
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

/// 正则方言：默认 ARE；`(?e)` 切 POSIX ERE，`(?b)` 切 POSIX BRE。
/// 方言在整条 pattern 内固定（tclsh 只识别 pattern 开头的切换）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Are,
    Ere,
    Bre,
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
    /// ERE 的分组深度：POSIX ERE 里未匹配的 `)` 是普通字符
    /// （tclsh 8.6.17 实测），只有深度 > 0 时才结束分支。
    depth: usize,
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
            depth: 0,
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

    fn mode(&self) -> Mode {
        if self.opts.bre {
            Mode::Bre
        } else if self.opts.ere {
            Mode::Ere
        } else {
            Mode::Are
        }
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
        // Expanded 是 ARE 扩展；BRE/ERE 里空白与 # 都是普通字符。
        if !self.opts.expanded || self.mode() != Mode::Are {
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
        // BRE 没有交替：`|` 是普通字符。
        if self.mode() == Mode::Bre {
            return self.parse_branch();
        }
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
        let bre = self.mode() == Mode::Bre;
        let mut parts = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some('|') if !bre => break,
                Some(')') if !bre && (self.mode() != Mode::Ere || self.depth > 0) => break,
                // BRE 的子表达式以 `\)` 结束。
                Some('\\') if bre && self.at(1) == Some(')') => break,
                _ => {}
            }
            // BRE 的 ^ 只在 RE/子表达式开头是约束；$ 只在结尾是约束。
            if bre && parts.is_empty() && self.peek() == Some('^') {
                self.i += 1;
                parts.push(Node::assert(if self.opts.anchors_by_line() {
                    AssertKind::Bol
                } else {
                    AssertKind::Bos
                }));
                continue;
            }
            if bre && self.peek() == Some('$') && self.bre_dollar_at_end() {
                self.i += 1;
                parts.push(Node::assert(if self.opts.anchors_by_line() {
                    AssertKind::Eol
                } else {
                    AssertKind::Eos
                }));
                continue;
            }
            parts.push(self.parse_quantified()?);
        }
        Ok(Node::concat(parts))
    }

    /// BRE 中 `$` 后面是 RE 结尾或子表达式结尾（`\)`）时才是锚点。
    fn bre_dollar_at_end(&self) -> bool {
        match self.at(1) {
            None => true,
            Some('\\') => self.at(2) == Some(')'),
            _ => false,
        }
    }

    /// 一个原子 + 可选量词（Spencer `parseqatom`）。
    fn parse_quantified(&mut self) -> Result<Node, Error> {
        let atom = self.parse_atom()?;
        self.skip_ws();
        let mode = self.mode();
        // 约束（^ $ \A \Z \y ... 与 lookahead）不得带量词。
        let begins_quantifier = match mode {
            Mode::Bre => {
                self.peek() == Some('*')
                    || (self.peek() == Some('\\') && self.at(1) == Some('{'))
            }
            _ => {
                matches!(self.peek(), Some('*') | Some('+') | Some('?'))
                    || (self.peek() == Some('{')
                        && self.at(1).map(|c| c.is_ascii_digit()).unwrap_or(false))
            }
        };
        if begins_quantifier
            && matches!(
                atom.kind,
                super::ast::NodeKind::Assert(_) | super::ast::NodeKind::Look { .. }
            )
        {
            return Err(Self::err("REG_BADRPT", "quantifier operand invalid"));
        }
        let (min, max, qprefer) = match mode {
            Mode::Bre => {
                if self.peek() == Some('*') {
                    self.i += 1;
                    (0, DUPINF, Flags::LONGER)
                } else if self.peek() == Some('\\') && self.at(1) == Some('{') {
                    let (m, n, comma) = self.scan_bre_bound()?;
                    let pref = if comma { Flags::LONGER } else { Flags::NONE };
                    (m, n, pref)
                } else {
                    return Ok(atom);
                }
            }
            Mode::Ere => match self.peek() {
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
            },
            Mode::Are => match self.peek() {
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
            },
        };
        // 惰性后缀（ARE 专有；ERE 里 `a+?` 是 BADRPT，BRE 里 ? 是字面）。
        let qprefer = if mode == Mode::Are && self.peek() == Some('?') {
            self.i += 1;
            match qprefer {
                Flags::LONGER => Flags::SHORTER,
                other => other,
            }
        } else {
            qprefer
        };
        // 连续量词（a** / a+? / a\{2\}\{3\}）→ BADRPT。
        self.skip_ws();
        let consecutive = match mode {
            Mode::Bre => {
                self.peek() == Some('*')
                    || (self.peek() == Some('\\') && self.at(1) == Some('{'))
            }
            _ => {
                matches!(self.peek(), Some('*') | Some('+') | Some('?'))
                    || (self.peek() == Some('{')
                        && self.at(1).map(|c| c.is_ascii_digit()).unwrap_or(false))
            }
        };
        if consecutive {
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
        let mut unbounded = false;
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
                unbounded = true;
            }
        }
        if !self.eat('}') {
            // 到串尾仍无 `}` → EBRACE；其他杂字符 → BADBR。
            if self.peek().is_none() {
                return Err(Self::err("REG_EBRACE", "braces {} not balanced"));
            }
            return Err(bad());
        }
        if m > DUPMAX || (!unbounded && n > DUPMAX) || m > n {
            return Err(bad());
        }
        Ok((m, n, comma))
    }

    /// BRE 边界 `\{m\}` / `\{m,\}` / `\{,n\}` / `\{m,n\}`（`\{` 已确认）。
    fn scan_bre_bound(&mut self) -> Result<(u32, u32, bool), Error> {
        let bad = || Self::err("REG_BADBR", "invalid repetition count(s)");
        self.i += 2; // '\{'
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
        let mut unbounded = false;
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
                unbounded = true;
            }
        }
        // 结束符 `\}`；缺 `}` 是 EBRACE，其他杂字符是 BADBR
        //（tclsh 实测：`a\{2` → braces {} not balanced）。
        if self.peek() == Some('\\') && self.at(1) == Some('}') {
            self.i += 2;
        } else if self.peek().is_none() {
            return Err(Self::err("REG_EBRACE", "braces {} not balanced"));
        } else {
            return Err(bad());
        }
        if m > DUPMAX || (!unbounded && n > DUPMAX) || m > n {
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
        match self.mode() {
            Mode::Bre => return self.parse_atom_bre(c),
            Mode::Ere => return self.parse_atom_ere(c),
            Mode::Are => {}
        }
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

    /// POSIX ERE：括号/交替/量词与 ARE 相同，但没有 ARE 扩展。
    fn parse_atom_ere(&mut self, c: char) -> Result<Node, Error> {
        match c {
            '(' => self.parse_ere_group(),
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
                self.parse_quote_escape()
            }
            '*' | '+' | '?' => Err(Self::err("REG_BADRPT", "quantifier operand invalid")),
            '{' if self.at(1).map(|c| c.is_ascii_digit()).unwrap_or(false) => {
                Err(Self::err("REG_BADRPT", "quantifier operand invalid"))
            }
            ')' if self.depth == 0 => {
                // POSIX ERE：没有对应 `(` 的 `)` 是普通字符。
                self.i += 1;
                Ok(Node::char(')' as u32))
            }
            ')' => Err(Self::err("REG_EPAREN", "parentheses () not balanced")),
            _ => {
                self.i += 1;
                Ok(Node::char(c as u32))
            }
        }
    }

    /// ERE 的 `\x`：反斜杠只取消下一个字符的特殊含义（`\1` 是字面
    /// `1`，`\A` 是字面 `A`；ERE 无回引/Perl 类/约束转义）。
    fn parse_quote_escape(&mut self) -> Result<Node, Error> {
        match self.peek() {
            Some(c) => {
                self.i += 1;
                Ok(Node::char(c as u32))
            }
            None => Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence")),
        }
    }

    fn parse_ere_group(&mut self) -> Result<Node, Error> {
        self.i += 1; // '('
        // ERE 没有 `(?:` / `(?=` / `(?i` 等扩展：`?` 紧跟 `(` 是
        // 没有操作数的量词（tclsh 报 quantifier operand invalid）。
        if self.peek() == Some('?') {
            return Err(Self::err("REG_BADRPT", "quantifier operand invalid"));
        }
        let subno = self.next_sub;
        self.next_sub += 1;
        self.depth += 1;
        let inner = self.parse_alt();
        self.depth -= 1;
        let inner = inner?;
        if !self.eat(')') {
            return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
        }
        if self.closed.len() <= subno {
            self.closed.resize(subno + 1, false);
        }
        self.closed[subno] = true;
        Ok(Node::capture(inner, subno))
    }

    /// POSIX BRE：`( ) | + ? { }` 普通；`\(`/`\)` 分组、
    /// `\{`/`\}` 边界、`\1`-`\9` 回引、`\<`/`\>` 词边界。
    fn parse_atom_bre(&mut self, c: char) -> Result<Node, Error> {
        match c {
            '\\' => {
                self.i += 1;
                self.parse_bre_escape_atom()
            }
            '[' => self.parse_class(),
            '.' => {
                self.i += 1;
                Ok(Node::class(CharSet::dot(self.opts.dot_stops_newline())))
            }
            _ => {
                // ^ 只在分支/子表达式开头（已由 parse_branch 处理）；
                // 其余特殊字符在 BRE 里都是普通字符。
                self.i += 1;
                Ok(Node::char(c as u32))
            }
        }
    }

    fn parse_bre_escape_atom(&mut self) -> Result<Node, Error> {
        let e = match self.peek() {
            Some(e) => e,
            None => return Err(Self::err("REG_EESCAPE", "invalid escape \\ sequence")),
        };
        self.i += 1;
        match e {
            '(' => {
                let subno = self.next_sub;
                self.next_sub += 1;
                let inner = self.parse_alt()?;
                if !(self.peek() == Some('\\') && self.at(1) == Some(')')) {
                    return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
                }
                self.i += 2;
                if self.closed.len() <= subno {
                    self.closed.resize(subno + 1, false);
                }
                self.closed[subno] = true;
                Ok(Node::capture(inner, subno))
            }
            '1'..='9' => {
                // 只允许单数字回引；`\10` 是 invalid backreference。
                if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    return Err(Self::err(
                        "REG_ESUBREG",
                        "invalid backreference number",
                    ));
                }
                let subno = e as usize - '0' as usize;
                let closed = self.closed.get(subno).copied().unwrap_or(false);
                if !closed {
                    return Err(Self::err(
                        "REG_ESUBREG",
                        "invalid backreference number",
                    ));
                }
                Ok(Node::backref(subno, 1, 1, Flags::NONE))
            }
            '<' => Ok(Node::assert(AssertKind::WordStart)),
            '>' => Ok(Node::assert(AssertKind::WordEnd)),
            // `\{` 在原子位置没有可量化的对象。
            '{' => Err(Self::err("REG_BADRPT", "quantifier operand invalid")),
            // 其余 `\x` 一律是字面 x（`\d`→d、`\n`→n、`\}`→}）。
            _ => Ok(Node::char(e as u32)),
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
                    // 非捕获组保留包裹层：`(?:\M)+` 可被量化（tclsh
                    // 允许），但不能被误判成「量化约束」。内部无捕获
                    // 时用 NoRecord 包裹（语义透明、记录行为不变）。
                    if has_capture(&inner) {
                        Ok(inner)
                    } else {
                        Ok(Node::no_record(inner))
                    }
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
                // `(?` 出现在 pattern 中间（或 `(?-i)` 这类不是内嵌
                // 选项的写法）时，tclsh 把它当缺操作数的量词。
                _ => Err(Self::err("REG_BADRPT", "quantifier operand invalid")),
            }
        } else {
            if self.lacon > 0 {
                // lookahead 内所有括号非捕获，且不占用组号（tclsh：
                // `(?=(a))(a)` 的 (a) 仍是第 1 组）。
                let inner = self.parse_alt()?;
                if !self.eat(')') {
                    return Err(Self::err("REG_EPAREN", "parentheses () not balanced"));
                }
                return Ok(inner);
            }
            let subno = self.next_sub;
            self.next_sub += 1;
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
            'A' => Ok(Node::assert(AssertKind::Bot)),
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
        // 紧跟在 range 后的 `-x` 会与前一 range 共享端点（`[a-b-c]`），
        // tclsh 报 invalid character range。范围错误延迟到 `]` 之后
        // 再报——未闭合的括号优先报 EBRACK（tclsh 实测 `[z-a`）。
        let mut just_range = false;
        let mut bad_range = false;
        // 第一处 range 错误结束的位置：若错误正好结束于串尾，tclsh
        // 报 EBRACK（`[z-a`），后面还有字符则报 ERANGE（`[z-ab`）。
        let mut bad_range_at = 0usize;
        loop {
            match self.peek() {
                None => {
                    // 串尾是 `[` 且紧跟在坏 range 之后时，tclsh 先报
                    // 括号未闭合（`[z-a[`）；否则报范围错误。
                    if bad_range
                        && bad_range_at < self.i
                        && !(bad_range_at + 1 == self.i
                            && self.chars.get(bad_range_at) == Some(&'['))
                    {
                        return Err(Self::err("REG_ERANGE", "invalid character range"));
                    }
                    return Err(Self::err("REG_EBRACK", "brackets [] not balanced"));
                }
                Some(']') => {
                    self.i += 1;
                    break;
                }
                Some('[') if matches!(self.at(1), Some(':') | Some('.') | Some('=')) => {
                    // 已发现坏 range 时，后续 class 元素的错误不再
                    // 抢先（tclsh：`[z-a[:x:]` → invalid character range）。
                    match self.parse_posix_or_collate() {
                        Ok(item) => {
                            if let Some(item) = item {
                                items.push(item);
                            }
                        }
                        Err(e) => {
                            if bad_range {
                                return Err(Self::err(
                                    "REG_ERANGE",
                                    "invalid character range",
                                ));
                            }
                            // 外层 class 没有闭合 `]` 时，tclsh 先报
                            // EBRACK（`[a-b[:foo:]`）。
                            if e.code == "REG_ECTYPE"
                                && !self.chars[self.i..].contains(&']')
                            {
                                return Err(Self::err(
                                    "REG_EBRACK",
                                    "brackets [] not balanced",
                                ));
                            }
                            return Err(e);
                        }
                    }
                    // class 元素后面跟 `-x` 也是非法 range
                    //（tclsh：`[[:digit:]-a]` → invalid character range）。
                    just_range = true;
                    continue;
                }
                _ => {}
            }
            if just_range && self.peek() == Some('-') {
                if let Some(n) = self.at(1) {
                    if n != ']' && !bad_range {
                        bad_range = true;
                        bad_range_at = self.i;
                    }
                }
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
                                if !bad_range {
                                    bad_range_at = self.i;
                                }
                                bad_range = true;
                            } else {
                                items.push(Item::Range(lo_c as u32, hi_c as u32));
                                just_range = true;
                            }
                        }
                        ClassAtom::Pred(_) => {
                            // class 端点不能构成 range（tclsh：
                            // `[a-\d]` → invalid character range）。
                            if !bad_range {
                                bad_range_at = self.i;
                            }
                            bad_range = true;
                        }
                    }
                    continue;
                }
            } else if self.peek() == Some('-')
                && self.at(1).is_some()
                && self.at(1) != Some(']')
            {
                // `[\d-a]`：谓词端点同样非法。
                if !bad_range {
                    bad_range_at = self.i;
                }
                bad_range = true;
            }
            just_range = false;
            match lo {
                ClassAtom::Char(c) => items.push(Item::Range(c as u32, c as u32)),
                ClassAtom::Pred(p) => items.push(Item::Pred(p)),
            }
        }
        if bad_range {
            return Err(Self::err("REG_ERANGE", "invalid character range"));
        }
        Ok(Node::new(
            NodeKind::Class(CharSet::new(
                neg,
                neg && self.opts.class_stops_newline(),
                items,
            )),
            Flags::BRACKET,
        ))
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
        // BRE/ERE 的方括号里反斜杠没有转义作用，是普通字符
        //（tclsh 实测：`[\n]` 匹配反斜杠或 n，`[\\]` 匹配一个反斜杠）。
        if self.mode() != Mode::Are {
            return Ok(ClassAtom::Char('\\'));
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

/// 子树是否含捕获组（决定 `(?:...)` 能否用 NoRecord 包裹）。
fn has_capture(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Capture { .. } => true,
        NodeKind::Concat(v) | NodeKind::Alt(v) => v.iter().any(has_capture),
        NodeKind::Repeat { node, .. }
        | NodeKind::NoRecord(node)
        | NodeKind::Look { node, .. } => has_capture(node),
        _ => false,
    }
}

/// 解析入口。
pub fn parse(pattern: &str, opts: Options) -> Result<Node, Error> {
    Parser::new(pattern, opts).parse()
}
