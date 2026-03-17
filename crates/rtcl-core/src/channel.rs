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

use std::collections::HashMap;
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
    fn configure(&mut self, _cfg: &ChannelConfig) {}
    fn channel_type(&self) -> &'static str;
}

/// Extension: write a full string, delegates to `write_bytes`.
pub(crate) fn channel_write_str(ch: &mut dyn Channel, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
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

/// Extension: read exactly `count` chars (UTF-8 bytes) and return as String.
pub(crate) fn channel_read_chars(ch: &mut dyn Channel, count: usize) -> io::Result<String> {
    let mut buf = vec![0u8; count];
    let n = ch.read_bytes(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

// ═══════════════════════════════════════════════════════════════════════
//  Channel configuration (fconfigure)
// ═══════════════════════════════════════════════════════════════════════

/// End-of-line translation mode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TranslationMode {
    /// Platform native (\r\n on Windows, \n on Unix).
    #[default]
    Auto,
    /// LF only.
    Lf,
    /// CR+LF.
    CrLf,
    /// CR only.
    Cr,
    /// Raw binary (no translation).
    Binary,
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
    pub translation: TranslationMode,
    pub buffering: Buffering,
    pub buffer_size: usize,
    pub blocking: bool,
    pub encoding: String,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        ChannelConfig {
            translation: TranslationMode::Auto,
            buffering: Buffering::Full,
            buffer_size: 4096,
            blocking: true,
            encoding: "utf-8".to_string(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  ChannelTable
// ═══════════════════════════════════════════════════════════════════════

/// The channel table — maps handle strings to open channels.
pub struct ChannelTable {
    channels: HashMap<ChannelId, Box<dyn Channel>>,
    configs: HashMap<ChannelId, ChannelConfig>,
    next_id: u32,
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
            next_id: 1,
        };
        table.channels.insert("stdin".to_string(), Box::new(OsStdin::new()));
        table.channels.insert("stdout".to_string(), Box::new(OsStdout));
        table.channels.insert("stderr".to_string(), Box::new(OsStderr));
        // stderr defaults to unbuffered (line); stdin to line; stdout to line
        table.configs.insert("stdin".to_string(), ChannelConfig { buffering: Buffering::Line, ..Default::default() });
        table.configs.insert("stdout".to_string(), ChannelConfig { buffering: Buffering::Line, ..Default::default() });
        table.configs.insert("stderr".to_string(), ChannelConfig { buffering: Buffering::None, ..Default::default() });
        table
    }

    pub fn open_file(&mut self, path: &str, mode: &str) -> io::Result<ChannelId> {
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
            _ => return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("bad access mode \"{}\"", mode),
            )),
        };
        let id = format!("file{}", self.next_id);
        self.next_id += 1;
        self.channels.insert(id.clone(), file);
        self.configs.insert(id.clone(), ChannelConfig::default());
        Ok(id)
    }

    /// Register a channel with a specific ID (used for pipe channels from exec).
    pub fn register(&mut self, id: ChannelId, channel: Box<dyn Channel>) {
        self.configs.entry(id.clone()).or_default();
        self.channels.insert(id, channel);
    }

    /// Allocate a new file-style channel ID, register the channel, and return the ID.
    pub fn register_new(&mut self, channel: Box<dyn Channel>) -> ChannelId {
        let id = format!("file{}", self.next_id);
        self.next_id += 1;
        self.register(id.clone(), channel);
        id
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Box<dyn Channel>> {
        self.channels.get_mut(id)
    }

    pub fn close(&mut self, id: &str) -> io::Result<()> {
        self.configs.remove(id);
        match self.channels.remove(id) {
            Some(ch) => ch.close(),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("can not find channel named \"{}\"", id),
            )),
        }
    }

    pub fn channel_names(&self) -> Vec<&str> {
        self.channels.keys().map(|s| s.as_str()).collect()
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

    /// Replace stdin with a custom channel (for embedders / testing).
    pub fn set_stdin(&mut self, channel: Box<dyn Channel>) {
        self.channels.insert("stdin".to_string(), channel);
    }

    /// Replace stdout with a custom channel (for embedders / testing).
    pub fn set_stdout(&mut self, channel: Box<dyn Channel>) {
        self.channels.insert("stdout".to_string(), channel);
    }

    /// Replace stderr with a custom channel (for embedders / testing).
    pub fn set_stderr(&mut self, channel: Box<dyn Channel>) {
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
}

impl FileChannel {
    fn new_read(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::Read(BufReader::new(f)), at_eof: false }
    }
    fn new_write(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::Write(f), at_eof: false }
    }
    fn new_readwrite(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::ReadWrite(BufReader::new(f)), at_eof: false }
    }
}

impl Channel for FileChannel {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = match &mut self.mode {
            FileMode::Read(r) => r.read(buf)?,
            FileMode::ReadWrite(r) => r.read(buf)?,
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
            FileMode::ReadWrite(r) => r.read_line(&mut line)?,
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
            FileMode::ReadWrite(r) => { r.read_to_string(&mut s)?; }
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
            FileMode::ReadWrite(r) => r.get_mut().write(data),
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
        match &mut self.mode {
            FileMode::Read(r) => r.seek(whence),
            FileMode::Write(f) => f.seek(whence),
            FileMode::ReadWrite(r) => r.seek(whence),
        }
    }
    fn tell(&mut self) -> io::Result<u64> {
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
        channel_write_str(ch.as_mut(), "captured!\n").unwrap();
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
