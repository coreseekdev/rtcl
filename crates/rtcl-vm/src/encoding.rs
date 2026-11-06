//! 编码转换（D-29 宽表）：tclsh 8.6 的 86 个编码名。
//!
//! Tcl 语义（2026-11-06 实测钉形）：
//! * 名字**区分大小写**、只收表内名（`UTF-8` 报 unknown encoding "UTF-8"）；
//! * `utf-8` 解码是 Tcl 的 modified-UTF-8：无效字节/截断尾按**字节值**
//!   出字符（`\xff` → U+00FF），编码为正常 UTF-8；
//! * `ascii`/`iso8859-1` 解码逐字节直映；`ascii` 编码非 ASCII → `?`，
//!   `iso8859-1` 编码 >U+00FF → `?`；
//! * `identity` 解码逐字节直映、编码 = 内部 UTF-8 字节（`é` → c3 a9）；
//! * `binary` 仅通道名（不在 names 表）：解码逐字节、编码取**低字节**；
//! * `unicode` = UCS-2LE（>U+FFFF → U+FFFD；奇尾字节 → U+FFFD）；
//! * 其余（iconv 覆盖的多数编码）：解码经 libc iconv；无效字节按字节值
//!   出字符继续；编码失败字符 → `?` 继续。
//!
//! iconv 后端是**可选的原生能力**（feature `iconv`，默认开；见
//! Cargo.toml）：仅在 unix + feature 同时成立时编译。wasm/无 libc 目标
//! 或关闭 feature 时，非内建编码退化为字节直映（decode）/UTF-8 直传
//! （encode），名字表与命令面保持完整。

/// tclsh 8.6 `encoding names` 的精确顺序（86 项）。
pub const NAMES: [&str; 86] = [
    "cp860", "cp861", "cp862", "cp863", "tis-620", "cp864", "cp865", "cp866", "gb12345",
    "gb2312-raw", "cp949", "cp950", "cp869", "dingbats", "ksc5601", "macCentEuro", "cp874",
    "cp165", "macUkraine", "jis0201", "gb2312", "euc-cn", "euc-jp", "macThai", "iso8859-10",
    "iso2022-jp", "jis0208", "iso8859-11", "iso2022", "macIceland", "iso8859-13", "jis0212",
    "iso8859-14", "iso8859-15", "cp737", "iso8859-16", "euc-kr", "big5", "macRomania",
    "iso2022-kr", "gb1988", "macTurkish", "macGreek", "ascii", "cp437", "macRoman",
    "iso8859-1", "iso8859-2", "iso8859-3", "koi8-r", "macCroatian", "ebcdic", "iso8859-4",
    "iso8859-5", "cp1250", "koi8-t", "cp1251", "iso8859-6", "macCyrillic", "koi8-u", "cp1252",
    "iso8859-7", "macDingbats", "cp1253", "iso8859-8", "iso8859-9", "koi8-ru", "cp1254",
    "cp850", "cp1255", "cp932", "cp1256", "identity", "cp1257", "cp852", "cns11643",
    "macJapan", "cp1258", "utf-8", "shiftjis", "cp936", "cp855", "cp775", "symbol", "cp857",
    "unicode",
];

/// `encoding` 命令可用的名字（精确、区分大小写）。
pub fn known(name: &str) -> bool {
    NAMES.contains(&name)
}

/// 通道 `-encoding` 可用的名字（额外接受 `binary`）。
pub fn known_for_channel(name: &str) -> bool {
    name == "binary" || known(name)
}

/// 是否编入 libc iconv 后端（native 默认；wasm / 无 libc / 关闭 feature
/// 的构建为 false——非内建编码走字节直映兜底）。
pub const ICONV_BACKEND: bool = cfg!(all(unix, feature = "iconv"));

/// 字节串 → 字符（Tcl 的字节值直映约定：每个输入字节出 0..255 的字符）。
pub fn decode(bytes: &[u8], enc: &str) -> String {
    match enc {
        "utf-8" => decode_tcl_utf8(bytes),
        "ascii" | "iso8859-1" | "identity" | "binary" => {
            bytes.iter().map(|&b| b as char).collect()
        }
        "unicode" => decode_ucs2le(bytes),
        _ => {
            #[cfg(all(unix, feature = "iconv"))]
            {
                iconv_decode(bytes, enc)
            }
            #[cfg(not(all(unix, feature = "iconv")))]
            {
                bytes.iter().map(|&b| b as char).collect()
            }
        }
    }
}

