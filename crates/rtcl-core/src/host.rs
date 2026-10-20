//! 宿主（OS）服务 trait 层。
//!
//! 解释器核心不直接触碰具体平台：文件系统、控制台、环境变量、时钟、
//! 进程执行等 OS 能力被建模为一组细粒度 trait，由宿主组合提供——
//!
//! - **std 原生构建**：`NativeConsole` 等默认实现直连 `std::io`/OS。
//! - **wasm（浏览器）**：没有真实 OS，宿主经 `Interp::set_console` 等
//!   注入口把 JS 实现挂进来（见 rtcl-wasm 的 `JsConsole`）——浏览器里
//!   缺失的 OS 服务由 JS 承担。
//!
//! 迁移路线：`io` 特性下的通道栈已有自己的 `Channel` trait（这是
//! "组合为 OS 服务"的第一个成员）；`file`、`env`、`clock`、`exec` 等
//! 特性目前仍直接调用 `std::*`，后续逐个收敛到本层的 `HostFs`、
//! `HostEnv`、`HostClock`、`HostExec` 预留接口上。每个命令族只依赖
//! 它需要的那个 trait，解释器因此可以按 feature 组合出不同大小的
//! "OS 表面"。

/// 控制台服务：`puts` 在无 `io` 特性构建下的 stdout/stderr 落点。
///
/// 原生构建默认使用 [`NativeConsole`]；wasm 宿主用
/// `Interp::set_console` 注入 JS 实现（每个 `puts` 恰好触发一次回调，
/// 含行尾换行）。
pub trait HostConsole: Send {
    /// 写一行到 stdout（`s` 已含 puts 补的换行，除非 -nonewline）。
    ///
    /// 返回 `Err(消息)` 时 `puts` 报 `error writing "stdout": 消息`。
    fn write_stdout(&self, s: &str) -> Result<(), String> {
        let _ = s;
        Ok(())
    }
    /// 写一行到 stderr（语义同 [`HostConsole::write_stdout`]）。
    fn write_stderr(&self, s: &str) -> Result<(), String> {
        let _ = s;
        Ok(())
    }
}

/// std 原生控制台：直连进程的 stdout/stderr，写后即刷。
#[cfg(feature = "std")]
pub struct NativeConsole;

#[cfg(feature = "std")]
impl HostConsole for NativeConsole {
    fn write_stdout(&self, s: &str) -> Result<(), String> {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        out.write_all(s.as_bytes()).and_then(|_| out.flush()).map_err(|e| e.to_string())
    }
    fn write_stderr(&self, s: &str) -> Result<(), String> {
        use std::io::Write;
        let mut out = std::io::stderr().lock();
        out.write_all(s.as_bytes()).and_then(|_| out.flush()).map_err(|e| e.to_string())
    }
}

/// no-std 占位控制台：所有写入静默丢弃（no-std 下 puts 本身是 no-op）。
pub struct NullConsole;

impl HostConsole for NullConsole {}

// ---------------------------------------------------------------------------
// 预留接口：以下 trait 是后续把 file/env/clock/exec 特性从 `std::*` 直调
// 收敛为宿主服务的接缝；尚未接线（命令仍走 `std::*`），API 以实际迁移
// 时为准。
// ---------------------------------------------------------------------------

/// 文件系统服务：`file`、`open`/`close`/`read`/`gets`/`source`/`glob`
/// 等 fs 侧能力的宿主接缝。浏览器（wasm）下由 JS 提供（例如挂载的
/// 虚拟 FS、OPFS 或 localStorage 后端）。
///
/// `open` 返回统一的 [`HostFd`]；exec 的重定向不解析路径，只把 fd 放进
/// 子进程 fd 表——与管道/内存流完全同构。
pub trait HostFs: Send {
    fn open(&self, path: &str, mode: FdMode) -> Result<Box<dyn HostFd>, String>;
}

/// 环境变量服务：`$env` 数组与 `info exists env(...)` 的宿主接缝。
pub trait HostEnv: Send {}

