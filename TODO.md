# TODO — 待优化/待完成任务清单

> **同步约定**：本清单与 `bench/BASELINE.md` 的台账保持同步——每完成
> 一项，在此勾销并在 BASELINE 记录实证数字；每新增一项，先在 BASELINE
> 补分析与方案再入列。条目按预期收益排序。

## 对标迁移（Tcl 9.1，2026-11-25 起）

> 实证底账（2026-11-25）：oracle 切到 9.1.0 后，judge **88/88** 全绿、
> 探针 **312/437**（125 项 DIFF = 迁移面）、命令清单差 **17 缺 1 多**
> （`case` 多）。以下按工作组拆解；每项标注探针证据（`judge/probes/`
> 文件名）。权威变更清单：9.0 vs 8.6 见 `.refer/tcl910/tcl90-changes.md`
> （内嵌 9.0.0 段），9.1 vs 9.0 见 `.refer/tcl910/tcl9.1.0/changes.md`。
> 每完成一项：对应探针转绿 + `judge/run.sh`（9.1 oracle）保持 88/88。

- [x] **oracle 切换 8.6.17 → 9.1.0（2026-11-25 完成）**：对标版本改为
  Tcl 9.1（官方 release
  [core-9-1-0](https://github.com/tcltk/tcl/releases/tag/core-9-1-0)）。
  `judge/run.sh`/`judge/probes.sh`/`judge/extract/extract.py` 的 oracle
  默认仓内构建 `.refer/tcl910/inst/bin/tclsh9.1`；`JUDGE_ORACLE=tclsh`
  可指回 8.6.17。

### T1 数字与表达式（~25 个探针，最大簇）

- [ ] **数字字面量 9 语法**：`0oNNN` 八进制、`0dNNN` 强制十进制、
  `0NNN` **不再八进制**（按十进制）、`N_NNN` 下划线分隔。覆盖 expr
  判定与 `string is integer`（bn2/bn3/bn4、expr_lead1/expr_lead2、
  s_probe、cmp_string_index、format_grammarF）。
- [ ] **整数任意精度默认**：`incr`/`int()` 溢出不回绕不截断
  （bnprobe2、incr_widen：up→`9223372036854775808`）。
- [ ] **`format` 32 位截断规则**：`%b/%d/%o/%x/%u` 无 size 修饰符一律
  截 32 位（9.0 明文；p44/p45/p48、format_grammarE），`%L` 不再截
  64 位；size 修饰符 `j/q/z/t` 补齐（9.0.1 completed）。
- [ ] **`scan` 同套 32 位截断**（p51、scan_consumption/scan_numeric/
  scan_numeric2/scan_overflow/scan_validation）。
- [ ] **`format` 杂项**：`%#o` → `0o` 前缀、`%#x` 零值 C99 行为
  （format_grammarC）；`%c` 码点上限错误文案（format_grammarD）；语法
  错误口径 `format string ended in middle of field specifier` + EC
  `TCL FORMAT INCOMPLETE`（format_grammar8/G/J）。
- [ ] **expr 错误消息 9 口径**：`cannot use <fp/non-numeric> value
  "<值>" as left/right operand of "<op>"`（bn6/bn7/bn9/bn10/bn12/
  bn13/bn14、numcmp1、verify、expr_bracket_top）；boolean 文案
  `expected boolean value but got a list`（p32/p33）。
- [ ] **expr 新比较符 `lt/gt/le/ge`**（字符串比较）+ expr 内注释
  （p11/p13；9.0 新特性清单）。
- [ ] **mathfunc C99 全集**：`acosh/asinh/atanh/cbrt/copysign/dim/erf/
  erfc/exp2/expm1/fma/gamma/ldexp/lgamma/log1p/log2/logb/nextafter/
  remainder/signbit/trunc` + `isinf/isnan/isnormal/issubnormal/
  isunordered`（info4）；新命令 `fpclassify/divmod/frexp/modf/remquo`。
- [ ] **`::tcl::mathop` 补 `lt/le/gt/ge`**（p40/dict_p40）。

### T2 索引与 64 位容量（TIP 626）

- [ ] **索引解析放宽**：`string index/range`/`lindex` 等接受 64 位与
  超大整数索引（越界返回空而非 `bad index` 报错）+ bad index 文案
  （ix1/ix2/ix3/ix4/ix5）。

### T3 encoding / Unicode / 通道（~20 个探针）

- [ ] **`-encoding binary` 移除**：报错文案 `unknown encoding "binary":
  No longer supported. please use either "-translation binary" or
  "-encoding iso8859-1"`（chan_copy_modes/chan_eofchar）；确认
  `identity` encoding 同步移除。
- [ ] **fconfigure 选项面**：`-profile`（默认 strict）+ `-stat` 进
  选项清单；写通道 `-eofchar` 移除；`-eofchar` 错误文案 `must be
  non-NUL ASCII character`（chan_io_p7/p8/p9/p16/p22）。
- [ ] **`chan` 子命令面**：`chan isbinary`；`chan pending` wrong-args
  文案（chan_io_p3/p4）。
- [ ] **`chan names` 排序与通道命名计数**：标准通道优先排序；fileN
  计数口径（chan_names_order、chan_io_p15/p17/p18/p20、chan_seek_tell）。
- [ ] **errorCode 统一 `TCL VALUE NUMBER`**（rtcl 现发 `TCL VALUE
  INTEGER`；chan_io_p11/p23、incr_window_p02/p07/p27、smallface d2、
  v_err）。
- [ ] **rtcl-unicode 数据升级 Unicode 18.0**（9.1 bundled；Adlam 等
  新 script 的 digit 类，probe5/probe6）。
- [ ] **`string is dict`** + 字符类清单与 `ambiguous class` 文案
  （probe5/probe9）。
- [ ] **`source` 默认 UTF-8**（9.0 国际化线；rtcl 读脚本编码路径核对）。

### T4 命令清单（17 缺 + 移除项 + 子命令面）

- [ ] **列表族**：`lpop`/`lremove`/`ledit`/`lseq`（注意 9.1 TIP 746
  已去掉 expr 行为）/`lfilter`。
- [ ] **dict**：`dict getdef`/`dict getwithdefault`（dict_p42、
  smallface_p24）。
- [ ] **array**：`array default`（会改变 `append`/`incr`/`lappend`
  行为——单独钉语义）/`array for`。
- [ ] **file**：`file home`/`file tempdir`/`file tildeexpand`；`~`
  不再自动展开（9.0 不兼容，路径各命令核对）。
- [ ] **const 族**：`const` + `info constant`/`info consts`；`info
  commandtype`（cmdtype）；`info line`（c5_l2/sub_inline_semantics
  清单核对）。
- [ ] **协程**：`coroinject`/`coroprobe`（替代 `::tcl::unsupported::inject`）。
- [ ] **时间**：`timer`（单调钟、微秒）/`timerate`。
- [ ] **`unicode`**（normalize：`tonfc/tonfd/tonfkc/tonfkd`）。
- [ ] **`zipfs`**。
- [ ] **`switch -integer`**（p25，9.1 新增）。
- [ ] **新选项线**（探针未覆盖，按 9.1 文档钉）：`regsub -command`、
  `lsearch -stride`、`clock add weekdays`、`clock scan -validate`、
  `socket -nodelay/-keepalive`、`vwait` 新选项、`chan configure
  -inputmode`、`encoding user`、`exec -encoding`、`subst
  -backslashes/-commands/-variables`（9.1）、`info loaded ?prefix?`。
- [ ] **utility 族**：`readFile`/`writeFile`/`foreachLine`、
  `tcl::process`、`tcl::idna`、`tcl::build-info`、`package files`
  （info2/tclns 清单核对）。
- [ ] **移除项**：`case` 命令、`puts`/`read` 旧变体语法、`trace
  variable|vdelete|vinfo`（9.0 明确移除清单；rtcl 现仍暴露 `case`）。
- [ ] **`::tcl::` 命名空间命令集**（tclns：9.1 有 27 个，rtcl 23——
  `build-info`/`process`/`InitAutoPath` 等）+ `::tcl::pkgconfig` 键集
  （`dllfile,runtime` 等，pkgconfig_probe）。

### T5 namespace / 变量模型（9.0 不兼容核心）

- [x] **未限定变量名当前 ns 优先、不回退全局**（2026-11-26 完成，
  migrate/ns91：`ns_chain_global` 链整体移除 + `global` 链的瞬态
  无值条目随帧消亡；四方对照 vm/tree=91；judge 88/88 双 oracle、
  crash-gate 126/126）。
- [ ] **全局初始变量集与 `info vars` 可见性**：9.1 下空 ns eval 的
  `info vars` 不含 tcl_* 系统变量（q114b/u72/p-uplevel-ns/trv_p8）。
- [ ] **`errorCode`/`errorInfo` 初始不预置**（ec2）。
- [ ] **变量 unset 重建/链接帧语义**（vm3/incr_frame）。
- [ ] **`upvar` bad level 口径**（upvar_levels）。
- [ ] **expr 数组下标解析**：`$foo("x)` 报 `invalid character in
  array index`（pidx/pidx3）。

### T6 TclOO（9.0/9.1 OO 扩展）

- [ ] **private variables/methods、class variables/methods、abstract/
  singleton classes、properties**（oo21 的 property 配置选项
  `-prepend/-remove/-appendifnew/-set/-append/-clear`）；`method
  ?option?`（`-export` 等，oo8/oo_dbg1）。
- [ ] **`oo::class`/`oo::object` 子命令清单**：`properties`/
  `creationid`/`isa`/`vars`（oo5）。
- [ ] **对象编号计数口径**（oo1/oo3/oo4/oo7/oo8/oo9：`::oo::Obj20`
  vs rtcl `::oo::Obj11`——9.1 启动预建对象多）——裁定对齐计数或
  探针豁免。
- [ ] **OO 错误文案**：`method name ?option? args body` 等。

### T7 内部口径 / unsupported（低优先级）

- [ ] **`tcl::unsupported::assemble` 指令集对 9.1**（TIP 720：新增
  `swap/isEmpty/strge/strgt/strle/strlt/jumpTableNum/dictGetDef/
  dictPut/dictRemove/tclooId`，移除 `land/lor/lshift`；cmp_assemble/
  unsupported_assemble）。
- [x] **errorstack 词法**（2026-11-26 完成：eval.rs funnel 处
  `op_word_91` 归一化——9.1 弃用窄形指令，`loadScalar1→loadScalar`、
  `invokeStk1→invokeStk` 等全词族；crash-gate 126/126）。
- [ ] **`binary format` 浮点 Inf/NaN 位保真**（pbr5：`7f800000` vs
  rtcl `7f7fffff` 饱和）。
- [ ] **regexp 编译错误文案**：`cannot compile regular expression
  pattern`（9.1 用 cannot；rtcl 用 couldn't；p6/v_re/utf_char_classes）。
- [ ] **`tcl_library` 等系统变量值语义**（cmp_globals：9.1 为
  zipfs 布局——裁定 rtcl 保持自有布局还是模拟 zipfs 值）。

### T8 待复核定性（已全部复核，2026-11-25）

- [x] objmech —— 输出一致，误报/瞬态，不入表。
- [x] cmp_string_index —— `string is integer` 数字字面量口径，归 T1。
- [x] s_probe —— `string is integer 08/0o` 判定，归 T1。

### T9 语料与工具链（迁移收尾）

- [ ] **语料从 9.1 官方测试套件重提取**：`judge/extract/extract.py`
  （自检 oracle 已默认 tclsh9.1），提取源指到 9.1.0 的 `tests/`
  （zip 里已有），重跑提取重建 case 级 burndown。
- [ ] **bench 基线对 tclsh 9.1 重测**（性能参照换版后 0.6-2.26× 数字
  全部重标）。
- [ ] **迁移完成后**：docs/PRODUCTION.md 差异分析改写为对 9.1；
  `JUDGE_ORACLE=tclsh` 保留为 8.6 历史基线入口。

## 性能优化（剩余杠杆）

- [x] **name-keyed 超指令（Round G，本批次提交）**：全局/eval 级循环的
  `CmpVar*Jump*`/`AddVar*`/`LappendVarVar`/`IncrVarPop`/`StoreLocalPop`；
  `ByteCode::unfused()` 桥保 JIT 全绿（`docs/JIT-OPCODE-DELTA.md`）。
  dloop 0.69x、sum_only 0.72x、fe 1.29x（指令数比）。
- [ ] **JIT 原生 lower（可选，JIT 侧）**：接受 A 族并直接 lower，免去
  `unfused()` 展开（清单见 `docs/JIT-OPCODE-DELTA.md` §4）。
- [x] **nanopass 融合（Round F，已提交 3964c97）**：`fuse_slot_pairs` → 
  `fuse_pairs` pass + 融合 op（cmp+jcc ×4、lappend/incr pop 版）——
  build-only 1.29x→0.85x 指令。
- [x] **fe 求和迭代（p 段，Rounds G–K 收尾）**：p 循环 9 op→**2 op**/迭代
  （`SET_SS_ADD` + `FOREACH_NEXT_CLOSE`），foreach_bind 快路径、body-mark
  折叠、融合分支直取布尔——fe 0.96x 指令 / 0.85x wall、build 0.76x。
- [ ] **表示层重构（用户已授权，Tcl byte-2-byte 语义为红线）**——deep-dive 批次：
  0. ~~`VarMap/VarSet/UpvarMap` 键 `Rc<str>`/`Name`~~ —— **Round S 实测
     中性偏负，已回退（见 BASELINE）**：ptr 命中省下的 memcmp 抵不过 hash
     与句柄克隆；键表示层在此架构关闭。剩余三条仍有效：
  1. **`Value` 字符串表示层可共享**：`ValueInner.string` 从 `OnceCell<String>`
     改为可共享句柄（如 `StrRep::Shared(Rc<str>) | Owned(Box<str>)`）+
     `Value::str_handle() -> Option<Rc<str>>`：常量池/派发站点名共享同一句柄，
     命令名/变量名的比较走 **ptr 快路径**（oovars/dloop 画像里 memcmp ~8%
     + hashbrown 探测的字符串比较部分是目标）。注意 `append`/字符串改写走
     COW；JIT 不触 Value 内部（仅 ABI）✓。
  2. **`VarMap/VarSet` 键 `Rc<str>` + ptr-first Eq**（`Borrow<str>` 保 &str
     查询）：link/prelink/OO memo 路径用共享句柄插查，探针 eq 去 memcmp；
     插入时用已有的 memoized `Rc`（而非新 String）。
  3. `call_proc` 帧准备缓存（按 ProcDef 预置 slots/绑定顺序）——fib 画像
     call_proc 12%。
  4. 空循环基线（dispatch + region 簿记）：`bodies` 预留 + unchecked push、
     `VmState` 帧指针缓存（unsafe 面小、收益 ~5-8%）。
- [x] **OO prelink 去税 + 变量语义修正（Rounds Q/R）**：注册一次性、
  `(qualified,ns_part)` 预计算、去冗余 seed、memo 身份补 ns —— oovars
  0.69x→**0.58x** 指令 / 0.96x→0.70x wall；新增 judge 回归
  `oo_instance_vars.tcl`（88/88）。
- [x] **dloop 画像结论（0.68x）**：剩余成本 = 派发式 `incr acc [dict get …]`
  的名字扫描（`split_array_ref`/`redirect_flat`，每次访问 2-3 扫）与 dict
  插入（IndexMap rehash/alloc，摊销固有）——进一步需下表的 VarMap 键表示层。
- [x] **rtcl-vm 边界 unfused()**（Round P）：`execute()` 入口展开融合 op
  （与 JIT 同桥）；canonical 集之外的面（lappend 族）维持其既有显式拒绝。
- [ ] **OO 派发 per-call 字符串面（Round O 画像，oovars 0.69x）**：
  1. prelink 索引预计算：链 memo 构建时把 `(slot_idx, key, name)` 预先算好
     （编译体 locals 已知），每调用安装省去 `resolve_var_link` memo 探测 +
     `slot_index_of` 线性扫描（≈5% oovars）。
  2. `ChainEntry` 携带 owner 变量表 `Rc`（9 个构建点），去每调用
     `owner_variables` 的表查找（≈2%）。
  3. 派发站点级 OO memo（`ResolvedCmd` 增 OoMethod 变体 + backfill 探测），
     去 KeyMemo/ChainMemo 的每调用哈希+memcmp（≈4-6%，改动大）。
  4. `head_cache` 的 typed 比较 → 与调用站点共享 `Rc<str>` 身份（需
     `Value` 字符串 rep 可暴露 Rc，或站点级持有）。
- [x] **站点惰性化（Round W，审计后关闭）**：读取点审计 8 处（foreach
  形状门/控制构造行基/namespace 辅助/tree-walk 词读）→ 收益 15-20
  instr/命令 vs 引入 (unit,site) 惰性解析 + 错误路径双模型 → 不做。
  结论见 BASELINE「Round W」。
- [ ] **下一批（按预期收益，Round L+ 候选）**：
  1. `BEGIN_CMD` 的 `word_srcs` 借用化：`cur_cmd_word_srcs` 改 `(*const
     SrcSpan, len)` 新type（site/单元生命周期内有效；save/restore 只拷指针），
     去每命令 rc++/--——p_only 画像里 `op_begin_cmd` 5% + `drop<Rc<Vec<SrcSpan>>>`
     3.7%；预期 fe/fib ~5%。**unsafe，需严肃审 lifetimer**（跨嵌套 eval 的
     saved 引用链：调用者的单元在其 `exec_bytecode` 帧存活期间一直有效）。
  2. fib 调用路径：`call_proc`（画像 11%）——参数绑定零拷贝化、`slot_mode`
     门的检查合并、`ns_of_qualified` 的每调用开销。
  3. 空循环基线 524 vs 337 instr/迭代：dispatch + region 簿记（Vec push/pop）；
     候选 = `bodies` 预留 + unchecked push（unsafe）、`frames.last()` 缓存
     （VmState 帧指针 + generation）——深水区。
  4. 更多语句窗口：`incr x [expr {...}]`、`set x [cmd ...]`（含 dispatch 的
     bracket 结果窗口）、`[SubEnd, DynCall/Call]`。
- [ ] **foreach 路由对齐**（G9f 存档，G24 已落 framed 双模式的一半）：
  全局编译 foreach 已工作（G21 实验 + G24 framed）✅ **已完成的
  部分：全局内联 + framed 错误协议**。剩余：`uplevel`/`namespace eval`
  上下文的 `foreach_lexical` 路由镜像验证（`foreach_inline_shape`
  的 word-srcs 条件）——用 sweep 逐上下文验证。
- [ ] **vars/oo 链接变量 1.7-1.8× → 指针稳定存储**：VarMap newtype
  （封装 insert/remove/clear/retain，自动 generation bump）+
  链接变量的 `*const Value` 指针缓存（traced 变量走原路径）。
  方案存档于 BASELINE G9f 节。预期 vars/oo 收敛至 ~1.2×。
- [ ] **dloop 收尾 1.05×**：cmd_dict 的 IndexMap 查找为固有大头
  （48%）；`get_index_of` 字符串键 → 可尝试 Rc<str> 键 + 指针缓存。
  `bench/scripts/dloop.tcl`。
- [ ] **oo 1.19× 收尾**：[typed, method] 头 Values + args Vec 已池化
  （G25）✅。剩余：`invoke_method`/`exec_chain_entry` 的分层调用
  （3 层函数）可合并；`resolve_object_arg`（String 化）的调用点
  梳理。`bench/scripts/oo_bench2.tcl`。

## 性能候选（评估结论存档，未排期）

> 来自 clippy 归零批次（type_complexity alias 化）途中的性能评估——
> 本轮只记录不实施；bench 显示热点后再入「性能优化」。

- [ ] **`Rc<Vec<ChainEntry>>` 双重间接 → `Rc<[ChainEntry]>`**：切掉一层
  Vec 头部（oo 链 memo 与 `ActiveMethod` 快照共用该形状）。机械小改，
  列为第一个 bench 候选（`bench/scripts/oo_bench2.tcl`）。
- [ ] **嵌套 memo 复合键**（`chain_memo`/`key_memo`，现 `ChainMemo`/
  `KeyMemo`）：双层 `FxMapK<Rc<str>, …>` = 每次查找两次哈希 + 两次
  Rc 间接。候选：单层复合键 `(Rc<str>, Rc<str>)`（(ns, name) 对，一次
  哈希）。**优先级低**——这些是冷路径 memo（miss 才付出），bench 显示
  OO 热点后再做。
- [ ] **offset+len 替代 `Rc<str>`（用户提议，评估为大手术）**：当前
  `Rc<str>` 已通过 refcount 免拷贝（parse_cache 以整段源码为键，word
  级无重复分配）；改 span 模型需动 parser 的 Word 表示与缓存键设计。
  收益点在 wasm 侧（减少分配器往返）。前置条件：bench 显示源码分配
  占比 >5%。

## 语义对齐（与 specs/DIVERGENCES.md 同步，复核 2026-10-07）

- [x] **case 级 4072/4072 严格对齐达成**：全量审计确认 0 项 rtcl 引起
  的语义分歧；gen_namespace-old 的 5 例 FAIL 为 tclsh 独立运行时的
  上下文失败（缺跨 case setup），rtcl 逐字节复现（非 rtcl 缺陷，
  见 DIVERGENCES.md"非分歧"节）
- [ ] 可选：extract.py 补跨 case setup 提取，使 5 例语料自洽
  （tclsh 侧协议变更，谨慎）
- [ ] `interp alias {} name {} target` 创建形式（空解释器占位参数；
  唯一 OPEN 语义分歧，严重度低——DIVERGENCES.md D-1）
- [x] 整数溢出 f64 提升 —— 已改为精确 BigInt 宽化（gen_expr 大数幂
  用例逐字节一致）
- [x] rename 遮蔽过程后旧名解析 —— 复核为两引擎一致（原记录有误）

## 原生 Tk —— 需 nsh/fork 渲染侧升级的任务（2026-11-12）

> rtcl 侧的纯语义差距已随 wish 对照（`specs/tk/run.sh` 八遍：字体
> 度量/几何/事件/selection-虚拟/winfo/子命令面/wm-tkwait/drag-drop）
> 基本清零。以下是需要 nsh fork 渲染器功能升级或平台工作的项——
> rtcl 模型侧已就绪，对照基建（八遍脚本 + tkprobe）可随时回归。

- [x] **`-alpha` 渲染通道**（2026-11-16 收口）：fork 663210d 加
  `DesktopCommand::SetOpacity`（Linux/GTK 窗口 set_opacity 落地；
  Win32 分层窗口 / macOS alphaValue / GPUI 以 no-op+注释标注接线点，
  本环境不可验）+ nsh-testing Screen 记录 opacity；rtcl 885a136 接线
  attributes 钩子 + 读回格式修齐（Tcl double "1.0" 形态）。回归：
  headless `alpha_nsh`（0.0/0.5/1.0 全链路）。wish 侧读回在无合成器
  X11 上中间值异步归 1.0（Tk 8.6.17 实测），不适合精确 diff。
- [x] **字体栈统一**（2026-11-16 收口，量化后小修 8bdbf95）：模型面
  逐字已齐（font measure 139/151/161 vs wish 139/152/161）。~15% 渲
  染偏差定位于 **canvas text**：fill_text 从未设置字体，painter 保持
  `10px sans-serif` 默认（117.5 vs wish 139 = -15.5%）；修复走 item
  `-font`→resolve_font→CSS shorthand（137.5，-1.1%）。label 路径本
  有 4/3 点-像素换算（140 vs 140）；DejaVu/Courier 残差 3-6% 为 Xft
  与 GTK pango hinting 差（方向相反，不追）。RichEditor 的 px/pt 混
  用同修。模型侧零改动，度量遍无涟漪。
- [x] **ttk 主题视觉化**（2026-11-16 小修 fbe82c9 + 残余交分析）：
  截图对比发现脚本中断级缺口——ttk::scale 的 `-value` 被拒（整段脚
  本创建处 abort，后续控件全部不渲染），补 SCALE_OPTIONS + 创建时
  应用；按钮标签居中（Tk 默认）；progressbar 默认水平（option_default
  补 -orient）+ 填充条从固定 px 改轨道份额。**残余（大改，不做）**：
  四主题（default/alt/clam/classic）渲染同一套现代扁平样式——按主题
  选样式表是渲染器侧样式系统重构；combobox 无输入区、disabled 未置
  灰、radio 指示符形状等属该系统内的逐元素打磨。
- [x] **treeview drag/drop DnD 深度**（2026-11-16 收口 3124e16）：
  复验确认渲染面无入口（表头只有 on_press）。交互落地：runtime 抽
  共享写核 + `treeview_resize_column`；表头单元格按住 + 表头移动 =
  实时改列宽（模型 bump → 重渲染跟随）。dnd 对照遍绿（模型面零涟
  漪）；手势本身 headless 无 mouse-move-with-button 原语可模拟，
  live 才可验。简化：Tk 的调宽挂分隔线/表头按下是列移动重排，rtcl
  v1 把调宽挂表头单元格（无分隔线元素与重排面）。
- [ ] **渲染器 wrap 一致性**：渲染折行点已与模型一致（截图验证），
  但渲染行宽比模型窄约 15%（同字体栈问题）；`text_request_pixels`
  已导出，渲染器按其约束 layout。
- [ ] **a11y 深度**（fork：nsh-platform-* access；2026-11-16 评估）：
  现状 = nsh 侧 per-view GtkAccessible 已通（access.rs 角色/标签/
  description/announce），rtcl 桥接面仅 3 处关键标签（Close/
  Increment/Decrement）。可行阶梯：A) 桥接批量补 accessibility_label
  （从 -text/-label 提取，小改高值）；B) Canvas/Chart 逐项语义需 fork
  canvas 暴露 per-item a11y 子树（canvas 是单元素自绘，AT-SPI 无逐项
  节点）——fork API 设计题，大改；C) 菜单键盘导航 + AT-SPI 树验证需
  读屏/accerciser 环境，本环境（Xvfb 无 AT-SPI 总线）不可完整验证。
