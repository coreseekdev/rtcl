# rtcl 内存/资源泄漏审计（2026-11-20 轮，perf/wave2 @ 94890ff）

- **范围**：2026-11-13..20 全部新面（M2/M3/M4/语句窗口/errorInfo
  sink/eval_commands 深度位/帧池/host-http/clock 剥离/lazy-string
  HostStr 池/clockpkgmod/JS dynmod glue/worker/JIT 池），逐面对照
  「资源生命周期设计 vs 实际释放路径」。
- **方法**：静态审计（全释放路径核对）+ 长跑驻留差分探针（native
  max-RSS 梯度、wasm `memory.buffer.byteLength` 梯度、JS heap）+
  oracle（tclsh 8.6.17）行为对照。judge/probes 全绿不作为内存结论
  依据（行为绿 ≠ 无泄漏）。
- **分级**：P0 = 真实泄漏（无界/永不回收）；P1 = 有界但过大或
  晚回收缺失（本轮修）；P2 = 设计内驻留（缓存/池有界）。
- 前轮审计（2026-11-08）：[`MEMORY-AUDIT.md`](MEMORY-AUDIT.md)；
  契约：[`memory-lifecycle.allium`](memory-lifecycle.allium)。

## 结论（TL;DR）

**窗口内新面（M2/M3/M4/语句窗口/HostStr 池/JS registry/worker）
的缓存与池生命周期全部有界且失效路径完备；三个预存面在审计点
复核中坐实为真实泄漏/晚回收缺失，已修：**

1. **P1（已修）coroutine rename 不触碰记录表**：挂起 record 成孤儿
   （帧链+冻结字节码滞留，可脚本化无界增长）+ resume-after-rename
   偏离 tclsh（新名应可续）。
2. **P1（已修）coroutine 挂起链深度位不回账**：每次挂起 call_depth
   永久 +N——约 1000 个挂起（或挂起-恢复-完成循环 ~1000 轮）即耗尽
   递归预算；resume 错误路径帧残留还破坏作用域解析（单轮即可让顶层
   变量读失败）。
3. **P1（已修）command_order 注册序日志 push-only**：命令 churn
   （协程 resume 命令、动态 proc 定义+rename away）~100B/命令线性
   增长（oracle 同型零增长）。
4. **P1（已修）rtcl-wasm JS_COMMANDS 桶无终点清理**：RtclHandle
   反复建/弃时桶按 interpId 单调增长；unregister_module 只清
   installed 条目——registered-but-never-loaded / unload 后的模块其
   JS 闭包与隐藏 proc 永久滞留。

## 逐面结论表

