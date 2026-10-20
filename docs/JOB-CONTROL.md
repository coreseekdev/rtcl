# Job Control（作业控制）规范 v0

- 日期：2026-10-20
- 状态：normative draft
- 范围：rtcl `exec` / 外部 wasm 作业的生命周期；rogo native 与 rogo web 同一语义
- 相关代码：`crates/rtcl-core/src/host.rs`（trait）
- 相关裁决：2026-10-20 用户确认「native 与 web 采用同一套规范；不做 native-only 抢占式增强」

## 0. 能力归属

**作业控制是 rtcl 的能力，不是 rogo 的。**

| 层 | 职责 |
|---|---|
| `rtcl-core` | `HostExec` / `HostProcess` 公共 trait、`JobState` 状态机、`exec` 与 `job` 的 Tcl 词语义 |
| `rtcl-wasi` | guest 侧 `host::exec::*` / `host::job::*` 绑定；ABI 与 trait 同义 |
| `rogo` | 宿主实现：native runner 子进程、web Worker/TS host；终端/REPL；profile 别名；conformance harness |
| `rogo-web` | 浏览器本地 host（Worker + SAB + Atomics）；不定义语义 |

rogo 不得新增 job 语义或 `if native` 分支；native 与 web 只有实现差异，没有能力差异。

## 1. 模型

- **job**：一个隔离执行单元。native = runner 子进程；web = Worker。
- **pipeline**：一组 job（每个 stage 一个），共享一个作业号；停止/恢复/杀死广播到全部 stage。
- **handle**：`JobId = u64`，不透明。native 通常是 PID，web 是合成 ID；调用方不得假设 OS PID 语义。
- **checkpoint**：宿主可拦截的 import 边界（WASI 调用、`host.call`、显式 `checkpoint`）。只有到达 checkpoint，job 才能从 `StopRequested` 进入 `Stopped`。
- **无抢占**：native 不使用 `SIGSTOP`，web 不使用 `terminate` 来做“挂起”（terminate = 杀死）。
- **终端归宿主所有**：job 不拥有 tty；native 也不把 tty 交给 job。job 的 stdin/stdout/stderr 是宿主代理的字节流。

## 2. 状态机

```
                 stop                 checkpoint
Running ───────────────▶ StopRequested ─────────────▶ Stopped
   ▲                         │                          │
   │        resume           │        resume            │
   └─────────────────────────┴──────────────────────────┘

Running ── exit(code) ──▶ Exited(code)
Running / StopRequested / Stopped ── kill ──▶ Killed
```

规则：

- `stop`：幂等；`Running → StopRequested`。若 job 没有 checkpoint，停在 `StopRequested`（显示 `stopping`），不报错。
- `resume`：幂等；`StopRequested → Running`（取消未生效的 stop），`Stopped → Running`。终态上为 no-op。
- `kill`：幂等；硬杀，立即 `Killed`；不保证 guest 清理。
- `wait`：非阻塞；返回 `Some(code)` 表示终态。`Killed` 的 code 固定为 `137`（128+SIGKILL 约定）。
- `state`：非阻塞；终态可重复读取。
- `Exited(code)` 与 `Killed` 是终态；`Running` / `StopRequested` / `Stopped` 不是。
- 不做 `Cancelled` 状态：Ctrl-C 的协作取消让 guest 以退出码结束（约定 130）；超时兜底 `kill` 才是 `Killed`。

## 3. Rust 公共 trait

`rtcl-core::host` 中扩展已有的 `HostProcess`（不新建平行 trait）。**没有 stdin/stdout/stderr 专用方法，也没有 spawn 时的 path/data 特例**；fd 表由调用方先建好，0/1/2 只是约定：

```rust
pub type JobId = u64;

pub enum JobState { Running, StopRequested, Stopped, Exited(i32), Killed }

pub trait HostFd: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ExecIoError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, ExecIoError>;
    fn close(&mut self) -> Result<(), String>;
    fn poll(&mut self, read: bool, write: bool, timeout_ms: u64) -> Result<(), String>;
    fn dup(&self) -> Result<Box<dyn HostFd>, String>;
}

pub enum FdMode { Read, Write, Append }

pub trait HostFs: Send {
    fn open(&self, path: &str, mode: FdMode) -> Result<Box<dyn HostFd>, String>;
}

pub struct NullFd;                      // /dev/null
pub struct BytesFd { /* data, off */ }  // 只读内存流（heredoc）

pub struct ExecCommand { pub prog: String, pub argv: Vec<String> }

pub trait HostExec: Send {
    /// fds 下标 = 子进程 fd；None = 该 fd 关闭；尾部缺失 = 关闭。
    fn spawn(&self, cmd: &ExecCommand,
             fds: Vec<Option<Box<dyn HostFd>>>,
             env: &[(String, String)]) -> Result<Box<dyn HostProcess>, String>;

    /// 建一对端点 (read, write)；一端进子进程 fd 表，另一端父进程保留，
    /// 或两端分别交给管道两端的 job。
    fn pipe(&self) -> Result<(Box<dyn HostFd>, Box<dyn HostFd>), String>;
}

pub trait HostProcess: Send {
    fn id(&self) -> JobId;
    fn state(&mut self) -> Result<JobState, String>;
    fn stop(&mut self) -> Result<(), String>;
    fn resume(&mut self) -> Result<(), String>;
    fn wait(&mut self) -> Result<Option<i32>, String>;
    fn kill(&mut self) -> Result<(), String>;
    fn poll(&mut self, timeout_ms: u64) -> Result<(), String>; // 只等状态/超时
}
```

