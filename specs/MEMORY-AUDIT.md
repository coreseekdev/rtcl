# rtcl 运行时内存 / 资源生命周期审计（2026-11-08）

- **行为规格**：[`specs/memory-lifecycle.allium`](memory-lifecycle.allium)
  （allium 3.6.1，`allium check` 0 error；仅 unused 字段 warning）。
- **方法**：allium 契约 + 代码审计（`rtcl-vm` / `rtcl-jit`）+
  OO/通道 churn 探针（RSS 与残留命令/命名空间计数）。
- **范围**：Value 引用计数、COW、调用帧、解释器缓存、OO 对象图、
  通道/子进程、JIT host/pool/module。不含 Tcl 语义正确性
  （见 `DIVERGENCES.md`）。

## 结论（TL;DR）

**Value/帧/错误路径的“即时释放”做得较好；解释器级 teardown、JIT
线程级缓存、少数脚本可控注册表还不够谨慎。**

- 已经足够谨慎的部分：
  - `Value` 是手写非原子引用计数 + COW；最后一个 handle 立即
    `drop_in_place`，list/dict 子值递归释放；盒子经线程级 FREELIST
    复用（上限 8192）。
  - 帧出栈会清空 `locals/array_locals/upvars/local_procs/
    deferred_scripts/slots/level0`；帧池上限 32；`args_pool` 在错误
    路径也归还。
  - 普通 `?`/early-return 路径的 Rust 临时值会立即析构；未发现
    Rc/refcount 环（MethodDef/proc_memo、head_cache、trace、upvar、
    object<->class、channel handler 均无回指）。
  - 显式 `close` 会跑 reflected `finalize`/transform `drain`，并
    `PipeChannel::close` 关闭管道后 `child.wait()`。

- 不够谨慎的部分（本轮已修 / 仍开放）：
  - **[已修] JIT 无 teardown**：`Interp` Drop 现调用注册的 teardown hook；
    classic wasmi/wasmtime pool/module 与 fast store/module 按 Interp
    指针清理，共享 emit 缓存整清。
  - **[已修] `exec &` 泄漏 Child**：后台 Child 进入
    `Interp.background_children`，exec/update/Drop 用 try_wait reap。
  - **[已修] Fast JIT host arena 无界增长**：fast Store 是线程常驻，
    `Host::tag_value` 每次宿主结果追加 arena，而 fast 路径从不
    `arena_truncate`；字符串/列表型 proc 随调用次数增长。
  - **[已修] wasmtime fast Store 不随 Interp 重绑**：旧 `*mut Interp`
    可能悬垂；wasmi 已有 rebind，wasmtime 缺失。
  - **[已修] ChannelTable 无 Drop**：Interp 析构不关通道，pipe 子进程
    不 wait。现 Drop 逐个 `close()`（文件 Flush/Close、pipe 关管道并
    wait）。
  - **[已修] OO 销毁不释放链缓存**：`chain_memo/obj_cache/my_cache/
    key_memo` 继续持有已销毁对象/类的 `MethodDef`（body Rc、proc_memo、
    head_cache Value）。
  - **[已修] `call_proc` 深度检查前未消费一次性状态**：
    `frame_prelink`/`frame_level0_args` 在超深调用时留在 Interp 上，
    泄漏到下一次调用（也是正确性问题）。
  - **[已修] `var_link_cache` 内层无上限**：外层 1024 ns，但单 ns 内
    可被脚本写入无限多变量名。
  - **[已修] JIT 同一 Interp 内的旧 ByteCode 条目**：新增
    ByteCodeRetireHook，proc 重定义/删除/local 出栈时按 ByteCode 指针
    驱逐 classic wasmi/wasmtime pool/module、emit 模块与 fast module
    缓存；不再等池满 64。
  - **[开放] classic pool 键仍含裸指针**：驱逐消除了主要滞留，但
    `(interp, ByteCode, epoch)` 地址复用 + epoch 相同的理论假命中仍在
    （概率极低；彻底修复需 generation/keepalive）。
  - **[开放] fast Store 的 Host 槽**：按 ByteCode 驱逐了 module，但
    fast store 内的 Host（source Rc、arena 基线）仍随 store 存活到
    Interp teardown / store rebind（slot 上限 64）。
  - **[开放] 后台子进程退出策略**：handle 已保留并 try_wait reap；
    Interp drop 时仍在运行的子进程不 kill（与 Tcl 一致，由 OS 接管）。
  - **[非差异] reflected finalizer 在解释器退出不跑**：实测 tclsh 8.6.17
    对仍打开的 reflected channel 退出时也不调用 `finalize`；显式
    `close` 才调用。rtcl 现状与 oracle 一致，不是资源泄漏。
  - **[开放] `references` 只增不减**：`ref` 每个值插强引用表，
    `collect` 是 no-op；脚本可无界持有 Value，直到 Interp drop。
  - **[已修] `const_pool` 旧 `ProcDef`/ByteCode 钉住**：proc 重定义、
    rename 删除、namespace delete/import 覆盖、local proc 出栈时，
    删对应 const_pool 条目并清 CmdSite 的 `def` 强引用。
  - **[开放] `namespace delete` 未清 exec traces**：命令 trace 表按
    删除/fire 处理，但 `exec_traces` 在命名空间整删路径没有对应
    wipe/rekey。
  - **[已修] 线程级 FREELIST 退出泄漏**：TLS 改为带 Drop 的 FreeList，
    线程结束时把回收盒真正 dealloc（上限 8192，约 0.6MB/线程）。
  - **[开放] `encoding` 全局缓存跨 Interp 无清理**：只有
    `encoding dirs` 会清；多 Interp/多编码场景可增长。
  - **[开放] CLI `process::exit` 绕过析构**：`exit`/顶层错误直接
    `std::process::exit`，所有 Drop（含 ChannelTable close、pipe
    wait）不执行；这与 Tcl 退出语义一致，但资源回收依赖 OS。