/// 字符 → 字节（输出按"每字节一个字符"的 Tcl 字符串约定返回）。
pub fn encode(text: &str, enc: &str) -> Vec<u8> {
    match enc {
        "utf-8" => text.as_bytes().to_vec(),
        "ascii" => text
            .chars()
            .map(|c| if (c as u32) < 128 { c as u8 } else { b'?' })
            .collect(),
        "iso8859-1" => text
            .chars()
            .map(|c| {
                let u = c as u32;
                if u < 256 { u as u8 } else { b'?' }
            })
            .collect(),
        "identity" => text.as_bytes().to_vec(),
        "binary" => text.chars().map(|c| (c as u32 & 0xFF) as u8).collect(),
        "unicode" => {
            let mut out = Vec::with_capacity(text.len() * 2);
            for c in text.chars() {
                let u = c as u32;
                if u <= 0xFFFF {
                    out.push((u & 0xFF) as u8);
                    out.push((u >> 8) as u8);
                } else {
                    out.push(0xFD);
                    out.push(0xFF); // U+FFFD LE
                }
            }
            out
        }
        _ => {
            #[cfg(all(unix, feature = "iconv"))]
            {
                iconv_encode(text, enc)
            }
            #[cfg(not(all(unix, feature = "iconv")))]
            {
                text.as_bytes().to_vec()
            }
        }
    }
}

/// Tcl 的 modified-UTF-8 解码：无效/截断字节逐个按字节值出字符。
fn decode_tcl_utf8(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match std::str::from_utf8(&bytes[i..]) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    // SAFETY: from_utf8 已确认前缀合法。
                    out.push_str(unsafe { std::str::from_utf8_unchecked(&bytes[i..i + valid]) });
                    i += valid;
                }
                if e.error_len().is_some() {
                    out.push(bytes[i] as char);
                    i += 1;
                } else {
                    // 截断尾：剩余字节逐个按字节值出字符。
                    for &b in &bytes[i..] {
                        out.push(b as char);
                    }
                    break;
                }
            }
        }
    }
    out
}

fn decode_ucs2le(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let u = (bytes[i] as u32) | ((bytes[i + 1] as u32) << 8);
        out.push(char::from_u32(u).unwrap_or('\u{FFFD}'));
        i += 2;
    }
    if i < bytes.len() {
        out.push('\u{FFFD}'); // 奇尾字节
    }
    out
}

/// 单字节编码的"逐字节出字符"集合（通道解码走直映路径）。
pub fn is_single_byte(enc: &str) -> bool {
    matches!(enc, "ascii" | "iso8859-1" | "identity" | "binary")
}

// ── iconv 后端（libc）────────────────────────────────────────────────

/// Tcl 名 → iconv 名（其余同名直接交给 iconv 的别名表）。
#[cfg(all(unix, feature = "iconv"))]
fn iconv_name(enc: &str) -> &str {
    match enc {
        "shiftjis" => "SHIFT_JIS",
        "iso2022-jp" => "ISO-2022-JP",
        "iso2022-kr" => "ISO-2022-KR",
        "iso2022" => "ISO-2022-JP",
        "unicode" => "UCS-2LE",
        "macRoman" => "MACINTOSH",
        "macCentEuro" => "MAC-CENTRALEUROPE",
        "macCyrillic" => "MAC-CYRILLIC",
        "euc-cn" => "EUC-CN",
        "gb2312-raw" => "GB2312",
        "ebcdic" => "EBCDIC-US",
        "tis-620" => "TIS-620",
        "identity" => "ISO-8859-1",
        other => other,
    }
}

#[cfg(all(unix, feature = "iconv"))]
mod iconv_ffi {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int, c_void};

    extern "C" {
        fn iconv_open(tocode: *const c_char, fromcode: *const c_char) -> *mut c_void;
        fn iconv(
            cd: *mut c_void,
            inbuf: *mut *mut c_char,
            inbytesleft: *mut usize,
            outbuf: *mut *mut c_char,
            outbytesleft: *mut usize,
        ) -> usize;
        fn iconv_close(cd: *mut c_void) -> c_int;
    }

    /// 一次转换。返回 (输出字节, 已消费输入字节, 是否在末尾遇到不完整序列)。
    /// EILSEQ（非法序列）处停止，由调用方以"字节值字符"继续。
    pub fn convert(input: &[u8], from: &str, to: &str) -> Option<(Vec<u8>, usize, bool)> {
        let from_c = CString::new(from).ok()?;
        let to_c = CString::new(to).ok()?;
        unsafe {
            let cd = iconv_open(to_c.as_ptr(), from_c.as_ptr());
            if cd as isize == -1 {
                return None;
            }
            let mut out: Vec<u8> = Vec::with_capacity(input.len() * 2 + 16);
            let mut in_off = 0usize;
            let mut incomplete = false;
            loop {
                let mut in_ptr = input[in_off..].as_ptr() as *mut c_char;
                let mut in_left = input.len() - in_off;
                if out.capacity() - out.len() < 8 {
                    out.reserve(out.capacity().max(16));
                }
                let spare = out.capacity() - out.len();
                let mut out_ptr = out.as_mut_ptr().add(out.len()) as *mut c_char;
                let mut out_left = spare;
                let r = iconv(cd, &mut in_ptr, &mut in_left, &mut out_ptr, &mut out_left);
                let produced = spare - out_left;
                out.set_len(out.len() + produced);
                in_off = input.len() - in_left;
                if r != usize::MAX {
                    break;
                }
                let err = std::io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(22) => {
                        // EINVAL：末尾不完整序列。
                        incomplete = true;
                        break;
                    }
                    Some(7) => {
                        // E2BIG：扩容后继续。
                        continue;
                    }
                    Some(84) => break, // EILSEQ：交给调用方跳过
                    _ => break,
                }
            }
            iconv_close(cd);
            Some((out, in_off, incomplete))
        }
    }
}

