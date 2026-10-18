# 评审材料 — `perf/interp-arch-opt`（解释器性能批次）

> **状态**：29 个提交，全部通过四道门；分支已推送 origin，不合并 master
> （由协调者处理）。本文件是评审入口：范围、实测指标、门禁、语义风险评估、
> 生态兼容与后续路线。

## 1. 范围与目标

- 目标：解释器性能。主指标 = `perf stat -e instructions:u` 的指令数比
  （rtcl/tclsh，wasm 视角更贴近）；wall 用交错 best-of-N 做验证
  （机器负载噪声大）。
- 语义最高裁判 = 系统 **tclsh 8.6.17**（`judge` 差分 + `sweep` 分歧扫描），
  红线是 byte-2-byte 兼容。
- 不引入新依赖；JIT/其它生态组件允许被 break，但必须给可跟进的说明
  （见 §5）。

## 2. 实测指标（起点 master f25d234 → 分支最新）

| bench | 指令比（起点→现在） | wall（5 次 best-of） |
|---|---|---|
| fib25 | 1.32x → **0.85x** | 0.87x |
| fe_bench | 1.82x → **0.93x** | 0.83x |
| dloop | 1.01x → **0.68x** | 0.80x |
| oo_bench2 | 1.07x → **0.48x** | 0.55x |
| oo_bench2_vars | 1.59x → **0.58x** | 0.70x |
| build-only（fe 的 build 段） | 1.29x → **0.74x** | — |
| sum_only | 0.78x → **0.59x** | 0.51x |

目标带 0.5–0.6x 指令（1.2x 为下限）：oo_bench2 0.48x、oovars 0.58x、
sum_only 0.59x 已入带；fib/fe 仍 ~0.85/0.93（见 §6）。

## 3. 手段清单（按提交轮次）

| 轮次 | 提交 | 内容 |
|---|---|---|
| A | `6892c48` `da44359` | `exec_op` 热径内联 hot lane（共享 `#[inline]` 语义体） |
| B1–B3 | `c7e8f57` `fb931c3` `203ef36` | Bool rep 快路径；`has_aliased` 帧级短路；`Value::drop` 分层、`as_int/as_float` 拆分、unchecked op fetch |
| D1–D3 | `bd71f9e` `2cd2669` | OO/namespace 表 Fx 哈希；派发链名字 `Rc<str>` 共享、prelink 记忆化、upvar 链接 Rc |
| E1 | `f6f446c` | 短名 `(` 扫描内联；dead-link/parent-ns 快速短路 |
| F | `3964c97` | **nanopass 融合 pass** `fuse_pairs`（跳转靶点感知）+ 槽对窗口 op |
| G | `ce66ee4` | **name-keyed 超指令族** + `ByteCode::unfused()` 桥 + `docs/JIT-OPCODE-DELTA.md` |
| H | `6f94395` | meta op/循环控制融合（`SwapToNextMark`/`ForeachNextClose` 等 11 op）+ class-A 靶点重定向 |
| I | `6250a5a` | 语句级融合 `SlotSetAddSS`/`VarSetAddVV` |
| J | `99cea73` | foreach 绑定快路径 + body-mark 折叠 |
| K | `160fe2b` | 融合分支直取布尔 `cmp_bool` |
| L | `3754647` | **`word_srcs` 借用切片**（`WordSrcs` ptr+len，去每命令 Rc 税） |
| M | `25ad64e` | fib 参数括号窗口族 8 op + 热径布局调优 |
| N | `957e183` | statics `Rc` 化 + alias/link 侧向量懒化 + 新帧预分配 |
| O | `e9662bb` | OO 变量表 `Rc` 化 + memo 指针校验 |
| P | `b11a6ff` `c3fe292` | 删除演示 VM/死 API；**interp 迁入 `rtcl-vm`**（`rtcl-core` = facade） |
| Q | `3816f54` | OO prelink 注册一次性 + `(qualified,ns_part)` 预计算 |
| R | `ace3259` | 去冗余 seed（活链接语义）+ memo 身份补 def ns（探针抓到的跨实例串扰） |

## 4. 门禁与语义风险

- `cargo test --workspace`：**1256/0**；`judge`：**88/88**（含本轮新增回归）；
  `judge/sweep.sh`：**0 分歧**。
- 本轮 judge 抓到的真 bug（均已成回归）：
  1. 融合臂 fall-through 多 +1 吞掉 `BEGIN_CMD`（错误帧丢失，`gen_proc-old`）；
  2. `ForeachNextClose.body` 漏进 `strip_nops` 重映射/靶点位图（judge 8 例 + panic）；
  3. class-A 融合的 `continue` 落点需靶点重定向（`control_flow`/`gen_lmap`）；
  4. **OO 跨实例变量串扰**（Q 批预计算引入，探针发现）→ 修 memo 身份 +
     新增 `judge/corpus/oo_instance_vars.tcl`。
- 两个**回退的实验**（记录在 `bench/BASELINE.md`）：`int` 原位快车道全量与
  定向（只加在 `SET_SS_ADD`）**均净负** —— 该架构下热 op 内加分支会牵动
  LLVM 寄存器分配/布局；per-op 语义体已到实用地板（~80–100 instr/op）。

## 5. 生态兼容

- **JIT**：`ByteCode::unfused()` 桥 + `docs/JIT-OPCODE-DELTA.md`（逐 op 语义/
  窗口表/建议 lower/降级保真三件事；JIT 编译边界已接线 → 接受集零改动、
  golden 全绿、零回退损失）；原生 lower 列为可选优化。
- **crate 布局**：解释器迁入 `rtcl-vm`（VM = Value/Error/VmContext + interp +
  types + command/host/parser/channel）；`rtcl-core` 为 facade（路径/feature
  全透传，下游 import 零改动）。`VmContext` 保留（JIT/wasm ABI 回调依赖）。
- `rtcl-vm` 的 `execute()` 入口自动 `unfused()`（canonical 集对齐）。

## 6. 已知剩余与下一步（详见 TODO.md）

- **fe 0.93x / fib 0.85x**：per-op 税（dispatch + 帧/槽访问 + rc）为架构地板；
  再进一步需要更激进结构（寄存器式/专用循环形）或表示层重构。
- **dloop 0.68x**：派发式 `incr acc [dict get …]` 的每次 2–3 次名字扫描 +
  dict 插入（摊销固有）→ 需 `VarMap` 键表示层。
- **下一批（表示层）**：
  1. `VarMap/VarSet/UpvarMap` 键 `Rc<str>`/`Name`（ptr-first Eq + `Borrow<str>`）；
  2. `Value` 字符串 rep 句柄共享（派发名 ptr 比较）；
  3. OO 派发站点级 memo（≈4–6% oovars）；
  4. `call_proc` 帧准备缓存（fib）。

## 7. 复现

```bash
cargo build --release -p rtcl-cli
bash judge/run.sh && bash judge/sweep.sh
# 指令数（需 perf）：/tmp/icount.sh 的逻辑见 bench/BASELINE.md 顶部
setarch $(uname -m) -R perf stat -e instructions:u ./target/release/rtcl -f bench/scripts/fib25.tcl
```
