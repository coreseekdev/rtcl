//! Word-boundary commands (tclsh tclCmdMZ.c): tcl_endOfWord,
//! tcl_startOfNextWord, tcl_startOfPreviousWord, tcl_wordBreakAfter,
//! tcl_wordBreakBefore.
//!
//! A word character is any Unicode alphanumeric plus `_`; all five commands
//! accept Tcl index forms (`end`, `end-1`) and clamp negative starts to 0.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::interp::commands::list::parse_tcl_index;
use crate::value::Value;

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Shared argument handling: (chars, clamped start index).
fn word_args(interp: &mut Interp, args: &[Value], name: &str) -> Result<(Vec<char>, usize)> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage(name, 2, args.len(), "string startIndex"));
    }
    let chars: Vec<char> = args[1].as_str().chars().collect();
    let raw = args[2].as_str();
    // tclsh: a whitespace-only index acts as 0; real parse garbage raises
    // the standard bad-index error.
    let effective =
        if crate::interp::commands::list::trim_tcl_space(raw).is_empty() { "0" } else { raw };
    let start = parse_tcl_index(effective, chars.len())
        .ok_or_else(|| crate::interp::commands::list::bad_index(interp, raw))?;
    let start = start.clamp(0, chars.len() as i64) as usize;
    Ok((chars, start))
}

