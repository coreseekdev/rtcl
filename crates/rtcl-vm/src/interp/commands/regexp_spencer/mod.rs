//! Henry Spencer 正则引擎的 Rust 移植（B 路径）。
//!
//! 项目计划与验收口径见 `specs/regex-spencer-port.md`；本模块当前处于
//! M1/M2（ARE 解析 + NFA 整体匹配），捕获 dissect/lookahead/回引在
//! M3/M4 落地。feature `regexp-spencer` 默认关闭，验证完成后转默认。

pub mod ast;
mod backtrack;
pub mod captures;
pub mod charset;
mod dfa;
pub mod matcher;
pub mod nfa;
pub mod parser;

/// 编译错误：`code` 为 tclsh 的 `REG_*` 名，`message` 为
/// "couldn't compile regular expression pattern: " 之后的文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: &str) -> Error {
        Error {
            code,
            message: message.to_string(),
        }
    }
}

pub use ast::{AssertKind, Flags, Node, DUPMAX, DUPINF};
pub use charset::{CharSet, Item, Pred};
pub use captures::Captures;
pub use matcher::Match;
pub use nfa::Prog;
pub use parser::Options;

/// 编译为 NFA（M2：不含回引/lookahead）。
pub fn compile(pattern: &str, opts: Options) -> Result<Prog, Error> {
    let (tree, effective) = parser::parse_with_options(pattern, opts)?;
    nfa::compile(&tree, effective.icase)
}

/// 运行时选择。Spencer 移植已是默认引擎；`RTCL_REGEX_ENGINE=fast`
///（或 `0`/`off`）可退回旧的 fast/BT 路径（差分/应急用），
/// `spencer`/`1` 显式选择新引擎。
pub fn engine_selected() -> bool {
    !matches!(
        std::env::var("RTCL_REGEX_ENGINE").as_deref(),
        Ok("fast") | Ok("0") | Ok("off")
    )
}

/// 解析为 `subre` 树（解析器单测/调试入口）。
pub fn parse(pattern: &str, opts: Options) -> Result<Node, Error> {
    parser::parse(pattern, opts)
}

/// 在 `text` 上从 `from` 起做 leftmost-longest（或 SHORTER）整体匹配。
pub fn find(prog: &Prog, text: &str, from: usize) -> Option<Match> {
    let chars: Vec<char> = text.chars().collect();
    matcher::find(prog, &chars, from)
}

pub fn is_match(prog: &Prog, text: &str, from: usize) -> bool {
    find(prog, text, from).is_some()
}

