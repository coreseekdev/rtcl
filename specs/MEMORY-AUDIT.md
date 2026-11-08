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
  - **[已修] classic pool 裸指针假命中**：`ByteCode` 新增进程唯一
    `serial`（Default/Clone 各分配新值），classic/fast 的 PoolKey 与
    共享 emit 模块键都包含 serial；地址复用不再命中旧模块。
  - **[已修] fast Store Host 槽滞留**：ByteCode 退役时按 owner+指针
    移除 slots 表项并 `Host::retire_code()` 清 source/consts/sites/
    arena/infos/foreaches；fast 调用中发生退役时延迟到下次 fast 入口
    或 teardown 执行。
  - **[已修] `references` 只增不减**：`collect` 现为真正的
    mark-and-sweep——从 globals/帧 locals+slots/proc statics/result
    出发，按**完整值精确匹配**引用 handle（list/dict 递归，不做子串
    匹配），可达引用继续展开其 value；不可达项按 handle 序跑 finalizer
    （错误忽略、至多一次）后移除，返回回收数。引用表仍是 Jim 式
    手动管理（脚本不调用 collect 时可达/不可达引用都保持），但不再
    是“collect 永远无效”的黑洞。
  - **[已修] `namespace delete` 未清 exec traces**：delete traces
    触发后扫除 `cmd_traces`/`exec_traces`（含非 proc 命令键）。
  - **[已修] `encoding` 全局缓存无上限**：Codec/Table 两缓存均封顶
    256（未知编码负项也计入），`encoding dirs` 仍整清；缓存有界，
    不再构成无界增长。
  - **[已修] CLI `process::exit` 绕过析构**：exit/顶层错误先
    `drop(interp)`，ChannelTable Drop、pipe wait、JIT teardown 正常
    执行。
  - **[非问题] 后台子进程退出策略**：handle 已保留并 try_wait reap；
    Interp drop 时仍在运行的子进程不 kill（与 Tcl 一致，由 OS 接管），
    不占解释器内存。
  - **[非差异] reflected finalizer 在解释器退出不跑**：实测 tclsh
    8.6.17 对仍打开的 reflected channel 退出时也不调用 `finalize`；
    显式 `close` 才调用。rtcl 与 oracle 一致。

**开放内存问题：无**（在上述审计口径下；引用表为 Jim 式手动
collect，后台运行子进程与 tclsh 一样交给 OS）。

补充收口（第七轮）：
- `array_generations` 无界 map → 单解释器全局 generation 计数器
  （`array get` 快照保守失效，无按数组名的累积表）。
- `chain_memo` 内层按对象上限 64（长寿命对象被无限未知方法名调用
  不再累积空链）。
- 帧出栈额外清理 `array_searches`/`array_stamps` 的 `F{idx}:` 键
  （proc 里 startsearch 未 donesearch 不再跨调用泄漏）。
- parse/bytecode cache 整清时同步清 `const_pool`（顶层 eval 单元被
  cache 驱逐后不再钉住字面量表到 2048 上限）。
- fast Store 退役改为**整 Store 丢弃**（wasm table/runs 索引无法安全
  压缩；丢整个 store 才能释放旧 Instance/Module/arena），下次 fast
  入口惰性重建；`PENDING_RETIRE` 在 teardown 清空。

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

## 第五轮修复

15. **ByteCode 进程唯一 serial**：Default/Clone 分配新 serial；
    classic/fast PoolKey 与 emit 模块缓存键加入 serial，消除裸指针
    地址复用的旧模块假命中。
16. **fast Host 槽退役**：`Host::retire_code()` + 延迟队列；proc 删除
    后 fast store 不再保留 source Rc/consts/sites/arena/foreach 宿主态。

## 第六轮修复（内存收口）

17. **references collect**：真实 mark-and-sweep + finalizer 至多一次，
    根集覆盖 globals / 帧 locals+slots / proc statics / result，
    list/dict 递归、完整 handle 精确匹配。
18. **ByteCode 进程唯一 serial**：Default/Clone 各分配新值；classic/
    fast PoolKey 与共享 emit 模块键加入 serial，彻底消除裸指针地址
    复用假命中。
19. **fast Host 槽退役**：`Host::retire_code()` + 调用中延迟队列，
    proc 删除后 fast store 清 source/consts/sites/arena/foreach，
    不再等 teardown。

## 第七轮修复

20. array generation 全局计数器、chain memo 内层 64 上限、帧出栈
    清理 array search/stamp、cache 驱逐清 const_pool、fast store
    整丢弃 + PENDING_RETIRE teardown 清空。

## 第八轮修复（embed 高水位）

21. **compile cache 总字节预算**：parse 32 MiB、bytecode 64 MiB、
    const_pool 32 MiB、lambda 16 MiB、expr verdict 16 MiB；任一超限即
    `clear_compile_caches()`（parse+bytecode+const 同族整清），不再只
    按 1024/2048 条数封顶。`ByteCode::estimated_heap_size()` 提供保守
    占用估算（源/常量/ops/sites/foreach）。

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

验证：`cargo test -p rtcl-vm --lib` 846/846（含 6 条 collect 测试）；
judge interp/jit 88/88；probes 337 pass/64 fail（基线，objquad
计时抖动除外）；jitsh fast-tier 三路 88/88；wasm32 构建通过。OO churn 探针在 480 次 create/call/destroy 后
`info commands MemClass*/memobj*` 为 0、命名空间只剩 `::oo ::tcl`；
RSS 的剩余增长主要来自分配器保留（对象/命令计数不增长），不是
注册表泄漏。

## 可选加固（非开放内存问题）

1. 引用表采用 Jim 式手动 `collect`；如产品要求“从不调用 collect 也
   有界”，可在 `ref` 插入超过阈值时自动触发一次 collect（语义变化，
   默认不做）。
2. `encoding` 缓存已封顶；如需热切换编码文件而不调用 `encoding dirs`，
   可再加 generation 失效（当前 `set_dirs` 已整清）。
3. 后台子进程在解释器退出时有意不 kill（tclsh 语义）；宿主若需要
   强清理可在自身退出路径显式 `exec kill`/等待。
