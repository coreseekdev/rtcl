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
   启停；没有 safe interp；dynmod 分派不校验实例身份（S24–S27）。
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
| S11 | `namespace unknown ::nosuchhandler` 后调用未知命令 | `interp/eval.rs` | 处理器自指递归 | 未修 |
| S12 | ensemble `-unknown` 映射到不存在的子命令 | `commands/namespace.rs` | 重分派不计深度 | 未修 |
| S13 | `alias a a; a` | `commands/introspect.rs` | 直调函数指针绕过深度检查 | 未修 |
| S14 | 一串 write-trace 互相写下一个变量 | `interp/vars.rs` `fire_traces_inner` | 仅 VM 引擎；树引擎正常报错 | 未修 |

## P0：深度与大小无上限

| # | 触发 | 位置 | 说明 | 处置 |
|---|---|---|---|---|
| S15 | 嵌套 list/dict 约 1.3 万层转字符串；16 万层的 list `unset` | `value.rs` `serialize_list` / `drop_last_owner` | 序列化与 Drop 都是原生递归；wasm 5000 层即中 | 未修 |
| S16 | `json decode` 约 1.5–2 万层嵌套 | `commands/json.rs` | 纯数据驱动，输入几十 KB；wasm 约 5000 层 | 未修 |
| S17 | `expr` 非花括号 4000 层括号、5 万个连续 `-` | `types/expr.rs`、`expr_check.rs`、`parser/src/expr_compile.rs` | 三套独立递归下降都无深度概念 | 未修 |
| S18 | proc 内 2 万层嵌套 `[…]` | `parser/src/compiler.rs` | 不调用也崩 | 未修 |
| S19 | 正则：6000 层嵌套括号；`((a{255}){255}){255}`；`(.*)x` 匹配 3 万字符 | `regexp-spencer` `parser.rs` / `captures.rs` | 后两项吃满近 3GB 后 abort | 未修 |
| S20 | proc 递归 650–1000 层 | `interp/mod.rs` `max_call_depth` | 上限 1000 没按栈大小校准：wasm 必崩且毒化实例，原生栈 ≤3MB 也崩 | 未修 |
| S21 | `lrepeat 4294967296 x`、`string repeat x 4294967296` | 同 S9 | 分配失败 = abort，不可 catch；tclsh 报 Tcl 错误 | 未修 |
| S22 | `eval eval … 1` 嵌套 2000 层 | `interp/eval.rs` | 输入 1 万字节，报递归错误前已占 306MB | 未修 |

## P0：wasm 专有（Node 22 + 11-20 产物实测）

| # | 现象 | 位置 | 处置 |
|---|---|---|---|
| S23 | 栈溢出 trap 后整个实例永久失效：同实例 `new RtclHandle()` 也崩，GC 终结器再触发 trap 可杀死 Node 进程；panic 则使当前句柄永久失效 | `rtcl-wasm/src/lib.rs` `exec` | 未修 |
| S24 | 句柄 A 的脚本 `_js_dispatch 1 <名字>` 可调到只在句柄 B 注册的宿主回调并拿到返回值 | `rtcl-wasm/src/js_command.rs` | 未修 |
| S25 | 子解释器不继承宿主服务：`$h eval {time …}` trap，`puts` 输出静默丢失 | `commands/interp_cmd.rs` | 未修 |

## P1：隔离与资源治理

| # | 问题 | 位置 | 处置 |
|---|---|---|---|
| S26 | 没有 safe interp；`interp create -safe` 报错，子解释器带全部命令 | `commands/interp_cmd.rs` | 未修 |
| S27 | `hide_command` 只从 `info commands` 隐身，脚本照常可调 | `interp/mod.rs` | 未修 |
| S28 | 步数预算默认无限，CLI 与 wasm 都没暴露；`timerate {} N`、`::tcl::unsupported::assemble` 可旁路 | `interp/mod.rs` | 未修 |
| S29 | 环境含非 UTF-8 变量时 `Interp::new()` panic | `interp/mod.rs` | 未修 |
| S30 | release `panic = "abort"`，全仓无 `catch_unwind` | `Cargo.toml` | 未修 |

## P1：原生 OS 面的挂起与文件安全

