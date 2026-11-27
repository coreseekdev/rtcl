//! tclsh-faithful expression syntax validation.
//!
//! Port of tclCompExpr.c's ParseExpr/ParseLexeme error detection (tcl
//! 8.6.17): a lexeme-driven precedence parser that produces byte-exact
//! parse-error messages, including the `_@_` position mark, the `...`
//! elision around the current lexeme, and the `should be ...` postscript
//! for barewords.  Runs as a pre-pass before rtcl's own evaluator, which
//! keeps full responsibility for the semantics of expressions that pass.

const LIMIT: usize = 25; // tclCompExpr.c substring limit
const HEAD: usize = LIMIT - 3; // 22 chars kept of over-long substrings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lx {
    // Leaves
    Number,
    Quoted,
    Braced,
    Script,
    Variable,
    BoolLit,
    // Unary
    UnaryPlus,
    UnaryMinus,
    Not,
    BitNot,
    Function,
    OpenParen,
    // Binary
    Plus,
    Minus,
    Mult,
    Div,
    Mod,
    Less,
    Greater,
    Leq,
    Geq,
    Equal,
    Neq,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
    Streq,
    Strneq,
    StrLt,
    StrGt,
    StrLe,
    StrGe,
    In,
    Ni,
    LeftShift,
    RightShift,
    // rtcl extensions (not in tclsh 8.6): glob/regexp match.  rotates
    // (`<<<`/`>>>`) 已移除对齐 tclsh 词法（bn13）。
    GlobMatch,
    ReMatch,
    Expon,
    Comma,
    Question,
    Colon,
    CloseParen,
    End,
    // Uncategorized
    Bareword,
    Invalid,
    Incomplete,
    Start,
}

impl Lx {
    fn cat(self) -> Cat {
        match self {
            Lx::Number
            | Lx::Quoted
            | Lx::Braced
            | Lx::Script
            | Lx::Variable
            | Lx::BoolLit => Cat::Leaf,
            Lx::UnaryPlus
            | Lx::UnaryMinus
            | Lx::Not
            | Lx::BitNot
            | Lx::Function
            | Lx::OpenParen
            | Lx::Start => Cat::Unary,
            _ => Cat::Binary,
        }
    }

    fn prec(self) -> u8 {
        match self {
            Lx::End => P_END,
            Lx::Start => P_START,
            Lx::CloseParen => P_CLOSE_PAREN,
            Lx::OpenParen => P_OPEN_PAREN,
            Lx::Comma => P_COMMA,
            Lx::Question | Lx::Colon => P_CONDITIONAL,
            Lx::Or => P_OR,
            Lx::And => P_AND,
            Lx::BitOr => P_BIT_OR,
            Lx::BitXor => P_BIT_XOR,
            Lx::BitAnd => P_BIT_AND,
            Lx::Equal | Lx::Neq | Lx::Streq | Lx::Strneq | Lx::In | Lx::Ni
            | Lx::GlobMatch | Lx::ReMatch => P_EQUAL,
            Lx::Less | Lx::Greater | Lx::Leq | Lx::Geq | Lx::StrLt | Lx::StrGt
            | Lx::StrLe | Lx::StrGe => P_COMPARE,
            Lx::LeftShift | Lx::RightShift => P_SHIFT,
            Lx::Plus | Lx::Minus => P_ADD,
            Lx::Mult | Lx::Div | Lx::Mod => P_MULT,
            Lx::Expon => P_EXPON,
            Lx::UnaryPlus | Lx::UnaryMinus | Lx::Not | Lx::BitNot | Lx::Function => P_UNARY,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy)]
enum Cat {
    Leaf,
    Unary,
    Binary,
}

// Precedences (enum Precedence, low → high).
const P_END: u8 = 1;
const P_START: u8 = 2;
const P_CLOSE_PAREN: u8 = 3;
const P_OPEN_PAREN: u8 = 4;
const P_COMMA: u8 = 5;
const P_CONDITIONAL: u8 = 6;
const P_OR: u8 = 7;
const P_AND: u8 = 8;
const P_BIT_OR: u8 = 9;
const P_BIT_XOR: u8 = 10;
const P_BIT_AND: u8 = 11;
const P_EQUAL: u8 = 12;
const P_COMPARE: u8 = 13;
const P_SHIFT: u8 = 14;
const P_ADD: u8 = 15;
const P_MULT: u8 = 16;
const P_EXPON: u8 = 17;
const P_UNARY: u8 = 18;

/// Tcl bareword characters: alphanumerics, underscore, all bytes >= 0x80.
/// tclsh's lexer: a bareword is ASCII alphanumerics and `_`. A non-ASCII
/// byte is an INVALID character (`expr 有` → `invalid character "有"`),
/// never a bareword.
fn is_bareword(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_tcl_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// The completed subtree waiting for its operator.  Mirrors the C's
/// `complete` OperandTypes: OT_LITERAL/OT_TOKENS are leaves; a subtree
/// rooted at a `:` node is tracked for the `?:` pairing checks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Complete {
    Leaf,
    Colon,
    Other,
}

struct Checker<'a> {
    e: &'a [u8],
    pos: usize,
    /// Incomplete trees (lexeme stack); START sits at the bottom.
    stack: Vec<Lx>,
    complete: Complete,
    /// Whether the most recently parsed item is an operand (the C's
    /// NotOperator(lastParsed)) rather than a pushed operator node.
    /// Initialization mirrors lastParsed = START node.
    last_operand: bool,
}