/// 时钟服务：`after`/`vwait` 计时与 `time`/`timerate` 测量的宿主接缝。
///
/// 原生构建默认使用 [`NativeClock`]；wasm 宿主经 `Interp::set_clock`
/// 注入 JS 实现（`Date.now`）。浏览器无法同步睡眠：wasm 宿主的
/// [`HostClock::sleep_ms`] 返回 `Err`，`after ms`（无脚本的同步睡眠）
/// 与 `vwait` 的阻塞等待分支因此报 Tcl 错误——浏览器脚本必须用
/// `after ms script` 的事件驱动形式（见 docs/UI-DSL.md §4）。
pub trait HostClock: Send {
    /// 单调毫秒（允许亚毫秒精度），起点宿主自定义、仅要求自洽。
    fn now_ms(&self) -> f64;
    /// 同步睡眠。浏览器宿主返回 `Err`（Worker 内阻塞会饿死事件投递）。
    fn sleep_ms(&self, ms: u64) -> Result<(), String>;
}

/// std 原生时钟：`Instant` 计时 + `thread::sleep`。
#[cfg(feature = "std")]
pub struct NativeClock;

#[cfg(feature = "std")]
impl HostClock for NativeClock {
    fn now_ms(&self) -> f64 {
        static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }

    fn sleep_ms(&self, ms: u64) -> Result<(), String> {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(())
    }
}

/// POSIX 错误的投影：`EINTR` 由宿主内部重试、**不暴露**；短读/短写正常；
/// `read` 返回 0 即 EOF。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecIoError {
    /// EAGAIN/EWOULDBLOCK（本 trait 的 IO 一律非阻塞）。
    WouldBlock,
    /// EPIPE：读端已关闭（写端视角）；不投递 SIGPIPE。
    BrokenPipe,
    /// 其它（EIO/EBADF…）。
    Other(String),
}

/// 父进程侧的 fd 端点：统一的字节流/文件视图。
///
/// 进程没有 `stdin/stdout/stderr` 专用方法——0/1/2 只是 fd 约定。
/// `HostFd` 同时覆盖管道、文件、内存流；WASI 的 `fd_read/fd_write/fd_close`
/// 直接映射到这一层。
pub trait HostFd: Send + 'static {
    /// 原生宿主用它 downcast 到具体 fd（管道/文件）；抽象层不依赖 OS。
    fn as_any(&self) -> &dyn std::any::Any;
    /// 0 = EOF。方向不符 → `Other`。
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ExecIoError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, ExecIoError>;
    /// 关闭父侧端点（子进程见 EOF / 写端 EPIPE）。
    fn close(&mut self) -> Result<(), String>;
    /// 等待可读/可写/超时（避免忙等）。
    fn poll(&mut self, read: bool, write: bool, timeout_ms: u64) -> Result<(), String>;
    /// 复制端点（`2>@1`、多 fd 指向同一管道）。默认不支持。
    fn dup(&self) -> Result<Box<dyn HostFd>, String> {
        Err("fd does not support dup".to_string())
    }
}

/// 文件打开模式（宿主按 p-root/OPFS 解析路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdMode {
    Read,
    Write,
    Append,
}

/// 一个待执行命令：`argv[0]` 含程序名；PATH 解析由宿主负责。
/// fd 表不在这里描述——`HostExec::spawn` 接收构建好的 fd 视图。
pub struct ExecCommand {
    pub prog: String,
    pub argv: Vec<String>,
}

impl ExecCommand {
    pub fn new(prog: impl Into<String>, argv: Vec<String>) -> Self {
        Self { prog: prog.into(), argv }
    }
}

/// `/dev/null` 的统一 fd 实现：读 EOF、写丢弃。
pub struct NullFd;

impl HostFd for NullFd {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize, ExecIoError> {
        Ok(0)
    }
    fn write(&mut self, buf: &[u8]) -> Result<usize, ExecIoError> {
        Ok(buf.len())
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self, _read: bool, _write: bool, _timeout_ms: u64) -> Result<(), String> {
        Ok(())
    }
    fn dup(&self) -> Result<Box<dyn HostFd>, String> {
        Ok(Box::new(NullFd))
    }
}

/// 只读内存字节流（heredoc / `<<`）：统一 fd 视图，不是 exec 特例。
pub struct BytesFd {
    data: Vec<u8>,
    off: usize,
}

impl BytesFd {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, off: 0 }
    }

    /// 原始字节（native runner 用管道把内存流接到子进程 fd）。
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

