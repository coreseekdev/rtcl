# rtcl ↔ tclsh 8.6.17 已知分歧清单

> 格式：行为声明 | Tcl 8.6 行为 | rtcl 行为 | 复现 | 严重度。
> 修复后标 `[FIXED YYYY-MM-DD]`。最近盘点：**2026-11-03 深水区全量建模**
> （error-deep / dict / matching / io / trace / oo 六份新 allium 规格
> + 探针套件差分实证；trace 蒸馏暴露致命级 D-36 崩溃，oo 蒸馏暴露
> D-40/41 构造析构与分派面）。条目后括注的
> spec [D-x] 是规格文件内的 spec-local 编号，以本台账为准。

## OPEN

### D-1 `interp alias` 的空解释器参数形式
- **行为**：`interp alias {} myalias {} puts` —— 用空字符串占位源/目标
  解释器创建别名。
- **Tcl 8.6**：接受，创建 `myalias` → `puts` 的别名。
- **rtcl**：`wrong # args: should be "interp interp"`。
- **复现**：`interp alias {} myalias {} puts`
- **严重度**：低。

### 异常面（error-deep.allium + specs/error-deep.probes）

### D-3 catch options 字典缺 `-errorline` / `-errorstack`（spec [D-2]）
- **Tcl 8.6**：error 完成的 options 字典 6 键齐全（含 -errorstack 的
  `INNER {invokeStk1 ...}` 栈项与 -errorline 行号）。
- **rtcl**：仅 4 键（-code -level -errorcode -errorinfo）。
- **复现**：`catch {error boom} m o; puts [dict keys $o]`
- **严重度**：中。

### D-4 非 error 完成的 options 字典 -code/-level 未按 proc 边界解码（spec [D-3]）
- **Tcl 8.6**：catch {f}（f 内 `return hi`）→ `-code 0 -level 1`；
  catch {break} → `-code 3 -level 0`；catch {return -code error boom}
  → `-code 1 -level 1 -errorcode NONE`。
- **rtcl**：分别 `-code 2 -level 0`、`-level 1`、`-code 2 -level 0`
  （透传未解码完成码）。
- **复现**：`proc f {} {return hi}; catch {f} v o; puts $o`
- **严重度**：中。

### D-5 `return -level N` 层级丢失（spec [D-4]/[D-5]）
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

### D-9 `try` 无 `trap` handler（spec [D-9]）
- **Tcl 8.6**：`try {...} trap {ARITH DIVZERO *} {m} {...}` 按 errorCode
  前缀匹配。
- **rtcl**：`bad handler type "trap"`。
- **复现**：`try {expr 1/0} trap {ARITH DIVZERO *} {m} {puts $m}`
- **严重度**：高（8.6 核心语法缺失）。

### D-10 try body 错误缺 `("try" body line N)` errorInfo 帧（spec [D-10]）
- **Tcl 8.6**：errorInfo 含 `("try" body line 1)` 出口帧。
- **rtcl**：帧缺失。
- **复现**：`try {error y} on error {m o} {puts $::errorInfo}`
- **严重度**：低。

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

### D-19 模式内 `\uXXXX` / `\x{...}` 被 rtcl 接受（spec [D-18]）
- **Tcl 8.6**：invalid escape（`\x41` 两位十六进制合法）。
- **rtcl**：接受并按 Unicode 语义匹配。
- **严重度**：低（超集）。

### D-20 惰性量词的匹配偏好语义（spec [D-19]）
- **Tcl 8.6**：`regexp -inline {(a+?)(a*)} aaa` → `{a a {}}`（Spencer
  引擎偏好：整体匹配停在惰性组最小展开）。
- **rtcl**：`{aaa a aa}`（回溯引擎首个命中 = Perl 风格）。
- **严重度**：中（BT 引擎需实现 Tcl 偏好序，非平凡）。

