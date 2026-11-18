//! 进程间管道（内存有界 SPSC ring buffer）——rtcl 运行时原语。
//!
//! 语义严格对齐 POSIX 的 pipe(2)/read(2)/write(2) 子集，不自创：
//! - `read`/`write` 返回 `Ok(n)`：**短读/短写正常**，调用方循环；
//! - `Ok(0)`（仅读）= EOF（所有写端关闭且缓冲排空）；
//! - 非阻塞：`Err(WouldBlock)`（POSIX `EAGAIN/EWOULDBLOCK`）；
//! - 写端向已关闭读端写：`Err(BrokenPipe)`（POSIX `EPIPE`；不投递 SIGPIPE）；
//! - `EINTR` 由本实现内部重试，**永不暴露**（与 Tcl 通道一致）；
//! - 容量有界（默认 64 KiB，对齐 Linux 管道默认值），写满背压；
//! - **仅内存**：不落盘、不缓冲无限数据。
//!
//! 阻塞版（`*_blocking`）只是“`WouldBlock` → condvar 等待”的调用方策略；
//! 非阻塞原语供事件循环（fileevent 同族）直接使用。

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

/// POSIX errno 的投影（不暴露完整 errno；映射表见模块文档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoError {
    /// EAGAIN/EWOULDBLOCK：当前不可读写（非阻塞模式）。
    WouldBlock,
    /// EPIPE：读端已关闭（写端视角）。
    BrokenPipe,
    /// 其它（EIO/EBADF…）；`EINTR` 已在实现内重试，不会出现。
    Other(String),
}

pub type IoResult<T> = Result<T, IoError>;

