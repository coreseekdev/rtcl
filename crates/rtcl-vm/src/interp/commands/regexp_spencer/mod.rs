//! Henry Spencer 正则引擎的 Rust 移植（B 路径）。
//!
//! 项目计划与验收口径见 `specs/regex-spencer-port.md`；本模块当前处于
//! M1/M2（ARE 解析 + NFA 整体匹配），捕获 dissect/lookahead/回引在
//! M3/M4 落地。feature `regexp-spencer` 默认关闭，验证完成后转默认。

pub mod ast;
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
    let tree = parser::parse(pattern, opts)?;
    nfa::compile(&tree, opts.icase)
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
}
