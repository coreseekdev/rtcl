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
| cases / cases2 / probe_round3 | parse | rtcl 命令替换词惰性文本使 parse 期错误退化为执行期（帧链多一层、引文完整）；tclsh 前置整条命令 parse（单帧 + 出错点截断引文）。历史 case 号 e8/e9（内层帧记账）已拆出收口（2026-11-23 192d698） |
| oo2 | var/trace | r4 行 var 物化序（tcl_order **投入产出比**边界——技术上可修、成本否决）——auto_index 首建已修（2026-11-22b 2912c81，桶位一致）；残差仅 e/errorCode ghost-insert 序一行 |
| probe7 | unicode | D-27【策略级】码元 vs 码点（len/index 对 astral 字符）+ `string byterange` 为 rtcl 有意扩展（STRING_EXTRA_SUBCMDS，8.6 无此子命令） |
| trv_p11 | var/trace | 五处语义缺陷已修（2026-11-23 7272638：冒号串分段键/upvar 元素占位与 dangling/ns delete 内外区分）；残差仅 t2-vars 序行 = oo2 族 ghost-insert 同源（定调并入 oo2） |
| utf_U_parsehex | unicode | D-27【策略级】：surrogate U+D842 在 tclsh 8.6 可存为单码元，rtcl（Rust char）替换为 U+FFFD |

已修复移出（2026-11-22b，探针移回 judge/probes/）：trv_d13/trv_p10
（9ef6f88 ns delete 两阶段拆除）、pidx（9d9e971 词首引号纪律）、
we5（378d980 err_pending_top 单元相对行）。

已修复移出（2026-11-22c，探针移回 judge/probes/）：globprobe
（引文终点 = EOF 时最深未闭合 `[` 的 opener——parse_cmd_sub 开
opener 栈钉定，白盒对照 tclParse.c ParseTokens bracket 分支逐命令
循环 + term 上浮语义；known 12→11）。
