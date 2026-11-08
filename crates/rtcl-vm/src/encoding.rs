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

// ── Tcl .enc 表后端（S/D/M/E）───────────────────────────────────────
//
// tclsh 的宽表编码全部来自 `$TCL_LIBRARY/encoding/*.enc`（或安装目录
// 的 encoding/）。这里直接解析同一批文件，保证与 oracle 同一张表：
//   S  单字节；D  双字节（所有字节都是首字节）；M  多字节（页内字节
//   为首字节，其余走 page 0）；E  转义序列驱动（iso2022-*），子表
//   复用 S/D/M 表。
// 文件格式：注释行 → 类型行 → S/D/M: `fallback symbol numPages` → 每页
// `hi` + 16×64 hex → 可选 `R` 反查段；E: key/value 行（init/final/
// 子表名+转义字节）。仅 std 构建可用；wasm/无文件系统回退 iconv。
#[cfg(feature = "std")]
mod table {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock, RwLock};

    pub struct TableEncoding {
        fallback: u16,
        prefix: [bool; 256],
        /// page[hi]：首字节 hi 的 256 项解码页；None = 该字节走 page 0。
        to: Vec<Option<Box<[u16; 256]>>>,
        /// Unicode → 编码 word（hi<<8|lo；hi=0 即单字节）。
        from: HashMap<u16, u16>,
    }

    impl TableEncoding {
        /// 解码整串（convertfrom 语义：非法/截断按字节值出字符）。
        pub fn decode(&self, bytes: &[u8]) -> String {
            let mut out = String::with_capacity(bytes.len());
            let mut i = 0usize;
            while i < bytes.len() {
                let b = bytes[i] as usize;
                if self.prefix[b] {
                    if i + 1 >= bytes.len() {
                        out.push(b as u8 as char);
                        i += 1;
                        continue;
                    }
                    let ch = self
                        .to[b]
                        .as_ref()
                        .map(|p| p[bytes[i + 1] as usize])
                        .unwrap_or(0);
                    if ch == 0 && b != 0 {
                        out.push(b as u8 as char);
                        i += 1;
                    } else {
                        out.push(char::from_u32(ch as u32).unwrap_or('\u{FFFD}'));
                        i += 2;
                    }
                } else {
                    let ch = self.page0(b);
                    if ch == 0 && b != 0 {
                        out.push(b as u8 as char);
                    } else {
                        out.push(char::from_u32(ch as u32).unwrap_or('\u{FFFD}'));
                    }
                    i += 1;
                }
            }
            out
        }

        fn page0(&self, b: usize) -> u16 {
            self.to[0].as_ref().map(|p| p[b]).unwrap_or(0)
        }

        /// 增量解码一步（通道用）；None = 需要更多字节。
        pub fn decode_step(&self, bytes: &[u8]) -> Option<(String, usize)> {
            if bytes.is_empty() {
                return None;
            }
            let b = bytes[0] as usize;
            if self.prefix[b] {
                if bytes.len() < 2 {
                    return None;
                }
                let ch = self
                    .to[b]
                    .as_ref()
                    .map(|p| p[bytes[1] as usize])
                    .unwrap_or(0);
                if ch == 0 && b != 0 {
                    Some(((b as u8 as char).to_string(), 1))
                } else {
                    let c = char::from_u32(ch as u32).unwrap_or('\u{FFFD}');
                    Some((c.to_string(), 2))
                }
            } else {
                let ch = self.page0(b);
                if ch == 0 && b != 0 {
                    Some(((b as u8 as char).to_string(), 1))
                } else {
                    let c = char::from_u32(ch as u32).unwrap_or('\u{FFFD}');
                    Some((c.to_string(), 1))
                }
            }
        }

        /// 编码整串：未收录字符 → fallback word（tclsh 同）。
        pub fn encode(&self, text: &str) -> Vec<u8> {
            let mut out = Vec::with_capacity(text.len());
            for c in text.chars() {
                let u = c as u32;
                let mut word = if u <= 0xFFFF {
                    self.from.get(&(u as u16)).copied().unwrap_or(0)
                } else {
                    0
                };
                if word == 0 && u != 0 {
                    word = self.fallback;
                }
                if self.prefix[(word >> 8) as usize] {
                    out.push((word >> 8) as u8);
                    out.push(word as u8);
                } else {
                    out.push(word as u8);
                }
            }
            out
        }
    }

    /// 转义序列编码（iso2022 / iso2022-jp / iso2022-kr）。
    pub struct EscapeEncoding {
        init: Vec<u8>,
        fin: Vec<u8>,
        /// (escape 字节, 子表)；顺序即 tclsh 的候选序。
        subs: Vec<(Vec<u8>, Arc<TableEncoding>)>,
        prefix: [bool; 256],
    }

    impl EscapeEncoding {
        /// 解码整串：转义序列切换子表；未知转义 → U+FFFD；子表查不到
        /// 的字节对 → NUL（Tcl EscapeToUtfProc 不回落字节值）。
        pub fn decode(&self, bytes: &[u8]) -> String {
            let mut out = String::with_capacity(bytes.len());
            let mut i = 0usize;
            let mut state = 0usize;
            while i < bytes.len() {
                let b = bytes[i] as usize;
                if self.prefix[b] {
                    let cur = &bytes[i..];
                    if !self.init.is_empty() && cur.starts_with(&self.init) {
                        i += self.init.len();
                        continue;
                    }
                    if !self.fin.is_empty() && cur.starts_with(&self.fin) {
                        i += self.fin.len();
                        continue;
                    }
                    let mut matched = false;
                    let mut partial = false;
                    let mut longest = self.init.len().max(self.fin.len());
                    for (si, (seq, _)) in self.subs.iter().enumerate() {
                        longest = longest.max(seq.len());
                        if cur.starts_with(seq) {
                            state = si;
                            i += seq.len();
                            matched = true;
                            break;
                        }
                        if seq.starts_with(cur) {
                            partial = true;
                        }
                    }
                    if matched {
                        continue;
                    }
                    out.push('\u{FFFD}');
                    if partial {
                        i = bytes.len();
                    } else {
                        i += longest.min(cur.len()).max(1);
                    }
                    continue;
                }
                let table = &self.subs[state].1;
                if table.prefix[b] {
                    if i + 1 >= bytes.len() {
                        out.push('\u{FFFD}');
                        i = bytes.len();
                        continue;
                    }
                    let ch = table
                        .to[b]
                        .as_ref()
                        .map(|p| p[bytes[i + 1] as usize])
                        .unwrap_or(0);
                    out.push(char::from_u32(ch as u32).unwrap_or('\0'));
                    i += 2;
                } else {
                    let ch = table.page0(b);
                    out.push(char::from_u32(ch as u32).unwrap_or('\0'));
                    i += 1;
                }
            }
            out
        }

        /// 编码整串：先试当前子表，未收录则按表序找候选并插入其
        /// 转义序列；都找不到 → 当前子表 fallback（不换状态）。
        pub fn encode(&self, text: &str, emit_init: bool) -> Vec<u8> {
            let mut out = Vec::new();
            if emit_init {
                out.extend_from_slice(&self.init);
            }
            let mut state = 0usize;
            for c in text.chars() {
                let mut u = c as u32;
                if u > 0xFFFF {
                    u = 0xFFFD;
                }
                let key = u as u16;
                let mut word = self.subs[state]
                    .1
                    .from
                    .get(&key)
                    .copied()
                    .unwrap_or(0);
                if word == 0 && u != 0 {
                    let old = state;
                    let mut found = false;
                    for (si, (_, tab)) in self.subs.iter().enumerate() {
                        if let Some(w) = tab.from.get(&key).copied() {
                            if w != 0 {
                                state = si;
                                word = w;
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        state = old;
                        word = self.subs[state].1.fallback;
                    } else if state != old {
                        out.extend_from_slice(&self.subs[state].0);
                    }
                }
                let tab = &self.subs[state].1;
                if tab.prefix[(word >> 8) as usize] {
                    out.push((word >> 8) as u8);
                    out.push(word as u8);
                } else {
                    out.push(word as u8);
                }
            }
            // END：非默认子表时先切回 subTables[0] 的转义序列，
            // 再写 final（iso2022-jp 的 ESC(B / iso2022-kr 的 SI）。
            if state != 0 {
                out.extend_from_slice(&self.subs[0].0);
            }
            out.extend_from_slice(&self.fin);
            out
        }

        /// 增量解码一步；空文本 + consumed 表示只消费了转义序列。
        pub fn decode_step(&self, bytes: &[u8], state: &mut u32) -> Option<(String, usize)> {
            if bytes.is_empty() {
                return None;
            }
            let b = bytes[0] as usize;
            if self.prefix[b] {
                if !self.init.is_empty() {
                    if bytes.starts_with(&self.init) {
                        return Some((String::new(), self.init.len()));
                    }
                    if self.init.starts_with(bytes) {
                        return None;
                    }
                }
                if !self.fin.is_empty() {
                    if bytes.starts_with(&self.fin) {
                        return Some((String::new(), self.fin.len()));
                    }
                    if self.fin.starts_with(bytes) {
                        return None;
                    }
                }
                let mut longest = self.init.len().max(self.fin.len());
                let mut partial = false;
                for (si, (seq, _)) in self.subs.iter().enumerate() {
                    longest = longest.max(seq.len());
                    if bytes.starts_with(seq) {
                        *state = si as u32;
                        return Some((String::new(), seq.len()));
                    }
                    if seq.starts_with(bytes) {
                        partial = true;
                    }
                }
                if partial {
                    return None;
                }
                return Some((
                    "\u{FFFD}".to_string(),
                    longest.min(bytes.len()).max(1),
                ));
            }
            let table = &self.subs[*state as usize].1;
            if table.prefix[b] {
                if bytes.len() < 2 {
                    return None;
                }
                let ch = table
                    .to[b]
                    .as_ref()
                    .map(|p| p[bytes[1] as usize])
                    .unwrap_or(0);
                Some((
                    char::from_u32(ch as u32).unwrap_or('\0').to_string(),
                    2,
                ))
            } else {
                let ch = table.page0(b);
                Some((
                    char::from_u32(ch as u32).unwrap_or('\0').to_string(),
                    1,
                ))
            }
        }
    }

    pub enum Codec {
        Table(TableEncoding),
        Escape(EscapeEncoding),
    }

    impl Codec {
        pub fn decode(&self, bytes: &[u8]) -> String {
            match self {
                Codec::Table(t) => t.decode(bytes),
                Codec::Escape(e) => e.decode(bytes),
            }
        }
        pub fn encode(&self, text: &str, emit_init: bool) -> Vec<u8> {
            match self {
                Codec::Table(t) => t.encode(text),
                Codec::Escape(e) => e.encode(text, emit_init),
            }
        }
        pub fn decode_step(
            &self,
            bytes: &[u8],
            state: &mut u32,
        ) -> Option<(String, usize)> {
            match self {
                Codec::Table(t) => t.decode_step(bytes),
                Codec::Escape(e) => e.decode_step(bytes, state),
            }
        }
    }

    fn default_dirs() -> Vec<PathBuf> {
        if let Ok(lib) = std::env::var("TCL_LIBRARY") {
            vec![PathBuf::from(lib).join("encoding")]
        } else {
            vec![PathBuf::from("/usr/share/tcltk/tcl8.6/encoding")]
        }
    }

    fn dirs() -> &'static RwLock<Vec<PathBuf>> {
        static DIRS: OnceLock<RwLock<Vec<PathBuf>>> = OnceLock::new();
        DIRS.get_or_init(|| RwLock::new(default_dirs()))
    }

    fn cache() -> &'static Mutex<HashMap<String, Option<Arc<Codec>>>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Codec>>>>> =
            OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn table_cache() -> &'static Mutex<HashMap<String, Option<Arc<TableEncoding>>>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<TableEncoding>>>>> =
            OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// `encoding dirs` 设置：更新全局搜索路径并清空已加载缓存
    /// （tclsh 的 encoding dirs 也是进程级的）。
    pub fn set_dirs(list: &[String]) {
        let paths: Vec<PathBuf> = list.iter().map(PathBuf::from).collect();
        if let Ok(mut d) = dirs().write() {
            *d = paths;
        }
        if let Ok(mut c) = cache().lock() {
            c.clear();
        }
        if let Ok(mut c) = table_cache().lock() {
            c.clear();
        }
    }

    fn read_enc(name: &str) -> Option<String> {
        let dirs = dirs().read().ok()?;
        for dir in dirs.iter() {
            let path = dir.join(format!("{}.enc", name));
            if let Ok(d) = std::fs::read_to_string(&path) {
                return Some(d);
            }
        }
        None
    }

    fn file_type(data: &str) -> Option<&str> {
        data.lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with('#'))
    }

    /// 取（并缓存）某编码的 codec；文件不存在/解析失败 → None。
    pub fn load(name: &str) -> Option<Arc<Codec>> {
        if let Ok(c) = cache().lock() {
            if let Some(e) = c.get(name) {
                return e.clone();
            }
        }
        let codec = (|| -> Option<Codec> {
            let data = read_enc(name)?;
            match file_type(&data)? {
                "S" | "D" | "M" => Some(Codec::Table(parse_table(&data)?)),
                "E" => Some(Codec::Escape(parse_escape(&data)?)),
                _ => None,
            }
        })()
        .map(Arc::new);
        const ENCODING_CACHE_MAX: usize = 256;
        if let Ok(mut c) = cache().lock() {
            if c.len() >= ENCODING_CACHE_MAX {
                c.clear();
            }
            c.insert(name.to_string(), codec.clone());
        }
        codec
    }

    fn load_table(name: &str) -> Option<Arc<TableEncoding>> {
        if let Ok(c) = table_cache().lock() {
            if let Some(e) = c.get(name) {
                return e.clone();
            }
        }
        let t = (|| -> Option<TableEncoding> {
            let data = read_enc(name)?;
            match file_type(&data)? {
                "S" | "D" | "M" => parse_table(&data),
                _ => None,
            }
        })()
        .map(Arc::new);
        const ENCODING_CACHE_MAX: usize = 256;
        if let Ok(mut c) = table_cache().lock() {
            if c.len() >= ENCODING_CACHE_MAX {
                c.clear();
            }
            c.insert(name.to_string(), t.clone());
        }
        t
    }

    fn hex4(s: &str) -> Option<u16> {
        u16::from_str_radix(s, 16).ok()
    }

    fn parse_table(data: &str) -> Option<TableEncoding> {
        let mut lines = data.lines();
        let typ = loop {
            let l = lines.next()?.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            break l.to_string();
        };
        if typ != "S" && typ != "D" && typ != "M" {
            return None;
        }
        let header = loop {
            let l = lines.next()?.trim();
            if l.is_empty() {
                continue;
            }
            break l.to_string();
        };
        let h: Vec<&str> = header.split_whitespace().collect();
        if h.len() < 3 {
            return None;
        }
        let fallback = hex4(h[0])?;
        let symbol = h[1].parse::<i64>().ok()? != 0;
        let num_pages = h[2].parse::<i64>().ok()?.clamp(0, 256) as usize;

        let rest: Vec<&str> = lines
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();
        let mut idx = 0usize;
        let mut to: Vec<Option<Box<[u16; 256]>>> = vec![None; 256];
        for _ in 0..num_pages {
            let hi = u8::from_str_radix(*rest.get(idx)?, 16).ok()? as usize;
            idx += 1;
            let mut page = Box::new([0u16; 256]);
            for g in 0..16 {
                let tok = *rest.get(idx)?;
                idx += 1;
                if tok.len() != 64 {
                    return None;
                }
                for k in 0..16 {
                    page[g * 16 + k] = hex4(&tok[k * 4..k * 4 + 4])?;
                }
            }
            to[hi] = Some(page);
        }

        let mut prefix = [false; 256];
        if typ == "D" {
            prefix = [true; 256];
        } else {
            for hi in 1..256usize {
                prefix[hi] = to[hi].is_some();
            }
        }

        // 反查表（Unicode → word）；后面的页覆盖前面的同码点（tclsh 同）。
        let mut from: HashMap<u16, u16> = HashMap::new();
        for hi in 0..256usize {
            if let Some(page) = to[hi].as_ref() {
                for lo in 0..256usize {
                    let ch = page[lo];
                    if ch != 0 {
                        from.insert(ch, ((hi as u16) << 8) | lo as u16);
                    }
                }
            }
        }
        if typ == "M" {
            from.entry(0x5C).or_insert(0x5C);
        }
        if symbol {
            if let Some(page) = to[0].as_ref() {
                for lo in 0..256usize {
                    if page[lo] != 0 {
                        from.insert(lo as u16, lo as u16);
                    }
                }
            }
        }

        // 可选 R 反查段：行首 4 hex 为目标 word，随后 4 hex 为源码点。
        if rest.get(idx).map(|s| *s == "R").unwrap_or(false) {
            idx += 1;
            while let Some(line) = rest.get(idx) {
                idx += 1;
                let toks: Vec<&str> = line.split_whitespace().collect();
                if toks.len() < 2 {
                    continue;
                }
                let Some(to_word) = hex4(toks[0]) else { continue };
                if to_word == 0 {
                    continue;
                }
                for t in &toks[1..] {
                    if let Some(from_ch) = hex4(t) {
                        if from_ch != 0 {
                            from.insert(from_ch, to_word);
                        }
                    }
                }
            }
        }

        Some(TableEncoding {
            fallback,
            prefix,
            to,
            from,
        })
    }

    fn parse_escape(data: &str) -> Option<EscapeEncoding> {
        let mut init: Vec<u8> = Vec::new();
        let mut fin: Vec<u8> = Vec::new();
        let mut subs_raw: Vec<(Vec<u8>, String)> = Vec::new();
        let mut first = true;
        for line in data.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if first {
                first = false;
                if l == "E" {
                    continue;
                }
                return None;
            }
            let mut it = l.split_whitespace();
            let key = it.next()?;
            let val = it.next()?;
            let seq = if val == "{}" {
                Vec::new()
            } else {
                unescape(val)
            };
            match key {
                "name" => {}
                "init" => init = seq,
                "final" => fin = seq,
                other => subs_raw.push((seq, other.to_string())),
            }
        }
        let mut subs = Vec::new();
        for (seq, name) in subs_raw {
            let tab = load_table(&name)?;
            subs.push((seq, tab));
        }
        if subs.is_empty() {
            return None;
        }
        // 只有各序列的**首字节**是转义前缀（Tcl LoadEscapeEncoding
        // 同款——把 init/final 的所有字节都当 prefix 会把 ")" 等普通
        // 字节误判成转义起点）。
        let mut prefix = [false; 256];
        if let Some(&b) = init.first() {
            prefix[b as usize] = true;
        }
        if let Some(&b) = fin.first() {
            prefix[b as usize] = true;
        }
        for (seq, _) in &subs {
            if let Some(&b) = seq.first() {
                prefix[b as usize] = true;
            }
        }
        Some(EscapeEncoding {
            init,
            fin,
            subs,
            prefix,
        })
    }

    /// Tcl 列表字面量的反斜杠解码（编码文件只用到 \xHH/\uHHHH/\NNN
    /// 与常见控制字符）。
    fn unescape(s: &str) -> Vec<u8> {
        let chars: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] != '\\' {
                let mut buf = [0u8; 4];
                out.extend_from_slice(chars[i].encode_utf8(&mut buf).as_bytes());
                i += 1;
                continue;
            }
            i += 1;
            if i >= chars.len() {
                out.push(b'\\');
                break;
            }
            match chars[i] {
                'x' => {
                    i += 1;
                    let mut v = 0u32;
                    let mut n = 0;
                    while n < 2 && i < chars.len() {
                        if let Some(d) = chars[i].to_digit(16) {
                            v = v * 16 + d;
                            i += 1;
                            n += 1;
                        } else {
                            break;
                        }
                    }
                    out.push(v as u8);
                }
                'u' => {
                    i += 1;
                    let mut v = 0u32;
                    let mut n = 0;
                    while n < 4 && i < chars.len() {
                        if let Some(d) = chars[i].to_digit(16) {
                            v = v * 16 + d;
                            i += 1;
                            n += 1;
                        } else {
                            break;
                        }
                    }
                    if let Some(c) = char::from_u32(v) {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                }
                '0'..='7' => {
                    let mut v = 0u32;
                    let mut n = 0;
                    while n < 3 && i < chars.len() {
                        if let Some(d) = chars[i].to_digit(8) {
                            v = v * 8 + d;
                            i += 1;
                            n += 1;
                        } else {
                            break;
                        }
                    }
                    out.push(v as u8);
                }
                'n' => {
                    out.push(b'\n');
                    i += 1;
                }
                'r' => {
                    out.push(b'\r');
                    i += 1;
                }
                't' => {
                    out.push(b'\t');
                    i += 1;
                }
                'a' => {
                    out.push(7);
                    i += 1;
                }
                'b' => {
                    out.push(8);
                    i += 1;
                }
                'f' => {
                    out.push(12);
                    i += 1;
                }
                'v' => {
                    out.push(11);
                    i += 1;
                }
                '\\' => {
                    out.push(b'\\');
                    i += 1;
                }
                other => {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
                    i += 1;
                }
            }
        }
        out
    }
}

