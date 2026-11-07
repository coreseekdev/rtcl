//! Channel abstraction for Tcl I/O.
//!
//! Modelled after Wasmtime's `wasi-common::pipe` design — channels use
//! `Arc<Mutex<T>>` for shared ownership so that embedders can inject virtual
//! stdin / capture stdout without owning the interpreter.
//!
//! Channel types:
//! - **OsStdin / OsStdout / OsStderr** — delegates to `std::io::{stdin,stdout,stderr}`
//! - **FileChannel** — an open `std::fs::File` with `BufReader` for read modes
//! - **PipeChannel** — wraps a `std::process::Child`
//! - **MemoryInputPipe** — readable from an `Arc<Mutex<Cursor<Vec<u8>>>>` buffer
//! - **MemoryOutputPipe** — writable to an `Arc<Mutex<Vec<u8>>>` buffer with capacity
//! - **SinkChannel** — discards all writes (like `/dev/null`)

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader, Cursor, Read, Seek, Write};
use std::sync::{Arc, Mutex};

/// A unique channel identifier (e.g. "file3", "stdin").
pub type ChannelId = String;

// ═══════════════════════════════════════════════════════════════════════
//  Channel trait
// ═══════════════════════════════════════════════════════════════════════

/// Trait object for an open channel.
pub trait Channel: Send {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn read_line(&mut self) -> io::Result<Option<String>>;
    fn read_all(&mut self) -> io::Result<String>;
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize>;
    fn flush(&mut self) -> io::Result<()>;
    fn seek(&mut self, whence: io::SeekFrom) -> io::Result<u64>;
    fn tell(&mut self) -> io::Result<u64>;
    fn eof(&self) -> bool;
    fn close(self: Box<Self>) -> io::Result<()>;
    fn is_readable(&self) -> bool;
    fn is_writable(&self) -> bool;
    /// Truncate a seekable, writable channel (`chan truncate`). The default
    /// implementation reports the operation as unsupported.
    fn set_length(&mut self, _len: u64) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "channel is not truncatable"))
    }
    fn configure(&mut self, _cfg: &ChannelConfig) {}
    fn channel_type(&self) -> &'static str;
    /// Return associated process IDs (for pipe channels). Empty list for non-pipe channels.
    fn pids(&self) -> Vec<u32> { vec![] }
}

/// 输出 EOL 翻译（写侧，字符级——编码之前）。
fn translate_output(s: &str, mode: TranslationMode) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match mode {
        TranslationMode::Lf => Cow::Borrowed(s),
        TranslationMode::Auto => translate_output(s, platform_translation()),
        TranslationMode::Cr => {
            if s.contains('\n') {
                Cow::Owned(s.replace('\n', "\r"))
            } else {
                Cow::Borrowed(s)
            }
        }
        TranslationMode::CrLf => {
            if s.contains('\n') {
                Cow::Owned(s.replace('\n', "\r\n"))
            } else {
                Cow::Borrowed(s)
            }
        }
    }
}

/// 写完整字节串（处理短写）。
fn write_all_bytes(ch: &mut dyn Channel, bytes: &[u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let n = ch.write_bytes(&bytes[offset..])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
        }
        offset += n;
    }
    Ok(())
}

/// 写串：按通道 `-translation`（输出侧）翻译，再按 `-encoding` 编码。
pub(crate) fn channel_write_str_cfg(
    ch: &mut dyn Channel,
    cfg: &mut ChannelConfig,
    s: &str,
) -> io::Result<usize> {
    let translated = translate_output(s, cfg.translation_out);
    let encoded = crate::encoding::encode_cont(&translated, &cfg.encoding, !cfg.enc_started);
    cfg.enc_started = true;
    write_all_bytes(ch, &encoded)?;
    Ok(encoded.len())
}

/// 写串（翻译后按 **UTF-8 内部字节** 落盘，忽略 `-encoding`）。
///
/// `chan copy` 的 tclsh 怪形：目标 `-encoding binary` 时，若源通道是
/// 真实编码（utf-8/iso8859-1/...），Tcl 把对象写为其 UTF-8 内部表示
/// （c3 a9），只有源通道也是 binary（字节串对象）时才逐低字节写
/// （e9）。`puts` 不适用此规则（仍走 `channel_write_str_cfg`）。
pub(crate) fn channel_write_str_utf8_cfg(
    ch: &mut dyn Channel,
    cfg: &ChannelConfig,
    s: &str,
) -> io::Result<usize> {
    let translated = translate_output(s, cfg.translation_out);
    write_all_bytes(ch, translated.as_bytes())?;
    Ok(translated.len())
}

/// 字符级通道读取器：负责按 `-encoding` 增量解码 + 按
/// `-translation`（输入侧）翻译 EOL。未成形的多字节尾巴、CR 前瞻
/// 回推字符都持久化在 [`ChannelConfig`] 中（跨 read/gets 调用）。
struct ChannelReader<'a> {
    ch: &'a mut dyn Channel,
    cfg: &'a mut ChannelConfig,
}

impl<'a> ChannelReader<'a> {
    fn new(ch: &'a mut dyn Channel, cfg: &'a mut ChannelConfig) -> Self {
        ChannelReader { ch, cfg }
    }

    fn read_byte(&mut self) -> io::Result<Option<u8>> {
        let mut b = [0u8; 1];
        let n = self.ch.read_bytes(&mut b)?;
        Ok(if n == 0 { None } else { Some(b[0]) })
    }