/// Index just past the word at/after `i`, or None if there is no such
/// character inside the string (tclsh returns -1).
fn end_of_word(chars: &[char], mut i: usize) -> Option<usize> {
    while i < chars.len() && !is_word_char(chars[i]) {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    while i < chars.len() && is_word_char(chars[i]) {
        i += 1;
    }
    if i < chars.len() {
        Some(i)
    } else {
        None
    }
}

/// Start of the first word after the word containing `i`.
fn start_of_next_word(chars: &[char], mut i: usize) -> Option<usize> {
    while i < chars.len() && is_word_char(chars[i]) {
        i += 1;
    }
    while i < chars.len() && !is_word_char(chars[i]) {
        i += 1;
    }
    if i < chars.len() {
        Some(i)
    } else {
        None
    }
}

/// Start of the run of word characters immediately before `i`.
fn start_of_previous_word(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i as isize - 1;
    while j >= 0 && !is_word_char(chars[j as usize]) {
        j -= 1;
    }
    if j < 0 {
        return None;
    }
    while j >= 0 && is_word_char(chars[j as usize]) {
        j -= 1;
    }
    Some((j + 1) as usize)
}

/// First word/non-word transition strictly after `i` (between j-1 and j).
fn word_break_after(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < chars.len() {
        if is_word_char(chars[j - 1]) != is_word_char(chars[j]) {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Last word/non-word transition at or before `i` (between j-1 and j).
fn word_break_before(chars: &[char], i: usize) -> Option<usize> {
    let mut j = (i as isize).min(chars.len() as isize - 1);
    while j >= 1 {
        if is_word_char(chars[j as usize - 1]) != is_word_char(chars[j as usize]) {
            return Some(j as usize);
        }
        j -= 1;
    }
    None
}

fn result(r: Option<usize>) -> Value {
    Value::from_int(r.map(|i| i as i64).unwrap_or(-1))
}

pub fn cmd_end_of_word(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (chars, i) = word_args(interp, args, "tcl_endOfWord")?;
    Ok(result(end_of_word(&chars, i)))
}

pub fn cmd_start_of_next_word(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (chars, i) = word_args(interp, args, "tcl_startOfNextWord")?;
    Ok(result(start_of_next_word(&chars, i)))
}

pub fn cmd_start_of_previous_word(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (chars, i) = word_args(interp, args, "tcl_startOfPreviousWord")?;
    Ok(result(start_of_previous_word(&chars, i)))
}

pub fn cmd_word_break_after(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (chars, i) = word_args(interp, args, "tcl_wordBreakAfter")?;
    Ok(result(word_break_after(&chars, i)))
}

pub fn cmd_word_break_before(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (chars, i) = word_args(interp, args, "tcl_wordBreakBefore")?;
    Ok(result(word_break_before(&chars, i)))
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    /// Probe-verified tclsh 8.6.17 table: each row is the results for
    /// start = -1..=len for one string.
    fn check_table(cmd: &str, s: &str, expected: &[i64]) {
        let mut interp = Interp::new();
        let n = s.chars().count() as i64;
        for (k, start) in (-1..=n).enumerate() {
            let got = interp
                .eval(&format!("{} {:?} {}", cmd, s, start))
                .unwrap()
                .as_int()
                .unwrap();
            assert_eq!(got, expected[k], "{} {:?} start={}", cmd, s, start);
        }
    }

    #[test]
    fn test_end_of_word_table() {
        //                     -1   0   1   2   3   4  end
        check_table("tcl_endOfWord", "abcd", &[-1, -1, -1, -1, -1, -1]);
        check_table("tcl_endOfWord", "ab cd", &[2, 2, 2, -1, -1, -1, -1]);
        check_table("tcl_endOfWord", " cd ", &[3, 3, 3, 3, -1, -1]);
    }

    #[test]
    fn test_start_of_next_word_table() {
        check_table("tcl_startOfNextWord", "abcd", &[-1; 6]);
        check_table("tcl_startOfNextWord", "ab cd", &[3, 3, 3, 3, -1, -1, -1]);
        check_table("tcl_startOfNextWord", " cd ", &[1, 1, -1, -1, -1, -1]);
    }

    #[test]
    fn test_start_of_previous_word_table() {
        check_table("tcl_startOfPreviousWord", "abcd", &[-1, -1, 0, 0, 0, 0]);
        check_table("tcl_startOfPreviousWord", "ab cd", &[-1, -1, 0, 0, 0, 3, 3]);
        check_table("tcl_startOfPreviousWord", " cd ", &[-1, -1, -1, 1, 1, 1]);
    }

    #[test]
    fn test_word_break_after_table() {
        check_table("tcl_wordBreakAfter", "abcd", &[-1; 6]);
        check_table("tcl_wordBreakAfter", "ab cd", &[2, 2, 2, 3, -1, -1, -1]);
        check_table("tcl_wordBreakAfter", " cd ", &[1, 1, 3, 3, -1, -1]);
    }

    #[test]
    fn test_word_break_before_table() {
        check_table("tcl_wordBreakBefore", "abcd", &[-1; 6]);
        check_table("tcl_wordBreakBefore", "ab cd", &[-1, -1, -1, 2, 3, 3, 3]);
        check_table("tcl_wordBreakBefore", " cd ", &[-1, -1, 1, 1, 3, 3]);
    }

    #[test]
    fn test_word_punct_and_underscore() {
        check_table("tcl_endOfWord", "ab-cd", &[2, 2, 2, -1, -1, -1, -1]);
        check_table("tcl_endOfWord", "a_b c", &[3, 3, 3, 3, -1, -1, -1]);
        check_table("tcl_wordBreakAfter", "ab-cd", &[2, 2, 2, 3, -1, -1, -1]);
        check_table("tcl_wordBreakBefore", "ab-cd", &[-1, -1, -1, 2, 3, 3, 3]);
    }

    #[test]
    fn test_word_end_index_forms() {
        // corpus uses `end` / `end-1` as the start argument
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("tcl_wordBreakAfter {ab cd} end")
                .unwrap()
                .as_int()
                .unwrap(),
            -1
        );
        assert_eq!(
            interp
                .eval("tcl_endOfWord {ab cd} end-4")
                .unwrap()
                .as_int()
                .unwrap(),
            2
        );
    }

    #[test]
    fn test_word_unicode_wordchars() {
        // é is alphanumeric → part of a word
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("tcl_endOfWord {café!} 2")
                .unwrap()
                .as_int()
                .unwrap(),
            4
        );
    }

    #[test]
    fn test_word_whitespace_index_is_zero() {
        // tclsh: `tcl_startOfPreviousWord "ab cd" {}` → -1, no error
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval(r#"tcl_startOfPreviousWord "ab cd" {}"#)
                .unwrap()
                .as_int()
                .unwrap(),
            -1
        );
        assert!(interp.eval(r#"tcl_endOfWord "ab cd" xyz"#).is_err());
    }
}
