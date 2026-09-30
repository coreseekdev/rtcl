//! Utility functions shared across the interpreter.

/// Split `name(index)` into `(name, index)`.
pub(crate) fn split_array_ref(name: &str) -> Option<(&str, &str)> {
    let paren = name.find('(')?;
    let end_paren = name.rfind(')')?;
    if end_paren > paren {
        Some((&name[..paren], &name[paren + 1..end_paren]))
    } else {
        None
    }
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
