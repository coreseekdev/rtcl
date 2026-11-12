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

/// 文件系统数据面服务：`file`/`open`/`source`/`glob`/`cd`/`pwd` 的
/// std::fs 直连点路由到本 trait（§10quinter 定调：通道协议面留原生，
/// 数据面走宿主服务）。stat 的 unix 深水键（dev/ino/uid/…）、readlink、
/// settimes 不在数据面，留原生实现。
///
/// 原生构建默认 [`NativeFs`]（std::fs 直连，行为与路由前逐字一致）；
/// wasm 宿主经 `Interp::set_host_fs` 注入 JS 实现（浏览器内存 fs /
/// OPFS worker），浏览器里 open/gets/puts/file 由此真正可用。
/// `file stat` 的 13 键全形（tclsh TclStatObj 形状）。NativeFs 从
/// std::fs 逐字填充（unix 扩展键取 MetadataExt）；宿主实现（内存 fs /
/// OPFS）合成自洽值或如实报错——值只需内部一致，tclsh 对一切文件系统
/// 同样如此。
#[derive(Clone, Debug, Default)]
pub struct FsStat {
    pub size: i64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub dev: i64,
    pub ino: i64,
    pub nlink: i64,
    pub uid: i64,
    pub gid: i64,
    pub mode: i64,
    pub blksize: i64,
    pub blocks: i64,
    pub is_dir: bool,
    pub is_file: bool,
    /// symlink（lstat 语义）。
    pub is_link: bool,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
    pub owned: bool,
}

pub trait HostFs: Send {
    /// 打开文件通道。`mode` 为 Tcl access 串（r/w/a/r+/w+/a+ 及
    /// PLATFORM 兼容的 RB/WB 组合变体）。返回 (通道, 是否二进制)。
    fn open(&mut self, path: &str, mode: &str)
        -> std::io::Result<(Box<dyn crate::channel::Channel>, bool)>;
    fn exists(&self, path: &str) -> bool;
    fn is_file(&self, path: &str) -> bool;
    fn is_dir(&self, path: &str) -> bool;
    fn size(&self, path: &str) -> std::io::Result<u64>;
    fn mtime(&self, path: &str) -> std::io::Result<i64>;
    fn atime(&self, path: &str) -> std::io::Result<i64>;
    fn readable(&self, path: &str) -> bool;
    fn writable(&self, path: &str) -> bool;
    fn executable(&self, path: &str) -> bool;
    fn mkdir(&mut self, path: &str, all: bool) -> std::io::Result<()>;
    fn delete(&mut self, path: &str) -> std::io::Result<()>;
    fn rename(&mut self, src: &str, dst: &str) -> std::io::Result<()>;
    fn copy(&mut self, src: &str, dst: &str) -> std::io::Result<()>;
    /// 目录列举（不含 `.`/`..`；glob 与 readdir 共用）。
    fn list_dir(&self, path: &str) -> std::io::Result<Vec<String>>;
    fn pwd(&self) -> String;
    fn cd(&mut self, path: &str) -> std::io::Result<()>;
    /// `source`/`file tempfile` 的整文件读取与临时路径。
    fn read_file(&mut self, path: &str) -> std::io::Result<String>;
    fn tempfile(&mut self, prefix: &str) -> std::io::Result<String>;
    /// `file stat`/`lstat` 全形（follow=false 为 lstat 语义）。
    /// NotFound 错误 = 不存在。
    fn stat_full(&self, path: &str, follow: bool) -> std::io::Result<FsStat>;
    /// `file mtime name time` / `file atime name time`（None = 不变）。
    /// 宿主实现不支持时如实报错（OPFS 无 utimes API）。
    fn set_times(
        &mut self,
        path: &str,
        atime: Option<i64>,
        mtime: Option<i64>,
    ) -> std::io::Result<()>;
    /// `file readlink`（无 symlink 概念的实现报 invalid argument）。
    fn read_link(&self, path: &str) -> std::io::Result<String>;
}

/// std 原生实现：逐字保留路由前的 std::fs 行为。
#[cfg(feature = "std")]
pub struct NativeFs;

