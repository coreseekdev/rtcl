# rtcl 代码审计（2026-11-23 轮，perf/wave2 @ c76ae3b）

- **范围**：全仓 13.6 万行 Rust——Value 表示与内存安全、执行核心（eval /
  call / vm_exec / vars）、OS 命令面（exec / io / file / socket / interp /
  clock / binary / HTTP）、解析器与字节码编译器、正则引擎（spencer 移植）、
  数据结构命令与 TclOO / namespace、wasm 绑定 / JIT / Tk 运行时。
- **方法**：六路分模块代码通读 + 现有 release 二进制实跑复现 + wasm
  产物（`crates/rtcl-wasm/pkg`，11-20 构建）在 Node 22 实跑 + 约 1.5 万次
  边界值模糊探测 + tclsh 8.6.17 对照。每一条高危结论都在 HEAD 对应的
  19:03 构建上重跑过。
- **分级**：P0 = 一行脚本让宿主进程 abort（`panic = "abort"`，`catch`
  拦不住）或 wasm 句柄/实例失效；P1 = 隔离与资源治理缺失、原生下的挂起
  与文件安全问题；P2 = 性能与超线性退化。
- **判官为什么没抓到**：`judge/run.sh` 的语料是 ASCII 的 tclsh 官方测试，
  不含非 ASCII 操作数、超深嵌套、天文数字尺寸这类输入。
- **可执行记录**：崩溃类与静默错误类逐条落在
  [`judge/crash-probes/`](../judge/crash-probes/)，门禁
  [`judge/crash-gate.sh`](../judge/crash-gate.sh)。

## 结论（TL;DR）

语义成熟度很高，但**不能承载不可信脚本**。确认 30 余类脚本可触发的
崩溃或挂起、11 类不报错的错误结果。根因集中在六个设计缺口：

1. **栈安全只守 proc 调用**。`call_depth` 只在 proc / 内建调用时递增；
   解析、编译、值序列化、Drop、JSON、ensemble 重分派、alias、trace
   回调都是守卫之外的原生递归（S11–S20）。
2. **没有资源治理层**。步数、内存、递归深度、时间默认无限或写死；
   分配失败走 Rust 的 abort 而不是 Tcl 错误（S21、S22、S28）。
3. **没有能力模型**。危险命令只在编译期由 feature 裁剪，运行时无法
   启停；没有 safe interp；dynmod 分派不校验实例身份（S24–S27——M6 已修）。
4. **panic 无兜底，也没有压数量的防线**。全仓没有 `catch_unwind`，
   `judge/panic-census.sh` 统计生产路径 513 处 panic 点；没有 CI、
   没有 fuzz（S1–S10、S29、S30）。
5. **解释器状态靠手工同步**。`Interp` 126 个字段，错误帧协议靠约 230
   处手工保存/恢复的标志位；`top_slots` 是变量存储的第二权威，
   `cmd_generation` 与 `tier1_epoch` 两套手工失效机制并存（C3、C4、
   C6、C7）。
6. **通道层是同步空壳**。逐字节读取，`-blocking` 只存不用，
   `fileevent` 不派发；exec 是朴素的顺序阻塞 IO（S31–S33、P1）。

## P0：脚本可触发的 abort（两引擎皆中）

| # | 触发 | 位置 | 根因 | 处置 |
|---|---|---|---|---|
| S1 | `set name 张三; if {$name < "x"}` | `types/bignum.rs` `parse_literal` | `&body[..2]` 按字节切 UTF-8；wasm 上 `==` 也中 | 已修（M1） |
| S2 | `set mode 有効なモードです; if $mode` | `types/expr_check.rs` `resolve_bareword` | `&word[..22]` 同类 | 已修（M1） |
| S3 | `format %.65536f 1.5` | `commands/io.rs` `cmd_format` | `FMT_WIDTH_CAP = 1<<20` 高于 Rust 浮点格式化的 65535 上限 | 已修（M1） |
| S4 | `format %3$*.*d 5 6 7` | `commands/io.rs` 星号取参 | 取参后索引越界 | 已修（M1） |
| S5 | `sleep inf`、`sleep 1e30` | `commands/os.rs` | `Duration::from_secs_f64` 对非有限值 panic | 已修（M1） |
| S6 | `clock format -100000000000000000` | `commands/clock.rs` `civil_from_jdn` | 负月份切片越界 | 已修（M1） |
| S7 | `/ -9223372036854775808 -1` | `commands/misc.rs` | 整除溢出 | 已修（M1） |
| S8 | `unset ::tcl_version; info tclversion` | `commands/misc.rs` | map 下标缺键 | 已修（M1） |
| S9 | `string repeat ab 9223372036854775807`、`lrepeat 9223372036854775807 x` | `string_cmds.rs` / `list.rs` | 乘积回绕后 capacity overflow | 已修（M1） |
| S10 | `pack v 1 le 0 -1` | `commands/introspect.rs` | 负偏移转 usize，分配 2^61 字节 | 已修（M1） |
| S11 | `namespace unknown ::nosuchhandler` 后调用未知命令 | `interp/eval.rs` | 处理器自指递归 | 已修（M2） |
| S12 | ensemble `-unknown` 映射到不存在的子命令 | `commands/namespace.rs` | 重分派不计深度 | 已修（M2） |
| S13 | `alias a a; a` | `commands/introspect.rs` | 直调函数指针绕过深度检查 | 已修（M2） |
| S14 | 一串 write-trace 互相写下一个变量 | `interp/vars.rs` `fire_traces_inner` | 仅 VM 引擎；树引擎正常报错 | 已修（M2） |

## P0：深度与大小无上限