- [ ] **IME 深度**（fork：nsh 原生 input/RichEditor；2026-11-16 评估，
  建议降级/收口）：Tk 8.6 的 XIM 语义 = 只在 commit 后产生 `<Key>`/
  `%A` 事件，组合中状态不经 Tcl（XFilterEvent 吞掉中间态）；rtcl 的
  组合完成字符已由 nsh 原生 input 承担——即残余的「组合中状态进
  %K/%A」实际**超出** Tk 语义，做齐反而偏离对照基准。preedit 视觉
  由 nsh 原生输入层负责，不经桥接。建议：确认对齐即可，不实现。
- [ ] **MSVC 构建矩阵**（平台）：Windows 目前是
  x86_64-pc-windows-gnu + MSYS2 路线（全绿）；MSVC 工具链未测。
- [ ] **macOS 平台**（硬件）：nsh-platform-macos 存在，未验证；
  Homebrew/部署链待建。

## 基础设施

- [ ] rtcl-core `embedded` no_std 构建修复（G5 起破损，~2200 错误；
  OnceCell/线程本地 freelist 的 no_std 替代是主因）
- [ ] perf 依赖：`kernel.perf_event_paranoid` 每次重启回 4——写入
  sysctl.d 或 CI 环境预设
- [ ] JIT（rtcl-jit）暂停中：内存管理/编译管线设计已定稿（路线 C，
  `docs/JIT-WASM-GC.md`）；任务包已就绪（`TASK-JIT-WASM.md`，重启时
  直接派发给新 agent）；若重启，按新文档 M1'（ABI v2 + Int 快车道）
  执行（见 HANDOFF.md §5/§6；解释器为主要引擎的前提下优先级最低）
