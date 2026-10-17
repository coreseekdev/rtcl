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

/// 进程执行服务：`exec`/`open |cmd` 的宿主接缝（浏览器下不存在，
/// 对应 feature 在 wasm 组合里天然缺席）。
pub trait HostExec: Send {}