/// 非内建编码的整串解码：优先 .enc 表（tclsh 同源），再 iconv，最后
/// 字节直映。
fn wide_decode(bytes: &[u8], enc: &str) -> String {
    #[cfg(feature = "std")]
    if let Some(t) = table::load(enc) {
        return t.decode(bytes);
    }
    #[cfg(all(unix, feature = "iconv"))]
    {
        iconv_decode(bytes, enc)
    }
    #[cfg(not(all(unix, feature = "iconv")))]
    {
        bytes.iter().map(|&b| b as char).collect()
    }
}

fn wide_encode(text: &str, enc: &str, emit_init: bool) -> Vec<u8> {
    #[cfg(feature = "std")]
    if let Some(t) = table::load(enc) {
        return t.encode(text, emit_init);
    }
    #[cfg(all(unix, feature = "iconv"))]
    {
        iconv_encode(text, enc)
    }
    #[cfg(not(all(unix, feature = "iconv")))]
    {
        text.as_bytes().to_vec()
    }
}

/// 增量解码一步（通道用）：优先 .enc 表（含 iso2022 转义状态），再
/// iconv。`state` 仅转义编码使用（调用方持久化）。
pub fn decode_step(bytes: &[u8], enc: &str, state: &mut u32) -> Option<(String, usize)> {
    #[cfg(feature = "std")]
    if let Some(t) = table::load(enc) {
        return t.decode_step(bytes, state);
    }
    #[cfg(all(unix, feature = "iconv"))]
    {
        let _ = state;
        iconv_decode_step(bytes, enc)
    }
    #[cfg(not(all(unix, feature = "iconv")))]
    {
        let _ = state;
        if bytes.is_empty() {
            None
        } else {
            Some(((bytes[0] as char).to_string(), 1))
        }
    }
}