- [ ] JIT 侧步数预算：A1 eval 步数预算（`set_max_steps`）只覆盖解释器
  与 vm_exec 回边——JIT 编译的 proc 内纯控制流循环（`while {1} {}`
  编进 wasm 原生循环）绕过全部计步点。**方案已定**：fuel/epoch
  imported mutable globals（`docs/JIT-WASM-GC.md` §3），随 M1' 落地。
- [ ] lazy-string（独立于 JIT 的 wasm 边界优化，可先行，设计见
  `docs/JIT-WASM-GC.md` §6）：`InternalRep::HostStr(u32)` +
  `HostStringBacking` trait（choke point = `value.rs` 的 as_str 物化
  入口）；rtcl-wasm 默认 on。前置：string.allium 加 astral probe 裁决
  `string length` 三方语义。

## perf/interp-arch-opt 评审跟进（2026-10-18，PR #1 合并时记录；wave-2 已修大半）

- [x] **P1 prelink 去 seed 语义回归**——wave-2 已修（master `materialize_frame_links`）：
  ns 删除/OO 销毁清除 `ns::*` 前，把存活帧的 `variable`/`global` 链接值补种进
  frame.locals（零热径成本，快照比 master 安装期 seed 更新）。探针恢复 tclsh
  一致（constructor 预设值在 `namespace delete [self namespace]` 后可读）。
