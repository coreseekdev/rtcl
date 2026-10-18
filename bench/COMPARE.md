# rtcl vs tclsh 8.6.17 — 全面性能对比

生成：`bash bench/compare_tcl.sh 3`（指令数 = `perf stat -e instructions:u`；
wall = 交错 best-of-3；主指标为指令比，wasm 视角更贴近）。

| workload | rtcl instr | tclsh instr | instr ratio | rtcl wall(ms) | tclsh wall(ms) | wall ratio |
|---|---|---|---|---|---|---|
| fib25 | 508,075,108 | 598,079,614 | **0.85x** | 38 | 39 | 0.97x |
| fe_bench | 577,505,982 | 624,042,136 | **0.93x** | 27 | 30 | 0.90x |
| dloop | 2,977,447,181 | 4,354,404,116 | **0.68x** | 194 | 246 | 0.79x |
| oo_bench2 | 4,358,122,982 | 9,016,775,113 | **0.48x** | 216 | 386 | 0.56x |
| oo_bench2_vars | 5,508,194,392 | 9,486,066,495 | **0.58x** | 292 | 425 | 0.69x |
| sum_only | 1,048,587,120 | 1,782,152,799 | **0.59x** | 47 | 82 | 0.57x |
| judge-corpus（88 文件） | 1,213,013 | 1,212,683 | ~1.00x | 1032 | 3706 | **0.28x** |
| micro_base（空 for 循环） | 444,890,884 | 337,349,826 | 1.32x | 21 | 17 | 1.24x |
| micro_incr（+`incr x`） | 533,898,682 | 391,561,609 | 1.36x | 23 | 17 | 1.35x |
| micro_add（+`set s [expr {$s+$i}]`） | 636,920,555 | 696,188,741 | **0.91x** | 29 | 29 | 1.00x |
| micro_lappend（+`lappend L $i`） | 581,975,643 | 788,088,354 | **0.74x** | 29 | 46 | 0.63x |

## 读法

- **基准套件 0.48–0.93x 指令、wall 全部 ≤1.0x**：oo2（0.48/0.56）、oovars
  （0.58/0.69）、sum_only（0.59/0.57）已入目标带（0.5–0.6x）；fib 0.85、
  fe 0.93 仍高于带但在 1.2x 下限内。
- **judge-corpus 行是启动主导**：88 个短脚本 ≈ 13.8k 指令/脚本（进程 exec +
  动态链接 + stdlib 装载都计入），解释器本体占比很小 —— 这行说明 rtcl 的
  **启动/内建 stdlib 更轻**（wall 0.28x），不是解释器吞吐的比较。
- **micro 矩阵给出真正的每迭代增量**（N=1M 迭代/脚本）：
  - `micro_add` 0.91x、`micro_lappend` 0.74x —— 逐语句层面 rtcl 已快于 tclsh；
  - **`micro_base` 1.32x / `micro_incr` 1.36x** —— 空 `for` 循环控制 + 裸
    `incr` 是我们唯一仍显著落后 tclsh 的形态（其 337 instr/迭代 vs 我们
    445；per-op 派发 + region 簿记 ≈ 80–100 instr/op 是架构地板）。
- 结论：**语句/表达式/列表/对象的执行层已普遍快于 tclsh；差距集中在
  「循环骨架」（条件+回边+incr 的极简循环）上**——后续若继续压，方向是
  循环骨架的专用形态（寄存器式/专用循环 op）与表示层（`VarMap` 键、
  `Value` 字符串句柄）。

## 复现

```bash
cargo build --release -p rtcl-cli
bash bench/compare_tcl.sh 3        # INSTR=0 可跳过 perf
```
