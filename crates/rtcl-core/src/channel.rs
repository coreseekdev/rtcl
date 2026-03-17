//! Channel abstraction for Tcl I/O.
//!
//! Mirrors jimtcl's `Jim_Channel` — each open file/pipe/socket is
//! identified by a string handle (e.g. "file3", "stdin") and provides
//! a uniform read/write/seek/close interface.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Seek, Write};

/// A unique channel identifier (e.g. "file3", "stdin").
pub type ChannelId = String;

/// Trait object for an open channel.
pub(crate) trait Channel: Send {
    fn read_chars(&mut self, count: usize) -> io::Result<String>;
    fn read_line(&mut self) -> io::Result<Option<String>>;
    fn read_all(&mut self) -> io::Result<String>;
    fn write_str(&mut self, s: &str) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
    fn seek(&mut self, offset: i64, whence: io::SeekFrom) -> io::Result<u64>;
    fn tell(&mut self) -> io::Result<u64>;
    fn eof(&self) -> bool;
    fn close(self: Box<Self>) -> io::Result<()>;
    #[allow(dead_code)]
    fn is_readable(&self) -> bool;
    #[allow(dead_code)]
    fn is_writable(&self) -> bool;
}

/// The channel table — maps handle strings to open channels.
pub(crate) struct ChannelTable {
    channels: HashMap<ChannelId, Box<dyn Channel>>,
    next_id: u32,
}

impl ChannelTable {
    pub fn new() -> Self {
        let mut table = ChannelTable {
            channels: HashMap::new(),
            next_id: 1,
        };
        // Register stdin/stdout/stderr
        table.channels.insert("stdin".to_string(), Box::new(StdinChannel::new()));
        table.channels.insert("stdout".to_string(), Box::new(StdoutChannel));
        table.channels.insert("stderr".to_string(), Box::new(StderrChannel));
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
                let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
                Box::new(FileChannel::new_readwrite(f)) as Box<dyn Channel>
            }
            "a+" => {
                let f = std::fs::OpenOptions::new().read(true).append(true).create(true).open(path)?;
                Box::new(FileChannel::new_readwrite(f)) as Box<dyn Channel>
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("bad access mode \"{}\"", mode))),
        };
        let id = format!("file{}", self.next_id);
        self.next_id += 1;
        self.channels.insert(id.clone(), file);
        Ok(id)
    }

    /// Register a channel with a specific ID (used for pipe channels from exec).
    pub fn register(&mut self, id: ChannelId, channel: Box<dyn Channel>) {
        self.channels.insert(id, channel);
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Box<dyn Channel>> {
        self.channels.get_mut(id)
    }

    pub fn close(&mut self, id: &str) -> io::Result<()> {
        match self.channels.remove(id) {
            Some(ch) => ch.close(),
            None => Err(io::Error::new(io::ErrorKind::NotFound, format!("can not find channel named \"{}\"", id))),
        }
    }

    #[allow(dead_code)]
    pub fn channel_names(&self) -> Vec<&str> {
        self.channels.keys().map(|s| s.as_str()).collect()
    }
}

// ── Stdin channel ──────────────────────────────────────────────────────

struct StdinChannel {
    reader: BufReader<io::Stdin>,
    at_eof: bool,
}

impl StdinChannel {
    fn new() -> Self {
        StdinChannel {
            reader: BufReader::new(io::stdin()),
            at_eof: false,
        }
    }
}

impl Channel for StdinChannel {
    fn read_chars(&mut self, count: usize) -> io::Result<String> {
        let mut buf = vec![0u8; count];
        let n = self.reader.read(&mut buf)?;
        if n == 0 { self.at_eof = true; }
        Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 { self.at_eof = true; return Ok(None); }
        if line.ends_with('\n') { line.pop(); }
        if line.ends_with('\r') { line.pop(); }
        Ok(Some(line))
    }
    fn read_all(&mut self) -> io::Result<String> {
        let mut s = String::new();
        self.reader.read_to_string(&mut s)?;
        self.at_eof = true;
        Ok(s)
    }
    fn write_str(&mut self, _s: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdin\" wasn't opened for writing"))
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
    fn seek(&mut self, _offset: i64, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdin is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdin is not seekable"))
    }
    fn eof(&self) -> bool { self.at_eof }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { true }
    fn is_writable(&self) -> bool { false }
}

// ── Stdout channel ─────────────────────────────────────────────────────

struct StdoutChannel;

impl Channel for StdoutChannel {
    fn read_chars(&mut self, _count: usize) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }
    fn write_str(&mut self, s: &str) -> io::Result<()> {
        print!("{}", s);
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
    fn seek(&mut self, _offset: i64, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdout is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdout is not seekable"))
    }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
}

// ── Stderr channel ─────────────────────────────────────────────────────

struct StderrChannel;

impl Channel for StderrChannel {
    fn read_chars(&mut self, _count: usize) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stderr\" wasn't opened for reading"))
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stderr\" wasn't opened for reading"))
    }
    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stderr\" wasn't opened for reading"))
    }
    fn write_str(&mut self, s: &str) -> io::Result<()> {
        eprint!("{}", s);
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
    fn seek(&mut self, _offset: i64, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stderr is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stderr is not seekable"))
    }
    fn eof(&self) -> bool { false }
    fn close(self: Box<Self>) -> io::Result<()> { Ok(()) }
    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
}