- [x] **P1 OO proc_memo 单槽 × 实例 ns 身份**——wave-2 已修：单槽改
  `(owner vars, 实例 ns, method 名)` 键控、上限 4 的 evict-oldest 多槽，
  多实例交替/renamemethod 不再全量重编译乒乓。
- [x] **P1 consts_code ABA 窗口**——wave-2 已修：身份键从裸指针改持
  `Rc<ByteCode>` 保活（状态池复用期间防地址复用误判）。
- [x] P2 proc_memo 身份缺 method 名——随多槽键控一并修复（renamemethod
  后 arity 文本与 tclsh 逐字节一致，探针钉住）。
- [x] P2 方法帧 `info vars` 不列 variable 链接名——wave-2 已修：链接名在
  目标存在时并入列表（`interp.get_var` 探测，不发 trace）。
- [x] P2 tier1_epoch 名单缺 `lappend`——wave-2 已修：名单补 lappend，
  双侧加 KEEP IN SYNC 锚定注释（vm_exec note_tier1_mutation ↔ compiler
  Tier-1 fold match）；宿主 register/delete_command 也补 bump（原来只
  bump cmd_generation）。探针：shadow lappend 后已编译 proc 正确改走分派。
- [x] P2 融合臂慢路径直调 cmd_incr/cmd_lappend 跳过 `err_code_raised`
  重置——wave-2 已修（5 处补 `err_code_raised = false`，与 dispatch
  builtin 臂同 idiom）；lappend 快路径吞 set_var 错误改 debug_assert。