| # | 触发 | 位置 | 说明 | 处置 |
|---|---|---|---|---|
| S15 | 嵌套 list/dict 约 1.3 万层转字符串；16 万层的 list `unset` | `value.rs` `serialize_list` / `drop_last_owner` | 序列化与 Drop 都是原生递归；wasm 5000 层即中 | 已修（M2） |
| S16 | `json decode` 约 1.5–2 万层嵌套 | `commands/json.rs` | 纯数据驱动，输入几十 KB；wasm 约 5000 层 | 已修（M2） |
| S17 | `expr` 非花括号 4000 层括号、5 万个连续 `-` | `types/expr.rs`、`expr_check.rs`、`parser/src/expr_compile.rs` | 三套独立递归下降都无深度概念 | 已修（M2） |
| S18 | proc 内 2 万层嵌套 `[…]` | `parser/src/compiler.rs` | 不调用也崩 | 已修（M2） |
| S19 | 正则：6000 层嵌套括号；`((a{255}){255}){255}`；`(.*)x` 匹配 3 万字符 | `regexp-spencer` `parser.rs` / `captures.rs` | 后两项吃满近 3GB 后 abort | 已修（M2） |
| S20 | proc 递归 650–1000 层 | `interp/mod.rs` `max_call_depth` | 上限 1000 没按栈大小校准：wasm 必崩且毒化实例，原生栈 ≤3MB 也崩 | 已修（M3） |
| S21 | `lrepeat 4294967296 x`、`string repeat x 4294967296` | 同 S9 | 分配失败 = abort，不可 catch；tclsh 报 Tcl 错误 | 未修 |
| S22 | `eval eval … 1` 嵌套 2000 层 | `interp/eval.rs` | 输入 1 万字节，报递归错误前已占 306MB | 已修（M3） |

## P0：wasm 专有（Node 22 + 11-20 产物实测）

| # | 现象 | 位置 | 处置 |
|---|---|---|---|
| S23 | 栈溢出 trap 后整个实例永久失效：同实例 `new RtclHandle()` 也崩，GC 终结器再触发 trap 可杀死 Node 进程；panic 则使当前句柄永久失效 | `rtcl-wasm/src/lib.rs` `exec` | 未修 |
| S24 | 句柄 A 的脚本 `_js_dispatch 1 <名字>` 可调到只在句柄 B 注册的宿主回调并拿到返回值 | `rtcl-wasm/src/js_command.rs` | 已修（M6）：分发桥按**调用它的解释器**找回调桶（`Interp::host_instance_id`，由宿主设置、脚本改不了），脚本给的 id 一律忽略。A/B 实测：修复前 A 用槽位 1 拿到 B 的回调返回值，修复后全部槽位被拒。一并用花括号词包装注册名（原来把名字直接拼进 `proc` 的脚本体） |
| S25 | 子解释器不继承宿主服务：wasm 宿主里 `$h eval {after …}` **trap**（wasm32 没有原生时钟）之后句柄永久不可用；`puts` 输出静默丢失。**而且子解释器是步数预算的旁路**：一个在宿主武装预算之前创建的子解释器里 `while {1} {}` 永远跑下去 | `commands/interp_cmd.rs`、`interp/mod.rs` | 已修（M6）：控制台与时钟改为 `Rc` 共享，子解释器继承；步数计数器父子共享（预算约束整个家族的总工作量），递归上限子不高于父，宿主之后调低也会传下去 |

## P0：脚本可触发的死循环（两引擎皆中）

| # | 触发 | 位置 | 根因 | 处置 |
|---|---|---|---|---|
| S41 | `expr {3 #}`、`expr {#}`、`expr {1 + #}` | `types/expr_check.rs` `check_expr` | 检查器把 `#` 当注释扫掉后 `continue` 却不推进游标——表达式里出现 `#` 即 100% CPU 永不返回。tclsh 报 `invalid character "#"`（`TCL PARSE EXPR BADCHAR`）。wasm 上就是冻结的标签页，且步数预算看不见它 | 已修（M4） |

## P1：隔离与资源治理

| # | 问题 | 位置 | 处置 |
|---|---|---|---|
| S26 | 没有 safe interp；`interp create -safe` 报错，子解释器带全部命令 | `commands/interp_cmd.rs` | 已修（M6）：`interp` 命令族按 tclsh `tclInterp.c` 重做。路径是列表（`{a b}` 是 a 里的 b，空路径是当前解释器）；`interp create ?-safe? ?--? ?path?`（`-safe` 可在路径之后，默认名 `interp0`）、解释器自身的命令（`$path eval/hide/…`）、`delete/exists/slaves(children)/eval/issafe/recursionlimit/marktrusted/debug`。`-safe`：隐藏 tclsh 的清单加上 rtcl 自有的 `auto_*`、`job`、`wait`、`http`，去掉 `env`、`tcl_platform(os/osVersion/machine/user)`、库路径变量和三个标准通道，`clock` 改成指向父解释器的别名；安全解释器里的 `interp` 仍可用，但只能建安全子解释器，`hide/expose/invokehidden/marktrusted` 与改递归上限一律 `permission denied…` + `TCL OPERATION INTERP UNSAFE`。有意保留的差异：不删正在求值的解释器（或其下有正在求值的）；`interp limit`、`bgerror` 的设置、`cancel`、`share`、`transfer` 未建（明确报 not supported）。探针 d21（295 条断言，期望值即 tclsh 8.6.17 的输出）两引擎全过 |
| S27 | `hide_command` 只从 `info commands` 隐身，脚本照常可调 | `interp/mod.rs` | 已修（M6）：`interp hide` 把命令从 `commands`/`procs`/`ensembles` 里真正拿走，按 token 放进 `hidden_cmds`，编译缓存同步失效——隐藏 `set` 之后，隐藏前编译的 proc、之后编译的 proc、顶层脚本都报 `invalid command name "set"`；只有 `interp invokehidden` 能调，`interp expose` 放回（可换名，重名报 `exposed command "x" already exists`）。`hide/expose/hidden/invokehidden` 的消息与 errorCode 逐项对齐 tclsh。宿主桥用的 `Interp::hide_command`（只影响列表）保留，文档写明它不是隔离手段 |
| S28 | 步数预算默认无限，CLI 与 wasm 都没暴露；`timerate {} N`、`::tcl::unsupported::assemble` 可旁路 | `interp/mod.rs` | 已修（M3） |
| S29 | 环境含非 UTF-8 变量时 `Interp::new()` panic | `interp/mod.rs` | 已修（M3） |
| S30 | release `panic = "abort"`，全仓无 `catch_unwind` | `Cargo.toml` | 未修 |

## P1：原生 OS 面的挂起与文件安全