## 契约要点（allium）

`memory-lifecycle.allium` 把资源生命周期写成 8 条规则：

1. `LastValueHandleReleasesPayload`：最后一个 Value handle 释放即清
   payload（不是等解释器退出）。
2. `ClosingChannelReleasesOsHandle`：close/teardown 必须释放 fd、
   kill/wait pipe 子进程、跑 reflected finalizer。
3. `PoppingFrameReleasesLocals`：出栈释放 locals/upvar/local procs/
   deferred scripts，复用的帧不残留上一调用的 Value。
4. `CacheInsertionRespectsLimit`：所有长生命周期缓存有上限，插入时
   evict/clear，不允许脚本可控无界增长。
5. `DestroyingObjectReleasesGraph`：销毁对象/类释放其方法体、编译体、
   per-object 链缓存。
6. `ShutdownReleasesEverything`：最后一个 Runtime handle drop 后
   values/channels/frames/objects/JIT modules 归零、caches 清空。
7. 两个 invariant：released Value 不保留 payload；closed channel 不
   保留 OS handle；cache entries <= limit。

## 证据（文件:行）

| 关注点 | 证据 |
|---|---|
| Value 立即释放 | `crates/rtcl-vm/src/value.rs:566`（Drop）、`:595`（drop_last_owner）、`:514-531`（COW） |
| 无 Rc 环 | `interp/commands/oo.rs:76-124`（proc_memo/head_cache）、`interp/call.rs`（UpvarLink 索引）、`interp/commands/trace.rs`（回调仅 String） |
| 帧清理/池 | `interp/call.rs:728-750`、`:1426-1457`（pop + 清字段），`:106` 起（frame_pool） |
| 一次性状态泄漏（已修） | `interp/call.rs:30-52`（take 移到 depth check 前） |
| ChannelTable Drop（本轮新增） | `crates/rtcl-vm/src/channel.rs:625` |
| Pipe close/wait | `channel.rs:1209-1214` |
| OO 销毁清缓存（本轮新增） | `interp/commands/oo.rs:1765` 起 |
| OO chain memo 结构/上限 | `interp/commands/oo.rs:349-358`、`:1411-1447` |
| const_pool CmdSite 持有 ProcDef | `interp/mod.rs:320-347`、`:1273-1300` |
| var_link 内层上限（本轮新增） | `interp/commands/namespace.rs:184-190` |
| references 只增 | `interp/commands/introspect.rs:189-203`、`:233-240` |
| fast arena 基线/截断（本轮新增） | `rtcl-jit/src/host.rs:32`、`:579`；`fast/wasmi.rs:692-694`；`fast/wt.rs:702-704` |
| fast Store rebind（本轮补 wasmtime） | `fast/wasmi.rs:526-536`、`fast/wt.rs:541-555` |
| JIT 线程级 pool/module 无 Drop | `classic/wasmi.rs:16-34`、`classic/wt.rs:35-48`、`fast/wasmi.rs:55-65`、`fast/wt.rs:66-82` |
| exec & forget Child | `interp/commands/exec_cmd.rs:265-273` |
| namespace delete 不清 exec_traces | `interp/commands/namespace.rs:532-588`（只 fire/retain，无 wipe_cmd_exec_traces） |
| CLI 直接 exit | `crates/rtcl-cli/src/main.rs:75/80/104/114/131` |