### D-21 `\A` 在 `-lineanchor -start` 下被改写为 `^`（spec [D-20]）
- **Tcl 8.6**：`\A` 只绑定 -start 偏移。
- **rtcl**：fast 引擎把 `\A` 改写 `(?m)^`，行首误命中
  （`regexp -lineanchor -start 1 {\Aa} "b\na"` → 1 vs tclsh 0）。
- **严重度**：低。

### D-22 `regsub -command` 为 rtcl 独有扩展（spec [D-21]）
- **Tcl 8.6**：`bad option "-command": must be -all, -nocase, ...`。
- **rtcl**：支持 -command（替换参数粗暴括引，值含 {} 时可能解析错乱）。
- **严重度**：低（建议默认拒绝、开关开启）。

### D-23 `string match` 参数面 + `-nocase` 折叠（spec [D-22]）
- **Tcl 8.6**：① >3 词时首词按选项解析：`string match a b c` →
  `bad option "a": must be -nocase`；② -nocase 逐字符简单折叠，
  `string match -nocase İ i` → 1。
- **rtcl**：① 返回 0；② 整串 Rust to_lowercase（İ 两字符展开）→ 0。
- **严重度**：低。

### D-24 `format %c` 无 BMP 截断（spec [D-23]）
- **Tcl 8.6**：%c 按 Tcl_UniChar（16 位）渲染，65536 → U+FFFD。
- **rtcl**：from_u32 直出星面字符 U+10000。
- **严重度**：低。

### D-25 clock：本地时区 / %z / scan 全缺（spec [D-24]）
- **Tcl 8.6**：clock format 按进程本地时区渲染（TZ 生效，%Z/%z 出
  缩写/偏移）；clock scan 支持 ISO/自由格式；clock add 存在。
- **rtcl**：恒 UTC、无视 TZ；%z 未实现；scan 仅接受整数 epoch（错误
  消息自造）；无 clock add；-gmt 布尔解析窄。
- **复现**：`puts [clock format 0]; puts [clock scan 2000-01-01]`
- **严重度**：高（时间类脚本系统性偏差；judge 语料因确定性要求未覆盖
  clock，属盲区）。

### D-26 glob：Rust glob crate 语义 + 排序（spec matching [D-25]）
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

### D-29 `encoding` 命令整体缺失 + `-encoding` 不校验不使用（spec io [D-30]）
- **Tcl 8.6**：`encoding convertfrom/convertto/names/system` 齐备
  （names 90+）；`fconfigure -encoding bogus` 报 `unknown encoding "bogus"`。
- **rtcl**：encoding 命令不存在（`invalid command name "encoding"`）；
  fconfigure -encoding 任意字符串静默存取（查询回显 bogus）；无任何
  转码实现；非 UTF-8 输入 read/gets 直接 io 错误。
- **严重度**：高。

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

### D-35 `array unset a`（整体形式）不移除数组变量本身（2026-11-03 规格复查发现）
- **Tcl 8.6**：`array set a {x 1}; array unset a; info exists a` → 0
  （无 pattern 的 array unset 删除变量本身）。
- **rtcl**：info exists → 1（数组壳残留）。
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

### D-37 变量 trace 触发面语义（打包）
- **Tcl 8.6**：① op 渲染固定序 read,write,unset,array（info/vinfo）
  **[①②⑦ 已修 2026-11-04]**；
  ② remove 每次删一条（重复注册逐条删）；③ `unset a`（整数组）逐元素
  触发 {a k unset}；④ `array unset a` = 一次整体 {a {} unset}；⑤ 对
  缺失变量/元素的 unset 仍触发（元素还抹记录）；⑥ array op 触发面：
  array set 对已存在数组增元素、array get/names/size/exists 对缺失
  数组、array unset 均触发；⑦ 整变量 trace 先于元素 trace；⑧ upvar
  链接的读写触发且 name1=别名名。
