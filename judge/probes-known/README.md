# known-divergent probes — 归档定调清单

这个目录的探针**刻意**钉住与 tclsh 8.6.17 的已知分歧（设计差异 /
无有效 oracle / 理论边界），逐项在 specs/DIVERGENCES.md 的
2026-11-22 节有定调条目（日期 + 根因 + 处置理由）。它们**不是**
回归目标，也不是"洗白的真分歧"——每一项都标注了重新开放的
条件（依赖哪一面基建落地）。

`judge/probes.sh` 的口径：
- `judge/probes/*.tcl`（active 集）必须**全绿**，任何 DIFF 都是回归；
- 本目录的探针单独展示（KNOWN 段），不计入 fail 口径。

| 探针 | 族 | 定调一句话 |
| --- | --- | --- |
| c5_dis / cmp_unsupported / cmp_unsupported2 | disassemble | 成功 case 的 ByteCode dump = rtcl 自有指令集格式，与 tclsh 内部指令文本逐字节不兼容（错误面已全对齐） |
| dbg_basic | disassemble | tclsh 8.6.17 自身 SEGV（rc=139）——无有效 oracle |
| globprobe | parse | bracket 失败的 errorInfo 引文/行号归属在嵌套引号替换形态需复刻 tclParseCommand 层级 scan（前缀执行与报错消息已修） |
| cases / cases2 / probe_round3 | parse | rtcl 命令替换词惰性文本使 parse 期错误退化为执行期（帧链多一层、引文完整）；tclsh 前置整条命令 parse（单帧 + 出错点截断引文） |
| oo2 | var/trace | r4 行 var 物化序（tcl_order 理论边界）+ tclsh auto_load 机制的 auto_index 变量 |
| pidx | expr | expr 下标替换路径（`$foo([x"y])`）的词中引号处理与报错归属（其余 pre-pass 面已对齐） |
| probe7 | unicode | D-27【策略级】码元 vs 码点（len/index 对 astral 字符）+ `string byterange` 为 rtcl 有意扩展（STRING_EXTRA_SUBCMDS，8.6 无此子命令） |
| trv_d13 / trv_p10 | var/trace | ns delete 的两阶段拆除：tclsh 回调期间 children/未处理变量仍可见（nukeQueue 模型）；rtcl 单阶段拆除 |
| we5 | 行号 | ns eval 体出口帧行在词替换 defer 形态读 stale 日志行；精确修需 err_pending_top 载荷增加单元相对行（ei_line_attribution 钉形反向约束） |
| utf_U_parsehex | unicode | D-27【策略级】：surrogate U+D842 在 tclsh 8.6 可存为单码元，rtcl（Rust char）替换为 U+FFFD |