## 第二轮修复（cedd02c 之后）

7. **JIT teardown hook**（`interp/vm_exec.rs` + `interp/mod.rs` +
   `rtcl-jit/*`）：`Interp` Drop → 线程级 wasmi/wasmtime pool/module 按
   Interp 指针 retain、fast store/module 清理、共享 emit 缓存整清。
8. **后台 exec reaper**（`exec_cmd.rs`/`event.rs`/`mod.rs`）：不再
   `mem::forget`；Child 入表，exec/update/Drop `try_wait` 回收。
9. **namespace delete 清 exec/cmd traces**（在 delete trace 触发**之后**
   扫除，避免破坏 trace-34.4/34.5）。
10. **encoding 缓存上限 256**（Codec 与 Table 两个全局缓存；未知编码名
    也会缓存负项，此前脚本可控无界）。
11. **CLI exit 前 drop(interp)**：`exit`/顶层错误不再让
    `std::process::exit` 绕过 ChannelTable Drop 与 JIT teardown。

## 第三轮修复

12. **const_pool 即时清理**：新增 `purge_proc_artifacts(def)`；在
    proc redefine、rename→空、namespace delete/import 覆盖、local
    proc 出栈处删除旧编译单元的 pool 条目，并清所有 CmdSite 中指向
    旧 `ProcDef` 的 `def` 引用。
13. **FREELIST TLS destructor**：回收盒在线程退出时 dealloc，不再把
    最多 8192 个 `ValueInner` 盒子留给进程生命周期。

## 第四轮修复

14. **ByteCode 退役钩子**：`ByteCodeRetireHook` + `rtcl-jit` 的
    `retire_bytecode(ptr)`；`purge_proc_artifacts` 触发，驱逐 classic
    wasmi/wasmtime pool/module、共享 emit 模块、fast module 缓存。

## 本轮修复与验证

1. **ChannelTable Drop**（`channel.rs`）：Interp 析构时逐个 close；文件
   flush/close、pipe 关管道并 `wait()`。显式 `close` 行为不变。
2. **Fast JIT arena 截断**（`host.rs` + `fast/wasmi.rs` + `fast/wt.rs`）：
   `Host.arena_base` 记录常量基线；每次顶层 fast 调用返回前
   `arena_truncate(base)` 并清 `foreaches`。返回值已 clone，安全。
3. **Fast Store rebind**：wasmtime fast store 发现 `sh.interp` 变化时
   重建 store 并清 `WT_FAST_MODULES`；wasmi 路径同样清
   `FAST_MODULES`，避免跨 Interp 悬垂/滞留。
4. **OO 销毁即时释放**：`detach_object` 清 `obj_cache/my_cache`，
   删/清 `chain_memo(_priv)`，清 `key_memo`。
5. **一次性调用状态**：`call_proc` 在深度检查前 take
   `frame_prelink/frame_level0_args/frame_level0_empty`，超深错误不再
   把 Rc/Value 留到下一次调用。
6. **var_link 内层上限**：单命名空间 256 个名字，超出整内层 clear。

验证：`cargo test -p rtcl-vm --lib` 840/840；judge interp/jit
88/88；probes 无回归（64 fail 基线）；jitsh fast-tier 三路 88/88；
wasm32 构建通过。OO churn 探针在 480 次 create/call/destroy 后
`info commands MemClass*/memobj*` 为 0、命名空间只剩 `::oo ::tcl`；
RSS 的剩余增长主要来自分配器保留（对象/命令计数不增长），不是
注册表泄漏。

## 建议的后续优先级

1. **P0**：给 `rtcl-jit` 增加显式 `teardown(interp)`（清该 Interp 的
   pool/module 条目），CLI/embedder 在 Interp drop 时调用；classic
   pool 键加 generation/keepalive，避免指针复用假命中。
2. **P1**：`exec &` 子进程 reaper（每 event loop tick try_wait，Interp
   drop 时 kill/wait 或显式 detach）；`references` 实现真正 collect
   （至少 `collect` 统计并释放无外部 handle 的引用）。
3. **P1**：`namespace delete` 清 `exec_traces`；`const_pool` 在 proc
   redefine/delete 时移除对应 CmdSite 的 def 强引用（或改 Weak）。
4. **P2**：`encoding` 缓存加 generation/上限；FREELIST 在 thread 退出
   前主动释放（TLS destructor）。