/// 匹配并返回捕获 span（0=整体；未参与组为 None）。
pub fn captures(prog: &Prog, text: &str, from: usize) -> Option<Captures> {
    let chars: Vec<char> = text.chars().collect();
    captures::exec(prog, &chars, from, from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, text: &str) -> Option<(usize, usize)> {
        let prog = compile(pat, Options::default()).unwrap();
        find(&prog, text, 0).map(|m| (m.start, m.end))
    }

    fn shorter(pat: &str) -> bool {
        compile(pat, Options::default()).unwrap().flags.shorter
    }

    #[test]
    fn longest_by_default() {
        // Spencer/POSIX leftmost-longest：a|ab 在 ab 上取 ab。
        assert_eq!(m("a|ab", "ab"), Some((0, 2)));
        assert_eq!(m("ab|a", "ab"), Some((0, 2)));
        assert_eq!(m("a+", "aaa"), Some((0, 3)));
        assert_eq!(m("a?", "aaa"), Some((0, 1)));
        assert_eq!(m("a{2,3}", "aaaa"), Some((0, 3)));
        assert_eq!(m("(a|ab)+", "abab"), Some((0, 4)));
        assert_eq!(m("(a|ab)(c|bcd)", "abcd"), Some((0, 4)));
    }

    #[test]
    fn shorter_when_lazy_present() {
        // 任一非贪婪量词 → REG_USHORTEST（还要看 COMBINE 的偏好序）。
        assert!(shorter("(a+?)(a*)"));
        assert!(!shorter("(a+)(a*?)"));
        assert!(shorter("a+?"));
        assert!(shorter("(a*?)(a*)"));
        assert_eq!(m("a+?", "aaa"), Some((0, 1)));
        assert_eq!(m("(a+?)(a*)", "aaa"), Some((0, 1)));
        assert_eq!(m("(a*?)(a*)", "aaa"), Some((0, 0)));
        assert_eq!(m("(a+)(a*?)", "aaa"), Some((0, 3)));
        assert_eq!(m("(a+?)(a+)", "aaa"), Some((0, 2)));
        assert_eq!(m("(a+?)(a*)b", "aaab"), Some((0, 4)));
    }

    #[test]
    fn anchors_and_asserts() {
        assert_eq!(m("^a+", "baa"), None);
        assert_eq!(m("^a+", "aaa"), Some((0, 3)));
        assert_eq!(m("a+$", "baa"), Some((1, 3)));
        assert_eq!(m(r"\Aa+", "aaa"), Some((0, 3)));
        assert_eq!(m(r"\A\s*[^b]*b", "ab"), Some((0, 2)));
        assert_eq!(m(r"a+\Z", "baa"), Some((1, 3)));
        assert_eq!(m(r"\y\w+", " baa"), Some((1, 4)));
    }

    #[test]
    fn classes_and_escapes() {
        assert_eq!(m("[a-c]+", "xabcx"), Some((1, 4)));
        assert_eq!(m("[^a]+", "aaab"), Some((3, 4)));
        assert_eq!(m("[[:alpha:]]+", "12ab3"), Some((2, 4)));
        assert_eq!(m(r"\d+", "ab123"), Some((2, 5)));
        assert_eq!(m(r"\w+", " a_1 "), Some((1, 4)));
        assert_eq!(m(r"\s+", "ab \t"), Some((2, 4)));
        assert_eq!(m(r"\u0041", "A"), Some((0, 1)));
        assert_eq!(m(r"\x41", "A"), Some((0, 1)));
        assert_eq!(m(r"\3161573148", "\u{ce}1573148"), Some((0, 8)));
        assert_eq!(m(r"[\101]", "A"), Some((0, 1)));
    }

    #[test]
    fn options() {
        let ic = Options {
            icase: true,
            ..Options::default()
        };
        let prog = compile("abc", ic).unwrap();
        assert!(is_match(&prog, "xABCy", 0));
        // newline-sensitive dot 不匹配 \n。
        let nl = Options {
            newline: true,
            ..Options::default()
        };
        let prog = compile("a.b", nl).unwrap();
        assert!(!is_match(&prog, "a\nb", 0));
        let prog = compile("a.b", Options::default()).unwrap();
        assert!(is_match(&prog, "a\nb", 0));
    }

    #[test]
    fn parse_errors() {
        let e = |p: &str| compile(p, Options::default()).unwrap_err();
        assert_eq!(e("*").code, "REG_BADRPT");
        assert_eq!(e("a**").code, "REG_BADRPT");
        assert_eq!(e("(").code, "REG_EPAREN");
        assert_eq!(e("[a").code, "REG_EBRACK");
        assert_eq!(e(r"\q").code, "REG_EESCAPE");
        assert_eq!(e("a{2,1}").code, "REG_BADBR");
        assert_eq!(e("a{256}").code, "REG_BADBR");
        assert_eq!(e("(?z)").code, "REG_BADOPT");
        assert_eq!(e(r"\1").code, "REG_ESUBREG");
        assert_eq!(e(r"[\1]").code, "REG_EESCAPE");
        assert_eq!(e("(?<=a)b").code, "REG_BADRPT");
        assert_eq!(e("***?").code, "REG_BADPAT");
        assert_eq!(e("***x").code, "REG_BADRPT");
    }

    #[test]
    fn parse_ok_shapes() {
        for p in ["", "a", "a|", "|a", "a||b", "()", "(?:a)", "(?=a)", "a{2,}",
                  "a{,2}", "***=", "***:a", "(?i)a", "(?x) a # c\n b", "\\Q"]
        {
            let _ = parse(p, Options::default());
        }
    }

    fn m_mode(pat: &str, text: &str, opts: Options) -> Option<(usize, usize)> {
        let prog = compile(pat, opts).unwrap();
        find(&prog, text, 0).map(|m| (m.start, m.end))
    }

    fn bre() -> Options {
        Options { bre: true, ..Options::default() }
    }

    fn ere() -> Options {
        Options { ere: true, ..Options::default() }
    }

    #[test]
    fn bre_dialect() {
        // | + ? ( ) { } 普通；\( \) \( \{ \) 是运算符。
        assert_eq!(m_mode("a|b", "a|b", bre()), Some((0, 3)));
        assert_eq!(m_mode("a|b", "a", bre()), None);
        assert_eq!(m_mode(r"a\+", "aaa", bre()), None);
        assert_eq!(m_mode(r"a\{2,3\}", "aaaa", bre()), Some((0, 3)));
        assert_eq!(m_mode(r"a\{,2\}", "aa", bre()), Some((0, 2)));
        assert_eq!(m_mode(r"\(a\)\1", "aa", bre()), Some((0, 2)));
        assert_eq!(m_mode("a^b", "a^b", bre()), Some((0, 3)));
        assert_eq!(m_mode("$a", "$a", bre()), Some((0, 2)));
        assert_eq!(m_mode(r"\(^a\)", "a", bre()), Some((0, 1)));
        assert_eq!(m_mode("*a*", "*aa", bre()), Some((0, 3)));
        assert_eq!(m_mode(r"\d", "d", bre()), Some((0, 1)));
        assert_eq!(m_mode(r"\[", "[", bre()), Some((0, 1)));
        assert_eq!(m_mode(r"[\n]", "\\", bre()), Some((0, 1)));
        assert_eq!(
            compile(r"a\{2", bre()).unwrap_err().code,
            "REG_EBRACE"
        );
        assert_eq!(
            compile(r"a\{2,1\}", bre()).unwrap_err().code,
            "REG_BADBR"
        );
        assert_eq!(compile(r"\10", bre()).unwrap_err().code, "REG_ESUBREG");
    }

    #[test]
    fn ere_dialect() {
        // ERE 无 Perl 类/回引/懒惰量词/内嵌选项；\x 一律字面 x。
        assert_eq!(m_mode(r"\d", "d", ere()), Some((0, 1)));
        assert_eq!(m_mode(r"(a)\1", "a1", ere()), Some((0, 2)));
        assert_eq!(m_mode(r"\{", "{", ere()), Some((0, 1)));
        assert_eq!(m_mode(r"[\n]", "\\", ere()), Some((0, 1)));
        assert_eq!(m_mode(r"(?e)a|b", "a", Options::default()), Some((0, 1)));
        assert!(compile(r"a+?", ere()).is_err());
        assert_eq!(
            compile("(?e)(?i)a", Options::default()).unwrap_err().code,
            "REG_BADRPT"
        );
        assert_eq!(
            compile("(?e)(?:a)", Options::default()).unwrap_err().code,
            "REG_BADRPT"
        );
        assert_eq!(
            compile("(?e)(?=a)", Options::default()).unwrap_err().code,
            "REG_BADRPT"
        );
        // BRE 里 (?...) 全是普通字符（tclsh 实测能编译、不匹配）。
        assert_eq!(m_mode("(?b)(?i)a", "a", Options::default()), None);
    }

    fn find_ss(pat: &str, text: &str, from: usize) -> Option<(usize, usize)> {
        let prog = compile(pat, Options::default()).unwrap();
        let chars: Vec<char> = text.chars().collect();
        matcher::find_from(&prog, &chars, from, from).map(|m| (m.start, m.end))
    }

    #[test]
    fn start_slice_assertion_context() {
        // -start 起点是虚拟串首：词断言看不到前一个字符。
        assert_eq!(find_ss(r"\y", "ab", 1), Some((1, 1)));
        assert_eq!(find_ss(r"\m", "ab", 1), Some((1, 1)));
        assert_eq!(find_ss(r"\M", "ab", 1), Some((2, 2)));
        assert_eq!(find_ss(r"\Y", "ab", 1), None);
        // `^` 非 newline 模式通常锚绝对串首，但在切片起点且前一个
        // 字符为换行时 tclsh 允许命中。
        assert_eq!(find_ss("^", "\na", 1), Some((1, 1)));
        assert_eq!(find_ss("^", "ba", 1), None);
        assert_eq!(find_ss(r"\A", "ba", 1), Some((1, 1)));
        // -start 越过串尾的虚拟空串：行首约束不得借前一个换行命中。
        let prog = compile(
            "^",
            Options {
                nlanch: true,
                ..Options::default()
            },
        )
        .unwrap();
        let chars: Vec<char> = "\n".chars().collect();
        assert_eq!(matcher::find_from(&prog, &chars, 1, 2), None);
    }

    #[test]
    fn backref_keeps_distinct_capture_paths() {
        // a? 的两条路径到达同一终点但 (a*) 捕获不同；去重不能只按
        // 终点，否则回引路径被错误丢弃（leftmost 0..3）。
        let prog = compile(r"a?(a*)[^a](a)\1", Options::default()).unwrap();
        let chars: Vec<char> = "abab".chars().collect();
        let m = matcher::find_from(&prog, &chars, 0, 0).unwrap();
        assert_eq!((m.start, m.end), (0, 3));
    }

    #[test]
    fn negated_shorthand_newline_stop() {
        // -linestop/-line 下 \W/\D 等否定简写不匹配换行（同 [^...]）。
        let nl = Options {
            nlstop: true,
            ..Options::default()
        };
        let prog = compile(r"\W", nl).unwrap();
        assert!(!is_match(&prog, "\n", 0));
        let prog = compile(r"\W", Options::default()).unwrap();
        assert!(is_match(&prog, "\n", 0));
        // ARE 字符类内只允许小写 \d\s\w。
        assert_eq!(compile(r"[\W]", Options::default()).unwrap_err().code, "REG_EESCAPE");
        assert_eq!(compile(r"[\D]", Options::default()).unwrap_err().code, "REG_EESCAPE");
        assert!(compile(r"[\W]", bre()).is_ok());
    }

    #[test]
    fn embedded_options_affect_matcher() {
        // `(?i)` 是 pattern 内嵌选项：必须传给 NFA/匹配器，不能只看
        // 调用方传入的 Options::icase。
        let prog = compile("(?i)A{0,2}", Options::default()).unwrap();
        assert!(is_match(&prog, "aaa", 0));
        let m = find(&prog, "aaa", 0).unwrap();
        assert_eq!((m.start, m.end), (0, 2));
        // 非 ASCII 大小写也使用生效后的 icase。
        let prog = compile("(?i)é", Options::default()).unwrap();
        assert!(is_match(&prog, "É", 0));
    }

    #[test]
    fn lacon_group_is_quantifiable() {
        // 环视内括号非捕获但保留分组层，`(^){1,2}` 可被量化
        //（tclsh 允许；曾误报 quantifier operand invalid）。
        for p in [r"(?=(^){1,2})", r"(?!(^){0,2}(?:[[:space:]]{2}|\n))"] {
            let prog = compile(p, Options::default()).unwrap();
            assert!(is_match(&prog, "", 0));
        }
    }

    #[test]
    fn required_char_prefilter_is_sound() {
        let prog = compile("[a-z]*z", Options::default()).unwrap();
        assert_eq!(prog.required_char, Some('z'));
        assert!(!is_match(&prog, "abc", 0));
        assert!(is_match(&prog, "abz", 0));
        // 可选/分支不同则不产生必需字符。
        assert_eq!(compile("z?", Options::default()).unwrap().required_char, None);
        assert_eq!(compile("(a|b)", Options::default()).unwrap().required_char, None);
        assert_eq!(compile("(ab|ac)", Options::default()).unwrap().required_char, Some('a'));
        // -nocase 下必需字符需按折叠变体匹配。
        let ic = Options {
            icase: true,
            ..Options::default()
        };
        let prog = compile("z", ic).unwrap();
        assert!(is_match(&prog, "Z", 0));
    }

    #[test]
    fn collating_element_must_be_single_char() {
        assert_eq!(
            compile(r"[[=ab=]]", Options::default()).unwrap_err().code,
            "REG_ECOLLATE"
        );
        assert_eq!(
            compile(r"[[.ab.]]", Options::default()).unwrap_err().code,
            "REG_ECOLLATE"
        );
        assert!(compile(r"[[=a=]]", Options::default()).is_ok());
        assert!(compile(r"[[=é=]]", Options::default()).is_ok());
        assert!(compile(r"[[.].]]", Options::default()).is_ok());
    }

    #[test]
    fn octal_expansion_quantifier_binds_last_char() {
        // `\101` = 'A'，多出的 '0' 是独立字面；`{2}` 只作用于 '0'。
        let prog = compile(r"\1010{2}", Options::default()).unwrap();
        assert!(is_match(&prog, "A00", 0));
        assert!(!is_match(&prog, "A0A0", 0));
        // `\1010{0,2}?^` = A 0{0,2}? ^：A 必消耗，^ 无法在起点命中。
        let prog = compile(r"\1010{0,2}?^", Options::default()).unwrap();
        assert!(!is_match(&prog, "A1_ ", 0));
    }

    #[test]
    fn dfa_start_tracking_keeps_current_starts() {
        // DFA 转移写入下一状态起点时不能复用当前 start_of：
        // `(..)+` @ abc 应得整体 0..2（曾因状态起点被覆盖得到 0..3）。
        let prog = compile(r"(..)+", Options::default()).unwrap();
        let chars: Vec<char> = "abc".chars().collect();
        let m = matcher::find_from(&prog, &chars, 0, 0).unwrap();
        assert_eq!((m.start, m.end), (0, 2));
        let caps = captures::exec(&prog, &chars, 0, 0).unwrap();
        assert_eq!(caps.spans[0], Some((0, 2)));
        assert_eq!(caps.spans[1], Some((0, 2)));
    }

    #[test]
    fn lookahead_does_not_leak_preference_flags() {
        // 环视内部子式的 LONGER 不得覆盖外层的 SHORTER：
        // `(?![^ab]{2}|$)[^ab]*?` 在位置 3 应取空匹配。
        let prog = compile(r"(?![^ab]{2}|$)[^ab]*?", Options::default()).unwrap();
        assert!(prog.flags.shorter);
        let chars: Vec<char> = [0xF0u32, 0x9F, 0x98, 0x80, 0x61]
            .iter()
            .map(|c| char::from_u32(*c).unwrap())
            .collect();
        let m = matcher::find_from(&prog, &chars, 3, 3).unwrap();
        assert_eq!((m.start, m.end), (3, 3));
    }

    #[test]
    fn lazy_child_repeat_capture_prefers_short_first() {
        // `(a{1,2}?){0,2}` @ "aaa"：tclsh 末次迭代取最长剩余段
        //（组 1..3），而不是前一次贪到最长、末次剩 1 字符。
        let prog = compile(r"(a{1,2}?){0,2}", Options::default()).unwrap();
        let chars: Vec<char> = "aaa".chars().collect();
        let caps = captures::exec(&prog, &chars, 0, 0).unwrap();
        assert_eq!(caps.spans[0], Some((0, 3)));
        assert_eq!(caps.spans[1], Some((1, 3)));
    }
}
