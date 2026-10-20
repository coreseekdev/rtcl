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

`rtcl-core::host` 中扩展已有的 `HostProcess`（不新建平行 trait）。**没有 stdin/stdout/stderr 专用方法**；统一走 fd 文件视图：

```rust
pub type JobId = u64;

pub enum JobState { Running, StopRequested, Stopped, Exited(i32), Killed }

pub trait HostFd: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ExecIoError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, ExecIoError>;
    fn close(&mut self) -> Result<(), String>;
    fn poll(&mut self, read: bool, write: bool, timeout_ms: u64) -> Result<(), String>;
}

pub enum PipeSide { ParentReads, ParentWrites }
pub enum FdMode { Read, Write, Append }

pub enum FdSpec {
    Null,
    Pipe(PipeSide),
    File { path: String, mode: FdMode },
    Data(Vec<u8>),
    Dup(u32),
    Handle(Box<dyn HostFd>),
}

pub struct ExecCommand {
    pub prog: String,
    pub argv: Vec<String>,
    pub fds: Vec<FdSpec>,   // 下标 = 子进程 fd；0/1/2 只是约定
}

pub trait HostProcess: Send {
    fn id(&self) -> JobId;
    fn fd(&mut self, fd: u32) -> Option<&mut dyn HostFd>;
    fn state(&mut self) -> Result<JobState, String>;
    fn stop(&mut self) -> Result<(), String>;
    fn resume(&mut self) -> Result<(), String>;
    fn wait(&mut self) -> Result<Option<i32>, String>;
    fn kill(&mut self) -> Result<(), String>;
    fn poll(&mut self, fds: &[u32], timeout_ms: u64) -> Result<(), String>;
}

pub trait HostExec: Send {
    fn spawn(&self, cmd: &ExecCommand, env: &[(String, String)])
        -> Result<Box<dyn HostProcess>, String>;
    /// 子进程侧端点对 (read, write)，供 FdSpec::Handle 直连两个 job。
    fn pipe(&self) -> Result<(Box<dyn HostFd>, Box<dyn HostFd>), String>;
}
```

- `fd(0/1/2)` 与 `fd(3+)` 完全同构；`0/1/2` 只是约定，不是特例。
- `FdSpec::Pipe` 由宿主建管道，父侧端点经 `HostProcess::fd(n)` 暴露；
  `FdSpec::Handle` 是**子进程侧**端点（管道直连、外部 fd）。
- 管道：`HostExec::pipe()` 返回子进程侧 `(read, write)`；stage A 的 fd 1 用
  `Handle(write)`，stage B 的 fd 0 用 `Handle(read)`，不经过父进程。
- 重定向：`<` = `File{Read}`@0，`<<` = `Data`@0，`>`/`>>` =
  `File{Write/Append}`@1，`2>@1` = `Dup(1)`@2。
- `HostExec::spawn` 返回 `Box<dyn HostProcess>`；不增加 capability 位。
- `poll` 除 I/O 就绪/超时/退出外，**状态变化**（stop/resume/stopped）也必须唤醒。
- 具体错误文本不作规范；Tcl 层按状态决定 `CHILDSTATUS` / `CHILDKILLED` / `CHILDStopped` 的呈现。

## 4. 宿主 ABI（wasm / 浏览器）

与 Rust trait 同义；沿用现有 `host.call` 帧格式（`[u32 nfields][u32 len + bytes]...`）。
fd 是第一等参数：

| 方法 | 语义 |
|---|---|
| `exec.open <prog> <fdplan> ?arg ...?` | 创建 job，返回 handle |
| `exec.pipe` | 建子进程侧端点对，返回 `[read-handle, write-handle]` |
| `exec.fd.read <id> <fd> ?max?` | 读父侧 fd（空 = EOF） |
| `exec.fd.write <id> <fd> <data>` | 写父侧 fd |
| `exec.fd.close <id> <fd>` | 关闭父侧 fd（子进程见 EOF） |
| `exec.fd.poll <id> <fd> ?timeout?` | 等单个 fd |
| `exec.poll <id> <fds> ?timeout?` | 等一组 fd + 状态变化 |
| `exec.state <id>` | `running` / `stopping` / `stopped` / `exited <code>` / `killed` |
| `exec.stop <id>` | 请求协作挂起 |
| `exec.resume <id>` | 清除挂起 |
| `exec.kill <id>` | 硬杀 |
| `exec.wait <id>` | 阻塞到终态；前台作业的 stop 由宿主 job prompt 处理，不把 Tcl 栈打断 |
| `exec.list` | 当前 job 列表（宿主 job table） |
| `exec.run <prog> ?arg ...?` | 一次性聚合便利方法，不参与 job control（内部用 0/1/2 fd） |

`fdplan` 是 fd 规则序列（每项 `fd:kind[:arg]`，逗号分隔）：
`null` / `pipe-r` / `pipe-w` / `file-r:<path>` / `file-w:<path>` /
`file-a:<path>` / `data:<bytes>` / `dup:<fd>` / `handle:<id>`。

- `exec.wait` 在宿主 job prompt 里遇到 `fg` 继续等待，遇到 `bg` 返回“已转后台”，遇到 `kill` 返回错误/终态。
- 所有实现必须支持 `exec.kill`（native SIGKILL / web `Worker.terminate()`）；`exec.stop` 允许停在 `StopRequested`。
- 不新增 `preemptive` 标志；没有“本地更强”的分支。
- 旧 `exec.read/write/close-stdin` 是 `exec.fd.*` 的 0/1/2 语法糖，新 ABI 不再作为规范接口。

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
- 前台 `exec` 被 Ctrl-Z 后进入**受限 job prompt**（只接受 `jobs/fg/bg/kill/help`），不执行任意 Tcl；`fg` 保留 Tcl 栈，`bg` 让 `exec` 返回 handle 列表，`kill` 结束作业。
- 非交互脚本不做作业控制；Ctrl-Z/Ctrl-C 不改变脚本语义。

## 6. 终端与 I/O

- native：rogo 拥有 tty；job 使用 OS pipe；rogo 代理 stdin/stdout/stderr。**不使用 `tcsetpgrp`/`SIGTSTP` 作业控制，不使用 `SIGSTOP`。**
- web：wterm/TS 行规程；job 使用 Worker + SAB ring；主线程拥有终端。
- 前台 job 的 stdin 由宿主按行转发；后台 job 的 stdin 关闭（读得 EOF）。
- 输出按宿主终端路由；有界背压：OS pipe 满 / SAB ring 满时 job 在写 import 阻塞。
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

1. 本规范 + `rtcl-core` trait 扩展（当前）；
2. rogo native runner 后端 + 统一 checkpoint 协议；
3. rtcl `job` 词层 + `exec` 接入；
4. rogo web host（Worker/SAB/COOP-COEP/wterm）；
5. conformance fixtures 与 CLI/Playwright 测试。