/// A leaf-construct scan error: (reason, term offset, incomplete flag).
type ScanErr = (String, usize, usize);

/// Validate an expression, returning the complete tclsh-style error
/// message on syntax errors.
pub fn check_expr(expr: &str) -> Result<(), String> {
    let mut c = Checker {
        e: expr.as_bytes(),
        pos: 0,
        stack: vec![Lx::Start],
        complete: Complete::Leaf,
        last_operand: false,
    };
    loop {
        // Skip whitespace between lexemes, plus TIP582 comments: a `#`
        // runs to end-of-line wherever a new lexeme may start
        // (`expr {1 # + 2}` → 1, `expr {max# c\n(1,2)}` → 2).
        loop {
            while c.pos < c.e.len() && is_tcl_ws(c.e[c.pos]) {
                c.pos += 1;
            }
            if c.pos < c.e.len() && c.e[c.pos] == b'#' {
                while c.pos < c.e.len() && c.e[c.pos] != b'\n' {
                    c.pos += 1;
                }
                continue;
            }
            break;
        }
        let (lex0, scanned, start) = match c.scan_lexeme() {
            Ok(v) => v,
            // Leaf-construct scan errors annotate at the error's own
            // term offset with no mark.
            Err((reason, term, inc)) => return Err(c.build(&reason, term, inc, false, None)),
        };
        // Uncategorized error indicators dispatch before the categories.
        if lex0 == Lx::Invalid {
            let text = c.slice(start, start + scanned);
            return Err(c.build(
                &format!("invalid character \"{text}\""),
                start,
                scanned,
                false,
                None,
            ));
        }
        if lex0 == Lx::Incomplete {
            return Err(c.build("incomplete operator \"=\"", start, scanned, false, None));
        }
        // `+`/`-` resolve to unary or binary from context.
        let lex = match lex0 {
            Lx::Plus if !c.last_operand => Lx::UnaryPlus,
            Lx::Minus if !c.last_operand => Lx::UnaryMinus,
            l => l,
        };
        // A BAREWORD resolves to FUNCTION (followed by `(`), a boolean
        // literal, or the invalid-bareword error.
        let lex = if lex == Lx::Bareword {
            match c.resolve_bareword(start, scanned) {
                BarewordResolve::Function => Lx::Function,
                BarewordResolve::Bool => Lx::BoolLit,
                BarewordResolve::Numeric => Lx::Number,
                BarewordResolve::Error(msg) => return Err(msg),
            }
        } else {
            lex
        };
        match lex.cat() {
            Cat::Leaf => {
                if c.last_operand {
                    return Err(c.missing_op(start));
                }
                c.complete = Complete::Leaf;
                c.last_operand = true;
                c.pos = start + scanned;
            }
            Cat::Unary => {
                if c.last_operand {
                    return Err(c.missing_op(start));
                }
                c.complete = Complete::Leaf;
                c.stack.push(lex);
                c.last_operand = false;
                c.pos = start + scanned;
            }
            Cat::Binary => {
                if !c.last_operand {
                    // An operator arriving while the previous item is
                    // another operator — one of them lacks its operand
                    // (the 1100-block).  The special f() case restarts
                    // the loop with an empty operand between the parens.
                    if lex == Lx::CloseParen
                        && *c.stack.last().unwrap() == Lx::OpenParen
                        && c.stack.len() >= 2
                        && c.stack[c.stack.len() - 2] == Lx::Function
                    {
                        c.complete = Complete::Leaf; // the OT_EMPTY operand
                        c.last_operand = true;
                        continue; // re-parse the `)` without advancing
                    }
                    return Err(c.arrival_error(lex, start, scanned));
                }
                // Reduce loop: attach the complete tree to pending
                // operators while precedence says so.
                let mut popped;
                loop {
                    popped = *c.stack.last().unwrap();
                    let tp = popped.prec();
                    if tp < lex.prec() {
                        break;
                    }
                    if tp == lex.prec() {
                        if lex == Lx::Expon {
                            break; // right-associative
                        }
                        // "?" and ":" pair up sensibly.
                        if popped == Lx::Question && c.complete != Complete::Colon {
                            break;
                        }
                        if popped == Lx::Colon && lex == Lx::Question {
                            break;
                        }
                    }
                    // Parens must balance.
                    if popped == Lx::OpenParen && lex != Lx::CloseParen {
                        return Err(c.build("unbalanced open paren", start, scanned, false, None));
                    }
                    // Right operand of "?" must be ":".
                    if popped == Lx::Question && c.complete != Complete::Colon {
                        return Err(c.build("missing operator \":\" at _@_", start, 0, true, None));
                    }
                    // Operator ":" may only be right operand of "?".
                    if c.complete == Complete::Colon && popped != Lx::Question {
                        return Err(c.build(
                            "unexpected operator \":\" without preceding \"?\"",
                            start,
                            scanned,
                            false,
                            None,
                        ));
                    }
                    // Attach the complete tree as the right operand.
                    c.complete = if popped == Lx::Colon {
                        Complete::Colon
                    } else {
                        Complete::Other
                    };
                    c.stack.pop();
                    if popped == Lx::Start {
                        // Completing START ends a successful parse —
                        // only END reaches this point.
                        return Ok(());
                    }
                    if popped == Lx::OpenParen {
                        break;
                    }
                }
                // Post-reduce checks (incompletePtr is the node at the
                // top when the loop was entered).
                if lex == Lx::CloseParen && popped != Lx::OpenParen {
                    return Err(c.build("unbalanced close paren", start, scanned, false, None));
                }
                if lex == Lx::Comma
                    && (popped != Lx::OpenParen
                        || c.stack.len() < 2
                        || c.stack[c.stack.len() - 2] != Lx::Function)
                {
                    return Err(c.build(
                        "unexpected \",\" outside function argument list",
                        start,
                        scanned,
                        false,
                        None,
                    ));
                }
                if c.complete == Complete::Colon {
                    return Err(c.build(
                        "unexpected operator \":\" without preceding \"?\"",
                        start,
                        scanned,
                        false,
                        None,
                    ));
                }
                // Create no node for CLOSE_PAREN; it just closes.
                if lex == Lx::CloseParen {
                    c.pos = start + scanned;
                    continue;
                }
                // Push the just-parsed operator as a new incomplete tree.
                c.complete = Complete::Leaf;
                c.stack.push(lex);
                c.last_operand = false;
                c.pos = start + scanned;
            }
        }
    }
}