| # | 问题 | 位置 | 处置 |
|---|---|---|---|
| S31 | exec 三处死锁（stderr 超 64KiB 且 stdout 未关、`<<` 超管道容量、后台作业写大量 stderr）；`exit` 被存活的管道子进程阻塞 3 秒 | `commands/exec_cmd.rs` | 未修 |
| S32 | `close` 仍在连接的 `-async` socket 无限阻塞 | `channel.rs` | 未修 |
| S33 | exec 遇非 UTF-8 输出报错并留僵尸进程 | `commands/exec_cmd.rs` | 未修 |
| S34 | `file tempfile` 任意路径前缀、无 `O_EXCL`、0644 | `host.rs` | 未修 |
| S35 | `file delete` 不带 `-force` 也递归删非空目录 | `commands/io.rs` | 未修 |
| S36 | `file normalize a/../x` 原样返回 | `commands/io.rs` | 未修 |
| S37 | `-async` socket 的 fd 无 `CLOEXEC` | `channel.rs` | 未修 |
| S38 | HTTP 客户端默认无超时、响应体无上限、无 URL 审查钩子（读码） | `rtcl-native/src/net.rs` | 未修 |
| S39 | 变量表无密钥哈希，可预构造冲突键（风险低） | `interp/varmap.rs` | 未修 |
| S40 | Tk：畸形 PNG、多字节颜色串、`image put -to` 负坐标 panic（读码，未运行） | `rtcl-tk-runtime/src/lib.rs` | 未修 |

## 静默出错（不报错，结果是错的）

| # | 现象 | 位置 | 处置 |
|---|---|---|---|
| C1 | 一个编译体内 ≥65536 个常量/局部变量/参数时操作数 `as u16` 截断；`p PARAM` 返回 `65535 65536` | `parser/src/compiler.rs`、`rtcl-ir/src/bytecode.rs` | 未修 |
| C2 | 回溯预算耗尽后 `regexp {a+()\1x}` 对 2000 个 `a` 返回“不匹配” | `regexp-spencer/src/backtrack.rs` | 未修 |
| C3 | `set g 0; upvar 0 g q; set q 5` 之后 `$g` 仍是 0 | `interp/vm_exec.rs` 顶层槽边界 | 未修 |
| C4 | 命令中途重定义后再注册的 write-trace 永不触发 | `interp/vm_exec.rs` tier1 重分派 | 未修 |
| C5 | `+ 9223372036854775807 1` 得负数，`expr` 正确宽化 | `commands/misc.rs` | 已修（M1） |
| C6 | `tailcall` 不做命名空间解析 | `interp/call.rs` | 未修 |
| C7 | `coroutine set …` 覆盖内建后已编译 proc 仍用内联 `set` | `commands/coro.rs` | 未修 |
| C8 | `oo::objdefine $o unexport m` 对继承方法无效 | `commands/oo.rs` | 未修 |
| C9 | 析构器内 `C destroy` 使析构器被跑约 500 遍 | `commands/oo.rs` | 未修 |
| C10 | 旧式八进制 ≥2^63 回绕；大写十六进制 bignum 判为非数字 | `value.rs`、`types/bignum.rs` | 已修（M1） |
| C11 | `wait` 对已退出后台进程返回 127，`-nohang` 恒为 NONE | `commands/os.rs` | 未修 |
| C12 | 修复过程中新查出：`set v 018; expr {$v + 0}` 得 18（tclsh 报 invalid octal）；超过 64 位的旧式八进制被当作十进制；带 bignum 操作数的浮点运算取错值。根因是 Tcl 整数字面量语法有**四套**各自独立的实现（`value.rs`、`bignum.rs`、`expr.rs` 两处、`list.rs`），对旧式八进制理解不一 | `types/bignum.rs` | 已修（M1）：合并为单一的 `bignum::parse_int`，四处改为调用它 |

## P2：性能（`TODO.md` 未列，本机单次实测，只看量级）

| # | 问题 | rtcl | tclsh | 位置 |
|---|---|---|---|---|
| P1 | TCP 读每字节一次系统调用（3MiB） | 387ms | 5ms | `channel.rs` |
| P2 | `string map` 每字符重建剩余串（15 万字节） | 7.1s | 8ms | `commands/string_cmds.rs` |
| P3 | `append` 每次两份整串拷贝（8 万次） | 4.7s | 10ms | `commands/misc.rs` |
| P4 | `info vars` 超线性（8 万变量）；`TclTable::insert` 每次全扫桶 | 13.2s | 整段 11ms | `interp/tclhash.rs` |
| P5 | `array size` 按作用域变量总数计 | 2.6s | ≈0 | `commands/array.rs` |
| P6 | 变量哈希不扩散，同前缀名退化 | 超线性 | 线性 | `interp/varmap.rs` |
| P7 | 编译期 O(n²)（2 万个 `for`） | 4.6s | 0.10s | `parser/src/compiler.rs` |
| P8 | 正则捕获组量词立方级；lookahead 慢约 9 倍；无已编译模式缓存 | 2.8s | 33ms | `regexp-spencer/src/captures.rs` |
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
  （a 类 24/24 全过）。
- 单元测试：`cargo test -p rtcl-vm -p rtcl-parser -p rtcl-ir -p rtcl-core`
  全绿。

## 已剔除 / 未能复现

- 「JS 回调重入是 UB」：实测被借用检查拒绝，剔除。
- 「嵌套协程上下文单槽丢失」「协程挂起后 `next` 失效」：当前二进制上
  VM 引擎行为正常，未能复现。
