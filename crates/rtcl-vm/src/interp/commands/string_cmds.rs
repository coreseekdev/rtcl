//! String commands: string subcommands.

use crate::error::{Error, Result};
use crate::interp::commands::list::{bad_index, parse_tcl_index, tcl_get_int};
use crate::interp::unicode;
use crate::interp::{glob_match, Interp};
use crate::value::Value;

/// tclsh's `string` ensemble in canonical order (used verbatim in the
/// unknown-subcommand message). `byterange` is an rtcl extension and is
/// matched exactly only, so it never shows up in that message.
const STRING_SUBCMDS: &[&str] = &[
    "bytelength", "cat", "compare", "equal", "first", "index", "is", "last",
    "length", "map", "match", "range", "repeat", "replace", "reverse",
    "tolower", "totitle", "toupper", "trim", "trimleft", "trimright",
    "wordend", "wordstart",
];
const STRING_EXTRA_SUBCMDS: &[&str] = &["byterange"];

/// Resolve a (possibly abbreviated) `string` subcommand the way
/// Tcl_GetIndexFromObj does: exact match wins, a unique prefix resolves,
/// anything else is "unknown or ambiguous".
fn resolve_string_subcmd(raw: &str) -> Result<String> {
    if STRING_EXTRA_SUBCMDS.contains(&raw) {
        return Ok(raw.to_string());
    }
    if let Some(exact) = STRING_SUBCMDS.iter().find(|s| **s == raw) {
        return Ok((*exact).to_string());
    }
    let matches: Vec<&str> = STRING_SUBCMDS
        .iter()
        .copied()
        .filter(|s| s.starts_with(raw))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).to_string()),
        _ => {
            let list = STRING_SUBCMDS
                .join(", ")
                .replacen(", wordstart", ", or wordstart", 1);
            Err(Error::Msg(format!(
                "unknown or ambiguous subcommand \"{}\": must be {}",
                raw, list
            )))
        }
    }
}

/// tclsh's `string is` class names, in the exact order used by its
/// "must be ..." message (which is not alphabetical).
const STRING_IS_CLASSES: &[&str] = &[
    "alnum", "alpha", "ascii", "control", "boolean", "digit", "double",
    "entier", "false", "graph", "integer", "list", "lower", "print",
    "punct", "space", "true", "upper", "wideinteger", "wordchar", "xdigit",
];

fn string_is_class_list() -> String {
    STRING_IS_CLASSES
        .join(", ")
        .replacen(", xdigit", ", or xdigit", 1)
}

fn resolve_is_class(interp: &mut Interp, raw: &str) -> Result<String> {
    if let Some(exact) = STRING_IS_CLASSES.iter().find(|s| **s == raw) {
        return Ok((*exact).to_string());
    }
    let matches: Vec<&str> = STRING_IS_CLASSES
        .iter()
        .copied()
        .filter(|s| s.starts_with(raw))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).to_string()),
        _ => {
            let kind = if matches.is_empty() { "bad" } else { "ambiguous" };
            super::list::set_error_code(
                interp,
                &format!(
                    "TCL LOOKUP INDEX class {}",
                    crate::value::tcl_quote(raw)
                ),
            );
            Err(Error::Msg(format!(
                "{} class \"{}\": must be {}",
                kind,
                raw,
                string_is_class_list()
            )))
        }
    }
}

/// Evaluate `string is class` over a non-empty/empty string. Returns the
/// boolean result plus (on false) the `-failindex` value.
fn string_is_eval(class: &str, s: &str, strict: bool) -> (bool, Option<usize>) {
    if s.is_empty() {
        // Empty strings satisfy every class unless -strict is given; the
        // list class is the sole exception (the empty list is valid).
        if strict && class != "list" {
            return (false, Some(0));
        }
        return (true, None);
    }
    let chars: Vec<char> = s.chars().collect();
    let char_class = |f: fn(char) -> bool| {
        match chars.iter().position(|c| !f(*c)) {
            Some(i) => (false, Some(i)),
            None => (true, None),
        }
    };
    match class {
        "alnum" => char_class(unicode::is_alnum),
        "alpha" => char_class(unicode::is_alpha),
        "ascii" => char_class(|c| c.is_ascii()),
        "control" => char_class(unicode::is_control),
        // tclsh 8.6 的 `string is digit` = 手工滞后的 Nd 旧表（devanagari
        // ९ = 1 而 mathematical 𝟘 U+1D7D8 / Adlam = 0——probe6 钉形）；
        // 完整 Nd 保留给 \d/[[:digit:]] 的 regexp 面（re:digit 行）。
        "digit" => char_class(unicode::is_digit_classic),
        "graph" => char_class(unicode::is_graph),
        // tclsh 8.6 分类 = 类别表本身（Lu/Ll），不是 Rust 的
        // Other_Lowercase 扩集（modifier letter ˡ U+02E1 tclsh = 0，
        // probe4 钉形）。
        "lower" => char_class(unicode::is_ll),
        "print" => char_class(unicode::is_print),
        "punct" => char_class(unicode::is_punct),
        "space" => char_class(unicode::is_space),
        "upper" => char_class(unicode::is_lu),
        "wordchar" => char_class(unicode::is_word),
        "xdigit" => char_class(|c| c.is_ascii_hexdigit()),
        "boolean" => match tcl_boolean_value(s) {
            Some(_) => (true, None),
            None => (false, Some(0)),
        },
        "false" => match tcl_boolean_value(s) {
            Some(false) => (true, None),
            _ => (false, Some(0)),
        },
        "true" => match tcl_boolean_value(s) {
            Some(true) => (true, None),
            _ => (false, Some(0)),
        },
        "integer" => match scan_tcl_integer(s) {
            Ok((_, mag)) if mag <= u32::MAX as u128 => (true, None),
            Ok(_) => (false, Some(usize::MAX)),
            Err(pos) => (false, Some(pos)),
        },
        "wideinteger" => match scan_tcl_integer(s) {
            Ok((_, mag)) if mag <= u64::MAX as u128 => (true, None),
            Ok(_) => (false, Some(usize::MAX)),
            Err(pos) => (false, Some(pos)),
        },
        "entier" => match scan_tcl_integer(s) {
            Ok(_) => (true, None),
            Err(pos) => (false, Some(pos)),
        },
        "double" => match scan_tcl_double(s) {
            Ok(()) => (true, None),
            Err(pos) => (false, Some(pos)),
        },
        "list" => match Value::from_str(s).as_list() {
            Some(_) => (true, None),
            None => (false, Some(usize::MAX)),
        },
        _ => (false, Some(0)),
    }
}

