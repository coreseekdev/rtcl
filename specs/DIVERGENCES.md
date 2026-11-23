# rtcl ↔ tclsh 8.6.17 已知分歧清单

> 格式：行为声明 | Tcl 8.6 行为 | rtcl 行为 | 复现 | 严重度。
> 修复后标 `[FIXED YYYY-MM-DD]`。最近盘点：**2026-11-03 深水区全量建模**
> （error-deep / dict / matching / io / trace / oo 六份新 allium 规格
> + 探针套件差分实证；trace 蒸馏暴露致命级 D-36 崩溃，oo 蒸馏暴露
> D-40/41 构造析构与分派面）。**2026-11-17 体帧行号 provenance 收口**
> （本节）。条目后括注的
> spec [D-x] 是规格文件内的 spec-local 编号，以本台账为准。

## 2026-11-23 trv_p11 五处语义缺陷收口 + e8/e9 内层帧链式载体（cases 拆出）+ oo2 表述精确化（judge 88/88；workspace tests 1471/0；probes active 437 全绿 + known 11 归档）

三项任务（trv_p11 主修 / e8-e9 帧记账 / oo2 表述）。代码两提交：
7272638（trv_p11）、192d698（e8/e9）。

**trv_p11（仍归档，残差收窄到 1 行）**：五处真语义缺陷全数修复：

- ①② 变量面冒号串语义：解析面新 split_ns_tail（分隔 = `::` 起
  整串冒号、尾段单冒号属名字、尾 `::` 串 = 空名变量）+
  join_var_key（键结构拼接）——canonical_global（前导整串吞
  `::::z` → 全局 z）、canonical_key_for_ns（`set q::` 在 ns w7
  建 w7::q 空名，键 `w7::q::`）、resolve/var_exists 读父检查、
  set 写宿主预检、`namespace which -variable`（相对 `:` 的名字
  就是 `:`——键 `tnv:::`、命中打印 `::tnv:::`；绝对 `::tnv:::`
  = tnv 的空名变量）、upvar 宿主检查（旧 rsplit 对 `::t4:::`
  算出 `::t4:` 恒 miss）。
- ③④ upvar 数组元素别名生命周期（tclsh ObjMakeUpvar 帧局部
  解析）：元素形建链落帧局部占位数组（基缺建占位 array exists
  nosuch=1；基是帧局部标量建链即报 variable isn't array；全局
  同名不顶替）；宿主 unset 后读 = no such variable、写 =
  upvar refers to element in deleted array、括号写 = variable
  isn't array。UpvarLink::Dead 增 was_global（帧局部死链读回退
  不得落 globals）。
- ⑤ ns delete 内外区分：deaden_links_under(inside_tree)——树内
  删除走 teardown 延迟（帧语境种子读仍通，namespace-46.8 钉
  形）；外部删除立即拆表 + install 期镜像清除（读/unset 报 no
  such variable、exists 0、写保持 dangling-namespace）。

残差：探针仅 t2-vars 序行 1 处 DIFF（`auto_path errorCode` vs
`errorCode auto_path`）= oo2 族 ghost-insert 序同源，**定调并入
oo2**（重开条件随 oo2：哈希桶链逐位重放），不单独追。单测
+12（vars.rs 冒号形/upvar 元素生命周期/内外删除双形）。

**e8/e9（cases 归档拆出，转绿）**：`set x [[a]b]` 括号全闭合形
——tclsh 词求值期内层命令 [a] 帧在列，rtcl 曾缺替换词内层命令
帧（词级 defer 的 pending 被外层属主 defer 覆写时静默丢失）。
修法 = err_pending_chain 链式载体：顶替时移链、error_report 文
件级退出先链后顶逐帧落账；其余边界与 pending_top 同生死
（catch/return/error 物化清空、harness 直落帧 ERR_ALREADY_LOGGED
单次消费清空、eval_isolated/trace save-restore 同步、flow 转换
丢弃）。tclsh 语义钉形（11 形双引擎逐字一致）：catch 形三帧
不含替换帧、proc 体边界只落 (procedure) 帧、while 体边界只落
body 帧、flow 转换只有 converting 帧、depth-2
`set x [[ [[a]b]c]d]` 全链五帧。early-commit 形（单元出口直接
落账）被 P4/P6/P9 钉形否决（穿透 catch 边界）。
残差留档：tree-walk 引擎 flow 形 converting 帧读内层 "break"
（tclsh = 外层 set 帧）——**既有缺陷与基线逐字同形，非本轮引
入**；默认 bytecode 引擎正确。重开条件：tree-walk flow defer
载荷对齐。cases/cases2/probe_round3 的 parse 架构残差不变
（条目 14 维持）。

**oo2 表述精确化**：tcl_order "理论边界" → "**投入产出比**边
界"（技术上可修：按 tclsh 内部创建时序做 ghost insert 级重放；
否决理由是成本——每收口一行可能翻红别行、无限细分——而非不可
修）——2026-11-22b 节三处同步。auto_index 标注已修（2912c81）
不变。

## 2026-11-22b known 16→12 深水修复轮（4 修复转绿出档 + 2 条残差收窄留档；judge 88/88；workspace tests 全绿；probes active 436 全绿 + known 12 归档）

known 16 条中的 5 条深水修复（trv_d13/trv_p10、pidx、oo2、globprobe、
we5）。4 条全修复转绿移回 active（trv_d13/trv_p10/pidx/we5），
2 条残差收窄留档（oo2/globprobe）。终态：active 436 全绿、
known 12。

1. **ns delete 两阶段拆除（nukeQueue 近似）**（trv_d13/trv_p10，
   9ef6f88）：tclsh 的 Tcl_DeleteNamespace 分两阶段——外部删先置
   NS_DYING（exists=0）但 ns 实体（命令/变量表 + 父 childTable 挂链）
   在全部 delete trace 走完前保持可见（TclTeardownNamespace 顺序：
   变量逐个 unset → 命令逐个 delete → 最后 unlink）；trace 回调里
   未处理变量仍可读、children 仍含 :foo、which -variable 仍解析
   （UnsetVarStruct 的"表内 undefined"三步）。rtcl 曾单阶段拆除。
   修复：ns_dying 集合（可见性摘除）+ ns 实体延迟到 trace 走完后
   拆表；var_nuke_pending 窗口（which -variable 在 unset trace 期仍
   解析）；dying ns 二次 delete = NS_TEARDOWN 静默跳过。自删
   （activationCount>0 等价）维持立即摘除（in-del 钉形不回退）。
2. **词首引号纪律（subst 括号扫描）**（pidx，9d9e971）：
   tclParse.c 词规则——`"` 只在词首开引文。rtcl 的 subst 括号扫描
   （run_bracket_script/check_bracket_tail）对任意位置 `"` 都置
   quote 态——`expr $foo([x"y])` 的下标替换把命令名 `x"y` 的词中
   引号当词首，吞 `]` 至 EOF。修复：word_start 跟踪（命令起点/
   `[` 后/空白后置真）+ 引文开放到 EOF 报 missing \"。
3. **::auto_index 惰性首建**（oo2 的 auto_index 行，2912c81）：
   tclsh 默认 unknown（init.tcl）一进来就跑 auto_load——auto_index
   **进入 auto_load 即建**（哪怕查找失败）；oo forward 缺目标走标准
   not-found 链同样触发。rtcl 的首建挂在 has_unknown 臂（只认用户
   proc/ensemble），默认 not-found 路径从不落表。修复：首建移到
   unknown 臂入口两臂共用。oo2 的 r4 行 auto_index 落在与 tclsh
   相同的桶位（errorInfo auto_index env）。