    /// 从字节尾巴里尝试解出一个字符；`None` = 需要更多字节。
    fn try_decode(&mut self, enc: &str) -> Option<char> {
        if self.cfg.read_bytes.is_empty() {
            return None;
        }
        match enc {
            "utf-8" => {
                let (c, used) = {
                    let bytes = &self.cfg.read_bytes;
                    match std::str::from_utf8(bytes) {
                        Ok(s) => {
                            let c = s.chars().next().unwrap();
                            (c, c.len_utf8())
                        }
                        Err(e) => {
                            if e.valid_up_to() > 0 {
                                let valid = e.valid_up_to();
                                // SAFETY: from_utf8 已确认前缀合法。
                                let s = unsafe {
                                    std::str::from_utf8_unchecked(&bytes[..valid])
                                };
                                let c = s.chars().next().unwrap();
                                (c, c.len_utf8())
                            } else if e.error_len().is_some() {
                                // 无效字节：按字节值出字符。
                                (bytes[0] as char, 1)
                            } else {
                                return None; // 不完整序列：继续读
                            }
                        }
                    }
                };
                self.cfg.read_bytes.drain(..used);
                Some(c)
            }
            "ascii" | "iso8859-1" | "identity" | "binary" => {
                let b = self.cfg.read_bytes[0];
                self.cfg.read_bytes.drain(..1);
                Some(b as char)
            }
            "unicode" => {
                if self.cfg.read_bytes.len() < 2 {
                    return None;
                }
                let u = (self.cfg.read_bytes[0] as u32)
                    | ((self.cfg.read_bytes[1] as u32) << 8);
                self.cfg.read_bytes.drain(..2);
                Some(char::from_u32(u).unwrap_or('\u{FFFD}'))
            }
            _ => {
                // .enc 表 / iconv / 字节直映统一走 decode_step；空文本
                // + consumed 表示只消费了转义序列（iso2022），继续解析。
                loop {
                    if let Some((text, consumed)) = crate::encoding::decode_step(
                        &self.cfg.read_bytes,
                        enc,
                        &mut self.cfg.dec_state,
                    ) {
                        if consumed > 0 {
                            self.cfg.read_bytes.drain(..consumed);
                        }
                        let mut it = text.chars();
                        let first = it.next();
                        for c in it {
                            self.cfg.read_chars.push_back(c);
                        }
                        if first.is_some() {
                            return first;
                        }
                        if self.cfg.read_bytes.is_empty() || consumed == 0 {
                            return None;
                        }
                        continue;
                    }
                    return None;
                }
            }
        }
    }

    /// EOF 处处理未成形的尾巴（Tcl 约定：按字节值 / U+FFFD）。
    fn flush_eof(&mut self, enc: &str) -> Option<char> {
        if self.cfg.read_bytes.is_empty() {
            return None;
        }
        if enc == "unicode" {
            // 奇尾字节 → U+FFFD。
            self.cfg.read_bytes.clear();
            return Some('\u{FFFD}');
        }
        if !matches!(
            enc,
            "utf-8" | "ascii" | "iso8859-1" | "identity" | "binary" | "unicode"
        ) {
            if let Some((text, consumed)) = crate::encoding::decode_step(
                &self.cfg.read_bytes,
                enc,
                &mut self.cfg.dec_state,
            ) {
                if consumed > 0 {
                    self.cfg.read_bytes.drain(..consumed);
                    let mut it = text.chars();
                    let first = it.next();
                    for c in it {
                        self.cfg.read_chars.push_back(c);
                    }
                    if first.is_some() {
                        return first;
                    }
                }
            }
        }
        let b = self.cfg.read_bytes.remove(0);
        Some(b as char)
    }

    /// 下一个原始（未翻译）字符。
    fn next_raw(&mut self, enc: &str) -> io::Result<Option<char>> {
        if let Some(c) = self.cfg.read_chars.pop_front() {
            return Ok(Some(c));
        }
        loop {
            if let Some(c) = self.try_decode(enc) {
                return Ok(Some(c));
            }
            match self.read_byte()? {
                Some(b) => self.cfg.read_bytes.push(b),
                None => return Ok(self.flush_eof(enc)),
            }
        }
    }

    /// 原始字符 + CR 前瞻回推优先。
    fn next_raw_or_pending(&mut self, enc: &str) -> io::Result<Option<char>> {
        if let Some(c) = self.cfg.read_pending.pop_front() {
            return Ok(Some(c));
        }
        self.next_raw(enc)
    }

    /// 下一个翻译后的字符（输入侧 EOL 归一）。
    fn next_translated(&mut self, enc: &str) -> io::Result<Option<char>> {
        let Some(c) = self.next_raw_or_pending(enc)? else {
            return Ok(None);
        };
        match self.cfg.translation_in {
            TranslationMode::Lf => Ok(Some(c)),
            TranslationMode::Cr => Ok(Some(if c == '\r' { '\n' } else { c })),
            TranslationMode::Auto | TranslationMode::CrLf => {
                if c != '\r' {
                    return Ok(Some(c));
                }
                let keep_cr = self.cfg.translation_in == TranslationMode::CrLf;
                let term = if keep_cr { '\r' } else { '\n' };
                match self.next_raw_or_pending(enc)? {
                    Some('\n') => Ok(Some('\n')),
                    Some(other) => {
                        self.cfg.read_pending.push_back(other);
                        Ok(Some(term))
                    }
                    None => Ok(Some(term)),
                }
            }
        }
    }
}

/// 按通道 `-encoding` 读 `count` 个**字符**（翻译后计数）。
pub(crate) fn channel_read_chars_cfg(
    ch: &mut dyn Channel,
    cfg: &mut ChannelConfig,
    count: usize,
) -> io::Result<String> {
    // count=0 不触碰 at_eof（tclsh：read ch 0 不置 EOF）。
    if count == 0 {
        return Ok(String::new());
    }
    let enc = cfg.encoding.clone();
    let mut reader = ChannelReader::new(ch, cfg);
    let mut out = String::new();
    let mut chars = 0usize;
    while chars < count {
        match reader.next_translated(&enc)? {
            Some(c) => {
                out.push(c);
                chars += 1;
            }
            None => break,
        }
    }
    Ok(out)
}

/// 按通道 `-encoding`/`-translation` 读一整行（gets 用，返回不含行尾）。
///
/// gets 的行界由**原始字符流**上的 translation 模式决定（不是先翻译
/// 再按 LF 切）：
/// * lf    → 原始 LF 为界，CR 留在内容里；
/// * cr    → 原始 CR 为界，LF 留在内容里；
/// * crlf  → 原始 CRLF 为界，孤立 CR/LF 都留在内容里；
/// * auto  → CRLF、单独 CR、单独 LF 均为界（CRLF 算一个界）。
pub(crate) fn channel_read_line_cfg(
    ch: &mut dyn Channel,
    cfg: &mut ChannelConfig,
) -> io::Result<Option<String>> {
    let enc = cfg.encoding.clone();
    let mode = cfg.translation_in;
    let mut reader = ChannelReader::new(ch, cfg);
    let mut out = String::new();
    let mut saw = false;
    loop {
        let next = reader.next_raw_or_pending(&enc)?;
        match mode {
            TranslationMode::Lf => match next {
                Some('\n') => {
                    saw = true;
                    break;
                }
                Some(c) => {
                    saw = true;
                    out.push(c);
                }
                None => break,
            },
            TranslationMode::Cr => match next {
                Some('\r') => {
                    saw = true;
                    break;
                }
                Some(c) => {
                    saw = true;
                    out.push(c);
                }
                None => break,
            },
            TranslationMode::CrLf => match next {
                Some('\r') => {
                    saw = true;
                    match reader.next_raw_or_pending(&enc)? {
                        Some('\n') => break,
                        Some(other) => {
                            out.push('\r');
                            reader.cfg.read_pending.push_back(other);
                        }
                        None => out.push('\r'),
                    }
                }
                Some(c) => {
                    saw = true;
                    out.push(c);
                }
                None => break,
            },
            TranslationMode::Auto => match next {
                Some('\r') => {
                    saw = true;
                    if let Some(other) = reader.next_raw_or_pending(&enc)? {
                        if other != '\n' {
                            reader.cfg.read_pending.push_back(other);
                        }
                    }
                    break;
                }
                Some('\n') => {
                    saw = true;
                    break;
                }
                Some(c) => {
                    saw = true;
                    out.push(c);
                }
                None => break,
            },
        }
    }
    if !saw {
        return Ok(None);
    }
    Ok(Some(out))
}

