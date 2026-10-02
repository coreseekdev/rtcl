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

/// Attempt one match at char index `p` of `full_string`; returns capture
/// ranges in full-string byte offsets, group 0 first.
#[cfg(feature = "regexp")]
pub(crate) fn engine_attempt(eng: &Engine, full_string: &str, p: usize) -> Option<GroupRanges> {
    match eng {
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
                let off = char_to_byte(full_string, p);
                regex_caps_at(ra, &full_string[off..], 0).map(|groups| {
                    groups
                        .into_iter()
                        .map(|g| g.map(|(s, e)| (s + off, e + off)))
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
    }
}

#[cfg(feature = "regexp")]
pub(crate) fn engine_group_count(eng: &Engine, pattern: &str) -> usize {
    match eng {
        Engine::Fast { .. } => count_groups(pattern),
        Engine::Full(p) => p.ngroups,
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
pub(crate) fn engine_attempt(eng: &Engine, full_string: &str, p: usize) -> Option<GroupRanges> {
    match &eng.anch {
        Some(ra) => {
            let off = char_to_byte(full_string, p);
            regex_caps_at(ra, &full_string[off..], 0).map(|groups| {
                groups
                    .into_iter()
                    .map(|g| g.map(|(s, e)| (s + off, e + off)))
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
fn parse_start_index(s: &str, len: usize) -> std::result::Result<usize, String> {
    let bad = || {
        format!(
            "bad index \"{}\": must be integer?[+-]integer? or end?[+-]integer?",
            s
        )
    };
    let (head, tail) = if let Some(rest) = s.strip_prefix("end") {
        (len as i64, rest)
    } else if let Ok(n) = s.parse::<i64>() {
        // Plain integer, including negatives (tclsh clamps to 0).
        (n, "")
    } else {
        // `int+int` / `int-int` — the sign separator is never the
        // leading minus of the number itself.
        match s[1..].find(['+', '-']).map(|i| i + 1) {
            Some(i) => {
                let h = s[..i].parse::<i64>().map_err(|_| bad())?;
                (h, &s[i..])
            }
            None => return Err(bad()),
        }
    };
    let idx = if tail.is_empty() {
        head
    } else {
        let n: i64 = tail[1..].parse().map_err(|_| bad())?;
        if tail.starts_with('+') {
            head + n
        } else {
            head - n
        }
    };
    Ok(idx.max(0) as usize)
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
) -> Vec<GroupRanges> {
    let mut out: Vec<GroupRanges> = Vec::new();
    if char_start > char_len {
        return out;
    }
    if !all {
        if let Some(groups) = attempt(char_start) {
            out.push(groups);
        }
        return out;
    }
    if char_len == 0 {
        if let Some(groups) = attempt(0) {
            out.push(groups);
        }
        return out;
    }
    let mut p = char_start;
    while p < char_len {
        match attempt(p) {
            Some(groups) => {
                let (s, e) = groups[0].unwrap_or((0, 0));
                out.push(groups);
                // An empty match restarts one char past its position;
                // a non-empty one right after its end.
                p = byte_to_char(full, e).max(byte_to_char(full, s) + 1);
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
        let eng = compile_engine(pattern_str, nocase, expanded, lineanchor, linestop)
            .map_err(|e| {
                Error::runtime(
                    format!("couldn't compile regular expression pattern: {}", e),
                    ErrorCode::Generic,
                )
            })?;
        let nongreedy = pattern_is_nongreedy(pattern_str);
        let mut flags: Vec<&str> = Vec::new();
        if pattern_is_nonposix(pattern_str, nongreedy) {
            flags.push("REG_UNONPOSIX");
        }
        if nongreedy {
            flags.push("REG_USHORTEST");
        }
        if nocase {
            flags.push("REG_ULOCALE");
        }
        return Ok(Value::from_str(&format!(
            "{} {{{}}}",
            engine_group_count(&eng, pattern_str),
            flags.join(" ")
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

    let eng = compile_engine(pattern_str, nocase, expanded, lineanchor, linestop).map_err(|e| {
        Error::runtime(
            format!("couldn't compile regular expression pattern: {}", e),
            ErrorCode::Generic,
        )
    })?;

    let attempt = |p: usize| -> Option<GroupRanges> { engine_attempt(&eng, full_string, p) };
    let matches = collect_matches(full_string, attempt, char_len, char_start, all);

    // Build a per-match list element: the group's text, or its
    // inclusive `start end` index pair under -indices.
    let group_elem = |groups: &GroupRanges, j: usize| -> Value {
        match groups.get(j).and_then(|g| *g) {
            Some((s, e)) => {
                if indices {
                    Value::from_str(&format!(
                        "{} {}",
                        byte_to_char(full_string, s),
                        byte_to_char(full_string, e) as i64 - 1
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
            set_match_var_groups(interp, last, var_args, indices, full_string)?;
        }
        return Ok(Value::from_int(matches.len() as i64));
    }

    // Normal mode: match vars are only assigned on a successful match.
    if let Some(groups) = matches.first() {
        set_match_var_groups(interp, groups, var_args, indices, full_string)?;
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
) -> Result<()> {
    for (vi, var) in var_args.iter().enumerate() {
        match groups.get(vi).and_then(|g| *g) {
            Some((s, e)) => {
                if indices {
                    interp.set_var(
                        var.as_str(),
                        Value::from_str(&format!(
                            "{} {}",
                            byte_to_char(full, s),
                            byte_to_char(full, e) as i64 - 1
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
    let mut command_mode = false;

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
            "-command" => { command_mode = true; i += 1; }
            "--" => { i += 1; break; }
            s => {
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": must be -all, -nocase, -expanded, -line, \
                         -linestop, -lineanchor, -start, or --", s
                    ),
                    ErrorCode::Generic,
                ));
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

    let eng = compile_engine(pattern_str, nocase, expanded, lineanchor, linestop).map_err(|e| {
        Error::runtime(
            format!("couldn't compile regular expression pattern: {}", e),
            ErrorCode::Generic,
        )
    })?;

    if command_mode {
        // -command: evaluate sub_spec as command prefix for each match
        // (rtcl extension; not exercised under -start by the corpus).
        let attempt = |p: usize| -> Option<GroupRanges> { engine_attempt(&eng, full_string, p) };
        let matches = collect_matches(full_string, attempt, char_len, char_start, all);
        let mut result = String::from(&full_string[..byte_start]);
        let mut last = byte_start;
        let mut count = 0i64;

        for groups in &matches {
            let Some((bs, be)) = groups[0] else { continue };
            result.push_str(&full_string[last..bs]);

            // Build command: subSpec fullMatch capture1 capture2 ...
            let mut cmd_str = sub_spec.to_string();
            for j in 0..groups.len() {
                let m = groups[j].map(|(s, e)| &full_string[s..e]).unwrap_or("");
                cmd_str.push(' ');
                // Quote the argument for Tcl eval
                cmd_str.push('{');
                cmd_str.push_str(m);
                cmd_str.push('}');
            }
            let replacement = interp.eval(&cmd_str)?;
            result.push_str(replacement.as_str());
            last = be;
            count += 1;
        }
        result.push_str(&full_string[last..]);

        if let Some(var) = var_name {
            interp.set_var(var, Value::from_str(&result))?;
            return Ok(Value::from_int(count));
        }
        return Ok(Value::from_str(&result));
    }

    // Standard mode: Tcl substitution spec applied to the matches the
    // -start/-all scan yields.  regsub assigns varName unconditionally,
    // even when nothing is replaced.
    let attempt = |p: usize| -> Option<GroupRanges> { engine_attempt(&eng, full_string, p) };
    let matches = collect_matches(full_string, attempt, char_len, char_start, all);

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

    // ── regsub -command tests ──────────────────────────────────────────────────

    #[test]
    fn test_regsub_command() {
        let mut interp = Interp::new();
        // Use a proc to transform matches — without -all, only first match replaced
        interp.eval("proc double s { return [string cat $s $s] }").unwrap();
        let result = interp.eval("regsub -command {\\d+} {a12b34} double").unwrap();
        assert_eq!(result.as_str(), "a1212b34");
    }

    #[test]
    fn test_regsub_command_all() {
        let mut interp = Interp::new();
        interp.eval("proc up s { string toupper $s }").unwrap();
        let result = interp.eval("regsub -all -command {[a-z]} {abc} up").unwrap();
        assert_eq!(result.as_str(), "ABC");
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
    fn test_regsub_command_all_captures() {
        let mut interp = Interp::new();
        interp.eval("proc double s { return [string cat $s $s] }").unwrap();
        let result = interp.eval("regsub -all -command {\\d+} {a12b34} double").unwrap();
        assert_eq!(result.as_str(), "a1212b3434");
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