/// Scan Tcl integer syntax. Returns (has_digits, magnitude) where overflow
/// of the magnitude accumulator saturates (callers compare against their
/// class bound). Err carries the `-failindex` character position.
fn scan_tcl_integer(s: &str) -> std::result::Result<(bool, u128), usize> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() && chars[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
        i += 1;
        if i >= chars.len() {
            return Err(start);
        }
    }
    let mut base: u32 = 10;
    let mut prefix_start = None;
    if i < chars.len() && chars[i] == '0' {
        if i + 1 < chars.len() {
            match chars[i + 1] {
                'x' | 'X' => { base = 16; prefix_start = Some(i); i += 2; }
                'b' | 'B' => { base = 2; prefix_start = Some(i); i += 2; }
                'o' | 'O' => { base = 8; prefix_start = Some(i); i += 2; }
                _ => { base = 8; }
            }
        } else {
            base = 8;
        }
    }
    let digits_start = i;
    let mut mag: u128 = 0;
    let mut overflow = false;
    while i < chars.len() {
        let d = match chars[i].to_digit(36) {
            Some(d) if d < base => d,
            _ => break,
        };
        match mag.checked_mul(base as u128).and_then(|m| m.checked_add(d as u128)) {
            Some(v) => mag = v,
            None => overflow = true,
        }
        i += 1;
    }
    if i == digits_start {
        // 0x/0b/0o without digits: tclsh blames the prefix character.
        return Err(prefix_start.unwrap_or(start) + if prefix_start.is_some() { 1 } else { 0 });
    }
    while i < chars.len() && chars[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < chars.len() {
        return Err(i);
    }
    if overflow {
        // Saturated: callers treat this as out of range (-1 failindex).
        return Ok((true, u128::MAX));
    }
    Ok((true, mag))
}

/// Scan Tcl double syntax: Inf/NaN, any Tcl integer form, or decimal
/// floating point with optional exponent. Range overflow is accepted.
fn scan_tcl_double(s: &str) -> std::result::Result<(), usize> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() && chars[i].is_ascii_whitespace() {
        i += 1;
    }
    let body_start = i;
    let mut j = i;
    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
        j += 1;
    }
    let rest: String = chars[j..].iter().collect::<String>().to_ascii_lowercase();
    let rest = rest.trim_end_matches(|c: char| c.is_ascii_whitespace());
    if matches!(rest, "inf" | "infinity" | "nan") {
        return Ok(());
    }
    if scan_tcl_integer(s).is_ok() {
        return Ok(());
    }
    // Decimal float. A leading 0 followed by a digit is octal (handled by
    // scan_tcl_integer), so failing that is a hard error, not a decimal.
    if j + 1 < chars.len() && chars[j] == '0' && chars[j + 1].is_ascii_digit() {
        return Err(scan_tcl_integer(s).err().unwrap_or(1));
    }
    let mut k = j;
    let mut mantissa_digits = 0;
    while k < chars.len() && chars[k].is_ascii_digit() {
        mantissa_digits += 1;
        k += 1;
    }
    let mut had_dot = false;
    if k < chars.len() && chars[k] == '.' {
        had_dot = true;
        k += 1;
        while k < chars.len() && chars[k].is_ascii_digit() {
            mantissa_digits += 1;
            k += 1;
        }
    }
    if mantissa_digits == 0 {
        return Err(body_start);
    }
    let exp_at = k;
    if k < chars.len() && (chars[k] == 'e' || chars[k] == 'E') {
        k += 1;
        if k < chars.len() && (chars[k] == '+' || chars[k] == '-') {
            k += 1;
        }
        let exp_digits = k;
        while k < chars.len() && chars[k].is_ascii_digit() {
            k += 1;
        }
        if k == exp_digits {
            return Err(exp_at);
        }
    }
    if !had_dot && exp_at == k {
        // No decimal point and no exponent: not a double unless it was a
        // valid integer (already handled above).
        return Err(k);
    }
    while k < chars.len() && chars[k].is_ascii_whitespace() {
        k += 1;
    }
    if k < chars.len() {
        return Err(k);
    }
    Ok(())
}

/// (minimum, maximum, usage suffix) per subcommand, matching tclsh's own
/// arity messages. `cat` accepts zero strings (min 2); compare/equal/is
/// `string map`: one pass over `input`; at each position the first source
/// that matches there wins (tclsh: the order of the map), otherwise one
/// character is copied.  Byte-wise — a UTF-8 sequence only ever starts
/// with a byte no other sequence contains, so no match can split one.
fn string_map(input: &str, sources: &[&str], targets: &[&str]) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    // The untranslated run that ends at `i`.
    let mut start = 0;
    let mut i = 0;
    // The bytes a source can start with: at any other byte there is no
    // looking at the sources at all.
    let mut firsts = [false; 256];
    for from in sources.iter().filter(|from| !from.is_empty()) {
        firsts[from.as_bytes()[0] as usize] = true;
    }
    while i < bytes.len() {
        let rest = &bytes[i..];
        let hit = if firsts[bytes[i] as usize] {
            sources.iter().zip(targets).find_map(|(from, to)| {
                (!from.is_empty() && rest.starts_with(from.as_bytes())).then_some((from.len(), *to))
            })
        } else {
            None
        };
        match hit {
            Some((len, to)) => {
                out.push_str(&input[start..i]);
                out.push_str(to);
                i += len;
                start = i;
            }
            None => i += utf8_char_len(bytes[i]),
        }
    }
    out.push_str(&input[start..]);
    out
}