impl HostFd for BytesFd {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ExecIoError> {
        if self.off >= self.data.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.data.len() - self.off);
        buf[..n].copy_from_slice(&self.data[self.off..self.off + n]);
        self.off += n;
        Ok(n)
    }
    fn write(&mut self, _buf: &[u8]) -> Result<usize, ExecIoError> {
        Err(ExecIoError::Other("read-only fd".into()))
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self, _read: bool, _write: bool, _timeout_ms: u64) -> Result<(), String> {
        Ok(())
    }
    fn dup(&self) -> Result<Box<dyn HostFd>, String> {
        Ok(Box::new(BytesFd {
            data: self.data.clone(),
            off: self.off,
        }))
    }
}

/// 一次性聚合结果（[`exec_collect`] 的返回）。
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: i32,
}

/// 作业句柄：不透明、在句柄生命周期内稳定。
///
/// native 通常是 runner 子进程 PID，web 是合成 ID；调用方**不得**假设 OS
/// PID/信号语义（见 `docs/JOB-CONTROL.md`）。
pub type JobId = u64;

/// 作业生命周期状态（native 与 web 同一套，无抢占式分支）。
///
/// 规范（2026-10-20）：
/// - `stop` 幂等：`Running → StopRequested`；guest 在下一个 checkpoint 才进入
///   `Stopped`。没有 checkpoint 的纯计算会一直停在 `StopRequested`。
/// - `resume` 幂等：`StopRequested/Stopped → Running`；终态上为 no-op。
/// - `kill` 幂等：立即进入 `Killed`（硬杀，不保证 guest 清理）。
/// - `Exited` / `Killed` 是终态；`wait` 对 `Killed` 返回 137（128+SIGKILL 约定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    StopRequested,
    Stopped,
    Exited(i32),
    Killed,
}

impl JobState {
    /// 终态：不会再生变化。
    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Exited(_) | JobState::Killed)
    }

    /// Tcl/`job list` 使用的稳定标签。
    pub fn label(self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::StopRequested => "stopping",
            JobState::Stopped => "stopped",
            JobState::Exited(_) => "exited",
            JobState::Killed => "killed",
        }
    }

    /// 终态退出码；非终态为 `None`。`Killed` 固定 137。
    pub fn exit_code(self) -> Option<i32> {
        match self {
            JobState::Exited(code) => Some(code),
            JobState::Killed => Some(137),
            JobState::Running | JobState::StopRequested | JobState::Stopped => None,
        }
    }
}

/// 运行中的作业句柄：生命周期，不含 fd 表。
///
/// 子进程 fd 表在 [`HostExec::spawn`] 时由调用方构建；父进程需要的端点
/// （捕获/写入）由调用方自己保留，不挂在进程句柄上。fd IO 恒为**非阻塞**
/// （`WouldBlock`），是否阻塞由 [`HostFd::poll`] 决定；进程状态由
/// [`HostProcess::poll`] 等待。管道实现应为有界内存（背压），**不落盘**。
///
/// 挂起是**协作式**的：只有 guest 到达宿主 import/checkpoint 时才会从
/// `StopRequested` 进入 `Stopped`。native 不得用 SIGSTOP、web 不得用
/// `Worker.terminate()` 冒充挂起（terminate 只能是 `kill`）。
pub trait HostProcess: Send {
    /// 稳定、不透明的作业句柄。
    fn id(&self) -> JobId;
    /// 非阻塞读取当前状态；终态可重复读取。
    fn state(&mut self) -> Result<JobState, String>;
    /// 请求协作式挂起；立即返回。幂等。
    fn stop(&mut self) -> Result<(), String>;
    /// 取消挂起 / 恢复运行；立即返回。幂等；终态上 no-op。
    fn resume(&mut self) -> Result<(), String>;
    /// 非阻塞：`Some(code)` = 已到终态；`None` = 仍在运行/挂起。
    /// `Killed` 返回 `Some(137)`。
    fn wait(&mut self) -> Result<Option<i32>, String>;
    /// 请求**硬杀**（不等待；随后 fd 写 → `BrokenPipe`，读端排空后 EOF）。
    /// 状态进入 `Killed`。
    fn kill(&mut self) -> Result<(), String>;
    /// 等待**状态变化**（stop/resume/stopped/exited/killed）或超时。
    /// 父侧 fd 的就绪用 [`HostFd::poll`]；两者可组合使用。
    fn poll(&mut self, timeout_ms: u64) -> Result<(), String>;
}

