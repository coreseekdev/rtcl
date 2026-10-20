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
pub trait HostFs: Send {}

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
pub trait HostFd: Send {
    /// 0 = EOF。方向不符 → `Other`。
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ExecIoError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, ExecIoError>;
    /// 关闭父侧端点（子进程见 EOF / 写端 EPIPE）。
    fn close(&mut self) -> Result<(), String>;
    /// 等待可读/可写/超时（避免忙等）。
    fn poll(&mut self, read: bool, write: bool, timeout_ms: u64) -> Result<(), String>;
}

/// 管道端在父进程侧的方向；子进程侧方向相反。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeSide {
    /// 父进程读（子进程写）：stdout/stderr 捕获。
    ParentReads,
    /// 父进程写（子进程读）：stdin 注入。
    ParentWrites,
}

/// 文件打开模式（宿主按 p-root/OPFS 解析路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdMode {
    Read,
    Write,
    Append,
}

/// spawn 时构建子进程 fd 表的单条规则；下标即子进程 fd 号。
///
/// 0/1/2 只是约定，3+ 同样支持；缺省时宿主补 `Null`。
/// `Handle`/`Data` 是**子进程侧**端点；`Pipe` 由宿主建管道并在
/// [`HostProcess::fd`] 暴露父侧端点。
pub enum FdSpec {
    /// /dev/null（读 EOF / 写丢弃）。
    Null,
    /// 宿主建管道；父侧端点通过 `HostProcess::fd(n)` 获取。
    Pipe(PipeSide),
    /// 宿主打开文件（native p-root / web OPFS）。
    File { path: String, mode: FdMode },
    /// 子进程读的内存字节流（heredoc / `<<`）。
    Data(Vec<u8>),
    /// 复制子进程另一个 fd（`2>@1`）。
    Dup(u32),
    /// 预先建好的子进程侧端点（管道连接、外部 fd）。
    Handle(Box<dyn HostFd>),
}

/// 一个待执行命令：`argv[0]` 含程序名；PATH 解析由宿主负责。
/// `fds` 是完整 fd 视图，不区分 stdio 特例。
pub struct ExecCommand {
    pub prog: String,
    pub argv: Vec<String>,
    pub fds: Vec<FdSpec>,
}