/// 按通道 `-encoding`/`-translation` 读到 EOF。
pub(crate) fn channel_read_all_cfg(
    ch: &mut dyn Channel,
    cfg: &mut ChannelConfig,
) -> io::Result<String> {
    let enc = cfg.encoding.clone();
    let mut reader = ChannelReader::new(ch, cfg);
    let mut out = String::new();
    while let Some(c) = reader.next_translated(&enc)? {
        out.push(c);
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════
//  Channel configuration (fconfigure)
// ═══════════════════════════════════════════════════════════════════════

/// End-of-line translation mode.
///
/// Tcl 的通道配置是**输入/输出两个独立模式**（fconfigure 可写
/// `{in out}` 二元组）；`Auto` 只出现在输入侧（CRLF 与单 CR 都归一为
/// LF），输出侧的 auto/platform 在设置时解析为平台默认
/// （unix lf / windows crlf）。`Binary` 在设置时解析为 lf + encoding
/// binary，不单独存储。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TranslationMode {
    /// 输入：CRLF 与单 CR → LF；输出：平台默认（unix LF）。
    #[default]
    Auto,
    /// LF only（输入/输出都不改写）。
    Lf,
    /// CR+LF（输入 CRLF→LF；输出 LF→CRLF）。
    CrLf,
    /// CR only（输入 CR→LF；输出 LF→CR）。
    Cr,
}

impl TranslationMode {
    /// fconfigure 查询串（Auto 出现在输入侧时显示 auto）。
    pub fn query_str(self) -> &'static str {
        match self {
            TranslationMode::Auto => "auto",
            TranslationMode::Lf => "lf",
            TranslationMode::CrLf => "crlf",
            TranslationMode::Cr => "cr",
        }
    }
}

/// 平台默认 EOL（unix LF / windows CRLF）。
pub fn platform_translation() -> TranslationMode {
    if cfg!(windows) {
        TranslationMode::CrLf
    } else {
        TranslationMode::Lf
    }
}

/// Buffering strategy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Buffering {
    #[default]
    Full,
    Line,
    None,
}

/// Per-channel configuration.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// 输入 EOL 翻译（默认 Auto：CRLF/CR → LF）。
    pub translation_in: TranslationMode,
    /// 输出 EOL 翻译（默认平台：unix LF / windows CRLF）。
    pub translation_out: TranslationMode,
    pub buffering: Buffering,
    pub buffer_size: usize,
    pub blocking: bool,
    pub encoding: String,
    /// Input end-of-file character (`fconfigure -eofchar`), if set.
    /// Tcl only allows non-NUL ASCII characters here.
    pub eofchar_in: Option<char>,
    /// Output end-of-file character (`fconfigure -eofchar`), if set.
    pub eofchar_out: Option<char>,
    /// 已解码但被 CR 前瞻回推的字符（跨 read/gets 调用保持）。
    pub(crate) read_pending: VecDeque<char>,
    /// 未成形的多字节输入尾巴。
    pub(crate) read_bytes: Vec<u8>,
    /// 解码器一次产出的多余字符（iconv 多字符输出）。
    pub(crate) read_chars: VecDeque<char>,
    /// 转义编码（iso2022-*）的当前子表状态（跨 read/gets 保持）。
    pub(crate) dec_state: u32,
    /// 输出编码是否已写过首串（E 编码的 init 只发一次）。
    pub(crate) enc_started: bool,
    /// `fileevent ch readable script` 的当前脚本（查询用；rtcl 无事件
    /// 循环，处理器只保存不派发）。
    pub(crate) read_handler: String,
    /// `fileevent ch writable script` 的当前脚本。
    pub(crate) write_handler: String,
}

impl ChannelConfig {
    /// 清空读取器的行/字节前瞻状态（换入新通道时用）。
    pub fn clear_read_state(&mut self) {
        self.read_pending.clear();
        self.read_bytes.clear();
        self.read_chars.clear();
        self.dec_state = 0;
        self.enc_started = false;
    }
}

