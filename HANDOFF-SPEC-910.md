# HANDOFF-SPEC-910 — allium 规格库对齐 tcl9.1（tend + distill 任务交接）

> **状态横幅（2026-11-27）**：独立任务包，与 `HANDOFF.md` 的 W-A/W-B
> 代码迁移流**并行不交叉**（见 §4 边界）。范围：**只动
> `specs/*.allium` 与本文件**，把 14 份行为规格从 oracle=tclsh 8.6.17
> 迁移/对齐到 oracle=tclsh 9.1.0——修 6 处实伤、清 2 处规格落后于实现
> 的漂移、按 T1-T7 补 9.1 新面规则、统一换锚。**不改 rtcl 任何代码。**
> 对照工作已完成（§3 是完整底账），接手者**不必重新做对照**，直接从
> §5 执行步骤开始。

## 1. 任务定义

把 `specs/` 下 14 份 allium 规格当作 rtcl 行为的域级规范库，对其做
9.1 迁移三件事：

1. **tend**：改写钉住 8.6 语义（已被 9.1 废除/改变）的规则（§3-A）；
2. **weed**：清理规格落后于实现/迁移进度的漂移（§3-B）；
3. **distill**：从 tcl9.1 来源 + 探针实证补 9.1 新增面的规则（§3-C）。