| # | 问题 | 位置 | 处置 |
|---|---|---|---|
| S31 | exec 三处死锁（stderr 超 64KiB 且 stdout 未关、`<<` 超管道容量、后台作业写大量 stderr）；`exit` 被存活的管道子进程阻塞 3 秒 | `commands/exec_cmd.rs` | 死锁三项已修（M5）；`exit` 阻塞见 S32 |
| S32 | `close` 仍在连接的 `-async` socket 无限阻塞（内核自己的 connect 超时是分钟级）；`exit` 到来时存活的管道子进程把退出拖住（`set f [open "|sleep 3"]; exit 0` 要 3 秒） | `channel.rs`、`rtcl-cli/src/main.rs` | 已修（M5）：没有缓冲输出时 `close` 直接丢弃未完成的连接（tclsh 0ms；有缓冲输出且阻塞模式时 tclsh 自己也等连接，保持不变）。CLI 的每次运行结束（脚本跑完、`exit`、出错）都走 `Interp::abandon_channels`——不等管道子进程；嵌入宿主里的普通 `Drop` 仍然等（活着的进程里不该留下没收的子进程） |
| S33 | exec 遇非 UTF-8 输出报错并留僵尸进程 | `commands/exec_cmd.rs` | 已修（M5）：非 UTF-8 逐字节映射成同值字符（同 tclsh）；不留僵尸——输出读不出来时 kill + 收尸，spawn 失败时把已启动的阶段分离、由下次 exec 收（tclsh 同） |
| S34 | `file tempfile` 任意路径前缀、无 `O_EXCL`、0644 | `host.rs`、`commands/io.rs` | 已修（M5）：按 tclsh 8.6.17 重做——`O_EXCL` 创建、0600、返回读写通道、路径写进 `nameVar`；相对模板只取末尾放进临时目录（带目录部分的原样用）、`_` 加 6 位随机串插在扩展名之前 |
| S35 | `file delete` 不带 `-force` 也递归删非空目录 | `commands/io.rs` | 已修（M5）：非空目录报 `error deleting "…": directory not empty`（`POSIX EEXIST`），内容不动；符号链接只删链接本身；不存在的路径不报错；未知选项是 tclsh 的 `bad option` |
| S36 | `file normalize a/../x` 原样返回 | `commands/io.rs` | 已修（M5）：词法规范化（接上工作目录、消解 `.`/`..`、合并分隔符、`~` 展开），不查文件系统。`~user` 未实现（C33） |
| S37 | `-async` socket 的 fd 无 `CLOEXEC` | `channel.rs` | 已修（M5）：裸 `socket()` 之后设 `FD_CLOEXEC`（A/B 实测：修复前 `exec` 子进程继承该描述符，修复后不继承） |
| S38 | HTTP 客户端默认无超时、响应体无上限、无 URL 审查钩子（读码） | `rtcl-native/src/net.rs` | 未修 |
| S39 | 变量表无密钥哈希，可预构造冲突键（风险低） | `interp/varmap.rs` | 未修 |
| S40 | Tk：畸形 PNG、多字节颜色串、`image put -to` 负坐标 panic（读码，未运行） | `rtcl-tk-runtime/src/lib.rs` | 未修 |

## 静默出错（不报错，结果是错的）