impl Default for ChannelConfig {
    fn default() -> Self {
        ChannelConfig {
            translation_in: TranslationMode::Auto,
            translation_out: platform_translation(),
            buffering: Buffering::Full,
            buffer_size: 4096,
            blocking: true,
            encoding: "utf-8".to_string(),
            eofchar_in: None,
            eofchar_out: None,
            read_pending: VecDeque::new(),
            read_bytes: Vec::new(),
            read_chars: VecDeque::new(),
            dec_state: 0,
            enc_started: false,
            read_handler: String::new(),
            write_handler: String::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  ChannelTable
// ═══════════════════════════════════════════════════════════════════════

/// Tcl/POSIX open 访问标志（`open name {RDONLY CREAT}` 形式）。
#[derive(Default, Clone, Copy)]
struct PosixAccess {
    read: bool,
    write: bool,
    append: bool,
    create: bool,
    truncate: bool,
    exclusive: bool,
    binary: bool,
}

/// 解析 POSIX 标志列表；未知词 → None（调用方报 invalid access mode）。
fn parse_posix_access(mode: &str) -> Option<PosixAccess> {
    let mut acc = PosixAccess::default();
    let mut any = false;
    for tok in mode.split_whitespace() {
        any = true;
        match tok {
            "RDONLY" => acc.read = true,
            "WRONLY" => acc.write = true,
            "RDWR" => {
                acc.read = true;
                acc.write = true;
            }
            "APPEND" => {
                acc.append = true;
                acc.write = true;
            }
            "CREAT" => acc.create = true,
            "EXCL" => acc.exclusive = true,
            "TRUNC" => {
                acc.truncate = true;
                acc.write = true;
            }
            "BINARY" => acc.binary = true,
            "NOCTTY" | "NONBLOCK" => {}
            _ => return None,
        }
    }
    if !any {
        return None;
    }
    if !acc.read && !acc.write {
        acc.read = true; // 默认 RDONLY
    }
    Some(acc)
}

/// The channel table — maps handle strings to open channels.
pub struct ChannelTable {
    channels: HashMap<ChannelId, Box<dyn Channel>>,
    configs: HashMap<ChannelId, ChannelConfig>,
    /// 系统编码（`encoding system`）：新开通道的默认 `-encoding`。
    pub system_encoding: String,
    /// `encoding dirs` 的值。
    pub encoding_dirs: String,
    /// Registration order of live channels (`chan names` is not sorted —
    /// tclsh reports channels in creation order).
    order: Vec<ChannelId>,
    /// Counter for reflected channel handles (`chan create` → `rc0`, `rc1`, ...).
    refl_counter: u32,
    /// Counter for transform handles (`chan push` → `rt0`, `rt1`, ...).
    trans_counter: u32,
}

impl Default for ChannelTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelTable {
    pub fn new() -> Self {
        let mut table = ChannelTable {
            channels: HashMap::new(),
            configs: HashMap::new(),
            system_encoding: "utf-8".to_string(),
            encoding_dirs: std::env::var("TCL_LIBRARY")
                .map(|p| format!("{}/encoding", p))
                .unwrap_or_else(|_| "/usr/share/tcltk/tcl8.6/encoding".to_string()),
            order: Vec::new(),
            refl_counter: 0,
            trans_counter: 0,
        };
        table.track("stdin".to_string());
        table.track("stdout".to_string());
        table.track("stderr".to_string());
        table.channels.insert("stdin".to_string(), Box::new(OsStdin::new()));
        table.channels.insert("stdout".to_string(), Box::new(OsStdout));
        table.channels.insert("stderr".to_string(), Box::new(OsStderr));
        // stderr defaults to unbuffered (line); stdin to line; stdout to line
        table.configs.insert("stdin".to_string(), ChannelConfig { buffering: Buffering::Line, ..Default::default() });
        table.configs.insert("stdout".to_string(), ChannelConfig { buffering: Buffering::Line, ..Default::default() });
        table.configs.insert("stderr".to_string(), ChannelConfig { buffering: Buffering::None, ..Default::default() });
        table
    }

    /// Record `id` in registration order (no-op when already tracked).
    fn track(&mut self, id: ChannelId) {
        if !self.order.iter().any(|e| e == &id) {
            self.order.push(id);
        }
    }

    fn untrack(&mut self, id: &str) {
        self.order.retain(|e| e != id);
    }

    pub fn open_file(&mut self, path: &str, mode: &str) -> io::Result<ChannelId> {
        let mut binary = false;
        let file = match mode {
            "r" => {
                let f = std::fs::File::open(path)?;
                Box::new(FileChannel::new_read(f)) as Box<dyn Channel>
            }
            "w" => {
                let f = std::fs::File::create(path)?;
                Box::new(FileChannel::new_write(f)) as Box<dyn Channel>
            }
            "a" => {
                let f = std::fs::OpenOptions::new().append(true).create(true).open(path)?;
                Box::new(FileChannel::new_write(f)) as Box<dyn Channel>
            }
            "r+" | "RDWR" => {
                let f = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
                Box::new(FileChannel::new_readwrite(f)) as Box<dyn Channel>
            }
            "w+" => {
                let f = std::fs::OpenOptions::new()
                    .read(true).write(true).create(true).truncate(true).open(path)?;
                Box::new(FileChannel::new_readwrite(f)) as Box<dyn Channel>
            }
            "a+" => {
                let f = std::fs::OpenOptions::new()
                    .read(true).append(true).create(true).open(path)?;
                Box::new(FileChannel::new_readwrite(f)) as Box<dyn Channel>
            }
            _ => {
                // POSIX 标志列表（Tcl 8.6：RDONLY/WRONLY/RDWR/APPEND/
                // BINARY/CREAT/EXCL/NOCTTY/NONBLOCK/TRUNC 的列表）。
                let Some(acc) = parse_posix_access(mode) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "invalid access mode \"{}\": must be RDONLY, WRONLY, RDWR, APPEND, BINARY, CREAT, EXCL, NOCTTY, NONBLOCK, or TRUNC",
                            mode
                        ),
                    ));
                };
                binary = acc.binary;
                let mut opts = std::fs::OpenOptions::new();
                opts.read(acc.read).write(acc.write);
                if acc.append {
                    opts.append(true);
                }
                if acc.create {
                    opts.create(true);
                }
                if acc.truncate {
                    opts.truncate(true);
                }
                if acc.exclusive && acc.create {
                    opts.create_new(true);
                }
                let f = opts.open(path)?;
                Box::new(FileChannel::new_readwrite(f)) as Box<dyn Channel>
            }
        };
        let id = self.next_file_name();
        self.track(id.clone());
        self.channels.insert(id.clone(), file);
        // 文件通道默认：输入 auto、输出平台（unix lf）——与 tclsh 的
        // fconfigure 查询一致（读通道显示 auto，写通道显示 lf）。
        self.configs.insert(id.clone(), ChannelConfig {
            encoding: if binary {
                "binary".to_string()
            } else {
                self.system_encoding.clone()
            },
            translation_in: if binary {
                TranslationMode::Lf
            } else {
                TranslationMode::Auto
            },
            translation_out: if binary {
                TranslationMode::Lf
            } else {
                platform_translation()
            },
            ..ChannelConfig::default()
        });
        Ok(id)
    }

    /// Register a channel with a specific ID (used for pipe channels from exec).
    pub fn register(&mut self, id: ChannelId, channel: Box<dyn Channel>) {
        let sys = self.system_encoding.clone();
        self.configs.entry(id.clone()).or_insert_with(|| ChannelConfig {
            encoding: sys,
            ..ChannelConfig::default()
        });
        if let Some(cfg) = self.configs.get_mut(&id) {
            cfg.clear_read_state();
        }
        self.track(id.clone());
        self.channels.insert(id, channel);
    }

    /// Allocate a new file-style channel ID, register the channel, and return the ID.
    pub fn register_new(&mut self, channel: Box<dyn Channel>) -> ChannelId {
        let id = self.next_file_name();
        self.register(id.clone(), channel);
        id
    }

    /// Tcl's file-channel naming: stdio occupy 0..2, and each open takes
    /// the lowest free `fileN` (closing `file3` makes the next open
    /// `file3` again — tclsh 8.6.17).
    fn next_file_name(&self) -> ChannelId {
        let mut n: u32 = 3;
        loop {
            let id = format!("file{}", n);
            if !self.channels.contains_key(&id) {
                return id;
            }
            n += 1;
        }
    }

    /// Next handle name for a reflected channel (`chan create`): `rc0`, `rc1`, ...
    pub fn alloc_reflected_name(&mut self) -> ChannelId {
        let id = format!("rc{}", self.refl_counter);
        self.refl_counter += 1;
        id
    }

    /// Next handle name for a transform (`chan push`): `rt0`, `rt1`, ...
    pub fn alloc_transform_name(&mut self) -> ChannelId {
        let id = format!("rt{}", self.trans_counter);
        self.trans_counter += 1;
        id
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Box<dyn Channel>> {
        self.channels.get_mut(id)
    }

    pub fn get(&self, id: &str) -> Option<&dyn Channel> {
        self.channels.get(id).map(|ch| ch.as_ref())
    }

    pub fn close(&mut self, id: &str) -> io::Result<()> {
        self.configs.remove(id);
        self.untrack(id);
        match self.channels.remove(id) {
            Some(ch) => ch.close(),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("can not find channel named \"{}\"", id),
            )),
        }
    }

    /// Remove a channel without invoking its close handler (used to swap in
    /// a transformed channel stack).
    pub fn take(&mut self, id: &str) -> Option<Box<dyn Channel>> {
        self.configs.remove(id);
        self.untrack(id);
        self.channels.remove(id)
    }

    pub fn channel_names(&self) -> Vec<&str> {
        self.order
            .iter()
            .filter(|id| self.channels.contains_key(id.as_str()))
            .map(|id| id.as_str())
            .collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.channels.contains_key(id)
    }

    /// Get the configuration for a channel.
    pub fn config(&self, id: &str) -> Option<&ChannelConfig> {
        self.configs.get(id)
    }

    /// Mutably borrow the configuration for a channel.
    pub fn config_mut(&mut self, id: &str) -> Option<&mut ChannelConfig> {
        self.configs.get_mut(id)
    }

    /// 同时可变借用通道与其配置（两个独立 map，字段级拆分）。
    pub fn channel_ctx_mut(
        &mut self,
        id: &str,
    ) -> Option<(&mut dyn Channel, &mut ChannelConfig)> {
        let ch = self.channels.get_mut(id)?;
        let cfg = self.configs.get_mut(id)?;
        Some((ch.as_mut(), cfg))
    }

    /// Replace stdin with a custom channel (for embedders / testing).
    pub fn set_stdin(&mut self, channel: Box<dyn Channel>) {
        self.track("stdin".to_string());
        if let Some(cfg) = self.configs.get_mut("stdin") {
            cfg.clear_read_state();
        }
        self.channels.insert("stdin".to_string(), channel);
    }

    /// Replace stdout with a custom channel (for embedders / testing).
    pub fn set_stdout(&mut self, channel: Box<dyn Channel>) {
        self.track("stdout".to_string());
        self.channels.insert("stdout".to_string(), channel);
    }

    /// Replace stderr with a custom channel (for embedders / testing).
    pub fn set_stderr(&mut self, channel: Box<dyn Channel>) {
        self.track("stderr".to_string());
        self.channels.insert("stderr".to_string(), channel);
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  OS standard streams
// ═══════════════════════════════════════════════════════════════════════

struct OsStdin {
    reader: BufReader<io::Stdin>,
    at_eof: bool,
}

impl OsStdin {
    fn new() -> Self {
        OsStdin { reader: BufReader::new(io::stdin()), at_eof: false }
    }
}

impl Channel for OsStdin {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        if n == 0 { self.at_eof = true; }
        Ok(n)
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 { self.at_eof = true; return Ok(None); }
        strip_eol(&mut line);
        Ok(Some(line))
    }
    fn read_all(&mut self) -> io::Result<String> {
        let mut s = String::new();
        self.reader.read_to_string(&mut s)?;
        self.at_eof = true;
        Ok(s)
    }
    fn write_bytes(&mut self, _data: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdin\" wasn't opened for writing"))
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdin is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> { self.seek(io::SeekFrom::Current(0)) }
    fn eof(&self) -> bool { self.at_eof }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { true }
    fn is_writable(&self) -> bool { false }
    fn channel_type(&self) -> &'static str { "tty" }
}