- `0/1/2` 与 `3+` 完全同构；没有 `read_stdout()`/`write_stdin()` 之类的方法。
- **文件视图由调用方构建**：
  - 捕获 stdout：`(r, w) = pipe()`，子进程 `fds[1] = Some(w)`，父进程读 `r`；
  - 注入 stdin：`(r, w) = pipe()`，子进程 `fds[0] = Some(r)`，父进程写 `w`；
  - 管道：`(r, w) = pipe()`，A 的 fd 1 = `Some(w)`，B 的 fd 0 = `Some(r)`；
  - 文件：`fs.open(path, Read/Write/Append)` 的返回值放进 fd 表；
  - heredoc：`BytesFd`；`/dev/null`：`NullFd`；`2>@1`：`fds[1].dup()`。
- `spawn` 接管子进程侧端点；父进程只保留自己需要的另一端。没有
  `HostProcess::fd()`，没有隐式 stdio。
- `HostProcess::poll` 只等生命周期；fd 就绪用 `HostFd::poll`（两者可组合）。
- `HostExec::spawn` 返回 `Box<dyn HostProcess>`；不增加 capability 位。
- 具体错误文本不作规范；Tcl 层按状态决定 `CHILDSTATUS` / `CHILDKILLED` / `CHILDStopped` 的呈现。

### 作业表

`exec &` 与 `job *` 通过宿主作业表访问；一个 entry 是 pipeline 的一组 stage：

```rust
pub struct JobInfo { pub id: JobId, pub command: String, pub state: JobState }

pub trait HostJobs: Send {
    fn add(&self, handles: Vec<Box<dyn HostProcess>>, command: String) -> JobId;
    fn list(&self) -> Vec<JobInfo>;
    fn stop(&self, id: JobId) -> Result<(), String>;
    fn resume(&self, id: JobId) -> Result<(), String>;
    fn kill(&self, id: JobId) -> Result<(), String>;
    fn wait(&self, id: JobId) -> Result<Option<i32>, String>; // 非阻塞
}
```

- 表必须**保留句柄**；drop 在 native 后端会杀掉仍在运行的 job。
- group state：所有 stage 终态 → 最后 stage 的退出码；否则有 stopped/stopping 就显示对应状态，否则 running。
- Tcl 的 `%N` 是用户层别名，落到 `JobId`（native 是内部作业号，不是 OS PID）。

## 4. 宿主 ABI（wasm / 浏览器）

与 Rust trait 同义；沿用现有 `host.call` 帧格式（`[u32 nfields][u32 len + bytes]...`）。
fd 是全局句柄，先建后用：

| 方法 | 语义 |
|---|---|
| `exec.pipe` | 返回 `[read-handle, write-handle]` |
| `fs.open <path> <mode>` | 返回文件 handle（native p-root / web OPFS） |
| `fs.bytes <data>` | 返回只读内存流 handle |
| `fd.null` | 返回 `/dev/null` handle |
| `fd.read <handle> ?max?` | 读父侧 handle（空 = EOF） |
| `fd.write <handle> <data>` | 写父侧 handle |
| `fd.close <handle>` | 关闭父侧 handle |
| `fd.poll <handle> ?timeout?` | 等单个 handle |
| `fd.dup <handle>` | 复制 handle（`2>@1`） |
| `exec.open <prog> <fdhandles> ?arg ...?` | 创建 job，返回 job id；handle 表被接管 |
| `exec.state <id>` | `running` / `stopping` / `stopped` / `exited <code>` / `killed` |
| `exec.stop <id>` | 请求协作挂起 |
| `exec.resume <id>` | 清除挂起 |
| `exec.kill <id>` | 硬杀 |
| `exec.wait <id>` | 阻塞到终态；前台作业的 stop 由宿主 job prompt 处理，不把 Tcl 栈打断 |
| `exec.list` | 当前 job 列表（宿主 job table） |
| `exec.run <prog> ?arg ...?` | 一次性聚合便利方法，不参与 job control（内部建 0/1/2 fd） |

- `fdhandles` 是逗号分隔的 handle 列表：`-` 表示 fd 关闭；列表下标 = fd 号。
- `exec.open` **消费** 列表里的 handle（所有权转移给子进程）；父进程要保留的一端不在列表里。
- `exec.wait` 在宿主 job prompt 里遇到 `fg` 继续等待，遇到 `bg` 返回“已转后台”，遇到 `kill` 返回错误/终态。
- 所有实现必须支持 `exec.kill`（native SIGKILL / web `Worker.terminate()`）；`exec.stop` 允许停在 `StopRequested`。
- 不新增 `preemptive` 标志；没有“本地更强”的分支。
- 旧 `exec.read/write/close-stdin` 是 `fd.read/write/close` 的 0/1/2 语法糖，新 ABI 不再作为规范接口。