| 面 | 级 | 证据摘要 |
|---|---|---|
| **A1** M4 PROC_MODULES 负缓存 + emit 缓存池 | P2 | cap 1024 满则全清（`classic/mod.rs:60`，上界 ~4MB；过驱逐=性能非泄漏）；负缓存 `None` 条目随 `retire_bytecode` 回收，死亡路径全覆盖（redefine `proc.rs:164`、rename→空 `proc.rs:1094`、delete `proc.rs:1092`、ns delete `namespace.rs:587`、import 覆盖 `namespace.rs:1082`、local proc 出栈 `call.rs:723/1419`）；teardown 整清。**差分**：jitsh 10k 轮 accepted=40000，REDEF 仅 +1.3MB/10k 轮附加驻留；RENAMEAWAY 50k 轮残余 ~64B/轮、5× 迭代亚线性（统计 `RTCL_JIT_AUTOCOMPILE_STATS=1`） |
| **A3** M3 shadowed_tier1_folds 扫描集 | P2 | `shadowed_tier1_folds_in` 每次调用现建 `Vec<String>` 用后即弃（`vm_exec.rs:4734`），调用点=定义点非热路径；TIER1_FOLD_LEAVES 静态表（`compiler.rs:43`）。无驻留面 |
| **A4** 语句窗口 4 op（fuse_pairs） | P2 | 窗口数据为编译期产物，随 ByteCode/const_pool 生命周期走（2048 条 + 字节预算双界，前轮第 21 项已封顶） |
| **A5** errorInfo sink 清除批 | P2 | err_from_subst/一次性旗标均为单槽 bool；清除点已覆盖 catch（`control.rs:977`）、try on/trap（`:1504/:1592`）、seed_parse_error（`eval.rs:376`）、decode_toplevel_flow（`eval.rs:1325`）+ OO next 分派臂（`oo.rs:2535/2624`，审计确认**第五/六路径已在位**）；err_calls 每个新错误清空（`eval.rs:369/451/484`）；err_uplevel push/pop 与 eval 结果无关恒配对（`proc.rs:525-529`）。错误循环稳态=单错误量驻留 |
| **A6** eval_commands 深度位 | P2 | save/占用/restore 顺序结构，Err 路径 restore 在 return 之前（`eval.rs:664-669/694-699`）；`panic = "abort"`（`Cargo.toml:85`）无 unwind 残留面 |
| **A7** 帧池不变式 + coroutine drain | **P1→修** | 池不变式复核通过：两个 push 站点（`call.rs:780-798`、`call.rs:1525-1545`）全清，池 cap 32。**coroutine 三缺口**（见上 TL;DR #1/#2）已修；完成/错误路径 record 随 `destroy_record` 释放（含帧链+冻结 levels+深度单元）。**长跑**：修复前 rename-away 循环 n≥5000 深度爆死；修复后 50k 轮 RSS 7.4MB 平 |
| **A8** host-http HostFd/HostJobs/pipe | P2 | pipe.rs 容量有界 64KiB 背压（`pipe.rs:32/66`）；native 后台子进程沿用 background_children try_wait 收割（`exec_cmd.rs:66-80`）；HostJobs trait 无 remove/forget——生产实现尚无（仅 trait+MockHost），**定调：落地首个生产实现前须补 reap 面**（契约缺口记录，非现症） |
| **A9** clock 剥离 | P2 | 4048dcd 仅剥 wasm feature（`rtcl-vm` features 去 clock/package，cfg 门控完备）；native rtcl-cli clock/package 未动。无新面 |
| **B1** C-lazy HostStr 池 | P2 | 池 `Vec<Option<Slot>>` + 空闲栈：release 归零→槽清+索引进空闲栈（`js_string.rs:143-159`），复用非单调；acquire/release 经 `InternalRep::Clone/Drop` 与 ValueInner 生命周期严格配平（`value.rs:151-170`）——变量 unset/作用域退出/覆盖写全部汇入 Value Drop→rep Drop。**长跑**：60k 次 JS→Tcl 串 churn + 往返双态，wasm 内存 2.25MB 恒平 |
| **B2** clockpkgmod JS registry | P2 | registry = 模块级 `Map`（`makeClockPkgMod`），`package provide` 同键 set 覆写、forget 删除——无重复堆积；dtfCache 仅收合法 IANA 名（parseZone 先校验），上界 ≈ IANA 全集。包名数语义有界 ✓ |
| **B3** spencermod INPUT/RESULT | P2 | INPUT 每调用 clear+resize 复用（`spencer-side/lib.rs:105-117`），容量=输入高水位；RES 驻留=上次结果大小（有界，大结果驻留留意项） |
| **B4** ABI 版本 export | P2 | 常量 + 协商函数，无分配面 |
| **B5** JS envelope 编码 / dynmod glue | **P1→修** | TextEncoder/Uint8Array 每调用临时（GC 管）✓；**JS_COMMANDS 桶两缺口**（TL;DR #4）：unregister_module 全量内部回调（`lib.rs` unregister_module 改扫 `dyn_module_internal_commands`）+ RtclHandle Drop 清桶（`js_command::drop_interp_bucket`）。**长跑**：register/unregister 循环 1.9KB/轮（前）→ 跨 4096 阈值后 32B/轮（后）；handle 建/弃 675B/轮（前）→ 52B/轮（后） |
| **B6** worker outputs/execQ | P2 | outputs 每次 exec 开头重置（`rtcl-worker.js:394`，驻留=上次结果）；execQ/BOOT_QUEUE 启动期局部并排空。无泄漏 |
| **C1** JIT fast/classic 池 | P2 | FAST_MODULES/WASMI_MODULES/PROC_POOL cap 64 全清 + retire_bytecode/teardown_interp retain 过滤（`fast/wasmi.rs:74/116/126`、`classic/wasmi.rs:39-46/115`）+ PENDING_RETIRE teardown 清空；M2 后失效点完备。与 A1 同源负缓存（见 A1） |

