//! Regular expression commands: regexp, regsub.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

#[cfg(feature = "regexp")]
use regex::Regex;
#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
use regex_lite::Regex;

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Escape regex metacharacters (for Tcl's `***=` literal prefix).
fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 把 Tcl ARE 的 `\uHHHH`/`\UHHHHHHHH`/`\xHH` 转成引擎可吃的字面字符
/// （regex crate 只认 `\x{...}`，BT 引擎的 unescape 不认 `\u`）。
/// 合法码点用 `regex_escape` 输出（保持元字符的字面语义）；代理区
/// （U+D800..DFFF）在 Tcl 里永不匹配：类外输出 `(?!)`、类内直接省略。
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn expand_unicode_escapes(pat: &str) -> (String, bool) {
    let b = pat.as_bytes();
    let mut out = String::with_capacity(pat.len());
    let mut i = 0usize;
    let mut in_class = false;
    // 反向引用需要 BT 引擎（fast 引擎不支持）；同时用于 Tcl 的
    // 八进制/反向引用启发式（多位数按已闭合组数判定）。
    let mut needs_bt = false;
    let mut paren_stack: Vec<bool> = Vec::new();
    let mut groups_closed = 0usize;
    while i < b.len() {
        if !in_class {
            match b[i] {
                b'[' => {
                    in_class = true;
                    out.push('[');
                    i += 1;
                    continue;
                }
                b'(' => {
                    paren_stack.push(b.get(i + 1) != Some(&b'?'));
                    out.push('(');
                    i += 1;
                    continue;
                }
                b')' => {
                    if paren_stack.pop().unwrap_or(false) {
                        groups_closed += 1;
                    }
                    out.push(')');
                    i += 1;
                    continue;
                }
                _ => {}
            }
        } else if b[i] == b']' {
            in_class = false;
            out.push(']');
            i += 1;
            continue;
        }
        if b[i] != b'\\' {
            let ch = pat[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let Some(ech) = pat[i + 1..].chars().next() else {
            out.push('\\');
            break;
        };
        // Tcl ARE 的八进制/反向引用启发式：整个数字串作为一个数——
        // 非零且 ≤ 已闭合组数 → 反向引用（留给 BT）；否则八进制
        // （最多 3 位）+ 其余字面。单个非零数字总是反向引用。
        if ech.is_ascii_digit() {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let run = &b[i + 1..j];
            let leading_zero = run[0] == b'0';
            let oct_start = (b'0'..=b'7').contains(&run[0]);
            let mut val = 0usize;
            let mut overflow = false;
            for d in run {
                val = val.saturating_mul(10).saturating_add((d - b'0') as usize);
                if val == usize::MAX {
                    overflow = true;
                }
            }
            let backref = !in_class
                && !leading_zero
                && (run.len() == 1 || (!overflow && val >= 1 && val <= groups_closed));
            if backref {
                needs_bt = true;
                out.push('\\');
                out.push_str(std::str::from_utf8(run).unwrap_or(""));
                i = j;
                continue;
            }
            if !leading_zero && !oct_start {
                // 校验阶段已拒绝（invalid escape）；保守保留原文。
                out.push('\\');
                out.push_str(std::str::from_utf8(run).unwrap_or(""));
                i = j;
                continue;
            }
            let mut k = i + 1;
            let mut oct = 0u32;
            let mut n = 0;
            while n < 3 && k < b.len() && (b[k] as char).is_digit(8) {
                oct = oct * 8 + (b[k] - b'0') as u32;
                k += 1;
                n += 1;
            }
            if let Some(c) = char::from_u32(oct) {
                out.push_str(&regex_escape(&c.to_string()));
            }
            for d in &b[k..j] {
                out.push(*d as char);
            }
            i = j;
            continue;
        }
        // 字符入口转义：\b = backspace、\B = 反斜杠、\a = bell
        // （与 Perl 的 word-boundary 语义不同）。
        if ech == 'b' {
            out.push_str("\\x08");
            i += 2;
            continue;
        }
        if ech == 'B' {
            out.push_str("\\\\");
            i += 2;
            continue;
        }
        if ech == 'a' {
            out.push_str("\\x07");
            i += 2;
            continue;
        }
        if ech == 'u' || ech == 'U' || ech == 'x' {
            let max = if ech == 'u' {
                4
            } else if ech == 'U' {
                8
            } else {
                2
            };
            let mut j = i + 2;
            let mut v = 0u32;
            let mut n = 0;
            while n < max && j < b.len() {
                let d = (b[j] as char).to_digit(16);
                match d {
                    Some(d) => {
                        v = v * 16 + d;
                        j += 1;
                        n += 1;
                    }
                    None => break,
                }
            }
            if n == 0 {
                out.push('\\');
                out.push(ech);
                i += 2;
                continue;
            }
            match char::from_u32(v) {
                Some(ch) => out.push_str(&regex_escape(&ch.to_string())),
                // 代理区（U+D800..DFFF）在 Tcl 中永不匹配；Rust 字符串
                // 也不能含代理。类内没有"永不匹配元素"的表示，用
                // U+10FFFF 占位（实际文本几乎不会包含）。
                None if in_class => out.push_str("\\x{10FFFF}"),
                None => out.push_str("(?!)"),
            }
            i = j;
            continue;
        }
        out.push('\\');
        out.push(ech);
        i += 1 + ech.len_utf8();
    }
    (out, needs_bt)
}

/// Quantifier-arrival state for the ARE scan: a quantifier must follow an
/// operand; `?` right after a quantifier is its lazy marker.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
#[derive(PartialEq, Clone, Copy)]
enum ArePrev {
    None,
    Operand,
    Quant,
    Lazy,
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// 校验一个反斜杠转义（`b[i] == b'\\'`），返回转义后的位置。
/// Tcl 8.6.17 ARE 实测：`\u` 收 1-4 hex、`\U` 收 1-8 hex、`\x` 收
/// 1-2 hex；`\x{...}` **不是** 8.6 语法（invalid escape）；其余字母
/// 转义必须在 Tcl 的已知表内，标点转义按字面。
fn are_escape_end(b: &[u8], i: usize) -> std::result::Result<usize, (&'static str, &'static str)> {
    let err = || ("REG_EESCAPE", "invalid escape \\ sequence");
    if i + 1 >= b.len() {
        return Err(err());
    }
    let esc = b[i + 1];
    match esc {
        b'u' | b'U' | b'x' => {
            let max = if esc == b'u' {
                4
            } else if esc == b'U' {
                8
            } else {
                2
            };
            if esc == b'x' && i + 2 < b.len() && b[i + 2] == b'{' {
                return Err(err());
            }
            let mut j = i + 2;
            let mut n = 0;
            while n < max && j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
                n += 1;
            }
            if n == 0 {
                return Err(err());
            }
            Ok(j)
        }
        _ => {
            if esc.is_ascii_alphanumeric() && !esc.is_ascii_digit() {
                // 8.6.17：`\z` 不是合法转义（\Z 才是串尾）；字符入口
                // 转义含 \a（bell）、\b（backspace）、\B（反斜杠）。
                if esc == b'z'
                    || !b"adDsSwWbBAZzmMyYCNceNntrfve".contains(&esc)
                {
                    return Err(err());
                }
            }
            Ok(i + 2)
        }
    }
}

/// Left-to-right Tcl-ARE compile-time validation: `Some((REG code,
/// message))` for patterns tclsh's regexp compiler rejects before
/// matching — `regexp {a**} x` → "quantifier operand invalid"
/// (REG_BADRPT), `{a{2,1}}` → REG_BADBR, `{a\}` → REG_EESCAPE, `{[[:foo:]]}`
/// → REG_ECTYPE, unbalanced `(`/`[` → REG_EPAREN/REG_EBRACK.  rtcl's
/// engines alone are more lenient and would silently accept some of
/// these.  Only the six rejection classes are detected; everything else
/// keeps the engines' own behavior.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
pub(crate) fn validate_are(pattern: &str) -> Option<(&'static str, String)> {
    const POSIX_CLASSES: &[&str] = &[
        "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
        "upper", "word", "xdigit", "<", ">",
    ];
    let b = pattern.as_bytes();
    // Tcl's `***=` literal prefix: everything after it is literal text —
    // no ARE syntax to validate (regexp-1.8: `regexp -- "***=o"`).
    if b.starts_with(b"***=") {
        return None;
    }
    let mut i = 0usize;
    let mut prev = ArePrev::None;
    let mut depth = 0usize;
    // 捕获组栈：`)` 闭合时 groups_closed += 1；反向引用只能指向
    // **已经闭合**的组（tclsh 8.6.17：`(\1)` 报 invalid backreference）。
    let mut paren_stack: Vec<bool> = Vec::new();
    let mut groups_closed = 0usize;
    while i < b.len() {
        match b[i] {
            b'\\' => match are_escape_end(b, i) {
                Ok(j) => {
                    let esc = b[i + 1];
                    if esc.is_ascii_digit() && esc != b'0' {
                        // 整个数字串：≤ 已闭合组数 → 反向引用；否则按
                        // 八进制转义（Spencer 启发式）。单个非零数字
                        // 必为反向引用（无合法组 → REG_ESUBREG）。
                        let mut j = i + 2;
                        while j < b.len() && b[j].is_ascii_digit() {
                            j += 1;
                        }
                        let run = &b[i + 1..j];
                        let mut val = 0usize;
                        let mut overflow = false;
                        for d in run {
                            val = val.saturating_mul(10).saturating_add((d - b'0') as usize);
                            if val == usize::MAX {
                                overflow = true;
                            }
                        }
                        if run.len() == 1 {
                            if !overflow && val > groups_closed {
                                return Some((
                                    "REG_ESUBREG",
                                    "invalid backreference number".to_string(),
                                ));
                            }
                        } else {
                            let valid_backref =
                                !overflow && val <= groups_closed;
                            // 非反向引用的多位数：按最多 3 位八进制 +
                            // 余字面；以 8/9 开头则无八进制数字 → invalid。
                            if !valid_backref
                                && !(b'0'..=b'7').contains(&run[0])
                            {
                                return Some((
                                    "REG_EESCAPE",
                                    "invalid escape \\ sequence".to_string(),
                                ));
                            }
                        }
                    }
                    i = j;
                    prev = ArePrev::Operand;
                }
                Err((code, msg)) => return Some((code, msg.to_string())),
            },
            b'[' => {
                // Bracket expression: scan to its `]` (a leading `]`,
                // after optional `^`, is a literal), validating
                // [:class:] runs along the way.
                let mut j = i + 1;
                if j < b.len() && b[j] == b'^' {
                    j += 1;
                }
                if j < b.len() && b[j] == b']' {
                    j += 1;
                }
                let mut closed = false;
                while j < b.len() {
                    if b[j] == b']' {
                        closed = true;
                        j += 1;
                        break;
                    }
                    if b[j] == b'[' && j + 1 < b.len() && b[j + 1] == b':' {
                        if let Some(rel) = b[j + 2..].windows(2).position(|w| w == b":]") {
                            let name = String::from_utf8_lossy(&b[j + 2..j + 2 + rel]);
                            if !POSIX_CLASSES.contains(&name.as_ref()) {
                                return Some((
                                    "REG_ECTYPE",
                                    "invalid character class".to_string(),
                                ));
                            }
                            j += 2 + rel + 2;
                            continue;
                        }
                    }
                    if b[j] == b'\\' {
                        match are_escape_end(b, j) {
                            Ok(nj) => {
                                // 类内：\0.. 与多位数按八进制；单个非零
                                // 数字是非法反向引用 → invalid escape；
                                // 多位含 8/9 也无法作八进制。
                                let esc = b[j + 1];
                                if esc.is_ascii_digit() && esc != b'0' {
                                    let mut k = j + 2;
                                    while k < b.len() && b[k].is_ascii_digit() {
                                        k += 1;
                                    }
                                    // 单个非零数字=非法反向引用；多位以
                                    // 8/9 开头无法按八进制 → invalid
                                    // escape；否则前 3 位八进制 + 余字面。
                                    if k == j + 2 || !(b'0'..=b'7').contains(&esc) {
                                        return Some((
                                            "REG_EESCAPE",
                                            "invalid escape \\ sequence".to_string(),
                                        ));
                                    }
                                }
                                j = nj;
                            }
                            Err((code, msg)) => return Some((code, msg.to_string())),
                        }
                    } else {
                        j += 1;
                    }
                }
                if !closed {
                    return Some((
                        "REG_EBRACK",
                        "brackets [] not balanced".to_string(),
                    ));
                }
                i = j;
                prev = ArePrev::Operand;
            }
            b'(' => {
                // D-17 对齐：lookbehind（(?<= / (?<!) 非 8.6 ARE 语法——
                // tclsh 编译报 quantifier operand invalid（REG_BADRPT）。
                // 前瞻 (?= / (?! 合法（lb3/lb4 实测）。
                if i + 2 < b.len()
                    && b[i + 1] == b'?'
                    && (b[i + 2] == b'<' || b[i + 2] == b'=' || b[i + 2] == b'!')
                {
                    if b[i + 2] == b'<' {
                        return Some((
                            "REG_BADRPT",
                            "quantifier operand invalid".to_string(),
                        ));
                    }
                }
                paren_stack.push(b.get(i + 1) != Some(&b'?'));
                depth += 1;
                i += 1;
                prev = ArePrev::None;
            }
            b')' => {
                if depth == 0 {
                    return Some((
                        "REG_EPAREN",
                        "parentheses () not balanced".to_string(),
                    ));
                }
                depth -= 1;
                if paren_stack.pop().unwrap_or(false) {
                    groups_closed += 1;
                }
                i += 1;
                prev = ArePrev::Operand;
            }
            b'*' | b'+' => match prev {
                ArePrev::Operand => {
                    prev = ArePrev::Quant;
                    i += 1;
                }
                _ => {
                    return Some((
                        "REG_BADRPT",
                        "quantifier operand invalid".to_string(),
                    ))
                }
            },
            b'?' => match prev {
                ArePrev::Operand => {
                    prev = ArePrev::Quant;
                    i += 1;
                }
                ArePrev::Quant => {
                    prev = ArePrev::Lazy;
                    i += 1;
                }
                // `(?:`, `(?i`, `(?=` — group syntax, not a quantifier.
                ArePrev::None if i > 0 && b[i - 1] == b'(' => {
                    i += if i + 1 < b.len() && b[i + 1] == b':' { 2 } else { 1 };
                    continue;
                }
                _ => {
                    return Some((
                        "REG_BADRPT",
                        "quantifier operand invalid".to_string(),
                    ))
                }
            },
            b'{' if prev == ArePrev::Operand => {
                // Bound `{m}` `{m,}` `{m,n}` `{,n}`: well-formed bounds
                // with m > n are rejected; malformed braces are literals.
                let mut j = i + 1;
                let mut m: usize = 0;
                let mut m_len = 0usize;
                while j < b.len() && b[j].is_ascii_digit() {
                    m = m * 10 + (b[j] - b'0') as usize;
                    j += 1;
                    m_len += 1;
                }
                let mut n: Option<usize> = None;
                let mut well_formed = m_len > 0;
                if j < b.len() && b[j] == b',' {
                    well_formed = true;
                    let mut k = j + 1;
                    let mut nv: usize = 0;
                    let mut n_len = 0usize;
                    while k < b.len() && b[k].is_ascii_digit() {
                        nv = nv * 10 + (b[k] - b'0') as usize;
                        k += 1;
                        n_len += 1;
                    }
                    if n_len > 0 {
                        n = Some(nv);
                    }
                    j = k;
                }
                if well_formed && j < b.len() && b[j] == b'}' {
                    if let Some(nv) = n {
                        if m > nv {
                            return Some((
                                "REG_BADBR",
                                "invalid repetition count(s)".to_string(),
                            ));
                        }
                    }
                    i = j + 1;
                    prev = ArePrev::Quant;
                } else {
                    i += 1;
                    prev = ArePrev::Operand;
                }
            }
            b'|' => {
                i += 1;
                prev = ArePrev::None;
            }
            // Anchors and literal characters are operands (lenient: a
            // quantifier on an anchor keeps today's engine behavior).
            _ => {
                i += 1;
                prev = ArePrev::Operand;
            }
        }
    }
    if depth > 0 {
        return Some((
            "REG_EPAREN",
            "parentheses () not balanced".to_string(),
        ));
    }
    None
}

/// `compile_engine` plus tclsh's compile-failure contract: an ARE the
/// validator rejects yields exactly tclsh's message ("couldn't compile
/// regular expression pattern: ...") and a `REGEXP REG_* {...}`
/// errorCode.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
pub(crate) fn compile_engine_tc(
    interp: &mut Interp,
    pattern: &str,
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> std::result::Result<Engine, String> {
    // M5：默认整条 regexp/regsub 路径走 Spencer 移植（其 parser 自带
    // tclsh 同源的校验与 REG_* 码）；lite 构建保持单一 regex_lite。
    #[cfg(all(feature = "regexp", feature = "regexp-spencer"))]
    if super::regexp_spencer::engine_selected() {
        return match super::regexp_spencer::compile(
            pattern,
            spencer_options(nocase, expanded, lineanchor, linestop),
        ) {
            Ok(prog) => Ok(Engine::Spencer(Box::new(prog))),
            Err(e) => {
                super::list::set_error_code(
                    interp,
                    &format!("REGEXP {} {{{}}}", e.code, e.message),
                );
                Err(format!(
                    "couldn't compile regular expression pattern: {}",
                    e.message
                ))
            }
        };
    }
    if let Some((code, msg)) = validate_are(pattern) {
        super::list::set_error_code(interp, &format!("REGEXP {} {{{}}}", code, msg));
        return Err(format!(
            "couldn't compile regular expression pattern: {}",
            msg
        ));
    }
    compile_engine(pattern, nocase, expanded, lineanchor, linestop)
}

/// 命令行开关 → Spencer `Options`。`-line` 同时置 nlanch/nlstop。
#[cfg(feature = "regexp-spencer")]
fn spencer_options(
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> super::regexp_spencer::Options {
    super::regexp_spencer::Options {
        icase: nocase,
        expanded,
        nlanch: lineanchor,
        nlstop: linestop,
        ..Default::default()
    }
}

/// Build regex pattern string from flags.  Tcl's newline sensitivity
/// differs from Rust's defaults: by default `.` matches newline (Rust's
/// `(?s)`), `-linestop` restores Rust's plain behavior, and
/// `-lineanchor`/`-line` make `^`/`$` match at line boundaries (`(?m)`).
/// A leading `***=` makes the rest of the pattern a literal string.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
pub(crate) fn build_pattern(
    pattern: &str,
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> String {
    let pattern = match pattern.strip_prefix("***=") {
        Some(rest) => regex_escape(rest),
        None => pattern.to_string(),
    };
    let mut prefix = String::new();
    if nocase { prefix.push_str("(?i)"); }
    if expanded { prefix.push_str("(?x)"); }
    if lineanchor { prefix.push_str("(?m)"); }
    if !linestop { prefix.push_str("(?s)"); }
    format!("{}{}", prefix, pattern)
}

/// The dual regex engine: the `regex` crate is the fast path; patterns it
/// refuses (backreferences, look-around, `\m\M\y\Y`, `(?b)` BRE mode,
/// literal braces) fall back to the hand-rolled backtracking engine in
/// [`super::regexp_bt`], which implements Tcl ARE semantics.
#[cfg(feature = "regexp")]
pub(crate) enum Engine {
    Fast {
        re: regex::Regex,
        anch: Option<regex::Regex>,
    },
    Full(super::regexp_bt::BtProg),
    /// Henry Spencer 引擎 Rust 移植（M5：feature `regexp-spencer`
    /// + `RTCL_REGEX_ENGINE=spencer` 灰度路由）。
    #[cfg(feature = "regexp-spencer")]
    Spencer(Box<super::regexp_spencer::Prog>),
}

/// Does the pattern use constructs whose membership follows tclsh's Unicode
/// category tables? POSIX `[[:class:]]` bracket classes are ASCII-only in the
/// `regex` crate but Unicode-aware in Tcl, and `\w`/`\W`/`\s`/`\S` disagree
/// with Tcl's tables on marks, format chars and connector punctuation —
/// such patterns are routed to the backtracking engine, which shares
/// `crate::interp::unicode` with `string is`.
#[cfg(feature = "regexp")]
fn uses_tcl_unicode_classes(pat: &str) -> bool {
    let b = pat.as_bytes();
    let (mut in_class, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() => {
                match b[i + 1] {
                    // `\\w` is a literal backslash + `w`, not the class.
                    b'\\' => i += 2,
                    b'w' | b'W' | b's' | b'S' => return true,
                    _ => i += 2,
                }
            }
            b'[' => {
                in_class = true;
                i += 1;
            }
            b':' if in_class && i >= 2 && b[i - 1] == b'[' => return true, // [[:class:]]
            b']' => {
                in_class = false;
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// Compile with the fast engine first; on a compile error, retry with the
/// backtracking engine. The reported error is the fast engine's.
#[cfg(feature = "regexp")]
pub(crate) fn compile_engine(
    pattern: &str,
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> std::result::Result<Engine, String> {
    // 无 Interp 的消费者（array names -regexp、lsort -regexp）也
    // 遵循同一灰度开关，保证整进程语义一致。
    #[cfg(feature = "regexp-spencer")]
    if super::regexp_spencer::engine_selected() {
        return super::regexp_spencer::compile(
            pattern,
            spencer_options(nocase, expanded, lineanchor, linestop),
        )
        .map(|prog| Engine::Spencer(Box::new(prog)))
        .map_err(|e| {
            format!("couldn't compile regular expression pattern: {}", e.message)
        });
    }
    let (expanded_pattern, needs_bt) = expand_unicode_escapes(pattern);
    let pattern = expanded_pattern.as_str();
    if needs_bt {
        return super::regexp_bt::bt_compile(pattern, nocase, expanded, lineanchor, linestop)
            .map(Engine::Full);
    }
    // Tcl-Unicode class semantics live only in the backtracking engine.
    if uses_tcl_unicode_classes(pattern) {
        return super::regexp_bt::bt_compile(pattern, nocase, expanded, lineanchor, linestop)
            .map(Engine::Full);
    }
    let built = build_pattern(pattern, nocase, expanded, lineanchor, linestop);
    match regex::Regex::new(&built) {
        Err(fe) => super::regexp_bt::bt_compile(pattern, nocase, expanded, lineanchor, linestop)
            .map(Engine::Full)
            .map_err(|_| fe.to_string()),
        Ok(re) => {
            let anch = if has_string_anchor(pattern) {
                Some(
                    regex::Regex::new(&built.replace("\\A", "^"))
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            Ok(Engine::Fast { re, anch })
        }
    }
}

/// `-all` 扫描期间复用的字符/字节偏移与 Matcher（DFA 缓存）缓存。
/// 避免每个起点都从 UTF-8 重建 Vec<char>、重建 DFA 缓存。
#[cfg(all(feature = "regexp-spencer", feature = "regexp"))]
pub(crate) struct CharCache {
    text: Option<(Vec<char>, Vec<usize>)>,
    matcher: Option<super::regexp_spencer::matcher::Matcher>,
    /// 全串是否 ASCII（必需字符预过滤可用字节 memchr 快路径）。
    ascii: Option<bool>,
}

#[cfg(all(feature = "regexp-spencer", feature = "regexp"))]
impl CharCache {
    pub(crate) fn new() -> CharCache {
        CharCache {
            text: None,
            matcher: None,
            ascii: None,
        }
    }
}

#[cfg(not(all(feature = "regexp-spencer", feature = "regexp")))]
pub(crate) struct CharCache;

#[cfg(not(all(feature = "regexp-spencer", feature = "regexp")))]
impl CharCache {
    pub(crate) fn new() -> CharCache {
        CharCache
    }
}

/// Attempt one match at char index `p` of `full_string`; returns capture
/// ranges in full-string byte offsets, group 0 first.  `search_start` is
/// the `-start` char offset: `\A` binds it (Spencer Bot assertion), while
/// `^` stays anchored to absolute position 0.
#[cfg(feature = "regexp")]
pub(crate) fn engine_attempt(
    eng: &Engine,
    full_string: &str,
    p: usize,
    search_start: usize,
    cache: &mut CharCache,
) -> Option<GroupRanges> {
    let _ = &cache;
    match eng {
        #[cfg(feature = "regexp-spencer")]
        Engine::Spencer(prog) => {
            if cache.text.is_none() {
                let chars: Vec<char> = full_string.chars().collect();
                let mut byte_at: Vec<usize> = full_string
                    .char_indices()
                    .map(|(b, _)| b)
                    .collect();
                byte_at.push(full_string.len());
                cache.text = Some((chars, byte_at));
            }
            if cache.matcher.is_none() {
                cache.matcher =
                    Some(super::regexp_spencer::matcher::Matcher::new(prog));
            }
            let (chars, byte_at) = cache.text.as_ref().unwrap();
            // 必需字面预过滤：ASCII 非 -nocase 用 str::contains
            // （memmem）；否则退回必需字符/折叠变体检查。
            if cache.ascii.is_none() {
                cache.ascii = Some(full_string.is_ascii());
            }
            let ascii = cache.ascii == Some(true);
            let from = p.min(chars.len());
            let mut filtered = false;
            if !prog.icase {
                if let Some(lit) = &prog.required_literal {
                    let found = if ascii {
                        full_string[byte_at[from]..].contains(lit.as_str())
                    } else {
                        let lc: Vec<char> = lit.chars().collect();
                        chars[from..]
                            .windows(lc.len())
                            .any(|w| w == lc.as_slice())
                    };
                    if !found {
                        return None;
                    }
                    filtered = true;
                }
            }
            if !filtered {
                if let Some(c) = prog.required_char {
                    let found = if ascii && c.is_ascii() {
                        let suffix = &full_string[byte_at[from]..];
                        let mut hit = suffix.contains(c);
                        if !hit && prog.icase {
                            hit = suffix.contains(c.to_ascii_lowercase())
                                || suffix.contains(c.to_ascii_uppercase());
                        }
                        hit
                    } else {
                        chars[from..].iter().any(|&ch| {
                            ch == c
                                || (prog.icase
                                    && (ch.to_lowercase().next() == Some(c)
                                        || ch.to_uppercase().next() == Some(c)))
                        })
                    };
                    if !found {
                        return None;
                    }
                }
            }
            let matcher = cache.matcher.as_mut().unwrap();
            // 无捕获组时跳过 dissect，只取整体 span（常见类/字面模式
            // 的 -all 热路径）。
            if prog.nsubs == 0 {
                let m = matcher.find(prog, chars, p, search_start)?;
                return Some(vec![Some((byte_at[m.start], byte_at[m.end]))]);
            }
            let caps = super::regexp_spencer::captures::exec_with(
                prog,
                chars,
                p,
                search_start,
                matcher,
            )?;
            Some(
                caps.spans
                    .into_iter()
                    .map(|g| g.map(|(s, e)| (byte_at[s], byte_at[e])))
                    .collect(),
            )
        }
        Engine::Full(prog) => {
            let off = char_to_byte(full_string, p);
            super::regexp_bt::bt_caps_at(prog, &full_string[off..]).map(|groups| {
                groups
                    .into_iter()
                    .map(|g| g.map(|(s, e)| (s + off, e + off)))
                    .collect()
            })
        }
        Engine::Fast { re, anch } => match anch {
            Some(ra) => {
                // D-20：串锚匹配必须**起于 slice 0**——captures 会命中
                // slice 内部的 (?m)^ 行首（全串偏移越界泄漏），故过滤。
                let off = char_to_byte(full_string, p);
                let slice = &full_string[off..];
                let caps = ra
                    .captures(slice)
                    .filter(|c| c.get(0).map(|m| m.start() == 0).unwrap_or(false));
                caps.map(|caps| {
                    (0..caps.len())
                        .map(|j| caps.get(j).map(|m| (m.start() + off, m.end() + off)))
                        .collect()
                })
            }
            None => regex_caps_at(re, full_string, char_to_byte(full_string, p)),
        },
    }
}

#[cfg(feature = "regexp")]
pub(crate) fn engine_is_match(eng: &Engine, s: &str) -> bool {
    match eng {
        Engine::Fast { re, .. } => re.is_match(s),
        Engine::Full(p) => super::regexp_bt::bt_is_match(p, s),
        #[cfg(feature = "regexp-spencer")]
        Engine::Spencer(p) => super::regexp_spencer::is_match(p, s, 0),
    }
}

#[cfg(feature = "regexp")]
pub(crate) fn engine_group_count(eng: &Engine, pattern: &str) -> usize {
    match eng {
        Engine::Fast { .. } => count_groups(pattern),
        Engine::Full(p) => p.ngroups,
        #[cfg(feature = "regexp-spencer")]
        Engine::Spencer(p) => p.nsubs,
    }
}

/// regexp-lite build (small-WASM path): single regex_lite engine, no
/// backtracking fallback — a pattern regex_lite refuses is a compile
/// error, mirroring the pre-dual-engine lite behavior.
#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
pub(crate) struct Engine {
    re: regex_lite::Regex,
    anch: Option<regex_lite::Regex>,
}

#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
pub(crate) fn compile_engine(
    pattern: &str,
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> std::result::Result<Engine, String> {
    let (expanded_pattern, _needs_bt) = expand_unicode_escapes(pattern);
    let pattern = expanded_pattern.as_str();
    let built = build_pattern(pattern, nocase, expanded, lineanchor, linestop);
    let re = regex_lite::Regex::new(&built).map_err(|e| e.to_string())?;
    let anch = if has_string_anchor(pattern) {
        Some(
            regex_lite::Regex::new(&built.replace("\\A", "^"))
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    Ok(Engine { re, anch })
}

/// Attempt one match at char index `p` of `full_string`; returns capture
/// ranges in full-string byte offsets, group 0 first.
#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
pub(crate) fn engine_attempt(
    eng: &Engine,
    full_string: &str,
    p: usize,
    _search_start: usize,
    _cache: &mut CharCache,
) -> Option<GroupRanges> {
    match &eng.anch {
        Some(ra) => {
            // D-20：同上——串锚捕获起于 slice 0。
            let off = char_to_byte(full_string, p);
            let slice = &full_string[off..];
            let caps = ra
                .captures(slice)
                .filter(|c| c.get(0).map(|m| m.start() == 0).unwrap_or(false));
            caps.map(|caps| {
                (0..caps.len())
                    .map(|j| caps.get(j).map(|m| (m.start() + off, m.end() + off)))
                    .collect()
            })
        }
        None => regex_caps_at(&eng.re, full_string, char_to_byte(full_string, p)),
    }
}

#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
pub(crate) fn engine_group_count(_eng: &Engine, pattern: &str) -> usize {
    count_groups(pattern)
}

/// Parse a `-start` index with Tcl's index grammar
/// (`integer?[+-]integer?` or `end?[+-]integer?`).  tclsh resolves
/// `end` to the string *length* here (not length-1), so `-start end-1`
/// points at the last character.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Tcl 索引整数语法：可选符号 + C 风格进制（0x/0o/0b/前导 0 八进制），
/// 返回 i64；最终由 `wrap_tcl_index` 做 32 位回绕（tclsh 行为）。
fn parse_index_int(t: &str) -> Option<i64> {
    if t.is_empty() {
        return None;
    }
    let (neg, body) = match t.as_bytes()[0] {
        b'+' => (false, &t[1..]),
        b'-' => (true, &t[1..]),
        _ => (false, t),
    };
    if body.is_empty() {
        return None;
    }
    let (radix, digits) = if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        (16, r)
    } else if let Some(r) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        (8, r)
    } else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        (2, r)
    } else if body.len() > 1 && body.starts_with('0') {
        (8, &body[1..])
    } else {
        (10, body)
    };
    if digits.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for c in digits.chars() {
        let d = c.to_digit(radix)? as u64;
        v = v.checked_mul(radix as u64)?.checked_add(d)?;
    }
    if neg {
        Some(-(v as i64))
    } else {
        Some(v as i64)
    }
}

/// tclsh 的 -start 解析结果是 32 位整数：|v| <= 2^32-1 时按 i32 回绕，
/// 超出则报 bad index（`-2147483649`→2147483647、`4294967296`→错误）。
fn wrap_tcl_index(v: i64) -> Option<i64> {
    const LIMIT: i64 = 4_294_967_295;
    if v < -LIMIT || v > LIMIT {
        None
    } else {
        Some(v as i32 as i64)
    }
}

/// Parse a `-start` index with Tcl's index grammar
/// (`integer?[+-]integer?` or `end?[+-]integer?`).  tclsh resolves
/// `end` to the string *length* here (not length-1), so `-start end-1`
/// points at the last character.
fn parse_start_index(s: &str, len: usize) -> std::result::Result<usize, String> {
    let bad = || {
        format!(
            "bad index \"{}\": must be integer?[+-]integer? or end?[+-]integer?",
            s
        )
    };
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return Err(bad());
    }
    let idx = if let Some(rest) = t.strip_prefix("end") {
        let base = len as i64;
        if rest.is_empty() {
            base
        } else {
            let op = rest.as_bytes()[0];
            if op != b'+' && op != b'-' {
                return Err(bad());
            }
            let n = parse_index_int(&rest[1..]).ok_or_else(bad)?;
            if op == b'+' {
                base + n
            } else {
                base - n
            }
        }
    } else if let Some(v) = parse_index_int(t) {
        v
    } else {
        // int+int / int-int：分隔符后的部分本身可带符号
        // （`1+-2` = 1 + (-2)，`1--2` = 1 - (-2)）。
        let mut found = None;
        for (i, c) in t.char_indices() {
            if i == 0 || (c != '+' && c != '-') {
                continue;
            }
            let head = match parse_index_int(&t[..i]) {
                Some(v) => v,
                None => continue,
            };
            let tail = match parse_index_int(&t[i + 1..]) {
                Some(v) => v,
                None => continue,
            };
            found = Some(if c == '+' { head + tail } else { head - tail });
            break;
        }
        match found {
            Some(v) => v,
            None => return Err(bad()),
        }
    };
    let v = wrap_tcl_index(idx).ok_or_else(bad)?;
    Ok(v.max(0) as usize)
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Byte offset of char index `ci` (clamped to the string length).
fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices()
        .nth(ci)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Char index of byte offset `bi` (which must lie on a char boundary).
fn byte_to_char(s: &str, bi: usize) -> usize {
    // ASCII 快路径：字节下标即字符下标（大串 -indices/-all 的
    // 每个匹配都重扫前缀会让输出退化为 O(n²)）。
    if s.is_ascii() {
        return bi;
    }
    s[..bi].chars().count()
}

/// Captured groups of one match as byte ranges into the haystack the
/// search ran on; `None` for groups that did not participate.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
type GroupRanges = Vec<Option<(usize, usize)>>;

#[cfg(feature = "regexp")]
fn regex_caps_at(re: &Regex, text: &str, start: usize) -> Option<GroupRanges> {
    re.captures_at(text, start)
        .map(|caps| (0..caps.len()).map(|j| caps.get(j).map(|m| (m.start(), m.end()))).collect())
}
#[cfg(all(feature = "regexp-lite", not(feature = "regexp")))]
fn regex_caps_at(re: &Regex, text: &str, start: usize) -> Option<GroupRanges> {
    re.captures_at(text, start)
        .map(|caps| (0..caps.len()).map(|j| caps.get(j).map(|m| (m.start(), m.end()))).collect())
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Does the pattern use `\A` outside a character class?  Tcl binds
/// `\A` to the -start offset (the virtual string start), which the
/// engine models by searching a slice with `\A` rewritten to `^`.
fn has_string_anchor(pat: &str) -> bool {
    let b = pat.as_bytes();
    let mut in_class = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                if !in_class && i + 1 < b.len() && b[i + 1] == b'A' {
                    return true;
                }
                i += 2;
            }
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            _ => i += 1,
        }
    }
    false
}

/// Collect the matches of a pattern in `full` with Tcl's `-start` /
/// `-all` semantics: scanning starts at `char_start` (nothing matches
/// at all when it lies past the end); an empty match advances one
/// character; `-all` stops at the length (a lone empty string still
/// gets one attempt at its single position).  `attempt(p)` must return
/// the match groups starting the scan at char `p`, as byte ranges
/// relative to `full`.  `^`/`$` keep their true-string meaning (the
/// closure uses `captures_at`), while `\A` binds to each scan start
/// (the closure searches a slice).
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn collect_matches(
    full: &str,
    mut attempt: impl FnMut(usize) -> Option<GroupRanges>,
    char_len: usize,
    char_start: usize,
    all: bool,
    include_end: bool,
) -> Vec<GroupRanges> {
    let mut out: Vec<GroupRanges> = Vec::new();
    // regsub 的 -start 越过串尾时不执行（tclsh：原样返回）。
    if include_end && char_start > char_len {
        return out;
    }
    if !all {
        if let Some(groups) = attempt(char_start) {
            out.push(groups);
        }
        return out;
    }
    // regsub -all 的循环是 offset <= length（非空匹配到串尾后仍会
    // 再尝试一次空匹配）；regexp -all 只在首个位置允许 offset==length。
    let mut p = char_start;
    // 匹配端点单调递增：用游标增量换算字节→字符，避免每个匹配都
    // O(n) 重扫前缀（-all 密集匹配的 1 MiB 性能瓶颈）。
    let mut cursor_byte = char_to_byte(full, char_start);
    let mut cursor_char = char_start;
    let mut first = true;
    loop {
        if !first {
            if include_end {
                if p > char_len {
                    break;
                }
            } else if p >= char_len {
                break;
            }
        }
        first = false;
        match attempt(p) {
            Some(groups) => {
                let (s, e) = groups[0].unwrap_or((0, 0));
                out.push(groups);
                if s >= cursor_byte {
                    cursor_char += full[cursor_byte..s].chars().count();
                    cursor_byte = s;
                }
                let start_char = cursor_char;
                if e >= cursor_byte {
                    cursor_char += full[cursor_byte..e].chars().count();
                    cursor_byte = e;
                }
                let end_char = cursor_char;
                // An empty match restarts one char past its position;
                // a non-empty one right after its end.
                p = end_char.max(start_char + 1);
            }
            None => break,
        }
    }
    out
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Count capturing groups for `-about`: unescaped `(` outside classes
/// that does not start a `(?` construct.
fn count_groups(pat: &str) -> usize {
    let b = pat.as_bytes();
    let (mut n, mut in_class, mut i) = (0, false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class => {
                if b.get(i + 1) != Some(&b'?') {
                    n += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    n
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Non-greedy quantifier present (`+?` `*?` `??` `}?`)?
fn pattern_is_nongreedy(pat: &str) -> bool {
    let b = pat.as_bytes();
    for i in 1..b.len() {
        if b[i] == b'?' && matches!(b[i - 1], b'+' | b'*' | b'?' | b'}') {
            return true;
        }
    }
    false
}

/// Constructs POSIX ERE cannot express (Perl classes like `\d`,
/// lookaround, `(?` groups)?  Non-greedy quantifiers count too.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn pattern_is_nonposix(pat: &str, nongreedy: bool) -> bool {
    if nongreedy {
        return true;
    }
    // `***=` / `***:` 元语法本身就是非 POSIX 扩展。
    if pat.starts_with("***=") || pat.starts_with("***:") {
        return true;
    }
    let b = pat.as_bytes();
    let (mut in_class, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => {
                if i + 1 < b.len() && b[i + 1].is_ascii_alphanumeric() {
                    return true;
                }
                i += 2;
            }
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class && b.get(i + 1) == Some(&b'?') => return true,
            _ => i += 1,
        }
    }
    false
}

/// `(?=` / `(?!` outside classes and escapes（-about 的
/// REG_ULOOKAHEAD；语法上是 non-POSIX，与 REG_UNONPOSIX 并列输出）。
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn pattern_has_lookahead(pat: &str) -> bool {
    if pat.starts_with("***=") {
        return false;
    }
    let (bre, ere) = about_dialect(pat);
    if bre || ere {
        return false;
    }
    let b = pat.as_bytes();
    let (mut in_class, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class => {
                if b.get(i + 1) == Some(&b'?')
                    && matches!(b.get(i + 2), Some(b'=') | Some(b'!'))
                {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// Tcl 里 REG_ULOCALE 表示用到了 Unicode 依赖的类：Perl 类
/// `\d\s\w` 及其反集、词断言 `\y\m\M\Y`、POSIX `[[:class:]]`。
/// `-nocase` 本身不产生该标志（tclsh 8.6.17 实测）。
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn pattern_uses_ulocale(pat: &str) -> bool {
    if pat.starts_with("***=") {
        return false;
    }
    let (bre, ere) = about_dialect(pat);
    let escapes_locale = !bre && !ere;
    let b = pat.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() => {
                if escapes_locale
                    && matches!(
                        b[i + 1],
                        b'd' | b'D' | b's' | b'S' | b'w' | b'W' | b'y' | b'm' | b'M' | b'Y'
                    )
                {
                    return true;
                }
                i += 2;
            }
            b'[' => {
                // `[[:alpha:]]`：POSIX 类名（Unicode 表）。
                if b.get(i + 1) == Some(&b'[') && b.get(i + 2) == Some(&b':') {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// ERE 方言（`(?e)` / `***:(?e)`）下存在多余的 `)`——tclsh 把它当
/// 字面字符并在 -about 里报 REG_UPBOTCH。
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn pattern_has_upbotch(pat: &str) -> bool {
    let p = pat.strip_prefix("***:").unwrap_or(pat);
    let Some(rest) = p.strip_prefix("(?") else {
        return false;
    };
    let Some(end) = rest.find(')') else {
        return false;
    };
    let letters = &rest[..end];
    if !letters.contains('e') || letters.contains('b') {
        return false;
    }
    let b = rest[end + 1..].as_bytes();
    let (mut depth, mut in_class, mut i) = (0usize, false, 0usize);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class => { depth += 1; i += 1; }
            b')' if !in_class => {
                if depth == 0 {
                    return true;
                }
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// `-about` 需要的生效选项子集（不依赖 regexp_spencer 模块类型，
/// lite/wasm 构建也能编译）。
#[derive(Clone, Copy, Default)]
struct AboutEff {
    expanded: bool,
    quote: bool,
    bre: bool,
    ere: bool,
}

/// `-about` 的语法扫描结果（与 tclsh 的 re_info 位一一对应）。
#[derive(Default)]
struct AboutLex {
    backref: bool,
    bounds: bool,
    braces: bool,
    ubsalnum: bool,
    unspec: bool,
}

/// 前缀里声明的方言：`(bre, ere)`；默认 ARE。
fn about_dialect(pat: &str) -> (bool, bool) {
    let p = pat.strip_prefix("***:").unwrap_or(pat);
    if let Some(rest) = p.strip_prefix("(?") {
        if let Some(end) = rest.find(')') {
            let letters = &rest[..end];
            if letters.chars().all(|c| c.is_ascii_alphabetic()) {
                let bre = letters.contains('b');
                let ere = !bre && letters.contains('e');
                return (bre, ere);
            }
        }
    }
    (false, false)
}

/// `{m}` / `{m,}` / `{m,n}`（ARE/ERE），返回 `}` 之后的位置。
fn about_valid_bound(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let start = j;
    let mut m = 0u32;
    while j < b.len() && b[j].is_ascii_digit() {
        m = m.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
        j += 1;
    }
    if j == start {
        return None;
    }
    let mut n = m;
    let mut unbounded = false;
    if j < b.len() && b[j] == b',' {
        j += 1;
        let ds = j;
        let mut v = 0u32;
        while j < b.len() && b[j].is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
            j += 1;
        }
        if j == ds {
            unbounded = true;
        } else {
            n = v;
        }
    }
    if j >= b.len() || b[j] != b'}' {
        return None;
    }
    if m > 255 || (!unbounded && n > 255) || m > n {
        return None;
    }
    Some(j + 1)
}

/// BRE `\{m,n\}`，返回 `\}` 之后的位置（m 可省略）。
fn about_valid_bre_bound(b: &[u8], i: usize) -> Option<usize> {
    if i + 1 >= b.len() || b[i] != b'\\' || b[i + 1] != b'{' {
        return None;
    }
    let mut j = i + 2;
    let mut m = 0u32;
    let ds = j;
    while j < b.len() && b[j].is_ascii_digit() {
        m = m.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
        j += 1;
    }
    let had_m = j > ds;
    let mut n = m;
    let mut unbounded = false;
    if j < b.len() && b[j] == b',' {
        j += 1;
        let ds2 = j;
        let mut v = 0u32;
        while j < b.len() && b[j].is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
            j += 1;
        }
        if j == ds2 {
            unbounded = true;
        } else {
            n = v;
        }
    } else if !had_m {
        return None;
    }
    if j + 1 >= b.len() || b[j] != b'\\' || b[j + 1] != b'}' {
        return None;
    }
    if m > 255 || (!unbounded && n > 255) || m > n {
        return None;
    }
    Some(j + 2)
}

/// 去掉 expanded 模式忽略的空白与 `#` 注释（保留转义字符与字符类
/// 内容），返回 (扫描串, 是否真的忽略过内容)。
fn strip_expanded(pat: &str, escape_in_class: bool) -> (String, bool) {
    let b = pat.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let (mut i, mut in_class, mut ignored) = (0usize, false, false);
    while i < b.len() {
        let c = b[i];
        if in_class {
            if c == b'\\' && escape_in_class && i + 1 < b.len() {
                out.push(c);
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            out.push(c);
            if c == b']' {
                in_class = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'[' => {
                in_class = true;
                out.push(c);
                i += 1;
            }
            b'\\' if i + 1 < b.len() => {
                out.push(c);
                out.push(b[i + 1]);
                i += 2;
            }
            b'#' => {
                ignored = true;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                if i < b.len() {
                    i += 1;
                }
            }
            c if c.is_ascii_whitespace() => {
                ignored = true;
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    (
        String::from_utf8(out).unwrap_or_else(|_| pat.to_string()),
        ignored,
    )
}

/// 字符类里出现反斜杠（任意方言）→ REG_UBBS。`***=` 字面模式除外。
fn pattern_has_class_backslash(pat: &str) -> bool {
    if pat.starts_with("***=") {
        return false;
    }
    let b = pat.as_bytes();
    let mut in_class = false;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'[' if !in_class => {
                in_class = true;
                i += 1;
            }
            b']' if in_class => {
                in_class = false;
                i += 1;
            }
            b'\\' if in_class => return true,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    false
}

/// 扫描 pattern（跳过前缀/字符类/转义）收集 -about 语法位。
fn about_lex(pat: &str) -> AboutLex {
    let mut out = AboutLex::default();
    let b = pat.as_bytes();
    let mut i = 0usize;
    let mut quoted = false;
    if pat.starts_with("***=") {
        quoted = true;
        i = 4;
    } else if pat.starts_with("***:") {
        i = 4;
    }
    let mut bre = false;
    let mut ere = false;
    // 开头的内嵌选项组（(?b) / (?e) / (?i) ...）。
    if !quoted {
        let (bflag, eflag) = about_dialect(pat);
        bre = bflag;
        ere = eflag;
        if b.get(i) == Some(&b'(') && b.get(i + 1) == Some(&b'?') {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            if b.get(j) == Some(&b')') {
                i = j + 1;
            }
        }
    }
    if quoted {
        out.unspec = i >= b.len();
        return out;
    }
    let are = !bre && !ere;
    if i >= b.len() {
        out.unspec = true;
        return out;
    }
    let mut in_class = false;
    // 上一个 token 是否可作分支端点（用于空分支 UUNSPEC）。
    let mut prev_atom = false;
    while i < b.len() {
        let c = b[i];
        if in_class {
            if c == b']' {
                in_class = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'[' => {
                in_class = true;
                i += 1;
                prev_atom = true;
            }
            b'\\' if i + 1 < b.len() => {
                let e = b[i + 1];
                if bre {
                    if e.is_ascii_digit() && e != b'0' {
                        out.backref = true;
                    } else if e.is_ascii_alphanumeric() {
                        // BRE 没有 `\d` 等转义：`\`+字母数字是
                        // 未指定语法（tclsh: UBSALNUM + UUNSPEC）。
                        out.ubsalnum = true;
                        out.unspec = true;
                    }
                    if e == b'{' {
                        if let Some(end) = about_valid_bre_bound(b, i) {
                            out.bounds = true;
                            i = end;
                            prev_atom = true;
                            continue;
                        }
                    }
                    if e == b'(' && b.get(i + 2) == Some(&b'\\')
                        && b.get(i + 3) == Some(&b')')
                    {
                        out.unspec = true;
                    }
                    i += 2;
                    prev_atom = true;
                } else {
                    if are && e.is_ascii_digit() && e != b'0' {
                        out.backref = true;
                    } else if ere && e.is_ascii_alphanumeric() {
                        out.ubsalnum = true;
                        out.unspec = true;
                    }
                    i += 2;
                    prev_atom = true;
                }
            }
            b'{' if !bre => {
                if let Some(end) = about_valid_bound(b, i) {
                    out.bounds = true;
                    i = end;
                } else {
                    out.braces = true;
                    out.unspec = true;
                    i += 1;
                }
                prev_atom = true;
            }
            b'(' if !bre => {
                // 空组 `()` / `(?:)` 是 POSIX 未指定行为。
                if b.get(i + 1) == Some(&b')')
                    || (b.get(i + 1) == Some(&b'?')
                        && b.get(i + 2) == Some(&b':')
                        && b.get(i + 3) == Some(&b')'))
                {
                    out.unspec = true;
                }
                i += 1;
                prev_atom = false;
            }
            b')' if !bre => {
                i += 1;
                prev_atom = true;
            }
            b'|' if !bre => {
                let next = b.get(i + 1).copied();
                if !prev_atom
                    || next.is_none()
                    || next == Some(b'|')
                    || next == Some(b')')
                {
                    out.unspec = true;
                }
                i += 1;
                prev_atom = false;
            }
            _ => {
                i += 1;
                prev_atom = true;
            }
        }
    }
    out
}

/// -about 的 REG_UEMPTYMATCH / REG_UIMPOSSIBLE / REG_USHORTEST：
/// 用 Spencer parser 重新解析获得 subre 树（与命令实际引擎无关）。
/// 返回 (可空, 不可命中, 根偏好 SHORTER, 类内范围, 生效 Options)。
#[cfg(all(feature = "regexp-spencer", any(feature = "regexp", feature = "regexp-lite")))]
fn engine_about_bits(
    _eng: &Engine,
    pat: &str,
    nocase: bool,
    expanded: bool,
    lineanchor: bool,
    linestop: bool,
) -> (bool, bool, bool, bool, Option<AboutEff>) {
    match super::regexp_spencer::compile_with_options(
        pat,
        spencer_options(nocase, expanded, lineanchor, linestop),
    ) {
        Ok((p, effective)) => (
            super::regexp_spencer::ast::nullable_matchable(&p.tree),
            super::regexp_spencer::ast::impossible(&p.tree),
            p.flags.shorter,
            super::regexp_spencer::ast::has_bracket_range(&p.tree),
            Some(AboutEff {
                expanded: effective.expanded,
                quote: effective.quote,
                bre: effective.bre,
                ere: effective.ere,
            }),
        ),
        Err(_) => (false, false, false, false, None),
    }
}

#[cfg(all(not(feature = "regexp-spencer"), any(feature = "regexp", feature = "regexp-lite")))]
fn engine_about_bits(
    _eng: &Engine,
    _pat: &str,
    _nocase: bool,
    _expanded: bool,
    _lineanchor: bool,
    _linestop: bool,
) -> (bool, bool, bool, bool, Option<AboutEff>) {
    (false, false, false, false, None)
}

/// `regexp ?switches? exp string ?matchVar? ?subMatchVar ...?`
///
/// Returns 1 if the regular expression matches, 0 otherwise.
/// If match variables are provided, stores the matched text.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
pub fn cmd_regexp(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    const USAGE: &str =
        "wrong # args: should be \"regexp ?-option ...? exp string ?matchVar? ?subMatchVar ...?\"";
    let mut i = 1;
    let mut nocase = false;
    let mut all = false;
    let mut inline = false;
    let mut indices = false;
    let mut expanded = false;
    let mut lineanchor = false;
    let mut linestop = false;
    let mut about = false;
    let mut start_offset: Option<String> = None;

    // Parse switches
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-all" => { all = true; i += 1; }
            "-about" => { about = true; i += 1; }
            "-inline" => { inline = true; i += 1; }
            "-indices" => { indices = true; i += 1; }
            "-expanded" => { expanded = true; i += 1; }
            "-line" => { lineanchor = true; linestop = true; i += 1; }
            "-lineanchor" => { lineanchor = true; i += 1; }
            "-linestop" => { linestop = true; i += 1; }
            "-start" => {
                if i + 1 >= args.len() {
                    return Err(Error::runtime(
                        "missing argument to \"-start\"", ErrorCode::Generic,
                    ));
                }
                start_offset = Some(args[i + 1].as_str().to_string());
                i += 2;
            }
            "--" => { i += 1; break; }
            s => {
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": must be -all, -about, -indices, -inline, \
                         -expanded, -line, -linestop, -lineanchor, -nocase, -start, or --", s
                    ),
                    ErrorCode::Generic,
                ));
            }
        }
    }

    if i >= args.len() {
        return Err(Error::runtime(USAGE, ErrorCode::Generic));
    }
    let pattern_str = args[i].as_str();

    // -about: report {numGroups flags} without needing a string.
    if about {
        let eng = compile_engine_tc(interp, pattern_str, nocase, expanded, lineanchor, linestop)
            .map_err(|e| Error::runtime(e, ErrorCode::Generic))?;
        let (empty_ok, impossible, shortest, bracket_range, effective) =
            engine_about_bits(&eng, pattern_str, nocase, expanded, lineanchor, linestop);
        let eff_expanded = effective.map(|o| o.expanded).unwrap_or(expanded);
        let quote = effective.map(|o| o.quote).unwrap_or(false);
        let escape_in_class = effective.map(|o| !(o.bre || o.ere)).unwrap_or(true);
        let (scan_owned, expanded_ignored) = if eff_expanded && !quote {
            strip_expanded(pattern_str, escape_in_class)
        } else {
            (pattern_str.to_string(), false)
        };
        let scan = scan_owned.as_str();
        let nongreedy = !quote && pattern_is_nongreedy(scan);
        let lex = about_lex(scan);
        let ubbs = pattern_has_class_backslash(scan);
        let mut flags: Vec<&str> = Vec::new();
        // 输出顺序与 tclsh 的 re_info 位序一致：
        // UBACKREF → ULOOKAHEAD → UBOUNDS → UBRACES → UPBOTCH →
        // UNONPOSIX → UUNSPEC → ULOCALE → UEMPTYMATCH → USHORTEST。
        if lex.backref {
            flags.push("REG_UBACKREF");
        }
        if pattern_has_lookahead(scan) {
            flags.push("REG_ULOOKAHEAD");
        }
        if lex.bounds {
            flags.push("REG_UBOUNDS");
        }
        if lex.braces {
            flags.push("REG_UBRACES");
        }
        if lex.ubsalnum {
            flags.push("REG_UBSALNUM");
        }
        if pattern_has_upbotch(scan) {
            flags.push("REG_UPBOTCH");
        }
        if ubbs {
            flags.push("REG_UBBS");
        }
        if ubbs || expanded_ignored || pattern_is_nonposix(scan, nongreedy) {
            flags.push("REG_UNONPOSIX");
        }
        if lex.unspec {
            flags.push("REG_UUNSPEC");
        }
        if bracket_range {
            flags.push("REG_UUNPORT");
        }
        // -nocase 本身不置 ULOCALE；只有 Unicode 依赖的类才置。
        if pattern_uses_ulocale(scan) {
            flags.push("REG_ULOCALE");
        }
        // 不可命中的模式 tclsh 不再置 UEMPTYMATCH。
        if empty_ok && !impossible {
            flags.push("REG_UEMPTYMATCH");
        }
        if impossible {
            flags.push("REG_UIMPOSSIBLE");
        }
        // Spencer 根偏好 SHORTER 才置位（`(a+)(a*?)` 虽含懒惰量词，
        // COMBINE 结果仍是 LONGER，tclsh 不置 REG_USHORTEST）。
        let shortest_flag = if cfg!(feature = "regexp-spencer") {
            shortest
        } else {
            nongreedy
        };
        if shortest_flag {
            flags.push("REG_USHORTEST");
        }
        // Tcl 列表规范形：空 `{}`、单元素裸词、多元素 `{a b}`。
        let flag_str = match flags.len() {
            0 => "{}".to_string(),
            1 => flags[0].to_string(),
            _ => format!("{{{}}}", flags.join(" ")),
        };
        return Ok(Value::from_str(&format!(
            "{} {}",
            engine_group_count(&eng, scan),
            flag_str
        )));
    }

    if i + 1 >= args.len() {
        return Err(Error::runtime(USAGE, ErrorCode::Generic));
    }

    // -inline conflicts with match variables
    if inline && i + 2 < args.len() {
        return Err(Error::runtime(
            "regexp match variables not allowed when using -inline",
            ErrorCode::Generic,
        ));
    }

    let full_string = args[i + 1].as_str();
    let var_args = &args[i + 2..];

    let char_len = full_string.chars().count();
    let char_start = match &start_offset {
        Some(off_str) => parse_start_index(off_str, char_len)
            .map_err(|e| Error::runtime(e, ErrorCode::Generic))?,
        None => 0,
    };

    let eng = compile_engine_tc(interp, pattern_str, nocase, expanded, lineanchor, linestop)
        .map_err(|e| Error::runtime(e, ErrorCode::Generic))?;

    // `\A` 绑定每次尝试的搜索起点：非 -all 只尝试 -start（故
    // `-start 1 {\Aa} aba` → 0）；-all 每次向后推进时，新起点成为
    // 虚拟串首（tclsh 16.4：`regsub -all {\A(\w)}` 逐字符替换）。
    // -start 越过串尾时 tclsh 在“虚拟空串”上匹配并保留原偏移
    //（`-start 9 {$} ba` → 索引 9 8），故按串尾执行并给索引加 delta。
    let virtual_start = char_start > char_len;
    let effective_start = char_start.min(char_len);
    let index_delta = (char_start - effective_start) as i64;
    let mut char_cache = CharCache::new();
    let attempt = |p: usize| -> Option<GroupRanges> {
        // 越过串尾的 -start 把原偏移作为虚拟搜索起点（\A 命中串尾，
        // 词断言/^ 看不到串尾之前的字符）。
        let ss = if virtual_start { char_start } else { p };
        engine_attempt(&eng, full_string, p, ss, &mut char_cache)
    };
    let matches = collect_matches(full_string, attempt, char_len, effective_start, all, false);

    // Build a per-match list element: the group's text, or its
    // inclusive `start end` index pair under -indices.
    let group_elem = |groups: &GroupRanges, j: usize| -> Value {
        match groups.get(j).and_then(|g| *g) {
            Some((s, e)) => {
                if indices {
                    Value::from_str(&format!(
                        "{} {}",
                        byte_to_char(full_string, s) as i64 + index_delta,
                        byte_to_char(full_string, e) as i64 - 1 + index_delta
                    ))
                } else {
                    Value::from_str(&full_string[s..e])
                }
            }
            None => {
                if indices {
                    Value::from_str("-1 -1")
                } else {
                    Value::from_str("")
                }
            }
        }
    };

    if inline {
        // -inline: list of matched substrings (all matches under -all)
        let mut results = Vec::new();
        for groups in &matches {
            for j in 0..groups.len() {
                results.push(group_elem(groups, j));
            }
        }
        return Ok(Value::from_list(&results));
    }

    if all {
        // -all: return count of matches; set vars to last match
        if let Some(last) = matches.last() {
            set_match_var_groups(interp, last, var_args, indices, full_string, index_delta)?;
        }
        return Ok(Value::from_int(matches.len() as i64));
    }

    // Normal mode: match vars are only assigned on a successful match.
    if let Some(groups) = matches.first() {
        set_match_var_groups(interp, groups, var_args, indices, full_string, index_delta)?;
        Ok(Value::from_int(1))
    } else {
        Ok(Value::from_int(0))
    }
}

#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
/// Set match variables from collected group ranges.
fn set_match_var_groups(
    interp: &mut Interp,
    groups: &GroupRanges,
    var_args: &[Value],
    indices: bool,
    full: &str,
    index_delta: i64,
) -> Result<()> {
    for (vi, var) in var_args.iter().enumerate() {
        match groups.get(vi).and_then(|g| *g) {
            Some((s, e)) => {
                if indices {
                    interp.set_var(
                        var.as_str(),
                        Value::from_str(&format!(
                            "{} {}",
                            byte_to_char(full, s) as i64 + index_delta,
                            byte_to_char(full, e) as i64 - 1 + index_delta
                        )),
                    )?;
                } else {
                    interp.set_var(var.as_str(), Value::from_str(&full[s..e]))?;
                }
            }
            None => {
                if indices {
                    interp.set_var(var.as_str(), Value::from_str("-1 -1"))?;
                } else {
                    interp.set_var(var.as_str(), Value::empty())?;
                }
            }
        }
    }
    Ok(())
}

/// `regsub ?switches? exp string subSpec ?varName?`
///
/// Substitutes regex matches. Returns the substituted string or count.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
pub fn cmd_regsub(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    const USAGE: &str =
        "wrong # args: should be \"regsub ?-option ...? exp string subSpec ?varName?\"";
    let mut i = 1;
    let mut nocase = false;
    let mut all = false;
    let mut expanded = false;
    let mut lineanchor = false;
    let mut linestop = false;
    let mut start_offset: Option<String> = None;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocase" => { nocase = true; i += 1; }
            "-all" => { all = true; i += 1; }
            "-expanded" => { expanded = true; i += 1; }
            "-line" => { lineanchor = true; linestop = true; i += 1; }
            "-lineanchor" => { lineanchor = true; i += 1; }
            "-linestop" => { linestop = true; i += 1; }
            "-start" => {
                if i + 1 >= args.len() {
                    return Err(Error::runtime(
                        "missing argument to \"-start\"", ErrorCode::Generic,
                    ));
                }
                start_offset = Some(args[i + 1].as_str().to_string());
                i += 2;
            }
            "--" => { i += 1; break; }
            s => {
                // D-21 对齐：-command 非 8.6 regsub 选项（tclsh 拒绝；
                // 原 rtcl 扩展按 parity 原则移除——错误形状与 tclsh 一致）。
                super::list::set_error_code(
                    interp,
                    &format!("TCL LOOKUP INDEX option {}", s),
                );
                return Err(Error::Msg(format!(
                    "bad option \"{}\": must be -all, -nocase, -expanded, -line, \
                     -linestop, -lineanchor, -start, or --", s
                )));
            }
        }
    }

    if i + 2 >= args.len() || i + 4 < args.len() {
        return Err(Error::runtime(USAGE, ErrorCode::Generic));
    }

    let pattern_str = args[i].as_str();
    let full_string = args[i + 1].as_str();
    let sub_spec = args[i + 2].as_str();
    let var_name = args.get(i + 3).map(|a| a.as_str());

    let char_len = full_string.chars().count();
    let char_start = match &start_offset {
        Some(off_str) => parse_start_index(off_str, char_len)
            .map_err(|e| Error::runtime(e, ErrorCode::Generic))?,
        None => 0,
    };
    let byte_start = char_to_byte(full_string, char_start);

    let eng = compile_engine_tc(interp, pattern_str, nocase, expanded, lineanchor, linestop)
        .map_err(|e| Error::runtime(e, ErrorCode::Generic))?;

    // Standard mode: Tcl substitution spec applied to the matches the
    // -start/-all scan yields.  regsub assigns varName unconditionally,
    // even when nothing is replaced.
    // `\A` 绑定每次尝试的搜索起点：非 -all 只尝试 -start（故
    // `-start 1 {\Aa} aba` → 0）；-all 每次向后推进时，新起点成为
    // 虚拟串首（tclsh 16.4：`regsub -all {\A(\w)}` 逐字符替换）。
    let mut char_cache = CharCache::new();
    let attempt = |p: usize| -> Option<GroupRanges> {
        engine_attempt(&eng, full_string, p, p, &mut char_cache)
    };
    let matches = collect_matches(full_string, attempt, char_len, char_start, all, true);

    let mut result = String::from(&full_string[..byte_start]);
    let mut last = byte_start;
    for groups in &matches {
        if let Some((bs, be)) = groups[0] {
            result.push_str(&full_string[last..bs]);
            expand_sub_spec(&mut result, sub_spec, groups, full_string);
            last = be;
        }
    }
    result.push_str(&full_string[last..]);

    if let Some(var) = var_name {
        interp.set_var(var, Value::from_str(&result))?;
        Ok(Value::from_int(matches.len() as i64))
    } else {
        Ok(Value::from_str(&result))
    }
}

/// Expand a Tcl regsub substitution spec against one match's groups:
/// `&`/`\0` → whole match, `\1`–`\9` → submatch, `\&`/`\\` → literal,
/// `\X` (other) → the backslash is retained before X.
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
fn expand_sub_spec(out: &mut String, spec: &str, groups: &GroupRanges, full: &str) {
    let group_text = |n: usize| -> &str {
        groups
            .get(n)
            .and_then(|g| *g)
            .map(|(s, e)| &full[s..e])
            .unwrap_or("")
    };
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '&' => {
                out.push_str(group_text(0));
                i += 1;
            }
            '\\' => match chars.get(i + 1) {
                Some(d) if d.is_ascii_digit() => {
                    out.push_str(group_text(*d as usize - '0' as usize));
                    i += 2;
                }
                Some('\\') => {
                    out.push('\\');
                    i += 2;
                }
                Some('&') => {
                    out.push('&');
                    i += 2;
                }
                Some(o) => {
                    out.push('\\');
                    out.push(*o);
                    i += 2;
                }
                None => {
                    out.push('\\');
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
}

#[cfg(not(feature = "std"))]
pub fn cmd_regexp(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "regexp requires std feature",
        ErrorCode::Generic,
    ))
}

#[cfg(not(feature = "std"))]
pub fn cmd_regsub(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "regsub requires std feature",
        ErrorCode::Generic,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(all(test, feature = "regexp"))]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn parse_start_index_tcl_grammar() {
        use super::parse_start_index;
        assert_eq!(parse_start_index("0x10", 3), Ok(16));
        assert_eq!(parse_start_index("010", 3), Ok(8));
        assert_eq!(parse_start_index("0b10", 3), Ok(2));
        assert_eq!(parse_start_index("end+1", 3), Ok(4));
        assert_eq!(parse_start_index("end-100", 3), Ok(0));
        assert_eq!(parse_start_index("1+-2", 3), Ok(0));
        assert_eq!(parse_start_index("1--2", 3), Ok(3));
        assert_eq!(parse_start_index(" 1 ", 3), Ok(1));
        assert_eq!(parse_start_index("-2147483649", 3), Ok(2147483647));
        assert!(parse_start_index("4294967296", 3).is_err());
        assert!(parse_start_index("", 3).is_err());
        assert!(parse_start_index("1 + 2", 3).is_err());
        assert!(parse_start_index("0d10", 3).is_err());
    }

    // ── basic regexp tests ─────────────────────────────────────────────────────

    #[test]
    fn test_regexp_basic_match() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp {hello} {hello world}").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    #[test]
    fn test_regexp_basic_no_match() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp {bye} {hello world}").unwrap();
        assert_eq!(result.as_str(), "0");
    }

    #[test]
    fn test_regexp_capture() {
        let mut interp = Interp::new();
        interp.eval("regexp {(\\d+)} {abc123def} match num").unwrap();
        let m = interp.get_var("match").unwrap();
        let n = interp.get_var("num").unwrap();
        assert_eq!(m.as_str(), "123");
        assert_eq!(n.as_str(), "123");
    }

    // ── regexp -nocase tests ───────────────────────────────────────────────────

    #[test]
    fn test_regexp_nocase() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -nocase {HELLO} {hello world}").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    // ── regexp -all tests ──────────────────────────────────────────────────────

    #[test]
    fn test_regexp_all_count() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -all {\\d} {a1b2c3}").unwrap();
        assert_eq!(result.as_str(), "3");
    }

    #[test]
    fn test_regexp_all_with_vars() {
        let mut interp = Interp::new();
        interp.eval("regexp -all {(\\d)} {a1b2c3} match digit").unwrap();
        // Vars should be set to last match
        let d = interp.eval("set digit").unwrap();
        assert_eq!(d.as_str(), "3");
    }

    // ── regexp -inline tests ───────────────────────────────────────────────────

    #[test]
    fn test_regexp_inline() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -inline {(\\d+)} {abc123def}").unwrap();
        assert_eq!(result.as_str(), "123 123");
    }

    #[test]
    fn test_regexp_inline_all() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -all -inline {\\d} {a1b2c3}").unwrap();
        // Should return list of all matches
        assert!(result.as_str().contains('1'));
        assert!(result.as_str().contains('2'));
        assert!(result.as_str().contains('3'));
    }

    // ── regexp -indices tests ──────────────────────────────────────────────────

    #[test]
    fn test_regexp_indices() {
        let mut interp = Interp::new();
        interp.eval("regexp -indices {(\\d+)} {abc123def} match num").unwrap();
        let m = interp.eval("set match").unwrap();
        // Match "123" is at indices 3-5 (0-indexed, inclusive)
        assert_eq!(m.as_str(), "3 5");
    }

    #[test]
    fn test_regexp_indices_no_match() {
        let mut interp = Interp::new();
        // tclsh leaves match variables untouched on a failed match.
        interp.eval("regexp -indices {(x)} {abc} m n").unwrap();
        assert_eq!(
            interp.eval("info exists n").unwrap().as_str(),
            "0"
        );
    }

    #[test]
    fn test_regexp_indices_inline() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -indices -inline {(\\d+)} {abc123def}").unwrap();
        // Should return index pairs
        assert_eq!(result.as_str(), "{3 5} {3 5}");
    }

    // ── regexp -start tests ────────────────────────────────────────────────────

    #[test]
    fn test_regexp_start() {
        let mut interp = Interp::new();
        // Search from position 2 onwards
        let result = interp.eval("regexp -start 2 {a} {xaax}").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    #[test]
    fn test_regexp_start_skip_first() {
        let mut interp = Interp::new();
        // Find first 'a' starting from position 2
        interp.eval("regexp -start 2 -indices {a} {a1a2} m").unwrap();
        let idx = interp.eval("set m").unwrap();
        // First 'a' is at 0, second is at 2, so with start=2 we get index 2
        assert_eq!(idx.as_str(), "2 2");
    }

    // ── regexp -expanded tests ─────────────────────────────────────────────────

    #[test]
    fn test_regexp_expanded() {
        let mut interp = Interp::new();
        // In expanded mode, whitespace and comments are ignored
        let result = interp.eval("regexp -expanded { a  b } {ab}").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    // ── regexp -line tests ─────────────────────────────────────────────────────

    #[test]
    fn test_regexp_line_multiline() {
        let mut interp = Interp::new();
        // -line makes ^ and $ match line boundaries
        let result = interp.eval("regexp -line {^world} {hello\nworld}").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    // ── basic regsub tests ─────────────────────────────────────────────────────

    #[test]
    fn test_regsub_basic() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub {world} {hello world} {Tcl}").unwrap();
        assert_eq!(result.as_str(), "hello Tcl");
    }

    #[test]
    fn test_regsub_with_var() {
        let mut interp = Interp::new();
        let count = interp.eval("regsub {o} {hello world} {0} result").unwrap();
        let r = interp.eval("set result").unwrap();
        assert_eq!(count.as_str(), "1"); // Only first occurrence
        assert_eq!(r.as_str(), "hell0 world");
    }

    #[test]
    fn test_regsub_all() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub -all {o} {hello world} {0}").unwrap();
        assert_eq!(result.as_str(), "hell0 w0rld");
    }

    #[test]
    fn test_regsub_nocase() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub -nocase {HELLO} {hello world} {bye}").unwrap();
        assert_eq!(result.as_str(), "bye world");
    }

    // ── regsub -start tests ────────────────────────────────────────────────────

    #[test]
    fn test_regsub_start() {
        let mut interp = Interp::new();
        // Replace 'a' only starting from position 2
        let result = interp.eval("regsub -start 2 {a} {aaa} {b}").unwrap();
        // Prefix "aa" + replace from index 2 onwards
        assert_eq!(result.as_str(), "aab");
    }

    // ── regsub -expanded tests ─────────────────────────────────────────────────

    #[test]
    fn test_regsub_expanded() {
        let mut interp = Interp::new();
        // -expanded ignores whitespace in pattern, so "  a  " becomes "a"
        let result = interp.eval("regsub -expanded { a } {bab} {c}").unwrap();
        assert_eq!(result.as_str(), "bcb");
    }

    #[test]
    fn test_regsub_expanded_pattern() {
        let mut interp = Interp::new();
        // -expanded ignores whitespace in pattern
        let result = interp.eval("regsub -expanded { x } {axb} {X}").unwrap();
        assert_eq!(result.as_str(), "aXb");
    }

    // ── regsub -command（D-21 对齐：非 8.6 选项，tclsh 拒绝）──────

    #[test]
    fn test_regsub_command_rejected() {
        let mut interp = Interp::new();
        let err = interp
            .eval("regsub -command {\\d+} {a12b34} double")
            .unwrap_err();
        assert!(
            err.to_string().starts_with("bad option \"-command\""),
            "{}",
            err
        );
    }

    // ── regsub backreference tests ─────────────────────────────────────────────

    #[test]
    fn test_regsub_backreference() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub {(\\w+)} {hello} {<\\1>}").unwrap();
        assert_eq!(result.as_str(), "<hello>");
    }

    #[test]
    fn test_regsub_ampersand() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub {\\w+} {hello} {[&]}").unwrap();
        assert_eq!(result.as_str(), "[hello]");
    }

    // ── error handling tests ───────────────────────────────────────────────────

    #[test]
    fn test_regexp_bad_switch() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -badswitch {a} {a}");
        assert!(result.is_err());
    }

    #[test]
    fn test_regsub_bad_switch() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub -badswitch {a} {a} {b}");
        assert!(result.is_err());
    }

    #[test]
    fn test_regexp_inline_with_vars_error() {
        let mut interp = Interp::new();
        // -inline with match variables should be an error
        let result = interp.eval("regexp -inline {a} {abc} m");
        assert!(result.is_err());
    }

    #[test]
    fn test_regexp_missing_start_arg() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -start {a} {abc}");
        assert!(result.is_err());
    }

    // ── regexp -- end-of-switches tests ────────────────────────────────────────

    #[test]
    fn test_regexp_end_of_switches() {
        let mut interp = Interp::new();
        // Pattern starts with '-', must use -- to signal end of switches
        let result = interp.eval("regexp -- {-test} {this-test works}").unwrap();
        assert_eq!(result.as_str(), "1");
    }

    // ── regexp -all -inline -indices combined ──────────────────────────────────

    #[test]
    fn test_regexp_all_inline_indices() {
        let mut interp = Interp::new();
        let result = interp.eval("regexp -all -inline -indices {\\d+} {a12b345c}").unwrap();
        // Two matches: "12" at 1-2, "345" at 4-6
        assert!(result.as_str().contains("1 2"));
        assert!(result.as_str().contains("4 6"));
    }

    // ── regexp with no match sets vars empty ───────────────────────────────────

    #[test]
    fn test_regexp_no_match_sets_empty_vars() {
        let mut interp = Interp::new();
        // tclsh leaves match variables untouched on a failed match.
        let result = interp.eval("regexp {zzzz} {abc} m").unwrap();
        assert_eq!(result.as_str(), "0");
        assert_eq!(
            interp.eval("info exists m").unwrap().as_str(),
            "0"
        );
    }

    // ── regsub -all with var sets count ────────────────────────────────────────

    #[test]
    fn test_regsub_all_with_var_returns_count() {
        let mut interp = Interp::new();
        let count = interp.eval("regsub -all {o} {foo oof} {0} result").unwrap();
        assert_eq!(count.as_str(), "4"); // foo has 2 o's, oof has 2
        let r = interp.eval("set result").unwrap();
        assert_eq!(r.as_str(), "f00 00f");
    }

    // ── regsub -start -all combined ────────────────────────────────────────────

    #[test]
    fn test_regsub_start_all() {
        let mut interp = Interp::new();
        // Replace all 'a' starting from position 2
        let result = interp.eval("regsub -all -start 2 {a} {aaaa} {x}").unwrap();
        // prefix "aa" + replace from index 2: "aa" -> "xx"
        assert_eq!(result.as_str(), "aaxx");
    }

    // ── regsub -command -all with capture groups ──────────────────────────────

    #[test]
    fn test_regsub_command_all_captures_rejected() {
        let mut interp = Interp::new();
        let err = interp
            .eval("regsub -all -command {[a-z]} {abc} up")
            .unwrap_err();
        assert!(
            err.to_string().starts_with("bad option \"-command\""),
            "{}",
            err
        );
    }

    // ── regsub no match returns original ──────────────────────────────────────

    #[test]
    fn test_regsub_no_match() {
        let mut interp = Interp::new();
        let result = interp.eval("regsub {zzz} {hello} {xxx}").unwrap();
        assert_eq!(result.as_str(), "hello");
    }

    // ── regsub -line mode ─────────────────────────────────────────────────────

    #[test]
    fn test_regsub_line() {
        let mut interp = Interp::new();
        // -line mode: ^ should match beginning of each line
        let result = interp.eval("regsub -all -line {^x} {x1\nx2\nx3} {y}").unwrap();
        assert_eq!(result.as_str(), "y1\ny2\ny3");
    }
}
