# known-divergent probes — 归档定调清单

这个目录的探针**刻意**钉住与 tclsh 8.6.17 的已知分歧（设计差异 /
无有效 oracle / 理论边界），逐项在 specs/DIVERGENCES.md 的
2026-11-22 / 2026-11-22b 节有定调条目（日期 + 根因 + 处置理由）。它们**不是**
回归目标，也不是"洗白的真分歧"——每一项都标注了重新开放的
条件（依赖哪一面基建落地）。

`judge/probes.sh` 的口径：
- `judge/probes/*.tcl`（active 集）必须**全绿**，任何 DIFF 都是回归；
- 本目录的探针单独展示（KNOWN 段），不计入 fail 口径。

| 探针 | 族 | 定调一句话 |
| --- | --- | --- |
| c5_dis / cmp_unsupported / cmp_unsupported2 | disassemble | 成功 case 的 ByteCode dump = rtcl 自有指令集格式，与 tclsh 内部指令文本逐字节不兼容（错误面已全对齐） |
| dbg_basic | disassemble | tclsh 8.6.17 自身 SEGV（rc=139）——无有效 oracle |
| globprobe | parse | bracket 失败的 errorInfo 引文归属：归属行/引文起点/150-cap 已修（2026-11-22b 41a3337，gp_1/2/3 转绿）；残差仅引文终点——≥4 层引号⇄bracket 互嵌套递归的最深 opener 深度（tclsh 行 11 处截、rtcl 行 9 处截），需复刻 tclParseCommand 嵌套递归栈语义 |
| cases / cases2 / probe_round3 | parse | rtcl 命令替换词惰性文本使 parse 期错误退化为执行期（帧链多一层、引文完整）；tclsh 前置整条命令 parse（单帧 + 出错点截断引文） |
| oo2 | var/trace | r4 行 var 物化序（tcl_order 理论边界）——auto_index 首建已修（2026-11-22b 2912c81，桶位一致）；残差仅 e/errorCode ghost-insert 序一行 |
| probe7 | unicode | D-27【策略级】码元 vs 码点（len/index 对 astral 字符）+ `string byterange` 为 rtcl 有意扩展（STRING_EXTRA_SUBCMDS，8.6 无此子命令） |
| trv_p11 | var/trace | 冒号串语义/tail/qual 部分行已修（8917651），残差行维持归档定调 |
| utf_U_parsehex | unicode | D-27【策略级】：surrogate U+D842 在 tclsh 8.6 可存为单码元，rtcl（Rust char）替换为 U+FFFD |

已修复移出（2026-11-22b，探针移回 judge/probes/）：trv_d13/trv_p10
（9ef6f88 ns delete 两阶段拆除）、pidx（9d9e971 词首引号纪律）、
we5（378d980 err_pending_top 单元相对行）。