| # | 现象 | 位置 | 处置 |
|---|---|---|---|
| C1 | 一个编译体内 ≥65536 个常量/局部变量/参数时操作数 `as u16` 截断；`p PARAM` 返回 `65535 65536` | `parser/src/compiler.rs`、`rtcl-ir/src/bytecode.rs` | 已修（M4） |
| C2 | 回溯预算耗尽后 `regexp {a+()\1x}` 对 2000 个 `a` 返回“不匹配” | `regexp-spencer/src/backtrack.rs` | 已修（M2） |
| C3 | `set g 0; upvar 0 g q; set q 5` 之后 `$g` 仍是 0 | `interp/vm_exec.rs` 顶层槽边界 | 已修（M4） |
| C4 | 命令中途重定义后再注册的 write-trace 永不触发 | `interp/vm_exec.rs` tier1 重分派 | 已修（M4） |
| C5 | `+ 9223372036854775807 1` 得负数，`expr` 正确宽化 | `commands/misc.rs` | 已修（M1） |
| C6 | `tailcall` 不做命名空间解析 | `interp/call.rs` | 已修（M4） |
| C7 | `coroutine set …` 覆盖内建后已编译 proc 仍用内联 `set` | `commands/coro.rs` | 已修（M4） |
| C8 | `oo::objdefine $o unexport m` 对继承方法无效 | `commands/oo.rs` | 已修（M4） |
| C9 | 析构器内 `C destroy` 使析构器被跑约 500 遍 | `commands/oo.rs` | 已修（M4） |
| C10 | 旧式八进制 ≥2^63 回绕；大写十六进制 bignum 判为非数字 | `value.rs`、`types/bignum.rs` | 已修（M1） |
| C11 | `wait` 对已退出后台进程返回 127，`-nohang` 恒为 NONE | `commands/os.rs` | 已修（M4） |
| C12 | 修复过程中新查出：`set v 018; expr {$v + 0}` 得 18（tclsh 报 invalid octal）；超过 64 位的旧式八进制被当作十进制；带 bignum 操作数的浮点运算取错值。根因是 Tcl 整数字面量语法有**四套**各自独立的实现（`value.rs`、`bignum.rs`、`expr.rs` 两处、`list.rs`），对旧式八进制理解不一 | `types/bignum.rs` | 已修（M1）：合并为单一的 `bignum::parse_int`，四处改为调用它 |
| C15 | 修复过程中新查出：命令名到注册键的解析（`rename`、执行 trace、`tailcall` 共用的 `resolve_command_key`）先查精确键再查当前命名空间，与真正的分派链顺序相反。命名空间里有同名命令时 `rename h {}` 删掉的是全局的 `h`（tclsh 删命名空间里的） | `commands/proc.rs` | 已修（M4）：顺序改为与分派链一致 |
| C14 | 修复过程中新查出：**约 100 字符的正则**——25 层嵌套的 `{2}`——在解析阶段吃光 3GB 后 abort；tclsh 对同样模式 16ms 内报 `out of memory`。量词重写克隆操作数，嵌套量词让语法树每层翻倍，NFA 再按乘积展开 | `regexp-spencer/src/ast.rs`、`parser.rs` | 已修（M2）：每个节点带预估状态数，解析时超编译空间预算即拒绝（tclsh 的 `REG_ESPACE`）；组嵌套上限 250 |
| C16 | （第二批）ARITH 错误的 expr 帧关键词：tclsh 对**只含字面量**的表达式（编译期折叠）记 `invoked from within`（`expr {1/0}`、`expr {1%0}`、`expr {0**-1}`），对有运行期值参与的记 `while executing`（`expr {1/$z}`、`expr {sqrt(-1)}`）。rtcl 字节码引擎恒为 `while executing`、树引擎恒为 `invoked from within`——**两个引擎彼此都不一致**；语料与 sweep 都没有覆盖 | `interp/vm_exec.rs`、`types/expr.rs` | 未修：暂存 `judge/probes-known/smallface_p24_open.tcl` |
| C17 | （第二批）`lt`/`le`/`gt`/`ge` 被 rtcl 当作字符串比较运算符；tclsh 8.6 里它们不是运算符（`expr {1 lt 2}` → `invalid bareword "lt"`、`TCL PARSE EXPR BAREWORD`；只有 `eq`/`ne`/`in`/`ni`）。属于语言层面的取舍，需要决定对齐 8.6 还是 8.7+ | `types/expr_check.rs`、`parser/src/expr_compile.rs` | 未修：暂存同上 |
| C18 | （第二批）读未定义变量的 errorCode：tclsh 在 proc 里（编译期局部槽）是 `TCL READ VARNAME`，顶层是 `TCL LOOKUP VARNAME <名>`；rtcl 一律 `TCL LOOKUP VARNAME <名>`，而它两个"不带名"的形态（proc 内 `catch {set y $nosuch}`、数组元素）又落错了位置。语料没有覆盖 | `error.rs` `tcl_error_code` 及各 raise 点 | 未修 |
| C19 | （第二批）TclOO 的一批错误不带 errorCode：未知方法应为 `TCL LOOKUP METHOD <名>`（对象命令与 `my` 同）、`X does not refer to an object/class` 应为 `TCL LOOKUP OBJECT/CLASS X`、`can't create object …` 应为 `TCL OO OVERWRITE_OBJECT`、`superclass`/`mixin` 应为 `TCL LOOKUP CLASS`；方法之外调用 `next`/`self`/`nextto` 应报 `<命令> may only be called from inside a method` + `TCL OO CONTEXT_REQUIRED`（rtcl 报 `no next method implementation` 或 `invalid command name "self"`） | `commands/oo.rs` | 已修（M4）；`my <cloned> x` 等零散形态仍未对齐 |
| C20 | （第二批）调用深度上限的错误：rtcl 报 `maximum recursion depth exceeded` 且 errorCode 留旧值；tclsh 8.6.17 是 `too many nested evaluations (infinite loop?)` + `TCL LIMIT STACK` | `error.rs` `recursion_limit` | 已修（M4）：七个 raise 点收拢为一个构造函数 |
| C21 | （第二批）`interp alias {} 名 {} 目标 ?前缀…?` 转投 `alias` 时漏了命令词，**把目标名当成了新别名名**：`interp alias {} als {} tgt X` 建出的是名为 `tgt`、指向 `X` 的别名，`als` 不存在；查询未知名报错（tclsh 返回空）；`interp aliases` 不存在。别名目标只按裸键查表，`::tgt`、`ns::tgt`、ensemble 都 `invalid command name` | `commands/interp_cmd.rs`、`introspect.rs` | 已修（M4）：别名目标走统一分派链 |
| C22 | （第二批）`tailcall` 到非 proc 目标：没有该命令的日志帧（`x3 / while executing "p1"`，缺 `while executing "error x3"`），词被换成解析后的键，`-errorstack` 的 INNER 形态错。按 tclsh 机制重做：命令名在**帧的命名空间**里解析（`lookupNsPtr`）、词原样分发、由分派器补日志事件（`TEOV_Error`）。A–O 十五种形态的 errorInfo 与 tclsh 一致 | `interp/call.rs`、`eval.rs` | 已修（M4） |
| C23 | （第二批）`::oo::ObjN::my` 的全限定名只在"最内层帧在对象命名空间内"时可用，其余一律 `invalid command name`；tclsh 里它是对象的普通命令，任何作用域都能调（普通 proc、别的对象的方法里都不行） | `commands/oo.rs` | 已修（M4）：可见性 = 命令查找本身 |
| C24 | （第二批）`::tcl::prefix match`：歧义前缀（`l` 对 `list`/`length`）静默取第一个；`-message` 不进消息；`-error` 被忽略；选项不接受缩写；空表与二项表的 `must be` 文案不同 | `commands/misc.rs` `prefix_match` | 已修（M4）：按 `Tcl_GetIndexFromObjStruct` + `PrefixMatchObjCmd` 重写，40 余个形态与 tclsh 一致 |
| C25 | （第二批）`::tcl::Bgerror`：handler 失败时静默吞掉（tclsh 向 stderr 报三行），`::errorCode` 留下 `TCL LOOKUP COMMAND bgerror` | `commands/misc.rs` `cmd_tcl_bgerror` | 已修（M4） |
| C26 | （第二批）`::tcl::unsupported::assemble` 的 errorCode：`TCL OPERATION ASSEMBLE BADLABEL/BADINSTR` 或 NONE；tclsh 是 `TCL ASSEM BADSTACK`、`TCL ASSEM NOLABEL @x`、`TCL ASSEM DUPLABEL @x`、`TCL LOOKUP INDEX instruction <名>` | `commands/misc.rs` | 已修（M4） |
| C27 | （第二批）expr 解析错误的 `-errorstack` 沿用上一个错误的条目（没有重置、INNER 没落） | `interp/eval.rs` `eval_expr` | 已修（M4） |
| C28 | （第二批）proc 内 `namespace eval … { tailcall … }`：tclsh 报 `tailcall can only be called from a proc, lambda or method`，rtcl 照常执行 | `interp/call.rs` | 未修 |
| C29 | （第二批）rtcl 没有默认 `bgerror` proc（tclsh 的 init.tcl 提供）；`interp` 命令是 jimtcl 模型——裸 `interp` 创建子解释器并返回 `interp#N` 句柄，没有 `interp create/slaves/eval/delete/exists/expose/hide/hidden/invokehidden/limit/recursionlimit`（与 S25/S26 同源） | `commands/interp_cmd.rs` | 已修（M6）：`interp` 改为 tclsh 模型（见 S26）。别名 `interp alias 从 名 ?主路径? 目标 ?前缀…?` 的目标可以是父解释器（空路径）、自身或兄弟解释器，错误原样带回调用方（errorInfo 在调用处记 `invoked from within`）；跨解释器的别名环在建立与 `rename` 时拒绝（`cannot define or rename alias "x": would create a loop`）；删除目标解释器时一并删掉指向它的别名；解释器自己的命令被 rename 后仍可用。默认 `bgerror` proc 仍未提供（`::tcl::Bgerror` 在） |
| C30 | （M5）exec 的错误语义与 tclsh 不同：子进程写了 stderr（未重定向、未 `-ignorestderr`）tclsh 让 exec **报错**、消息 = stdout + stderr（`catch {exec …} msg` 取 stderr 是 Tcl 惯用法），rtcl 把 stderr 打到自己的 stderr 并返回成功；失败消息多了 `(exit code N)`、stdout 与原因之间缺换行；被信号杀死 tclsh 报 `child killed: <描述>` + `CHILDKILLED pid SIGNAME {描述}`，rtcl 当成退出码 1；成功时 rtcl 把 `::errorCode` 清成 NONE（tclsh 不动）；重定向失败的消息与 `POSIX ENOENT …` errorCode 不同；非 UTF-8 字节 tclsh 逐个映射成同值的 Unicode 字符。语料与探针里没有任何 exec 调用，所以一直没被看见 | `commands/exec_cmd.rs` `cmd_exec` | 已修（M5）：按 `TclCleanupChildren` 与实测重写了原生路径的参数扫描、管道搭建与收尾（探针 d19，42 条断言在 tclsh 上同样全过）。一并对齐：`2>@1` 两流共用一根管道（保持写出顺序）、重定向在 spawn 之前按命令行顺序打开、目标可连写在操作符后（`2>/dev/null`）、CR LF 与孤立 CR 按换行处理、开关不接受缩写、各语法错误的 errorCode、子进程继承调用方的 stdin、中途失败时已启动的子进程分离而不是杀掉 |
| C31 | （M5）`@通道` 重定向（`exec cmd 2>@ stderr`、`>@ $chan`、`<@ $chan`、`>>@`、`2>>@`、`>&@`）：tclsh 把通道的描述符传给子进程；rtcl 原来把这些词当普通参数传给命令（静默出错）。一度改成明确报错 | `commands/exec_cmd.rs`、`channel.rs` | 已修（M5）：`Channel::dup_for_child` 给出描述符的副本（文件、TCP、标准流、管道通道），文件通道的副本共享位置；未知通道 `can not find channel named` + `TCL LOOKUP CHANNEL`，方向不对 `channel "x" wasn't opened for writing/reading` + `TCL OPERATION EXEC BADCHAN`。已知差异：`>>@ chan` 时 tclsh 8.6.17 会**丢掉**子进程的输出，rtcl 追加在通道已有内容之后。带宿主 HostExec 的路径仍不支持（明确报错） |
| C32 | （M5）带宿主 `HostExec` 的 exec 路径（`cmd_exec_host`，注入了宿主的 wasm 环境走它）仍是旧语义：stderr 不使 exec 失败、`(exit code N)`、成功时清 `::errorCode`、`bad switch` | `commands/exec_cmd.rs` `cmd_exec_host` | 未修 |
| C33 | （M5）`file` 命令的零散缺口：`file attributes` 完全没有实现（`bad option "attributes"`）；`file normalize ~user` 不查用户（tclsh 报 `user "…" doesn't exist` / `TCL VALUE PATH NOUSER`，存在的用户展开到家目录）；`socket -server` 不支持（`server sockets are not supported in rtcl`） | `commands/io.rs`、`channel.rs` | 未修 |
| C34 | （M5）`file rename` 跨文件系统直接失败（`Invalid cross-device link`；tclsh 复制后删除）。一并对齐了的：目标 是目录时移入其中、源不存在与已存在的消息和 `POSIX` 码、`mkdir`/`copy` 的消息（原来是 Rust 的 `No such file or directory (os error 2)` 且没有 errorCode） | `commands/io.rs` | 部分已修（M5）；跨文件系统未修 |
| C35 | （M6）`tcl_hash`（Tcl 字符串哈希的移植）用普通 `+`：键长到 22 个字符即 u64 溢出。release 回绕（与 C 的无符号运算一致），debug 构建直接 panic——`array names env`、`unset env` 这类要给元素名排插入序的操作，在单元测试里一碰长环境变量名就崩 | `interp/tclhash.rs` | 已修（M6）：`wrapping_add` |
| C36 | （M6）`exec` 不带参数报 `wrong # args: should be "exec exec ?switches? arg ?arg ...?"`（命令名重复，用法文本过时）；只有重定向、没有命令时报 `no command given`，tclsh 是 `illegal use of | or |& in command` + `TCL OPERATION EXEC PIPESYNTAX` | `commands/exec_cmd.rs` | 已修（M6，原生路径）；宿主路径未动（C32） |
| C37 | （M6）`rename` 一个不存在的命令只报消息，不设 `::errorCode`（tclsh：`TCL LOOKUP COMMAND <名>`） | `commands/proc.rs` `cmd_rename` | 已修（M6） |
| C39 | （M6，实测）有效递归深度只有 tclsh 的一半到三分之一：`interp recursionlimit` 报 1000，但 rtcl 的 proc 递归每层还把命令替换、`if`/`foreach` 体计入嵌套深度（tclsh 内联编译的都不计，只有 proc 调用计一层）。二分实测的最大深度，tclsh / rtcl：`A [expr {$n-1}]` 996 / 496，`return [B [expr …]]` 996 / 496，`foreach` 体内递归 996 / 330，`eval {E …}` 498 / 331（rtcl 两引擎一致）。经别名重入时两引擎不一样：`r 400` 字节码引擎通过，树引擎到上限 | `interp/eval.rs` `enter_nested`、`call.rs` | 未修（记录）：两个引擎都用 Rust 递归求命令替换，这个计数同时是栈溢出的防线；要对齐得先把嵌套上限与调用深度分开计量，并确认每层的栈用量 |
| C38 | （M6，已知差异）别名目标的 wrong-args 消息里的命令名：tclsh 自己不一致（目标是父解释器时写别名名 `pc varName ?newValue?`，是兄弟解释器时写目标名 `set varName ?newValue?`），rtcl 一律写目标名；跨解释器别名的 `-errorstack`（TIP 356）INNER 条目与 tclsh 不同；安全解释器没有 `::tcl::mathfunc::min/max` 别名（rtcl 这两个是内建，`interp aliases` 只列 `clock`） | `commands/interp_cmd.rs` | 未修（记录） |
| C40 | （M7）数组命令用绝对名字时落空：`array size ::a` 给 0、`array get ::a` 给空（tclsh 2 和 `1 x 2 y`）；proc 里 `array unset ::a 1` 什么也没删 | `interp/vars.rs` `array_owner` | 已修（M7）：`::a` 按全局表里的键解析 |
| C41 | （M7）`array statistics` 的桶数、直方图、平均查找距离是写死的假数（「N entries, N buckets，每个桶 1 个」） | `commands/array.rs` | 已修（M7）：按元素表的插入历史重放（tclsh 的 `Tcl_HashStats`）：删元素后桶数不缩、数组删掉重建从头算 |
| C42 | （M7）`info locals`：unset 后重建的局部变量被列两次（tclsh 一次、在原位）；带 `args` 的 proc 把 `args` 列在最前（tclsh 在最后，即参数顺序）；`upvar`/`global`/`variable` 链接进来的名字被当成局部变量列出（tclsh 一个都不列，`info vars` 才列） | `commands/misc.rs`、`interp/call.rs` | 已修（M7） |
| C43 | （M7）`apply` 的 lambda 第三个元素指向不存在的命名空间：消息对，没有 errorCode（tclsh `TCL LOOKUP NAMESPACE ::x`） | `commands/proc.rs` `cmd_apply` | 已修（M7） |
| C13 | 修复过程中新查出：`json encode` 的 schema 文本按词递归（`list list list …` 重复几万次即栈溢出），JSON 解码的递归下降无深度上限 | `commands/json.rs` | 已修（M2）：文档与 schema 的嵌套上限 1000，超限报错 |

