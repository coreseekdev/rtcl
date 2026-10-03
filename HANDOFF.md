# rtcl 工程 Handoff — 多轮修正 → JIT 全任务交接

> 最后更新：2026-10-02（修正轮已收敛 99.9%，JIT 阶段启动）
> 用途：任一 agent 读本文件即可接管全部剩余工作，无需对话历史。

## 0. 项目与仓库

- 仓库：`git@github.com:coreseekdev/rtcl.git`，master 工作树 = `/home/nzinfo/src.note/rtcl-master`（持久盘；2026-10-03 从 /tmp tmpfs 迁出——/tmp 重启即失，工作成果必须及时 commit+push），分支 `master`。
- 定位：Rust 实现的 Tcl 解释器（jimtcl 血统的轻量定位），target = native + **wasm32-unknown-unknown（预期主路径）** + wasm32-wasip1 + embedded no_std。
- crate 结构：`rtcl-parser`（递归下降解析 + ByteCode 编译）、`rtcl-ir`（OpCode 三层：primitive / `Call(CmdId)` 0..127 stdlib 128+ 扩展 / `DynCall`）、`rtcl-vm`（execute.rs dispatch 循环 + `VmContext` trait + Value/Error 唯一实现）、`rtcl-core`（Interp + 内建命令，`value.rs`/`error.rs` 只是 re-export）、`rtcl-cli`、`rtcl-wasm`、`rtcl-expect`、`rtcl-jit`（ByteCode → wasm 发射器，JIT M0 起）。
- Tcl 官方源码（行为 oracle 的测试语料）：`/home/nzinfo/src.note/rtcl/.refer/tcl`（已 gitignore；如被删重新 `git clone --depth 1 https://github.com/tcltk/tcl.git .refer/tcl`）。
- 系统 oracle：`/usr/bin/tclsh` = Tcl 8.6.17。**一切语义以 tclsh 实测为准，不凭记忆。**

## 1. 基础设施（已建，勿重建）

| 资产 | 路径 | 用法 |
|---|---|---|
| judge parity harness | `judge/run.sh` | `./judge/run.sh`（全量）/ `-v <name>`（单文件带 diff）。递归 corpus/ 子目录，15s timeout/文件 |
| 手写语料 | `judge/corpus/*.tcl` | 10 个，全绿 |
| 官方测试集语料 | `judge/corpus/gen/gen_*.tcl` | 77 文件 / 4072 case，由提取器生成 |
| 提取器 | `judge/extract/extract.py` | 保守提取 tcltest 用例，oracle 自验证；勿轻易改协议 |
| burndown 报表 | `judge/extract/burndown.py` → `judge/BURNDOWN.md` | 生成物，不入 git |
| allium 行为规格 | `specs/*.allium` | 6 域：interp/expr/list/string/variables/control-flow |
| 分歧清单 | `specs/DIVERGENCES.md` | 格式：行为声明 \| Tcl 8.6 \| rtcl \| 复现脚本 \| 严重度；修复后标 `[FIXED YYYY-MM-DD]` |
| 对拍探针 | `specs/probe.sh` | stdin 逐行 Tcl 片段，自动 tclsh vs rtcl diff |
| 性能基线 | `bench/run.sh` → `bench/BASELINE.md` | rtcl vs tclsh best-of-5；基线：循环/数据结构密集慢 4–16×，string_build 1.5× |
| 单测 | `cargo test --workspace` | ~746 个，必须保持全绿 |

## 2. 方法论（两条 skill 的硬规则）

- **code-migration**（`~/.claude/skills/code-migration/`）：judge 先行，任何重构/优化不许改变 judge 判据；失败三次即规则 bug（停手改规则，不修实例）；进度报 burndown 数字不报散文。
- **allium**（`~/.agents/skills/allium/`）：spec 只写可观察行为；分歧必须实际运行佐证；distill（从代码提取 spec）→ weed（spec vs 实现分歧）循环。
- **修复纪律**：以 tclsh 实测为最高裁判；最小侵入；不顺手重构；不引入新依赖（rtcl 是嵌入式定位）；子任务不 git commit，由协调者统一提交推送；并行 agent 严格文件互斥。