struct OsStdout;

impl Channel for OsStdout {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        io::stdout().write(data)
    }
    fn flush(&mut self) -> io::Result<()> { io::stdout().flush() }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdout is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> { self.seek(io::SeekFrom::Current(0)) }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
    fn channel_type(&self) -> &'static str { "tty" }
}

struct OsStderr;

impl Channel for OsStderr {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stderr\" wasn't opened for reading"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stderr\" wasn't opened for reading"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stderr\" wasn't opened for reading"))
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        io::stderr().write(data)
    }
    fn flush(&mut self) -> io::Result<()> { io::stderr().flush() }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stderr is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> { self.seek(io::SeekFrom::Current(0)) }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
    fn channel_type(&self) -> &'static str { "tty" }
}

// ═══════════════════════════════════════════════════════════════════════
//  FileChannel
// ═══════════════════════════════════════════════════════════════════════

enum FileMode {
    Read(BufReader<std::fs::File>),
    Write(std::fs::File),
    ReadWrite(BufReader<std::fs::File>),
}

struct FileChannel {
    mode: FileMode,
    at_eof: bool,
    /// ReadWrite 的共享逻辑位置（tclsh r+ 单一位置语义，D-28）：读经
    /// BufReader 预读使底层 fd 位置超前于逻辑位，写前必须把 fd 回卷到
    /// 逻辑位（弃缓冲）再写，否则读后写落到错误偏移。Read/Write 模式
    /// 单向，无此问题（None）。
    rw_pos: Option<u64>,
}

impl FileChannel {
    fn new_read(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::Read(BufReader::new(f)), at_eof: false, rw_pos: None }
    }
    fn new_write(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::Write(f), at_eof: false, rw_pos: None }
    }
    fn new_readwrite(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::ReadWrite(BufReader::new(f)), at_eof: false, rw_pos: Some(0) }
    }
}