enum BarewordResolve {
    Function,
    Bool,
    Numeric,
    Error(String),
}

impl<'a> Checker<'a> {
    /// `e[a..b]` with the byte offsets tclsh's layout computes. They can
    /// fall inside a multi-byte character; both offsets back up to a
    /// character start, so the quote never shows half a character (the
    /// same rule tclsh applies: its 22-byte window over CJK text keeps
    /// the 7 whole characters that fit).
    fn slice(&self, a: usize, b: usize) -> String {
        let e = self.e;
        // Offset `i` opens a character iff its byte is not a continuation.
        let opens_char = |i: usize| i >= e.len() || e[i] & 0xC0 != 0x80;
        let mut a = a.min(e.len());
        let mut b = b.min(e.len());
        while a > 0 && !opens_char(a) {
            a -= 1;
        }
        while b > a && !opens_char(b) {
            b -= 1;
        }
        String::from_utf8_lossy(&e[a..b]).into_owned()
    }

    /// The annotated `in expression "..."` message.  Layout follows the
    /// C: `[...][head 22][mid 22+...][_@_][tail 22+...][...]` with an
    /// optional postscript after `;\n`.
    fn build(
        &self,
        reason: &str,
        start: usize,
        scanned: usize,
        mark: bool,
        post: Option<&str>,
    ) -> String {
        let n = self.e.len();
        let head_dots = start >= LIMIT;
        let head_start = if head_dots { start - HEAD } else { 0 };
        // The lexeme itself is quoted in full below LIMIT bytes.
        let mid_dots = scanned >= LIMIT;
        let mid_end = start + if mid_dots { scanned.min(HEAD) } else { scanned };
        let tail_dots = start + scanned + LIMIT <= n;
        let tail_end = if tail_dots { start + scanned + HEAD } else { n };
        let mut msg = format!("{reason}\nin expression \"");
        if head_dots {
            msg.push_str("...");
        }
        msg.push_str(&self.slice(head_start, start));
        msg.push_str(&self.slice(start, mid_end));
        if mid_dots {
            msg.push_str("...");
        }
        if mark {
            msg.push_str("_@_");
        }
        msg.push_str(&self.slice(start + scanned, tail_end));
        if tail_dots {
            msg.push_str("...");
        }
        msg.push('"');
        if let Some(p) = post {
            msg.push_str(";\n");
            msg.push_str(p);
        }
        msg
    }