## P2：性能（`TODO.md` 未列，本机单次实测，只看量级）

| # | 问题 | rtcl | tclsh | 位置 |
|---|---|---|---|---|
| P1 | TCP 读每字节一次系统调用（3MiB） | 387ms | 5ms | `channel.rs` |
| P2 | `string map` 每字符重建剩余串（15 万字节） | 7.1s | 8ms | `commands/string_cmds.rs` | 已修（M7）：一趟扫描，按字节匹配；64KB 从 1.0s 到 0.7ms（tclsh 0.8ms）。顺带：奇数个元素的 map 给 `TCL OPERATION MAP UNBALANCED`，不是列表的 map 报列表错误（原来静默当成一个词）。探针 d22（28 条断言） |
| P3 | `append` 每次两份整串拷贝（8 万次） | 4.7s | 10ms | `commands/misc.rs` |
| P4 | `info vars` 超线性（8 万变量）；`TclTable::insert` 每次全扫桶 | 13.2s | 整段 11ms | `interp/tclhash.rs` | 已修（M7）：`info vars` 对每个列出的名字再线性搜一遍条目表（平方），`info commands` 同一处；插入时不再全扫桶；顺序重放改为借用键（不再每个键拷三次）。`info vars` 4 万变量 0.9s → 约 0.02s |
| P5 | `array size` 按作用域变量总数计 | 2.6s | ≈0 | `commands/array.rs` | 未修：`array size`/`exists` 仍扫整张变量表（8 万元素 0.8ms，tclsh 是常数） |
| P14 | （M7）每个作用域的变量插入日志只增不减：循环里建了再删的变量永远留在日志里，内存和每次枚举的耗时都随之增长。三轮各 30 万次「建再删」后常驻 96MB（tclsh 5MB）；`array names` 对日志的每个条目做一次哈希查找，2 万元素 40ms、8 万元素 624ms | 96MB / 624ms | 5MB / 0.2ms | `interp/vars.rs` `var_order` | 已修（M7）：日志长到活跃键的两倍以上就收回到活跃键（全局表留最后一次出现，帧内的普通局部变量留第一次——对应 tclsh 的槽位序）；日志里没有失效条目时枚举直接顺着读，不做查找。7.8MB；2 万元素 1.1ms，8 万 5ms。探针 d23（45 条断言） |
| P6 | 变量哈希不扩散，同前缀名退化 | 超线性 | 线性 | `interp/varmap.rs` |
| P7 | 编译期 O(n²)（2 万个 `for`） | 4.6s | 0.10s | `parser/src/compiler.rs` |
| P8 | 正则捕获组量词立方级；相邻变长捕获组 `(a+)(b+)` 是 O(n³)（400 字符单次 20ms、800 字符跑不完，基线版本 400 字符即挂死）；lookahead 慢约 9 倍；每次调用重新编译模式（无捕获 7–12µs、带捕获 12µs 对 tclsh 0.5µs） | 2.8s / 20ms / 12µs | 33ms / 7µs / 0.58µs | `regexp-spencer/src/captures.rs`、`commands/regexp_cmds.rs` |
| P8a | （已修，M2）上行的前三项：定长操作数按算术数迭代；切分点由一次锚定 NFA 模拟给出而不是逐个盲试；定长部分只有一个候选切分点 | 20ms / 2.8s | 7µs / 33ms | 同上 |
| P9 | 文件读取逐字节（32MB） | ≈0.6s | 58ms | `channel.rs` |
| P10 | 树引擎对嵌套立方级，且无条件编译 | >10s | 0.2s | `interp/eval.rs` |
| P11 | 每次创建子解释器约 460KB / 3.4ms | — | — | `commands/interp_cmd.rs` |
| P12 | JIT 每次命令分派按行号 memcpy；模块池满即全清（读码） | — | — | `rtcl-jit/src/exec.rs` |
| P13 | Tk：每次像素操作重编码整图；text 索引 O(n)（读码） | — | — | `rtcl-tk-runtime/src/lib.rs` |