impl Channel for FileChannel {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = match &mut self.mode {
            FileMode::Read(r) => r.read(buf)?,
            FileMode::ReadWrite(r) => {
                let n = r.read(buf)?;
                if let Some(pos) = self.rw_pos.as_mut() {
                    *pos += n as u64;
                }
                n
            }
            FileMode::Write(_) => return Err(io::Error::new(
                io::ErrorKind::PermissionDenied, "channel wasn't opened for reading",
            )),
        };
        if n == 0 { self.at_eof = true; }
        Ok(n)
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let n = match &mut self.mode {
            FileMode::Read(r) => r.read_line(&mut line)?,
            FileMode::ReadWrite(r) => {
                let n = r.read_line(&mut line)?;
                if let Some(pos) = self.rw_pos.as_mut() {
                    *pos += n as u64;
                }
                n
            }
            FileMode::Write(_) => return Err(io::Error::new(
                io::ErrorKind::PermissionDenied, "channel wasn't opened for reading",
            )),
        };
        if n == 0 { self.at_eof = true; return Ok(None); }
        strip_eol(&mut line);
        Ok(Some(line))
    }
    fn read_all(&mut self) -> io::Result<String> {
        let mut s = String::new();
        match &mut self.mode {
            FileMode::Read(r) => { r.read_to_string(&mut s)?; }
            FileMode::ReadWrite(r) => {
                r.read_to_string(&mut s)?;
                // 整读后逻辑位 = fd 位（EOF）：BufReader 可能还持缓冲，
                // 但底层 fd 已到真 EOF。
                if let Some(pos) = self.rw_pos.as_mut() {
                    *pos = r.get_mut().stream_position()?;
                }
            }
            FileMode::Write(_) => return Err(io::Error::new(
                io::ErrorKind::PermissionDenied, "channel wasn't opened for reading",
            )),
        }
        self.at_eof = true;
        Ok(s)
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        match &mut self.mode {
            FileMode::Write(f) => f.write(data),
            FileMode::ReadWrite(r) => {
                // D-28：写前把 fd 回卷到共享逻辑位——BufReader::seek 弃
                // 预读缓冲并把 fd 定位到逻辑位，写后逻辑位 = fd 位。
                let pos = self.rw_pos.unwrap_or(0);
                r.seek(io::SeekFrom::Start(pos))?;
                let n = r.get_mut().write(data)?;
                self.rw_pos = Some(pos + n as u64);
                Ok(n)
            }
            FileMode::Read(_) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied, "channel wasn't opened for writing",
            )),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match &mut self.mode {
            FileMode::Write(f) => f.flush(),
            FileMode::ReadWrite(r) => r.get_mut().flush(),
            FileMode::Read(_) => Ok(()),
        }
    }
    fn seek(&mut self, whence: io::SeekFrom) -> io::Result<u64> {
        let pos = match &mut self.mode {
            FileMode::Read(r) => r.seek(whence)?,
            FileMode::Write(f) => f.seek(whence)?,
            FileMode::ReadWrite(r) => {
                let p = r.seek(whence)?;
                self.rw_pos = Some(p);
                p
            }
        };
        Ok(pos)
    }
    fn tell(&mut self) -> io::Result<u64> {
        // ReadWrite 的逻辑位由 rw_pos 记账（BufReader 预读使 fd 位超前，
        // seek(Current 0) 探到的是 fd 位而非 tclsh 语义的通道位）。
        if let Some(pos) = self.rw_pos {
            return Ok(pos);
        }
        self.seek(io::SeekFrom::Current(0))
    }
    fn eof(&self) -> bool { self.at_eof }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool {
        !matches!(self.mode, FileMode::Write(_))
    }
    fn is_writable(&self) -> bool {
        !matches!(self.mode, FileMode::Read(_))
    }
    fn set_length(&mut self, len: u64) -> io::Result<()> {
        match &mut self.mode {
            FileMode::Write(f) => f.set_len(len),
            FileMode::ReadWrite(r) => r.get_mut().set_len(len),
            FileMode::Read(_) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied, "channel wasn't opened for writing",
            )),
        }
    }
    fn channel_type(&self) -> &'static str { "file" }
}

// ═══════════════════════════════════════════════════════════════════════
//  PipeChannel (child process)
// ═══════════════════════════════════════════════════════════════════════

/// A channel wrapping a child process's stdin/stdout.
pub(crate) struct PipeChannel {
    child: std::process::Child,
    reader: Option<BufReader<std::process::ChildStdout>>,
    writer: Option<std::process::ChildStdin>,
    at_eof: bool,
}

impl PipeChannel {
    pub fn new(mut child: std::process::Child) -> Self {
        let stdout = child.stdout.take();
        let stdin = child.stdin.take();
        PipeChannel {
            child,
            reader: stdout.map(BufReader::new),
            writer: stdin,
            at_eof: false,
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Channel for PipeChannel {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(r) = &mut self.reader {
            let n = r.read(buf)?;
            if n == 0 { self.at_eof = true; }
            Ok(n)
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe channel not readable"))
        }
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        if let Some(r) = &mut self.reader {
            let mut line = String::new();
            let n = r.read_line(&mut line)?;
            if n == 0 { self.at_eof = true; return Ok(None); }
            strip_eol(&mut line);
            Ok(Some(line))
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe channel not readable"))
        }
    }
    fn read_all(&mut self) -> io::Result<String> {
        if let Some(r) = &mut self.reader {
            let mut s = String::new();
            r.read_to_string(&mut s)?;
            self.at_eof = true;
            Ok(s)
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe channel not readable"))
        }
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(w) = &mut self.writer {
            w.write(data)
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe channel not writable"))
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(w) = &mut self.writer { w.flush() } else { Ok(()) }
    }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "pipe channel is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> { self.seek(io::SeekFrom::Current(0)) }
    fn eof(&self) -> bool { self.at_eof }
    fn close(mut self: Box<Self>) -> io::Result<()> {
        drop(self.writer.take());
        drop(self.reader.take());
        let _ = self.child.wait();
        Ok(())
    }
    fn is_readable(&self) -> bool { self.reader.is_some() }
    fn is_writable(&self) -> bool { self.writer.is_some() }
    fn channel_type(&self) -> &'static str { "pipe" }
    fn pids(&self) -> Vec<u32> { vec![self.child.id()] }
}

// ═══════════════════════════════════════════════════════════════════════
//  MemoryInputPipe  — Wasmtime-inspired virtual read pipe
// ═══════════════════════════════════════════════════════════════════════

/// A readable in-memory pipe backed by `Arc<Mutex<Cursor<Vec<u8>>>>`.
///
/// Inspired by Wasmtime's `MemoryInputPipe`.  The `Arc<Mutex<…>>` design
/// lets an embedder write data to the buffer *after* handing the pipe to
/// the interpreter — useful for feeding stdin programmatically.
///
/// ```ignore
/// let data = Arc::new(Mutex::new(Cursor::new(b"hello\n".to_vec())));
/// interp.channels.set_stdin(Box::new(MemoryInputPipe::from_shared(data.clone())));
/// ```
#[derive(Clone)]
pub struct MemoryInputPipe {
    buf: Arc<Mutex<Cursor<Vec<u8>>>>,
}

impl MemoryInputPipe {
    /// Create a pipe pre-loaded with `data`.
    pub fn new(data: impl Into<Vec<u8>>) -> Self {
        Self { buf: Arc::new(Mutex::new(Cursor::new(data.into()))) }
    }

    /// Create from a shared buffer (allows external writes after creation).
    pub fn from_shared(buf: Arc<Mutex<Cursor<Vec<u8>>>>) -> Self {
        Self { buf }
    }
}

impl From<&str> for MemoryInputPipe {
    fn from(s: &str) -> Self { Self::new(s.as_bytes().to_vec()) }
}

impl From<String> for MemoryInputPipe {
    fn from(s: String) -> Self { Self::new(s.into_bytes()) }
}

impl From<Vec<u8>> for MemoryInputPipe {
    fn from(v: Vec<u8>) -> Self { Self::new(v) }
}

impl Channel for MemoryInputPipe {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut inner = self.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        let n = inner.read(buf)?;
        Ok(n)
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut inner = self.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        let mut line = String::new();
        let n = inner.read_line(&mut line)?;
        if n == 0 { return Ok(None); }
        strip_eol(&mut line);
        Ok(Some(line))
    }
    fn read_all(&mut self) -> io::Result<String> {
        let mut inner = self.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        let mut s = String::new();
        inner.read_to_string(&mut s)?;
        Ok(s)
    }
    fn write_bytes(&mut self, _data: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "MemoryInputPipe is read-only"))
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, whence: io::SeekFrom) -> io::Result<u64> {
        let mut inner = self.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        inner.seek(whence)
    }
    fn tell(&mut self) -> io::Result<u64> { self.seek(io::SeekFrom::Current(0)) }
    fn eof(&self) -> bool {
        if let Ok(inner) = self.buf.lock() {
            inner.position() >= inner.get_ref().len() as u64
        } else {
            true
        }
    }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { true }
    fn is_writable(&self) -> bool { false }
    fn channel_type(&self) -> &'static str { "memory" }
}