    fn missing_op(&self, start: usize) -> String {
        self.build("missing operator at _@_", start, 0, true, None)
    }

    /// The 1100-block: an operator arriving while the previous item is
    /// another operator.  `scanned` is the arriving lexeme's width.
    fn arrival_error(&self, lex: Lx, start: usize, scanned: usize) -> String {
        let top = *self.stack.last().unwrap();
        if lex == Lx::CloseParen && top == Lx::OpenParen {
            return self.build("empty subexpression at _@_", start, 0, true, None);
        }
        if top.prec() > lex.prec() {
            if top == Lx::OpenParen {
                return self.build("unbalanced open paren", start, scanned, false, None);
            }
            if top == Lx::Comma {
                return self.build("missing function argument at _@_", start, 0, true, None);
            }
            if top == Lx::Start {
                return self.build("empty expression", start, scanned, false, None);
            }
        } else if lex == Lx::CloseParen {
            return self.build("unbalanced close paren", start, scanned, false, None);
        } else if lex == Lx::Comma
            && top == Lx::OpenParen
            && self.stack.len() >= 2
            && self.stack[self.stack.len() - 2] == Lx::Function
        {
            return self.build("missing function argument at _@_", start, 0, true, None);
        }
        self.build("missing operand at _@_", start, 0, true, None)
    }

    /// Resolve a BAREWORD lexeme at `start`: FUNCTION if followed (over
    /// whitespace) by `(`, BOOL if a boolean word, NUMERIC if the word
    /// parses fully as a number (tclsh: `expr inf` → Inf, `expr nan` →
    /// NaN — letter-led numbers; `Influence` is no number), else the
    /// invalid-bareword error.
    fn resolve_bareword(&self, start: usize, scanned: usize) -> BarewordResolve {
        let word = self.slice(start, start + scanned);
        let mut j = start + scanned;
        // Function-name lookahead crosses whitespace AND TIP582 comments
        // (`expr {max# comment\n(1,2)}` is a call).
        loop {
            while j < self.e.len() && is_tcl_ws(self.e[j]) {
                j += 1;
            }
            if j < self.e.len() && self.e[j] == b'#' {
                while j < self.e.len() && self.e[j] != b'\n' {
                    j += 1;
                }
                continue;
            }
            break;
        }
        if j < self.e.len() && self.e[j] == b'(' {
            return BarewordResolve::Function;
        }
        // Boolean words, including tclsh's unique-prefix abbreviations
        // (`expr bool(y)` — expr-31.0.4.0; "o" alone stays invalid because
        // it prefixes both "on" and "off").
        if crate::types::expr_funcs::bool_from_string(&word).is_some() {
            return BarewordResolve::Bool;
        }
        if let Some((len, _)) = scan_number(word.as_bytes()) {
            if len == word.len() {
                return BarewordResolve::Numeric;
            }
        }
        // tclsh quotes the whole word below LIMIT bytes, else its first
        // HEAD bytes (never splitting a character) and an ellipsis.
        let (w, d): (&str, &str) = if word.len() >= LIMIT {
            let mut cut = HEAD;
            while !word.is_char_boundary(cut) {
                cut -= 1;
            }
            (&word[..cut], "...")
        } else {
            (word.as_str(), "")
        };
        let post = format!(
            "should be \"${w}{d}\" or \"{{{w}{d}}}\" or \"{w}{d}(...)\" or ...{}",
            crate::types::bignum::bareword_hint(&word)
        );
        let reason = format!("invalid bareword \"{w}{d}\"");
        BarewordResolve::Error(self.build(&reason, start, scanned, false, Some(&post)))
    }