- [x] P2 varmap.rs 注释错误（"abc" vs "abc\0" 哈希可区分）——已改为如实
  描述（HashMap 等价比较兜底，最坏一次多余探测）。
- [ ] P2 ctor/dtor 未迁移 prelink（双轨 OO variable 链接实现并存；ctor/dtor
  每对象创建仍付拼串+重解析成本）。
- [ ] P2 `oo::objdefine <obj> renamemethod` 对类方法静默成功且不改名
  （tclsh 报 "method old does not exist"）——wave-2 探针发现的预存分歧。
- [ ] P2 PrelinkEntry.registered 一次性注册无 ns 重建重注册路径（触发面
  极窄：对象 ns 被删后按同名重建）。
- [ ] P2 内联折叠 Tier1 命令不计步（直线大编译体 max_steps 低估）——
  设计取舍，文档化即可。

## 已完成（近三轮，供对照）

- [x] G22 槽内存减半（Vec<Value> + UNSET 哨兵）
- [x] G21 槽超指令融合（Add/Cmp 的 slot 变体）
- [x] G20/G19 Fx 哈希（expr_check/namespaces/code/bytecode/OO 表/备忘录）
- [x] G18 arrays_env_only 标志
- [x] G17 LappendVar 内联快速路径
- [x] G16 执行器栈裸 Value 化
- [x] G12+G14 内联整数 Value + 全算术/比较立即数快速路径
- [x] G13 链接键缓存（单次 globals 探测）

