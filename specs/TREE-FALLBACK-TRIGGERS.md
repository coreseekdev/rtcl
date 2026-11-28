# 树引擎回落触发点全清单（T13 产出，2026-11-26）

> 只列不改。这是 HANDOFF「审计后续轮」§3.3（树引擎去留）的决策输入：
> 每个触发点给出触发条件、复现、VM 补齐还缺什么。基线 migrate/ns91
> （perf/wave2 + T1/T2）。扫源：`RTCL_NO_BYTECODE`、`bytecode_applicable`、
> `fallback`、`mark_index_overflow`、`PARSE_CACHE_MAX_SCRIPT`、
> `exec_traces`、`tier1_epoch`、`body_memo`。

## 结论先行

编译单元走字节码的全部门槛是四个条件的与：**`!code.fallback` &&
`code.epoch == tier1_epoch` && `bytecode_applicable()` && 缓存命中**。
`code.fallback` 由编译器置位（`compiler.rs:228`：`index_overflow()` 或
op 白名单外）；`bytecode_applicable` = 无 exec trace && 无手动关断
（`vm_exec.rs:98`）。四个缓存入口：proc 体（call.rs Site A/B）、
parse-cache 单元（eval.rs:252）、循环体 memo（eval.rs:361）、
`EvalScript` 嵌套单元。

**白名单条件实际恒真**：编译器当前只发白名单内的 op；枚举里其余 14
个变体（`AppendVar`/`CallProc`/`CatchStart`/`Global`/`Line`/
`LoadArrayElem`/`MakeList`/`PushFrame`/`ReturnCode`/`StoreArrayElem`/
`TailCallProc`/`UnsetVar`/`UpVar`/`VarExists`）在 compiler.rs 零发射，
只服务 `::tcl::unsupported::assemble` 手组码（assemble 产物本就标
fallback）。**故真实回落面 = F2/F3/F4/F5/F6；F1 已收（D2 宽操作
数，2026-11-26）；F3/F5/F6 已收（D3 第一批，task/d3-fallback1
2026-11-28——见各节新状态）。驻留面收敛到 F2（exec trace 钉回）
+ F4（诊断模式）。**

## F1. u16 操作数溢出（= C55）——已收（D2 第二步，2026-11-26）

- **触发**：任一操作数超 u16——(a) 常量池超 65535 条
  （`bytecode.rs:294` add_const）；(b) 局部表超 65535 名
  （`:315` add_local）；(c) 单命令超 65535 词
  （`compiler.rs:1176` note_argc_overflow）；(d) 跳转/行号等手记数
  （同 `mark_index_overflow` 通路）。置 `fallback=true`，整单元落树。
- **复现**：`bash judge/audit-cases/big-unit.sh`（约 5900 个 for；
  小单元 errorInfo 同 tclsh、大单元多一层 `invoked from within "set x …"`）。
- **VM 补齐**：码表第二步（HANDOFF §3.2）——跳转改宽形或相对 i32；
  `Call`/`DynCall` 的 argc 宽形；常量索引已有 `PushConstWide` 但
  `get_const` 仍 u16 通路。补齐后 F1 消失（大单元照常编译）。
- **tclsh 对照**：tclsh 9.1.0 同场景照常编译执行（经典字节码引擎，
  操作数 4 字节起步）。

## F2. 执行 trace 钉回

- **触发**：`trace add execution` 存在任何挂着的 exec trace
  （`exec_traces` 非空）→ `bytecode_applicable=false` → 该解释器的
  proc 体/缓存单元全部树引擎（call.rs:100/860/1224/1255、eval.rs:993）。
- **复现**：挂 exec trace 后调用 proc；sweep 既有 12 处 DIFF 中的
  ei_line_attribution、sub_inline_*、loop_body_err_line* 族与此通路
  相关（trace 观测下两引擎的行号/帧语义差）。
- **VM 补齐**：VM 的命令站点上发 per-command trace 事件——
  `BeginCmd`/`Call` 站点已有行号与命令文本记账，缺的是 exec trace
  协议本身（before/after 事件、result/opcode 修改、trace 删除时的
  语义），补齐后 trace 下不再钉回。

## F3. Tier1 epoch 失配

- **触发**：编译后 Tier1 内建被 rename/遮蔽/重定义
  （`code.epoch != self.tier1_epoch`）→ 该单元在重新编译前落树。
- **复现**：`rename dict x; x create a b` 类（judge 语料有 rename
  遮蔽用例，两 oracle 88/88 内）。
- **VM 补齐**：失配时**重编译**该单元（失效缓存 + 再编译）而不是
  回落——成本在编译期一次，不在执行期。树引擎收掉后这一步是必做
  （否则 rename 后语义面换引擎）。

## F4. 手动关字节码

- **触发**：`--no-bytecode` 旗标（cli main.rs:92→105）、
  `RTCL_NO_BYTECODE` env、宿主 programmatic forced-off
  （vm_exec.rs `BYTECODE_FORCED_OFF`）。
- **复现**：`rtcl --no-bytecode -f …`（sweep 用它跑双引擎差分）。
- **VM 补齐**：§3.3 的方向是"`--no-bytecode` 变成字节码 VM 的一种
  运行方式（不缓存、不融合超指令）"——补齐 = VM 加一个
  no-cache/no-fusion 模式位，而不是独立执行器；诊断用途保留。

## F5. 大脚本不走 parse-cache（>256KB→现 1MiB 可配）——已收（D3 第一批，2026-11-28）

- **触发**：`script.len() > PARSE_CACHE_MAX_SCRIPT`（262144，
  eval.rs:151）→ 顶层 eval 的字节码快路径（缓存查取+存入）整段跳过，
  走 `eval_commands` 树路径。
- **复现**：>256KB 的脚本文件 `rtcl -f`（行为 = C55 同款 errorInfo
  差，若其中命令出错）。
- **VM 补齐**：大单元直接编译入口（不经 parse-cache 键，按请求编译
  一次执行），或把缓存键改成内容指纹。与 F1 叠加时上限更低。

## F6. 循环体 memo 未命中——已收（容量 4→32，2026-11-28）

- **触发**：`body_memo` 指针驻留是 FIFO 有界（eval.rs:352-361）——
  多层嵌套循环 + 大体把更早的体挤出后，挤出的体落树。
- **复现**：三层以上嵌套循环、每层大体（无固定最小复现，属容量
  形状问题）。
- **VM 补齐**：memo 容量提升，或体单元持久化进单元级编译缓存。

## 澄清（不是回落，列出防止再误判）

- **vm_exec 内的 "fallback ladder"**（名路径慢路径、slot 失效降级）：
  op 内的回退，仍在字节码引擎里。
- **jitsh 的接受集**（rtcl-jit）：A 族之外的 op 回落**字节码 VM**，
  不是树引擎；jitsh 单元里 trace add 之类直接拒绝编译（proc_exec.rs:156）。
- **Site A 的 extra 条件**（`current_statics` 空、`params_aligned`、
  locals 非空）：只决定 slot-快路径是否可用，不满足时走 Site B 的
  名路径字节码，不落树。

## 对 §3.3 决策的含义

树引擎的真实驻留面 = F2（trace）+ F3（epoch）+ F4（诊断开关）+
F5/F6（缓存容量形状）。F1 由码表第二步顺带消灭。"只留一个引擎"
的补齐清单按成本排序：F3 重编译（小）→ F5/F6 缓存策略（小）→
F2 exec trace 协议（中，最大头）→ F4 模式位（小）。清单齐了，
去留可以拍板。