## P1 修复清单（本轮落地）

| # | 修复 | 文件 | 回归数字 |
|---|---|---|---|
| 1 | coroutine rename 分支（away=destroy、新名=平移） | `rtcl-vm/src/interp/commands/proc.rs`、`coro.rs`（`destroy_record`） | rename-away 50k 轮 RSS 7.4MB 平（修复前 ~1000 轮深度爆死）；resume-after-rename `catch=0 result=r`（=tclsh 钉形） |
| 2 | 挂起链深度位随帧走（record.depth/on_stack + CoroCtx.depth_base）；resume 错误臂/越界臂 truncate 回 frames_base | `coro.rs` | 挂起-恢复-完成 3000 轮 + resume 错误 50 轮后顶层作用域/深度预算健康（修复前 900..2000 轮内死、错误路径单轮即 `can't read n`） |
| 3 | command_order 有界护栏（>4096 且 >4×live 整表重建，hidden_commands 同批） | `rtcl-vm/src/interp/mod.rs`（note_new_command） | 命令 churn 200k 轮 RSS 28784KB→7608KB（线性分量消除，边际 3.2B/轮=噪声级） |
| 4 | wasm JS_COMMANDS 桶终点清理（RtclHandle Drop）+ unregister_module 全量内部回调（`dyn_module_internal_commands`） | `rtcl-wasm/src/lib.rs`、`js_command.rs`、`rtcl-vm/src/interp/commands/dynmod.rs` | register/unregister 循环 1.9KB/轮（前）→272B/轮且跨阈值后边际 32B/轮（后）；handle 建/弃 675B/轮（前）→52B/轮（后） |

回归测试：`crates/rtcl-vm/tests/probe_coro_mem.rs`（4 测）+ `probe_cmd_order_guard.rs`（1 测）。

## 回归门禁（修复后全绿）

- judge(interp) **88/88**；judge(jit) **88/88**
- jit_battery **52/52**（三方逐字节差分）
- workspace tests **1460/0**（`cargo test --workspace --release`，含本轮新增 5 测）
- wasm：test.js **26/26**、test-worker.mjs **3/3**（含 ABI v2 校验）
- 行为钉形（tclsh 8.6.17 oracle）：挂起 coroutine `rename c ""`=删除、
  `rename c c2`=新名可续——rtcl 对齐

## 探针与坑

- 探针脚本（审计工作区 /tmp/rtcl-audit，不入库）：coro-leak.tcl /
  coro-depth-probe2.tcl / coro-control.tcl / m4-{keep,redef,rename}.tcl /
  wasm-mem.mjs（`node --expose-gc`，wasm 内存取 `init()` 解析值的
  `memory.buffer.byteLength`）。
- 坑 1：rtcl CLI 不透传脚本参数（用 `-f` + 模板生成）；jitsh 的
  interp 不加载 env 数组（`$env(...)` 不可用）。
- 坑 2：wasm 线性内存单调（grow 不还页）——不显式 `free()`（wasm-bindgen
  确定性 drop）时 handle 建/弃循环表现为 ~1MB/轮的「增长」，实为
  finalization 时机 × 单调性假象；显式 free 后同型循环 0.56MB/2000 轮。
- 坑 3：`command_order` 护栏重建在 `note_new_command`（插入点）做，
  删除点不做逐条 retain（O(n²) 陷阱），摊还 O(1)。
- 坑 4：M4 统计钩子在 while 循环体内正常触发（accepted 计数=定义数），
  与 `-c` 顶层单定义一致——预编译覆盖面结论不受脚本形态影响。
