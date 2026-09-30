//! Backtracking regex engine — the "full-featured" half of rtcl's dual
//! regex setup. The fast path (the `regex` crate) handles the common
//! cases; patterns the fast engine rejects compile here instead:
//! backreferences `\1`-`\9`, look-around `(?=)`/`(?!)`/`(?<=)`/`(?<!)`,
//! the `\m\M\y\Y` word assertions, `(?b)` BRE mode, embedded flags, and
//! literal braces that the `regex` crate's parser refuses. Semantics
//! follow Tcl ARE as accepted by tclsh 8.6.17, with leftmost-first
//! preference order (matching what the fast engine produces for the
//! patterns both accept).

/// Character-class predicate.
#[derive(Clone)]
enum Pred {
    Lit(char),
    Range(char, char),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
    Alpha,
    Alnum,
    Upper,
    Lower,
    XDigit,
    Punct,
    Print,
    Graph,
    Cntrl,
    Blank,
}

impl Pred {
    fn has(&self, c: char, nocase: bool) -> bool {
        match self {
            Pred::Lit(x) => *x == c || (nocase && lower_eq(*x, c)),
            Pred::Range(a, b) => {
                (*a..=*b).contains(&c) || (nocase && (*a..=*b).contains(&lower_var(c)))
            }
            Pred::Digit => c.is_ascii_digit(),
            Pred::NotDigit => !c.is_ascii_digit(),
            Pred::Word => is_word(c),
            Pred::NotWord => !is_word(c),
            Pred::Space => matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c'),
            Pred::NotSpace => !matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c'),
            Pred::Alpha => c.is_alphabetic(),
            Pred::Alnum => c.is_alphanumeric(),
            Pred::Upper => c.is_uppercase(),
            Pred::Lower => c.is_lowercase(),
            Pred::XDigit => c.is_ascii_hexdigit(),
            Pred::Punct => c.is_ascii_punctuation(),
            Pred::Print => !c.is_control(),
            Pred::Graph => !c.is_control() && !c.is_whitespace(),
            Pred::Cntrl => c.is_control(),
            Pred::Blank => c == ' ' || c == '\t',
        }
    }
}

fn lower_eq(a: char, b: char) -> bool {
    a.to_lowercase().eq(b.to_lowercase())
}