/// 管道容量（`ROGO_PIPE_LIMIT`，默认 64 KiB = Linux pipe 默认）。
pub fn default_capacity() -> usize {
    std::env::var("ROGO_PIPE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64 * 1024)
}

struct State {
    buf: VecDeque<u8>,
    cap: usize,
    write_closed: bool,
    read_closed: bool,
}

pub struct Pipe {
    state: Mutex<State>,
    readable: Condvar,  // 读端等待：有数据 / EOF
    writable: Condvar,  // 写端等待：有空间 / BROKEN
}

pub struct PipeReader {
    pipe: std::sync::Arc<Pipe>,
    owner: bool,
}
pub struct PipeWriter {
    pipe: std::sync::Arc<Pipe>,
    owner: bool,
}

/// 创建管道；`cap` = 内核管道缓冲的对应物（默认 64 KiB）。
pub fn pipe(cap: usize) -> (PipeReader, PipeWriter) {
    let p = std::sync::Arc::new(Pipe {
        state: Mutex::new(State {
            buf: VecDeque::with_capacity(cap.max(1)),
            cap: cap.max(1),
            write_closed: false,
            read_closed: false,
        }),
        readable: Condvar::new(),
        writable: Condvar::new(),
    });
    (
        PipeReader { pipe: p.clone(), owner: true },
        PipeWriter { pipe: p, owner: true },
    )
}

impl PipeWriter {
    /// 简化版 `dup(2)`：共享同一管道（drop 任一 handle 即关闭写端；仅在
    /// "写完结果即关"的短命场景使用）。
    pub fn try_clone(&self) -> PipeWriter {
        PipeWriter { pipe: self.pipe.clone(), owner: false }
    }

    /// 非阻塞写：写入 0..=n 字节（短写正常）；满或无空间 → WouldBlock。
    pub fn try_write(&self, data: &[u8]) -> IoResult<usize> {
        let mut st = self.pipe.state.lock().map_err(|e| IoError::Other(e.to_string()))?;
        if st.read_closed {
            return Err(IoError::BrokenPipe);
        }
        if data.is_empty() {
            return Ok(0); // POSIX: 零长度写返回 0
        }
        let space = st.cap.saturating_sub(st.buf.len());
        if space == 0 {
            return Err(IoError::WouldBlock);
        }
        let n = space.min(data.len());
        st.buf.extend(data[..n].iter().copied());
        drop(st);
        self.pipe.readable.notify_all();
        Ok(n)
    }

    /// 阻塞写（调用方选择阻塞时用；内部把 WouldBlock 变成 condvar 等待）。
    pub fn write_blocking(&self, data: &[u8]) -> IoResult<usize> {
        let mut off = 0;
        while off < data.len() {
            let mut st = self.pipe.state.lock().map_err(|e| IoError::Other(e.to_string()))?;
            loop {
                if st.read_closed {
                    return Err(IoError::BrokenPipe);
                }
                if st.buf.len() < st.cap {
                    break;
                }
                st = self
                    .pipe
                    .writable
                    .wait(st)
                    .map_err(|e| IoError::Other(e.to_string()))?;
            }
            let space = st.cap - st.buf.len();
            let n = space.min(data.len() - off);
            st.buf.extend(data[off..off + n].iter().copied());
            off += n;
            drop(st);
            self.pipe.readable.notify_all();
        }
        Ok(off)
    }

    /// 关闭写端（POSIX close(write end)）→ 读端在读空后见 EOF。
    pub fn close(&self) {
        if let Ok(mut st) = self.pipe.state.lock() {
            st.write_closed = true;
        }
        self.pipe.readable.notify_all();
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        if self.owner {
            self.close();
        }
    }
}

impl PipeReader {
    /// 简化版 `dup(2)`：非拥有句柄（drop 不关闭管道）。
    pub fn try_clone(&self) -> PipeReader {
        PipeReader { pipe: self.pipe.clone(), owner: false }
    }

    /// 非阻塞读：`Ok(0)` = EOF；空且写端未关 → WouldBlock。
    pub fn try_read(&self, out: &mut [u8]) -> IoResult<usize> {
        let mut st = self.pipe.state.lock().map_err(|e| IoError::Other(e.to_string()))?;
        if out.is_empty() {
            return Ok(0); // POSIX: 零长度读返回 0（非 EOF 语义，调用方用非空缓冲区分）
        }
        if st.buf.is_empty() {
            if st.write_closed {
                return Ok(0); // EOF
            }
            return Err(IoError::WouldBlock);
        }
        let n = out.len().min(st.buf.len());
        for (i, b) in st.buf.drain(..n).enumerate() {
            out[i] = b;
        }
        drop(st);
        self.pipe.writable.notify_all();
        Ok(n)
    }

    /// 阻塞读：读到 ≥1 字节或 EOF 才返回（调用方选择阻塞时用）。
    pub fn read_blocking(&self, out: &mut [u8]) -> IoResult<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut st = self.pipe.state.lock().map_err(|e| IoError::Other(e.to_string()))?;
        while st.buf.is_empty() && !st.write_closed {
            st = self
                .pipe
                .readable
                .wait(st)
                .map_err(|e| IoError::Other(e.to_string()))?;
        }
        if st.buf.is_empty() {
            return Ok(0); // EOF
        }
        let n = out.len().min(st.buf.len());
        for (i, b) in st.buf.drain(..n).enumerate() {
            out[i] = b;
        }
        drop(st);
        self.pipe.writable.notify_all();
        Ok(n)
    }

    /// 关闭读端（POSIX close(read end)）→ 写端后续写入见 BrokenPipe。
    pub fn close(&self) {
        if let Ok(mut st) = self.pipe.state.lock() {
            st.read_closed = true;
        }
        self.pipe.writable.notify_all();
        self.pipe.readable.notify_all();
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        if self.owner {
            self.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn eof_after_writer_close() {
        let (r, w) = pipe(64);
        w.write_blocking(b"abc").unwrap();
        w.close();
        let mut buf = [0u8; 16];
        assert_eq!(r.read_blocking(&mut buf).unwrap(), 3);
        assert_eq!(&buf[..3], b"abc");
        assert_eq!(r.read_blocking(&mut buf).unwrap(), 0, "empty + writer closed = EOF");
    }

    #[test]
    fn broken_pipe_on_reader_close() {
        let (r, w) = pipe(64);
        r.close();
        assert_eq!(w.try_write(b"x"), Err(IoError::BrokenPipe));
    }

    #[test]
    fn backpressure_reports_would_block() {
        let (r, w) = pipe(4);
        assert_eq!(w.try_write(b"abcd").unwrap(), 4);
        assert_eq!(w.try_write(b"e"), Err(IoError::WouldBlock));
        let mut buf = [0u8; 2];
        assert_eq!(r.try_read(&mut buf).unwrap(), 2);
        assert_eq!(w.try_write(b"e").unwrap(), 1);
    }

    #[test]
    fn blocking_writer_wakes_on_read() {
        let (r, w) = pipe(4);
        let (tx, rx) = std::sync::mpsc::channel();
        let t = std::thread::spawn(move || {
            w.write_blocking(b"0123456789").unwrap();
            tx.send(()).unwrap();
            // 线程结束时 drop w → 关闭写端 → 读端见 EOF
        });
        std::thread::sleep(Duration::from_millis(20));
        assert!(rx.try_recv().is_err(), "writer must be blocked (backpressure)");
        let mut got = Vec::new();
        let mut buf = [0u8; 8];
        loop {
            let n = r.read_blocking(&mut buf).unwrap();
            if n == 0 {
                break; // EOF（写端线程结束后关闭）
            }
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(got, b"0123456789");
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        t.join().unwrap();
    }
}