/// 设置 .enc 表搜索目录（`encoding dirs` 设置时调用；std 构建）。
pub fn set_table_dirs(list: &[String]) {
    #[cfg(feature = "std")]
    table::set_dirs(list);
    #[cfg(not(feature = "std"))]
    let _ = list;
}

/// 字节串 → 字符（Tcl 的字节值直映约定：每个输入字节出 0..255 的字符）。
pub fn decode(bytes: &[u8], enc: &str) -> String {
    match enc {
        "utf-8" => decode_tcl_utf8(bytes),
        "ascii" | "iso8859-1" | "identity" | "binary" => {
            bytes.iter().map(|&b| b as char).collect()
        }
        "unicode" => decode_ucs2le(bytes),
        _ => wide_decode(bytes, enc),
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
        _ => wide_encode(text, enc, true),
    }
}

/// 通道写串：`emit_init=false` 时省掉 E 编码的起始转义（tclsh 的
/// channel encoder 只在首次写时发 init；后续写直接延续状态）。
pub fn encode_cont(text: &str, enc: &str, emit_init: bool) -> Vec<u8> {
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
                    out.push(0xFF);
                }
            }
            out
        }
        _ => wide_encode(text, enc, emit_init),
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
    fn missing_iconv_uses_table_or_byte_pass_through() {
        assert!(!ICONV_BACKEND);
        // std 构建 + encoding 目录可用时，.enc 表后端与 iconv 无关地
        // 保持精确；无文件系统（wasm/no_std）才退回字节直映。
        let dec = decode(b"\x82\xa0", "shiftjis");
        assert!(dec == "あ" || dec == "\u{82}\u{a0}", "got {dec:?}");
        let enc = encode("あ", "shiftjis");
        assert!(enc == b"\x82\xa0" || enc == "あ".as_bytes(), "got {enc:?}");
    }
}