## 核实为稳妥的部分

- 默认正则引擎对 `(a+)+$`、`(a|aa)+$`、`(a*)*b` 线性，无经典 ReDoS。
- JS 回调抛异常转成 Tcl 错误，句柄 afterward 可用；回调里重入 `exec`
  被 wasm-bindgen 借用检查拒绝（不是内存安全问题）。
- wasm 构建里 `exec` / `env` / `socket` / `signal` / 原生 `load` 被正确
  排除；无宿主 FS 桥时读不到任何文件。
- 约 1.5 万次边界值模糊探测中 string/list/dict/binary/scan/clock 面无
  崩溃（`format` 精度除外）。
- OO 派发缓存 13 项时效探针全部正确；`lsort -command` 与 dict 迭代的
  重入安全；freelist / COW / iconv FFI 审计无问题；TLS 校验默认开启；
  `exec` 不过 shell，不用临时文件。

## 架构改进方向

见 [TODO.md](../TODO.md)「审计后续」节与各设计的单独文档；一句话版：

1. **统一的栈预算**替代逐点计数器：所有脚本驱动的重入汇到一个
   `enter()` 检查点，按目标栈大小校准；数据驱动的递归（序列化、Drop、
   JSON、正则 dissect）改迭代。
2. **`InterpLimits`**（步数 / 深度 / 内存）带 wasm 默认值，大分配走
   `try_reserve` 并返回 Tcl 错误。