## 5. Tcl 词语义

规范命令（rtcl 提供，rogo 只做别名）：

```
job list
job stop   ?job-id?
job resume ?job-id?
job fg     ?job-id?
job bg     ?job-id?
job kill   ?job-id?
job wait   ?job-id?
```

- `job-id` 接受 handle 或 `%N` 作业号；缺省为当前 job。
- `jobs` 是 `job list` 的只读别名，由 profile 提供；`kill` 不覆盖 rtcl 已有的 OS `kill`，统一用 `job kill`。
- `exec ... &` 返回 handle 列表（tclsh 的 PID 列表位置）；`job list` 同时给出 `%N`。
- `exec` 的重定向先在 Tcl 侧构建 fd 视图：`<`/`>`/`>>` 用 `fs.open`，`<<` 用 `fs.bytes`，`2>@1` 用 `dup`，`|` 用 `exec.pipe` + 两端 handle；spawn 只看到 fd 表，不看到路径/数据。
- 前台 `exec` 被 Ctrl-Z 后进入**受限 job prompt**（只接受 `jobs/fg/bg/kill/help`），不执行任意 Tcl；`fg` 保留 Tcl 栈，`bg` 让 `exec` 返回 handle 列表，`kill` 结束作业。
- 非交互脚本不做作业控制；Ctrl-Z/Ctrl-C 不改变脚本语义。

## 6. 终端与 I/O

- native：rogo 拥有 tty；job 使用 OS pipe；rogo 代理 fd 0/1/2。**不使用 `tcsetpgrp`/`SIGTSTP` 作业控制，不使用 `SIGSTOP`。**
- web：wterm/TS 行规程；job 使用 Worker + SAB ring；主线程拥有终端。
- 前台 job 的 stdin 由宿主按行转发；后台 job 的 stdin 关闭（读得 EOF）。
- 输出按宿主终端路由；有界背压：OS pipe 满 / SAB ring 满时 job 在写 import 阻塞。
- **流式不变量**：pipe/ring 必须有界且边读边消费；宿主 pipeline 禁止把 stage 输出整体缓存后再转发。`< file` 由宿主流式读。stderr 是宿主流目标：默认阻塞写终端（背压），`2>@1` 并入 stdout 管道，`2>file` 流文件，`-ignorestderr` 丢弃；禁止全量内存收集。
- **fd 继承边界**：HostFd 端点一律 `FD_CLOEXEC`；子进程只保留 spawn 时 `dup2` 的目标 fd。否则 runner 会继承自己管道的另一端（heredoc 写端会让 stdin 永远等不到 EOF）。
- 父侧 HostFd 保持非阻塞接口；子进程 stdio 在 `dup2` 后清 `O_NONBLOCK`，runner 才能按阻塞语义做 stdin relay。
- 前台 Ctrl-Z 仅在 stdin/stdout 都是 tty 时启用：rogo 捕获 `SIGTSTP`，runner 在**独立进程组**（不接收终端信号），随后进入受限 `job%` prompt（`fg/bg/jobs/kill/help`，不执行 Tcl）。`fg` 保留 Tcl 栈；`bg` 把句柄交给 HostJobs 并让 `exec` 返回 job id。
- 不支持 termios/全屏 TUI（WASI p1 无 termios）；两边一致。
- Ctrl-Z = stop 请求；Ctrl-C = cancel 请求（协作退出，超时 kill）；guest 不接收 OS 信号。

## 7. checkpoint 与延迟

- rogo 自家 rtcl wasm：解释器每 N 条命令调用 `host.checkpoint`（可配置），延迟 ≤ N 条命令。
- 构建时插桩的 wasm：loop 回边/函数入口插 checkpoint；仍是协作式。
- 未插桩的第三方 wasm：只能在 I/O/import 边界停；纯计算停在 `StopRequested`，只能 `kill`。
- 两边规则相同；native 不因操作系统能力而“更强”。

## 8. 一致性与维护

- 一份本规范 + 一套共享 conformance 测试（Tcl 脚本 + wasm fixtures）；
- CLI（pty harness）与 Web（Playwright）跑同一组断言：stop/pending/resume/kill、fg/bg、管道、背压、退出码；
- 两个 host adapter 只实现 `spawn/read/write/stop/resume/kill/wait/state`；
- 环境差异（Web 需要 COOP/COEP；刷新丢作业；后台标签节流）写进文档，不算能力差异。

## 9. 非目标

- 抢占式挂起任意 wasm；
- `kill` 的 guest 清理保证；
- OS PID/信号语义；
- 全屏 TUI/termios；
- 页面刷新后的作业恢复（无快照）。

## 10. 实施切片

1. 本规范 + `rtcl-core` fd 视图 trait（当前）；
2. rogo native runner 后端 + 统一 checkpoint 协议；
3. rtcl `job` 词层 + `exec` 接入；
4. rogo web host（Worker/SAB/COOP-COEP/wterm）；
5. conformance fixtures 与 CLI/Playwright 测试。
