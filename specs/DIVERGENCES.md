# rtcl ↔ tclsh 8.6.17 已知分歧清单

> 格式：行为声明 | Tcl 8.6 行为 | rtcl 行为 | 复现 | 严重度。
> 修复后标 `[FIXED YYYY-MM-DD]`。最近盘点：**2026-11-03 深水区全量建模**
> （error-deep / dict / matching / io / trace / oo 六份新 allium 规格
> + 探针套件差分实证；trace 蒸馏暴露致命级 D-36 崩溃，oo 蒸馏暴露
> D-40/41 构造析构与分派面）。条目后括注的
> spec [D-x] 是规格文件内的 spec-local 编号，以本台账为准。

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
  客户端；-async 不支持延迟报错，记 OPEN）/fcopy（=chan copy）/
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
  sockN、tclsh 用指针名——OPEN）；chan names 保持创建序（tclsh 的
  通道表哈希序依赖注册历史，纯重放不吻合——见 2026-11-08 第 5 类，
  记 OPEN）。

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
2. errorInfo/errorstack/return 帧（9）：`ei2`、
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

### D-1 `interp alias` 的空解释器参数形式
- **行为**：`interp alias {} myalias {} puts` —— 用空字符串占位源/目标
  解释器创建别名。
- **Tcl 8.6**：接受，创建 `myalias` → `puts` 的别名。
- **rtcl**：`wrong # args: should be "interp interp"`。
- **复现**：`interp alias {} myalias {} puts`
- **严重度**：低。

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