## OO 优化第二批（perf/interp-arch-opt-2，2026-10-18）

**已落地**
1. ✅ head_cache 身份比较（`Value::same_allocation`，免 memcmp/`to_string`）— `94957ef`
2. ✅ ChainEntry 预取 owner 变量表 Rc（memo 检查免表探测；`fresh_chain` 门控）— `94957ef`
   → oo2 **0.48x**、oovars **0.57x**；全门禁绿（tests 1256/0、judge 88/88、sweep 0）

**已实验关闭**
6. ❌ while 体尾融合 `[BeginCmdIncr*Pop, Jump]`→单 op：while 微基准 **+1.54%**
   （+12 instr/迭代，热派发面增长吃回）→ 回退留档（`d85033d`，Round Z）。
   注：micro_incr 的 for 体尾是 fall-through（无 Jump），该窗口对它无效。

**已落地（续）**
3. ✅ OO 派发站点级 memo（`OoCallCache`，`04fd3ed`）——身份(typed/method
   `same_allocation`)+ns 指针+`mutation_ctr` 四项验证，命中直达
   `exec_chain_entry`。oo2 **0.43x**、oovars **0.52x**（各 −10.5%/−8.3%）。
   续做实验（`f8c3007`/Round AB）：`my` 复用缓存与免 `format!` 门禁均无正收益
   （单槽被交替方法冲刷，−2.89%/−0.01%）→ 回退；后由 Round AG 的 `my` 双槽
   落地解决（17.466G→11.827G，−32.3%），多槽索引不再需要。

**待做（按序）**
4. ✅ call_proc 帧准备（Round AF/AH/AI）：画像定位后落地「去三连 Rc clone +
   take 守卫 + ns 身份写/池化帧保留」——fib25 507.46M→**493.75M（−2.70%，
   0.85x→0.83x）**、oo2 −1.38%、oovars −1.01%；批次 3（借用合并）实测净负
   已回退（`aa5b3cf` 收口）。剩余的后期配方（原条目保留作参考）：
   `CARGO_TARGET_DIR=/tmp/opencode/rtcl-prof CARGO_PROFILE_RELEASE_STRIP=false     CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C force-frame-pointers=yes"     cargo build --release -p rtcl-cli` → `setarch $(uname -m) -R perf record     -F 8000 -g ./target/release/rtcl -f bench/scripts/fib25.tcl` →
    `perf script` 导出后 python 自聚合（本机 `perf report` 会超时）。
   已审读 call.rs 的候选（逐项都 <1%，需画像确认可叠加值 ≥2% 的组合）：
   level0 每参 clone+push（~20 instr/调用）、slot_mode 门禁的
   `Rc::clone(code)`+`slots.resize`、`ns_of_qualified`+两次 ns Rc clone、
   帧池收发与 11 项字段 clear、返回值/`call_depth` 管道。
   注意红线：`info level 0` 的语义是「调用时刻的词快照」，level0 惰性化
   不可行（args 切片不驻帧；且调用后无法重建）——除非能证明全局无读者。
5. ✅ 关闭（有证据，2026-10-18）：S1 被 Round AA/AG 缓存提前兑现；S2 经
   成本/收益评估判定不做（每次短串构造加哈希 vs 罕见站点收益；板面不覆盖）；
   S3 原计划不做。判定与理由见 `docs/DESIGN-value-handles.md` 1.5/1.6 节。
   （下方为当时的中间状态记录）：派发名的 identity 命中已在
   对象派发（单槽）与 `my`（双槽，−32.3%）落地；`dispatch_dynamic`
   （`$o $m` 变量名）与字面量同速。**S1（名字构造点显式共享）已被
   上述缓存实现取代**——设计文档已加「实测修订」节。剩余价值集中在
   **S2（短串驻留在 Value 构造点，服务「拼出来的名字」）**：收益不在
   板面，且需先在 micro_add/lappend 上证明数据串无回退；S3 暂不计划。
   结论与证据见 `docs/DESIGN-value-handles.md`。

**其余候选**：`call_proc` 帧准备之外的四板点（fib 0.85x / fe 0.92x / dloop 0.68x /
build 0.72x）已在上一节定界；micro_add 0.89x、micro_lappend 0.72x（已 <1x）。

**perf/wave2 深水区轮（2026-11-16，基线 7f2e0df，收口见 BASELINE「深水区组合拳」节）**：
1. ✅ call_proc 帧池双清（458c80e）：池不变式成文（两 push 站点全清 +
   coro drain 不回池）→ pool-pop 11 项 clear 全删 + first_iter 重绑门控
   ——fib25 **−5.66%**（578.67M→545.97M，0.97x→0.91x）、oo2 −2.26%、
   oovars −1.87%、fe/dloop/build 持平。门禁全绿（tests 1419/0、judge
   88/88、probes 381/34=基线±objquad 抖动）。