/// 进程执行服务：`spawn` 是必需的；`pipe` 为管道连接提供子进程侧端点。
/// 一次性 `exec` 是解释器侧 helper（[`exec_collect`]），不属于宿主 ABI。
///
/// 返回的 [`HostProcess`] 同时是作业句柄；`stop/resume/state/kill` 的语义见
/// 该 trait 与 `docs/JOB-CONTROL.md`。
///
/// 范围（2026-10-19 裁决）：仅 wasm 命令（WASI p1 命令形态）；native binary
/// 不在本机制内（将来若需要，另设显式能力的 exec 通道）。
pub trait HostExec: Send {
    /// 用**完整的 fd 表**启动命令：下标即子进程 fd 号，`None` = 该 fd 关闭。
    /// 宿主接管这些端点；调用方保留自己需要的另一端（捕获/写入）。
    /// 0/1/2 只是约定；缺失的尾部 fd 视为关闭。
    fn spawn(
        &self,
        cmd: &ExecCommand,
        fds: Vec<Option<Box<dyn HostFd>>>,
        env: &[(String, String)],
    ) -> Result<Box<dyn HostProcess>, String>;

    /// 创建一对端点 `(read, write)`；一端放进子进程 fd 表，另一端由父进程
    /// 保留（捕获/写入），或把两端分别交给管道两端的 job。
    fn pipe(&self) -> Result<(Box<dyn HostFd>, Box<dyn HostFd>), String> {
        Err("host does not support exec pipes".to_string())
    }
}

/// 阻塞聚合 helper：构建 fd 0/1/2（stdin 用内存流或 NullFd，stdout/stderr
/// 用两条管道），收齐输出与退出码；超时 → kill 并报错。非零退出**不是**
/// `Err`（由 Tcl 层决定是否按 `CHILDSTATUS` 抛错）。
#[cfg(feature = "std")]
pub fn exec_collect(
    host: &dyn HostExec,
    cmd: ExecCommand,
    stdin: &[u8],
    env: &[(String, String)],
    timeout_ms: u64,
) -> Result<ExecOutput, String> {
    let (stdin_r, mut stdin_w): (Box<dyn HostFd>, Option<Box<dyn HostFd>>) =
        if stdin.is_empty() {
            (Box::new(NullFd), None)
        } else {
            let (r, w) = host.pipe()?;
            (r, Some(w))
        };
    let (mut out_r, out_w) = host.pipe()?;
    let (mut err_r, err_w) = host.pipe()?;
    let fds = vec![Some(stdin_r), Some(out_w), Some(err_w)];
    let mut p = host.spawn(&cmd, fds, env)?;

    let start = std::time::Instant::now();
    let mut in_off = 0usize;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut out_eof = false;
    let mut err_eof = false;
    let mut buf = [0u8; 64 * 1024];

    loop {
        if let Some(w) = stdin_w.as_mut() {
            while in_off < stdin.len() {
                match w.write(&stdin[in_off..]) {
                    Ok(0) => break,
                    Ok(n) => in_off += n,
                    Err(ExecIoError::WouldBlock) => break,
                    Err(ExecIoError::BrokenPipe) => in_off = stdin.len(),
                    Err(ExecIoError::Other(e)) => return Err(format!("stdin: {e}")),
                }
            }
            if in_off >= stdin.len() {
                w.close()?;
                stdin_w = None;
            }
        }
        if !out_eof {
            out_eof = read_available(out_r.as_mut(), &mut out, &mut buf)?;
        }
        if !err_eof {
            err_eof = read_available(err_r.as_mut(), &mut err, &mut buf)?;
        }
        if let Some(code) = p.wait()? {
            // 退出后把管道剩余字节排空（忽略关闭/方向错误）。
            let _ = read_available(out_r.as_mut(), &mut out, &mut buf);
            let _ = read_available(err_r.as_mut(), &mut err, &mut buf);
            return Ok(ExecOutput { stdout: out, stderr: err, code });
        }
        if timeout_ms > 0 && start.elapsed().as_millis() as u64 >= timeout_ms {
            let _ = p.kill();
            return Err(format!("exec: timeout after {timeout_ms}ms"));
        }
        // fd 就绪 / 状态变化；10ms 粒度足够且不忙等。
        if let Some(w) = stdin_w.as_mut() {
            w.poll(false, true, 0)?;
        }
        if !out_eof {
            out_r.poll(true, false, 0)?;
        }
        if !err_eof {
            err_r.poll(true, false, 0)?;
        }
        p.poll(10)?;
    }
}