3. **能力模型**：真正的隐藏命令表 + per-instance 令牌 + 子解释器继承。
4. **工具化防线**：`judge/crash-gate.sh` 入门禁，对 `rtcl-vm` 开 clippy
   的 `unwrap` / 索引 / 算术禁用规则，panic 普查做棘轮。
5. **分派结构**：把每个 op 前的动态检查（epoch、顶层槽分类）换成
   编译期插入的显式 op 与入口检查；之后再把 `Vec<OpCode>` 换成数字
   码表 + 紧凑指令流（见 TODO「op 码表」）。
7. **探针本身也会坏**：`judge/probes/smallface_p24.tcl` 开头就读未初始化
   的 `::errorCode`，在 tclsh 和 rtcl 上都死在第一行——两个解释器报同一个
   错即算"通过"。它钉着的 `::tcl::prefix`、ARITH 首帧、`interp alias`、
   表达式里的 `#`（死循环）因此从未被检查过（见「第二批」）。

## 门禁基线（2026-11-24，HEAD c76ae3b 之上）

- `judge/run.sh`：88/88。
- `judge/probes.sh`：active 全绿。
- `judge/sweep.sh`：**12 处双引擎分歧，在 HEAD 上原本就有**（去掉
  审计修复后重建，名单一字不差：`ei_line_attribution`、
  `expr_bracket_control`、`expr_bracket_errors2`、`expr_bracket_multiline`、
  `incr_window_p25_errline`、`loop_body_err_line{,3,4}`、
  `sub_inline_errors1`、`sub_inline_if_body`、`sub_inline_proc_frames`、
  `we5`）。全部是 errorInfo 行号/帧归属在两引擎间的差异，来自
  2026-11-23 的 errorInfo 提交；文档里的"sweep 0"在 `perf/wave2` 上
  已不成立。审计修复未增减该名单。
- `judge/crash-gate.sh`：基线 99 次运行 14 过；M1 后 40 过
  （a 类 24/24 全过）；M2a/M2b 后 60 过（b01–b10 全过）；M2c/M2d 后
  78 过（b 类 21 个全过）；M3 后 100 次运行 79 过（新增 a13）；M4
  后 **108 次运行 103 过**——d 类 16 个全过，新增 c06（表达式里的
  `#`）与 d15–d18（tailcall、`my` 可见性、`interp alias`、TclOO
  errorCode）。余下 c 类 5 个（原生 OS 面的挂起，M5）。
- M5（exec）：c01–c03 转绿；c01 的期望从"不挂"改成 tclsh 的行为
  （stderr 输出使 exec 报错）；新增 d19（exec 错误语义，42 条断言）。
  `d01` 的预算放宽到 90 秒——它是编译上限的压力探针（超大单元走
  树引擎，负载高的机器上每个引擎 5–7 秒），不是挂起探针，20 秒
  默认预算在负载 20+ 时抖过。
- M5 收尾：**crash-gate 110 次运行 110 过**——四类探针首次全绿
  （基线 99 次 14 过）。顺带修了门禁的一处测量假象：输出原来用
  `$(…)` 捕获，脚本留下的子进程继承了被捕获的 stdout/stderr，shell
  要等它退出才读到 EOF，于是计的是子进程的命而不是探针的（`exit`
  旁的 `open "|sleep 3"`：解释器 30ms 就退了，门禁量出 3 秒）。现在
  输出落文件。
- M5 文件操作：S34–S37 全部修掉，新增 d20（33 条断言，tclsh 上同样
  全过）。crash-gate 至此 **112 次运行 112 过**。
- M7（性能）的量法：`target/audit/bench/` 下的微基准（循环、分配、变量、
  正则、编译），tclsh 与两个引擎并排。字节码引擎在纯循环和 proc 调用上
  与 tclsh 相当或更快（空 `for` 快 2.4 倍、`foreach` 快 3.5 倍），差距
  集中在：`string map` 3345 倍（已修）、`info vars` 7633 倍（已修）、
  固定模式的 `regexp` 198 倍、`string index` 51 倍、编译 30–66 倍、
  数组读写 3–4 倍、`append` 3 倍；峰值内存 108MB 对 23MB。
- M6（能力模型）：`interp` 命令族按 tclsh 源码（`tclInterp.c`）重做，
  对照探针 315 行逐项与 tclsh 8.6.17 比过；剩余差异只有哈希表顺序、
  rtcl 自带的命令集合和四处有意保留的差别（见 S26、C38）。探针 d21
  （295 条断言，期望值由 `_gen_d21.tcl` 跑 tclsh 生成）使 crash-gate
  到 **114 次运行 114 过**。单元测试顺带抓到 C35。
- M6 收尾：`judge/crash-wasm.mjs` 39 → **52 项**全过（隐藏命令不可调、
  `-safe` 的隐藏与拒绝、别名环被拒、任意两个解释器之间的别名、孙解释器
  的别名、目标被删、错误原样带回调用方、求值中不可删、rename 后的
  解释器命令）。原来用裸 `[interp]` 建子解释器的两项检查改用
  `interp create`——裸 `interp` 现在是 tclsh 的 arity 错误。
- M6（wasm 隔离）：`judge/crash-wasm.mjs` 35 → **39 项**全过（跨实例
  `_js_dispatch`、不可拼写的注册名、子解释器继承宿主服务且不 trap、
  步数预算覆盖子解释器）。
- `judge/crash-wasm.mjs`：**35 项**全过（M3 时 27 项）。每项在自己的
  worker 线程里跑，15 秒无结果即判 `HANG` 并终止线程——原来遇到挂起类
  回归是门禁自己卡死（步数预算看不见解析器/检查器里的循环）。
- 对照基线：10 万层嵌套 list 的销毁 8–13ms → 4–5ms（迭代路径反而更快）；
  宽列表（200 万元素）与字符串列表的销毁无变化；1.6 万层嵌套 list 上
  25 个命令里基线有 17 个栈溢出，现全部正常返回。
- 嵌套 `{2}` 量词正则（100 字符）：3GB 后 abort → 约 40ms 内报
  `couldn't compile regular expression pattern: out of memory`（同 tclsh）。
- 编译：常量池加了哈希索引（原来每次插入线性扫描，7 万个字面量的
  编译是二次方的）。单元里交给 u16 操作数的常量/局部槽/参数个数超过
  65535 时整个单元回落树走（原来静默截断：`p PARAM` 返回
  `65535 65536`）。已知限制：这种超大单元在树走上较慢（7 万条命令
  约 4.5 秒）——操作数加宽形式属于 ISA 重写范围。