2. ❌ 语句窗口 `incr var [script]`：完整实现（4 op+编译器+融合窗）语义
   全绿但**净负**（dloop +107%），判别实验坐实窗口运行时（关编译器分支
   即回基线）；根因未收敛，两次止损回退。reopen 线索：RTCL_DBG 逐写点
   插桩对比新旧每迭代 flush 序。
3. ✅ 关闭：word_srcs 借用化 Round L 已落地（§10.2 条目陈旧）。
4. ✅ 关闭（画像后不动）：micro_base 每迭代仅 2 op；残项 frames.last
   2.06%/Region push 1.58%/is_unset 5.7%（语义必需）——台账 5-8% 估计
   系 Round V 时代数字，结构性项不动作。
5. ⏸ 缓议：vars 指针稳定存储——目标数字已消失（oovars 0.75x/oo2
   0.65x 均 <1.0x），VarMap 全消费面改造（Round S 实测 178 处）性价比
   为负；待 dloop/vars 重新放大再评估。

**本轮后板面**（指令比 rtcl/tclsh）：fib **0.91x**、fe 0.98x、dloop
0.95x、oo2 **0.63x**、oovars **0.74x**、build ~0.72x；micro_base 1.16x、
micro_incr 1.42x（派发地板，见 BASELINE 板点 4 节）。

## 代码审计后续（2026-11-23 轮，specs/CODE-AUDIT-20261123.md）

审计确认 30 余类脚本可触发的崩溃/挂起与 11 类静默错误，逐条清单与
复现见 `specs/CODE-AUDIT-20261123.md`（编号 S/C/P）与
`judge/crash-probes/`（门禁 `judge/crash-gate.sh`，提交前与
judge/sweep 同跑）。下面只列**结构**动作：按类别消除，而不是逐点打补丁。

- [x] **M1 止血**（S1–S10，e513371）：12 个 panic 点——`&str` 按字节切片、Rust
  格式化/时间库的边界、脚本可控尺寸的裸分配。全部是局部修复。
- [x] **M2 栈安全**（S11–S22，c298b77、0725bd2）：唯一的守卫 `call_depth` 只数 proc/内建
  调用。目标形态 = 一个 `StackGuard`：脚本驱动的重入（unknown 处理器、
  ensemble 重分派、alias、trace 回调、嵌套 `[…]` 求值、`eval` 嵌套）汇
  到同一检查点；解析/编译（expr 递归下降、`try_compile_sub`、正则
  parser）各自带深度上限；**数据驱动**的递归（`serialize_list`/
  `serialize_dict`、`drop_last_owner`、JSON 解码/编码、regexp dissect）
  改显式栈迭代——它们没有"报错"的位置，只能不递归。递归上限按目标栈
  校准（wasm 上 650–1000 层即 trap 并毒化实例）。
- [x] **M3 资源治理**（S21、S22、S28，129966d）：`InterpLimits { max_steps,
  max_call_depth, max_value_bytes, max_list_elements }`，wasm 带默认值
  并经 `RtclHandle` 暴露；脚本可控尺寸的分配一律走 `try_reserve` 并
  返回 Tcl 错误（Rust 默认分配失败 = abort，`catch` 拦不住）。
- [x] **M4 静默错误**（C1–C15、C19–C27；两批，同一提交）：其中 C1 是
  结构性的，见下"op 码表"。第二批由第一批的门禁暴露（回归 `oo17`
  + 死探针 `smallface_p24`），顺带修掉 S41（表达式里的 `#` 死循环）。
  - [ ] 余项：C16 ARITH 首帧关键词（**两引擎不一致**，sweep 未覆盖）、
    C17 `lt/le/gt/ge` 运算符（语言层取舍）、C18 `READ/LOOKUP VARNAME`、
    C28 `namespace eval` 内的 `tailcall`；C29 归入 M6。
- [x] **M5 OS 面**（S31–S37）——已完成：exec（S31 死锁、S33、C30）、
  `close` 连接中的 socket 与 `exit` 不等子进程（S32）、文件安全四项
  （S34–S37）。余项另记：C31 `@通道` 重定向、C32 宿主 exec 路径、
  C33 `file attributes`/`~user`/`socket -server`、C34 跨文件系统
  rename、S38 HTTP 超时、S39/S40。原计划：exec 两路并发排空、后台 stderr drain、
  失败路径 kill+wait；`close` 对连接中的 async socket 直接丢弃；
  `file tempfile` 用 `create_new` + 0600；`file delete` 非 `-force`
  只删空目录；`-async` fd 加 `SOCK_CLOEXEC`。
- [x] **M6 能力模型**（S24–S27）——已完成：`_js_dispatch` 按调用它的
  解释器找回调（S24）；子解释器继承宿主的控制台与时钟、步数计数器
  整个家族共用、递归上限不高于父（S25）；`interp` 命令族按 tclsh
  `tclInterp.c` 重做——路径、`create ?-safe?`、`$path` 自身的命令、
  hide/expose/hidden/invokehidden（S26、S27、C29），别名的目标可以是
  父、自身或兄弟解释器，别名环在建立与 rename 时拒绝；安全解释器里
  `hide/expose/invokehidden/marktrusted` 一律拒绝。门禁：探针 d21
  （295 条断言，两引擎）、`crash-wasm.mjs` 52 项。
  - [ ] 余项：S23（栈溢出后实例失效，未修）；`interp limit`、
    `bgerror` 设置、`cancel`、`share`、`transfer` 未建；C32 宿主 exec
    路径；C35–C39（见审计文档；C39 是有效递归深度只有 tclsh 的一半）。
    原计划里的「`exec` 入口检测半途状态并复位」没有对应的缺陷：M5
    重写后的 exec 没有跨调用的持久状态（d19 探针覆盖成功与失败交替）。