/// 读一个父侧 fd 直到 `WouldBlock`；返回是否 EOF。
#[cfg(feature = "std")]
fn read_available(f: &mut dyn HostFd, out: &mut Vec<u8>, buf: &mut [u8]) -> Result<bool, String> {
    loop {
        match f.read(buf) {
            Ok(0) => return Ok(true),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(ExecIoError::WouldBlock) => return Ok(false),
            Err(ExecIoError::BrokenPipe) => return Ok(true),
            Err(ExecIoError::Other(e)) => return Err(format!("exec fd: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP 客户端服务（rogo R0 探针，2026-10-18）
// ---------------------------------------------------------------------------

/// HTTP 客户端服务：`http::*` 词表层（`crates/rtcl-core/src/http.tcl`）
/// 在 native 侧的宿主接缝。
///
/// 形态约定：
/// - **阻塞式流**：`request` 返回可逐块读取的响应体；宿主命令在读取循环里
///   以 re-entrant `eval` 驱动 Tcl 回调（`::http::_delta` / `::http::_done`）。
///   Tcl 层面的"异步"始终是回调 + 事件循环——与 wasm（JS fetch +
///   `dispatch_event` 回灌）保持同一事件面，脚本无感。
/// - **无 async/await**：native 实现用 blocking 客户端即可（rtcl-cli 的
///   `net` feature 用 reqwest blocking，tokio 不进入 rtcl 依赖面）。
/// - wasm 构建只携带本 trait（无实现）：JS 桥不经过它，走
///   `__rtcl_http` + `post_event`/`dispatch_event`。
pub trait HostHttp: Send {
    /// 发起请求并返回流式响应；连接/构建失败立即 `Err(消息)`。
    fn request(&self, req: &HttpRequest) -> Result<Box<dyn HttpResponse>, String>;
}

/// 一次 HTTP 请求（最小形状，wire 无关）。
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    /// 请求头（保序）。
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// 总超时（毫秒；0 = 不设）。
    pub timeout_ms: u64,
    /// 是否跟随重定向（默认 true）。
    pub follow_redirects: bool,
}

/// 流式响应：先拿 head（状态/头/final URL），再逐块 [`HttpResponse::read_chunk`]
/// 直到 `Ok(None)`（EOF）。`Err` 即流中止（网络/超时/取消）。
pub trait HttpResponse: Send {
    fn status(&self) -> u16;
    fn final_url(&self) -> String;
    fn header(&self, name: &str) -> Option<String>;
    fn headers(&self) -> Vec<(String, String)>;
    /// 下一块原始字节；`Ok(None)` = 正常结束。
    fn read_chunk(&mut self) -> Result<Option<Vec<u8>>, String>;
}

/// 无网络服务占位（wasm / 未注入宿主）：请求即报错，绝不静默。
pub struct NullHttp;

impl HostHttp for NullHttp {
    fn request(&self, _req: &HttpRequest) -> Result<Box<dyn HttpResponse>, String> {
        Err("http service is not available in this host (native: build rtcl-cli with --features net)".to_string())
    }
}

#[cfg(test)]
mod job_state_tests {
    use super::*;

    #[test]
    fn terminal_and_exit_codes() {
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::StopRequested.is_terminal());
        assert!(!JobState::Stopped.is_terminal());
        assert!(JobState::Exited(0).is_terminal());
        assert!(JobState::Killed.is_terminal());

        assert_eq!(JobState::Exited(7).exit_code(), Some(7));
        assert_eq!(JobState::Killed.exit_code(), Some(137));
        assert_eq!(JobState::Stopped.exit_code(), None);
    }

    #[test]
    fn stable_labels() {
        assert_eq!(JobState::Running.label(), "running");
        assert_eq!(JobState::StopRequested.label(), "stopping");
        assert_eq!(JobState::Stopped.label(), "stopped");
        assert_eq!(JobState::Exited(3).label(), "exited");
        assert_eq!(JobState::Killed.label(), "killed");
    }

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct PipeState {
        buf: VecDeque<u8>,
        closed: bool,
    }

    struct PipeEnd {
        state: Arc<Mutex<PipeState>>,
        writer: bool,
    }

    impl HostFd for PipeEnd {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, ExecIoError> {
            if self.writer {
                return Err(ExecIoError::Other("write-only fd".into()));
            }
            let mut s = self.state.lock().unwrap();
            if s.buf.is_empty() {
                return if s.closed {
                    Ok(0)
                } else {
                    Err(ExecIoError::WouldBlock)
                };
            }
            let n = buf.len().min(s.buf.len());
            for slot in &mut buf[..n] {
                *slot = s.buf.pop_front().unwrap();
            }
            Ok(n)
        }
        fn write(&mut self, buf: &[u8]) -> Result<usize, ExecIoError> {
            if !self.writer {
                return Err(ExecIoError::Other("read-only fd".into()));
            }
            let mut s = self.state.lock().unwrap();
            if s.closed {
                return Err(ExecIoError::BrokenPipe);
            }
            s.buf.extend(buf.iter().copied());
            Ok(buf.len())
        }
        fn close(&mut self) -> Result<(), String> {
            self.state.lock().unwrap().closed = true;
            Ok(())
        }
        fn poll(&mut self, _read: bool, _write: bool, _timeout_ms: u64) -> Result<(), String> {
            Ok(())
        }
        fn dup(&self) -> Result<Box<dyn HostFd>, String> {
            Ok(Box::new(PipeEnd {
                state: self.state.clone(),
                writer: self.writer,
            }))
        }
    }

    fn pipe_pair() -> (PipeEnd, PipeEnd) {
        let state = Arc::new(Mutex::new(PipeState::default()));
        (
            PipeEnd {
                state: state.clone(),
                writer: false,
            },
            PipeEnd { state, writer: true },
        )
    }

    struct MockProcess {
        state: JobState,
    }

    impl HostProcess for MockProcess {
        fn id(&self) -> JobId {
            7
        }
        fn state(&mut self) -> Result<JobState, String> {
            Ok(self.state)
        }
        fn stop(&mut self) -> Result<(), String> {
            if self.state == JobState::Running {
                self.state = JobState::StopRequested;
            }
            Ok(())
        }
        fn resume(&mut self) -> Result<(), String> {
            if matches!(self.state, JobState::StopRequested | JobState::Stopped) {
                self.state = JobState::Running;
            }
            Ok(())
        }
        fn wait(&mut self) -> Result<Option<i32>, String> {
            Ok(self.state.exit_code())
        }
        fn kill(&mut self) -> Result<(), String> {
            self.state = JobState::Killed;
            Ok(())
        }
        fn poll(&mut self, _timeout_ms: u64) -> Result<(), String> {
            Ok(())
        }
    }

    struct MockHost {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        code: i32,
    }

    impl HostExec for MockHost {
        fn spawn(
            &self,
            _cmd: &ExecCommand,
            mut fds: Vec<Option<Box<dyn HostFd>>>,
            _env: &[(String, String)],
        ) -> Result<Box<dyn HostProcess>, String> {
            let Some(mut out) = fds.get_mut(1).and_then(Option::take) else {
                return Err("mock requires fd 1".into());
            };
            out.write(&self.stdout).map_err(|e| format!("{e:?}"))?;
            out.close()?;
            let Some(mut err) = fds.get_mut(2).and_then(Option::take) else {
                return Err("mock requires fd 2".into());
            };
            err.write(&self.stderr).map_err(|e| format!("{e:?}"))?;
            err.close()?;
            Ok(Box::new(MockProcess {
                state: JobState::Exited(self.code),
            }))
        }

        fn pipe(&self) -> Result<(Box<dyn HostFd>, Box<dyn HostFd>), String> {
            let (r, w) = pipe_pair();
            Ok((Box::new(r), Box::new(w)))
        }
    }

    #[test]
    fn null_and_bytes_fd_are_unified() {
        let mut b = BytesFd::new(b"abc".to_vec());
        let mut buf = [0u8; 8];
        assert_eq!(b.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf[..3], b"abc");
        assert_eq!(b.read(&mut buf).unwrap(), 0);

        let mut n = NullFd;
        assert_eq!(n.read(&mut buf).unwrap(), 0);
        assert_eq!(n.write(b"discard").unwrap(), 7);
        assert!(n.dup().is_ok());
    }

    #[test]
    fn exec_collect_uses_fd_0_1_2_only() {
        let host = MockHost {
            stdout: b"out\n".to_vec(),
            stderr: b"err\n".to_vec(),
            code: 3,
        };
        let out = exec_collect(&host, ExecCommand::new("mock", vec![]), b"", &[], 1000).unwrap();
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
        assert_eq!(out.code, 3);
    }
}