#[cfg(feature = "std")]
impl HostFs for NativeFs {
    fn open(&mut self, path: &str, mode: &str)
        -> std::io::Result<(Box<dyn crate::channel::Channel>, bool)>
    {
        crate::channel::open_file_channel(path, mode)
    }
    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
    fn is_file(&self, path: &str) -> bool {
        std::path::Path::new(path).is_file()
    }
    fn is_dir(&self, path: &str) -> bool {
        std::path::Path::new(path).is_dir()
    }
    fn size(&self, path: &str) -> std::io::Result<u64> {
        Ok(std::fs::metadata(path)?.len())
    }
    fn mtime(&self, path: &str) -> std::io::Result<i64> {
        let t = std::fs::metadata(path)?.modified()?;
        Ok(t.duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default().as_secs() as i64)
    }
    fn atime(&self, path: &str) -> std::io::Result<i64> {
        let t = std::fs::metadata(path)?.accessed()?;
        Ok(t.duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default().as_secs() as i64)
    }
    fn readable(&self, path: &str) -> bool {
        crate::interp::commands::io::fs_access(path, crate::interp::commands::io::AccessCheck::Read)
    }
    fn writable(&self, path: &str) -> bool {
        crate::interp::commands::io::fs_access(path, crate::interp::commands::io::AccessCheck::Write)
    }
    fn executable(&self, path: &str) -> bool {
        crate::interp::commands::io::fs_access(path, crate::interp::commands::io::AccessCheck::Exec)
    }
    fn mkdir(&mut self, path: &str, all: bool) -> std::io::Result<()> {
        if all {
            std::fs::create_dir_all(path)
        } else {
            std::fs::create_dir(path)
        }
    }
    fn delete(&mut self, path: &str) -> std::io::Result<()> {
        if std::path::Path::new(path).is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    }
    fn rename(&mut self, src: &str, dst: &str) -> std::io::Result<()> {
        std::fs::rename(src, dst)
    }
    fn copy(&mut self, src: &str, dst: &str) -> std::io::Result<()> {
        std::fs::copy(src, dst).map(|_| ())
    }
    fn list_dir(&self, path: &str) -> std::io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if let Some(s) = entry.file_name().to_str() {
                out.push(s.to_string());
            }
        }
        Ok(out)
    }
    fn pwd(&self) -> String {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }
    fn cd(&mut self, path: &str) -> std::io::Result<()> {
        std::env::set_current_dir(path)
    }
    fn read_file(&mut self, path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
    fn tempfile(&mut self, prefix: &str) -> std::io::Result<String> {
        // 与路由前 file_tempfile 同形：temp 目录 + <prefix><纳秒时间戳>。
        let dir = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!("{}{}", prefix, stamp));
        std::fs::File::create(&path)?;
        Ok(path.to_string_lossy().to_string())
    }

    fn stat_full(&self, path: &str, follow: bool) -> std::io::Result<FsStat> {
        let meta = if follow {
            std::fs::metadata(path)?
        } else {
            std::fs::symlink_metadata(path)?
        };
        let secs = |t: std::io::Result<std::time::SystemTime>| {
            t.ok()
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        };
        let ft = meta.file_type();
        let mut s = FsStat {
            size: meta.len() as i64,
            mtime: secs(meta.modified()),
            atime: secs(meta.accessed()),
            is_dir: ft.is_dir(),
            is_file: ft.is_file(),
            is_link: ft.is_symlink(),
            readable: crate::interp::commands::io::fs_access(path, crate::interp::commands::io::AccessCheck::Read),
            writable: crate::interp::commands::io::fs_access(path, crate::interp::commands::io::AccessCheck::Write),
            executable: crate::interp::commands::io::fs_access(path, crate::interp::commands::io::AccessCheck::Exec),
            ..Default::default()
        };
        // `file stat` 的 13 键全形：unix 扩展键取 MetadataExt 真值
        //（与路由前逐字一致）；非 unix 全 0（行为同路由前 cfg(not(unix))）。
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            s.dev = meta.dev() as i64;
            s.ino = meta.ino() as i64;
            s.nlink = meta.nlink() as i64;
            s.uid = meta.uid() as i64;
            s.gid = meta.gid() as i64;
            s.mode = meta.mode() as i64;
            s.blksize = meta.blksize() as i64;
            s.blocks = meta.blocks() as i64;
            s.ctime = meta.ctime() as i64;
            s.owned = s.uid == unsafe { libc_getuid() };
        }
        Ok(s)
    }

    fn set_times(
        &mut self,
        path: &str,
        atime: Option<i64>,
        mtime: Option<i64>,
    ) -> std::io::Result<()> {
        use std::fs::FileTimes;
        let mut ft = FileTimes::new();
        if let Some(a) = atime {
            ft = ft.set_accessed(std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(a.max(0) as u64));
        }
        if let Some(m) = mtime {
            ft = ft.set_modified(std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(m.max(0) as u64));
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|f| f.set_times(ft))
    }

    fn read_link(&self, path: &str) -> std::io::Result<String> {
        let t = std::fs::read_link(path)?;
        Ok(t.to_string_lossy().to_string())
    }
}

/// getuid without pulling libc into non-unix builds.
#[cfg(unix)]
unsafe fn libc_getuid() -> i64 {
    unsafe { libc_getuid_impl() }
}
#[cfg(unix)]
unsafe fn libc_getuid_impl() -> i64 {
    // std exposes no uid accessor; use libc via the raw syscall shim the
    // crate already links on unix (clock feature pulls libc).
    extern "C" {
        fn getuid() -> u32;
    }
    getuid() as i64
}