// ══════════════════════════════════════════════════════
//  PipePair — connected in-memory pipe (`chan pipe`)
// ══════════════════════════════════════════════════════

/// Shared buffer behind a `chan pipe` pair.
struct PipeShared {
    buf: Mutex<Vec<u8>>,
    /// Cleared when the write end is closed — after that, reads at the end
    /// of the buffer report EOF.
    open: std::sync::atomic::AtomicBool,
}

impl PipeShared {
    fn new() -> Arc<Self> {
        Arc::new(PipeShared {
            buf: Mutex::new(Vec::new()),
            open: std::sync::atomic::AtomicBool::new(true),
        })
    }
}

/// Read end of a `chan pipe` pair.
pub struct PipeReaderEnd {
    shared: Arc<PipeShared>,
    pos: usize,
}

/// Write end of a `chan pipe` pair.
pub struct PipeWriterEnd {
    shared: Arc<PipeShared>,
}

/// Create a connected `chan pipe` pair: (read end, write end).
pub fn pipe_pair() -> (PipeReaderEnd, PipeWriterEnd) {
    let shared = PipeShared::new();
    (
        PipeReaderEnd { shared: shared.clone(), pos: 0 },
        PipeWriterEnd { shared },
    )
}

impl Channel for PipeReaderEnd {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let inner = self.shared.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        let avail = inner.len().saturating_sub(self.pos);
        if avail == 0 {
            // Nothing buffered (or the write end closed): report EOF. The
            // single-threaded interpreter has no way to block for data.
            return Ok(0);
        }
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&inner[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut byte = [0u8; 1];
        let mut line = Vec::new();
        loop {
            let n = self.read_bytes(&mut byte)?;
            if n == 0 {
                break;
            }
            line.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        if line.is_empty() {
            return Ok(None);
        }
        let mut s = String::from_utf8_lossy(&line).into_owned();
        strip_eol(&mut s);
        Ok(Some(s))
    }
    fn read_all(&mut self) -> io::Result<String> {
        let mut out = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = self.read_bytes(&mut byte)?;
            if n == 0 {
                break;
            }
            out.push(byte[0]);
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
    fn write_bytes(&mut self, _data: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe read end is not writable"))
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "pipe is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> { Ok(self.pos as u64) }
    fn eof(&self) -> bool {
        !self.shared.open.load(std::sync::atomic::Ordering::SeqCst)
            && self.pos >= self.shared.buf.lock().map(|b| b.len()).unwrap_or(0)
    }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { true }
    fn is_writable(&self) -> bool { false }
    fn channel_type(&self) -> &'static str { "pipe" }
}

impl Channel for PipeWriterEnd {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe write end is not readable"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe write end is not readable"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe write end is not readable"))
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut inner = self.shared.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        inner.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "pipe is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> {
        let inner = self.shared.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        Ok(inner.len() as u64)
    }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> {
        self.shared.open.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
    fn channel_type(&self) -> &'static str { "pipe" }
}

// ══════════════════════════════════════════════════════
//  PlaceholderChannel — handle for reflected channels (chan create)
// ══════════════════════════════════════════════════════

/// Placeholder channel registered for a reflected channel. Actual I/O is
/// routed through the handler script by the command layer; this object only
/// carries the direction flags and keeps the handle alive in the table.
pub struct PlaceholderChannel {
    readable: bool,
    writable: bool,
}

impl PlaceholderChannel {
    pub fn new(readable: bool, writable: bool) -> Self {
        PlaceholderChannel { readable, writable }
    }
}

impl Channel for PlaceholderChannel {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> { Ok(0) }
    fn read_line(&mut self) -> io::Result<Option<String>> { Ok(None) }
    fn read_all(&mut self) -> io::Result<String> { Ok(String::new()) }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> { Ok(data.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> { Ok(0) }
    fn tell(&mut self) -> io::Result<u64> { Ok(0) }
    fn eof(&self) -> bool { true }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { self.readable }
    fn is_writable(&self) -> bool { self.writable }
    fn channel_type(&self) -> &'static str { "reflected" }
}

// ═══════════════════════════════════════════════════════════════════════
//  MemoryOutputPipe — Wasmtime-inspired virtual write pipe
// ═══════════════════════════════════════════════════════════════════════

/// A writable in-memory pipe with an optional capacity limit.
///
/// Inspired by Wasmtime's `MemoryOutputPipe`. The `Arc<Mutex<Vec<u8>>>`
/// design lets an embedder read the captured output after execution.
///
/// ```ignore
/// let capture = Arc::new(Mutex::new(Vec::new()));
/// interp.channels.set_stdout(Box::new(
///     MemoryOutputPipe::from_shared(capture.clone(), 1 << 20),
/// ));
/// interp.eval("puts {hello world}").unwrap();
/// let output = capture.lock().unwrap().clone();
/// assert_eq!(String::from_utf8(output).unwrap(), "hello world\n");
/// ```
#[derive(Clone)]
pub struct MemoryOutputPipe {
    buf: Arc<Mutex<Vec<u8>>>,
    capacity: usize,
}

impl MemoryOutputPipe {
    /// Create a new output pipe with the given capacity (bytes).
    /// Use `usize::MAX` for unlimited.
    pub fn new(capacity: usize) -> Self {
        Self { buf: Arc::new(Mutex::new(Vec::new())), capacity }
    }

    /// Create an output pipe backed by a shared buffer.
    pub fn from_shared(buf: Arc<Mutex<Vec<u8>>>, capacity: usize) -> Self {
        Self { buf, capacity }
    }

    /// Snapshot the current contents.
    pub fn contents(&self) -> Vec<u8> {
        self.buf.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Try to unwrap the inner buffer (only succeeds if this is the last reference).
    pub fn try_into_inner(self) -> Option<Vec<u8>> {
        Arc::into_inner(self.buf).map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Channel for MemoryOutputPipe {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "MemoryOutputPipe is write-only"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "MemoryOutputPipe is write-only"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "MemoryOutputPipe is write-only"))
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut inner = self.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        let available = self.capacity.saturating_sub(inner.len());
        if available == 0 {
            return Err(io::Error::other("MemoryOutputPipe capacity exceeded"));
        }
        let n = data.len().min(available);
        inner.extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "MemoryOutputPipe is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> {
        let inner = self.buf.lock().map_err(|_| io::Error::other("lock poisoned"))?;
        Ok(inner.len() as u64)
    }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
    fn channel_type(&self) -> &'static str { "memory" }
}

// ═══════════════════════════════════════════════════════════════════════
//  SinkChannel — /dev/null
// ═══════════════════════════════════════════════════════════════════════

/// A write-only channel that discards all data (like `/dev/null`).
/// Inspired by Wasmtime's `SinkOutputStream`.
#[derive(Clone, Copy)]
pub struct SinkChannel;

impl Channel for SinkChannel {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "SinkChannel is write-only"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "SinkChannel is write-only"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "SinkChannel is write-only"))
    }
    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> { Ok(data.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "SinkChannel is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> { Ok(0) }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
    fn channel_type(&self) -> &'static str { "null" }
}

// ═══════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════

/// Strip trailing \n and optionally preceding \r (CRLF → empty).
fn strip_eol(s: &mut String) {
    if s.ends_with('\n') { s.pop(); }
    if s.ends_with('\r') { s.pop(); }
}

// ═══════════════════════════════════════════════════════════════════════
//  Cursor<Vec<u8>> → BufRead (needed by MemoryInputPipe::read_line)
// ═══════════════════════════════════════════════════════════════════════
// `Cursor<Vec<u8>>` already implements `BufRead`, so no extra impl needed.

// ═══════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_input_pipe_basic() {
        let mut pipe = MemoryInputPipe::new(b"hello world\n".to_vec());
        let line = pipe.read_line().unwrap();
        assert_eq!(line, Some("hello world".to_string()));
        let line2 = pipe.read_line().unwrap();
        assert_eq!(line2, None);
        assert!(pipe.eof());
    }

    #[test]
    fn test_memory_input_pipe_read_bytes() {
        let mut pipe = MemoryInputPipe::new(b"abcdef".to_vec());
        let mut buf = [0u8; 3];
        let n = pipe.read_bytes(&mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf, b"abc");
        let n2 = pipe.read_bytes(&mut buf).unwrap();
        assert_eq!(n2, 3);
        assert_eq!(&buf, b"def");
    }

    #[test]
    fn test_memory_output_pipe_basic() {
        let mut pipe = MemoryOutputPipe::new(1024);
        let n = pipe.write_bytes(b"hello").unwrap();
        assert_eq!(n, 5);
        let n2 = pipe.write_bytes(b" world").unwrap();
        assert_eq!(n2, 6);
        assert_eq!(pipe.contents(), b"hello world");
    }

    #[test]
    fn test_memory_output_pipe_capacity() {
        let mut pipe = MemoryOutputPipe::new(5);
        let n = pipe.write_bytes(b"hello").unwrap();
        assert_eq!(n, 5);
        // Exceeds capacity
        let result = pipe.write_bytes(b"x");
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_output_pipe_shared() {
        let shared = Arc::new(Mutex::new(Vec::new()));
        let mut pipe = MemoryOutputPipe::from_shared(shared.clone(), usize::MAX);
        pipe.write_bytes(b"test").unwrap();
        assert_eq!(shared.lock().unwrap().as_slice(), b"test");
    }

    #[test]
    fn test_sink_channel() {
        let mut sink = SinkChannel;
        let n = sink.write_bytes(b"discarded").unwrap();
        assert_eq!(n, 9);
        assert!(!sink.eof());
        assert!(!sink.is_readable());
        assert!(sink.is_writable());
    }

    #[test]
    fn test_channel_table_set_stdin() {
        let mut table = ChannelTable::new();
        let input = MemoryInputPipe::from("line one\nline two\n");
        table.set_stdin(Box::new(input));
        let ch = table.get_mut("stdin").unwrap();
        let line = ch.read_line().unwrap();
        assert_eq!(line, Some("line one".to_string()));
    }

    #[test]
    fn test_channel_table_set_stdout_capture() {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let mut table = ChannelTable::new();
        table.set_stdout(Box::new(MemoryOutputPipe::from_shared(capture.clone(), usize::MAX)));
        let ch = table.get_mut("stdout").unwrap();
        channel_write_str_cfg(ch.as_mut(), &mut ChannelConfig::default(), "captured!\n").unwrap();
        assert_eq!(capture.lock().unwrap().as_slice(), b"captured!\n");
    }

    #[test]
    fn test_channel_table_register_new() {
        let mut table = ChannelTable::new();
        let pipe = MemoryInputPipe::from("data");
        let id = table.register_new(Box::new(pipe));
        assert!(id.starts_with("file"));
        assert!(table.contains(&id));
    }
}