最终态：14 份规格全部锚 `Oracle: tclsh 9.1.0`，`allium check`/`analyse`
全绿，每条改动/新增规则带证据引用（探针文件名 / changes.md 条目 /
doc/*.n 章节）。

## 2. 必读上下文

- **规格库现状**：`specs/*.allium`，14 份、447 条规则、~2450 行，全部
  `-- allium: 3`；allium CLI 3.6.1（mise shim），2026-11-27 基线
  check/analyse 全绿（仅 interp.allium 几条 info 级 unused-field）。
  规格 idiom：一条规则一个可观测断言，`when: Evaluates("探针脚本")` +
  `ensures` 值 + probe 注释；rtcl↔oracle 已知分歧按 DIVERGENCES.md 的
  D-x 编号内嵌注释。
- **规格锁定的 oracle**：14 份头部全部 `Oracle: tclsh 8.6.17`
  （2026-11-03 深水区全量建模产物）——而项目 oracle 已于 2026-11-25/26
  切到 9.1.0（`.refer/tcl910/inst/bin/tclsh9.1`，`JUDGE_ORACLE=tclsh`
  回指 8.6.17）。规格整体落后 oracle 一代，这就是本任务的由来。
- **来源权威**（证据等级从高到低，见 §4 证据原则）：
  1. **oracle 实测**：`.refer/tcl910/inst/bin/tclsh9.1` 直接跑探针脚本
     （项目哲学：judge 期望运行期求值，实测钉死语义）；
  2. **手册页**：`.refer/tcl910/tcl9.1.0/doc/*.n`（行为规范文本）；
  3. **变更清单**：`.refer/tcl910/tcl90-changes.md`（9.0 vs 8.6）、
     `.refer/tcl910/tcl9.1.0/changes.md`（9.1 vs 9.0）。
- **实证底账**：`TODO.md`「对标迁移」T1-T7 工作组（judge 9.1 基线
  4152/4849=85.6%，697 失败 case 已归组；探针档案 317/437，122 项 DIFF，
  每项带 `judge/probes/` 文件名）；`specs/DIVERGENCES.md`（D-x 台账）。

## 3. 对照底账（2026-11-27，已完成，接手者直接消费）

### A 类：规格钉住已被 9.1 废除/改变的语义（实伤，6 处）

| # | 位置 | 所钉的 8.6 行为 | 9.1 行为 | 证据 |
|---|---|---|---|---|
| A1 | `expr.allium:78` `ExprLegacyOctalLiteral` | **ensures 值** `expr {010}` = "8" | 0 前缀按十进制 = "10"；八进制必须 `0o` | tcl90-changes「0NNN 不再八进制」；TODO T1 bn2/bn3/bn4 |
| A2 | `expr.allium:133` `ExprBitwiseRequiresIntegers` 注释 | 文案 `can't use floating-point value as operand of "\|"` | `cannot use … value "…" as left/right operand of …`（含值与左右指位）。ensures（error）不变，只改注释口径 | TODO T1 bn6-bn14、numcmp1 |
| A3 | `variables.allium:246` `WhichVariableFollowsChain` | ns 体 `namespace which -variable g` → `::g`（回退全局） | 当前 ns 优先不回退 → ""（空） | TODO T5 [x] migrate/ns91（2026-11-26 已修实现）|
| A4 | `variables.allium:256-282` `NsBodyChainIncrGlobalSlot`（"3 3"）`ProcGlobalWriteThenNsBodyIncr`（"3"）`NsBodyArrayElementChain`（"1 2"） | ns 体未限定 set/incr **回退全局** | 当前 ns 优先；探针结果整体变化（`NsBodyArrayElementChain` 的 when 里 `$a(y)` 在 9.1 语义下可能直接报 no such variable——**用 oracle 实测重新钉**，含改写 when 的可能） | 同 A3 |
| A5 | `trace.allium:144-155` legacy 形两条 | `trace variable a(k) w cb` 可用、vinfo 形可用 | 9.0 移除 `trace variable\|vdelete\|vinfo`；应改钉为报错（以 oracle 实测文案为准） | tcl90-changes Incompatibilities；TODO T4 移除项 |
| A6 | `error-deep.allium:32` `ErrorstackEntries` | ensures/注释含 `invokeStk1` 窄形词 | 9.1 弃窄形（`invokeStk1→invokeStk` 等全词族）；**实现 2026-11-26 `op_word_91` 已归一**，规格未跟。ensures 主值 `INNER {returnImm x {}} CALL p1` 中 `returnImm` 形是否也变——**对 9.1 oracle 复跑钉定** | TODO T7 errorstack 条目 [x]（2026-11-26）|

### B 类：规格落后于实现（weed 方向）

| # | 位置 | 问题 |
|---|---|---|
| B1 | `io.allium:103-109` `EncodingCommandExists` | 注释记录 D-30「encoding 命令整体缺失」——实现已注册 encoding（`registry.rs`/`encoding.rs`，`.enc` 表后端全线）。注释须更新；规则面补 9.x encoding 子命令面（`convertto/convertfrom` 的 `-profile`/`-failindex`，以 doc/encoding.n + 实测钉）|
| B2 | 同 A6 | errorstack 词形：实现先行（11-26），规格未跟 |

### C 类：9.1 新增面规格未覆盖（distill 补规则清单，按 T 组）

每组条目后括号内是 `judge/probes/` 探针名（TODO 对标迁移原文）。

- **T1 数字与表达式**（最大簇，judge 失败 266 case）：`0o`/`0d`/下划线
  字面量与 `string is integer` 面（bn2-bn4、expr_lead1/2、s_probe、
  cmp_string_index、format_grammarF）；`incr`/`int()` 任意精度
  （bnprobe2、incr_widen）；format/scan 32 位截断 + size 修饰符
  j/q/z/t（p44/p45/p48/p51、format_grammarE、scan_*）；`%#o`/`%#x`/
  `%c` 上限文案/`TCL FORMAT INCOMPLETE`（format_grammarC/D/8/G/J）；
  expr 错误文案族（bn6-bn14、numcmp1、verify、expr_bracket_top、
  p32/p33）；`lt/gt/le/ge` + expr 内注释（p11/p13）；C99 mathfunc
  全集 + `fpclassify`/`divmod`/`frexp`/`modf`/`remquo`（info4）；
  `::tcl::mathop` 补 lt/le/gt/ge（p40）。
- **T2 索引与 64 位容量**：`string index/range`/`lindex` 等 64 位索引
  放宽 + bad index 文案（ix1-ix5）。
- **T3 encoding/Unicode/通道**（失败 2 case + ~20 探针）：
  `-encoding binary` 移除文案（chan_copy_modes/chan_eofchar）；
  fconfigure `-profile`（默认 strict）/`-stat`、写通道 `-eofchar`
  移除、`must be non-NUL ASCII character` 文案（chan_io_p7/8/9/16/22）；
  `chan isbinary`/`pending` 文案（chan_io_p3/p4）；`chan names` 排序与
  fileN 计数（chan_names_order、chan_io_p15/17/18/20、chan_seek_tell）；
  errorCode 统一 `TCL VALUE NUMBER`（chan_io_p11/23、incr_window_p02/
  07/27、smallface d2、v_err）；Unicode 18.0 字符类（probe5/6）；
  `string is dict` + ambiguous class 文案（probe5/9）；`source` 默认
  UTF-8；通道读坏字节严格解码报错。
- **T4 命令清单**（失败 374 case 主战场）：列表族 `lpop`/`lremove`/
  `ledit`/`lseq`（9.1 TIP 746 已去 expr 行为）/`lfilter`；`dict getdef`/
  `getwithdefault`（dict_p42、smallface_p24）；`array default`（**会改
  变 append/incr/lappend 行为，单独钉语义**）/`array for`；`file home`/
  `tempdir`/`tildeexpand` + `~` 不再自动展开；`const` 族 + `info
  constant/consts/commandtype/line`；`coroinject`/`coroprobe`；`timer`/
  `timerate`；`unicode`；`zipfs`；`switch -integer`（p25）；新选项线
  （`regsub -command`、`lsearch -stride`、`clock add weekdays`、
  `clock scan -validate`、`socket -nodelay/-keepalive`、vwait 新选项、
  `chan configure -inputmode`、`encoding user`、`exec -encoding`、
  `subst -backslashes/-commands/-variables`（9.1）、`info loaded
  ?prefix?`）；utility 族（`readFile`/`writeFile`/`foreachLine`、
  `tcl::process`、`tcl::idna`、`tcl::build-info`、`package files`）；
  **移除项**：`case` 命令（rtcl 现仍暴露）、`puts`/`read` 旧变体、
  `trace variable` 族（同 A5）；`::tcl::` 命令集 27 vs 23（tclns）+
  `::tcl::pkgconfig` 键集（pkgconfig_probe）。
- **T5 namespace/变量模型**（失败 11 case）：全局初始变量集与
  `info vars` 可见性（q114b/u72/p-uplevel-ns/trv_p8）；
  errorCode/errorInfo 不预置（ec2）；变量 unset 重建/链接帧语义
  （vm3/incr_frame）；`upvar` bad level 口径（upvar_levels）；expr
  数组下标 `$foo("x)` 报 `invalid character in array index`
  （pidx/pidx3）。
- **T6 TclOO**（失败 19 case）：private variables/methods、class
  variables/methods、abstract/singleton、properties（oo21 的
  `-prepend/-remove/-appendifnew/-set/-append/-clear`、oo8/oo_dbg1）；
  `oo::class`/`oo::object` 子命令面 properties/creationid/isa/vars
  （oo5）；对象编号计数（oo1/oo3/oo4/oo7/oo8/oo9）→ **裁定项，见 §6**；
  OO 错误文案。
- **T7 内部口径**：`tcl::unsupported::assemble` 9.1 指令面（TIP 720：
  新增 swap/isEmpty/strge/strgt/strle/strlt/jumpTableNum/dictGetDef/
  dictPut/dictRemove/tclooId，移除 land/lor/lshift；cmp_assemble、
  unsupported_assemble）；`binary format` 浮点 Inf/NaN 位保真（pbr5）→
  **裁定项候选，见 §6**；regexp 编译错误文案 `couldn't→cannot`
  （p6/v_re/utf_char_classes）。
- **spec 归属缺口**：`switch` 整命令无规格归属（control-flow 只钉完成
  码，matching 钉 regexp/glob/format/scan/binary/clock）——`switch -integer`
  等需要落 matching.allium 或新增面。

### 规格 ↔ T 组映射（工作量分域）

| 规格（规则数） | 涉及 | 动作 |
|---|---|---|
| expr (21) | T1 | A1/A2 tend + C 类大量补 |
| variables (32) | T5 | A3/A4 tend（最急：已与实现漂移）+ T5 补 |
| error-deep (13) | T5/T7 | A6 tend + 六键序对 9.1 复核 |
| trace (22) | T4 | A5 tend |
| io (19) | T3 | B1 tend + T3 补 |
| string (11) | T1/T2/T3 | 现规则稳，补新面 |
| matching (17) | T1/T7 + switch 归属 | 补 |
| control-flow (13) | — | 完成码协议 9.1 未变，基本不动 |
| dict (19) / list (18) | T4 | 现规则稳，补新子命令面 |
| oo (26) | T6 | 大量补 + 1 裁定项 |
| regex-spencer (23) | T7 | 补 1 条文案 |
| memory-lifecycle (8) / interp (0 规则) | T4 | interp 补 TIP 728 子解释器变量读写面 |

## 4. 边界、纪律与证据原则

- **与在途工作流的边界**：W-A/W-B（`HANDOFF.md` 横幅）动的是 crates
  代码与 judge/probes——本任务**只读引用**它们的产出（探针转绿记录、
  migrate/ns91 提交）。`git diff --stat` 最终只允许含 `specs/*.allium`
  与本文件。**2026-11-27 用户授权新增探针**：本轮新增
  `judge/probes/oo_objcount91.tcl`、`oo_features91.tcl`、
  `pbr6_float_bits.tcl`（只加不改，用于 T6/T7 实证）。
- **不改**：crates/、judge/、TODO.md、HANDOFF.md、DIVERGENCES.md、
  Agent.md（全部只读引用）。tend 中若发现**新的实现缺陷**（规格说 A
  实现做 B），记录到 §7 新发现台账，**不修码**。
- **证据原则**（写入每条规则的注释）：oracle 实测（tclsh9.1 直接跑）
  > doc/*.n 手册页 > changes.md 概述。探针脚本与 9.1 oracle 实测冲突
  时**以实测为准**并在注释标日期。规格锚的是 **oracle 行为**，不是
  rtcl 行为；rtcl 与规格的差异按 DIVERGENCES.md 惯例内嵌
  `-- rtcl: …` 注释（引用 D-x 编号），不改 DIVERGENCES.md 本身。
- **移除项的钉法**：9.1 已移除的命令/形式（`case`、`trace variable`
  等），规格钉 **9.1 语义（报错/不存在）**；rtcl 的保留行为属迁移
  清障账（TODO 对标迁移），在规格内以 `-- rtcl: 仍暴露（迁移面）`
  一类注释记录即可，**不要**为 rtcl 扩展行为立 ensures。
- **多 agent 纪律**（Agent.md）：独立 worktree（`git worktree add`），
  每完成一份规格即 `git add` 挑自己的文件提交；提交信息
  `type(scope): 中文摘要`（建议 `type(specs):` 或 `type(spec):`，
  与仓内惯例一致即可），正文写改法与没动的地方，终态一行写
  allium check/analyse 结果。
- **接手第一步**：本文件当前是工作树未提交状态——先提交本文件入库
  （防共享检出被其他 agent 的 checkout/reset 抹除），再开 worktree。
  **注意**：`.gitignore` 全局忽略 `*.md`，必须 `git add -f
  HANDOFF-SPEC-910.md`（仓内 HANDOFF.md/TODO.md 等台账均为 -f 强制
  入库的同一惯例）；普通 `git add` 会静默跳过。
- **门禁口径**：规格-only 改动不需要跑全门禁；但 `judge/run.sh`（双
  oracle）不得因本任务变红（规格-only 理论上不可能，变红即说明误改了
  代码，立即回滚核对）。需要验证探针时直接跑
  `.refer/tcl910/inst/bin/tclsh9.1`（不构建 rtcl）。
- **OOM 纪律**：本任务原则上零构建；若必须构建（不该发生），按
  Agent.md `ulimit -v 3400000` + 核对二进制时间戳。

## 5. 执行步骤（建议顺序）

1. **环境自检**：`allium --version`（3.6.1）；`git log -1` 确认检出
   新鲜；基线 `allium check specs/ && allium analyse specs/` 全绿。
2. **提交本文件**（§4）。
3. **tend A 类 6 处**（每处独立提交）：A3/A4 最急（已与实现漂移）；
   A1/A2/A5 一批；A6 先对 9.1 oracle 复跑 errorstack 形再钉。全部
   用 `.refer/tcl910/inst/bin/tclsh9.1` 实测后改写，注释带探针证据。
4. **tend B1**（io.allium encoding 注释 + 9.x 子命令面）。
5. **distill C 类**，优先级：T1（最大簇）→ T5 → T3 → T4 新命令面 →
   T6 → T2/T7。每份规格完成后 `allium check specs/` + `allium analyse
   specs/` 全绿再提交。新规则的 when 一律取可复现的探针脚本（可直接
   跑 oracle 验证），ensures 值一律实测字节。
6. **换锚**：每份规格头部改 `-- Oracle: tclsh 9.1.0 (core-9-1-0)`，
   保留一行 `-- Migrated from 8.6.17 spec (2026-11-03 round), see
   HANDOFF-SPEC-910.md` 类历史说明。
7. 分期交付允许：若一次做不完全部 C 类，按组登记进度到 §8，
   但 A 类 6 处 + B1 必须在本期完成。

## 6. 裁定项（2026-11-27 用户口径：2 按 9.1 对齐，3 允许加探针）

1. **oo 对象计数口径**：已按用户 2026-11-27 裁定——**首个用户对象固定
   为 `::oo::Obj20` / `creationid 20`**。9.1.0 核心预建 20 个对象；
   rtcl 预建少但用户可见起点保持 20（计数对齐）。规格钉绝对首对象 +
   名字形 + 后续递增/不复用关系，不再作为 open question。
2. **binary 浮点 Inf/NaN 位保真**：已按 tclsh 9.1.0 实测字节钉死
   （`pbr6_float_bits`）：float32 Inf `7f800000`、-Inf `ff800000`、
   NaN `7fc00000`、-0.0 `80000000`；double 对应 `7ff0…/fff0…/
   7ff8…/8000…`。rtcl 的 `7f7fffff` 饱和是代码面待修，不在本任务改码。
3. **`case` 命令**：规格已钉 9.1 语义（invalid command name），rtcl
   仍暴露以 `-- rtcl: 迁移面` 注释记录，不再单列裁定。

## 7. 新发现台账（接手者追加）

> 格式：日期 | 位置（file:rule） | 发现 | 证据（oracle 实测输出/探针）
> | 处置建议。发现实现缺陷只登记不修码；发现 9.1 文档与实测冲突时登记
> 并按实测钉。

（暂无）

### 2026-11-27（spec-910 接手轮）

1. `variables.allium` `VariableLinkIsPerEvalBodyFrame` — A4 未列出的同类实伤：
   原 ensures `"9 2"` 依赖 8.6 的 ns→global 回退；9.1 内层体写
   `::o::i::s`，实测 `list $::o::s $s [set ::o::i::s]` = `"9 1 2"`。
   已按实测改写（与 A4 同批提交）。
2. `trace.allium` `TraceInfoRoundTripsAndOrdersOps` — 注释钉的 op 序
   `read, write, unset, array` 对 8.6.17 也不成立；两 oracle 实测均为
   `array read write unset`。且原 ensures 漏了外层列表括号（完整返回是
   `{{write unset} cb}`）。已把 when 改为 `lindex [...] 0` 并修正注释。
3. `io.allium` `EncodingCommandExists` — 原文引 `[D-30]`，但 DIVERGENCES
   的 encoding 条目是 D-29（D-30 是 read 计数）。已改引 D-29（FIXED）。
4. B1 实现面差异（只登记，不修码）：rtcl `encoding` 已存在，但 NAMES 仍是
   8.6 的 86 名（保 identity，无 utf-16/utf-32/ucs-2/cesu-8；9.1 为 95），
   且 `profiles`/`user` 子命令与 `-profile`/`-failindex` 解析缺失
   （`io.rs` 的 convertfrom/to 要求恰好 4 个参数）。规格按 oracle 钉，
   差异以 `-- rtcl:` 注释内嵌，属 T3 迁移面。
5. 9.1 oracle 与 doc 的冲突实例：`doc/chan.n` 的 errorstack 示例仍写
   `invokeStk1`，实测 9.1.0 已统一为 `invokeStk`（按 §4 证据原则取实测）。
6. `matching.allium` `FormatCIsBmpOnly` — 9.1 把 `%c` 上限从 16 位
   `Tcl_UniChar` 提到 Unicode 标量 0..0x10FFFF：65536/128512 正常输出
   星光面字符，1114112 与负数才落 U+FFFD。原 8.6 规则已改为
   `FormatCRendersUnicodeScalar`（T1 批）。
7. `expr` 的 `017` 是十进制 17，而 `scan %i`/`%o` 的 `017` 仍是八进制
   15（C 语义）。两处都已按 oracle 分别钉死，勿当同一套字面量语法。
8. 规则数口径：§2 写的「447 条规则」与实测不符——交接基线 `442f4ff`
   上 `grep '^rule ' specs/*.allium` 为 242 条；本轮结束为 349 条
   （新增 107 条）。§2 的 447 可能是含 probe/其他分支的旧数，后续以
   grep 计数为准。
9. T7 指令面修正：9.1 的 `assemble` 菜单仍保留 `lshift`，移除的是
   `land`/`lor`；§3-C 写的「移除 land/lor/lshift」应改为「移除
   land/lor」。新增 `swap`/`isEmpty`/`strge|strgt|strle|strlt`/
   `jumpTableNum`/`dictGetDef`/`dictPut`/`dictRemove`/`tclooId` 已由
   cmp_assemble 实测确认。
10. T6 文档超前于 9.1.0 实现：`doc/classvariable.n` 存在，但
    `oo::define` 里的 `classvariable` 实测是 invalid command；核心
    `oo::class` 的 `property` 也未注册（只有 `oo::configurable` 提供），
    且 `info class properties` 实测只认 `-get/-kind/-set`，不认
    `doc/info.n` 写的 `-readable/-writable/-all`。故 properties/
    classvariable 不写 ensures，等 9.2 或实现落地后再补；本轮只钉
    abstract/singleton/private/classmethod/info class variables。
11. T4 清单里的 `readFile`/`writeFile`/`foreachLine`/`tcl::idna` 在
    9.1.0 实测不存在；`info commandtype/line` 实际名为 `info cmdtype`，
    没有 `info line`。`unicode` 只有 tonfc/tonfd/tonfkc/tonfkd 四个
    归一化子命令，没有 version/names。后续不要再按旧清单硬造规则。
12. OO 对象基准：9.1.0 首个用户对象（new/create/class）是
    `::oo::Obj20` / `creationid 20`，命名对象与类共用同一计数器。
    用户确认 rtcl 虽预建少但用户可见起点保持 20；规格已钉绝对首对象
    （`AnonymousObjectCounterStartsAt20`、`NamedAndClassObjectsShareCounter`）
    与递增/不复用关系，探针 `oo_objcount91.tcl` 断言 Obj20。
13. binary 浮点位形：9.1.0 为规范 IEEE-754（float32 Inf `7f800000`，
    double Inf `7ff0000000000000`），rtcl 的 `7f7fffff` 饱和是实现
    分歧；规格已钉 oracle（`pbr6_float_bits.tcl`），代码待修。
14. `oo::class new` 在 9.1.0 是 invalid method（类对象用
    `oo::class create`）；`oo::object new` 才是匿名对象入口。

## 8. 交接记录

### 2026-11-27（交出）

- 交出方：对照与底账（§3 全部内容）由本轮会话完成；产物即本文件。
- 接收方：spec-910 会话（见下一节）。
- 完成范围：仅交出底账，未改任何规格。
- 门禁终态：基线 `allium check/analyse specs/` 与 2026-11-27 一致。

### 2026-11-27（接收/本期执行）

- 接收方：spec-910 会话（worktree `/home/nzinfo/src.note/rtcl-spec910`，
  branch `task/spec-910`）。
- 完成范围（必达项）：
  - A1 `ExprLeadingZeroIsDecimal`（原 `ExprLegacyOctalLiteral`）：
    `expr {010}` ensures `"8"` → `"10"`，补 0o/0d/0b/下划线。
  - A2 位运算错误文案改 9.1 口径（含值与左右操作数指位）。
  - A3 `WhichVariableDoesNotFallBackToGlobal`（原
    `WhichVariableFollowsChain`）：`"::g"` → `""`。
  - A4 `NsBodyIncrCreatesNamespaceVariable` /
    `NsBodyIncrDoesNotTouchGlobal` / `NsBodyArrayElementStaysInNamespace`
    三条按 9.1 重钉；另补 `VariableLinkIsPerEvalBodyFrame`（§7-1）。
  - A5 legacy `trace variable|vdelete|vinfo` 两条改为 bad-option 报错；
    `TraceInfoRoundTripsAndOrdersOps` 的 op 序一并修正（§7-2）。
  - A6/B2 `ErrorstackEntries`：`invokeStk1`→`invokeStk`，补 `UP <n>`
    token 说明；`returnImm` 经 9.1 复跑确认不变。
  - B1 `EncodingCommandExists` 注释更新（D-29 FIXED、86 vs 95 名）+
    5 条 9.1 编码档规则（profiles / strict / -failindex / tcl8 / user）。
- 换锚：expr / variables / trace / error-deep / io 五份头部改
  `Oracle: tclsh 9.1.0 (core-9-1-0)`，并注明 C 类未覆盖范围；其余 9 份
  仍为 8.6.17，待各自 C 类完成后再换。
- C 类：T1/T5/T3/T4/T2 已完成。
  - T1：expr/string/matching 补 29 条（数字字面量、任意精度、
    lt/le/gt/ge、expr 注释、C99 mathfunc、新数值命令、string is 字面量、
    format/scan 32 位与 size 修饰符、format 语法错误、%c Unicode 标量）。
  - T5：variables 补 10 条（info vars 可见性、ns eval uplevel 帧、
    variable 无值/重建、errorCode 不预置、upvar 各级错误、incr 失败、
    父 ns 缺失、expr 数组下标解析）。
  - T3：io/string 补 13 条（binary 编码移除、-profile/-stat、输出
    -eofchar 移除、chan isbinary/names、TCL VALUE NUMBER、strict UTF-8、
    source、string is dict、Unicode 18 数字类、ambiguous class）。
  - T4：list/dict/variables/control-flow/matching/io 补 21 条
    （lpop/lremove/ledit/lseq/lfilter、lsearch -stride、dict getdef/
    getwithdefault、array default/for、const/info consts、switch -integer、
    case 移除、subst 9.1 选项、regsub -command、file home/tildeexpand/
    tempdir）。
  - T2：string/list 补 4 条（索引表达式 1±n、end±n、进制字面量、
    64 位不截断、bad index 文案）。
  - T7：matching 补 regexp 编译错误文案（couldn't -> cannot；
    quantifier operand invalid -> invalid quantifier operand），并更新
    三条既有规则的注释；interp 补 assemble 指令面（swap 可用、land
    移除、lshift 仍在）与栈平衡错误；matching 补 binary 浮点
    Inf/NaN/-0.0 的 IEEE-754 位形（R/r/Q/q）。binary 不再挂裁定项。
  - T4 余项未做：timerate 细节、unicode（仅归一化）、zipfs、socket
    新选项、info loaded、package files、::tcl 命令集 27 vs 23、
    pkgconfig 键集（实测 19 键）。清单中的 readFile/writeFile/
    foreachLine/tcl::idna 在 9.1.0 不存在（§7-11）。
  - T6：已钉 abstract/singleton/private method/classmethod/
    info class variables（5 条）+ 匿名对象首基准 Obj20、名字/creationid
    关系、命名/类共用计数器、info object isa（4 条）。
    properties/classvariable 因文档超前于 9.1.0 实现未写（§7-10）；
    对象计数绝对基准已按用户裁定钉死，不再留 open question。新增探针
    `oo_objcount91.tcl`、`oo_features91.tcl`、`pbr6_float_bits.tcl`
    （§4 授权）。
- 门禁终态：`allium check specs/` 0 error（4 warning 为 memory-lifecycle
  既有 unused entity/binding；5 info 为既有 unused field）；
  `allium analyse specs/` 0 finding。未改 crates/judge，未跑 judge
  （规格-only 改动不触发代码门禁）。
- git diff --stat 范围：`specs/*.allium`（已换锚的 12 份）+ 本文件 +
  `judge/probes/{oo_objcount91,oo_features91,pbr6_float_bits}.tcl`
  （§4 授权新增探针，只加不改）。