- [ ] **M7 性能**（审计文档 P1–P15，一项一个小提交，每项带前后
  数字和一条探针）：
  - [x] P2 `string map`：400KB 从一百秒跑不完到 2.7ms。
  - [x] P4、P14：变量插入日志回收（96MB → 7.8MB）、`info vars` /
    `info commands` 去掉平方项、`array names` 不再逐条查找；顺带修掉
    C40–C43（数组命令的绝对名字、`array statistics`、`info locals`、
    `apply` 的 errorCode）。
  - [x] P8 的调用期部分：模式编译缓存与匹配器池、纯文本模式走搜索、
    长文本先制表前 1024 个字符；顺带修掉 C44–C46（`switch -regexp`
    与 `regexp` 不是一个引擎、`array names -regexp` 逐元素编译、
    `regsub -all {}`）。
  - [x] P15 `string length`/`index`/`range`/`first`/`last`：24KB 的串上
    13–18µs → ≤1µs（探针 d25，209 条断言）。
  - [x] P3 `append`：proc 里 2 万次追加 49ms → 1.5ms（接到变量自己的
    缓冲区上）；`lappend`、`dict set`、`incr` 的快路径同样改成就地改。
    顺带修掉 C48（`unset` 后重建的全局变量在 `info globals` 里的位置）
    和 C50（变量读写出错时的 errorCode）。探针 d26（79 条断言）。
  - [ ] 数组读写；P7 编译期；P1/P9 通道缓冲读；P5 `array size`；C47
    回引预算；P15 剩下的三处（`string last` 命中、长模式未命中、非
    ASCII 的字符计数）。
  - [ ] 这一批里对出来的新差异：
    - [x] C49：proc 里执行的 `namespace eval` 体读不到、写不进命名空间
      的变量，反而覆盖 proc 的局部变量（两引擎）。顺带修掉 C52（体里
      用裸名访问全局变量时 trace 不触发、数组子命令不走链）和 C53
      （有条目没值的全局变量不接写，读/`unset` 的 errorCode）。探针
      d27（139 条断言）。
    - [ ] proc 里 `info locals` / `info vars` 的顺序：tclsh 是编译局部
      变量在前、运行期建的在后并按哈希序，rtcl 按建名的先后；而且
      每次调用都把整个帧降级成名键存储（之后这一帧都走慢路径）。
    - [ ] C51：trace 与编译后的命令（`append` 每个值一次写 trace、读
      trace 报错后 `incr`/`lappend` 的行为、`unset` trace 的 errorCode）。
- [ ] **工具化防线**：`judge/crash-gate.sh` 入门禁（已入：原生 118 次
  运行、wasm `judge/crash-wasm.mjs` 52 项带 worker 看门狗）；**排查
  其余探针是否也在两个解释器上共同早退而空转**（`smallface_p24` 是）；`rtcl-vm` 开
  clippy `unwrap_used` / `indexing_slicing` / `arithmetic_side_effects`
  （先按文件棘轮，再全量）；`judge/panic-census.sh` 的计数只许降。

### op 码表（ISA 方向，M4/C1 的结构性解法）

现状：指令是 `Vec<OpCode>`——141 个变体的 Rust 枚举，最宽变体 5 个
操作数，`size_of::<OpCode>() == 16`（`Pop`/`Dup` 也占 16 字节）。
后果有四条，都能在审计里找到对应的缺陷：

1. **操作数裸 `as u16` 收窄**（C1）：常量池/局部表/argc 超过 65535 即
   静默截断；`validate.rs` 在运行期是死代码。
2. **每个 op 前的两次分类 match**（`exec_inner` 的 `begin_cmd_site` 与
   `top_op_class`）：带载荷的枚举无法转成下标，只能靠 match。
3. **超指令的七处同步**：枚举变体、编译器发射、热通道臂或 `exec_op`
   臂、`top_op_class` 臂、`begin_cmd_site` 臂、`unfused()` 臂、JIT
   lower 臂——每加一个融合要改七处。
4. **宽窄两套变体**（`PushConst`/`PushConstWide`）由枚举承担了本该
   属于编码器的宽度选择。

目标形态：

```rust
#[repr(u16)] enum Op { PushConst, LoadLocal, CmpSlotImmJumpFalse, /* … */ }  // 纯码，无载荷
struct OpInfo {
    name: &'static str,
    flags: OpFlags,                    // PURE | HAS_SITE | TIER1 | JUMP | CALL
    operands: [OpKind; 5],             // Const | Local | Slot | Imm | Site | Target | None
    lowers_to: &'static [(Op, &'static [OperandMap])],   // 超指令的声明式展开
}
static OPS: [OpInfo; N];               // 唯一真源
```

- 指令流 = `op: u16` + 按 `OpKind` 选宽度的操作数（窄形默认、值放不下
  时发宽形）——宽度选择回到编码器，C1 那类截断由构造消除。
- `top_op_class` / `begin_cmd_site` 变成 `OPS[op].flags` 的位测试；
  `unfused()`、反汇编、JIT lower 读 `lowers_to`；新增融合 = 加一行表
  + 一个执行臂。
- `validate` 改成编译期断言（debug/test 构建必跑）。

**分两步，风险不同**：

- 第一步（小、可先做）：只引入 `Op` + `OPS` 表与 `lowers_to`，指令
  载荷仍留在枚举里，先把分类 match 和 `unfused()` 换成查表。
- 第二步（重写级）：换紧凑指令流。直接性能收益**未测**——已知的是
  派发税约 20ns/op（直接线程化可到 7ns，见 PRODUCTION.md §5），编码
  本身占多少需要先 perf 制导；它的确定性收益在正确性与维护成本，
  以及 JIT/AOT 缓存的线性解码。建议随 JIT 重启一起做，由 judge +
  sweep 两个门禁兜底，不插在安全工作前面。

### 其他结构项

- **`Interp` 分解**：126 个字段；错误帧协议靠约 230 处手工保存/恢复
  的标志位（C3/C4 的来源之一）。先把 `err_*` + `cur_cmd_*` + 词法标志
  收进一个带 RAII 守卫的上下文结构，`top_slots` 的失效也走同一个入口。
- **两套手工失效机制**（`cmd_generation` / `tier1_epoch`）合并为一个
  带原因枚举的 `invalidate()`。
- **数组容器**：`array size/exists/get` 现在按作用域变量总数扫描
  （P5），需要 `base → 元素集合` 的索引。
- **通道层**：同步空壳（P1/P9、S32）——缓冲读取 + `poll` 事件循环，
  exec 统一到已有 `poll` 的 `HostExec` 路径。