## 3. 当前状态（burndown）

- judge：**文件级 87/87 全绿，case 级 4067/4072 = 99.9%**（fail 5，全部在 gen_namespace-old 的 namespace/lsort/variable/proc 边缘组合，died 0）。
- 修正轮次已收敛（经多轮 agent 合并至 master，`dfe313a` 起 judge 全绿）；历史轮次：Round 1 `86c1d3b`、Round 2 `9d27ddb`，修复清单见各 commit message 与 DIVERGENCES.md 的 [FIXED] 标注（28 条）。
- 剩余 5 个 case 级失败：namespace-old 边缘语义，低优先，记录备查即可。
- **解释器快车道已落地（2026-10-02，`58c2839`..`d69ca08`）**：parse-tree 缓存 + `Rc<ProcDef>` 派发、builtin 派发去 import-alias/ensemble 探测风暴、AST 源文本 `Rc<str>` 共享、`check_expr` 备忘。judge/case 级无回退；数字见 `bench/BASELINE.md` 第二张表。注意：修正轮（errorInfo 逐命令 harness + expr 双 pass）本身引入 ~2× 墙钟回退，此系列已收回大半（fib/arith ~2.3×/2.1×）；剩余差距主因 = 逐命令 harness 与 expr 解释，Phase 3/JIT 路线均绕开。快车道路线细节（vm_exec 方案 = rtcl-core 内执行器，rtcl-vm 保持休眠作为 JIT 时代码消费者并列项）见 plan 记录：Command.text/word_srcs 已 Rc 化，ByteCode 站点表/compile-once 尚未动工。
- **字节码 VM 已落地（2026-10-03，C5+C6）**：`rtcl-core/src/interp/vm_exec.rs` 执行器成为第二条活路径——(1) C5：命名 proc 在 `proc` 定义点编译一次（`ProcDef.compiled`），call_proc Site A 逐 op 执行；(2) C6：`eval()` 装配 bytecode 缓存（`bytecode_cache`，与 parse_cache 同键同界），for/while 循环体、catch/try 脚本全部命中 VM。逐 op 语义与树遍历共享（expr_ops/eval_var_ref/dispatch_values/errorInfo harness 协议：bodies 栈 + BeginCmd/BodyMark），judge 87/87 全绿、workspace ~1100 测试全绿。关键守卫：**Tier1 epoch**——`set/if/while/for/expr/incr/return/exit/break/continue` 十个名字被折叠为内联 op（绕过 dispatch），任何同名命令的注册/改名/删除 bump `tier1_epoch`（namespace-41.1），不匹配的编译体回退树遍历；fallback 白名单、`RTCL_NO_BYTECODE`、exec traces 同样钉回树遍历。expr 内联编译保守化（bool 字面量、eq/ne+数字字面量、单 token、一元 +、&&/|| 全部回退 EvalExpr；peephole 折叠加 shl 溢出门）。bench：arith 1.62×、fib 1.83×、var_incr 2.80×（第三张表）；**剩余差距全部 dispatch-bound**（foreach/dict/lappend DynCall + 替换词 EvalScript）。
- **JIT M0 已落地（2026-10-03）**：`crates/rtcl-jit` crate——rtcl-ir `ByteCode` 的第二个消费者。发射器（`emit.rs`，wasm-encoder 0.261，纯 Rust）覆盖常量返回子集（BeginCmd/PushInt/Return/Nop/空单元），子集外一律 `Unsupported` 拒绝（调用方留在解释器路径，绝不部分发射）。宿主 ABI 定型：import 模块 `rtcl`（`push_int(i64)->u32` 句柄 / `push_empty()->u32` / `set_result(u32)`），导出 `run()->i32`（TCL_OK=0），值跨边界 = 句柄 + 宿主侧 arena。round-trip 单测（wasmi 2.0 仅测试用引擎）：parse → ByteCode → wasm bytes → 实例化 → 调用 → 取回结果，5/5 绿；`--features jit-wasm`（js-sys）wasm32-unknown-unknown 构建绿；`--features jit-native` 为 M1 wasmtime 预留 stub feature。
- **下一阶段 = JIT M1（expr Int 快车道）**，验收见 §5。
- 历史裁决：
  - **Rc vs Arc**：保持 Rc（wasm 单线程模型；`2257f20` 的静默回退恰好正确）。多 worker 场景每 worker 一个 Interp，不共享。
  - **Value 类型已统一**：rtcl-core 只是 re-export rtcl-vm，无需合并。
  - **bignum 不引入**：整数溢出提升为 f64（与 Tcl bignum 的残留差异已记录在 DIVERGENCES.md）。