impl ExecCommand {
    /// 默认 fd 表：0=Null，1/2=父读管道。
    pub fn new(prog: impl Into<String>, argv: Vec<String>) -> Self {
        Self {
            prog: prog.into(),
            argv,
            fds: vec![
                FdSpec::Null,
                FdSpec::Pipe(PipeSide::ParentReads),
                FdSpec::Pipe(PipeSide::ParentReads),
            ],
        }
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

/// 运行中的作业句柄：fd 表 + 生命周期。
///
/// 父进程只持有需要自己读写的端点（`FdSpec::Pipe`）；`File/Data/Handle/Null`
/// 没有父侧端点，`fd(n)` 返回 `None`。fd IO 恒为**非阻塞**
/// （`WouldBlock`），是否阻塞由 [`HostProcess::poll`] / [`HostFd::poll`] 决定。
/// 管道实现应为有界内存（背压），**不落盘**（宿主负责）。
///
/// 挂起是**协作式**的：只有 guest 到达宿主 import/checkpoint 时才会从
/// `StopRequested` 进入 `Stopped`。native 不得用 SIGSTOP、web 不得用
/// `Worker.terminate()` 冒充挂起（terminate 只能是 `kill`）。
pub trait HostProcess: Send {
    /// 稳定、不透明的作业句柄。
    fn id(&self) -> JobId;
    /// 父侧 fd 端点；没有父侧端点 → `None`。
    fn fd(&mut self, fd: u32) -> Option<&mut (dyn HostFd + 'static)>;
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
    /// 等待指定父侧 fd 就绪 / 超时 / **状态变化**（stop/resume/stopped/
    /// exited/killed）；`fds` 为空时纯等状态/超时。
    fn poll(&mut self, fds: &[u32], timeout_ms: u64) -> Result<(), String>;
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
    fn spawn(
        &self,
        cmd: &ExecCommand,
        env: &[(String, String)],
    ) -> Result<Box<dyn HostProcess>, String>;

    /// 创建一对**子进程侧**端点 `(read, write)`，供 `FdSpec::Handle` 连接
    /// 两个 job（管道）；不经过父进程。默认不支持（单命令宿主可忽略）。
    fn pipe(&self) -> Result<(Box<dyn HostFd>, Box<dyn HostFd>), String> {
        Err("host does not support exec pipes".to_string())
    }
}

/// 阻塞聚合 helper：把命令的 fd 0/1/2 重建成
/// `Data(stdin)` + 两条父读管道，收齐 stdout/stderr 与退出码；
/// 超时 → kill 并报错。非零退出**不是** `Err`（由 Tcl 层决定是否按
/// `CHILDSTATUS` 抛错）。fd 3+ 的调用方规则保留。
#[cfg(feature = "std")]
pub fn exec_collect(
    host: &dyn HostExec,
    mut cmd: ExecCommand,
    stdin: &[u8],
    env: &[(String, String)],
    timeout_ms: u64,
) -> Result<ExecOutput, String> {
    while cmd.fds.len() < 3 {
        cmd.fds.push(FdSpec::Null);
    }
    cmd.fds[0] = if stdin.is_empty() {
        FdSpec::Null
    } else {
        FdSpec::Data(stdin.to_vec())
    };
    cmd.fds[1] = FdSpec::Pipe(PipeSide::ParentReads);
    cmd.fds[2] = FdSpec::Pipe(PipeSide::ParentReads);

    let mut p = host.spawn(&cmd, env)?;
    let start = std::time::Instant::now();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut out_eof = false;
    let mut err_eof = false;
    let mut buf = [0u8; 64 * 1024];

    loop {
        if !out_eof {
            out_eof = read_available(p.as_mut(), 1, &mut out, &mut buf)?;
        }
        if !err_eof {
            err_eof = read_available(p.as_mut(), 2, &mut err, &mut buf)?;
        }
        if let Some(code) = p.wait()? {
            // 退出后把管道剩余字节排空（忽略方向/关闭错误）。
            let _ = read_available(p.as_mut(), 1, &mut out, &mut buf);
            let _ = read_available(p.as_mut(), 2, &mut err, &mut buf);
            return Ok(ExecOutput { stdout: out, stderr: err, code });
        }
        if timeout_ms > 0 && start.elapsed().as_millis() as u64 >= timeout_ms {
            let _ = p.kill();
            return Err(format!("exec: timeout after {timeout_ms}ms"));
        }
        let mut poll_fds = Vec::new();
        if !out_eof {
            poll_fds.push(1);
        }
        if !err_eof {
            poll_fds.push(2);
        }
        p.poll(&poll_fds, 10)?;
    }
}

/// 读一个父侧 fd 直到 `WouldBlock`；返回是否 EOF。`exec_collect` 内部使用。
#[cfg(feature = "std")]
fn read_available(
    p: &mut dyn HostProcess,
    fd: u32,
    out: &mut Vec<u8>,
    buf: &mut [u8],
) -> Result<bool, String> {
    let Some(f) = p.fd(fd) else {
        return Err(format!("exec: host did not expose fd {fd}"));
    };
    loop {
        match f.read(buf) {
            Ok(0) => return Ok(true),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(ExecIoError::WouldBlock) => return Ok(false),
            Err(ExecIoError::BrokenPipe) => return Ok(true),
            Err(ExecIoError::Other(e)) => return Err(format!("fd {fd}: {e}")),
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

    struct ReadFd {
        data: Vec<u8>,
        off: usize,
    }

    impl HostFd for ReadFd {
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
    }

    struct MockProcess {
        fds: Vec<Option<Box<dyn HostFd>>>,
        state: JobState,
    }

    impl HostProcess for MockProcess {
        fn id(&self) -> JobId {
            7
        }
        fn fd(&mut self, fd: u32) -> Option<&mut (dyn HostFd + 'static)> {
            self.fds.get_mut(fd as usize)?.as_deref_mut()
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
        fn poll(&mut self, _fds: &[u32], _timeout_ms: u64) -> Result<(), String> {
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
            cmd: &ExecCommand,
            _env: &[(String, String)],
        ) -> Result<Box<dyn HostProcess>, String> {
            assert!(matches!(cmd.fds[1], FdSpec::Pipe(PipeSide::ParentReads)));
            assert!(matches!(cmd.fds[2], FdSpec::Pipe(PipeSide::ParentReads)));
            Ok(Box::new(MockProcess {
                fds: vec![
                    None,
                    Some(Box::new(ReadFd { data: self.stdout.clone(), off: 0 })),
                    Some(Box::new(ReadFd { data: self.stderr.clone(), off: 0 })),
                ],
                state: JobState::Exited(self.code),
            }))
        }
    }

    #[test]
    fn exec_command_defaults_to_fd_view() {
        let cmd = ExecCommand::new("hello", vec!["hello".into()]);
        assert_eq!(cmd.fds.len(), 3);
        assert!(matches!(cmd.fds[0], FdSpec::Null));
        assert!(matches!(cmd.fds[1], FdSpec::Pipe(PipeSide::ParentReads)));
        assert!(matches!(cmd.fds[2], FdSpec::Pipe(PipeSide::ParentReads)));
    }

    #[test]
    fn exec_collect_reads_fd_1_2_not_stdio_methods() {
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
