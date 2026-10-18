//! Utility functions shared across the interpreter.

/// Split `name(index)` into `(name, index)`.
pub(crate) fn split_array_ref(name: &str) -> Option<(&str, &str)> {
    let paren = find_paren(name)?;
    let end_paren = name.rfind(')')?;
    if end_paren > paren {
        Some((&name[..paren], &name[paren + 1..end_paren]))
    } else {
        None
    }
}

/// Byte offset of the first `(` in `name`.  Every variable read/write and
/// every link install asks this of a short name ("i", "n", "acc" — the
/// element form is rare); `str::find`'s memchr setup dominated the leaf
/// samples for them, so short names take a byte loop.
#[inline]
pub(crate) fn find_paren(name: &str) -> Option<usize> {
    let b = name.as_bytes();
    if b.len() <= 16 {
        for (i, &c) in b.iter().enumerate() {
            if c == b'(' {
                return Some(i);
            }
        }
        None
    } else {
        name.find('(')
    }
}

/// `name` without its array-element suffix (`a(x)` → `a`).
#[inline]
pub(crate) fn strip_paren(name: &str) -> &str {
    match find_paren(name) {
        Some(i) => &name[..i],
        None => name,
    }
}

/// Does `name` contain a `::` namespace separator?  The variable and
/// dispatch fast paths ask this about every short name ("n", "acc",
/// "fib"); a two-byte window scan avoids `str::contains`' substring
/// searcher setup (StrSearcher::new was several percent of the fib
/// profile — the needle is two bytes and the haystacks are 1-4).
#[inline]
pub(crate) fn has_ns_sep(name: &str) -> bool {
    name.as_bytes().windows(2).any(|w| w == b"::")
}

/// Byte offset of the LAST `::` in `name` — `str::rfind("::")` without
/// the substring-searcher setup (`ns_of_qualified` probes every proc
/// call, and bare names like "fib" paid StrSearcher::new + a reverse
/// search each time).
#[inline]
pub(crate) fn rfind_ns_sep(name: &str) -> Option<usize> {
    let b = name.as_bytes();
    (0..b.len().saturating_sub(1)).rev().find(|&i| &b[i..i + 2] == b"::")
}

/// Glob pattern matching, a faithful port of tclsh's `TclUniCharMatch`
/// (tclUtf.c): `*`, `?`, `\\` escapes outside brackets, and `[...]` classes
/// where `]` at any member position fails, `-` always starts a range, ranges
/// match symmetrically, and backslash is not special inside classes.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    fn match_helper(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
        loop {
            if pi == p.len() {
                return ti == t.len();
            }
            let c = p[pi];
            if ti == t.len() && c != '*' {
                return false;
            }
            match c {
                '*' => {
                    pi += 1;
                    while pi < p.len() && p[pi] == '*' {
                        pi += 1;
                    }
                    if pi == p.len() {
                        return true;
                    }
                    let mut k = ti;
                    loop {
                        if match_helper(p, pi, t, k) {
                            return true;
                        }
                        if k == t.len() {
                            return false;
                        }
                        k += 1;
                    }
                }
                '?' => {
                    pi += 1;
                    ti += 1;
                }
                '[' => {
                    let ch1 = t[ti];
                    ti += 1;
                    pi += 1;
                    loop {
                        if pi >= p.len() || p[pi] == ']' {
                            return false;
                        }
                        let start = p[pi];
                        pi += 1;
                        if pi < p.len() && p[pi] == '-' {
                            pi += 1;
                            if pi >= p.len() {
                                return false;
                            }
                            let end = p[pi];
                            pi += 1;
                            if (start <= ch1 && ch1 <= end)
                                || (end <= ch1 && ch1 <= start)
                            {
                                break;
                            }
                        } else if start == ch1 {
                            break;
                        }
                    }
                    // Matched: skip to the closing `]` (which may be absent —
                    // Tcl then treats the class as ending the pattern).
                    while pi < p.len() && p[pi] != ']' {
                        pi += 1;
                    }
                    if pi < p.len() {
                        pi += 1;
                    }
                }
                '\\' => {
                    pi += 1;
                    if pi == p.len() {
                        return false;
                    }
                    if p[pi] != t[ti] {
                        return false;
                    }
                    pi += 1;
                    ti += 1;
                }
                _ => {
                    if p[pi] != t[ti] {
                        return false;
                    }
                    pi += 1;
                    ti += 1;
                }
            }
        }
    }

    match_helper(&pattern, 0, &text, 0)
}


/// Borrowed word source spans of the command currently dispatching: a raw
/// slice into the owning unit's/AST's `word_srcs` vector.
///
/// The interpreter used to hold an `Rc<Vec<SrcSpan>>` and clone it on
/// every `BeginCmd` — refcount traffic on the hottest path in the VM even
/// though the spans are immutable for their owner's lifetime.  Every
/// reader (`Interp::cur_cmd_word_srcs` consumers: the word-span readers
/// and the `foreach`/`lmap` inline-shape gate) runs while the owning
/// unit/AST is borrowed on the stack: `exec_bytecode` holds the unit for
/// its whole run, a tree-walk `eval_command` holds its `ScriptUnit`, and
/// the save/restore around nested evaluation copies the pointer pair.
///
/// SAFETY contract: the pointer is dereferenced *only* by those readers,
/// each of which runs inside an active command of the owning unit; the
/// pair may outlive the owner as a stale value but must never be read
/// then (every read site sits in an active command's evaluation).
#[derive(Clone, Copy)]
pub(crate) struct WordSrcs {
    ptr: *const rtcl_parser::SrcSpan,
    len: usize,
}

impl WordSrcs {
    /// The empty slice (no owner) — the value every unit entry swaps in.
    #[inline]
    pub(crate) fn empty() -> Self {
        Self {
            ptr: core::ptr::NonNull::<rtcl_parser::SrcSpan>::dangling().as_ptr(),
            len: 0,
        }
    }

    /// Borrow `rc`'s spans; the caller guarantees `rc` outlives every
    /// read of the result (see the type's contract).
    #[inline]
    pub(crate) fn from_rc(rc: &std::rc::Rc<Vec<rtcl_parser::SrcSpan>>) -> Self {
        Self {
            ptr: rc.as_ptr(),
            len: rc.len(),
        }
    }
}

impl core::ops::Deref for WordSrcs {
    type Target = [rtcl_parser::SrcSpan];
    #[inline]
    fn deref(&self) -> &[rtcl_parser::SrcSpan] {
        if self.len == 0 {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }
}