/// Lowercased form of `c` if it has one, else `c` itself.
fn lower_var(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

enum Node {
    Empty,
    Char(char),
    Class { neg: bool, preds: Vec<Pred> },
    Dot,
    Cat(Vec<Node>),
    Alt(Vec<Node>),
    Cap { idx: usize, sub: Box<Node> },
    NCap(Box<Node>),
    Rep {
        sub: Box<Node>,
        min: u32,
        max: Option<u32>,
        lazy: bool,
    },
    /// Placeholder for a `\ddd` run, resolved to a backref or octal char
    /// once the total group count is known.
    Digits(String),
    Backref(usize),
    Bol,
    Eol,
    Bos,
    Eos,
    WordB,
    NotWordB,
    WordStart,
    WordEnd,
    Look {
        neg: bool,
        behind: bool,
        sub: Box<Node>,
    },
}

pub(crate) struct BtProg {
    root: Node,
    pub(crate) ngroups: usize,
    nocase: bool,
    lineanchor: bool,
    linestop: bool,
}

struct Parser {
    p: Vec<char>,
    i: usize,
    ngroups: usize,
    expanded: bool,
    bre: bool,
    literal: bool,
    nocase: bool,
    lineanchor: bool,
    linestop: bool,
}

pub(crate) fn bt_compile(
    pattern: &str,
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> Result<BtProg, String> {
    let mut p = Parser {
        p: pattern.chars().collect(),
        i: 0,
        ngroups: 0,
        expanded,
        bre: false,
        literal: false,
        nocase,
        lineanchor,
        linestop,
    };
    let root = p.parse_alt()?;
    if p.i != p.p.len() {
        return Err("unmatched closing parenthesis".to_string());
    }
    let root = resolve_digits(root, p.ngroups);
    Ok(BtProg {
        root,
        ngroups: p.ngroups,
        nocase: p.nocase,
        lineanchor: p.lineanchor,
        linestop: p.linestop,
    })
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.p.get(self.i).copied()
    }
    fn at(&self, j: usize) -> Option<char> {
        self.p.get(j).copied()
    }

    /// -expanded: whitespace and `#`-to-end-of-line comments are ignored
    /// between tokens.
    fn skip_x(&mut self) {
        if !self.expanded {
            return;
        }
        loop {
            match self.peek() {
                Some(c) if is_ws(c) => self.i += 1,
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

    fn parse_alt(&mut self) -> Result<Node, String> {
        let mut branches = vec![self.parse_cat()?];
        loop {
            self.skip_x();
            if !self.bre {
                if self.peek() == Some('|') {
                    self.i += 1;
                    branches.push(self.parse_cat()?);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(Node::Alt(branches))
        }
    }

    fn parse_cat(&mut self) -> Result<Node, String> {
        let mut items: Vec<Node> = Vec::new();
        loop {
            self.skip_x();
            let c = match self.peek() {
                None => break,
                Some(c) => c,
            };
            if !self.bre && (c == '|' || c == ')') {
                break;
            }
            if self.bre && c == '\\' && self.at(self.i + 1) == Some(')') {
                break;
            }
            let atom = self.parse_atom()?;
            let atom = self.parse_quant(atom)?;
            items.push(atom);
        }
        match items.len() {
            0 => Ok(Node::Empty),
            1 => Ok(items.pop().unwrap()),
            _ => Ok(Node::Cat(items)),
        }
    }

    /// Apply a following quantifier (`*`, `+`, `?`, `{m,n}`), if any.
    /// A `{` that does not start a valid bound is a literal in Tcl ARE;
    /// the atom is returned untouched and `{` is pushed as its own atom.
    fn parse_quant(&mut self, atom: Node) -> Result<Node, String> {
        self.skip_x();
        let (min, max) = match self.peek() {
            Some('*') => {
                self.i += 1;
                (0, None)
            }
            Some('+') if !self.bre => {
                self.i += 1;
                (1, None)
            }
            Some('?') if !self.bre => {
                self.i += 1;
                (0, Some(1))
            }
            Some('{') if !self.bre => match self.scan_bound(self.i) {
                Some((m, n, end)) => {
                    self.i = end;
                    (m, n)
                }
                None => return Ok(atom),
            },
            Some('\\') if self.bre && self.at(self.i + 1) == Some('{') => {
                match self.scan_bre_bound(self.i) {
                    Some((m, n, end)) => {
                        self.i = end;
                        (m, n)
                    }
                    None => return Ok(atom),
                }
            }
            _ => return Ok(atom),
        };
        // Lazy suffix.
        self.skip_x();
        let lazy = if !self.bre && self.peek() == Some('?') {
            self.i += 1;
            true
        } else {
            false
        };
        Ok(Node::Rep {
            sub: Box::new(atom),
            min,
            max,
            lazy,
        })
    }

    /// Scan an ARE `{m}`, `{m,}`, `{,n}`, `{m,n}` starting at `at`;
    /// returns (min, max, index-after-`}`).
    fn scan_bound(&self, at: usize) -> Option<(u32, Option<u32>, usize)> {
        let mut j = at + 1;
        let mut min_s = String::new();
        while let Some(c) = self.at(j) {
            if c.is_ascii_digit() && min_s.len() < 9 {
                min_s.push(c);
                j += 1;
            } else {
                break;
            }
        }
        let max_s;
        match self.at(j) {
            Some('}') => {
                if min_s.is_empty() {
                    return None; // `{}` or `{,}`-ish: literal brace
                }
                max_s = min_s.clone();
                let m: u32 = min_s.parse().ok()?;
                return Some((m, Some(m), j + 1));
            }
            Some(',') => j += 1,
            _ => return None,
        }
        let mut mx = String::new();
        while let Some(c) = self.at(j) {
            if c.is_ascii_digit() && mx.len() < 9 {
                mx.push(c);
                j += 1;
            } else {
                break;
            }
        }
        if self.at(j) != Some('}') {
            return None;
        }
        let m: u32 = if min_s.is_empty() { 0 } else { min_s.parse().ok()? };
        let n: Option<u32> = if mx.is_empty() { None } else { Some(mx.parse().ok()?) };
        Some((m, n, j + 1))
    }

    /// BRE `\{m,n\}` starting at the backslash position.
    fn scan_bre_bound(&self, at: usize) -> Option<(u32, Option<u32>, usize)> {
        if self.at(at + 1) != Some('{') {
            return None;
        }
        let mut j = at + 2;
        let mut min_s = String::new();
        while let Some(c) = self.at(j) {
            if c.is_ascii_digit() && min_s.len() < 9 {
                min_s.push(c);
                j += 1;
            } else {
                break;
            }
        }
        let comma = self.at(j) == Some(',');
        if comma {
            j += 1;
        }
        let mut mx = String::new();
        while let Some(c) = self.at(j) {
            if c.is_ascii_digit() && mx.len() < 9 {
                mx.push(c);
                j += 1;
            } else {
                break;
            }
        }
        if self.at(j) != Some('\\') || self.at(j + 1) != Some('}') {
            return None;
        }
        let m: u32 = if min_s.is_empty() { 0 } else { min_s.parse().ok()? };
        let n: Option<u32> = if !comma {
            Some(m)
        } else if mx.is_empty() {
            None
        } else {
            Some(mx.parse().ok()?)
        };
        Some((m, n, j + 2))
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", c))
        }
    }

    fn parse_atom(&mut self) -> Result<Node, String> {
        self.skip_x();
        if self.literal {
            // (?q): the rest of the RE is a literal string.
            if let Some(c) = self.peek() {
                self.i += 1;
                return Ok(Node::Char(c));
            }
            return Ok(Node::Empty);
        }
        let c = match self.peek() {
            Some(c) => c,
            None => return Ok(Node::Empty),
        };
        match c {
            '(' if !self.bre => self.parse_paren(),
            '(' if self.bre => {
                self.i += 1;
                Ok(Node::Char('('))
            }
            '[' => self.parse_class(),
            '.' => {
                self.i += 1;
                Ok(Node::Dot)
            }
            '^' => {
                self.i += 1;
                Ok(Node::Bol)
            }
            '$' => {
                self.i += 1;
                Ok(Node::Eol)
            }
            '*' | '+' | '?' => Err("quantifier with nothing to repeat".to_string()),
            '\\' => self.parse_escape(),
            _ => {
                self.i += 1;
                Ok(Node::Char(c))
            }
        }
    }

    /// `(` — embedded flags `(?flags)` / `(?flags:...)`, non-capturing
    /// `(?:...)`, look-around, or a plain capturing group.
    fn parse_paren(&mut self) -> Result<Node, String> {
        if self.at(self.i + 1) == Some('<')
            && matches!(self.at(self.i + 2), Some('=') | Some('!'))
        {
            let neg = self.at(self.i + 2) == Some('!');
            self.i += 3;
            let sub = self.parse_alt()?;
            self.expect(')')?;
            return Ok(Node::Look {
                neg,
                behind: true,
                sub: Box::new(sub),
            });
        }
        if self.at(self.i + 1) == Some('?') {
            let mut j = self.i + 2;
            let mut flags = String::new();
            while let Some(f) = self.at(j) {
                if f == ':' || f == ')' {
                    break;
                }
                if f.is_ascii_alphabetic() {
                    flags.push(f);
                    j += 1;
                } else {
                    break;
                }
            }
            match self.at(j) {
                Some('=') | Some('!') if flags.is_empty() => {
                    let neg = self.at(j) == Some('!');
                    self.i = j + 1;
                    let sub = self.parse_alt()?;
                    self.expect(')')?;
                    return Ok(Node::Look {
                        neg,
                        behind: false,
                        sub: Box::new(sub),
                    });
                }
                Some(':') => {
                    self.i = j + 1;
                    self.apply_flags(&flags);
                    let sub = self.parse_alt()?;
                    self.expect(')')?;
                    return Ok(Node::NCap(Box::new(sub)));
                }
                Some(')') => {
                    self.i = j + 1;
                    self.apply_flags(&flags);
                    return Ok(Node::Empty);
                }
                _ => return Err("unrecognized flag".to_string()),
            }
        }
        self.i += 1;
        let idx = self.ngroups + 1;
        self.ngroups += 1;
        let sub = self.parse_alt()?;
        self.expect(')')?;
        Ok(Node::Cap {
            idx,
            sub: Box::new(sub),
        })
    }

    fn apply_flags(&mut self, flags: &str) {
        for f in flags.chars() {
            match f {
                'b' => self.bre = true,
                'c' => self.nocase = false,
                'i' => self.nocase = true,
                'e' | 'x' => self.expanded = true,
                'q' => self.literal = true,
                // Newline-sensitivity variants (Tcl: n = full, p = partial
                // both ways, w = linestop, s = lineanchor).
                'n' => {
                    self.lineanchor = true;
                    self.linestop = false;
                }
                'p' => {
                    self.lineanchor = true;
                    self.linestop = true;
                }
                'w' => self.linestop = true,
                's' => self.lineanchor = true,
                _ => {}
            }
        }
    }

    fn parse_escape(&mut self) -> Result<Node, String> {
        self.i += 1; // consume backslash
        let c = match self.peek() {
            Some(c) => c,
            None => return Err("trailing backslash".to_string()),
        };
        self.i += 1;
        if self.bre {
            if c.is_ascii_digit() && c != '0' {
                return Ok(Node::Backref(c as usize - '0' as usize));
            }
            return Ok(Node::Char(unescape_char(c)?));
        }
        if c == 'x' {
            let mut v = 0u32;
            let mut n = 0;
            while n < 2 {
                match self.peek().and_then(|h| h.to_digit(16)) {
                    Some(d) => {
                        v = v * 16 + d;
                        self.i += 1;
                        n += 1;
                    }
                    None => break,
                }
            }
            if n == 0 {
                return Err("invalid hex escape".to_string());
            }
            return Ok(Node::Char(char::from_u32(v).unwrap_or('\u{FFFD}')));
        }
        if c.is_ascii_digit() {
            let mut run = String::new();
            run.push(c);
            while let Some(d) = self.peek() {
                if d.is_ascii_digit() && run.len() < 9 {
                    run.push(d);
                    self.i += 1;
                } else {
                    break;
                }
            }
            return Ok(Node::Digits(run));
        }
        match c {
            'd' => Ok(Node::Class { neg: false, preds: vec![Pred::Digit] }),
            'D' => Ok(Node::Class { neg: false, preds: vec![Pred::NotDigit] }),
            'w' => Ok(Node::Class { neg: false, preds: vec![Pred::Word] }),
            'W' => Ok(Node::Class { neg: false, preds: vec![Pred::NotWord] }),
            's' => Ok(Node::Class { neg: false, preds: vec![Pred::Space] }),
            'S' => Ok(Node::Class { neg: false, preds: vec![Pred::NotSpace] }),
            'm' => Ok(Node::WordStart),
            'M' => Ok(Node::WordEnd),
            'y' => Ok(Node::WordB),
            'Y' => Ok(Node::NotWordB),
            'A' => Ok(Node::Bos),
            'z' | 'Z' => Ok(Node::Eos),
            _ => Ok(Node::Char(unescape_char(c)?)),
        }
    }

    fn parse_class(&mut self) -> Result<Node, String> {
        self.i += 1; // consume '['
        let mut neg = false;
        if self.peek() == Some('^') {
            neg = true;
            self.i += 1;
        }
        let mut preds: Vec<Pred> = Vec::new();
        let mut first = true;
        loop {
            let c = match self.peek() {
                Some(c) => c,
                None => return Err("unmatched '['".to_string()),
            };
            if c == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            // POSIX class [:name:]
            if c == '[' && self.at(self.i + 1) == Some(':') {
                let mut j = self.i + 2;
                let mut negp = false;
                if self.at(j) == Some('^') {
                    negp = true;
                    j += 1;
                }
                let mut name = String::new();
                while let Some(n) = self.at(j) {
                    if n == ':' {
                        break;
                    }
                    name.push(n);
                    j += 1;
                }
                if self.at(j) == Some(':') && self.at(j + 1) == Some(']') {
                    if let Some(pred) = posix_pred(&name) {
                        preds.push(if negp { negate(pred) } else { pred });
                        self.i = j + 2;
                        continue;
                    }
                }
                // Not a valid POSIX class: '[' is a literal member.
            }
            // One member: char, escape-set, or range.
            let lo = self.class_char()?;
            match lo {
                ClassItem::Set(mut ps) => preds.append(&mut ps),
                ClassItem::One(lo_c) => {
                    // Range? `a-z` — but `[-a]` and `[a-]` keep '-' literal.
                    if self.peek() == Some('-')
                        && self.at(self.i + 1).is_some()
                        && self.at(self.i + 1) != Some(']')
                    {
                        self.i += 1;
                        let hi = self.class_char()?;
                        match hi {
                            ClassItem::One(hi_c) => preds.push(Pred::Range(lo_c, hi_c)),
                            ClassItem::Set(mut ps) => {
                                preds.push(Pred::Range(lo_c, lo_c));
                                preds.append(&mut ps);
                            }
                        }
                    } else {
                        preds.push(Pred::Lit(lo_c));
                    }
                }
            }
        }
        Ok(Node::Class { neg, preds })
    }

    /// One class member: a single char or an escape shorthand set.
    fn class_char(&mut self) -> Result<ClassItem, String> {
        let c = match self.peek() {
            Some(c) => c,
            None => return Err("unmatched '['".to_string()),
        };
        self.i += 1;
        if c != '\\' {
            return Ok(ClassItem::One(c));
        }
        let e = match self.peek() {
            Some(e) => e,
            None => return Err("trailing backslash".to_string()),
        };
        self.i += 1;
        Ok(match e {
            'x' => {
                let mut v = 0u32;
                let mut n = 0;
                while n < 2 {
                    match self.peek().and_then(|h| h.to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            self.i += 1;
                            n += 1;
                        }
                        None => break,
                    }
                }
                if n == 0 {
                    return Err("invalid hex escape".to_string());
                }
                ClassItem::One(char::from_u32(v).unwrap_or('\u{FFFD}'))
            }
            'd' => ClassItem::Set(vec![Pred::Digit]),
            'D' => ClassItem::Set(vec![Pred::NotDigit]),
            'w' => ClassItem::Set(vec![Pred::Word]),
            'W' => ClassItem::Set(vec![Pred::NotWord]),
            's' => ClassItem::Set(vec![Pred::Space]),
            'S' => ClassItem::Set(vec![Pred::NotSpace]),
            _ => ClassItem::One(unescape_char(e)?),
        })
    }
}

enum ClassItem {
    One(char),
    Set(Vec<Pred>),
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

fn posix_pred(name: &str) -> Option<Pred> {
    Some(match name {
        "alpha" => Pred::Alpha,
        "digit" => Pred::Digit,
        "alnum" => Pred::Alnum,
        "upper" => Pred::Upper,
        "lower" => Pred::Lower,
        "xdigit" => Pred::XDigit,
        "space" => Pred::Space,
        "blank" => Pred::Blank,
        "punct" => Pred::Punct,
        "print" => Pred::Print,
        "graph" => Pred::Graph,
        "cntrl" => Pred::Cntrl,
        "word" => Pred::Word,
        _ => return None,
    })
}

fn negate(p: Pred) -> Pred {
    match p {
        Pred::Digit => Pred::NotDigit,
        Pred::NotDigit => Pred::Digit,
        Pred::Word => Pred::NotWord,
        Pred::NotWord => Pred::Word,
        Pred::Space => Pred::NotSpace,
        Pred::NotSpace => Pred::Space,
        other => other,
    }
}

/// Resolve a non-shorthand `\c` escape to its literal char.
fn unescape_char(c: char) -> Result<char, String> {
    Ok(match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'f' => '\x0c',
        'v' => '\x0b',
        'a' => '\x07',
        'e' => '\x1b',
        'b' => '\x08',
        other => other,
    })
}

/// Walk the tree replacing `Digits` runs: the longest prefix that names an
/// existing group becomes a backref; otherwise up to three octal digits
/// form a character and the rest are literals (`\316` with no groups is
/// U+00CE followed by literal `1573148`).
fn resolve_digits(n: Node, ngroups: usize) -> Node {
    match n {
        Node::Digits(run) => {
            let chars: Vec<char> = run.chars().collect();
            // Longest valid backref prefix (group numbers start at 1).
            let mut best: Option<(usize, usize)> = None; // (num, digits_used)
            for take in (1..=chars.len()).rev() {
                if chars[0] == '0' {
                    break;
                }
                let num: usize = chars[..take].iter().collect::<String>().parse().unwrap_or(0);
                if num <= ngroups {
                    best = Some((num, take));
                    break;
                }
            }
            let mut items: Vec<Node> = Vec::new();
            match best {
                Some((num, used)) => {
                    items.push(Node::Backref(num));
                    for c in &chars[used..] {
                        items.push(Node::Char(*c));
                    }
                }
                None => {
                    // Octal: up to three leading digits.
                    let oct_take = chars.iter().take(3).take_while(|c| c.is_digit(8)).count();
                    let oct: u32 = chars[..oct_take].iter().collect::<String>().parse().unwrap_or(0);
                    if let Some(c) = char::from_u32(oct) {
                        items.push(Node::Char(c));
                    }
                    for c in &chars[oct_take..] {
                        items.push(Node::Char(*c));
                    }
                }
            }
            match items.len() {
                0 => Node::Empty,
                1 => items.pop().unwrap(),
                _ => Node::Cat(items),
            }
        }
        Node::Cat(v) => Node::Cat(v.into_iter().map(|x| resolve_digits(x, ngroups)).collect()),
        Node::Alt(v) => Node::Alt(v.into_iter().map(|x| resolve_digits(x, ngroups)).collect()),
        Node::Cap { idx, sub } => Node::Cap {
            idx,
            sub: Box::new(resolve_digits(*sub, ngroups)),
        },
        Node::NCap(sub) => Node::NCap(Box::new(resolve_digits(*sub, ngroups))),
        Node::Rep {
            sub,
            min,
            max,
            lazy,
        } => Node::Rep {
            sub: Box::new(resolve_digits(*sub, ngroups)),
            min,
            max,
            lazy,
        },
        Node::Look {
            neg,
            behind,
            sub,
        } => Node::Look {
            neg,
            behind,
            sub: Box::new(resolve_digits(*sub, ngroups)),
        },
        other => other,
    }
}

type Caps = Vec<Option<(usize, usize)>>;

struct Cx<'a> {
    s: &'a [char],
    nocase: bool,
    lineanchor: bool,
    linestop: bool,
}

fn word_boundary(cx: &Cx, pos: usize) -> bool {
    let before = pos > 0 && is_word(cx.s[pos - 1]);
    let after = pos < cx.s.len() && is_word(cx.s[pos]);
    before != after
}

fn mnd(
    cx: &Cx,
    n: &Node,
    pos: usize,
    caps: &mut Caps,
    k: &mut dyn FnMut(&mut Caps, usize) -> bool,
) -> bool {
    match n {
        Node::Empty => k(caps, pos),
        // Resolved away by resolve_digits before matching.
        Node::Digits(_) => false,
        Node::Char(c) => pos < cx.s.len() && (*c == cx.s[pos] || (cx.nocase && lower_eq(*c, cx.s[pos]))) && k(caps, pos + 1),
        Node::Dot => pos < cx.s.len() && (!cx.linestop || cx.s[pos] != '\n') && k(caps, pos + 1),
        Node::Class { neg, preds } => {
            pos < cx.s.len()
                && {
                    let c = cx.s[pos];
                    let hit = preds.iter().any(|p| p.has(c, cx.nocase))
                        || (cx.nocase
                            && preds.iter().any(|p| {
                                p.has(lower_var(c), false) || p.has(c.to_uppercase().next().unwrap_or(c), false)
                            }));
                    hit != *neg
                }
                && k(caps, pos + 1)
        }
        Node::Bol => (pos == 0 || (cx.lineanchor && cx.s[pos - 1] == '\n')) && k(caps, pos),
        Node::Eol => (pos == cx.s.len() || (cx.lineanchor && cx.s[pos] == '\n')) && k(caps, pos),
        Node::Bos => pos == 0 && k(caps, pos),
        Node::Eos => pos == cx.s.len() && k(caps, pos),
        Node::WordB => word_boundary(cx, pos) && k(caps, pos),
        Node::NotWordB => !word_boundary(cx, pos) && k(caps, pos),
        Node::WordStart => {
            pos < cx.s.len()
                && is_word(cx.s[pos])
                && (pos == 0 || !is_word(cx.s[pos - 1]))
                && k(caps, pos)
        }
        Node::WordEnd => {
            pos > 0
                && is_word(cx.s[pos - 1])
                && (pos == cx.s.len() || !is_word(cx.s[pos]))
                && k(caps, pos)
        }
        Node::Cat(items) => {
            mcat(cx, items, 0, pos, caps, k)
        }
        Node::Alt(bs) => {
            for b in bs {
                if mnd(cx, b, pos, caps, k) {
                    return true;
                }
            }
            false
        }
        Node::Cap { idx, sub } => {
            let start = pos;
            mnd(cx, sub, pos, caps, &mut |caps, p| {
                let old = caps[*idx];
                caps[*idx] = Some((start, p));
                if k(caps, p) {
                    true
                } else {
                    caps[*idx] = old;
                    false
                }
            })
        }
        Node::NCap(sub) => mnd(cx, sub, pos, caps, k),
        Node::Backref(g) => match caps.get(*g).copied().flatten() {
            Some((s, e)) => {
                let len = e - s;
                if pos + len > cx.s.len() {
                    return false;
                }
                for j in 0..len {
                    if cx.s[s + j] != cx.s[pos + j] && !(cx.nocase && lower_eq(cx.s[s + j], cx.s[pos + j])) {
                        return false;
                    }
                }
                k(caps, pos + len)
            }
            None => false,
        },
        Node::Rep {
            sub,
            min,
            max,
            lazy,
        } => mrep(cx, sub, pos, 0, *min, *max, *lazy, caps, k),
        Node::Look { neg, behind, sub } => {
            let mut probe = caps.clone();
            let hit = if *behind {
                (0..=pos).any(|j| mnd(cx, sub, j, &mut probe, &mut |_c, p| p == pos))
            } else {
                mnd(cx, sub, pos, &mut probe, &mut |_c, _p| true)
            };
            if hit != *neg {
                k(caps, pos)
            } else {
                false
            }
        }
    }
}

fn mcat(
    cx: &Cx,
    items: &[Node],
    at: usize,
    pos: usize,
    caps: &mut Caps,
    k: &mut dyn FnMut(&mut Caps, usize) -> bool,
) -> bool {
    if at == items.len() {
        return k(caps, pos);
    }
    mnd(cx, &items[at], pos, caps, &mut |caps, p| {
        mcat(cx, items, at + 1, p, caps, k)
    })
}

#[allow(clippy::too_many_arguments)]
fn mrep(
    cx: &Cx,
    sub: &Node,
    pos: usize,
    done: u32,
    min: u32,
    max: Option<u32>,
    lazy: bool,
    caps: &mut Caps,
    k: &mut dyn FnMut(&mut Caps, usize) -> bool,
) -> bool {
    let can_more = max.map_or(true, |mx| done < mx);
    if lazy {
        if done >= min && k(caps, pos) {
            return true;
        }
        if !can_more {
            return false;
        }
        mnd(cx, sub, pos, caps, &mut |caps, p2| {
            if p2 == pos && done + 1 > min {
                return false; // zero-width guard
            }
            mrep(cx, sub, p2, done + 1, min, max, lazy, caps, k)
        })
    } else {
        if can_more
            && mnd(cx, sub, pos, caps, &mut |caps, p2| {
                if p2 == pos && done >= min {
                    return false; // zero-width guard
                }
                mrep(cx, sub, p2, done + 1, min, max, lazy, caps, k)
            })
        {
            return true;
        }
        if done >= min {
            k(caps, pos)
        } else {
            false
        }
    }
}

/// Find the leftmost match in `slice` (independent of any larger string);
/// returns capture ranges in slice-BYTE offsets, group 0 first.
pub(crate) fn bt_caps_at(prog: &BtProg, slice: &str) -> Option<Caps> {
    let chars: Vec<char> = slice.chars().collect();
    let cx = Cx {
        s: &chars,
        nocase: prog.nocase,
        lineanchor: prog.lineanchor,
        linestop: prog.linestop,
    };
    // char index -> byte offset (with sentinel for end).
    let mut bmap: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut b = 0usize;
    for c in &chars {
        bmap.push(b);
        b += c.len_utf8();
    }
    bmap.push(slice.len());
    for start in 0..=chars.len() {
        let mut caps: Caps = vec![None; prog.ngroups + 1];
        let mut k = |caps: &mut Caps, end: usize| -> bool {
            caps[0] = Some((start, end));
            true
        };
        if mnd(&cx, &prog.root, start, &mut caps, &mut k) {
            return Some(
                caps.into_iter()
                    .map(|g| g.map(|(s, e)| (bmap[s], bmap[e])))
                    .collect(),
            );
        }
    }
    None
}

pub(crate) fn bt_is_match(prog: &BtProg, s: &str) -> bool {
    bt_caps_at(prog, s).is_some()
}