    /// Scan one lexeme at `self.pos`.  Returns (lexeme, scanned, start).
    fn scan_lexeme(&mut self) -> Result<(Lx, usize, usize), ScanErr> {
        let start = self.pos;
        let e = self.e;
        let n = e.len();
        if start >= n {
            return Ok((Lx::End, 0, start));
        }
        let rest = &e[start..];
        let b = rest[0];
        match b {
            b'"' => match scan_quoted(e, start) {
                Ok(end) => Ok((Lx::Quoted, end + 1 - start, start)),
                Err(err) => Err(err),
            },
            b'{' => match scan_braced(e, start) {
                Ok(end) => Ok((Lx::Braced, end + 1 - start, start)),
                Err(err) => Err(err),
            },
            b'[' => match scan_script(e, start) {
                Ok(end) => Ok((Lx::Script, end + 1 - start, start)),
                Err(err) => Err(err),
            },
            b'$' => match scan_variable(e, start) {
                Ok(end) => Ok((Lx::Variable, end - start, start)),
                Err(err) => Err(err),
            },
            b'*' if rest.len() > 1 && rest[1] == b'*' => Ok((Lx::Expon, 2, start)),
            b'*' => Ok((Lx::Mult, 1, start)),
            b'=' if rest.len() > 1 && rest[1] == b'=' => Ok((Lx::Equal, 2, start)),
            // rtcl extensions: glob/regexp match (tclsh has no `=` alone,
            // so bare `=` stays Incomplete for it).
            b'=' if rest.len() > 1 && rest[1] == b'*' => Ok((Lx::GlobMatch, 2, start)),
            b'=' if rest.len() > 1 && rest[1] == b'~' => Ok((Lx::ReMatch, 2, start)),
            b'=' => Ok((Lx::Incomplete, 1, start)),
            b'!' if rest.len() > 1 && rest[1] == b'=' => Ok((Lx::Neq, 2, start)),
            b'!' => Ok((Lx::Not, 1, start)),
            b'&' if rest.len() > 1 && rest[1] == b'&' => Ok((Lx::And, 2, start)),
            b'&' => Ok((Lx::BitAnd, 1, start)),
            b'|' if rest.len() > 1 && rest[1] == b'|' => Ok((Lx::Or, 2, start)),
            b'|' => Ok((Lx::BitOr, 1, start)),
            // rotate 扩展（`<<<`/`>>>`）已移除——tclsh 词法把 `1 <<< 2`
            // 解析为 `<<` 后随非法操作数（bn13）。
            b'<' if rest.len() > 1 && rest[1] == b'<' => Ok((Lx::LeftShift, 2, start)),
            b'<' if rest.len() > 1 && rest[1] == b'=' => Ok((Lx::Leq, 2, start)),
            b'<' => Ok((Lx::Less, 1, start)),
            b'>' if rest.len() > 1 && rest[1] == b'>' => Ok((Lx::RightShift, 2, start)),
            b'>' if rest.len() > 1 && rest[1] == b'=' => Ok((Lx::Geq, 2, start)),
            b'>' => Ok((Lx::Greater, 1, start)),
            b'(' => Ok((Lx::OpenParen, 1, start)),
            b')' => Ok((Lx::CloseParen, 1, start)),
            b'+' => Ok((Lx::Plus, 1, start)),
            b'-' => Ok((Lx::Minus, 1, start)),
            b',' => Ok((Lx::Comma, 1, start)),
            b'/' => Ok((Lx::Div, 1, start)),
            b'%' => Ok((Lx::Mod, 1, start)),
            b'^' => Ok((Lx::BitXor, 1, start)),
            b'~' => Ok((Lx::BitNot, 1, start)),
            b'?' => Ok((Lx::Question, 1, start)),
            b':' => Ok((Lx::Colon, 1, start)),
            _ => {
                // Two-letter word operators, only when not the start of
                // a longer bareword (a non-alpha third byte is fine).
                if rest.len() >= 2 && !rest.get(2).is_some_and(|&ch| ch.is_ascii_alphabetic()) {
                    let op = match &rest[..2] {
                        b"in" => Some(Lx::In),
                        b"eq" => Some(Lx::Streq),
                        b"ne" => Some(Lx::Strneq),
                        b"ni" => Some(Lx::Ni),
                        b"lt" => Some(Lx::StrLt),
                        b"le" => Some(Lx::StrLe),
                        b"gt" => Some(Lx::StrGt),
                        b"ge" => Some(Lx::StrGe),
                        _ => None,
                    };
                    if let Some(op) = op {
                        return Ok((op, 2, start));
                    }
                }
                // Number or bareword, with TclParseNumber's join rule.
                // Only digits/dot start a number scan — letter-led words
                // (`inf`, `nan`, `info`, `Influence`) are barewords, with
                // full-numeric words resolved later (resolve_bareword).
                if b.is_ascii_digit() || b == b'.' {
                    if let Some((len, dbl)) = scan_number(rest) {
                        let end = start + len;
                        if end >= n || !is_bareword(e[end]) {
                            return Ok((Lx::Number, len, start));
                        }
                        // A number followed directly by bareword
                        // characters.  A number whose tail is a word
                        // operator stays a number (`3eq2` → 0); a
                        // double-like token ends at the number and its
                        // tail scans as its own bareword (`1.8x` blames
                        // "x", `1.p` blames "p", `0x1.8p3` splits at the
                        // dot — `missing operator` there); an int plus a
                        // non-operator tail re-joins into one bareword
                        // (`5x` blames "5x").
                        if matches!(
                            probe_lexeme(&e[end..]),
                            Some(Lx::Streq)
                                | Some(Lx::Strneq)
                                | Some(Lx::In)
                                | Some(Lx::Ni)
                                | Some(Lx::StrLt)
                                | Some(Lx::StrLe)
                                | Some(Lx::StrGt)
                                | Some(Lx::StrGe)
                        ) {
                            return Ok((Lx::Number, len, start));
                        }
                        if dbl {
                            return Ok((Lx::Number, len, start));
                        }
                    }
                }
                if !is_bareword(b) || b == b'_' {
                    // INVALID: one UTF-8 character.
                    let mut i = 1;
                    while i < rest.len() && (rest[i] & 0xC0) == 0x80 {
                        i += 1;
                    }
                    return Ok((Lx::Invalid, i, start));
                }
                let mut i = 0;
                while i < rest.len() && is_bareword(rest[i]) {
                    i += 1;
                }
                Ok((Lx::Bareword, i, start))
            }
        }
    }
}