/// The number of bytes of the UTF-8 character that starts with `lead`.
fn utf8_char_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// A character's lowercase as tclsh compares it: the simple (one for one)
/// mapping.  Rust's is the full one, which differs for the characters
/// whose lowercase is several characters long — of those only LATIN
/// CAPITAL LETTER I WITH DOT ABOVE has a simple mapping of its own.
fn lower_char(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ if c == '\u{0130}' => 'i',
        _ => c,
    }
}

/// `string map -nocase`: the same pass over lowercased characters; what is
/// copied out comes from the original.
fn string_map_nocase(input: &str, sources: &[&str], targets: &[&str]) -> String {
    let chars: Vec<char> = input.chars().collect();
    let lowered: Vec<char> = chars.iter().map(|c| lower_char(*c)).collect();
    let sources: Vec<Vec<char>> = sources
        .iter()
        .map(|from| from.chars().map(lower_char).collect())
        .collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let rest = &lowered[i..];
        let hit = sources.iter().zip(targets).find_map(|(from, to)| {
            (!from.is_empty() && rest.starts_with(from.as_slice())).then_some((from.len(), *to))
        });
        match hit {
            Some((len, to)) => {
                out.push_str(to);
                i += len;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// parse trailing words as options, so they have no maximum here.
fn string_subcmd_usage(sub: &str) -> Option<(usize, Option<usize>, &'static str)> {
    Some(match sub {
        "cat" => (2, None, "?string ...?"),
        "bytelength" | "length" | "reverse" => (3, Some(3), "string"),
        "compare" | "equal" => (4, None, "?-nocase? ?-length int? string1 string2"),
        "first" | "last" => (4, Some(5), "needleString haystackString ?startIndex?"),
        "index" => (4, Some(4), "string charIndex"),
        "is" => (4, None, "class ?-strict? ?-failindex var? str"),
        "map" => (4, Some(5), "?-nocase? charMap string"),
        "match" => (4, Some(6), "?-nocase? pattern string"),
        "range" => (5, Some(5), "string first last"),
        "repeat" => (4, Some(4), "string count"),
        "replace" => (5, Some(6), "string first last ?string?"),
        "tolower" | "totitle" | "toupper" => (3, Some(5), "string ?first? ?last?"),
        "trim" | "trimleft" | "trimright" => (3, Some(4), "string ?chars?"),
        "wordend" | "wordstart" => (4, Some(4), "string index"),
        "byterange" => (5, Some(5), "string first last"),
        _ => return None,
    })
}

pub fn cmd_string(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "string", 2, args.len(), "subcommand ?arg ...?",
        ));
    }

    let subcmd = resolve_string_subcmd(args[1].as_str())?;
    if let Some((min, max, usage)) = string_subcmd_usage(&subcmd) {
        if args.len() < min || max.is_some_and(|m| args.len() > m) {
            return Err(Error::wrong_args_with_usage(
                format!("string {}", subcmd), min, args.len(), usage,
            ));
        }
    }
    // lazy-string（§6）：equal/compare/cat 的操作数可能持宿主句柄——主体
    // 串对这三个子命令延迟物化（equal 有免物化快路径；compare 刻意物化，
    // 见 host_string.rs 白名单表；cat 全折叠）。其余子命令照旧即时物化。
    let lazy_subject = matches!(subcmd.as_str(), "equal" | "compare" | "cat");
    let str_val = if lazy_subject {
        ""
    } else {
        args.get(2).map(|v| v.as_str()).unwrap_or("")
    };

    match subcmd.as_str() {
        "length" => Ok(Value::from_int(char_count(str_val) as i64)),
        "bytelength" => Ok(Value::from_int(str_val.len() as i64)),
        "tolower" => string_case_range(interp, str_val, args, CaseMode::Lower),
        "toupper" => string_case_range(interp, str_val, args, CaseMode::Upper),
        "totitle" => string_case_range(interp, str_val, args, CaseMode::Title),
        "trim" => {
            let chars = if args.len() > 3 { args[3].as_str() } else { " \t\n\r" };
            Ok(Value::from_str(str_val.trim_matches(|c| chars.contains(c))))
        }
        "trimleft" => {
            let chars = if args.len() > 3 { args[3].as_str() } else { " \t\n\r" };
            Ok(Value::from_str(str_val.trim_start_matches(|c| chars.contains(c))))
        }
        "trimright" => {
            let chars = if args.len() > 3 { args[3].as_str() } else { " \t\n\r" };
            Ok(Value::from_str(str_val.trim_end_matches(|c| chars.contains(c))))
        }
        "range" => {
            if args.len() != 5 {
                return Err(Error::wrong_args("string range", 5, args.len()));
            }
            let len = char_count(str_val);
            let start_raw = match parse_tcl_index(args[3].as_str(), len) {
                Some(v) => v,
                None => return Err(bad_index(interp, args[3].as_str())),
            };
            let end_raw = match parse_tcl_index(args[4].as_str(), len) {
                Some(v) => v,
                None => return Err(bad_index(interp, args[4].as_str())),
            };
            // Tcl clamps: start < 0 → 0; end >= len → len-1; empty when
            // the range is backwards or starts past the end.
            let start = start_raw.max(0);
            let end = end_raw.min(len as i64 - 1);
            if start <= end && start < len as i64 {
                Ok(Value::from_str(char_slice(
                    str_val,
                    start as usize,
                    end as usize + 1,
                )))
            } else {
                Ok(Value::empty())
            }
        }
        "index" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string index", 4, args.len()));
            }
            // Only `end` needs the length; a number goes straight to its
            // character.  (The whole string was collected first: every
            // `string index` on a 24KB string cost 8µs.)
            let index = args[3].as_str();
            let len = || char_count(str_val);
            match parse_tcl_index(index, if super::list::index_is_from_end(index) { len() } else { 0 }) {
                Some(idx) if idx >= 0 => {
                    let found = if str_val.is_ascii() {
                        str_val.as_bytes().get(idx as usize).map(|b| *b as char)
                    } else {
                        str_val.chars().nth(idx as usize)
                    };
                    Ok(found.map_or_else(Value::empty, |c| Value::from_str(&c.to_string())))
                }
                Some(_) => Ok(Value::empty()),
                None => Err(bad_index(interp, index)),
            }
        }
        "equal" => {
            // lazy-string 白名单（§6）：裸形 `string equal s1 s2`（恰 4 词
            // = 无 -nocase/-length）两操作数都持宿主句柄时免物化比较。
            // UTF-8↔UTF-16 对合法标量值双射 → 深层相等安全；池内容恒为
            // 合法标量值（入池孤立代理过滤，见 host_string.rs 契约）。
            if args.len() == 4 {
                if let (Some(a), Some(b)) =
                    (args[2].host_str_handle(), args[3].host_str_handle())
                {
                    if let Some(eq) = crate::host_string::with_backing(|bk| bk.equal(a, b)) {
                        return Ok(Value::from_bool(eq));
                    }
                }
            }
            let (nocase, length, s1, s2) = parse_compare_args(interp, args, "equal")?;
            let a = truncate_chars(s1, length);
            let b = truncate_chars(s2, length);
            if nocase {
                Ok(Value::from_bool(a.to_lowercase() == b.to_lowercase()))
            } else {
                Ok(Value::from_bool(a == b))
            }
        }
        "compare" => {
            let (nocase, length, s1, s2) = parse_compare_args(interp, args, "compare")?;
            let a = truncate_chars(s1, length);
            let b = truncate_chars(s2, length);
            let cmp = if nocase {
                a.to_lowercase().cmp(&b.to_lowercase())
            } else {
                a.cmp(&b)
            };
            Ok(Value::from_int(match cmp {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        }
        "match" => {
            // tclsh 8.6.17 实测（三形状钉死）：位置词必须 2 或 3 个；
            // 只有 3 个位置词时首词才是选项（Tcl C：
            // `length > 1 && strncmp(word, "-nocase", length) == 0`，
            // 大小写敏感、唯一前缀 -n/-noc/-nocas 可）。
            // 2 个位置词时首词即使写成 -nocase/-x 也按 pattern 处理
            // （`string match -* $opt` 合法，ui.tcl 依赖）。
            let npos = args.len().saturating_sub(2);
            if !(2..=3).contains(&npos) {
                return Err(Error::wrong_args_with_usage(
                    "string match",
                    4,
                    args.len(),
                    "?-nocase? pattern string",
                ));
            }
            let (nocase, i) = if npos == 3 {
                let word = args[2].as_str();
                if word.len() > 1 && "-nocase".starts_with(word) {
                    (true, 3)
                } else {
                    // tclsh：Tcl_GetIndexFromObj 系前缀匹配失败 →
                    // TCL LOOKUP INDEX option <word>。
                    super::list::set_error_code(
                        interp,
                        &format!("TCL LOOKUP INDEX option {}", word),
                    );
                    return Err(Error::Msg(format!(
                        "bad option \"{}\": must be -nocase",
                        word
                    )));
                }
            } else {
                (false, 2)
            };
            let pattern = args[i].as_str();
            let text = args[i + 1].as_str();
            // D-22②：-nocase 逐字符简单折叠（原整串 to_lowercase——
            // İ 两字符展开与 tclsh 的单字符折叠相悖）。
            let fold = |s: &str| -> String {
                // 简单折叠：单字符（tclsh 的 İ→i；Rust 全折叠会出
                // i̇ 两字符——串 match 的逐字语义不兼容）。
                s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
            };
            if nocase {
                Ok(Value::from_bool(glob_match(&fold(pattern), &fold(text))))
            } else {
                Ok(Value::from_bool(glob_match(pattern, text)))
            }
        }
        "first" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string first", 4, args.len()));
            }
            // string first needleString haystackString ?startIndex?
            let needle = args[2].as_str();
            let haystack = args[3].as_str();
            let len = char_count(haystack);
            let start = if args.len() > 4 {
                match parse_tcl_index(args[4].as_str(), len) {
                    Some(v) => v.max(0) as usize,
                    None => return Err(bad_index(interp, args[4].as_str())),
                }
            } else {
                0
            };
            // The library's search, over bytes: a match of a valid UTF-8
            // needle lies on a character boundary, so the offset counts
            // back to characters in one pass.  (The search compared a
            // slice at every position, after collecting the text.)
            let pos = if needle.is_empty()
                || needle.len() > haystack.len()
                || start > haystack.len() - needle.len()
            {
                None
            } else {
                let from = char_byte_offset(haystack, start);
                haystack[from..]
                    .find(needle)
                    .map(|at| start + haystack[from..from + at].chars().count())
            };
            Ok(Value::from_int(pos.map_or(-1, |i| i as i64)))
        }
        "last" => {
            if args.len() < 4 {
                return Err(Error::wrong_args("string last", 4, args.len()));
            }
            // string last needleString haystackString ?lastIndex?
            let needle = args[2].as_str();
            let haystack = args[3].as_str();
            let len = char_count(haystack);
            // A match must end at or before lastIndex (default: end of string).
            let last = if args.len() > 4 {
                match parse_tcl_index(args[4].as_str(), len) {
                    // Negative index: no character is at or before it.
                    Some(i) if i < 0 => return Ok(Value::from_int(-1)),
                    Some(i) => (i as usize).min(len.saturating_sub(1)),
                    None => return Err(bad_index(interp, args[4].as_str())),
                }
            } else {
                len.saturating_sub(1)
            };
            let pos = if needle.is_empty() || needle.len() > haystack.len() {
                None
            } else {
                // Up to and including the character `last`; see `first`.
                let upto = char_byte_offset(haystack, last + 1);
                haystack[..upto]
                    .rfind(needle)
                    .map(|at| haystack[..at].chars().count())
            };
            Ok(Value::from_int(pos.map_or(-1, |i| i as i64)))
        }
        "map" => {
            if args.len() < 4 || args.len() > 5 {
                return Err(Error::wrong_args_with_usage("string map", 4, args.len(), "?-nocase? mapping string"));
            }
            let (nocase, idx) = if args.len() == 5 {
                if args[2].as_str() != "-nocase" {
                    super::list::set_error_code(
                        interp,
                        &format!(
                            "TCL LOOKUP INDEX option {}",
                            crate::value::tcl_quote(args[2].as_str())
                        ),
                    );
                    return Err(Error::Msg(format!(
                        "bad option \"{}\": must be -nocase",
                        args[2].as_str()
                    )));
                }
                (true, 3)
            } else {
                (false, 2)
            };
            // tclsh: the map has to be a list, with an even number of
            // elements.
            let mapping = super::list::strict_list(interp, &args[idx])?;
            if mapping.len() % 2 != 0 {
                super::list::set_error_code(interp, "TCL OPERATION MAP UNBALANCED");
                return Err(Error::Msg("char map list unbalanced".to_string()));
            }
            let sources: Vec<&str> = (0..mapping.len() / 2).map(|k| mapping[2 * k].as_str()).collect();
            let targets: Vec<&str> = (0..mapping.len() / 2).map(|k| mapping[2 * k + 1].as_str()).collect();
            let input = args[idx + 1].as_str();
            let result = if nocase {
                string_map_nocase(input, &sources, &targets)
            } else {
                string_map(input, &sources, &targets)
            };
            Ok(Value::from_str(&result))
        }
        "repeat" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("string repeat", 4, args.len()));
            }
            // Tcl_GetInt semantics — a single (trimmed) integer literal,
            // no `int+int` chains: `string repeat ab 1+2` errors with
            // `expected integer but got "1+2"` (probed).
            let count = match tcl_get_int(args[3].as_str()) {
                Some(n) => n,
                None => {
                    return Err(Error::runtime(
                        format!("expected integer but got \"{}\"", args[3].as_str()),
                        crate::error::ErrorCode::InvalidOp,
                    ))
                }
            };
            if count > i32::MAX as i64 || count < i32::MIN as i64 {
                // tclsh parses the count as a C `int`.
                super::list::set_error_code(
                    interp,
                    "ARITH IOVERFLOW {integer value too large to represent}",
                );
                return Err(Error::runtime(
                    "integer value too large to represent".to_string(),
                    crate::error::ErrorCode::Generic,
                ));
            }
            if count < 0 {
                return Ok(Value::empty());
            }
            // Result length is checked up front (tclsh: `result exceeds max
            // size for a Tcl value (2147483647 bytes)`) and the allocation
            // is fallible: the unchecked `repeat` was a capacity panic or
            // an aborting allocation on `string repeat x 4294967296`.
            let total = match (count as usize).checked_mul(str_val.len()) {
                Some(n) if n <= super::list::MAX_VALUE_BYTES => n,
                _ => {
                    super::list::set_error_code(interp, "TCL MEMORY");
                    return Err(Error::runtime(
                        "result exceeds max size for a Tcl value (2147483647 bytes)".to_string(),
                        crate::error::ErrorCode::Generic,
                    ));
                }
            };
            let mut out = String::new();
            out.try_reserve(total)
                .map_err(|_| super::list::out_of_memory(interp, total))?;
            for _ in 0..count {
                out.push_str(&str_val);
            }
            Ok(Value::from_str(&out))
        }
        "reverse" => {
            Ok(Value::from_str(&str_val.chars().rev().collect::<String>()))
        }
        "replace" => {
            if args.len() < 5 {
                return Err(Error::wrong_args_with_usage("string replace", 5, args.len(), "string first last ?newString?"));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len() as i64;
            let first_raw = match parse_tcl_index(args[3].as_str(), chars.len()) {
                Some(v) => v,
                None => return Err(bad_index(interp, args[3].as_str())),
            };
            let last_raw = match parse_tcl_index(args[4].as_str(), chars.len()) {
                Some(v) => v,
                None => return Err(bad_index(interp, args[4].as_str())),
            };
            let first = first_raw.max(0);
            let last = last_raw.min(len - 1);
            let new_str = if args.len() > 5 { args[5].as_str() } else { "" };
            if first > last || first >= len {
                return Ok(Value::from_str(str_val));
            }
            let mut result: String = chars[..first as usize].iter().collect();
            result.push_str(new_str);
            if last + 1 < len {
                result.extend(&chars[(last + 1) as usize..]);
            }
            Ok(Value::from_str(&result))
        }
        "is" => {
            // string is class ?-strict? ?-failindex var? str
            let n = args.len();
            let class = resolve_is_class(interp, args[2].as_str())?;
            let mut strict = false;
            let mut failvar: Option<String> = None;
            // tclsh parses options only when there are more than four
            // words: `string is integer -strict` tests the literal word
            // "-strict" as the value.
            if n > 4 {
                let mut i = 3;
                while i + 1 < n {
                    match args[i].as_str() {
                        "-strict" => {
                            strict = true;
                            i += 1;
                        }
                        "-failindex" => {
                            i += 1;
                            if i + 1 >= n {
                                return Err(Error::wrong_args_with_usage(
                                    format!("string is {}", class),
                                    5,
                                    n,
                                    "?-strict? ?-failindex var? str",
                                ));
                            }
                            failvar = Some(args[i].as_str().to_string());
                            i += 1;
                        }
                        other => {
                            super::list::set_error_code(
                                interp,
                                &format!(
                                    "TCL LOOKUP INDEX option {}",
                                    crate::value::tcl_quote(other)
                                ),
                            );
                            return Err(Error::Msg(format!(
                                "bad option \"{}\": must be -strict or -failindex",
                                other
                            )));
                        }
                    }
                }
            }
            let test_val = args[n - 1].as_str();
            let (result, fail_at) = string_is_eval(&class, test_val, strict);
            if !result {
                if let (Some(var), Some(idx)) = (failvar.as_deref(), fail_at) {
                    let _ = interp.set_var(var, Value::from_int(idx as i64));
                }
            }
            Ok(Value::from_bool(result))
        }
        "cat" => {
            // string cat str1 ?str2 ...?
            // lazy-string 白名单（§6）：全宿主句柄且 ≥2 个时免物化折叠拼接
            // （「拼接→转码 ≡ 转码→拼接」，合法标量值）；concat 产出新槽
            // （1 引用起算，所有权移交 from_host_str）。单参数宿主形退化为
            // 共享原值的克隆（Value 引用计数，不转手池引用）。其余形态走
            // 既有物化路径。
            if args.len() == 3 {
                if args[2].host_str_handle().is_some() {
                    return Ok(args[2].clone());
                }
            } else if args.len() > 3 {
                let handles: Option<Vec<u32>> =
                    args[2..].iter().map(Value::host_str_handle).collect();
                if let Some(handles) = handles {
                    // 折叠：首个 acc 借自参数（非持有）；每步产出新槽后
                    // 释放上一轮**持有的**中间槽（concat 借用语义，见
                    // HostStringBacking::concat）。≥2 参数时最终 acc 恒为
                    // 持有态 → 所有权移交 from_host_str。
                    let folded = crate::host_string::with_backing(|bk| {
                        let mut it = handles.into_iter();
                        let mut acc = it.next()?;
                        let mut owned = false;
                        for h in it {
                            let next = bk.concat(acc, h);
                            if owned {
                                bk.release(acc);
                            }
                            acc = next;
                            owned = true;
                        }
                        Some(acc)
                    });
                    // with_backing 的 R 本身已是 Option<u32>（闭包内的 ?），
                    // 外层再 flatten 一层。
                    if let Some(idx) = folded.flatten() {
                        return Ok(Value::from_host_str(idx));
                    }
                }
            }
            let mut result = String::new();
            for arg in &args[2..] {
                result.push_str(arg.as_str());
            }
            Ok(Value::from_str(&result))
        }
        "byterange" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "string byterange", 5, args.len(), "string first last",
                ));
            }
            let bytes = str_val.as_bytes();
            let len = bytes.len() as i64;
            let first_raw = match parse_tcl_index(args[3].as_str(), bytes.len()) {
                Some(v) => v,
                None => return Err(bad_index(interp, args[3].as_str())),
            };
            let last_raw = match parse_tcl_index(args[4].as_str(), bytes.len()) {
                Some(v) => v,
                None => return Err(bad_index(interp, args[4].as_str())),
            };
            let first = first_raw.max(0);
            let last = last_raw.min(len - 1);
            if first <= last && first < len {
                let end = (last + 1).min(len);
                let s = String::from_utf8_lossy(&bytes[first as usize..end as usize]);
                Ok(Value::from_str(&s))
            } else {
                Ok(Value::empty())
            }
        }
        "wordstart" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "string wordstart", 4, args.len(), "string index",
                ));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            let idx = match parse_tcl_index(args[3].as_str(), len) {
                Some(v) => v.max(0).min(len as i64 - 1).max(0) as usize,
                None => return Err(bad_index(interp, args[3].as_str())),
            };
            let mut start = idx;
            while start > 0 && is_word_char(chars[start - 1]) {
                start -= 1;
            }
            Ok(Value::from_int(start as i64))
        }
        "wordend" => {
            if args.len() != 4 {
                return Err(Error::wrong_args_with_usage(
                    "string wordend", 4, args.len(), "string index",
                ));
            }
            let chars: Vec<char> = str_val.chars().collect();
            let len = chars.len();
            let idx = match parse_tcl_index(args[3].as_str(), len) {
                Some(v) => v.max(0).min(len as i64 - 1).max(0) as usize,
                None => return Err(bad_index(interp, args[3].as_str())),
            };
            let mut end = idx;
            while end < len && is_word_char(chars[end]) {
                end += 1;
            }
            Ok(Value::from_int(end as i64))
        }
        _ => Err(Error::runtime(
            format!("unknown string subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

/// Parse `-nocase` / `-length N` for `string compare/equal`, mirroring
/// tclsh's C loop: with exactly two words left after the subcommand no
/// option parsing happens at all (`string compare -nocase a` compares the
/// literal strings), and with more words every word before the final two
/// must be a recognized option (`string compare a b c` → bad option "a").
fn parse_compare_args<'a>(
    interp: &mut Interp,
    args: &'a [Value],
    name: &str,
) -> Result<(bool, Option<usize>, &'a str, &'a str)> {
    let n = args.len();
    if n < 4 {
        return Err(Error::wrong_args_with_usage(
            format!("string {}", name),
            4,
            n,
            "?-nocase? ?-length int? string1 string2",
        ));
    }
    let mut nocase = false;
    let mut length: Option<usize> = None;
    let mut i = 2;
    while i + 2 < n {
        match args[i].as_str() {
            "-nocase" => {
                nocase = true;
                i += 1;
            }
            "-length" => {
                i += 1;
                // The value must still leave two positional strings.
                if i >= n - 2 {
                    return Err(Error::wrong_args_with_usage(
                        format!("string {}", name),
                        4,
                        n,
                        "?-nocase? ?-length int? string1 string2",
                    ));
                }
                let v = match tcl_get_int(args[i].as_str()) {
                    Some(v) => v,
                    None => {
                        crate::interp::commands::list::set_error_code(
                            interp, "TCL VALUE INTEGER",
                        );
                        return Err(Error::runtime(
                            format!("expected integer but got \"{}\"", args[i].as_str()),
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                };
                // Negative lengths mean "compare the whole string".
                length = if v < 0 { None } else { Some(v as usize) };
                i += 1;
            }
            other => {
                crate::interp::commands::list::set_error_code(
                    interp,
                    &format!(
                        "TCL LOOKUP INDEX option {}",
                        crate::value::tcl_quote(other)
                    ),
                );
                return Err(Error::runtime(
                    format!("bad option \"{}\": must be -nocase or -length", other),
                    crate::error::ErrorCode::InvalidOp,
                ));
            }
        }
    }
    Ok((nocase, length, args[n - 2].as_str(), args[n - 1].as_str()))
}

fn truncate_chars(s: &str, length: Option<usize>) -> String {
    match length {
        Some(n) => s.chars().take(n).collect(),
        None => s.to_string(),
    }
}

/// The length of `s` in characters — its byte length where all its
/// bytes are characters.
fn char_count(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.chars().count()
    }
}

/// The byte offset of character `ci` (`s.len()` past the end).
fn char_byte_offset(s: &str, ci: usize) -> usize {
    if s.is_ascii() {
        (ci).min(s.len())
    } else {
        s.char_indices().map(|(b, _)| b).nth(ci).unwrap_or(s.len())
    }
}

/// `s` from character `start` up to (not including) character `end`.
fn char_slice(s: &str, start: usize, end: usize) -> &str {
    &s[char_byte_offset(s, start)..char_byte_offset(s, end)]
}

/// A word character: alphanumeric or underscore (jimtcl convention).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Tcl titlecase (UCS4ToTitle, tclUtf.c): only Ll characters with a simple
/// (1:1) uppercase move; Lu and Lt characters are already "titled", and
/// characters whose uppercase expands to multiple chars (ß, ﬀ, ...) have no
/// entry in Tcl's table, so they stay. Georgian Mkhedruli (U+10D0..U+10FF)
/// carry Tcl's special case mode 0x7: `string toupper` maps them to Mtavruli
/// but their titlecase is themselves.
#[derive(Clone, Copy)]
enum CaseMode {
    Lower,
    Upper,
    Title,
}

/// `string tolower/toupper/totitle string ?first? ?last?`：仅转换
/// 指定索引区间（tclsh 8.6.17），区间外原样保留。
fn string_case_range(
    interp: &mut Interp,
    s: &str,
    args: &[Value],
    mode: CaseMode,
) -> Result<Value> {
    let name = match mode {
        CaseMode::Lower => "tolower",
        CaseMode::Upper => "toupper",
        CaseMode::Title => "totitle",
    };
    if args.len() > 5 {
        return Err(Error::wrong_args_with_usage(
            format!("string {}", name),
            1,
            args.len(),
            "string ?first? ?last?",
        ));
    }
    if args.len() == 3 {
        let out = match mode {
            CaseMode::Lower => s.to_lowercase(),
            CaseMode::Upper => s.to_uppercase(),
            CaseMode::Title => {
                let mut chars = s.chars();
                match chars.next() {
                    Some(first) => {
                        let mut out = String::new();
                        out.push(to_titlecase(first));
                        for c in chars {
                            if ('\u{1C90}'..='\u{1CBF}').contains(&c) {
                                out.push(c);
                            } else {
                                out.push(to_lower_simple(c));
                            }
                        }
                        out
                    }
                    None => String::new(),
                }
            }
        };
        return Ok(Value::from_str(&out));
    }
    let chars: Vec<char> = s.chars().collect();
    let first = match parse_tcl_index(args[3].as_str(), chars.len()) {
        Some(v) => v,
        None => return Err(bad_index(interp, args[3].as_str())),
    };
    let last_raw = match args.get(4) {
        Some(v) => match parse_tcl_index(v.as_str(), chars.len()) {
            Some(v) => v,
            None => return Err(bad_index(interp, v.as_str())),
        },
        None => first.max(0),
    };
    let len = chars.len() as i64;
    let start = first.max(0);
    let end = last_raw.min(len - 1);
    if len == 0 || start > end || start >= len {
        return Ok(Value::from_str(s));
    }
    let mut out = String::with_capacity(s.len());
    for (i, c) in chars.iter().enumerate() {
        let i = i as i64;
        if i < start || i > end {
            out.push(*c);
            continue;
        }
        match mode {
            CaseMode::Lower => out.extend(c.to_lowercase()),
            CaseMode::Upper => out.extend(c.to_uppercase()),
            CaseMode::Title => {
                if i == start {
                    out.push(to_titlecase(*c));
                } else if ('\u{1C90}'..='\u{1CBF}').contains(c) {
                    out.push(*c);
                } else {
                    out.push(to_lower_simple(*c));
                }
            }
        }
    }
    Ok(Value::from_str(&out))
}

fn to_titlecase(c: char) -> char {
    // The uppercase paired digraphs (ǄǇǊǱ, table mode 0x3) titlecase to the
    // following title form (+1), even though they are Lu and all other Lu
    // characters are returned unchanged.
    match c {
        'Ǆ' => return 'ǅ',
        'Ǉ' => return 'ǈ',
        'Ǌ' => return 'ǋ',
        'Ǳ' => return 'ǲ',
        _ => {}
    }
    if ('\u{10D0}'..='\u{10FF}').contains(&c) || !unicode::is_ll(c) {
        return c;
    }
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => match u {
            // Digraph letters: Tcl's titlecase is the following title form
            // (ǳ→ǲ, ǆ→ǅ, ǉ→ǈ, ǌ→ǋ), not the plain capital digraph.
            'Ǳ' => 'ǲ',
            'Ǆ' => 'ǅ',
            'Ǉ' => 'ǈ',
            'Ǌ' => 'ǋ',
            _ => u,
        },
        _ => c,
    }
}

/// Tcl's simple per-character lowercase (TclUCS4ToLower): the first char of
/// the (possibly full) mapping is the simple mapping in every current
/// Unicode case-expansion (e.g. U+0130 -> "i\u{307}", simple U+0069).
fn to_lower_simple(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Tcl boolean: 0/1 or an unambiguous prefix of true/false/yes/no/on/off
/// (case-insensitive).
fn tcl_boolean_value(s: &str) -> Option<bool> {
    if s == "0" {
        return Some(false);
    }
    if s == "1" {
        return Some(true);
    }
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    const WORDS: [&str; 6] = ["true", "false", "yes", "no", "on", "off"];
    let hits: Vec<&str> = WORDS
        .iter()
        .copied()
        .filter(|w| w.starts_with(lower.as_str()))
        .collect();
    match hits.as_slice() {
        ["true"] | ["yes"] | ["on"] => Some(true),
        ["false"] | ["no"] | ["off"] => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // -- subcommand resolution / arity / string is --

    #[test]
    fn test_string_subcmd_prefix_and_arity() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("string len abc").unwrap().as_str(), "3");
        assert_eq!(interp.eval("string byt abc").unwrap().as_str(), "3");
        assert_eq!(interp.eval("string cat").unwrap().as_str(), "");
        let msg = interp
            .eval(r#"catch {string to abc} m; set m"#)
            .unwrap();
        assert_eq!(
            msg.as_str(),
            "unknown or ambiguous subcommand \"to\": must be bytelength, cat, compare, equal, first, index, is, last, length, map, match, range, repeat, replace, reverse, tolower, totitle, toupper, trim, trimleft, trimright, wordend, or wordstart"
        );
        let msg = interp
            .eval(r#"catch {string trim a b c d} m; set m"#)
            .unwrap();
        assert_eq!(
            msg.as_str(),
            r#"wrong # args: should be "string trim string ?chars?""#
        );
    }

    #[test]
    fn test_string_is_prefix_strict_failindex() {
        let mut interp = Interp::new();
        // Class prefixes resolve; ambiguous/bad classes use tclsh wording.
        assert_eq!(interp.eval("string is ent 123").unwrap().as_str(), "1");
        assert_eq!(interp.eval("string is wor a_b").unwrap().as_str(), "1");
        let msg = interp.eval(r#"catch {string is d 5} m; set m"#).unwrap();
        assert!(msg.as_str().starts_with("ambiguous class \"d\":"));
        // Empty strings pass unless -strict (list stays valid).
        assert_eq!(interp.eval("string is integer {}").unwrap().as_str(), "1");
        assert_eq!(
            interp.eval("string is integer -strict {}").unwrap().as_str(),
            "0"
        );
        assert_eq!(
            interp.eval("string is list -strict {}").unwrap().as_str(),
            "1"
        );
        // -failindex is written on failure only, at the failing char.
        let r = interp
            .eval("unset -nocomplain ix; string is alpha -failindex ix ab1")
            .unwrap();
        assert_eq!(r.as_str(), "0");
        assert_eq!(interp.eval("set ix").unwrap().as_str(), "2");
        let r = interp
            .eval("unset -nocomplain ix; string is digit -failindex ix 123")
            .unwrap();
        assert_eq!(r.as_str(), "1");
        assert_eq!(interp.eval("info exists ix").unwrap().as_str(), "0");
    }

    // -- string equal -length --

    #[test]
    fn test_string_equal_length() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string equal -length 3 "abcdef" "abcxyz""#).unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_string_equal_length_nocase() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string equal -nocase -length 3 "ABCdef" "abcxyz""#).unwrap();
        assert_eq!(r.as_str(), "1");
    }

    #[test]
    fn test_string_equal_length_mismatch() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string equal -length 4 "abcdef" "abcxyz""#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    // -- string compare -length --

    #[test]
    fn test_string_compare_length() {
        let mut interp = Interp::new();
        // First 2 chars of "abc" and "abd" are both "ab" → equal
        let r = interp.eval(r#"string compare -length 2 "abc" "abd""#).unwrap();
        assert_eq!(r.as_str(), "0");
        // First 3 chars differ: "abc" < "abd"
        let r2 = interp.eval(r#"string compare -length 3 "abc" "abd""#).unwrap();
        assert_eq!(r2.as_str(), "-1");
    }

    #[test]
    fn test_string_compare_nocase_length() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string compare -nocase -length 5 "Hello World" "HELLO THERE""#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    // -- string byterange --

    #[test]
    fn test_string_byterange() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string byterange "hello" 0 2"#).unwrap();
        assert_eq!(r.as_str(), "hel");
    }

    #[test]
    fn test_string_byterange_end() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string byterange "hello" 2 end"#).unwrap();
        assert_eq!(r.as_str(), "llo");
    }

    // -- string wordstart / wordend --

    #[test]
    fn test_string_wordstart() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordstart "hello world foo" 6"#).unwrap();
        assert_eq!(r.as_str(), "6");
    }

    #[test]
    fn test_string_wordstart_at_word_begin() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordstart "hello world" 0"#).unwrap();
        assert_eq!(r.as_str(), "0");
    }

    #[test]
    fn test_string_wordend() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordend "hello world foo" 6"#).unwrap();
        assert_eq!(r.as_str(), "11");
    }

    #[test]
    fn test_string_wordend_at_end() {
        let mut interp = Interp::new();
        let r = interp.eval(r#"string wordend "hello" 0"#).unwrap();
        assert_eq!(r.as_str(), "5");
    }

    #[test]
    fn test_string_case_index_range() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("string toupper abcdef 2").unwrap().as_str(), "abCdef");
        assert_eq!(
            interp.eval("string toupper abcdef 2 end+5").unwrap().as_str(),
            "abCDEF"
        );
        assert_eq!(interp.eval("string toupper abcdef -1").unwrap().as_str(), "Abcdef");
        assert_eq!(interp.eval("string totitle abcdef 2 4").unwrap().as_str(), "abCdef");
    }
}