/// 增量解码一步（通道用）：返回 (本次解出的文本, 消费的输入字节数)。
/// `None` = 输入还是不完整序列，需要继续读。
#[cfg(all(unix, feature = "iconv"))]
pub fn iconv_decode_step(bytes: &[u8], enc: &str) -> Option<(String, usize)> {
    if bytes.is_empty() {
        return None;
    }
    let from = iconv_name(enc);
    match iconv_ffi::convert(bytes, from, "UTF-8") {
        None => Some(((bytes[0] as char).to_string(), 1)),
        Some((produced, consumed, incomplete)) => {
            if consumed == 0 && produced.is_empty() && incomplete {
                return None;
            }
            if consumed == 0 {
                // 非法首字节：按字节值出字符并消费一个字节。
                return Some(((bytes[0] as char).to_string(), 1));
            }
            Some((String::from_utf8_lossy(&produced).into_owned(), consumed))
        }
    }
}

/// 解码：iconv 到 UTF-8；非法字节按字节值出字符继续；末尾不完整序列
/// 的剩余字节同样按字节值出字符（Tcl 的字节直映约定）。
#[cfg(all(unix, feature = "iconv"))]
fn iconv_decode(bytes: &[u8], enc: &str) -> String {
    let from = iconv_name(enc);
    let mut out = String::new();
    let mut off = 0usize;
    while off < bytes.len() {
        match iconv_ffi::convert(&bytes[off..], from, "UTF-8") {
            None => {
                // iconv 不认识：按字节直映兜底（诊断面；见台账残差）。
                return bytes.iter().map(|&b| b as char).collect();
            }
            Some((produced, consumed, incomplete)) => {
                out.push_str(&String::from_utf8_lossy(&produced));
                off += consumed;
                if incomplete || (consumed == 0 && produced.is_empty()) {
                    // 剩余（不完整/非法）字节按字节值出字符。
                    out.push(bytes[off] as char);
                    off += 1;
                }
            }
        }
    }
    out
}

/// 编码：逐字符转换；不可编码字符 → `?` 继续（状态保持在同一 iconv 句柄
/// 由整串调用承担——失败时退回逐字符）。
#[cfg(all(unix, feature = "iconv"))]
fn iconv_encode(text: &str, enc: &str) -> Vec<u8> {
    let to = iconv_name(enc);
    match iconv_ffi::convert(text.as_bytes(), "UTF-8", to) {
        Some((produced, consumed, incomplete)) if consumed == text.len() && !incomplete => {
            // ISO-2022-JP：tclsh 在结尾补 ASCII 复位序列 ESC ( B（iconv
            // 的一次转换不收尾），否则后续字节会按 JIS 状态解释。
            let mut out = produced;
            if matches!(enc, "iso2022-jp" | "iso2022") && !out.is_empty() {
                out.extend_from_slice(b"\x1b(B");
            }
            out
        }
        _ => {
            // 慢路径：逐字符（ISO-2022 等有状态编码会在每字符重置状态，
            // 结果仍可解码，字节序列与 tclsh 可能不同——台账残差）。
            let mut out = Vec::new();
            for c in text.chars() {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                match iconv_ffi::convert(s.as_bytes(), "UTF-8", to) {
                    Some((produced, consumed, incomplete))
                        if consumed == s.len() && !incomplete =>
                    {
                        out.extend_from_slice(&produced)
                    }
                    _ => out.push(b'?'),
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_the_tcl_8_6_table() {
        assert_eq!(NAMES.len(), 86);
        assert!(known("utf-8"));
        assert!(!known("UTF-8")); // Tcl 名大小写敏感
        assert!(known_for_channel("binary"));
        assert!(!known("binary"));
    }

    #[cfg(all(unix, feature = "iconv"))]
    #[test]
    fn iconv_backend_converts_wide_encodings() {
        assert!(ICONV_BACKEND);
        assert_eq!(decode(b"\x82\xa0", "shiftjis"), "あ");
        assert_eq!(encode("あ", "shiftjis"), b"\x82\xa0");
    }

    #[cfg(not(all(unix, feature = "iconv")))]
    #[test]
    fn missing_iconv_backend_falls_back_to_byte_pass_through() {
        assert!(!ICONV_BACKEND);
        assert_eq!(decode(b"\x82\xa0", "shiftjis"), "\u{82}\u{a0}");
        assert_eq!(encode("あ", "shiftjis"), "あ".as_bytes());
    }
}