## 4. 修正轮次：已收敛

判据达成：judge 文件级 87/87 全绿、case 级 99.9%、无未修复的 semantic-error 级分歧。剩余 5 个 case 为 namespace-old 边缘组合，记录备查、不再追修。

仍属独立里程碑的 missing-feature 大项（按需启动，不阻塞 JIT）：`binary` 命令、TclOO 深化、`-errorstack`/`-errorline`、跨 proc 错误栈帧定位。

## 5. 待办：JIT（修正收敛后启动）

目标：rtcl compile Tcl → wasm module → 实例化执行。**预期路径 = wasm32-unknown-unknown + js-sys `new WebAssembly.Module(bytes)` 同步实例化**（<4KB 模块；per-proc 模块足够小）。

### 架构裁决（已讨论定案）

- **从 rtcl-ir OpCode 出发，不从源码出发**：JIT 是 ByteCode 的第二个消费者（与 rtcl-vm 并列）。primitive opcodes → wasm 原生指令；`Call(CmdId)` → import 调用宿主 `rtcl_cmd_call(id, args, argc)`；`DynCall`/`EvalScript`/`uplevel` → 回调 VmContext 同解释器路径。
- **粒度 = per-proc module**（Mono jiterpreter 模式）：实例化亚毫秒，失效 = 丢弃单实例。
- **跨边界值表示 = u32 handle + 运行时侧 `Vec<Value>` arena**；不 import 主实例线性内存（wasm-bindgen 内存导出方案脆弱）。native 侧（wasmtime crate 嵌入）可考虑 externref，后期优化。
- **快车道 = expr Int**：`InternalRep::Int` 守卫 + i64 直接运算，守卫失败回退 runtime call。收益集中在数值循环（基线显示 4–16× 差距就在这类负载）；字符串/列表密集负载收益有限（1.2–2×），靠后或不 JIT。
- **失效 = epoch 计数器**：registry.rs 加 epoch（rename/proc redefine/trace 时 bump），JIT 入口一次内存读守卫，失败回解释器。不用反向指针表。
- **错误传播沿用 Tcl result code**（i32 返回 + out-param），不用 wasm exception。
- **平台 feature gate**：`rtcl-jit` crate + `jit-wasm`（js-sys 路径）/ `jit-native`（wasmtime 嵌入）两个 feature；wasip1/embedded no_std 永远解释执行。
- **正确性门 = 三向差分**：tclsh vs rtcl-interp vs rtcl-jit 跑同一 judge 语料（judge/run.sh 加 `--engine jit` 档），外加随机脚本 fuzz。
- **发射器**：`wasm-encoder` crate（Bytecode Alliance，纯 Rust 可编译进 wasm）。不用 Binaryen（C++ 编译进 wasm 体积大）。

### JIT 里程碑

