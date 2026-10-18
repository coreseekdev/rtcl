# 评审材料 — `perf/interp-arch-opt-2`（解释器性能第二批）

> **状态**：20 个提交（其中 4 个是纯实验记录/审计/设计文档，2 个负结果已回退并留档），
> 四道门全绿；分支已推送 origin，**领先 master 20 / 落后 0**（未与 master 合并，
> 由协调者处理）。本文件即 PR 描述：范围、实测、手段清单、门禁、语义风险、
> 生态兼容与未做项。

## 1. 范围与目标

- 承接 PR #1（`perf/interp-arch-opt`，已合并），主指标不变：`perf stat -e
  instructions:u` 的**指令数比（rtcl/tclsh，wasm 视角）**；wall 用交错 best-of-N 佐证。
- 语义红线不变：系统 **tclsh 8.6.17** 差分（`judge` 88 例 + `sweep` 分歧扫描），byte-2-byte。
- 用户指定的两条线：**OO 派发深水区**（站点级 memo、`my` 快径、prelink/head_cache）
  与 **call_proc 帧准备**；副线：Value 句柄共享（设计）、while 尾融合、站点惰性化审计。
- 允许 break 生态组件但必须给跟进契约（JIT 见 §6）；不新增依赖。

## 2. 实测指标（起点 = 合并后 master → 分支最新）

指令数比（`bash /tmp/icount.sh`）：

| bench | 起点 | 现在 | 绝对值（现在） |
|---|---|---|---|
| fib25 | 0.85x | **0.83x** | 493.75M |
| fe_bench | 0.92x | 0.92x | 576.31M |
| dloop | 0.68x | 0.68x | 2978.66M |
| oo_bench2 | 0.49x | **0.42x** | 3781.13M |
| oo_bench2_vars | 0.58x | **0.52x** | 4889.20M |
| build-only(x30) | 0.72x | 0.72x | 3371.21M |
| micro_base | 1.08x | 1.08x | — |
| micro_incr | 1.33x | 1.33x | — |
| micro_add / micro_lappend | — | 0.89x / 0.72x | — |

wall（`bash /tmp/heavy.sh 3`，best-of-3，机器噪声大）：

| bench | 起点 | 现在 |
|---|---|---|
| fib25 | 0.79x | 0.79x |
| fe_bench | 0.87x | **0.85x** |
| dloop | 0.86x | **0.79x** |
| oo_bench2 | 0.56x | **0.46x** |
| oo_bench2_vars | 0.69x | **0.58x** |

新微基准 `/tmp/my_bench.tcl`（a↔b 交替 `my` 递归，2.1M 次 `my` 调用）：
17.466G → **11.761G（−32.3%）**；`dispatch_dynamic.tcl`（`$o $m` 变量名派发）
4.448G 与字面量派发同速。两脚本内容见 `bench/COMPARE.md` 的微基准矩阵附录/台账。

## 3. 手段清单（按轮次）

| 轮次 | 提交 | 内容 | 结果 |
|---|---|---|---|
| S/T | `ee7dba5` | VarMap RC 键全量实现；骨架融合首版 | 中性偏负 / 破防 judge → 均回退，根因留档 |
| U | `81dfb65` | 循环骨架融合**修复版**：`[SwapToNextMark, BeginCmdIncr*, Jump]` → `ForNextIncrLocal/Var`；根因 = 融合 op 的跳转靶点漏进 `strip_nops` 重映射/靶点位图 | micro_base 1.33→1.28x、build 0.75→0.72x、fe 0.93→0.92x |
| V | `93db54f` | 死推入消除 `[Push*, Pop]→Nop` + micro 成本归因表 | micro_base 1.28→**1.08x** |
| W | `ae63d5b` | 站点惰性化**审计**（8 读点清单 + 成本对照） | 收益 15-20 instr/命令 vs 高风险 → **不做**（留档） |
| X | `b72a771` | prelink 槽位预计算（安装免 `slot_index_of` 扫描） | oovars 5536M→5499M |
| Y | `94957ef` | `head_cache` 身份比较（`same_allocation`）+ `ChainEntry` 预取 owner 变量表 | oo2 4394M→4284M、oovars 5499M→5389M |
| Z | `d85033d` | while 体尾融合 `[BeginCmdIncr*, Jump]` → 单 op | **+1.54%** → 回退留档 |
| AA | `04fd3ed` | **OO 派发站点级 memo**（`OoCallCache`：ns 指针 + `mutation_ctr` + 两词分配身份，四项验证） | oo2 −10.5%、oovars −8.3% |
| AB | `f8c3007` | `my` 单槽快径 + 免 `format!` 门禁 | −2.89% / −0.01% → 均回退留档 |
| AC | `14b0cc9` | 4 号画像配方 + 5 号 `Value` 句柄共享设计稿 | 文档 |
| AD/AE | `e5ddbe3`…`7906d0f` | fib25 符号化画像（`call_proc` self 10.73%）+ 行级归因（绕过 3 条 perf 工具坑，`sym+offset` + 批量 `addr2line`） | 画像数据入库 |
| AF | `d785ca5` | call_proc 去每调用三连 `Rc::clone`（def 经引用读，尾调用才取 owned） | fib −1.17% |
| AG | `6cbe978` | **`my` 双槽派发缓存**（对象派发保持单槽——双槽 +7 instr/调用实测回退） | `my_bench` **−32.3%** |
| AH | `aa5b3cf` | call_proc 批 1（`take()` 守卫）+ 批 2（ns 身份写 + 池化帧保留 ns/call_ns） | fib −0.24% / −1.31% |
| AI | `6b75526` | call_proc 三次帧借用合并实验 | +0.71% → 回退留档 |

**item 4（call_proc）累计**：fib 507.46M→493.75M（−2.70%），oo2 −1.38%、oovars −1.01%。
**item 5（`Value` 句柄共享）**：S1 被 AA/AG 提前兑现，S2 经成本/收益评估**有证据关闭**，
S3 原计划不做；红线与准入标准保留在 `docs/DESIGN-value-handles.md` §1.5/§1.6。

## 4. 门禁（分支最新，本轮全绿）

- `cargo test --workspace`：**1256 passed / 0 failed**
- `./judge/run.sh`（tclsh 8.6.17 差分）：**88 pass / 0 fail**
- `./judge/sweep.sh`（分歧扫描）：**total-diffs: 0**
- 指令数/`wall` 见 §2；每个落地轮次单独跑过同一套门。

## 5. 语义风险与验证要点（评审关注）

1. **站点缓存的身份+戳推理链**（AA/AG）：命中判定 = `mutation_ctr` 相等 ∧
   `current_namespace` `Rc` 指针相等 ∧ 两词 `Value::same_allocation`。推理是
   **单向**的：identity ⟹ 文本相等；表结构变更一律由 `mutation_ctr` 覆盖，
   命名空间切换由 `Rc` 指针覆盖；缓存条目自身持有 `Value`/`Rc` 句柄，无 ABA。
   `my` 路径仍**逐调用**跑 `frame_in_object_ns` 门禁（帧栈相关，不进缓存）。
2. **池化帧保留 `ns`/`call_ns`**（AH）：帧入池不再清空、复用时按指针相等
   决定是否改写；读点已逐处核对（`call_ns` 仅 `upvar_target` 的 Bottom 分支，
   `frame.ns` 只在进入定义命名空间后读取）。池化深度 ≤32，保留至多两个
   命名空间 `Rc`。
3. **`take()` 守卫**（AH）：`frame.tailcall`/`frame_level0_args` 的裸 `take()`
   改为 `is_some()` 守卫；等价性 = 未武装时两者都返回 `None` 且不写内存。
4. **回退实验全部留档**（Z/AB/AI/S/T/W）：每条都记录了测量值与根因判定，
   `bench/BASELINE.md` 可核对「为什么不做」。
5. 新增融合 op 的靶点/计步规则见 §6 与 `docs/JIT-OPCODE-DELTA.md` §1 Round U 增补。

## 6. 生态兼容（JIT 侧）

- 本分支新增 **`ForNextIncrLocal` / `ForNextIncrVar`**（`for` 尾骨架融合，
  窗口 3 op → 1 op）。契约已登记到 `docs/JIT-OPCODE-DELTA.md`「Round U 增补」：
  展开形状、靶点双向义务（`collect_jump_targets` + `strip_nops` 重映射）、
  回边计步口径（`op_jump_from(..., st.pc + 1)`）。
- `ByteCode::unfused()` 边界接线**不变**，JIT 的 `classify` 接受集**零改动**
  （`unfused_covers_every_family` 持续覆盖）。Round Z 的实验 op 已回退，
  不在本分支表面。
- 其它组件：`rtcl-core` facade 透传不变；`Value`/`Interp` 的对外类型未变
  （新增字段均为内部）。

## 7. 未做项与后续候选（评审后可另开批次）

- fe/dloop/build 三板仍未入 0.5–0.6 目标带；fib 0.83x。
- call_proc 剩余：`slot` 清空族与 `frames.last_mut` 合并（AI 实测净负，
  需要新的思路）。
- `micro_incr` 1.33x / `micro_base` 1.08x：单 op 语义体 ~80–180 instr 的
  架构地板，突破需要更深的重构。
- `Value` 句柄共享 S2（拼出来的名字）；OO `my` 之外的计算名派发。
- 评审遗留 4 项（TODO「评审跟进」节）：ctor/dtor prelink 双轨、
  `oo::objdefine renamemethod` 类方法静默成功、PrelinkEntry ns 重建不重注册、
  Tier1 计步文档。

## 8. 复现

```bash
cargo build --release -p rtcl-cli
bash /tmp/icount.sh                 # 指令数板（fib/fe/dloop/oo/oovars/build）
bash /tmp/heavy.sh 3                # wall（交错 best-of-3）
./judge/run.sh && ./judge/sweep.sh  # 语义门禁
cargo test --workspace
# 微基准（本 PR 新增）：
perf stat -x, -e instructions:u ./target/release/rtcl -f /tmp/my_bench.tcl
perf stat -x, -e instructions:u ./target/release/rtcl -f /tmp/dispatch_dynamic.tcl
```
