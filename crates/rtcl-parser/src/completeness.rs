/// Check whether `source` is a complete Tcl script (balanced braces, quotes,
/// and brackets).  Returns `true` if the script can be parsed without needing
/// more input.  Used by `info complete` and multi-line REPL input.
pub fn is_complete(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;

    /// Scan forward until the matching close-brace, respecting nesting and
    /// backslash-escaped braces.  Returns the index *after* the `}`, or
    /// `None` if EOF is reached first.
    fn skip_braces(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start;
        let mut depth: u32 = 1;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => { depth += 1; i += 1; }
                b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 { return Some(i); }
                }
                b'\\' => {
                    i += 1; // skip backslash
                    if i < bytes.len() { i += 1; } // skip escaped char
                }
                _ => { i += 1; }
            }
        }
        None // unmatched
    }

    /// Scan forward until the closing `"`, handling backslash escapes and
    /// nested brackets / command-subs.  Returns index *after* the `"`.
    fn skip_quotes(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => { return Some(i + 1); }
                b'\\' => {
                    i += 1;
                    if i < bytes.len() { i += 1; }
                }
                b'[' => {
                    i += 1;
                    match skip_brackets(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                _ => { i += 1; }
            }
        }
        None
    }

    /// Scan forward until the matching `]`.
    fn skip_brackets(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b']' => { return Some(i + 1); }
                b'{' => {
                    i += 1;
                    match skip_braces(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                b'"' => {
                    i += 1;
                    match skip_quotes(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                b'\\' => {
                    i += 1;
                    if i < bytes.len() { i += 1; }
                }
                b'[' => {
                    i += 1;
                    match skip_brackets(bytes, i) {
                        Some(end) => { i = end; }
                        None => { return None; }
                    }
                }
                _ => { i += 1; }
            }
        }
        None
    }

    // Main scan — top-level Tcl script
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                i += 1;
                match skip_braces(bytes, i) {
                    Some(end) => { i = end; }
                    None => { return false; }
                }
            }
            b'"' => {
                i += 1;
                match skip_quotes(bytes, i) {
                    Some(end) => { i = end; }
                    None => { return false; }
                }
            }
            b'[' => {
                i += 1;
                match skip_brackets(bytes, i) {
                    Some(end) => { i = end; }
                    None => { return false; }
                }
            }
            b'\\' => {
                i += 1;
                if i < bytes.len() { i += 1; }
            }
            _ => { i += 1; }
        }
    }
    true
}