- **rtcl**：① 词表序 array,read,unset,write 【已修：VAR_OPS_RENDER
  渲染序 + legacy r,w,u,a 序】；② retain 全删 【已修：逐条删】；③④⑧
  缺失触发；⑤ 标量 missing 不触发但抹记录、元素 missing 不触发不抹；
  ⑥ 仅创建时 array set + array names 缺失；⑦ 反序 【已修：分组
  latest-first + 整组先于元素组（A1E1）】。OPEN=③④⑤⑥⑧。
- **严重度**：中（trace 触发面系统性偏差；⑦⑧ 影响回调语义）。

### D-38 legacy trace 形态（打包）
- **Tcl 8.6**：① `trace variable x q cb` → `bad operations "q": should
  be one or more of rwua`，空 op 串也报错；② `trace variable a(k) w cb`
  注册元素 trace，元素写触发。
- **rtcl**：① 报新式消息、空 op 静默登记；② a(k) 当整体标量名，永不
  触发（vinfo 显示正常）。
- **严重度**：低。

### D-39 execution trace 触发面（打包）
- **Tcl 8.6**：① enter/leave 覆盖一切命令（内建/apply/namespace eval）；
  ② leave 在 body 以 return 结束也触发（`foo 0 1 leave`）；③ 回调
  出错替换命令结果且保留记录；④ enterstep/leavestep 覆盖栈上所有被
  追踪 proc 的动态范围。
- **rtcl**：① 仅挂 call_proc（内建/apply 不触发）；② return 完成不
  触发 leave（call.rs:710-716）；③ 吞错且删记录（仅 enter 传播）；
  ④ 只匹配最内层帧（exec_step_stack.last()）。
- **严重度**：中。注：trace.rs 头注 "stored-but-not-fired" 已过时
  （command/execution trace 实际触发但面窄），本轮已改。

### TclOO 面（oo.allium + specs/oo-deep.probes；2026-11-03 蒸馏轮）

### D-40 构造/析构链语义【高】
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
- **rtcl**：① 无 unknown 回退；② filter 只存不执行（对象目标被静默
  吞）；③ 造出非类普通对象；④ 脚本当 ctor 参数丢（仅 ::oo::class
  精确匹配 is_meta）；⑤ no-op；⑥ must-be 无条件 append destroy；⑦
  接受（linearize 深度 64 截断静默）。
- **严重度**：高。

### D-42 对象生命周期与别名（打包）
- **Tcl 8.6**：① `rename o ""` 触发析构；② `rename o o2` 后 o2 可用
  （OO 与 rename 联动）；③ oo::copy 支持类；④ copy 三参形式要求目标
  ns 不存在、作为副本底层 ns，命令放当前 ns。
- **rtcl**：① 仅删命令（析构器不跑）；② o2 invalid；③ 报 does not
  refer to an object；④ 命令放 targetNS 内且不查已存在。
- **严重度**：中-高。

### D-43 info 面与私有方法（打包）
- **Tcl 8.6**：① `info object isa *` 对缺失名返回 0；② subclasses
  不含自身、含 mixin 用户类；③ `info class methods C -all` 含 mixin
  方法、不含 hidden；④ `info object variables` 列声明（未赋值也列）、
  vars 支持.pattern；⑤ oo::object 有私有方法 `my variable/varname`
  （及 `<cloned>` eval unknown）；⑥ `self call`/`self next` 的
  owner 词与形状（`{<描述符> <index>}` / 两元 `::A m`）；⑦
  filters/forward/methodtype/definition/constructor/destructor 子命令
  全实现（8.6.17）；⑧ `info object methods` 校验 -all/-localprivate/
  -private。
- **rtcl**：① 抛 does not refer to an object；② 含自身、缺 mixin
  用户；③ 只走 superclass 链、列 hidden；④ 只列活变量、vars 恰 4 参；
  ⑤ 缺 variable/varname；⑥ owner 词空串/四元描述符；⑦ 全 stub；⑧
  任意 -flag 静默忽略。
- **严重度**：中（信息面/内省面）。**外观差异（非分歧）**：::oo::ObjN
  编号计数与 tclsh 不同（元对象预占号），脚本不应依赖。

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