- **M0 ✅（2026-10-03）**：`rtcl-jit` crate 骨架 + wasm-encoder 发射最小 module（常量返回子集 round-trip：编译→实例化→调用→拿回结果），双 feature 编译通过。验收达成：单元测试（wasmi round-trip 5/5）+ wasm32-unknown-unknown 构建绿。注意：测试引擎用 wasmi 2.0（纯 Rust、轻量）；wasmtime（jit-native 的真身）推迟到 M1 落地——本机 3.4GB 构建内存限制下先不引 cranelift。
- **M1**：expr Int 快车道——`expr` bytecode 子集（PushInt/算术/比较/跳转）编译为 wasm i64 指令，InternalRep 守卫。验收：judge 三向差分全绿 + `bench/cases/arith_loop.tcl` 提速 ≥3×。
- **M2**：proc body + primitive ops（变量经 handle arena）。验收：proc_fib.tcl 提速。
- **M3**：epoch 失效守卫 + rename/redefine 风暴 fuzz。
- **M4**：热点 tier-up（proc 调用计数 ~50 阈值）+ 与解释器的无缝回落。
- 每里程碑 gate：judge 三向全绿 + bench 不回退 + 单测绿。

### 参考先例

Mono jiterpreter（Blazor，.NET 7+，运行时 IL→wasm→实例化，最直接对标）；tclquadcode（Tcl 官方 LLVM JIT 实验，已停滞，其停滞原因 = 语义角落难题集中在 trace/unknown/ensemble，不在代码生成）；CheerpJ。

## 6. 并行 agent 派发模板（code-migration 纪律）

每个并行域的 prompt 必须包含：

```
你在 rtcl 项目（/home/nzinfo/src.note/rtcl，Rust 实现的 Tcl 解释器）做多轮缺陷修正的 Round N。

## 共同上下文
- 构建：cargo build --release -p rtcl-cli；快速验证：./target/release/rtcl -c '<script>'；
  oracle：echo '<script>' | tclsh（Tcl 8.6.17），一切语义以 tclsh 实测为准。
- 分歧清单：specs/DIVERGENCES.md（[FIXED] 条目别动）；行为规格 specs/*.allium。
- judge：./judge/run.sh；当前基线见 §3（修复前重新跑一遍取实时基线）。
  不要在并行构建负载下跑 judge（15s timeout 会抖动；先 cargo test 再 judge）。
- 硬约束：只许修改分配给你的文件；不许 git commit；不多修分配外条目；
  修好后 DIVERGENCES.md 标注 [FIXED <当日日期>]；cargo test --workspace 保持全绿；
  单测断言依赖旧错误行为且新行为才正确的，更新断言并在报告中说明。
- 原则：最小侵入；不顺手重构；不引入新依赖。

## 你的任务
<具体条目 + 分配文件清单 + 复现脚本位置>

## 返回
每条修正的根因一句话 + 修改位置 path:line + 验证证据（前后输出对比）
+ 单测/judge 结果 + 未解决条目及原因。
```

文件互斥参考（按域）：string_cmds.rs / expr.rs+expr_funcs.rs / value.rs+list.rs / list_sort.rs / control.rs+loops.rs / error.rs(rtcl-vm+rtcl-core)+call.rs / namespace.rs / vars.rs+call.rs。两域需要同一文件时串行，不并行。

## 7. 提交规约

- 每轮修正一个 commit，message 格式：`fix: Round N divergence burndown — <主题>`，正文列条目，尾部署名 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- 提交前必跑：`cargo test --workspace`（全绿）+ `./judge/run.sh`（无回退）。
- `git push origin HEAD`（master，已配置 ssh）。
- `.refer/`、`judge/BURNDOWN.md` 以外的新资产都可入库。

## 8. 本文件维护

每完成一轮修正或一个 JIT 里程碑：更新 §3 的数字、§4/§5 的完成状态、补充新裁决。handoff 文件自身入库（`HANDOFF.md` 在仓库根）。