/// Classify the word-operator lexeme at a position following a number,
/// for the number/bareword join rule.
fn probe_lexeme(rest: &[u8]) -> Option<Lx> {
    if rest.len() < 2 {
        return None;
    }
    // A following alphabetic byte means the 2-letter form is a prefix of
    // a longer bareword — not an operator.
    if rest.get(2).is_some_and(|&ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    match &rest[..2] {
        b"in" => Some(Lx::In),
        b"eq" => Some(Lx::Streq),
        b"ne" => Some(Lx::Strneq),
        b"ni" => Some(Lx::Ni),
        b"lt" => Some(Lx::StrLt),
        b"le" => Some(Lx::StrLe),
        b"gt" => Some(Lx::StrGt),
        b"ge" => Some(Lx::StrGe),
        _ => None,
    }
}

/// Scan a quoted string from its opening `"`.  Returns the index of the
/// closing quote, or the scan error inside it.  Quoted strings process
/// `$` references, `[` scripts, and backslash escapes.
thread_local! {
    /// Nesting open in the scanners below.
    static SCAN_DEPTH: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// Scanner nesting accepted: the evaluation-nesting limit the same text
/// would hit when evaluated (the parser's own cap on `[` nesting).
const SCAN_NEST_LIMIT: u32 = 1000;

/// One nesting level of the mutually recursive scanners (a `[` inside a
/// quoted word, a `$` inside a script, …). They are free functions over a
/// byte slice, so the depth is thread-local. Each level is native
/// recursion: `expr` with 16,000 nested `[` ran the stack out. Past the
/// limit the scan fails with tclsh's evaluation-nesting error.
fn scan_nested(
    e: &[u8],
    open: usize,
    f: fn(&[u8], usize) -> Result<usize, ScanErr>,
) -> Result<usize, ScanErr> {
    let depth = SCAN_DEPTH.with(|d| d.get());
    if depth >= SCAN_NEST_LIMIT {
        return Err((
            "too many nested evaluations (infinite loop?)".to_string(),
            open,
            1,
        ));
    }
    SCAN_DEPTH.with(|d| d.set(depth + 1));
    let r = f(e, open);
    SCAN_DEPTH.with(|d| d.set(d.get() - 1));
    r
}

fn scan_quoted(e: &[u8], open: usize) -> Result<usize, ScanErr> {
    let mut i = open + 1;
    while i < e.len() {
        match e[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i),
            b'[' => {
                let end = scan_nested(e, i, scan_script)?;
                i = end + 1;
            }
            b'$' => {
                let end = scan_nested(e, i, scan_variable)?;
                i = end;
            }
            _ => i += 1,
        }
    }
    Err(("missing \"".to_string(), open, 1))
}

/// Scan a braced body from its opening `{`.  Returns the index of the
/// matching `}`.
fn scan_braced(e: &[u8], open: usize) -> Result<usize, ScanErr> {
    let mut i = open + 1;
    let mut depth = 1usize;
    while i < e.len() {
        match e[i] {
            b'\\' => i += 2,
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    Err(("missing close-brace".to_string(), open, 1))
}

/// Is `b` a valid byte to follow a complete word inside a script?
fn word_boundary(b: u8) -> bool {
    is_tcl_ws(b) || b == b';' || b == b']'
}

/// Scan a `[` script with Tcl word rules inside: quoted and braced words
/// start only at word boundaries (mid-word quotes/braces are literal),
/// with the extra-characters rule after their close; `$` references and
/// nested `[` scripts work anywhere.  `{*}` at a word start followed by
/// non-whitespace that isn't a command end is word expansion (tclParse.c
/// Tcl_ParseCommand): the rest of the word re-parses fresh.  Returns the
/// index of the matching `]`.
fn scan_script(e: &[u8], open: usize) -> Result<usize, ScanErr> {
    let mut i = open + 1;
    let mut word_start = true;
    while i < e.len() {
        if e[i] == b']' {
            return Ok(i);
        }
        if word_boundary(e[i]) {
            word_start = true;
            i += 1;
            continue;
        }
        match e[i] {
            b'\\' => {
                i += 2;
                word_start = false;
            }
            b'[' => {
                let end = scan_nested(e, i, scan_script)?;
                i = end + 1;
                word_start = false;
            }
            b'"' | b'{' if word_start => {
                let (closed, reason) = if e[i] == b'"' {
                    (scan_nested(e, i, scan_quoted)?, "extra characters after close-quote")
                } else {
                    (scan_braced(e, i)?, "extra characters after close-brace")
                };
                let after = closed + 1;
                // Expansion prefix: a braced word containing exactly `*`
                // with a non-command-end character right behind it.
                let expands = e[i] == b'{'
                    && closed == i + 2
                    && e[i + 1] == b'*'
                    && after < e.len()
                    && !word_boundary(e[after])
                    && e[after] != b';';
                if expands {
                    word_start = true;
                } else if after < e.len() && !word_boundary(e[after]) {
                    return Err((reason.to_string(), after, 0));
                } else {
                    word_start = false;
                }
                i = after;
            }
            b'$' => {
                let end = scan_nested(e, i, scan_variable)?;
                i = end;
                word_start = false;
            }
            _ => {
                i += 1;
                word_start = false;
            }
        }
    }
    Err(("missing close-bracket".to_string(), open, 1))
}

/// Scan a `$` variable reference: `$name`, `${...}`, or `$name(index)`.
/// Returns the end offset.  `$` with no name raises `invalid
/// character "$"`; an unterminated array index raises `missing )`
/// blaming the `(`.  Index text follows tclsh 8.6 rules: only `)`
/// terminates it, quotes and braces are plain text, while `$`
/// references, `[` scripts, and backslash escapes substitute.
fn scan_variable(e: &[u8], dollar: usize) -> Result<usize, ScanErr> {
    let mut i = dollar + 1;
    if i >= e.len() {
        return Err(("invalid character \"$\"".to_string(), dollar, 1));
    }
    if e[i] == b'{' {
        let closed = scan_braced(e, i)?;
        return Ok(closed + 1);
    }
    while i < e.len()
        && (e[i].is_ascii_alphanumeric()
            || e[i] == b'_'
            || e[i] == b':'
            || e[i] >= 0x80)
    {
        i += 1;
    }
    if i < e.len() && e[i] == b'(' {
        // Array index: scan to the terminating `)`.
        let open_paren = i;
        i += 1;
        while i < e.len() {
            match e[i] {
                b')' => return Ok(i + 1),
                b'\\' => i += 2,
                b'[' => {
                    let end = scan_nested(e, i, scan_script)?;
                    i = end + 1;
                }
                b'$' => {
                    let end = scan_nested(e, i, scan_variable)?;
                    i = end;
                }
                // 9.1 index grammar（tclParse.c TYPE_BAD_ARRAY_INDEX =
                // OPEN_PAREN | CLOSE_PAREN | QUOTE | BRACE）：未引号下标
                // 文本里的 ( " { } 直接报 invalid character in array
                // index（parseExpr-21.44 的 `"foo$bar(abc…"` 钉形），
                // blame 前括号、scanned 含非法字符本身。
                b'(' | b'"' | b'{' | b'}' => {
                    return Err((
                        "invalid character in array index".to_string(),
                        open_paren,
                        i - open_paren + 1,
                    ));
                }
                _ => i += 1,
            }
        }
        return Err(("missing )".to_string(), open_paren, 1));
    }
    if i == dollar + 1 {
        // `$` with no name at all.
        return Err(("invalid character \"$\"".to_string(), dollar, 1));
    }
    Ok(i)
}

/// TclParseNumber subset: recognize a numeric token at the start of
/// `s`.  Returns (length, is_double_like).
fn scan_number(s: &[u8]) -> Option<(usize, bool)> {
    if s.is_empty() {
        return None;
    }
    let mut low = [0u8; 8];
    let take = s.len().min(8);
    low[..take].copy_from_slice(&s[..take].to_ascii_lowercase());
    for word in ["infinity", "inf", "nan"] {
        if low.starts_with(word.as_bytes()) {
            return Some((word.len(), true));
        }
    }
    // The shared 9.1 grammar scanner (bignum::scan_number): bare leading
    // zeros are decimal, 0d compels decimal, `_` separators are validated
    // between digits (misplaced ones — `1_`, `1_x` — kill the whole
    // token, so the lexeme falls through to the bareword branch and
    // tclsh's invalid-bareword error). The returned stop is
    // TclParseNumber's acceptPoint, i.e. exactly the lexeme length the C
    // lexer uses.
    let text = core::str::from_utf8(s).ok()?;
    let scan = crate::types::bignum::scan_number(text, false)?;
    Some((
        scan.stop,
        matches!(
            scan.payload,
            crate::types::bignum::NumPayload::Float { .. }
        ),
    ))
}

/// The `(parsing expression "...")` errorInfo frame body (limit 25).
pub fn parsing_frame(expr: &str) -> String {
    let b = expr.as_bytes();
    if b.len() < LIMIT {
        format!("    (parsing expression \"{}\")", String::from_utf8_lossy(b))
    } else {
        format!(
            "    (parsing expression \"{}...\")",
            String::from_utf8_lossy(&b[..HEAD])
        )
    }
}