4. **parse 错误归属最外层命令**（globprobe 归属半边，41a3337）：
   tclsh TclLogCommandInfo 的 parse 失败引文 = **最外层命令**起点..
   失败点（term = 最内层死点 opener 含）、150-cap 多行截断、行号 =
   外层命令行。rtcl 曾按失败点所在行截引文 + 内层行号。修复：
   ParseError 载荷增加 cmd_start/cmd_line（parse_command 失败上浮
   逐层盖章）+ seed_parse_error 读章 + skip_quoted_in_cmd 的
   missing " offset 改 opener（gp_2 引文 `set x [foo "`）。
   **残差（留档）**：globprobe 的引文**终点**——tclsh term 在 4 层
   互嵌套（引号⇄bracket）递归的最深 opener（行 11）；rtcl 在浅 2 层
   （行 9）。归属行/引文起点/150-cap 已对齐；黑盒二分钉死为位置
   敏感的扫描栈语义差（4 层互嵌套里某层引号闭合点不同），定位需
   复刻 tclParseCommand 的嵌套引文/bracket 递归栈语义（引号词内
   `[` 的错误立即传播、闭合点逐层配对）。重开条件：rd 扫描器的
   嵌套栈改造（gp_1/gp_2/gp_3 最小形已绿，残差仅在 ≥4 层互嵌套）。
   **[FIXED 2026-11-22c]** 引文终点 = EOF 时**最深未闭合 `[`** 的
   opener——白盒定谳（gdb 逐层 trace Tcl_ParseCommand 入口 +
   frameprobe 逐帧重放）:tclParse.c ParseTokens bracket 分支把 `[`
   内容按**逐命令循环**解析（Tcl_ParseCommand(nested=1) 反复调用），
   仅当某条命令的 term == `]` 且 !incomplete 才算闭合；globprobe
   行 9/11 的 `]` 都在引号内（`"]y"`/`"]z"`），bracket 一路吞行
   10/11 直至 EOF，numBytes==0 处 term = tokenPtr->start = **当时
   正在收命令的那层 `[`**（tclParse.c:1163），外层逐层原样上浮
   （`parsePtr->term = nestedPtr->term`，tclParse.c:1136）。rtcl 的
   平铺 depth 计数只能报本调用 opener。修复：parse_cmd_sub 的
   depth 换 **opener 栈**，EOF 失败报栈顶（最深未闭合 `[`）——
   引号/brace 吞吐边界与 tclsh 逐命令循环逐一相同，故栈顶 ≡ tclsh
   的 EOF 失败层。globprobe 双端逐字节转绿移回 active（known
   12→11）；gp_1/gp_2/gp_3、b1/b2/b5、t2/t3、sp1 全量重放不回退
   （probes active 437 全绿）。
5. **err_pending_top 单元相对行**（we5，378d980）：ns eval 体出口帧
   defer 形态（`set r [boomq]` 的 `[boomq]` 词失败）读 stale 的
   最内层日志行（boomq 体行 1）；tclsh = 体相对行 2。修复：
   err_pending_top 载荷 (text, abs, unit_rel)——词替换 defer 位点
   显式携带**体相对行** = 属主行 + 内层日志行 - 1（err_line 此刻 =
   内容单元坐标；we5 `set r [boomq]`@2 + boomq 帧@1 → 2；
   ei_line_attribution D `set x [`@1 + nosuch@2 → 2——两形折算同为
   体行 2）；ns eval 体出口帧挂起在位读新字段、无挂起维持 err_line。
   反向约束全数保持（ei_line_attribution A-J/file_ctx_frames/
   loop_body_err_line×4 双端一致；catch -errorline 族不回退）。

归档移档：trv_d13/trv_p10/pidx/we5 四探针双端逐字节绿，移回
judge/probes/（active 432→436）。known 终态 12：
c5_dis/cmp_unsupported×2（disassemble 设计差异）、dbg_basic
（oracle SEGV）、globprobe（残差见上）、cases/cases2/probe_round3
（parse 架构差异）、oo2（残差见下）、probe7（D-27 策略级）、
trv_p11、utf_U_parsehex（D-27 策略级）。
**[更新 2026-11-22c]** globprobe 引文终点转绿（4，FIXED 见上），
移回 judge/probes/（active 436→437，known 12→11）。
**[更新 2026-11-23]** trv_p11 五处语义缺陷已修（7272638，见
2026-11-23 节）——探针仍归档，残差仅 t2-vars 序行（oo2 族
ghost-insert 同源，定调并入 oo2）。

oo2 残差更新：auto_index 部分已修（3）；残差仅 r4 物化序一行——
tclsh `clog e auto_path errorCode` vs rtcl `clog errorCode e
auto_path`（同桶链 e/errorCode 的 ghost-insert 序，tcl_order
**投入产出比**边界——技术上可修（模拟 tclsh 内部创建时序的 ghost
insert 重放），否决理由 = 每收口一行可能翻红别行、无限细分的成本
而非不可修，2026-11-16 既有定调）。重开条件：哈希桶链逐位重放。

## 2026-11-22 probes 36 DIFF 清账（19 修复转绿 + dbg_trace 探针修正 + 16 归档移档；judge 88/88；workspace tests 全绿；probes active 432 全绿 + known 16 归档）

36 条残差逐族定调（探针 → 根因 → 处置）。**口径落地**：`judge/
probes-known/` 归档目录 + `probes.sh` 改为「active 集必须全绿，
known 单独展示」，过时的 "known-divergent until the errorInfo batch
lands" 头注释移除（errorInfo 批早已落地）。known 每项在本节有
定调条目；归档 = 设计差异/无有效 oracle/理论边界，非真分歧洗白。

### 修复转绿（19 代码修复 + 1 探针修正）

1. **流式 parse 前缀执行**（ix1/ix2/ix4/pidx2/pidx3/pidx4，6 条）：
   tclsh 的文件执行（TclEvalEx）parse 到哪执行到哪——brace 未闭合词
   吸收到 EOF 时**失败命令之前的前缀已经跑过**；rtcl 的 `-f` 整体
   parse 失败则一条不执行。修复：`eval_impl` 的 parse 失败分支在
   文件语境（was_file_armed）回落逐命令流（`eval_prefix_stream`，
   语义同 eval_incremental），编译语境（proc 体等）保持整体报错。
   sp1 钉形：`puts A\nputs B\nt {$foo({)` → tclsh/rtcl 均输出 A B。
2. **parse 错误引文 opener 截断**（t2/t3/b5，3 条）：命令替换内的
   brace 词未闭合（`puts [expr {"{" eq "{"}]`）引文应为 `puts [expr {`
   （起点..opener 含）；rtcl 曾给空串（skip_braced 的 offset=scan
   末端）。修复：skip_braced offset = opener_pos；missing close-bracket
   回归 start-1（b1/b2/b5 钉形：`set x [abc...` 引文一律 `set x [`）。
   注：globprobe 的 150-cap 多行引文形态与 opener 规则不可通约，见
   移档条目。
3. **expr in/ni 右操作数 list 校验**（t4）：`{"a" in $x}`（x=`{`）→
   tclsh 报 `unmatched open brace in list`；rtcl 曾静默空表判 0。修复：
   parse_equality 的 in/ni 臂走 strict_list。
4. **proc 内相对 ns 限定名全局解析**（incr_window_p10_ns + 通用面）：
   tclsh proc 内 `$n::v`/`set n::v` 一律按**全局**解析（nsq 钉形：
   deep::r2 内 `$n::v` 读 `::n::v`——相对限定名的根是全局，与
   current ns 无关）；rtcl 曾按字面名走 proc 局部（incr 从 0 起、
   局部建变量）。修复：resolve_var/store_var 的 frame 分支对含 `::`
   名直落 globals；get_var 的父 ns 检查按原样名剥前导（不拼
   current ns）。
5. **incr read trace**（incr_window_p11_trace）：tclsh incr 读现值
   触发 read trace（Tcl_GetVarFromObj）；rtcl 走 get_var 无 trace。
   修复：cmd_incr 读现值改 read_var。
6. **info vars 全局 fallback + 全局表插入序**（namespace_varmodel2/
   q114b/u72/p-uplevel-ns/trv_p8，5 条）：tclsh 的 `info vars` 在
   非全局 ns 语境 = 当前 ns 表 + 全局表（proc 内仅局部——IVP 钉形
   `argx a g1`）；两表各自 tcl_order 重放、当前表段在前（IVQ 钉形
   A: `bb cc aa` + 全局段）。修复：misc.rs 的 ns 分支合并全局表
   （global_split 分界）+ 分段重放。**全局表 rcFileName 序**：
   tcl_rcFileName 是 Tcl_Main 在 init.tcl 批**之前**设的应用变量
   （`info globals` 序以它为首，同桶链头位）——插入点移到 tcl_version
   前，模拟 tcl_order 重放全序与 tclsh 逐字一致（python 数值验证）。
7. **string is 分类表**（probe4/probe6）：tclsh 的 is upper/lower =
   类别表 Lu/Ll 本身（Roman numeral Ⅷ U+2167 的 Other_Uppercase、
   modifier letter ˡ U+02E1 的 Other_Lowercase 全 0——Rust
   char::is_uppercase/is_lowercase 是扩集）；is digit = **手工滞后的
   Nd 旧表**（devanagari ९ = 1 而 mathematical 𝟘 U+1D7D8 / Adlam
   U+1E950 = 0；regexp 的 \d/[[:digit:]] 是完整 Nd，另一张表——
   re:digit 行）。修复：rtcl-unicode 增 LU 表（is_lu）与
   DIGIT_CLASSIC 表（is_digit_classic，31 段，tclsh 实测反推）。
8. **regexp POSIX 类表裁剪**（utf_char_classes）：tclsh 8.6 的
   `[[:word:]]` 不支持（报 invalid character class 终止）；rtcl 曾
   接受为 \w 同义继续跑。修复：charset::from_posix 移除 "word"。
9. **namespace tail/qualifiers 冒号语义**（trv_p11 部分行）：tclsh
   的分隔符是冒号串（≥2）——`namespace tail a:b` = `a:b`（单 `:`
   是名字字符）、`a:::` = 空。修复：命令层新 split_colon_pair
   （变量/命令解析面的 split_colon_run 不动）。
   **[更新 2026-11-23]** 变量解析面对齐同款冒号串规则
   （split_ns_tail/join_var_key，7272638）——trv_p11 余量见
   2026-11-23 节。
10. **dbg_trace 探针修正**（非 interp 面）：case trace-28.2/28.3 引用
    extract.py 未提取的 traceExecute proc（tclsh 在 enterstep 触发时
    `invalid command name` 终止、后续 case 无 oracle；rtcl enterstep
    未触发继续跑）——剪除两个 case 后双端逐字节一致（含 rc）。
    剪除的 28.2/28.3 记录在案：rtcl 的 execution trace enterstep
    触发/错误传播面缺口仍是 OPEN 项（D-39 残差）。

### 归档移档（16 条，judge/probes-known/）

11. **c5_dis / cmp_unsupported / cmp_unsupported2**（disassemble 设计
    差异）：错误面已全对齐（cmp_unsupported 前 30 行逐字同）；
    成功 case 的 ByteCode dump = rtcl 自有指令集（Instructions/
    constants 格式 vs tclsh 的 push1/invokeStk1 内部指令文本 + 指针
    地址 + epoch）——逐字节兼容不可能。重开条件：无（架构差异）。
12. **dbg_basic**（无有效 oracle）：tclsh 8.6.17 自身 SEGV（rc=139）。
13. **globprobe**（parse 深水）：前缀执行、missing close-bracket 消息
    已修；残差 = 嵌套引号替换跨行形态的 bracket 失败 errorInfo 归属
    （tclsh partial = 起点 150-cap 多行、行 7；rtcl = opener 截、
    行 9）——需复刻 tclParseCommand 的层级 scan。重开条件：parser
    词层递归 scan 改造。**[更新 2026-11-22b]** 归属行/引文起点/
    150-cap 已修（41a3337，gp_1/gp_2/gp_3 转绿）；残差仅引文终点
    （≥4 层互嵌套递归的最深 opener 深度），见 2026-11-22b 节 4。
    **[FIXED 2026-11-22c]** 引文终点转绿（opener 栈，见 2026-11-22b
    节 4 的 FIXED 注），探针移回 judge/probes/。
14. **cases / cases2 / probe_round3**（parse 架构差异）：rtcl 命令替换
    词存惰性文本，parse 期错误（brace 词后多余 `}` → extra characters
    after close-brace）退化为执行期——帧链多一层内层 catch 帧、外层
    引文完整；tclsh 前置整条命令 parse（单帧 + 出错点截断引文）。
    重开条件：替换词主动 parse（架构级）。
    **[更新 2026-11-23]** 历史归档 case 号 e8/e9（`set x [[a]b]`
    内层帧记账）已从本条拆出单独收口（192d698，err_pending_chain
    链式载体——见 2026-11-23 节）；本条残差 = parse 期错误退化
    （架构级）不变，三探针 DIFF 形状逐字不变。
15. **oo2**（var-order **投入产出比**边界；auto_index/init 机制部分
    **[FIXED 2026-11-22b]** 2912c81）：r4 行的 info vars 物化序
    （tcl_order 投入产出比边界——**技术上可修**：按 tclsh 内部创建
    时序做 ghost insert 级重放，但每收口一行可能翻红别行、需无限
    细分逐键建模，**否决理由是成本而非不可修**，2026-11-16 轮既有
    定调）+ tclsh 的 auto_index 数组由 auto_load 机制在
    unknown→auto_load 首建（rtcl 探针路径未触发）。
    **[更新 2026-11-22b]** auto_index 首建已修（2912c81，桶位与
    tclsh 一致）；残差仅 r4 的 e/errorCode ghost-insert 序一行，见
    2026-11-22b 节。
16. **pidx**（expr 深水残差）：`expr $foo([x"y])` 下标替换路径的词中
    引号（词中 `"` 是普通字符）与报错归属——tclsh 报
    `invalid command name "x"y"`，rtcl 报 missing close-bracket。
    其余 pidx 面（pre-pass 主干）已对齐（pidx2/3/4 全绿）。
    **[FIXED 2026-11-22b]**（9d9e971，词首引号纪律——探针移回
    active）。
17. **probe7**（D-27【策略级】+ 有意扩展）：len(𝟘)=2 / string first
    = 2 是 tclsh 8.6 UCS-2 码元语义（D-27 策略级，rtcl 码点语义
    定调不改）；`string byterange` 是 rtcl 有意扩展
    （STRING_EXTRA_SUBCMDS，8.6 报 unknown subcommand）。
18. **trv_d13 / trv_p10**（ns delete 两阶段拆除）：tclsh 的 ns delete
    拆除分两阶段（exists=0 后 nukeQueue 延迟清理）——command delete
    trace 回调里 `namespace children ::` 仍含 :foo、var unset trace
    回调里未处理变量仍可读（trv_d13 的 `set ::ref::var2` = BBB）；
    rtcl 单阶段拆除。18.4 计数形状（已绿）与此不冲突：变量在 trace
    前已 detach。重开条件：nukeQueue 模型。**[FIXED 2026-11-22b]**
    （9ef6f88，ns_dying + var_nuke_pending 两阶段模型——探针移回
    active）。
19. **we5**（行号 defer 深水）：ns eval 体出口帧行（`script line N`）
    在词替换 defer 形态（err_from_subst 臂不动 err_line）读 stale 的
    最内层日志行（boom 的 1）；tclsh = 体相对行 2。精确修需
    err_pending_top 载荷增加单元相对行字段——ei_line_attribution
    钉形反向约束（其 defer 行为已绿），直接改 defer 行值会翻红。
    **[FIXED 2026-11-22b]**（378d980，载荷 (text, abs, unit_rel) +
    体相对折算 = 属主行 + 内容日志行 - 1；D 钉形同式保持绿——探针
    移回 active）。
20. **utf_U_parsehex**（D-27【策略级】）：surrogate U+D842 在 tclsh
    8.6 内部可存为单码元（format %x 回读 d842）；rtcl（Rust char）
    替换为 U+FFFD——码元 vs 码点的策略级定调不改。

## 2026-11-17 体帧行号 provenance 收口 + err_from_subst 跨错误泄漏（judge 88/88×2；battery 52/52；workspace tests 1420/0；fuzz seed7 400 例 5→0、seed0 500 例 0；probes 407/37 → 412/36）

M1' 验收轮（87c468e）fuzz 定谳的 5 条 preexist 分歧
（findings preexist_wasmi_f000{054,162,213,246,272}，全部 jit==tclsh 而
解释器路径行号不同）全数收口。

**根因**（tclBasic.c TclLogCommandInfo 钉死：errorLine = 1 +
换行计数(当前单元 script[0..失败命令起点)，每日志事件按最内层单元重
算；体出口帧 ("while"/"for"/"foreach" body line N) 读边界处 errorLine）：
tclsh 文件语境的循环体经迭代器分派 = **独立编译单元**，且词括号
`[...]` **内联编进体单元**——最内层日志恰在体单元层，errorLine 天然 =
体相对行。rtcl 的词括号失败在**新嵌套单元**记**括号相对**行
（entry_offset = line_ctx = 体首行，tclsh 文件语境 errorLine 同为内层
单元相对——k2 钉形 `catch {set x [expr …]} -errorline` = 1 双端一致），
体臂的 `n = err_line - (体首行 - 1)` 拿到的是污染值。

**修复**（vm_exec.rs unwind_subs）：体帧行基改用本次错误的**词括号
defer 行**（`err_pending_top` —— exec_inner 的 word-op defer 与 Sub 臂
同源：属主命令的绝对行），减体首行还原体相对行；无 defer（体内直接
失败，err_line 已是绝对行）回落 err_line。BodyFramed（foreach/lmap）
出口帧同臂。仅文件语境门控（in_file_script——proc 体编译语境的
err_line 已是单元绝对行，保持 e28c24a 口径）。jit 三 sink 的
`jit_err_set_line(fi.site)` 早已是同口径（属主站点行），本修复后
解释器与 jit 逐字节对齐。

- f000054（while 体括号 expr 失败 → body line 1，应 3）：最小形 =
  `set k 0; while {$k < 6} { incr k; set m [expr {~($x)}] }`。
- f000162（同形，体行 1 → 3）。
- f000213（while 内嵌 for 体括号失败 → line 3，应 5；for 体无帧 p20d
  不变）。
- f000246（foreach 体 → line 1，应 2；BodyFramed 臂）。
- f000272（for 体 → line 1，应 3）：行号面收口；**ARITH 首帧关键词残
  差另账**（见下）。

**err_from_subst 跨错误泄漏**（探针固化暴露的预存 bug，一并收口）：
unwind_subs 的 Sub 臂为更外层消费方置位 `err_from_subst`，错误终止于
catch/try 处理器时已无外层消费方——残留把**下一条**错误的首帧日志误判
为替换起点而吞帧（钉形：`catch {set m [expr {~($x)}]}` 后跟同形未捕获
错误，内层帧整体消失、只剩外圈命令帧）。修复：catch 消费点、try 两个
处理器清点、seed_parse_error、decode_toplevel_flow 四处并入一次性旗标
清除批。

**新固化 judge/probes**：loop_body_err_line{,2,3,4}.tcl（while/嵌套
for/foreach/for 四构形的 catch 无体帧钉段 + 尾段未捕获最小复现，tclsh
逐字 SAME）。

**残差记账**（本轮钉形实证、不修）：
- **ARITH 首帧关键词（嵌套语境）**：expr ARITH 类错误
  （`expr {1/0}`、`expr {true % "0x10"}`）在 while/for/foreach 体、
  proc 体、顶层 catch 体内，tclsh 首帧关键词 = `invoked from within`
  （errorInfo 先被裸消息置非空——空 append 即从 result 初始化，
  tclUtil/tclResult 机制），rtcl 双引擎一致报 `while executing`（C2 的
  双日志事件只在树走 catch+proc 矩阵装备；编译 EvalExpr 路径未装备）。
  顶层直排/顶层括号词形双端同字（while executing）✓。双引擎共有、与
  行号 provenance 正交；修正需钉死 8.6.17 的先行置位点并复验 378 探针
  EI 基线，缓议。
- **跨行词括号的体帧行**：`set m [\n    expr {~($x)}]`（属主命令行 ≠
  括号内最内层命令行）——tclsh 报括号内最内层命令的体相对行（内联至
  同单元），rtcl/jit 同报属主命令行（jit fi.site 同取属主）。双引擎
  同形残差，单行括号（含全部 fuzz 样本）无差。
- **多行 catch 体括号失败的 -errorline**：tclsh = 失败命令的 catch 体
  相对行（内联日志），rtcl = 括号相对行（1）——#15（uplevel/catch
  -errorline，2026-11-14/16b 记账「err_boundary_line 基建已落、修正需
  动 e28c24a 行号单元化深水区」）的同族新实例，缓议。

## 2026-11-16c incr 两预存分歧收口（性能轮 Agent 6 入账 → 本轮修复；judge 88/88；probes 380/35 + incr_widen 固化）

1. **incr 坏旧值/坏增量的 errorCode**：rtcl 报 `expected integer but
   got "..."` 文本正确但 errorCode 为空；tclsh 为 **TCL VALUE
   INTEGER**（两处——读现值路径与 `(reading increment)` 增量解析
   路径）。修复：set_error_code + Error::Msg（misc.rs cmd_incr）。
2. **incr i64 溢出回绕**：tclsh 把溢出和提升为精确 bignum（i64::MAX
   incr → 2^63；i64::MIN incr -1 → -2^63-1），rtcl 此前 wrapping
   回绕。修复：checked_add 失败走 types/bignum BigInt 路径（expr 的
   numeric_binop 同约定）。

固化 judge/probes/incr_widen.tcl（六断言：坏旧值/坏增量/上溢/下溢/
正常面/bothbad——tclsh 逐字 SAME）。

## 2026-11-16b 组 6/7 小尾巴 + 两轮移交残差收口（judge 88/88；probes 378/36——基线 373/41，翻绿 bn13/pbr3/probe1/ivars2/tclns；workspace tests 全绿）

HANDOFF §10.1 组 4（dict 残差）+ 组 5（小尾巴）+ 2026-11-14 节 5 条 EI 相邻
残差 + var-order 组移交的 oo2.r4，逐例三选一定调：

**A. dict 残差**
1. `dict info`【已收口（对齐）】：tclhash 重放模型下 12/38 项（含桶重建
   边界）与 tclsh 逐字节一致（db 差分实证），哈希轮已收口，无需定调
   设计差异。
2. `dict getdef`【已收口（对齐=8.6 无此命令）】：rtcl 已随哈希轮删除
   8.7 前瞻特性，`unknown or ambiguous subcommand "getdef"` 与 tclsh
   同文；errorcode 缺失随本轮 d1 修复补齐。
3. D-12（dict get 奇数长）【已收口】：复现脚本双端同错同 EC。
4. D-13（dict set a(1)）【已收口】：8.6.17 实测双端均创建数组元素，
   台账原记 tclsh 报错已过时（复核定调）。
5. D-14【本轮收口】：incr 现值非整数 → EC 补 `TCL VALUE INTEGER`；
   lappend 现值坏列表 → 由静默重置改为 `unmatched open brace in list`
   + `TCL VALUE LIST BRACE` 且 dict 不动（as_list_strict 路径）。
6. D-15/D-16/D-17 主体【已收口】：本轮复核一致；d17 的 unknown 子命令
   EC 缺失随本轮补 `TCL LOOKUP SUBCOMMAND <name>`（d17b/gd1）。

**B. 小尾巴**
7. expr 词法 `<<<`/`>>>`【对齐】：移除 rtcl rotate 扩展（jimtcl 血统，
   未列入有意扩展清单）——tclsh 词法把 `1 <<< 2` 解析为 `<<` 后随非法
   操作数（missing operand at _@_，标记紧跟 `<<`）；bn13 全绿。nan/inf
   接受面哈希轮已收口（本轮无残差）。
8. representation 文案【对齐+归一】：截断从 40 改 16 字符 + "..."
   （tclsh Tcl_AppendLimitedToObj(limit=16)）；refcount/object pointer
   为实现细节不可复现，bnprobe2 探针侧归一化该两字段后全绿。
9. 大整数结果规范化【对齐】：expr 结果为 bignum 范围整数时按
   Tcl_SetBignumObj 语义重渲染十进制（`expr {$x}` x=0x10…00 →
   18446744073709551616；pbr3 全绿）；十进制原文 E4 不降级语义保留。
10. disassemble/assemble 面【部分对齐 + 设计差异定调】：
    * `assemble`【对齐】：over（INST_OVER 立即数复制）、undefined
      label 文案、duplicate label 检查、异常语境一致性检查
      （ProcessCatchesInBasicBlock 的指令级镜像，TCL ASSEM BADCATCH）
      全落地——cmp_assemble 全绿（含 getbytecode 装配子集 dict）。
    * `getbytecode script` 对 `tcl::unsupported::assemble {push…}`
      字面命令按 tclsh **编译期装配**复现 dict schema（literals/
      instructions/commands/scriptto 含尾下标 34 钉形）。
    * `disassemble`/`getbytecode` 成功清单（c5_dis/cmp_unsupported/2）
      【设计差异】：tclsh 输出嵌 raw 指针/epoch 且指令表是 tclsh 自身
      字节码——rtcl 自有 VM，清单形态为实现细节（校验/错误面已对齐，
      eval.rs 注记）；两探针保留 DIFF 作诚实标记。
    * dbg_basic【非分歧（oracle 崩溃）】：tclsh 8.6.17 在 basic-15.3
      （namespace delete current 后再 proc）当场 SEGV（rc=139、单进程
      复现），无有效 oracle；rtcl 语义（ns 存续=0）与 tclsh 崩溃前
      可见行为无对照可言。
    * dbg_trace【探针提取残缺 + 残差】：提取器丢了 traceExecute 回调
      proc，tclsh 中途 abort（rc=1）——另实测 rtcl 变量 write trace
      回调错误缺 `(write trace on "vv")` 退出帧、enter/enterstep/leave
      回调错误归因链不同（tclsh 记回调命令链 + trace-on 帧）——记
      OPEN 残差（B4，见下）。
11. trace add 执行点【残差记账】：unset trace 回调错误吞掉已对齐；
    write trace 回调错误的首帧已对齐；缺 `(write/read/unset trace on
    "<name>")` 与 `(enter/leave/enterstep trace on "<cmd>")` 退出帧、
    enter/enterstep/leave 回调错误的归因链（tclsh 记 `error eerr /
    while executing "error eerr" / (procedure "bad" line 1) / invoked
    from within "bad {foo q} enter" / (enter trace on "foo q")"`，rtcl
    归因到 tracee 单帧）——trv_* 组基线不动的前提下未动 trace 帧机，
    记 OPEN。
12. `interp alias {}`（D-1）【对齐收口】：`interp alias {slavePath}
    slaveCmd ?{masterPath} targetCmd ?arg...??` 落地——{} 路径即主解释
    器，创建形转投别名表、查询形返回 {target ?prefix...?}；非空路径
    报 `could not find interpreter`（smallface_p24 al1-al6）。
13. `::tcl::prefix`【对齐落地】：ensemble 面 all/longest/match
    （-error/-exact/-message、ambiguous/bad option 文案、TCL LOOKUP
    INDEX/SUBCOMMAND、TCL OPERATION NOARG）+ `::tcl::Bgerror`
    （元数/missing return option/-errorcode/-errorinfo 副作用/bgerror
    调用，无 handler 时静默）+ `::tcl::CopyDirectory`（init.tcl 逐行
    移植为 stdlib proc）——probe1 `::tcl::*` 计数 4 对齐（-f 模式
    ::tcl::history 同随 stdin 语境消失）。
14. `::tcl::pkgconfig`【已收口（哈希轮）】：本轮复核 list/get/bad
    subcommand 面一致；get 值为安装配置面（各自构建的路径值），非语义
    分歧。

**C. 2026-11-14 移交的 5 条 EI 相邻残差**
15. uplevel 体错误 -errorline【残差保留（机制钉死）】：tclsh 的
    errorLine 在外围命令日志事件处**即时**更新，-errorline = 体单元内
    最外层失败命令的行（U1-U4/c1c-c1h/rdm 去 scrub 矩阵实证；动态体
    catch 读 catch 命令自身行）。rtcl 的 err_boundary_line 捕获基建已
    落（dispatch_values 出错时记末次失败命令行），但行号换基撞上
    line_ctx 单元化（e28c24a 剥体首空行 vs tclsh 计头行）——修正需动
    2026-11-14 基线深水区，本轮保留原口径，return_decode_matrix 继续
    scrub 该键，残差缩窄为「多行 proc harness 差 1」。
16. ARITH 首帧关键词【对齐】：expr ARITH 运行期错误落 tclsh 双日志
    事件模型（表达式机先记裸消息，expr 命令帧以 `invoked from within`
    追加；INNER 由 err_force_inner 在 (Some,false) 臂消费）——
    `expr {1/0}` 全矩阵同字。
17. expr 解析错误 EC/INNER【对齐】：-errorcode = TCL PARSE EXPR
    <CLASS>（MISSING/UNBALANCED/BADCHAR/BAREWORD 分类落地）；INNER
    的 syntax 形 -errorinfo 载荷 = 抬升点预装 err_info（带 parsing
    frame）、-errorcode = 抬升码——陈旧 INNER 泄漏同根修复。
18. `dict get` INNER【对齐】：缺键错误装 INNER `dictGet`（eval 形同，
    twokeys/odd 矩阵：odd 走 invokeReplace）。
19. `error msg info code` 三参形【对齐】：INNER = `returnImm <msg>
    <options 列表>`（{-errorinfo {} -errorcode <code>} 按给定序；
    2 参 `returnImm msg {}`；空 info 3 参 `{-errorinfo {}}`）；非空
    info 形不装 INNER（errorstack 陈旧泄漏与 tclsh 同形，FORM 矩阵
    实测）。

**D. var-order（oo2.r4 移交）**
20. catch 结果变量与 ::errorCode/::errorInfo 物化序【对齐】：catch 的
    全局创建序改为 e → errorCode → errorInfo（tclsh 实测序；不动
    catch/EI 语义，仅 set_var 换序）——ivars2/tclns 翻绿。
21. ::auto_index 惰性首建【对齐】：启动期建表移除；首个 unknown 命令
    入口（dispatch 的 unknown 分派前）落表——tclsh 文件模式启动不含
    auto_index、unknown→auto_load 首次运行创建（ai4/ai5/oo2 l7 二分
    实证：oo2 的触发点 = `fw g` forward 到 nosuchcmd 走 unknown）。
22. oo2 r4 残差【部分收口】：info vars 序剩 2 位置差（tcl_rcFileName
    桶位 + e/auto_path 转置）——tclsh 全局表序含「最终活键重放」
    表不出的插入历史（ghost insert/unset 级；**技术上可修**——按
    创建时序逐键 ghost-insert 重放，否决理由 = 每收口一行可能翻红
    别行、无限细分的投入产出比），tcl_order 模型（d7cc38e）
    + 上述两项修复已收 ivars2/tclns，oo2 r4 单行残差如实记账，不追。

**新固化**：judge/probes/smallface_p24.tcl（40+ 断言：prefix 12 形/
Bgerror 4 形/CopyDirectory 存在性/pkgconfig 面/dict EC/error 三参 INNER
矩阵/expr 解析 EC 分类/ARITH 首帧/assemble over-label-语境/getbytecode
dict/alias 6 形）+ bnprobe2 repr 行归一化。probe 数字：373/41 →
378/36（objquad 偶发超时抖动 ±1，与本轮无关）。

## 2026-11-16 OO 可见性组收口（judge 88/88；probes 373 pass / 41 fail（含新固化 oo21/oo22）；order.probes 48/48；workspace tests 1420/0）

HANDOFF §10.1 组 3（OO 可见性面，~10 例）全数定调入账。基线探针
362/50 → 371/41：oo1、oo4 本组收口，**连带修复 order 组 7 个挂账**
（cmds/dict_p40/ic/ic2/info2/p40/topcmds——根因同为 history 命令列名）。
新固化探针 judge/probes/oo21.tcl（methodtype/forward/ctor-dtor/Slot 面）
+ oo22.tcl（对象 ns 可见性/内部命令直调/mention/nextto/filter 拦截/
children 序/history 门控）。

1. **::oo bootstrap 形状**（oo1 a5、oo4 x2/x3）[FIXED 2026-11-16]
   - `info commands ::oo::*` 补齐 ::oo::UnknownDefinition / ::oo::InfoClass
     / ::oo::InfoObject（9 命令，插入序 = UnknownDefinition class
     objdefine InfoClass Slot object define copy InfoObject，tclhash 反推）。
   - ::oo 子命名空间补齐 define/objdefine/Helpers/InfoClass/InfoObject +
     Obj1..Obj10（核心对象 ns；Obj4..10 = 7 个 slot 实例
     ::oo::define::filter/mixin/superclass/variable + ::oo::objdefine::
     filter/mixin/variable，皆 ::oo::Slot 实例）。children 插入序
     = Obj7 Obj6 define Obj10 Helpers Obj9 Obj2 InfoObject InfoClass
     Obj5 Obj1 Obj3 Obj8 objdefine Obj4——用 tclhash 反推机对 **fresh-15
     与 oo4-x2-33 双态**同时求解验证（单态解会被后续重建错位）。
   - **坑（本轮最大）**：`ns_children` 的重放键此前按**全名**哈希，
     tclsh 子表按**尾名**哈希——15 键状态巧合并无害，33 键桶位漂移即
     显形（oo4 x2）；已改尾名重放（info commands 同款出口）。
   - 失败的 `oo::copy` **泄漏目标 ns**（oo4 语境 ::oo::Obj21 活体）并烧
     ObjN 号——rtcl 已同行为，编号链（Obj11..Obj30，Obj28/30 销毁）双态
     逐字一致。
2. **元类实例表**（oo1 a13）[FIXED]：`info class instances oo::class` =
   `::oo::object ::oo::class ::oo::Slot` + 用户类（creation 序——类对象
   的元类归属入列）；oo::Slot instances = 7 slot 对象。
3. **对象命名空间可见性**（oo4 p4/p9/p12/p13）[FIXED]：对象 ns 只露
   `my`（oo4 oracle）；self/next/nextto 以**隐藏命令**注册（列表面不
   可见），`namespace which` 按归属重映射：对象 ns 内裸名 →
   `::oo::Helpers::self/next`，显式限定名 `::oo::ObjN::self` → 空
   （tclsh 无此命令）。::oo::Helpers ns 落地（self/next/nextto，
   **nextto 新实现**——`nextto <class>` 以指定类实现为下一站、显式实参
   调用，元数文案分形：0 参 `nextto class ?arg...?`、≥2 参
   `nextto <给定类名>`）。host bridge 等 hidden 命令的 which **不**受
   影响（http.tcl::_require_bridge 回归风险点，已钉 http_tcl 6/6）。
4. **methodtype/definition 自有作用域** [FIXED]：`info class/object
   methodtype` 只查**自有**方法表——继承一律 `unknown method "X"` 错误
   （mt4/q1/x6）；自有核心方法 → `core method: "destroy"/"create"` 形
   （mt8/mt9）。oo::Slot 面收口：自有 -clear/-append/-set（导出、
   methodtype=method）、`::oo::Slot -clear` 仍是对象链 must-be
   （create/destroy/new——类对象派发不吃自有方法表）。
5. **info class/object forward** [FIXED]：新子命令；2 参形返回 forward
   目标词（fw2 `::set k` / fw5 `::set q`）；1 参元数错误（fw1/fw4，
   usage 词 = 调用名）；miss → `unknown method` 错误。
6. **constructor/destructor 实体**（D-43⑦ 修正）[FIXED]：tclsh 8.6.17
   对已定义者**返回实体**——ctor = `{参数 体}`（ct1 =
   `{a {b 2}} {set v $a}`，逐参渲染保默认值）、dtor = **体字符串**
   （ct2 = `my cleanup`）；未定义为空。D-43 原"恒空"系对核心类误测，
   本轮以 oracle 推翻。
7. **deletemethod/renamemethod 自有限定 + 提及名单**（oo7 d15）[FIXED]：
   对继承名/未提及未知名 → `method X does not exist`（p12/p13/u5/u6/
   n3/n4）；export/unexport **提及过**的未知名（tclsh 的 exportedMes/
   hiddenMes 记账）→ deletemethod/renamemethod **静默消费**（oo7 d15/
   u2/n2）。rtcl 新增 mentioned 记账字段。
8. **filter 拦截未导出/hidden 调用**（cp4/uf2/g5-g7）[FIXED]：filter
   名按**私有视图**解析——filter 方法自身被 unexport（oo7 g2/g3）或
   对象级 hidden（p1）后拦截照常；对象级 hidden 方法调用不再提前报错
   （filter 先行，用尽才落 unknown——D-41② 的未覆盖面）。
9. **history 仅 stdin 注册**（oo4 p8）[FIXED]：tclsh **文件参数模式无
   `history` 命令**（tclMain 的 RecordAndEvalObj `history add` 自动装载
   只在 stdin/交互运行），-c 同（stock tclsh 无 -c；specs/probe.sh 的
   oracle 是 `tclsh <<<`，语境 = stdin）。rtcl -f 时移除
   （`Interp::drop_stdin_history`，::tcl::history 实现保留——tclsh 同），
   -c/REPL 保留。**连带修复**：cmds/dict_p40/ic/ic2/info2/p40/topcmds。
10. **内部命令直调面** [FIXED]：::oo::InfoClass/::oo::InfoObject 本体与
    13/12 个 ::oo::InfoClass::X / ::oo::InfoObject::X 子命令 native
    （info procs 空——tclsh 全为 native）；usage 词 = **调用名**（di8：
    `::oo::InfoClass::forward` 不重复 forward 尾）；::oo::UnknownDefinition
    0 参 `bad call of unknown handler`、带参语境错误（ud/ud2）；define
    词 native 化后直调**元数先、语境后**（d1/d3/d5/d6 vs d7），slot 词
    （variable/filter/mixin/superclass + objdefine 三槽）语境外一律
    语境错误（sd/so/sc 批）；定义**脚本**内槽字经 eval 派发的 define
    语境门遮蔽全局内建 `variable`（旧 proc 遮蔽同效，多词形帧抑制
    分态保留——脚本形不抑制、多词形抑制，oo8 回归教训）。
11. **isa typeof usage**：`info object isa typeof objName className`
    （原 "objName arg"）。
12. **方法列表序**：plain（无 -all）= 自有方法表 **Tcl 哈希序**
    （sl1 = `-clear -append -set`，非排序非定义序）；-all = 排序 +
    **oo::Slot 为链截断点**（穿 Slot 的继承不计 oo::object 方法——
    Slot -all 无 destroy、Y(super oo::Slot) 同、Z(super oo::object) 有）。

**遗留/移交定调**：
- **oo2 r4**（catch 结果变量与 ::errorCode/::errorInfo 全局物化序 +
  ::auto_index 首建时机）：tclsh 全局创建序 = e → errorCode → errorInfo
  且 auto_index 惰性首建；rtcl 为抬升点记账 + 启动期建表。根因属
  var materialization/哈希序组（2026-11-08 清单第 11 类），**不属 OO
  可见性**，不动 catch/errorInfo 基线（2026-11-14 组语义）。oo2 全文件
  其余标签已逐字一致。
- **::oo::ObjN 编号已对齐**（D-43 原记"外观差异"由本轮 x2 钉形推翻为
  可观察面——编号链含失败 create 的烧号与 oo::copy 泄漏，双态一致）。
- we5/q114b/p-uplevel-ns/tclns/probe1/u72/namespace_varmodel2/trv_* 为
  var/trace 组基线既有失败，本轮未触碰（基线二进制逐探针对照确认）。

## 2026-11-15 IO/通道组收口（judge 88/88；probes 362 pass / 50 fail——fail 均他组；workspace tests 1420/0）

§10.1 组 5（2026-11-08 清单第 5 类 13 例）全部定调：

**已收口**（逐条 tclsh 差分实证，前 agent 11 提交 + 本轮 3 提交）：

1. **通道编号/复用**：fileN 最小空闲号复用、chan names 表键哈希序
   （4ef732c）——差分全 SAME（ids/close-reuse/names 序）。
2. **read 计数**：字符计数（UTF-8 多字节边界逐字符/tell 字节位）、
   binary 字节读、`-nonewline`、全读——全 SAME。
3. **fblocked 元数**：阻塞读永不 blocked、arity 文案 + EC
   TCL WRONGARGS、不存在通道 EC TCL LOOKUP CHANNEL——全 SAME。
4. **chan copy -size/-command**：字节快路（751e071）+ -size 半量/
   -size 0/tell-after/回读内容/同步 -command 形——全 SAME。
5. **eofchar**：读侧 \x1a 截断置 EOF（gets/read 两形）、写侧 close
   一次写出（68690a03 = "hi\n\x03"）、查询形状 ro `{}`/wo `{}`/rw
   `{{} {}}`、设/清两形——全 SAME。
6. **socket -async** [FIXED 2026-11-15，原 2026-11-12 记 OPEN]：
   **socket 先建、连接后台化、完成惰性**——`socket -async` 经 libc
   socket()+非阻塞 connect()（EINPROGRESS）即建通道返句柄；
   `-sockname` 立即可查（socket 建出即绑临时端口，tclsh 同）；
   首次真实 I/O（flush 带缓冲/读/`-peername`）poll(POLLOUT)+
   SO_ERROR 补齐（`-blocking 1` 等连接完成语义）；补齐成功即外送
   connecting 期缓冲（tclsh 连接完成事件自动 flush）；失败置终态，
   后续 flush/gets/read/peername 一律
   `socket is not connected`（EC POSIX ENOTCONN，不保留原始拒因——
   tclsh 实测同形）且 close 仍成功；第二次 flush 空缓冲 rc=0
   （失败 flush 弃缓冲）。puts 连接期只缓冲不尾随 flush
   （`-buffering full` 用户态缓冲形）。
7. **socket fconfigure 面**（新落地）：`-peername`/`-sockname` =
   {数字地址 反解名 端口}（127.0.0.1 → localhost，getnameinfo
   flags=0 + NI_NUMERICHOST 两调）；`-connecting` 查询以 poll(100ms)
   有界探测近似 tclsh 的后台推进（回环实测 tclsh 立即为 0）；全查询
   尾部追加 `-peername {…} -sockname {…}`，取不到的选项跳过不报错
   （实测死路全查询只出 -sockname）；bad-option 文案 socket 表
   9 项（…-translation, -connecting, -peername, or -sockname），
   set 面只认基础 6 项（只读项 set 报基础表）。
8. **socket 数据面默认** [FIXED 2026-11-15]：`-translation` 查询
   `auto crlf`、写侧 \n 落线即 \r\n（binary 回读 hex 41420d0a43 双
   解释器同）——原 rtcl 用文件的平台 lf。
9. **ECONNRESET 归 EOF** [FIXED 2026-11-15]：server 收数据不读即
   close（RST），client read 得空串 + eof=1，不抛错（tclsh 同）。
10. **端口形** [FIXED 2026-11-15]（TclSockGetPort 形，实测差分）：
    整数 >65535 → `couldn't open socket: port number too high`
    （EC NONE）；整数 <0 → `couldn't open socket: Servname not
    supported for ai_socktype`（EC NONE）；非整数先 getservbyname
    （"http"→80，-async 同）未命中保留 GetInt 错误
    `expected integer but got "<raw>"`（无 couldn't 前缀）+ EC
    TCL VALUE INTEGER；DNS 失败 → `couldn't open socket: Name or
    service not known`（剥 Rust lookup 包装文案，保 gai_strerror
    原大小写）。
11. **通道 IO 错误面** [FIXED 2026-11-15]：gets/read/flush/puts 的
    真实 io 错误统一 `error reading/writing/flushing "<chan>":
    <strerror>` + EC POSIX 随 errno（errno_text 补 ECONNRESET/
    ENOTCONN/ETIMEDOUT/ECONNREFUSED/EADDRINUSE）——async 死路
    gets 报 `error reading "sockX": socket is not connected`
    与 tclsh 同字。

**记 OPEN（设计差异，非缺陷）**：

- **socket 通道名**：rtcl `sockN`（创建序）vs tclsh 指针名
  （`sock646889354370`）——不可复现，探针归一化（quoted/裸名 +
  `(?![a-z])` 防 "socket" 误吃）。判定探针 chan_io_p23 内置归一化。
- **socket -server**：rtcl 不支持（TCP 客户端 only）——伺服面/事件
  泵架构边界（wasm net feature 亦不含）。回环差分 harness 以 tclsh
  当 server（specs/scripts/probe_sock_peer.sh、probe_sock_async_cmp.sh、
  probe_sock_data.sh）。
- **-async 无事件循环后台连接**：tclsh 的 async 连接由后台事件推进
  （connect 完成即自动 flush）；rtcl 惰性补齐 + `-connecting` 有界
  探测近似。慢网真实连接下 puts-without-flush 的数据要等下一次
  flush/读/close 才外送——loopback 全窗一致，记行为边界。
- **非 unix（net 特性可用的非 unix 平台）**：`-async` 回落 std 阻塞
  连接（无 libc 非阻塞 connect 路径）；原生 linux 为准。

**新固化**：judge/probes/chan_io_p23.tcl（28 断言：端口形矩阵/
async 死路终态/fconfigure socket 选项表/全查询跳过形——self-contained
死端口 0/1，无 server 依赖）；specs/scripts/probe_sock_peer.sh、
probe_sock_async_cmp.sh、probe_sock_data.sh（live 回环差分，24+3 断
言）、probe_io_residual.sh（文件面残余差分 24 断言，全 SAME）。

## 2026-11-14 errorInfo/errorstack/return 帧组收口（judge 88/88；probes 340 → 349）

§10.1 组 1（语义域最深一组）收口，6 个 commit：

1. **90f4ad6 return 完成码解码**：TclMergeReturnOptions 的 `-code return`
   换形（≡ `-code ok -level+1`）+ `-level 0` 物化原码（TclProcessReturn
   level 0 直接返回码；break/continue 值随完成走）+ 原码穿透边界不解码。
   `return -code return` 在 foreach/for/while 体不再被循环边界误转 OK。
2. **ccbb24d catch options 键序**：根因 = tclsh GetReturnOptions 的
   returnOpts 键位模型——ec-first 当且仅当 return 式抬升带 -errorcode
   载荷（invalid command 实为 unknown proc 的
   `return -code error -errorcode {...}`，init.tcl:275）；error 命令的
   code 经 SetErrorCode 不进 returnOpts。error 命令抬升不再携带对象
   载荷，ControlFlow.error_code 专属 return 式抬升。
3. **e28c24a errorLine 归属**：err_pending_top 带 (text, line)，defer
   不再覆写 err_line（构造出口帧读最内层已记日志命令的行）；Compiler
   拆 line_ctx/line_base——proc 体语境括号内容行号单元化
   （tclsh 编译单元行表），文件语境保持括号相对。
4. **626ea6a -f 文件执行语境帧**：in_file_script（TclEvalEx 逐命令语义
   判别，run_file/cmd_source 武装、eval_impl 逐单元消费、call_proc 体
   截断）——文件级 if/while/for 字面体错误挂体出口帧 + invoker 帧
   （体相对行、最外层体限定）；运行时 expr 括号行重定基改
   cur_cmd_line_abs（单 Cmd token 表达式的运行时求值器路径）。
5. **8055dd1 uplevel ns-eval 帧保活**：目标为 namespace eval 帧时
   ns_level0/ns_stack 条目保留（tclsh 目标 varFrame 不销毁）——
   info level 计层、info level 0 报 ns eval 调用词、跨 ns 帧 uplevel 2
   层级算术（p7 D 形 bad level）。
6. **3ab5a1f procedure 帧名 = 调用词**：TclNRInterpProcCore(interp,
   objv[0])——帧 level0[0] 而非规范全名。

收口探针（原 2026-11-08 清单第 2 类 9 例）：`ei2`、
`expr_bracket_multiline`、`loops_return_level0_loop_bodies`、
`namespace_uplevel`、`namespace_uplevel2`、`probe_catch`、
`return_level0_codes`、`sub_inline_errors1`、`sub_inline_if_body` 全绿。
新固化 judge 探针：`return_decode_matrix`（return 解码 + options 键序，
-errorline 残差 scrub）、`return_flow_matrix`（循环体 return 传播）、
`uplevel_ns_level`（level 计数矩阵）、`file_ctx_frames`（文件语境帧
矩阵 + proc 体/eval/ns 对照）、`ei_line_attribution`（行号归属 +
procedure 帧名矩阵）。

**新发现残差**（相邻面）——【2026-11-16b 全数定调，见 2026-11-16b
节 15-19：ARITH 首帧/expr 解析 EC+INNER/dictGet INNER/error 三参形
对齐；uplevel -errorline 基建落地、残差缩窄为多行 proc harness 差 1】：
- uplevel 体错误的 -errorline：tclsh 在外围命令日志事件处更新
  errorLine（即时），rtcl 的 defer 帧到顶层 append 才写——uplevel 体
  错误被 catch 时差 1（t2 形钉探针在 return_decode_matrix 中 scrub）。
- expr 1/0（ARITH 类）被 catch 后的 -errorinfo 首帧关键词：
  tclsh `invoked from within`（内部双日志事件，首帧无命令可记）vs
  rtcl `while executing`（单事件）。
- `catch {expr {1 +}}`（expr **解析**错误）：-errorcode 应为
  TCL PARSE EXPR MISSING 且 -errorstack 带 INNER syntax 特形；rtcl 报
  NONE 且 INNER 陈旧（前一个错误的 INNER 泄漏——expr 解析错误抬升
  点未装 INNER/errorCode）。
- `dict get {a} b` 的 INNER：tclsh `dictGet`（dict 内部形）vs rtcl
  `invokeReplace`。
- `error msg info code` 三参形：tclsh 的 errorInfo 首帧 =
  `"error boom "" {A B} "`（源文本含尾空格）且 INNER returnImm 带显式
  options 列表 `{-errorinfo {} -errorcode {A B}}`、catch options 键序
  -errorinfo/-errorcode 在前（returnOpts 键位）；rtcl 三处均未对齐。

## 2026-11-07 对齐盘点（judge 88/88；probes 300 pass / 97 fail）

本轮收口（commit 2f2e520）：
- `string` 子命令改为 tclsh 的唯一前缀解析；未知名按
  `unknown or ambiguous subcommand ...` + `TCL LOOKUP SUBCOMMAND`；
  各子命令的最小/最大元数与 usage 文案对齐（含 `string cat` 零参、
  `length/bytelength/reverse` 精确一元、`trim` 最多一元）。
- `string compare/equal` 的 `-nocase`/`-length` 按 tclsh 的 argv 窗口
  解析：恰两词时不解析选项（`string compare -nocase a` 比较字面量），
  更多词时选项区逐词校验（`bad option "a"`、`--` 不接受、负 length
  视为全文比较）；`map`/`match`/`is` 同样按词窗解析。
- `string is` 全 21 类（补 `entier/false/true/wordchar`）与 tclsh 的
  类名前缀/`ambiguous class`/`bad class` 文案；`-strict` 空串语义
  （`list` 例外）、`-failindex` 失败字符下标（整数越界 -1、double
  失败位置、boolean 0）；Tcl 整数（u32/u64/任意精度三档）与
  double（Inf/NaN/0b/0o/0x/十进制指数）语法扫描。
- `string tolower/toupper/totitle string ?first? ?last?` 区间语义。
- 初始化全局：`tcl_patchLevel` 8.6.17、`tcl_rcFileName`、`tcl_pkgPath`；
  `errorInfo` 延迟到首次错误；`info library`；`info vars/globals/locals`
  不再枚举数组元素（`set a(1) x` 只报 `a`）。
- 裸 `$` 变量名只吞 ASCII 字母数字/下划线：`$变量` 是字面量，
  `${变量}` 仍是变量引用（parser 与 tclsh 8.6 一致）。
- `update` 元数、`pid` usage 文案。

剩余 97 例按根因分组（括号内为该类探针数）：
1. **增量求值 vs 整脚本预解析（15）**：`globprobe`、`ix1/2/4`、
   `cases/cases2`、`pidx/2/3/4`、`t2/t3/t4`、`probe_round3`、`r3probe`。
   tclsh 边解析边执行，后续语法错误前已有输出；rtcl 先解析整文件。
   需把 eval 改成流式（或至少顶层按命令边界增量解析/执行）。
2. **errorInfo/errorstack/return 帧归属（19）**：`ei2`、
   `expr_bracket_*`、`h_break_proc_boundary`、`h_info_call_chain`、
   `h_return_options`、`h_teardown_trace_line`、
   `loops_return_level0_loop_bodies`、`p25/p29/p31`、`probe_catch`、
   `return_level0_codes`、`sub_inline_*`、`namespace_uplevel*`。
   涉及内联体帧（if/while/for）、dict with/filter 帧、return -code/
   -level 解码与 catch options 的帧记录顺序。
3. **哈希/迭代序与命令面（11）**：`array_names_regexp_backref`、
   `cmds`、`topcmds`、`ic/ic2`、`info2`、`tclns`、`p-uplevel-ns`、
   `u72`、`trv_p8`、`namespace_varmodel2`。Tcl 用自研哈希表插入序，
   rtcl 多为排序或 Rust HashMap（随机种子，跨进程抖动）；且 rtcl
   命令集是 jimtcl 超集、缺 `::tcl::prefix`/`::tcl::pkgconfig` 等。
4. **OO（10）**：`oo1`–`oo9`、`oo_dbg1`。namespace children 序、
   方法可见性/self/next、`must be ... or ...` 连接词、methodtype、
   objdefine 的 usage 上下文。
5. **IO/通道（13）**：`chan_io_p2/3/5/9/10/11/13/13b/15/17/18/19/20`。
   通道编号/复用（fileN 计数、chan names 序）、`read` 计数/
   `fblocked` 元数、`copy` 方向/选项、`eofchar`、socket、只读/只写
   方向错误、`-size/-command` 选项窗口。
   **[FIXED 2026-11-15]** 全组收口（11 提交 90f4ad6 前：4ef732c/713773f/
   f2208f8/751e071 + 本轮 3 提交）——逐例定调见 2026-11-15 节；
   残余 OPEN 仅 socket 命名/-server（设计差异）。
6. **dict 命令面（6）**：`dict_p35/p40/p42`、`p35/p40/p42`。
   `::tcl::dict::*` 集成命令缺失（`info commands ::tcl::dict::*` 空）、
   dict 子命令 errorCode/`dict info` 形态。
7. **Unicode/8.6 UCS-2 语义（5）**：`probe4`、`probe6`、`probe7`、
   `utf_char_classes`、`utf_U_parsehex`。`string length/index` 对增补
   平面按码元计 2、`\U` 代理对（d842）、Unicode 类表版本差异、正则
   `[[:class:]]` 对若干类报 invalid。
8. **表达式词法（3）**：`bn7`、`bn8`、`bn13`。`nan()`/`inf()` 函数
   识别、浮点 `<<`/`>>` 的操作数错误位置与消息。
9. **大整数格式化/对象表示（2）**：`bnprobe2`、`pbr3`。`0x` 前缀与
   十进制混排、refcount/指针/截断的 `tcl::unsupported::representation`
   文案。
10. **反汇编/字节码调试面（6）**：`c5_dis`、`cmp_assemble`、
    `cmp_unsupported/2`、`dbg_basic`、`dbg_trace`。`disassemble` 头/
    指令名/源码行、`::tcl::unsupported::assemble` 指令集与栈检查、
    `debug`/`trace` 执行步。
11. **namespace/变量/trace 模型（6）**：`vm3`（`variable` 无值后
    unset 回退全局）、`namespace_varmodel`、`trv_d13`、`trv_p10`、
    `trv_p11`、`trace_array_get_firing`（read trace 触发时机/顺序）。
12. **命令集计数（1）**：`probe1` 的 `info commands` 计数因超集
    命令与 `::tcl::*` 缺失而不同（与第 3 类同根）。

非缺陷、不再修：`string byterange` 是 rtcl 扩展，tclsh 无此子命令。

## 2026-11-19 clock+package 外置（wasm 剥离 → JS dynmod 补回，K/P 区 21 例真机）

wasm 构建剥离 clock/package feature（净重 85KiB raw；perf 轨 pkg
−73,545 / size 轨 pkg −56,359）；命令面经 dynmod 注入
www/clockpkgmod.js 同面补回（`::clock`/`::package` wrapper + clockpkg
命名空间，计数面 99=99 逐字不变）。行为矩阵自 native clock.rs
（D-24/D-25 收口态）逐条移植：format 规定符全集 / scan -format
（strptime 面）/ 自由 scan 怪值收口 / add 顺序单位与月末夹取 /
儒略-格里历切换 / 错误消息+errorCode 同文（__CLKERR 哨兵列表 +
wrapper `return -code error -errorcode` 解包——JS 派发前缀不外泄）。
118 例 node 差分（期望值 = rtcl release 探针钉定）全过。

**wasm 构建特有升级（相对剥离前的 UTC-only stub，非分歧）**：
clock format/scan/add 默认渲染 = 宿主本地时区（Date 偏移，DST 由引擎
tzdata 承担——native unix 面 libc localtime_r 同源数据）；`-timezone`
支持 IANA 名（Intl.DateTimeFormat 校验/求值——剥离前 wasm 恒报 not
found）。tclsh 对齐度不降反升（如 `-timezone Asia/Shanghai` 的 %z）。

**JS 面语义定调（wasm 构建特有分歧）**：
- `%Z` 缩写：-gmt 恒 "GMT"；否则 Intl short 名（"UTC"/"GMT+8" 风格）
  ——与 libc/tclsh 的缩写表（"CST" 式）不同文；%z（+HHMM 数字偏移）
  两面同值。
- `package require` 的 pkgIndex 扫描在 wrapper proc（Tcl 面）编排
  （JS 命令回调无法重入解释器 source）；registry 状态在 JS 模块。
  `package unknown`/`prefer`/`ifneeded`/`present`/`versions` 等 tclsh
  子命令不在 native 面也不在 JS 面（native package.rs 六子命令同形）。
- DST 消解：fold（秋拨双解）取转换前偏移（DST 支；tclsh 同）；gap
  （春拨空档）落转换后墙钟（glibc mktime isdst=-1 同形）。native 的
  with_zone 在同进程 gap 例之后 fold 会取反常值（glibc 状态泄漏，
  P42 探针钉形 1636266600 vs 干净上下文 1636263000）——JS 恒取干净
  值（tclsh 同）。
- `clock scan -format` 的 `%Z` + `-base 0` 组合：JS 随 native
  （61200）；tclsh 是 -25200（native 既有残差，非本轮引入）。
- 错误 errorInfo 帧链含 wrapper proc 帧（clock 的 Tcl 实现帧残差
  D-25 沿用）；`-locale` 收值忽略（native 同）；超出 Date/Intl 精度
  域（|epoch| > 2^53 秒级/BC 年代 wall 场求值）退化 offset-0。

## 2026-11-13b regexp 外置（wasm 语义子集，R/S 区 22/22 真机）

wasm 子集剔除 regexp-lite（净重 97KiB raw）；regexp/regsub 经 dynmod
注入 www/regexpmod.js（Tcl ARE → JS RegExp **语义子集**）。已支持：
POSIX 类展开、\y→\b、\m/\M/\</\> 近似、quoting ***=、-nocase/
-line/-lineanchor/-linestop/-all/-inline/-indices/-start、捕获变量
（__RXCAP/__RXIDX 结构化编码 + wrapper upvar）、regsub \1..\9/&。
**子集边界（wasm 构建特有分歧）**：非贪婪支持 ✓（JS 原生）但 Tcl
ARE 的 quoting 细目/collating 元素/反向引用组**起点偏移为 indexOf
近似**；Spencer 全集（BT 引擎、精确 -indices 组起点、**/quoting）仍
在 native（rtcl_std 默认）。eval/-f 双路径探针不受影响（跑 native）。

## 2026-11-13 file/io 外置评估（测量 + feature 图解耦，定调未实现）

- file+io 净重 164KiB raw（≈56KiB gzip，本体 6%）——外置动机是浏览器
  里 std::fs 必败（功能诚实性），不是尺寸。
- 外置形态：通道协议面留原生（反射通道机制已在），数据面走 HostFs
  宿主服务（HostConsole/HostClock 同款缝）——详见 HANDOFF §10quinter。
  尚未实现。
- feature 图修复：event→io 解耦后 `(std,dynmod)` 构建可达（含无
  regexp 构建的 parse_index_int cfg bug）。

## 2026-11-12c dynmod：wasm `load`/`unload` 注册表形态（5 单测）

- rtcl 的 `load` 增加**宿主注册表**形态（feature `dynmod`）：宿主
  预实例化的侧 wasm 模块经 `Interp::register_dyn_module` 声明，
  `load name` 同步安装 `::<name>::<cmd>` 包装命令（dispatch words 由
  宿主给出——rtcl-wasm 经 `_js_dispatch` 接到模块 exports）。
- 与 tclsh 的语义差异（本质性，非缺陷）：tclsh 的 .so 与解释器共享
  地址空间、Init 自注册；wasm 模块隔离，**命令面由 rtcl 包装、参数
  字符串进出**。路径形（`load /x/y.so`）未登记时回落 load-cmd 语义
  （缺文件文案逐字；真 .so 报 dynamic loading not supported）。
- 共有面与 tclsh 一致：load 幂等（已加载重 load 为 no-op）、unload
  未加载报 `file "x" has never been loaded`、unload 解除命令绑定。

## 2026-11-12b 协程引擎落地 + wasm 可选特性门控（coroutine.probes 23/23）

- **coroutine/yield/yieldto 全语义实现**：rtcl 的字节码执行器按单元
  递归（每单元一个 Rust 栈帧），挂起无法冻结 Rust 续延——改为**挂起
  链重放**：`Error::Suspend`（新补全码，帧lessly 传播、穿透 catch/
  try、不记 errorInfo 帧）逐层把执行器状态（pc/操作栈/循环帧）冻结进
  `Interp::coro_chain`；proc 出口对 Suspend 跳过 epilogue（deferred/
  静态写回/弹帧全延迟）；收集器把帧栈移入协程记录。resume 从最内层
  重放：resume 单参成为挂起 yield 的返回值，层完成把结果喂给下层并
  弹掉该层帧——精确镜像展开时的返回路径。已对齐：coroutine 自身返回
  首个 yield 值、resume ≤1 参（wrong # args: "name ?arg?"）、完成即
  删命令（含 rename 后）、同名重建为替换、体错误销毁协程、catch 不拦
  截挂起、嵌套 proc 挂起（链式）、yieldto 外层执行（其结果=resume 命
  令结果，resume 参成为 yieldto 返回值）。探针 specs/coroutine.probes
  23/23。
- **分歧（记 OPEN）**：嵌套 `eval` 的 AST 脚本内 yield 无法冻结 AST
  续延——报 `yield cannot suspend across an eval boundary`（字节码层
  的 yield——proc 体/循环体/if 臂——任意深度均可挂起）。
- **wasm 可选特性门控**（rtcl 定位=网页 command dispatcher）：`net`
  （socket，TCP 仅原生）与 `load-cmd`（load/unload 动态加载）从
  rtcl_std 拆出、默认仅原生；wasm 构建显式不含 → info commands 计数
  wasm=98/原生=101（可选差异，记录于 HANDOFF）。纯 Rust/纯 Tcl 面
  （coroutine/zlib crc32/adler32/fcopy/case/history）全平台保留。

## 2026-11-12 哈希/迭代序对齐 + 命令面补齐（judge 88/88；order.probes 48/48）

本轮收口（Tcl 哈希表迭代序 —— 原 2026-11-08 清单第 3/5 类的序类根因）：

- **tclhash 重放模型**（crates/rtcl-vm/src/interp/tclhash.rs）：忠实重放
  tclsh 8.6.17 `Tcl_HashTable`（tclHash.c）——`h = h*9 + byte`（u64）、
  `hash & mask` 索引、4 桶起步、rebuildSize=size×3、重建 ×4 且按旧表
  桶序头插重排、迭代桶 0..N 链头→尾。Python 拟合经 20 例随机数组 +
  letters26/words15（重建路径）+ `dict info` 桶直方图全验证；模型与
  8.6.17 源码逐条对上（specs/tclhash/fit_model.py + init_order_8_6_17.txt）。
- **插入序日志**：Interp 增加 command_order/var_order/ns_order 与
  per-frame locals_order；写点经 global_insert/frame_local_insert/各
  register/note 钩子记录。迭代面（info commands/procs/vars/globals/
  locals、array names/keys/values/get、namespace children、dict info）
  从排序输出改为**按表重放**：命名空间表按**尾名**哈希（tclsh 每 ns
  一张表），帧表按**插入序**（tclsh 编译帧是线性局部数组，非哈希），
  删除项惰性过滤、重复键取最后出现（Tcl delete+recreate 语义）。
- **`info commands` 精确对齐**：canonical 101 命令注册序从 8.6.17
  tclBasic.c builtInCmds[] + ensembleCommands + TclClockInit/
  TclInterpInit/TclZlibInit + init.tcl proc 定义序 + tclsh stdin 的
  `history add` auto-load 反推，重放验证 101/101 全序一致；rtcl 超集
  命令（lambda/curry/parray/`dict getdef` 空格名注册等）标记
  hidden——可调用但不出现在 info commands（host 桥同理；http.tcl 桥
  探测改 namespace which）。`::tcl::mathop` 全 23 算子按
  mathOpCmds[] 注册序落地（** 右折叠、一元 / 为实除 0.5、
  in/ni/`%`/`<<`/`>>`/`~`/`!` 限元），命名空间表序与 tclsh 全同。
- **init.tcl 面补齐**：tclLog/auto_qualify（逐分支移植）/auto_load/
  auto_load_index/auto_import/auto_execok/history（裁剪移植）+
  case 内建（tclCmdAH.c Tcl_CaseObjCmd 逐分支移植）+ socket（TCP
  客户端；-async 不支持延迟报错，记 OPEN
  [FIXED 2026-11-15 惰性连接，见 2026-11-15 节]）/fcopy（=chan copy）/
  load/unload（无动态加载器，缺文件文案逐字）/zlib（crc32/adler32
  真实现；压缩记 OPEN）+ ::zlib 命名空间。`info commands` 计数
  101 == tclsh。
- **dict 三例收口**：`dict create -ordered/-unordered`（8.7/jimtcl
  特性）移除——前导词就是键、奇数个即标准 arity 错；stdlib 空格名
  proc `{dict getdef}`（8.7）删除（报 unknown or ambiguous）；`dict
  info` 按 Tcl_HashStats 输出桶直方图 + 平均搜索距离（%.1f）。
- **expr 词法**（bn7/bn8 族）：`nan`/`inf` 按 tclsh 词法处理为
  **常量**——后随 `(` 即 missing operator at _@_（标记紧跟名字，
  `nan(<整数>)` 例外：到达 mathfunc 报 ARITH DOMAIN）；未知函数报
  `invalid command name "tcl::mathfunc::<name>"`（expr-38.12）。
- **socket/chan**：socket TCP 客户端落地（错误前缀
  `couldn't open socket: <strerror>` 与 tclsh 同式；通道名 rtcl 用
  sockN、tclsh 用指针名——OPEN，现记设计差异）；chan names 保持创建
  序（tclsh 的通道表哈希序依赖注册历史，纯重放不吻合——见
  2026-11-08 第 5 类，记 OPEN [FIXED 2026-11-08 后 tclhash 重放模型，
  2026-11-15 差分实证]）。

新探针：specs/order.probes（48/48）、specs/expr-lex.probes（11/11）。

## 2026-11-08 增量求值落地 + 第二轮对齐（probes 97 → 80）

设计定谳：**增量求值只属于解释执行路径**。REPL、`-c`、`eval`、
`uplevel`、`source` 已改为逐命令解析执行（前缀先执行，后面的语法错误
再报）；`rtcl -f` 是编译运行入口，仍整脚本解析——因此 `-f` 下
`globprobe`/`pidx*`/`t2-t4` 等 15 个“先输出后语法错”探针是**设计差异，
不再作为对齐目标**。

本轮收口（commit c956f5b / 0cff469 / 327a960 / c1c3383）：
- parser `IncrementalScript::next_command` + `Interp::eval_incremental`；
  source 补 `(file ... line N)` 退出帧；解析错误 `-errorstack` 的
  `INNER {syntax ...}` 与 catch options 键序；`tcl_quote` 对不平衡
  花括号改用反斜杠转义（修复 catch options 字典不可解析）。
- catch `-errorline` 对内联体按 catch 命令行重定基（proc/ns 内）。
- `::tcl::dict::*` 集成命令可见（绝对键 + stdlib 建命名空间），
  `dict_p35/p42`、`p35/p42` 通过；`tcl::mathop` 仅 `+`。
- OO：`must be ... , or ...` 的 Oxford 逗号仅用于 GetIndex 风格列表；
  `oo::define/objdefine` 多词形式的元数错误带 `oo::define <typed>`；
  `self <sub>` 拒绝多余参数；`a class` 冠词。oo3/oo7/oo9/oo_dbg1 通过。
- IO：fileN 取最小空闲号（stdio 占 0..2）；`chan copy` 先校验通道
  方向再解析选项、选项必须成对；`read` 计数错误 `TCL VALUE NUMBER`；
  `-eofchar` 取元素首字符；`chan event` 空前缀报 ambiguous。
  chan_io_p3/p10/p11/p13/p13b/p17/p18/p19/p20 通过。
- 追加（957d81f/2bab128）：namespace resolver 的全局回退（读/写/
  数组 + 根限定 `::x` 绕过裸名链接），`info vars` 同名去重；
  reflected channel read 按 4096 请求并截断调用方计数（chan_io_p5）。
  probes 80 → 77。
- 追加（4847711/d1e56d4）：expr 括号替换错误抑制外层 expr/set 帧、
  空串操作数文案、嵌套 expr 逐层抑制；switch 匹配臂帧；dict with/
  filter 退出帧；unset -nocomplain 不改 ::errorCode；OO 的
  methodtype/definition、core 方法表、destroy 返回析构结果、
  instances 精确类+mixin、定义词内部帧抑制、子命名空间自动创建；
  info body/isa 元数。probes 77 → 64。

剩余 64 例（其中 15 例为上述 `-f` 整脚本设计差异，需对齐 49 例）：
1. 整脚本编译设计差异（15）：`globprobe`、`ix1/2/4`、`cases*`、
   `pidx*`、`t2/3/4`、`probe_round3`、`r3probe`。
2. errorInfo/errorstack/return 帧（9）——**2026-11-14 全收口**（见
   2026-11-14 节，6 commit）：`ei2`、
   `expr_bracket_multiline`（多行括号的 proc 行号）、
   `loops_return_level0_loop_bodies`、`namespace_uplevel/2`、
   `probe_catch`、`return_level0_codes`、`sub_inline_*`（未捕获顶层
   替换帧链）。expr 括号帧、switch arm、dict with/filter、h_* 已修。
3. 哈希/迭代序与命令面（12）：`array_names_regexp_backref`、`cmds`、
   `topcmds`、`ic/ic2`、`info2`、`tclns`、`p-uplevel-ns`、`u72`、
   `trv_p8`、`namespace_varmodel2`、`probe1`。Tcl 哈希插入序 vs
   rtcl 排序/Rust HashMap 随机种子；命令集超集 + 缺 `socket/tell/
   ::tcl::prefix/::tcl::pkgconfig` 等。
4. OO（3）：`oo1`（::zlib/标准类与哈希序）、`oo2`（info vars 序）、
   `oo4`（::oo::Helpers 与命名空间内建命令集/序）。内省/可见性/
   destroy 返回值等已修。
5. IO（3）：`chan_io_p15`（chan names 哈希序）、`p2`（socket 命令
   缺失）、`p9`（std 通道 seek/tell）。
6. dict/mathop 命令面（2）：`dict_p40`、`p40`（`::tcl::mathop::*`
   仅 +；`namespace eval tcl::mathop` 命令清单）。
7. Unicode/8.6（5）：`probe4`、`probe6`、`probe7`、`utf_U_parsehex`、
   `utf_char_classes`。tclsh 8.6 的 UCS-2/CESU-8 表示、Unicode 表版本、
   `format %c` 大码点替换、正则字符类名。
8. 表达式词法（3）：`bn7/bn8`（nan()/inf()）、`bn13`（浮点移位）。
9. 大整数表示（2）：`bnprobe2`、`pbr3`。
10. 汇编/调试（6）：`c5_dis`、`cmp_assemble`、`cmp_unsupported*`、
    `dbg_basic`、`dbg_trace`。
11. namespace/变量/trace（4）：`trv_d13/p10/p11`、
    `trace_array_get_firing`。（`vm3`/`namespace_varmodel` 已修：
    根限定 `::x` 不走裸名重定向；裸名 transient 目标 unset 后读/写/
    数组回退到同名全局；`info vars` 去重。）

## OPEN

### D-1 `interp alias` 的空解释器参数形式 [FIXED 2026-11-16]
- **行为**：`interp alias {} myalias {} puts` —— 用空字符串占位源/目标
  解释器创建别名。
- **Tcl 8.6**：接受，创建 `myalias` → `puts` 的别名。
- **rtcl 修复态**：`interp alias {slavePath} slaveCmd ?{masterPath}
  targetCmd ?arg...??` 落地——{} 路径即主解释器，创建形转投别名表、
  查询形返回 {target ?prefix...?}；非空路径报 could not find
  interpreter（smallface_p24 al1-al6）。
- **复现**：`interp alias {} myalias {} puts`
- **严重度**：已关闭。

### 异常面（error-deep.allium + specs/error-deep.probes）

### D-3 catch options 字典缺 `-errorline` / `-errorstack`（spec [D-2]）[FIXED 2026-11-05；CALL/UP 模型 2026-11-06]
- **Tcl 8.6**：error 完成的 options 字典 6 键齐全（含 -errorstack 的
  `INNER {invokeStk1 ...}` 栈项与 -errorline 行号）。
- **rtcl 修复态（2026-11-05）**：六键齐 + 键序按错误种类三分对齐
  （error 命令 raise → code-level-es-ec-ei-el；invalid command/一般
  失败 → ec-code-level-es-ei-el；ARITH → code-level-ec-ei-el-es——
  tclsh 的 dict 构造路径各有插入序，四形状实测稳定）。-errorstack =
  平铺 [INNER, <体>, CALL, <名>, ...]：INNER 体 = returnImm（cmd_error
  经 err_pending_inner 一次性预装，防被自身 dispatch harness 帧覆写）/
  invokeStk1（其余失败，harness 首帧捕获，载荷用**已求值调用词**——
  tclsh 的 pc/tos 内省；≤255 词 invokeStk1、更多 invokeStk）。
- **D-3 收口（2026-11-06，tclsh TclLogCommandInfo 模型）**：CALL/UP
  条目改为**每次错误日志事件**（err_harness_frame 两臂）按当前活动
  帧追加——不再是"proc 错误出口"模型：
  * 普通帧 = `info level 0` 渲染（调用词，含实参；proc / OO 方法 /
    ns-eval 帧各取自己的调用词；ns eval 用重建词不是源文本）；
  * uplevel 求值期间 = `UP <delta>`（delta = 弹出的 proc/ns 作用域数）；
  * 根帧不记；objc=0 特殊帧（析构器）不记且 level0 为空。
  由此自动获得：同一 proc 帧在**被内层 catch 捕获前**也已记入（tclsh
  实测 `proc p {} {catch {error x}}` → CALL p）；eval/uplevel/动态
  构造体的 invoker 日志事件各带一条；apply 的 CALL 用 `apply {term} ...`
  （level0 含 term）；tailcall 目标绑定失败 INNER = 目标调用词（无
  invokeStk1 词缀）；`next` 自身分派失败 INNER = `tclooNext`；`next`
  帧 usage 词 = `next`（`wrong # args: should be "next x"`，合成方法
  名参数不进 usage）。
  同时对齐的构造错误出口形（实测钉形）：`if`/`while`/`for` 字面体
  内联（无体帧）vs 动态体（`("while"/"for" body line N)` + invoker；
  if 只有 invoker）；`for` 的 loop-end 脚本 = `("for" loop-end
  command)`（无行号）；catch 动态体在 proc/ns 帧内多记 invoker 帧；
  try 编译态（proc 体）不加 `("try" body line N)`、handler/finally
  错误另起帧序，派发态（顶层/eval）加；构造器帧 level0 = 类命令调用
  词（`K create o 5` / `K new 5`）；对象编号与 tclsh 对齐（核心消耗到
  Obj10，用户对象从 ::oo::Obj11 起）。
- **复现**：`catch {error boom} m o; puts [dict keys $o]`
- **严重度**：已关闭。
- **D-3 残差收口（2026-11-06 第二批）**：INNER 的 bytecode 操作码形
  全部落地——替换失败按上下文记 `loadScalar1`/`loadScalar4`（局部槽位
  <256/≥256，含隐式局部）、`loadArray1`/`loadArray4`/`loadArrayStk`
  （数组元素）、`loadStk`（名键单元/限定名），tree-walk 与 bytecode
  双引擎各按自身现场判定；分派期失败记 `invokeReplace`（字面命令词
  且为 tclsh ensemble：namespace/string/info/dict/array/file/binary/
  chan/encoding）与 `invokeExpanded`（**动态** `{*}` 展开；字面展开
  tclsh 折叠为普通调用）；tailcall 内置/arity 失败 = 裸目标词。对象
  自有方法出口帧 `(object "::oo::ObjN" method "m" line N)` 已修；对象
  自有方法 `next` 的"怪形"实为 D-41a 解析序（mixin 先于自有方法）的
  观测假象，顺序修正后逐字一致。17 条操作码探针 + 3 条对象 next 探针
  全 SAME。
- **D-3 残差收口（2026-11-06 第三批）**：
  * `return` 的 -errorinfo 特例全部对齐：`-level 0 -code error` 且
    -errorinfo 为空/缺省 → 正常错误 + INNER `returnImm <msg> <显式
    options 列表>`（按给定序，去 -code/-level，-options 已展开，未知
    键照收，空 → `{}`）；-errorinfo 非空或 `-level>=1` 的 -code error
    → 帧边界物化：基 = EI（非空）或 msg，本帧不加 procedure 帧，首个
    调用方站点用 `while executing`（无 EI）/`invoked from within`（有
    EI）+ INNER invokeStk1 调用方词 + CALL 调用方帧。
  * 融合 lappend 读失败槽位精确化：编译路径按 `code.locals()` 查槽
    选 `loadScalar1`/`loadScalar4`、`loadArray1`/`loadArray4`（>255
    槽脚本实测 SAME）。
- **D-3 第四批收口（2026-11-06，局部槽位补全）**：编译局部表此前
  只登记标量写目标，数组基名与普通读名缺席，导致 >255 槽 / 数组
  交错时 INNER 操作码退回 `loadScalar1`/`loadArray1`。现按 tclsh 的
  编译局部表口径补全：数组元素引用的**基名**占一个局部槽（首次
  引用登记）；普通名的**读**也登记槽（未定义名读取失败按槽号选
  loadScalar1/4）。数组化时既有 degrade 机制把帧槽迁回名存储；
  链接变量走槽 aliased 回落。slots3（loadArray4）/slots4
  （数组先于标量 → loadScalar4；混合数组错误文本）逐字 SAME；
  多行脚本 specs/scripts/slot-hints.tcl 固化。
- **D-3 第五批收口（2026-11-06，特化命令 INNER 指令名）**：
  tclsh 编译器把 `set`/`incr`/`lappend`/`append`/`unset`/`regexp`
  编译成专用指令，运行期失败时 INNER 记录**指令名**而非 invokeStk1。
  按 tclsh 8.6.17 指令表逐类对齐（lexical = proc 局部槽；stk =
  eval/uplevel 脚本）：
  * set → `storeScalar1/4`、`storeArray1/4`（stk：`storeStk`/
    `storeArrayStk`）；
  * incr → `incrScalar1Imm`/`incrScalar1`、`incrArray1Imm`/
    `incrArray1`；槽 ≥256 或脚本体落 `incrStk(Imm)`/
    `incrArrayStk(Imm)`（无 4 字节名）；动态增量去 Imm；
  * lappend → `lappendScalar1/4`、`lappendArray1/4`（stk：
    `lappendListStk`/`lappendListArrayStk`）；
  * append → `appendScalar1/4`、`appendArray1/4`（stk：`appendStk`/
    `appendArrayStk`）；
  * unset → `unsetScalar`/`unsetArray`（stk：`unsetStk`/
    `unsetArrayStk`，无宽度）；
  * regexp 字面命令的 **pattern 编译失败** → INNER 为已求值词列表
    本身（无操作码名）；arity/bad option/-start 错误仍是 invokeStk1。
  融合指令（StoreVarPop/IncrLocalPop/LappendVarVar/ForNextIncr* 等）
  同样按目标槽/名映射；tree-walk（eval/apply）按语法选 stk 名
  （lexical 体无槽表时宽度按 1 近似）。
  同时修正 `incr` 类型冲突方向：标量目标实际为数组 → `can't set
  "...": variable is array` + `TCL WRITE VARNAME`（旧为 can't read）；
  数组元素目标基名为标量仍为 `can't read ... variable isn't array`
  + TCL LOOKUP VARNAME。
  error-deep +10 探针全 SAME（70/70）；opbat/innerbat/unsetbat/
  slots/bt 各差分脚本全 SAME；specs/scripts/opcode-inner.tcl 固化。
- **余留残差**（低频，有意保留）：
  * clock 错误的 errorInfo/errorstack 含 tclsh 的 **Tcl 实现帧**
    （`::tcl::clock::add` line 98、`::tcl::clock::scan` + `UP 1`、
    `::tcl::clock::format` 等）。tclsh 的 clock 是 Tcl 脚本库，报错帧
    反映其内部实现；rtcl 为 native 实现，无法在不伪造脚本帧栈的前提下
    逐字复现。仅 clock 的**错误消息/errorcode** 对齐，帧链有意不匹配。

### D-4 非 error 完成的 options 字典 -code/-level 未按 proc 边界解码（spec [D-3]）
- **Tcl 8.6**：catch {f}（f 内 `return hi`）→ `-code 0 -level 1`；
  catch {break} → `-code 3 -level 0`；catch {return -code error boom}
  → `-code 1 -level 1 -errorcode NONE`。
- **rtcl**：分别 `-code 2 -level 0`、`-level 1`、`-code 2 -level 0`
  （透传未解码完成码）。
- **复现**：`proc f {} {return hi}; catch {f} v o; puts $o`
- **严重度**：中。

### D-5 `return -level N` 层级丢失（spec [D-4]/[D-5]）[FIXED 2026-11-05]
- **Tcl 8.6**：① `return -code N`（N>4 整数）catch 报 2，原始 N 落
  options（-code 7 -level 1）；② 值形式 `return -level 2 v` 越两层，
  catch 得 `-code 0 -level 2`；③ `return -level 2 -code error` 在 N 层
  之上物化（errorInfo 只含 f1 帧）。
- **rtcl**：① catch 直通报 7；② `-code 2 -level 0`；③ 在第一层 proc
  边界提前物化（多出 f2/(procedure f1) 帧）。
- **复现**：`proc f2 {} {return -level 2 -code error deep}; proc f1 {} {f2}; catch f1 m; puts $::errorInfo`
- **严重度**：中。

### D-6 `return -errorcode` 污染 ::errorCode（spec [D-6]）
- **Tcl 8.6**：`return -errorcode {A B} v`（code ok）不写 ::errorCode。
- **rtcl**：写全局 ::errorCode 并置 raised 位，下一个错误的码被污染
  （后续 `expr 1/0` 的 errorCode 变 A B）。
- **复现**：`proc p {} {return -errorcode {A B} v}; catch p; catch {expr 1/0}; puts $errorCode`
- **严重度**：中。

### D-7 `exit` 可被 catch（spec [D-7]）
- **Tcl 8.6**：`catch {exit 3}` 直接退出进程 rc=3，后续代码不执行。
- **rtcl**：exit 是普通可捕获完成，打印 after，rc=0。
- **复现**：`catch {exit 3} m; puts after`
- **严重度**：高。

### D-8 `throw` 非 TIP 388 语义（spec [D-8]）
- **Tcl 8.6**：`throw errorCode message`（`throw {E C} msg` → msg / E C）。
- **rtcl**：jimtcl 风格 `throw code ?msg?`（stdlib.tcl proc），
  `throw {E C} msg` 报 bad completion code。
- **复现**：`catch {throw {E C} msg} m; puts "$m|$::errorCode"`
- **严重度**：中。

### D-9 `try` trap：语义反转【超集→已对齐 2026-11-05】
- **tclsh 8.6.17 实测反转**：trap 是 8.6 的**保留字但匹配语义为元素级
  精确前缀**（无 glob！）——{E} 匹配 [E C]、{E *} 不匹配 [E C]、
  {ARITH DIVZERO *} **不匹配** [ARITH DIVZERO divide by zero]（glob
  是 Tcl 9 语义；{ARITH DIVZERO *} 惯用法在 8.6 本就不命中）。rtcl 原
  两种状态都偏：先 bad handler 拒绝，后 glob 实现（超集）。
- **修复**：元素级精确前缀（pats.len() <= ec.len() 且逐元素 ==）；
  未匹配错误照常传播。
- **严重度**：已对齐。

### D-10 try body 错误缺 `("try" body line N)` errorInfo 帧（spec [D-10]）[FIXED 2026-11-05]
- **Tcl 8.6**：errorInfo 含 `("try" body line 1)` 出口帧。
- **rtcl**：帧缺失。**已修**：体错误先 err_exit_frame("\"try\" body")
  再落 ::errorInfo（handler 读到的即完整形）。
- **复现**：`try {error y} on error {m o} {puts $::errorInfo}`
- **严重度**：已关闭。

### D-11 文件模式顶层 `return -code error` / `break` 收尾语义 [FIXED 2026-11-03]
- **Tcl 8.6**：顶层 `return -code error boom` 打印完整 errorInfo（含
  `(file ...)` 帧）rc=1；顶层 `break` 打印
  `invoked "break" outside of a loop` + 帧序列 rc=1。
- **rtcl 原状**：`return -code error` 静默 rc=0；`break` 打印裸 `break`。
- **修复**：`Interp::decode_toplevel_flow`（eval.rs）+ 双引擎
  `defer_flow_frame`/unwind_subs(flow) 帧延迟——按 -code 解码顶层
  return（error/break/continue→错误，值形式→正常完成），帧归属=
  「看到该完成的命令」（最外层胜）。-f/-c/REPL 三面统一；t1/t2/t3/
  t5/t6/ns 七形状与 tclsh 逐字节一致（foreach 体归属亦一致）。
- **精化项（OPEN，先前即有）**：proc 内 break 经 call.rs 出口转换后
  跨内联 if 体仍缺 `invoked from within "if {1} {f}"` 帧（
  err_fresh-attribution：转换错误不应享受体内错误的抑制）。
- **严重度**：已关闭（精化项低）。

### dict 面（dict.allium + specs/dict-deep.probes）

### D-12 `dict get` 无 key 不校验 dict 形（spec [D-11]）
- **Tcl 8.6**：`dict get {a b c}`（奇数长）报 `missing value to go with key`。
- **rtcl**：原样返回 "a b c"。
- **严重度**：低。

### D-13 `dict set a(1) k v` 创建数组元素（spec [D-12]）
- **Tcl 8.6**：`can't set "a(1)": variable isn't array`。
- **rtcl**：成功创建元素 a(1)={k v}。
- **严重度**：低。

### D-14 `dict incr` / `dict lappend` 对坏现值静默重置（spec [D-13]）
- **Tcl 8.6**：incr 现值非整数 → `expected integer but got "abc"`（dict
  不变）；lappend 现值坏列表 → `unmatched open brace in list`（dict 不变）。
- **rtcl**：静默丢弃现值重置（{k 1} / {k z}）。
- **严重度**：中（静默丢数据）。

### D-15 `dict for` 返回值 / `dict map` break 保留（spec [D-14]）
- **Tcl 8.6**：dict for 恒返回 ""；dict map break 丢弃已累积，返回空 dict。
- **rtcl**：for 返回 body 末值；map break 保留已累积。
- **严重度**：低。

### D-16 `dict update` 变量不存在（spec [D-15]）
- **Tcl 8.6**：立即报 `can't read "novar": no such variable`。
- **rtcl**：当空 dict 跑 body，不创建变量不报错。
- **严重度**：低。

### D-17 dict 子命令面（打包）（spec [D-16]）
- **Tcl 8.6**：① 唯一前缀分发（`dict cr`/`dict ge` 可用）；② unknown
  消息 `unknown or ambiguous subcommand "zzz": must be ...`（errorCode
  `TCL LOOKUP SUBCOMMAND`）；③ `dict info` 多行哈希统计；④ `key not
  known` 带 `TCL LOOKUP DICT`、`dict for` 变量数错带 `TCL SYNTAX dict
  for`；⑤ usage 用 dictVarName/.../script 措辞。
- **rtcl**：仅精确匹配；`unknown dict subcommand: X`；info 一行；相关
  errorCode 全 NONE；usage dictVariable/.../body。
- **严重度**：低。

### 匹配 / 格式面（matching.allium + specs/matching-deep.probes）

### D-18 lookbehind：路由缺口泄漏内部错误 + 策略超集（spec [D-17]）
- **Tcl 8.6**：`(?<=...)` / `(?<!...)` **不是** 8.6 ARE 语法，编译报
  `couldn't compile regular expression pattern: quantifier operand invalid`。
- **rtcl**：fast 引擎路由漏检 `(?<=` / `(?<!`，Rust regex-crate 的
  "look-around ... not supported" 解析错误直接泄漏（rc=1）。修路由后 BT
  引擎会答 0/1——超集行为，严格对齐应在 validate_are 拒绝。
- **复现**：`puts [regexp {(?<!x)y} xy]`
- **严重度**：中。

### D-19 模式内十六进制转义语义（spec [D-18]）[FIXED 2026-11-06]
- **Tcl 8.6.17**（braced pattern 实测）：`\u` 收 **1-4** 位 hex、
  `\U` 收 **1-8** 位、`\x` 收 **1-2** 位（`\u0041`/`\u041` → A，
  `\u00041` = U+0004 + "1"，`\x4` = U+0004）；`\x{41}` **不是**
  8.6 语法（`invalid escape \ sequence` + REG_EESCAPE）；无 hex
  数字的 `\u`/`\U`/`\x` 同样 invalid。
- **rtcl 修复态**：`are_escape_end` 在 validate_are（含字符类内部）
  统一校验并给出 REG_EESCAPE；`expand_unicode_escapes` 把合法转义
  展开为 `regex_escape` 后的字面字符（fast/BT 两引擎共用）；
  代理区码点（U+D800..DFFF，Tcl 中永不匹配）类外展开为 `(?!)`，
  类内用 U+10FFFF 占位（Rust 字符串不可能含代理的近似）。
  * 字符入口转义：`\b` = backspace、`\B` = 反斜杠、`\a` = bell
    （不是 Perl 的 word-boundary）；`\z` 在 8.6.17 非法（`\Z`
    才是串尾），均按 oracle 钉死。
  * 数字转义（Spencer 启发式，oracle 矩阵实测）：**整个数字串**
    作为一个数——非零且 ≤ 已闭合捕获组数 → 反向引用（留给 BT
    引擎）；否则按最多 3 位八进制 + 余字面（`\3161573148` =
    U+00CE + "1573148"；`(a)\12` 在 1 组时 = "a" + 换行）。
    单个非零数字总是反向引用（无组 → REG_ESUBREG）；多位以 8/9
    开头无法八进制 → REG_EESCAPE；类内单非零数字同样 invalid。
    fast 引擎会把八进制误读（`\12` 当 U+000C），现已由展开层
    统一改写为 `\x{...}` 字面；含反向引用的模式显式路由 BT。
  regex-escapes.tcl / oct*.tcl / bref.tcl 差分全 SAME。
- **严重度**：已关闭。

### D-20 惰性量词的匹配偏好语义（spec [D-19]）[FIXED 2026-11-06]
- **Tcl 8.6**：`regexp -inline {(a+?)(a*)} aaa` → `{a a {}}`（Spencer
  引擎偏好：整体匹配停在惰性组最小展开）。
- **rtcl**：`{aaa a aa}`（回溯引擎首个命中 = Perl 风格）。
- **关闭（B 路径）**：Spencer 移植的 subre `COMBINE` 偏好序
  已复刻（`(a+?)(a*)` → `{a a {}}`、`(a+)(a*?)` → `aaa`），
  且 `regexp`/`regsub` 默认走新引擎（`RTCL_REGEX_ENGINE=fast`
  可退回旧路径）。759 例定向语料捕获 0 差异。
- **残余说明（2026-11-06）**：matching-deep 的其余 4 条 DIFF 已确认
  是 probe harness 伪差（未 catch 的顶层错误：tclsh stdin rc=0 无
  "Error: " 前缀 vs rtcl -c rc=1 自带前缀），探针已改为 catch/结构化
  断言；**唯一真实残差就是本条**（119 SAME / 1 DIFF），现已关闭。
- **量化评估**：见 `specs/regex-engine-eval.md`（定向 759 例 62 差异
  = 8.2%，随机 4000 例 523 差异 = 13.1%；惰性/SHORTER 约 87%、
  leftmost-longest 约 9%、POSIX 子匹配约 4%）。sregex 类
  Perl 兼容流式引擎**不能**修复本条，不建议复刻；路线建议（BT 精确化
  / Spencer 移植）见该文档。

### D-21 `\A` 在 `-lineanchor -start` 下被改写为 `^`（spec [D-20]）[FIXED 2026-11-06]
- **Tcl 8.6**：`\A` 只绑定 -start 偏移。
- **rtcl**：fast 引擎把 `\A` 改写 `(?m)^`，行首误命中
  （`regexp -lineanchor -start 1 {\Aa} "b\na"` → 1 vs tclsh 0）。
- **关闭（B 路径）**：Spencer 断言拆成 `Bos`（`^`，绝对串首）与
  `Bot`（`\A`，绑定 `search_start`/`-start`）；`-lineanchor` 下
  `\A` 不再变行首。`regexp -start 1 {\Aa} ba` → 1、
  `-start 1 {^a} ba` → 0 与 tclsh 一致。
- **严重度**：低。

### D-22 `regsub -command` 为 rtcl 独有扩展（spec [D-21]）
- **Tcl 8.6**：`bad option "-command": must be -all, -nocase, ...`。
- **rtcl 修复态（2026-11-06）**：-command 臂移除（parity 对齐）——
  bad option "-command": must be -all, ... 错误 + TCL LOOKUP INDEX
  option -command（string match 的 bad-option 同批加码）。相关测试
  改钉拒绝语义；command_mode 死分支已随清理删除（2026-11-06）。
- **严重度**：已关闭。

### D-23 `string match` 参数面 + 选项解析（spec [D-22]）[FIXED 2026-11-06]
- **Tcl 8.6.17**（三形状实测钉死）：
  * **位置词数必须 2 或 3**；否则
    `wrong # args: should be "string match ?-nocase? pattern string"`
    （不是 `subcommand ?arg ...?`）。
  * 只有 **3 个位置词**时首词按选项解析：Tcl C 的条件
    `length > 1 && strncmp(word, "-nocase", length) == 0`——大小写
    敏感、长度 >1 的唯一前缀（-n/-no/-noc/-nocas/-nocase）可；
    `-`/`--`/`-NOCASE`/`-nocasex` → `bad option "<word>": must be
    -nocase` + `TCL LOOKUP INDEX option <word>`。
  * **2 个位置词时不解析选项**：首词即使写成 `-nocase`/`-*`/`-x`
    也按 pattern 处理（`string match -* $opt` 合法——ui.tcl 依赖）。
  * `-nocase` 逐字符简单折叠（`string match -nocase İ i` → 1；Rust
    整串 to_lowercase 的 İ→两字符展开相悖）。
- **rtcl 旧态**：① 任何以 '-' 开头的词都按选项解析（`string match
  -x abc` 误报 bad option）；② 只认精确 `-nocase`（前缀 -noc 被拒）；
  ③ 4 词（`string match a b c`）报 bad option 而非 wrong # args；
  ④ 折叠整串 to_lowercase。
- **rtcl 修复态**：按位置词数分支（npos==3 才解析，前缀大小写敏感、
  长度>1），2 词直接 pattern/string；arity 用
  `wrong_args_with_usage("string match", 4, len, "?-nocase? pattern
  string")`；折叠逐字符。9 条新探针（含 proc 体内/动态词）全 SAME。
- **发现途径**：wasm node 套件的 ui.tcl 加载（`string match -* $opt`）
  暴露——native 判官语料未覆盖 2 词首词带 '-' 的形状。
- **严重度**：已关闭。

### D-24 `format %c` 无 BMP 截断（spec [D-23]）
- **Tcl 8.6**：%c 按 Tcl_UniChar（16 位）渲染，65536 → U+FFFD。
- **rtcl**：from_u32 直出星面字符 U+10000。
- **严重度**：低。

### D-25 clock：本地时区 / %z / scan 全缺（spec [D-24]）[FIXED 2026-11-06]
- **Tcl 8.6**：clock format 按进程本地时区渲染（TZ 生效，%Z/%z 出
  缩写/偏移）；clock scan 支持 ISO/自由格式；clock add 存在。
- **rtcl**：恒 UTC、无视 TZ；%z 未实现；scan 仅接受整数 epoch（错误
  消息自造）；无 clock add；-gmt 布尔解析窄。
- **rtcl 修复态（2026-11-06）**：
  * 子命令派发：Tcl_GetIndexFromObj 语义（精确/唯一前缀，歧义报
    `unknown or ambiguous subcommand`，码 TCL LOOKUP SUBCOMMAND）；
    seconds/microseconds/milliseconds 的 arity、clicks 的
    `?-switch?`（-milliseconds/-microseconds 前缀匹配 + TCL LOOKUP
    INDEX option）对齐。
  * `clock add` 落地：单位 year(s)/month(s)/week(s)/day(s)/hour(s)/
    minute(s)/second(s)，**按给定顺序**顺序作用（Jan 30 +1d +1m =
    Feb 29，+1m +1d = Mar 1）；年/月/日/周按目标时区**日历**运算
    （月末夹取 Jan 31 +1m = Feb 29、Feb 29 +1y = Feb 28；DST 下
    +1 day 保持墙钟、+24 hours 定长秒）；错误形/码逐字对齐
    （`unknown unit "...", must be  years, months,...` 双空格 +
    CLOCK badUnit、bad option 短语 + CLOCK badOption、布尔
    `expected boolean value but got "2"` ec=NONE、`cannot use -gmt and
    -timezone in same call` CLOCK gmtWithTimezone、长短两套 wrong #
    args：CLOCK wrongNumArgs/TCL WRONGARGS）；-gmt 严格布尔（含
    t/f/y/n 前缀、o 歧义拒）；-locale 值接受忽略。
  * `-timezone`（format/scan/add 共用）：zoneinfo 名校验（
    `time zone ":X" not found` + CLOCK badTimeZone :X），unix 下经
    libc TZ/tzset/localtime_r/mktime 求值——与 tclsh 的转换表同源
    （America/New_York、Europe/Berlin、Asia/Shanghai、Australia/
    Sydney 的格式/add 实测逐字一致，含 DST 跳变 02:30 +1d → 03:30）。
    format 的 `-gmt` 是 expr 布尔（整数 0/nonzero 收，坏词错误码
    TCL VALUE NUMBER），scan/add 是 Tcl_GetBoolean（整数 2 拒，
    ec=NONE）；format/scan 的 bad option 短语各自不同源（冒号+牛津
    逗号 vs 逗号列表）——逐字对齐。
  * 修 clock scan 带时刻 ISO 形的既有 bug（小时字段错读 4 字符，
    `{YYYY-MM-DD HH:MM:SS}` 一律 syntax error）。
  * **clock scan -format（strptime 面）**：%Y（恰 4 位）/%y（1-2 位，
    窗口 00-37→20xx、38-99→19xx）/%m/%d/%e/%H/%I/%M/%S/%j/%b/%h/%B/
    %p/%a/%A/%z/%Z/%s/%%/%n/%t + 字面与空白匹配；字段合成规则按
    tclsh 8.6.17 实测：年只在**月+日**（或 %j）同时出现时生效，日可
    单独生效，单独年/月忽略；时间只有 %H/%I 出现才整体生效；格式
    缺席的日期字段取 -base、时间默认 0；输入两端多余空白忽略、格式
    内空白须有对应输入空白；日期归一化（Feb 30 → Mar 1、13 月 → 次年
    1 月）；mismatch = `input string does not match supplied format`
    + CLOCK badInputString；%z 支持 +HHMM/+HH:MM 并回推。
  * **儒略历/格里历切换**（tclsh 8.6.17 实测：1582-10-15 切换，JDN
    2299161；1752 前后无跳变）：scan/format/add 的日期换算走
    jdn_from_civil/civil_from_jdn（<切换点用儒略公式与儒略闰年），
    `0001-01-01` → -62135769600、1582-10-04 与 1752 日期逐字一致。
- **复现**：`puts [clock format 0]; puts [clock scan 2000-01-01];
  puts [clock add 0 1 month]`
- **严重度**：已关闭（32/37 matching-deep；15 条 clock 探针全 SAME）。
  * **自由格式 scan 落地（2026-11-06，tclsh ClockFreeScan 常用子集）**：
    空串/now/today（= base）、tomorrow/yesterday、ISO/YYYYMMDD/M\/D\/Y/
    月名/日名与 D-Mon-YYYY 日期、时刻（H:MM[:SS][am|pm]、3pm、3 pm、裸
    数字≤4 位按 HHMM）、数字时区 ±HHMM、UTC/GMT、星期名（可带
    next/last；Plain=当天或之后）、相对短语（`<n> <unit>[s] [ago]`、
    next|last `<unit>`；单位 second/minute/hour/day/week/fortnight/
    month/year，sec/secs 亦收；ago 否定其**之前**未否定的全部相对项）。
    组合规则：缺日期取 base；有日期或非零日级相对量时时刻归零（显式
    时刻优先）；相对量按序施加（月/年夹月末）。裸数字不再按 epoch
    （`clock scan 123` = 今日 01:23）。35 条 -base 固定探针全 SAME。
  * **clock scan %Z 时区缩写表（2026-11-06）**：tclsh 8.6.17 的
    TimezoneTable（tclDate.c 同源，约 60 名：gmt/ut/utc/wet/bst/wat/
    at/nft/nst/ndt/ast/adt/est/edt/cst/cdt/mst/mdt/pst/pdt/yst/ydt/
    akst/akdt/hst/hdt/cat/ahst/nt/idlw/cet/cest/met/mewt/mest/swt/sst/
    fwt/fst/eet/bt/it/zp4/zp5/ist/zp6/wast/wadt/jt/cct/jst/jdt/kst/kdt/
    cast/cadt/east/eadt/gst/nzt/nzst/nzdt/idle）——名字不分大小写、
    %Z token 收字母+可选数字（zp4），优先于 -gmt/-timezone；未知缩写
    `time zone "<lower>" not found` + CLOCK badTZName。全表 70+ 项
    逐一 SAME。
  * **自由格式 scan 怪值收口（2026-11-06 第二批）**：tclsh ClockFreeScan
    的余下低频面全部逐字对齐——
    * 军事单字母时区（a=-1h…m=-12h、n=+1h…y=+12h、z=0，j 未定义）与
      全 zone_abbrev 表在自由 scan 生效（名字不分大小写）；日期后无
      时刻仅 **DST 缩写**可接受（否则 `syntax_err(end,end)`），日期后
      DST 半时区特例 ndt=+2h/cadt=-10h/eadt=-11h；
    * 数值短横日期 `Y-M-D`（年 <100 走 %y 窗口、月/日越界归一化，
      `0-0-0` = 1999-11-30；两段数值短横报 token 尾 syntax error）；
    * `next|last <月名>` 怪值（last = (年,月,1) 前推 2 天；last 后数字
      = 年份按 %y 窗口；last 后时刻报错且错误位按"年+剩余字符"规则；
      last + 绝对日期 → `more than one time zone in string`）；
    * 裸单位 = +1（`clock scan days` = 明日此刻）；裸数字 1-2 位 =
      小时、3-4 位 = HHMM（`Jan 2 2000` 无逗号 4 位数字 = 20:00，
      已有时刻则当年份）；
    * 未知 token 错误位细化（数字前缀后字母段 → 到 token 尾、标点 →
      单字符）；拼接缩写（`est5edt`）错误消息按有无 tset 二选一。
    自由 scan 电池 fs_common/fs1-6/mil 全 SAME；matching-deep 追加
    15 条（111 条中 106 SAME，5 条为存量 regex/string/scan 残差）。
  残差（有意保留）：
  * clock 错误的 errorInfo/errorstack 含 tclsh 的 Tcl 实现帧
    （`::tcl::clock::add` line 98、`::tcl::clock::scan` + `UP 1` 等）——
    tclsh 的 clock 是 Tcl 脚本库，native 实现无法逐字复现其内部帧链；
    错误消息与 errorcode 已对齐，帧链有意不匹配。
  * `-locale` 只接受值、不参与格式/解析。

### D-26 glob：Rust glob crate 语义 + 排序（spec matching [D-25]）[FIXED 2026-11-05]
- **Tcl 8.6**：`**` 只是两个 `*`（无递归）、`{a,b}` 非交替、前导点不
  被 `*` 匹配；返回顺序为目录读序（不排序）；`-tails` 存在。
- **rtcl**：`**` 递归、`{a,b}` 交替、匹配前导点、结果字典序排序、
  `-tails` 缺失（io-deep 探针实证 `**` 与 `.hid` 两例）。
- **复现**：`glob -nocomplain /tmp/rt_gt/**`（含子目录时递归）
- **严重度**：中。

### D-27 string 族：码点语义 vs tclsh 8.6 码元（UCS-2）语义【策略级】
- **Tcl 8.6**：string index/length/range 按内部 UTF-16 码元：
  `string length "a𐀀c"` = 4；index 1 返回孤立代理项。
- **rtcl**：按 Unicode 码点（= Tcl 8.7/9 语义）：length = 3，index 返回 𐀀。
- **复现**：`puts [string length "a𐀀c"]`
- **严重度**：策略分歧——rtcl 选择了 8.7/9 方向（更正确），但对 8.6
  语料系统性偏差（judge 语料未含星面字符，属盲区）。需拍板：保持码点
  （推荐，文档声明）或加 8.6 兼容模式。

### IO / chan / file / encoding 面（io.allium + specs/io-deep.probes）

### D-28 r+ 通道读写交错位置错乱（数据损坏级）[FIXED 2026-11-03]
- **Tcl 8.6**：r+ 通道单一共享位置：`puts AAAA; seek 0; read 2; puts B`
  得 "AAB"（原地覆写）。
- **rtcl 原状**：得 "AAAA\nB"——BufReader 预读使 fd 位置超前。
- **修复**：FileChannel 记账 `rw_pos` 共享逻辑位——写前 BufReader::seek
  回卷弃缓冲、写后逻辑位=fd 位；tell 返回逻辑位；read_all 后同步到
  EOF。读写交错/覆写/tell 三探针与 tclsh 逐字节一致。
- **严重度**：已关闭。

### D-29 `encoding` 命令整体缺失 + `-encoding` 不校验不使用（spec io [D-30]）[大部分 FIXED 2026-11-05]
- **Tcl 8.6**：`encoding convertfrom/convertto/names/system` 齐备
  （names 90+）；`fconfigure -encoding bogus` 报 `unknown encoding "bogus"`。
- **rtcl 修复态（2026-11-05）**：encoding 命令落地（names/system/
  convertfrom/convertto；前缀分发）；支持集 ascii/binary/identity/
  iso8859-1/utf-8（convertto/from 双向 + 不可编码字符 '?' 替换）；
  fconfigure -encoding 校验（unknown encoding "bogus" +
  TCL LOOKUP ENCODING）。
- **D-29 收口（2026-11-06）**：**86 名宽表 + 通道实际转码**。
  * `encoding names` 返回 tclsh 8.6 的**精确 86 名与顺序**；名字区分
    大小写（`UTF-8` → `unknown encoding "UTF-8"` + TCL LOOKUP
    ENCODING）。
  * `convertfrom/convertto`：utf-8 用 Tcl 的 modified-UTF-8（无效/截断
    字节按**字节值**出字符）；ascii/iso8859-1/identity 按字节直映，
    编码不可表示 → `?`；identity 编码 = 内部 UTF-8 字节；binary 仅通道
    特名（解码逐字节、编码取低字节）；unicode = UCS-2LE（>U+FFFF →
    U+FFFD、奇尾 → U+FFFD）；其余经 libc iconv（SHIFT_JIS/GB2312/
    BIG5/EUC-JP/EUC-KR/KOI8/CP8xx/CP12xx/ISO8859-*/ISO-2022-JP…），
    解码非法字节按字节值继续、编码失败字符 → `?`，ISO-2022-JP 末尾补
    `ESC ( B` 复位（tclsh 同）。
  * `encoding system` 取/设（新通道默认 -encoding；既有通道不变）、
    `encoding dirs` 取/设。
  * **通道转码**：`-encoding` 实际作用于 `read ?n?` / `gets` / `puts` /
    `chan copy`（文件/管道/内存通道统一走 cfg 编码）；UTF-8 走
    modified 解（无效字节 e9 → U+00E9，回写 UTF-8 为 c3a9）；多字节
    iconv 编码增量解码（不完整尾续读、EOF 余字节按字节值）；unicode
    按对读（奇尾 → U+FFFD）；`-translation binary` 按 tclsh 耦合为
    `-translation lf + -encoding binary`（查询显示 lf），`-encoding
    binary` 不改 translation；fconfigure 校验收 `binary`。
  * **.enc 表后端（2026-11-06 第三批收口）**：非内建编码优先解析
    `encoding dirs`（默认 `$TCL_LIBRARY/encoding` 或
    `/usr/share/tcltk/tcl8.6/encoding`）下的 `*.enc`——与 tclsh
    **同一批文件、同一张表**：S（单字节）/D（双字节，全字节为首
    字节）/M（多字节页表，含 symbol 特殊映射与 M 反斜杠补映射）/
    E（iso2022-* 转义驱动，子表复用 S/D/M；init/final/子表切换、
    END 时切回 subTables[0] 再写 final）。84 个非内建名（含
    iconv 缺名的 macGreek/macTurkish/jis0201/0208/0212/cns11643/
    gb12345/gb2312-raw/symbol/dingbats/ebcdic/cp165 与
    iso2022/iso2022-jp/iso2022-kr）在 86 名 × 强样本矩阵
    （decode 8 字节 + encode 8 码点）上与 tclsh 逐字节一致；
    通道 gets/read/puts 对 E 编码保持转义状态（init 只随首写）。
    `encoding dirs` 设置会清空表缓存。
- **残差**：无文件系统环境（wasm32-unknown-unknown、no_std、
  自定义 dirs 无 .enc）回退 iconv（若编入）/字节直映与 `?`；E 编码
  的通道写状态在 fconfigure -encoding 变更时重置（tclsh 同语义）。
- **严重度**：已关闭。

### D-44 `-translation` 的 EOL 实际改写未落地（原 D-29 残差）[FIXED 2026-11-06]
- **Tcl 8.6.17**（实测钉死）：
  * 配置是**输入/输出两套模式**；fconfigure 值收 1 或 2 元素列表
    （`{in out}`），单元素同时设两侧；非法列表 =
    `must be a one or two element list`，非法元素 =
    `must be one of auto, binary, cr, lf, crlf, or platform`（均
    ec=NONE）。
  * **auto 输入**：CRLF→LF、单独 CR→LF；**auto/platform 输出** =
    平台默认（unix LF / windows CRLF）；**platform 输入** = 平台默认
    （unix 不翻译，CR 原样保留）；lf/binary 两侧恒等；
    cr 输入 CR→LF、输出 LF→CR；crlf 输入 CRLF→LF、输出 LF→CRLF。
  * **查询形状按通道方向**：rw 恒二元组（`auto lf`），ro 只报输入
    （`auto`），wo 只报输出（`lf`）；`fconfigure $f` 的 rw 输出带
    括引（`-translation {auto lf}` / `-eofchar {{} {}}`）。
  * **默认**：输入 auto、输出平台（所以 ro 查 auto、wo 查 lf，
    而默认读行为是 auto 翻译）。
  * `-translation binary` = lf + `-encoding binary`；列表中的
    binary 元素同样置 encoding binary。
  * **gets 的行界由原始字符流决定**（不是先翻译再按 LF 切）：lf 以
    LF、cr 以 CR、crlf 以 CRLF、auto 以 CRLF/CR/LF 为界；非界的
    CR/LF 原样留在返回行内容里（`gets` 内容不做 EOL 翻译）。
  * **read ?n?/read all** 先解码再做输入翻译，N 按翻译后字符数；
    `read -nonewline` 只去掉末尾一个 `\n`（lf 模式下保留其前的
    `\r`）。
  * **chan copy**：两侧各自 translation/encoding；返回值在源/目的
    编码相同时按字节计数、不同时按字符计数；`-size` 按字符。
    binary 源（字节串对象）写入真实编码通道时按 UTF-8 解释原始字节；
    真实编码源写入 binary 通道时写 UTF-8 内部字节（Tcl_WriteObj
    的字节串/字符串差异，`puts` 不适用）。
- **rtcl 修复态**：ChannelConfig 拆 `translation_in/translation_out`
  （含持久化的 CR 前瞻回推缓冲 `read_pending`/多字节尾
  `read_bytes`/iconv 溢出 `read_chars`）；`ChannelReader` 做
  编码增量解码 + 输入翻译，gets 单独按原始字符流切行；写侧字符级
  翻译后编码；fconfigure 查询/设置/错误形按方向与列表语义对齐；
  chan copy 按上述计数/字节串规则。17 条新探针（io-deep 31→48
  SAME，余 6 条存量 DIFF）。
- **严重度**：已关闭。

### D-30 `read chan N` 按字节计数 + `read chan 0` 置 eof（spec io [D-31]）
- **Tcl 8.6**：N 是**字符数**（"héllo" read 3 → 3 字符）；read 0 不置 eof。
- **rtcl**：N 按字节分配 buffer + from_utf8_lossy（→ 2 字符）；read 0
  把 at_eof 置位（后续 eof=1）。
- **严重度**：中。

### D-31 `-c` 模式 `exit N` 退出码错误（spec io [D-32]）[FIXED 2026-11-03]
- **tclsh**：`tclsh -c 'exit 7'` rc=7 无输出。
- **rtcl 原状**：打印 `Error: exit with value: 7` 且 rc=1。
- **修复**：run_command is_exit→rc=N 静默、is_return→正常完成（2026-11-03
  工作区）；顶层 return 解码归 D-11。
- **严重度**：已关闭。

### 在途观察（2026-11-03 记）：并行会话批次
- error-deep 探针显示 D-4/D-6/D-7/D-8 已转 SAME（catch options 解码、
  -errorcode 污染、exit 不可捕获、TIP 388 throw）；dict-deep DIFF 由
  12→5。控制面批次（control.rs/dict.rs/stdlib.tcl/mod.rs）仍在途：
  **judge gen_join/gen_split/gen_ioCmd 三例 errorCode 送达回归
  （TCL WRONGARGS / TCL LOOKUP CHANNEL → NONE），待其收官验证**。

### D-32 `vwait` 空队列报错而非阻塞（spec io [D-33]）
- **tclsh**：vwait 永久阻塞（有事件循环）。
- **rtcl**：`can't wait for variable "zz": would wait forever` rc=1
  （无后台事件循环的设计后果；after 脚本仅 update/vwait 时执行）。
- **严重度**：中（设计分歧，需文档声明或实现事件循环）。

### D-33 file 族面（打包）（spec io [D-34]）
- **Tcl 8.6**：① `file mtime name time` 设置并返回新值；② file stat 13
  键（含 blksize blocks ctime）；③ `error copying "src" to "dst": file
  already exists`；④ `file dirname /` = "/"；⑤ owned 按 uid 比较。
- **rtcl**：① setter 静默忽略；② 10 键缺 blksize/blocks/ctime；③
  `error copying "src": target exists`；④ dirname / = "."；⑤ owned 近似
  （metadata 可读即 1）；⑥ copy/rename 不支持多源。
- **严重度**：低-中。

### D-34 IO 错误/解析消息面（打包）（spec io [D-35]）
- **Tcl 8.6**：① open 失败 `no such file or directory`（小写、无
  os-error 尾）；② access 支持 POSIX flag-list `{RDONLY}`；③
  `seek stdin 0` → `invalid seek`；④ fileevent 校验通道方向
  （`channel is not readable`）。
- **rtcl**：① Rust io::Error 原文 `No such file or directory (os error 2)`；
  ② flag-list 报 `bad access mode "RDONLY"`；③ `stdin is not seekable`；
  ④ fileevent 是 tailcall 假命令（把句柄当命令调 → `invalid command
  name "stdout"`）；⑤ `puts stderr -nonewline msg` 4 参静默接受
  （tclsh wrong # args）；⑥ source 不支持 -encoding、不更新 info
  script、错误帧标主文件名；⑦ open 管道句柄 `file<pid>` 命名易撞名。
- **严重度**：低（消息面/边缘面）。

### D-35 `array unset a`（整体形式）不移除数组变量本身（2026-11-03 规格复查发现）[FIXED 2026-11-05]
- **Tcl 8.6**：`array set a {x 1}; array unset a; info exists a` → 0
  （无 pattern 的 array unset 删除变量本身）。
- **rtcl**：info exists → 1（数组壳残留）。**已修 2026-11-05**：
  array unset 无 pattern 委托 unset_var 全链（变量移除 + {a {} unset}）。
- **复现**：`array set a {x 1}; array unset a; puts [info exists a]`
- **对照**：元素形式（`array unset a x`）、pattern 形式（`array unset
  a y*`）、对不存在名的 no-op 三者与 tclsh 一致。
- **严重度**：低-中。

### trace 面（trace.allium + specs/trace-deep.probes；2026-11-03 蒸馏轮）

### D-36 trace 回调重入无守卫——**原生栈溢出崩溃**【致命】[FIXED 2026-11-04]
- **Tcl 8.6**：VAR_TRACE_ACTIVE 重入守卫——回调内读写被追踪变量不再
  触发；`trace add variable x write {set x 9;#}; set v [set x 0]` → `9 9`。
- **rtcl**：`fire_traces`（vars.rs:1848-1863）无重入守卫，eval_depth
  无上限、max_call_depth 只拦 proc 调用 → **栈溢出 abort（rc 134,
  core dumped）**。read 版同样崩溃。
- **复现**：探针 trace-deep.probes 第 11/12 行。
- **严重度**：已关闭（2026-11-04）。修复：Interp.trace_active
  重入位（tclsh VAR_TRACE_ACTIVE 语义，per-exact-target：整变量
  W\0key / 元素 E\0key\0elem 各自独立守卫——整/元素互不遮蔽，与
  per-Var 语义一致）+ write trace 触发后 set 返回值重读（tclsh
  `9 9` 语义；vars.rs 三写路径 + op_store_var/op_store_local 回写
  栈顶）+ unset-in-trace 边缘（get_var 失败回空串）。五形状探针
  逐字节一致；judge 无新回归、cargo lib 零新增失败。

### D-37 变量 trace 触发面语义（打包）[①-⑧ 全部 FIXED 2026-11-04/05]
- **Tcl 8.6**：① op 渲染固定序 read,write,unset,array（info/vinfo）
  **[①②⑦ 已修 2026-11-04]**；
  ② remove 每次删一条（重复注册逐条删）；③ `unset a`（整数组）逐元素
  触发 {a k unset}；④ `array unset a` = 一次整体 {a {} unset}；⑤ 对
  缺失变量/元素的 unset 仍触发（元素还抹记录）；⑥ array op 触发面：
  array set 对已存在数组增元素、array get/names/size/exists 对缺失
  数组、array unset 均触发；⑦ 整变量 trace 先于元素 trace；⑧ upvar
  链接的读写触发且 name1=别名名。
- **rtcl**：① 词表序 array,read,unset,write 【已修：VAR_OPS_RENDER
  渲染序 + legacy r,w,u,a 序】；② retain 全删 【已修：逐条删】；
  ③ 整数组 unset 缺元素 trace 【已修 2026-11-05：删除前枚举元素、
  fire_traces_elem_only 专用通道（元素扫描不发整组——实测 tclsh 整组
  恰一次 {a {} unset} 来自末尾整变量 fire）】；④ array unset 无
  pattern 【已修：委托 unset_var 全链】；⑤ 缺失变量/元素 unset 不
  触发 【已修：scalar 触发后抹、元素触发+抹记录】；⑥ array op 触发
  面 【已修：分派级恒触发（set/get/names/size/exists/unset 五子命令、
  无论存在与否——2026-11-05 实测矩阵推翻 array.rs 旧注释的 trace-5.8
  解读），移除创建时/names 缺失时的散点 fire】；⑦ 反序 【已修：分组
  latest-first + 整组先于元素组（A1E1）】；⑧ upvar 链接写绕过
  【已修：frame_slot_write/_move 的 aliased 分支在有 trace 时回落名
  路径（别名名触发 + 重读语义），_move 变体 Err(value) 句柄归还】。
  OPEN=无。
- **严重度**：中（trace 触发面系统性偏差；⑦⑧ 影响回调语义）。

### D-38 legacy trace 形态（打包）[FIXED 2026-11-05]
- **Tcl 8.6**：① `trace variable x q cb` → `bad operations "q": should
  be one or more of rwua`，空 op 串也报错；② `trace variable a(k) w cb`
  注册元素 trace，元素写触发。
- **rtcl**：① 报新式消息、空 op 静默登记 【已修：bad operations
  "q" 文本 + 空 op 串同报错】；② a(k) 当整体标量名 【已修：legacy
  variable/vdelete/vinfo 三子命令 split_elem 路由到 elem_traces】。
- **严重度**：低。

### D-39 execution trace 触发面（打包）[②③④+①非内联面 FIXED 2026-11-05]
- **Tcl 8.6**：① enter/leave 覆盖一切命令（内建/apply/namespace eval）；
  ② leave 在 body 以 return 结束也触发（`foo 0 1 leave`）；③ 回调
  出错替换命令结果且保留记录；④ enterstep/leavestep 覆盖栈上所有被
  追踪 proc 的动态范围。
- **rtcl 修复态（2026-11-05）**：① dispatch_values Builtin 括号
  （invoke_builtin_traced，cache 命中臂+慢链两处）——非内联 builtin
  的 enter/leave 与 tclsh 一致【精化项 OPEN：字节码内联的
  set/incr/lappend/expr/while/foreach/槽 op 不经分派，需编译期 inline
  抑制（top_slots 降级先例），工程量大】；② deferred return 解码后
  进 leave（foo 0 1 leave）✓；③ handler 错误恒传播替换命令结果且
  记录保留 ✓（旧 discard+auto-remove 是误读）；④ enterstep 栈范围
  （ExecStepCtx.steps 收全栈）✓；⑤ 触发序矩阵实测钉死：enter/
  enterstep 最新先、leave/leavestep 注册序最旧先（与变量 trace 的
  latest-first 不同——trace-34.2 自洽所需）；⑥ firing 途中被 remove
  的记录即时哑（34.1 恒等式全 op 通用）。
- **精化项 OPEN**：leave 回调返回值链式进位（tclsh 实测 A 的返回成为
  B 的 result 实参）；leavestep 序未单独钉测。
- **严重度**：已关闭主体（精化项低）。

### TclOO 面（oo.allium + specs/oo-deep.probes；2026-11-03 蒸馏轮）

### D-40 构造/析构链语义【高】[FIXED 2026-11-05]
- **Tcl 8.6**：① 析构只跑最具体一个，靠 `next` 链；② 子类无 ctor 时
  沿链跑最具体继承 ctor（create 参数不丢）；③ ctor 的 variable 链到
  **定义该 ctor 的类**（基类 ctor 写基类变量）；④ `next` 无参=以无参
  调下一实现（不转发当前实参）；⑤ next 帧 level0=`next <args>`；⑥
  析构 next 用尽报 `no next destructor implementation`；⑦
  `oo::define C {superclass}`（空）重置为 ::oo::object。
- **rtcl**：① 无条件跑全 dtor 链 + next 双跑（B A A E vs B A E）；②
  只查实例类自身 ctor（参数静默丢）；③ 链到实例类（can't read "va"）；
  ④ 重传实参（A:q vs A:）；⑤ level0=`o m 7`；⑥ 统一 method 消息；
  ⑦ 置空（实例从此无 destroy）。
- **严重度**：高（TclOO 构造/析构语义核心；judge oo_probe 钉的是链序
  与报错文本，未覆盖此四处）。

### D-41 方法分派面（打包）
- **Tcl 8.6**：① 未命中转调类 `unknown` 方法；② filter 先跑并以返回
  值替换结果（objdefine filter 对对象也生效）；③ `oo::class new` 报
  unknown method（不造对象）；④ 用户元类的 create 定义脚本照常求值；
  ⑤ 对象级 unexport 隐藏继承方法；⑥ 全隐藏时报
  `object "::o" has no visible methods`；⑦ 间接 mixin 环定义时即报
  `may not mix a class into itself`。
- **rtcl 修复态（2026-11-05）**：① 链空转调 `unknown` 方法（实参
  {原方法名 实参...}；filter 存在时 filter 先拦截——D-41①② 交互）✓；
  ② filter 前置进链（对象 filters 最先、类链 linearize 序；filter 的
  `next` 顺真实方法链；**未知方法调用也进 filter**——tclsh 实测
  `o whatever` 走 FILTERED）✓；对象级 filter 落 OoObject.filters
  （objdefine 双形态）✓；③ `oo::class new` 拒绝 ✓；④ is_meta 改
  链含 ::oo::class 判定（chain_contains——用户元类脚本求值）✓；⑤
  对象 unexport → OoObject.hidden（set_export 对象臂：自有方法翻
  exported 位——oo-4.1 export Foo 转正；未定义名进 hidden）✓；⑥
  全隐藏时 `object "::o" has no visible methods` ✓；⑦ 间接 mixin 环
  定义时报（DFS mixin+superclass 边；seen 不预置目标——预置会挡回边，
  实测踩坑）✓。
- **严重度**：已关闭（oo-deep 16→21/32）。

### D-42 对象生命周期与别名（打包）[FIXED 2026-11-05]
- **Tcl 8.6**：① `rename o ""` 触发析构；② `rename o o2` 后 o2 可用
  （OO 与 rename 联动）；③ oo::copy 支持类；④ copy 三参形式要求目标
  ns 不存在、作为副本底层 ns，命令放当前 ns。
- **rtcl 修复态（2026-11-05）**：① cmd_rename 的 OO 分支——oo_rename
  （空新名 → destroy_object 析构器照跑；重键 → objects/classes/
  creation_order/实例 .class/子类与混入引用边全迁移；oo 表键 ::-canonical、
  命令表裸/限定双变体搬）✓；② o2 可用 ✓；③ copy 支持类源（OoClass
  全 clone：methods/variables/filters/ctor/dtor；不迁移实例与子类）✓；
  ④ 三参形：targetNamespace 必须不存在（exists refers to an existing
  namespace）+ 作为副本底层 ns + 命令落当前 ns ✓。
- **严重度**：已关闭。

### D-43a OO 方法/构造器/析构器错误出口帧 [FIXED 2026-11-05]
- **Tcl 8.6**：`(class "::K" method "m" line N)` / `(class "::A"
  destructor line N)` / `(class "::B" constructor line N)`（method 带
  引号，destructor/constructor 不带；类 = 当前链条目定义类）。
- **rtcl**：曾 `(procedure "o" line N)`。已修：oo_method_exit_tag
  （ActiveMethod 栈顶 + frame_depth+1==len 活性判据）挂 call.rs 出口
  帧发射点；三形状探针 SAME。
- **2026-11-06 补**：对象自有方法（oo::objdefine）出口帧 =
  `(object "::oo::ObjN" method "m" line N)`（原回落 procedure 帧）；
  析构器帧为 tclsh 的 **objc=0 特殊帧**——`info level 0` 为空、错误
  日志事件不记 CALL（frame_level0_empty 一次性旗标）。
- **严重度**：已关闭。

### D-41a 对象自有方法 vs 对象 mixin 的解析序 [FIXED 2026-11-06]
- **Tcl 8.6.17 实测**：`oo::objdefine o method m` 与
  `oo::objdefine o mixin P`（P 也有 m）并存时，**mixin 的 m 先跑**，
  其 `next` 落到对象自有方法、再类链（`$o m` → MIXIN OWN CLASS）；
  类对象（`oo::objdefine Q method m` + `oo::objdefine Q mixin R`）同序
  （`Q m` → RMIXIN QOWN）。`my` 分派未命中时的 must-be 清单是**私有
  视图**：链上全部方法名（含 unexport/对象级 hidden）+ 核心私有
  （`<cloned>`/eval/variable/varname）+ 恒在的 `unknown`，字节序排序。
- **rtcl**：build_chain/full_walk 原为"对象自有方法先于 mixin"；`my`
  清单只列导出方法。已修（顺序对调；include_private 列表按私有视图）。
- **复现**：`oo::class create K {method m {} {puts CLASS}}; set o [K new];
  oo::objdefine $o method m {} {puts OWN; next};
  oo::class create P {method m {} {puts MIXIN; next}};
  oo::objdefine $o mixin P; $o m`
- **严重度**：已关闭（语义序，不只是诊断面）。

### D-43 info 面与私有方法（打包）[大部分 FIXED 2026-11-05]
- **Tcl 8.6**：① `info object isa *` 对缺失名返回 0；② subclasses
  不含自身、含 mixin 用户类；③ `info class methods C -all` 含 mixin
  方法、不含 hidden；④ `info object variables` 列声明（未赋值也列）、
  vars 支持.pattern；⑤ oo::object 有私有方法 `my variable/varname`
  （及 `<cloned>` eval unknown）；⑥ `self call`/`self next` 的
  owner 词与形状（`{<描述符> <index>}` / 两元 `::A m`）；⑦
  filters/forward/methodtype/definition/constructor/destructor 子命令
  全实现（8.6.17）；⑧ `info object methods` 校验 -all/-localprivate/
  -private。
- **rtcl 修复态（2026-11-05）**：① isa 缺失名 → 0 ✓；② subclasses
  直系（superclass/mixin 直接含 target；不含自身）✓；⑤ oo::object 私有
  核心方法 variable/varname/eval/<cloned> 落地（Builtin 臂：variable 链
  对象 ns 变量、varname 全名、eval 对象 ns 求值；**私有**——外部调用
  报 unknown，tclsh 实测）✓；⑦ 部分：info object filters（o.filters）、
  info class constructor/destructor（恒空——tclsh 8.6.17 对已定义者也
  返回空，oracle 形状）✓。
- **精化项收口（2026-11-05 第二批）**：③ methods -all 含 mixin 方法
  （build_chain 同款遍历）+ hidden 排除 ✓；⑧ methods -flag 校验
  （bad option ... must be -all, -localprivate, or -private；object/
  class 双命令）+ -localprivate（本类私有）✓；⑥ self call =
  {全链描述符列表 index}、self next = 平铺**紧邻一对** {定义类 方法}
  （Cc→B→A 链实测只报 ::B m）✓；D-40⑤ 精化：next 帧 level0 =
  `next <args>`（frame_level0_args 一次性覆写通道，apply 同款）✓。
  **余量 OPEN**：④ vars pattern；⑦ 其余 stub（forward/methodtype/
  definition）。
- **严重度**：已关闭主体。**外观差异（非分歧）**：::oo::ObjN 编号
  计数与 tclsh 不同（元对象预占号），脚本不应依赖。

### D-45 io 命令面：POSIX 错误文本/访问模式/mtime/fileevent（2026-11-06）[FIXED]
- **Tcl 8.6.17 实测**：
  * `open` 失败 → `couldn't open "P": <小写 strerror>` +
    `POSIX <ERR> {<text>}`；`file size/atime/mtime/type/stat` 失败 →
    `could not read "P": <text>` + 同 errorCode。
  * `open name {RDONLY CREAT}` 等 POSIX 标志列表合法（RDONLY/WRONLY/
    RDWR/APPEND/BINARY/CREAT/EXCL/NOCTTY/NONBLOCK/TRUNC）；未知词 →
    `invalid access mode "X": must be ...`（ec=NONE）。BINARY 打开后
    encoding=binary、translation=lf。
  * `seek` 非 seekable → `error during seek on "c": invalid seek` +
    `POSIX ESPIPE {invalid seek}`；负 start 偏移 → EINVAL。
  * `file mtime name time` 设置 mtime（bad time → TCL VALUE NUMBER）。
  * `fileevent ch readable|writable ?script?` 查询/设置处理器；未知
    通道 → TCL LOOKUP CHANNEL；方向不符 → `channel is not readable/
    writable`（ec=NONE）；坏事件名 → TCL LOOKUP INDEX {event name}。
  * `puts stderr -nonewline hi`（4 词且首词非 -nonewline）→ wrong #
    args。
  * `dict for/map/filter` 变量表非二元 → `TCL SYNTAX dict <sub>`。
- **rtcl 修复态**：新增 errno 文本/码表与 `posix_error`（消息去
  "(os error N)" 后缀并小写首字母；errorCode `POSIX <NAME> {text}`）；
  open 支持 POSIX 标志列表；seek/mtime/fileevent 原生命令（fileevent
  取代 stdlib tailcall shim，处理器保存/查询不派发）；puts 4 词
  校验；dict 三处变量表错误装 TCL SYNTAX。io-deep **62/0**。
- **残差**：`dict info` 的 Tcl 哈希表桶统计（1 entries/4 buckets…）
  不复刻；`dict create -unordered`/`dict getdef` 仍是台账声明的
  有意扩展（dict-deep 17/3）。
- **严重度**：已关闭（io 面）。

### D-46 POSIX leftmost-longest vs leftmost-first（2026-11-06 评估）[FIXED 2026-11-06]
- **Tcl 8.6.17**（Spencer `regexec`）：默认同一位置取**最长**整体匹配
  （`a|ab` 在 `ab` 上 → `ab`；`(a|ab)` 捕获 `ab`）；子匹配按 POSIX
  消歧（`(a*)+` 在 `a` 上整体 `a`、组 `{}`）。
- **rtcl 旧引擎**：fast 引擎（Rust `regex` crate）与 BT 引擎均为
  leftmost-first（Perl）：`a|ab` → `a`；重复组捕获取贪婪迭代
  （`(a*)+` → 组 `a`）。
- **影响面**：随机 4000 例语料 47 例纯最长差异 + 21 例子匹配差异；
  与 D-20（惰性/SHORTER）合计 523/4000（13.1%）。
- **关闭（B 路径）**：Spencer Rust 移植 M0-M3 完成——ARE 解析 +
  NFA/DFA + POSIX-leftmost-longest/SHORTER 整体匹配 + `cdissect`
  捕获消歧；随机 4000 例 `-inline`（整体+全部捕获）与 tclsh
  **0 差异**，定向 759 例 0 差异。`regexp`/`regsub` 已默认路由新引擎
  （`RTCL_REGEX_ENGINE=fast` 退回旧引擎）。
- **评估**：`specs/regex-engine-eval.md`（含 sregex 复刻取舍：Perl
  语义 + octet-only + 无回引/环视，不能修本条，不建议引入）。
- **严重度**：已关闭。

## rtcl 有意扩展（超集，非 bug，parity 语料不应使用）

- `dict create -ordered/-unordered`、`dict getdef`/`getwithdefault`、
  `regsub -command`（D-22）、readdir 命令。
- BT 正则引擎支持 lookbehind（tclsh 8.6 不支持，见 D-18 策略项）、
  `\uXXXX`/`\x{...}` 模式转义（D-19）。
- string 族码点语义（D-27，8.7/9 方向）。

## 非分歧（tclsh 侧上下文失败，rtcl 严格对齐）

### gen_namespace-old 2.3 / 6.10 / 6.13 / 6.16 / 6.19（5 例）
- tclsh 8.6.17 独立运行语料自身 FAIL（提取器未含跨 case setup）；
  rtcl 逐字节复现相同失败输出。非 rtcl 缺陷。

## FIXED

### [FIXED 2026-11-03] D-2 ns eval 体中自删后的 set 复活 ns 变量表
- 体中自删后限定查询守卫 + ns_eval 退出清扫（commit d72465c）。
- variables-deep.probes 17/17 SAME 复核通过（2026-11-03）。

### [FIXED 2026-11-03（1dd2d81，工作区→已提交）] variables 深水区 3 例
- DIFF-6 `upvar 0 ::a(e) x` 元素写丢失；DIFF-7 `upvar 0 ::d::v`（ns 已删）
  静默建链；DIFF-8 ns eval 体 flat alias 泄漏外层帧。

### [FIXED 2026-10-07] 整数溢出 f64 提升（vs Tcl bignum）
- 现与 Tcl 一致提升为精确 BigInt。

### [FIXED 2026-10-07] gen_namespace-old 的 5 个 case 级失败
- 逐字节一致（judge case 级 4072/4072）。

## 2026-11-03 深水区复核确认一致的高风险面（正向保证）

- errorInfo 帧格式族（proc 链 / namespace eval / uplevel / foreach /
  lmap / 命令替换 / `(file ...)` 顶层）逐字节一致；catch-rethrow 链、
  `error ... info` 替换+帧抑制、errorCode 派生与持久化、wrong # args
  消息（builtin+proc）一致。
- try 的 finally 顺序/出错覆盖、handler 变量作用域、on 空变量表、
  无匹配重抛一致。
- dict：COW 无别名错误（嵌套取出/共享值/alias 三例）；update/with 写回
  与 break/continue/return 语义；解析错误消息逐字一致。
- regexp：BT 引擎后向引用/环视/非贪婪基本机制、`\y \m \M \A \z`、
  `-all -inline -indices` 组合、`***=` 字面、regsub `& \0 \1 \\` 展开；
  fast/BT 路由对后向引用与前瞻正确。
- binary：t/n/m 大端正确（早前嫌疑为误报）、a/A/b/B/h/H 填充矩阵、
  scan 短输入变量不动、format/scan 校验器。
- trace：变量 trace read/write/unset/array 四操作语义。
- list 深角（lindex 多层/lset/linsert 负界/lreplace 越界/lsort -unique
  -integer/split/join/concat）与 string 深角（map 空值/空键、trim、
  first 负索引、totitle、reverse）全部一致。
- io：gets EOF 三元组（-1/置空/eof 时序）、seek 越界+tell+read、
  fconfigure 未知选项消息、gets 后 seek current 位置。
- trace：变量 trace 基本 rwua 触发/参数（name/elem/op、整变量 elem=""）、
  同表 latest-first、write trace 出错后写入已落地、unset trace 错误被吞、
  info 再列表化、trace on 未创建变量、unset 抹记录、command trace
  rename/delete 参数序与 trace 随移、exec leave 参数序（cmdLine code
  result op）、enterstep 的 cmdtext 规范化。
- TclOO：C3 链序与 mixin 优先级、must-be 列表（排序+destroy 附加+
  oo::class 抑制 new）、小写导出与 export/unexport/deletemethod/
  renamemethod、方法与构造器 wrong#args 文本、info call 描述符全形
  （filter 项/unknown 项/owner 词/errorCode）、定义脚本错误帧与多词
  形无帧、create 重名、copy 保留面与 <cloned>、namespace delete 级联、
  析构错误先删后抛。