- 带捕获的匹配：20 万次 `regexp {(\w+)-(\d+)}` 1872ms → 2325ms（基线
  每次较慢是另一形态）——单次 94µs → 12µs；`regexp -inline {(a+)(b+)}`
  400 字符：基线跑不完 → 26µs/次；24 个对照用例结果与 tclsh 逐字一致。
- 已知限制（与 tclsh 的对象模型相同，非本次引入）：嵌套 list 的字符串化
  是 O(n²) 内存——每层缓存自己的字符串。4 万层时 rtcl 峰值 3.2GB
  并正常完成；tclsh 涨到 17.5GB 后段错误。
- 单元测试：`cargo test -p rtcl-vm -p rtcl-parser -p rtcl-ir -p rtcl-core`
  全绿。

## wasm 侧的实测与结论（M3，Node 22 / V8，2026-11-24）

- **真正的栈约束是 JS 引擎的调用栈**（约 1MiB；V8 把 wasm 帧放在 JS
  栈上），不是线性内存。加大 `-zstack-size` 不提高递归深度，只增加
  线性内存——试过，已撤回。
- 每层递归的栈耗量随调用形态变化：最简单的 proc 递归 736 层溢出
  （约 1.4KB/层）；proc 内循环 664；`try`/`finally` 364–392；OO 方法
  与 `uplevel` 420；`apply` 356；`catch` 嵌套 268–284；`switch` 内递归
  252；`apply` 套在 `catch` 里 **200**（约 5.2KB/层）。原默认上限 1000
  对每一种都越过栈。
- 处置：**wasm32 的默认递归上限改为 150**（最坏实测形态的 1.3 倍
  余量），原生保持 1000（各形态在 999 层都需要约 4MB 栈，默认 8MB
  主线程栈有 2 倍余量；叠加全部重构造的形态也实测通过）。宿主可经
  `RtclHandle::set_recursion_limit` / `Interp::set_max_call_depth` 调整；
  **栈比主线程小的宿主（子线程、嵌入式小栈线程）必须按栈下调**。
- 深度计数看不见每层的重量：按栈指针的实际余量做守卫是更稳的方案，
  但 wasm32 上读不到引擎栈指针，只能计数。列为后续项。
- `RtclHandle` 新增 `set_step_budget` / `set_recursion_limit`；
  `timerate {} <ms>` 与 `::tcl::unsupported::assemble` 的空转循环
  原来绕过步数预算，现按迭代/指令计步。
- 新门禁 `judge/crash-wasm.mjs`（Node 里对 wasm 产物跑同类缺陷，
  每项一个全新实例）：27 项全过；`crates/rtcl-wasm/test.js` 26/26。

## 第二批（2026-11-24 夜）：门禁暴露的问题

第一批 M4 的全量门禁抓到一个**自己引入的回归**：`judge/probes/oo17`
（方法体内 `tailcall my go …` → `invalid command name "::oo::ObjN::my"`）。
顺着它查出上面 C16–C29 一串既有缺陷。做法与第一批相同：先把 tclsh
的行为和机制查清，再按机制实现，不迁就探针。

- **依据是 tclsh 源码**（core-8-6-branch，只读）：`TclNRTailcallEval`
  把命令词的**查找**放进 tailcall 所在帧的命名空间（`lookupNsPtr`），
  词原样分发；`TEOV_Error` → `TclLogCommandInfo` 为按词调用的命令补
  日志事件（词的 list 形式；errorInfo 为空时 `while executing`，否则
  `invoked from within`；raise 点已记过则不记），同时重置
  `-errorstack`。rtcl 对应为 `Interp::cmd_lookup_ns`（一次性查找
  命名空间覆盖）+ `Interp::cmd_lookup_in`（运行中的命令是在哪个命名
  空间被找到的——`my`、对象命令要再解析一次自己的词）+
  `Interp::log_tailcalled`。
- **`my` 的可见性**：rtcl 有一道"最内层帧在对象命名空间内"的守卫，
  在 tclsh 里没有依据——6 个合法形态里基线错了 4 个。去掉后可见性 =
  命令查找本身（`qualify(当前命名空间, 词)` 命中对象）。
- **探针 `judge/probes/smallface_p24.tcl` 是死的**：第一行就读
  `::errorCode`（尚未初始化），tclsh 与 rtcl 同样报错，于是"一致"。
  修活后它逐项暴露了 C16–C17、C19、C21、C24–C27 与 S41；除 C16/C17
  外全部当场修掉，这两项暂存
  `judge/probes-known/smallface_p24_open.tcl`。
- **工具教训**（与「判官为什么没抓到」同类）：用"两个解释器输出相同"
  做门禁时，共同的早期失败等于空转。修活的探针现在 53 行、与 tclsh
  逐字节一致。
- **同类空转的规模（已排查，未修）**：`judge/probes/` 的 437 个探针里
  有 57 个在 tclsh 上以未捕获错误结束，其中约 30 个在文件末尾之前就
  中断——后面的用例在两个解释器上都没跑过（启发式统计，少数几项的
  "后续行"其实是同一条多行命令的续行）。中断点最靠前的：
  `trv_d22`（第 1/8 行）、`sub_inline_multiline`（1/3）、`ei1`（2/24，
  15 行未执行）、`p32`（2/12）、`p29`（3/10）、`probe6`（10/28，
  18 行未执行）、`oo19`（41/56，15 行）、`namespace_uplevel`（13/25，
  12 行）；其余：`c5_l2`、`ens`、`globprobe`、`ix1`、`ix2`、
  `oo1`、`oo8`、`p21`、`p25`、`p28`、`p47`、`p48`、`pidx`、`pidx2`、
  `pidx4`、`r4reg2`、`scan_numeric2`、`scan_validator_edges`、
  `trv_p1`、`utf_char_classes`、`namespace_varmodel`。逐个修活是独立
  的一批工作——`smallface_p24` 修活一项就带出 7 类缺陷。
- **已知未修并登记**：C16（且两引擎不一致——sweep 语料里加上 ARITH
  形态就会红，需先修）、C17、C18、C28、C29。

## 已剔除 / 未能复现

- 「JS 回调重入是 UB」：实测被借用检查拒绝，剔除。
- 「嵌套协程上下文单槽丢失」「协程挂起后 `next` 失效」：当前二进制上
  VM 引擎行为正常，未能复现。