// ── File channel ───────────────────────────────────────────────────────

enum FileMode {
    Read(BufReader<std::fs::File>),
    Write(std::fs::File),
    ReadWrite(std::fs::File),
}

struct FileChannel {
    mode: FileMode,
    at_eof: bool,
    #[allow(dead_code)]
    readable: bool,
    #[allow(dead_code)]
    writable: bool,
}

impl FileChannel {
    fn new_read(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::Read(BufReader::new(f)), at_eof: false, readable: true, writable: false }
    }
    fn new_write(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::Write(f), at_eof: false, readable: false, writable: true }
    }
    fn new_readwrite(f: std::fs::File) -> Self {
        FileChannel { mode: FileMode::ReadWrite(f), at_eof: false, readable: true, writable: true }
    }
}

impl Channel for FileChannel {
    fn read_chars(&mut self, count: usize) -> io::Result<String> {
        let mut buf = vec![0u8; count];
        let n = match &mut self.mode {
            FileMode::Read(r) => r.read(&mut buf)?,
            FileMode::ReadWrite(f) => f.read(&mut buf)?,
            FileMode::Write(_) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel wasn't opened for reading")),
        };
        if n == 0 { self.at_eof = true; }
        Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        let n = match &mut self.mode {
            FileMode::Read(r) => r.read_line(&mut line)?,
            FileMode::ReadWrite(f) => {
                let mut br = BufReader::new(f);
                br.read_line(&mut line)?
            }
            FileMode::Write(_) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel wasn't opened for reading")),
        };
        if n == 0 { self.at_eof = true; return Ok(None); }
        if line.ends_with('\n') { line.pop(); }
        if line.ends_with('\r') { line.pop(); }
        Ok(Some(line))
    }
    fn read_all(&mut self) -> io::Result<String> {
        let mut s = String::new();
        match &mut self.mode {
            FileMode::Read(r) => { r.read_to_string(&mut s)?; }
            FileMode::ReadWrite(f) => { f.read_to_string(&mut s)?; }
            FileMode::Write(_) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel wasn't opened for reading")),
        }
        self.at_eof = true;
        Ok(s)
    }
    fn write_str(&mut self, s: &str) -> io::Result<()> {
        match &mut self.mode {
            FileMode::Write(f) => f.write_all(s.as_bytes()),
            FileMode::ReadWrite(f) => f.write_all(s.as_bytes()),
            FileMode::Read(_) => Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel wasn't opened for writing")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match &mut self.mode {
            FileMode::Write(f) => f.flush(),
            FileMode::ReadWrite(f) => f.flush(),
            FileMode::Read(_) => Ok(()),
        }
    }
    fn seek(&mut self, _offset: i64, whence: io::SeekFrom) -> io::Result<u64> {
        match &mut self.mode {
            FileMode::Read(r) => r.seek(whence),
            FileMode::Write(f) => f.seek(whence),
            FileMode::ReadWrite(f) => f.seek(whence),
        }
    }
    fn tell(&mut self) -> io::Result<u64> {
        self.seek(0, io::SeekFrom::Current(0))
    }
    fn eof(&self) -> bool { self.at_eof }
    fn close(self: Box<Self>) -> io::Result<()> {
        // File is closed when dropped
        Ok(())
    }
    fn is_readable(&self) -> bool { self.readable }
    fn is_writable(&self) -> bool { self.writable }
}

// ── Pipe channel (for exec / open |cmd) ────────────────────────────────

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
    fn read_chars(&mut self, count: usize) -> io::Result<String> {
        if let Some(r) = &mut self.reader {
            let mut buf = vec![0u8; count];
            let n = r.read(&mut buf)?;
            if n == 0 { self.at_eof = true; }
            Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe channel not readable"))
        }
    }
    fn read_line(&mut self) -> io::Result<Option<String>> {
        if let Some(r) = &mut self.reader {
            let mut line = String::new();
            let n = r.read_line(&mut line)?;
            if n == 0 { self.at_eof = true; return Ok(None); }
            if line.ends_with('\n') { line.pop(); }
            if line.ends_with('\r') { line.pop(); }
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
    fn write_str(&mut self, s: &str) -> io::Result<()> {
        if let Some(w) = &mut self.writer {
            w.write_all(s.as_bytes())
        } else {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "pipe channel not writable"))
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(w) = &mut self.writer {
            w.flush()
        } else {
            Ok(())
        }
    }
    fn seek(&mut self, _offset: i64, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "pipe channel is not seekable"))
    }
    fn tell(&mut self) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "pipe channel is not seekable"))
    }
    fn eof(&self) -> bool { self.at_eof }
    fn close(mut self: Box<Self>) -> io::Result<()> {
        // Drop stdin to let child finish
        drop(self.writer.take());
        drop(self.reader.take());
        let _ = self.child.wait();
        Ok(())
    }
    fn is_readable(&self) -> bool { self.reader.is_some() }
    fn is_writable(&self) -> bool { self.writer.is_some() }
}
