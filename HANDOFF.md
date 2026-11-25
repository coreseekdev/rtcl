# rtcl 工程 Handoff — 多轮修正 → JIT 全任务交接

> **状态横幅（2026-11-25）**：对标版本改为 **Tcl 9.1.0**（oracle 是仓内
> 构建的 `.refer/tcl910/inst/bin/tclsh9.1`；`JUDGE_ORACLE=tclsh` 指回
> 8.6.17）。2026-11-23..25 做了一轮代码审计，按清单修了 22 个提交
> （91bb15c..1da7ebe，`perf/wave2`，没有 push），清单与逐项状态在
> `specs/CODE-AUDIT-20261123.md`；**没修完的任务和架构评审见下面
> 「审计后续轮」一节**（任务包、9.1 对这一轮的影响、评审意见、坑）。迁移本身见 `TODO.md`
> 「对标迁移」。当前待办见 `TODO.md`；性能台账见 `bench/BASELINE.md`；
> 分歧台账见 `specs/DIVERGENCES.md`。本文件 §1 基础设施/§2 方法论
> 仍然有效；§3–§6 的数字是 2026-10 的历史快照（当前数字以 PRODUCTION
> 与 BASELINE 为准）；JIT 保持暂停（2026-10-17 内存管理/编译管线设计
> 定稿 = 路线 C，见 `docs/JIT-WASM-GC.md`，实现仍暂停）。
>
> 最后更新：2026-11-25（横幅与「审计后续轮」一节）；其余各节按各自日期。
> 用途：任一 agent 读本文件即可接管全部剩余工作，无需对话历史。

## 0. 项目与仓库

- 仓库：`git@github.com:coreseekdev/rtcl.git`，master 工作树 = `/home/nzinfo/src.note/rtcl-master`（持久盘；2026-10-03 从 /tmp tmpfs 迁出——/tmp 重启即失，工作成果必须及时 commit+push），分支 `master`。
- 定位：Rust 实现的 Tcl 解释器（jimtcl 血统的轻量定位），target = native + **wasm32-unknown-unknown（预期主路径）** + wasm32-wasip1 + embedded no_std。
- crate 结构：`rtcl-parser`（递归下降解析 + ByteCode 编译）、`rtcl-ir`（OpCode 三层：primitive / `Call(CmdId)` 0..127 stdlib 128+ 扩展 / `DynCall`）、`rtcl-vm`（execute.rs dispatch 循环 + `VmContext` trait + Value/Error 唯一实现）、`rtcl-core`（Interp + 内建命令，`value.rs`/`error.rs` 只是 re-export）、`rtcl-cli`、`rtcl-wasm`、`rtcl-expect`、`rtcl-jit`（ByteCode → wasm 发射器，JIT M0 起）。
- Tcl 官方源码（行为 oracle 的测试语料）：`/home/nzinfo/src.note/rtcl/.refer/tcl`（已 gitignore；如被删重新 `git clone --depth 1 https://github.com/tcltk/tcl.git .refer/tcl`）。
- 系统 oracle：`/usr/bin/tclsh` = Tcl 8.6.17。**一切语义以 tclsh 实测为准，不凭记忆。**
  **2026-11-25 起 oracle 是 Tcl 9.1.0**：仓内构建的
  `.refer/tcl910/inst/bin/tclsh9.1`，源码在 `.refer/tcl910/tcl9.1.0/`；
  8.6.17 只用来跑历史基线。

## 1. 基础设施（已建，勿重建）

| 资产 | 路径 | 用法 |
|---|---|---|
| judge parity harness | `judge/run.sh` | `./judge/run.sh`（全量）/ `-v <name>`（单文件带 diff）。递归 corpus/ 子目录，15s timeout/文件 |
| 手写语料 | `judge/corpus/*.tcl` | 10 个，全绿 |
| 官方测试集语料 | `judge/corpus/gen/gen_*.tcl` | 77 文件 / 4072 case，由提取器生成 |
| 提取器 | `judge/extract/extract.py` | 保守提取 tcltest 用例，oracle 自验证；勿轻易改协议 |
| burndown 报表 | `judge/extract/burndown.py` → `judge/BURNDOWN.md` | 生成物，不入 git |
| allium 行为规格 | `specs/*.allium` | 6 域：interp/expr/list/string/variables/control-flow |
| 分歧清单 | `specs/DIVERGENCES.md` | 格式：行为声明 \| Tcl 8.6 \| rtcl \| 复现脚本 \| 严重度；修复后标 `[FIXED YYYY-MM-DD]` |
| 对拍探针 | `specs/probe.sh` | stdin 逐行 Tcl 片段，自动 tclsh vs rtcl diff |
| 性能基线 | `bench/run.sh` → `bench/BASELINE.md` | rtcl vs tclsh best-of-N；微套件 0.6–1.0×（arith/var_incr/string_build 反超），重型 1.05–2.26× |
| 单测 | `cargo test --workspace` | 1143 个，必须保持全绿 |

## 2. 方法论（两条 skill 的硬规则）

- **code-migration**（`~/.claude/skills/code-migration/`）：judge 先行，任何重构/优化不许改变 judge 判据；失败三次即规则 bug（停手改规则，不修实例）；进度报 burndown 数字不报散文。
- **allium**（`~/.agents/skills/allium/`）：spec 只写可观察行为；分歧必须实际运行佐证；distill（从代码提取 spec）→ weed（spec vs 实现分歧）循环。
- **修复纪律**：以 tclsh 实测为最高裁判；最小侵入；不顺手重构；不引入新依赖（rtcl 是嵌入式定位）；子任务不 git commit，由协调者统一提交推送；并行 agent 严格文件互斥。

## 3. 当前状态（burndown）

- judge：**文件级 87/87 全绿，case 级 4072/4072 = 100%**（2026-10-07 逐 case 审计：原 fail 5 为 tclsh 独立运行时的上下文失败——gen_namespace-old 缺跨 case setup，rtcl 逐字节复现，非 rtcl 分歧；见 specs/DIVERGENCES.md"非分歧"节）。
- 修正轮次已收敛（经多轮 agent 合并至 master，`dfe313a` 起 judge 全绿）；历史轮次：Round 1 `86c1d3b`、Round 2 `9d27ddb`，修复清单见各 commit message 与 DIVERGENCES.md 的 [FIXED] 标注（28 条）。
- 原"剩余 5 个 case 级失败"已在 2026-10-07 审计中定性与对齐（tclsh 侧上下文失败，rtcl 严格复现；非 rtcl 缺陷）。
- **解释器快车道已落地（2026-10-02，`58c2839`..`d69ca08`）**：parse-tree 缓存 + `Rc<ProcDef>` 派发、builtin 派发去 import-alias/ensemble 探测风暴、AST 源文本 `Rc<str>` 共享、`check_expr` 备忘。judge/case 级无回退；数字见 `bench/BASELINE.md` 第二张表。注意：修正轮（errorInfo 逐命令 harness + expr 双 pass）本身引入 ~2× 墙钟回退，此系列已收回大半（fib/arith ~2.3×/2.1×）；剩余差距主因 = 逐命令 harness 与 expr 解释，Phase 3/JIT 路线均绕开。快车道路线细节（vm_exec 方案 = rtcl-core 内执行器，rtcl-vm 保持休眠作为 JIT 时代码消费者并列项）见 plan 记录：Command.text/word_srcs 已 Rc 化，ByteCode 站点表/compile-once 尚未动工。
- **字节码 VM 已落地（2026-10-03，C5+C6）**：`rtcl-core/src/interp/vm_exec.rs` 执行器成为第二条活路径——(1) C5：命名 proc 在 `proc` 定义点编译一次（`ProcDef.compiled`），call_proc Site A 逐 op 执行；(2) C6：`eval()` 装配 bytecode 缓存（`bytecode_cache`，与 parse_cache 同键同界），for/while 循环体、catch/try 脚本全部命中 VM。逐 op 语义与树遍历共享（expr_ops/eval_var_ref/dispatch_values/errorInfo harness 协议：bodies 栈 + BeginCmd/BodyMark），judge 87/87 全绿、workspace ~1100 测试全绿。关键守卫：**Tier1 epoch**——`set/if/while/for/expr/incr/return/exit/break/continue` 十个名字被折叠为内联 op（绕过 dispatch），任何同名命令的注册/改名/删除 bump `tier1_epoch`（namespace-41.1），不匹配的编译体回退树遍历；fallback 白名单、`RTCL_NO_BYTECODE`、exec traces 同样钉回树遍历。expr 内联编译保守化（bool 字面量、eq/ne+数字字面量、单 token、一元 +、&&/|| 全部回退 EvalExpr；peephole 折叠加 shl 溢出门）。bench：arith 1.62×、fib 1.83×、var_incr 2.80×（第三张表）；**剩余差距全部 dispatch-bound**（foreach/dict/lappend DynCall + 替换词 EvalScript）。
- **JIT M0 已落地（2026-10-03）**：`crates/rtcl-jit` crate——rtcl-ir `ByteCode` 的第二个消费者。发射器（`emit.rs`，wasm-encoder 0.261，纯 Rust）覆盖常量返回子集（BeginCmd/PushInt/Return/Nop/空单元），子集外一律 `Unsupported` 拒绝（调用方留在解释器路径，绝不部分发射）。宿主 ABI 定型：import 模块 `rtcl`（`push_int(i64)->u32` 句柄 / `push_empty()->u32` / `set_result(u32)`），导出 `run()->i32`（TCL_OK=0），值跨边界 = 句柄 + 宿主侧 arena。round-trip 单测（wasmi 2.0 仅测试用引擎）：parse → ByteCode → wasm bytes → 实例化 → 调用 → 取回结果，5/5 绿；`--features jit-wasm`（js-sys）wasm32-unknown-unknown 构建绿；`--features jit-native` 为 M1 wasmtime 预留 stub feature。
- **下一阶段 = 阶段一：解释器架构对齐 tclsh 8.6（§5 A 系列）**；JIT M1+ 推后（用户裁决 2026-10-03：tclsh 无 JIT，先榨干解释器架构）。
- 历史裁决：
  - **Rc vs Arc**：保持 Rc（wasm 单线程模型；`2257f20` 的静默回退恰好正确）。多 worker 场景每 worker 一个 Interp，不共享。
  - **Value 类型已统一**：rtcl-core 只是 re-export rtcl-vm，无需合并。
  - **bignum 不引入**：整数溢出提升为 f64（与 Tcl bignum 的残留差异已记录在 DIVERGENCES.md）。

## 4. 修正轮次：已收敛

判据达成：judge 文件级 87/87 全绿、case 级 100%（4072/4072，2026-10-07 审计）、无未修复的 semantic-error 级分歧。原 5 个 case 为 namespace-old 边缘组合的 tclsh 侧上下文失败，已定性并对齐。

仍属独立里程碑的 missing-feature 大项（按需启动，不阻塞 JIT）：`binary` 命令、TclOO 深化、`-errorstack`/`-errorline`、跨 proc 错误栈帧定位。

## 5. 阶段一（当前主线）：解释器架构对齐 tclsh 8.6（2026-10-03 起）

**用户裁决：JIT 推后** —— tclsh 本身并无 JIT，却比 rtcl 快 5–11×；先以 tclsh 为参照逐一补齐其解释器架构机制，榨干现有架构后再评估是否引入 JIT。rtcl-jit crate（M0）保留在库中不删除，M1+ 挂起。

### 依据：逐项热点对照（静态剖析 + opt-level A/B 实测）

tclsh 8.6 的性能来自六个机制（.refer/tcl 源码可查），rtcl 现状对照：

| # | tclsh 机制 | tcl 位置 | rtcl 现状 | 差距量级 |
|---|---|---|---|---|
| 1 | **proc 局部变量编译期槽位**（INST_LOAD_STK4/STORE_STK4 直索 Vec，无哈希无分配） | tclProc.c InitLocalCache / tclExecute.c | 每次 LoadVar/StoreVar/IncrVar：split_array_ref 扫描 + resolve_loc（**每访问 1–2 次 String 堆分配**，vars.rs:71/75 `name.to_string()`）+ `HashMap<String,Value>` 查找 ×2；LoadLocal/StoreLocal opcode 已定义但从未发射 | fib 5.12×、arith 5.22×、var_incr 2.39× 的主因 |
| 2 | **Tcl_Obj 引用计数 + refcnt==1 原地变更**（lappend/dict set 均摊 O(1)） | tclListObj.c ListRep / tclDictObj.c | ValueInner 不可变：lappend 每次 4 次容器复制（get_var clone→strict_list→from_list→set_var clone），O(n) per append；dict set 同理（IndexMap 整体 clone） | list_ops 6×、dict_ops 11.25× 的主因 |
| 3 | **list 内部表示零拷贝读取**（TclListObjGetElements 给指针） | tclListObj.c | `as_list()` 每次调用 `items.clone()` 整体拷贝 Vec（foreach 每次 `interp.eval(body)` 前也拷一遍） | list/foreach 全部路径 |
| 4 | **循环体编译进同一 bytecode**（foreach/while 的 body 无每迭代查找） | tclCompile.c TclCompileForeachCmd | 循环体走 `interp.eval(body)`：每迭代对 body 全文做 **两次** HashMap 哈希（parse_cache + bytecode_cache，均以整段文本为键） | 所有循环负载 |
| 5 | **errorInfo 惰性构建**（成功路径零逐命令记账；出错时才从 addr→line 表重建） | tclExecute.c（无 per-cmd hook）/ tclLog.c | VM 每命令执行 BeginCmd：保存/恢复 cur_source/cur_cmd_*（Rc bump ×3+），树遍历路径同 | 全局常数开销 |
| 6 | **字面量驻留表**（per-interp literal interning，编译期共享 Tcl_Obj） | tclCompile.c TclCreateLiteral | 常量池每次 PushConst 从 pool 索引取 String→重建 Value | 中等 |

附带实测：release profile `opt-level="s"`（体积优先）→ 改 3 后 arith 1.13× / var_incr 1.23× / string_build 1.33×（真实但次要；wasm 主目标体积敏感，暂不改默认，结论记录在此）。

另：rtcl 的 dispatch（DynCall→dispatch_values 7 臂链 + 守卫）每命令一次，tclsh 对已知命令编译为 INST_INVOKE_STK 直呼函数指针 —— Tier1 已折叠 10 个命令，foreach/dict/lappend 等仍走 dispatch（与 #2/#3 叠加后才显著）。

### 工作项（A 系列，judge 门控逐项落地）

- **A1 快赢（先落地）**：VarLoc 去分配化（借用/Rc 化，消每访问 String 堆分配）；IncrVar 单次解析（get+set 合一）；flat_aliases 线性扫改哈希。
- **A2 结构项（最大头）**：slotted proc frame —— CallFrame 增 `slots: Vec<Slot>`（Own/Link(ArrayElem 或 VarLoc)/Array/Absent）+ 名字↔槽位表；编译器在 compile_proc_body 为纯局部名发射 LoadLocal/StoreLocal；upvar/global/variable/unset/array 写穿槽位保持名字路径一致（tclProc.c 模型）；调用点绑定参数直达槽位。
- **A3 零拷贝 list**：`as_list_ref() -> Option<&[Value]>` + foreach/lindex/llength/ExpandList 热路径采纳。
- **A4 原地变更**：InternalRep 容器 Rc 化（`List(Rc<Vec<Value>>)`/`Dict(Rc<..>)`）+ var 槽 take→`Rc::make_mut`→放回（无 trace/upvar 别名时走快路径，否则旧路径）——lappend/dict set 均摊 O(1)。
- **A5 循环体免哈希**：eval 的单条目指针同一性缓存（同 Value 指针直接命中 bytecode）或循环体编译进单元。
- **A6（可选）**：BeginCmd 惰性化 —— 成功路径记账移到出错时由 pc→site 表重建（先测 A1–A5 后的占比再决定）。
- 每项 gate：`cargo test --workspace` 全绿 + judge 87/87 + bench 表更新；A2/A4 动 vars.rs/value.rs 核心语义，必要时逐 judge 文件比对 errorInfo 字节。

## 6. 待办：JIT（**推后**，架构优化榨干后再评估）

目标：rtcl compile Tcl → wasm module → 实例化执行。**预期路径 = wasm32-unknown-unknown + js-sys `new WebAssembly.Module(bytes)` 同步实例化**（<4KB 模块；per-proc 模块足够小）。

### 架构裁决（已讨论定案）

- **从 rtcl-ir OpCode 出发，不从源码出发**：JIT 是 ByteCode 的第二个消费者（与 rtcl-vm 并列）。primitive opcodes → wasm 原生指令；`Call(CmdId)` → import 调用宿主 `rtcl_cmd_call(id, args, argc)`；`DynCall`/`EvalScript`/`uplevel` → 回调 VmContext 同解释器路径。
- **粒度 = per-proc module**（Mono jiterpreter 模式）：实例化亚毫秒，失效 = 丢弃单实例。
- **跨边界值表示 = u32 handle + 运行时侧 `Vec<Value>` arena**；不 import 主实例线性内存（wasm-bindgen 内存导出方案脆弱）。native 侧（wasmtime crate 嵌入）可考虑 externref，后期优化。
- **快车道 = expr Int**：`InternalRep::Int` 守卫 + i64 直接运算，守卫失败回退 runtime call。收益集中在数值循环（基线显示 4–16× 差距就在这类负载）；字符串/列表密集负载收益有限（1.2–2×），靠后或不 JIT。
- **失效 = epoch 计数器**：registry.rs 加 epoch（rename/proc redefine/trace 时 bump），JIT 入口一次内存读守卫，失败回解释器。不用反向指针表。
- **错误传播沿用 Tcl result code**（i32 返回 + out-param），不用 wasm exception。
- **平台 feature gate**：`rtcl-jit` crate + `jit-wasm`（js-sys 路径）/ `jit-native`（wasmtime 嵌入）两个 feature；wasip1/embedded no_std 永远解释执行。
- **正确性门 = 三向差分**：tclsh vs rtcl-interp vs rtcl-jit 跑同一 judge 语料（judge/run.sh 加 `--engine jit` 档），外加随机脚本 fuzz。
- **发射器**：`wasm-encoder` crate（Bytecode Alliance，纯 Rust 可编译进 wasm）。不用 Binaryen（C++ 编译进 wasm 体积大）。

### JIT 里程碑

- **M0 ✅（2026-10-03）**：`rtcl-jit` crate 骨架 + wasm-encoder 发射最小 module（常量返回子集 round-trip：编译→实例化→调用→拿回结果），双 feature 编译通过。验收达成：单元测试（wasmi round-trip 5/5）+ wasm32-unknown-unknown 构建绿。注意：测试引擎用 wasmi 2.0（纯 Rust、轻量）；wasmtime（jit-native 的真身）推迟到 M1 落地——本机 3.4GB 构建内存限制下先不引 cranelift。
- **M1**：expr Int 快车道——`expr` bytecode 子集（PushInt/算术/比较/跳转）编译为 wasm i64 指令，InternalRep 守卫。验收：judge 三向差分全绿 + `bench/cases/arith_loop.tcl` 提速 ≥3×。
- **M2**：proc body + primitive ops（变量经 handle arena）。验收：proc_fib.tcl 提速。
- **M3**：epoch 失效守卫 + rename/redefine 风暴 fuzz。
- **M4**：热点 tier-up（proc 调用计数 ~50 阈值）+ 与解释器的无缝回落。
- 每里程碑 gate：judge 三向全绿 + bench 不回退 + 单测绿。

### 参考先例

Mono jiterpreter（Blazor，.NET 7+，运行时 IL→wasm→实例化，最直接对标）；tclquadcode（Tcl 官方 LLVM JIT 实验，已停滞，其停滞原因 = 语义角落难题集中在 trace/unknown/ensemble，不在代码生成）；CheerpJ。

### 设计定稿：内存管理路线 C（2026-10-17）

- **GC 问题已闭环**：Tcl = 引用计数 + 无环树，不需要 tracing GC；WasmGC
  路线（无 finalizer、回收时机不可控）会丢「refcount==0 即时析构」语义
  （trace/命令删除回调依赖），否决。**定案 = 路线 C**：模块内 tagged-i64
  立即数快车道（与宿主 Value 8 字节形状同构）+ 堆对象 handle-arena 留
  宿主 RC + 字符串边界 js-string-builtins 窄切口。完整设计（业界对照、
  ABI v2 全文、编译器架构参考、验证矩阵）：`docs/JIT-WASM-GC.md`。
- **ABI v2 取代 M0 三函数**（breaking）：值穿越 = tagged i64，立即数
  零往返；multi-value 错误标志；**fuel/epoch imported mutable globals**
  （顺带补齐 JIT 步数预算缺口）；wasm→wasm 直通（裸 extern "C" 导出
  直塞 import object，省 JS trampoline）。原 M1 修订为 **M1'**，重启时
  按新文档 §3/§4 执行。
- **两个新开关**：`lazy-string`（JS 串默认保持 externref、首次字节操作
  才物化——**独立于 JIT，rtcl-wasm 默认 on，可先行**，解释器主路径即
  受益）与 `jit-float-lane`（f64 快车道，默认 off，三向差分绿后评估）。
- **fallback 原则措辞修订**：M0 是编译级全有/全无；路线 C 起为「编译
  仍全量、运行时快慢分流」（守卫失败 = 调宿主 full-semantics import）。
- **编译管线架构取向 = Chez Scheme / Nanopass**（参考 guenchi 0x7c06、
  王垠《我不是编译器专家》、Keep ICFP'13）：编译快到无需分级（per-proc
  <1ms 同步编译本来就是 Chez 式增量编译）；复杂度放运行时（值表示/
  freelist/lazy-string）而非编译器；寄存器分配委托 wasm 引擎（Chez 换
  图着色付的 2× 编译税我们不用付）；M1' 的 emit.rs 按 nanopass 纪律拆
  为单职责 pass 链（L0-L4 IR 分层 + 每 pass 独立 golden 测试）。
  **后继项目实证**（2026-10-17 补）：Racket CS 默认全编译转正、
  per-lambda 懒编译被官方判「should be discontinued」——**M4 热点
  tier-up 据此降级为可选**（默认 = proc 定义即编译）；Idris2 以 Chez
  为唯一默认后端十年；Chez 自身仍活跃（10.4.1，2026-05）。参考代码
  阅读清单（Chez s/ 各 pass 文件行数与顺序）见 `docs/JIT-WASM-GC.md`
  §7.5；MoonBit 编译器管线对照（两段式结构、ANF 而非 SSA、contification
  免 tail-call 提案、br_table 分派启发式、自研 wasm 编码器直出）见
  §7.7（源码实证：moonbitlang/moonbit-compiler 开源部分）。
- 重启前置不变：阶段一榨干 + 新文档 §8 假设 A–E 先实测再承诺。
- **任务包已就绪（2026-10-17）**：`TASK-JIT-WASM.md` —— 把本任务完整
  委托给新 agent 的自足施工图（必读清单、M1' 范围与验收门禁、硬约束、
  文件互斥、返回格式、派发 prompt）。重启时直接派发该文件。

## 7. 并行 agent 派发模板（code-migration 纪律）

每个并行域的 prompt 必须包含：

```
你在 rtcl 项目（/home/nzinfo/src.note/rtcl，Rust 实现的 Tcl 解释器）做多轮缺陷修正的 Round N。

## 共同上下文
- 构建：cargo build --release -p rtcl-cli；快速验证：./target/release/rtcl -c '<script>'；
  oracle：echo '<script>' | .refer/tcl910/inst/bin/tclsh9.1（Tcl 9.1.0，2026-11-25 起），
  一切语义以它实测为准；8.6.17 的 tclsh 只用来对照。
- 分歧清单：specs/DIVERGENCES.md（[FIXED] 条目别动）；行为规格 specs/*.allium。
- judge：./judge/run.sh；当前基线见 §3（修复前重新跑一遍取实时基线）。
  不要在并行构建负载下跑 judge（15s timeout 会抖动；先 cargo test 再 judge）。
- 硬约束：只许修改分配给你的文件；不许 git commit；不多修分配外条目；
  修好后 DIVERGENCES.md 标注 [FIXED <当日日期>]；cargo test --workspace 保持全绿；
  单测断言依赖旧错误行为且新行为才正确的，更新断言并在报告中说明。
- 原则：最小侵入；不顺手重构；不引入新依赖。

## 你的任务
<具体条目 + 分配文件清单 + 复现脚本位置>

## 返回
每条修正的根因一句话 + 修改位置 path:line + 验证证据（前后输出对比）
+ 单测/judge 结果 + 未解决条目及原因。
```

文件互斥参考（按域）：string_cmds.rs / expr.rs+expr_funcs.rs / value.rs+list.rs / list_sort.rs / control.rs+loops.rs / error.rs(rtcl-vm+rtcl-core)+call.rs / namespace.rs / vars.rs+call.rs。两域需要同一文件时串行，不并行。

## 审计后续轮. 2026-11-23..25 代码审计与修正——安全、静默出错、性能（perf/wave2 @ 91bb15c..1da7ebe）

> **接管先读这一节。** 2026-11-23 对全仓做了一轮审计，清单在
> `specs/CODE-AUDIT-20261123.md`：S1–S40 安全、C1–C55 静默出错、
> P1–P16 性能，每行最后一列是状态。之后按清单修了 22 个提交，都在
> `perf/wave2` 的本地分支上，**没有 push**。没修完的在 §4，分「可以直接
> 派给事务性 agent」和「先要评审裁决」两类；§3 是架构评审。
> 复现脚本在 `judge/audit-cases/`（先读那里的 README）。

### 0. 现状（动手前重新核实，别直接信这里的数字）

- **oracle 换成 Tcl 9.1.0**（2026-11-25，迁移会话做的）：
  `judge/run.sh`、`judge/probes.sh` 默认用仓内构建的
  `.refer/tcl910/inst/bin/tclsh9.1`；`JUDGE_ORACLE=tclsh` 指回系统的
  8.6.17。**这一轮的 22 个提交都是对 8.6.17 做的**，探针 d20–d27 的
  期望值取自 8.6.17。哪些在 9.1 下不再成立见 §2。
- 门禁基线（1da7ebe，对 8.6.17 实测；对 9.1.0 另注）：judge 88/88
  （两个 oracle 都是）；`bash judge/crash-gate.sh` 126 次运行 126 过；
  `bash judge/probes.sh` 437/0，另有 12 项归档分歧（对 9.1.0 是
  312/437——迁移会话的数字，清障归它）；`cargo test --workspace`
  1481/0（63 个套件）；`bash judge/sweep.sh` 12 处 DIFF，是这轮开始前
  就有的名单：ei_line_attribution、expr_bracket_control、
  expr_bracket_errors2、expr_bracket_multiline、incr_window_p25_errline、
  loop_body_err_line、loop_body_err_line3、loop_body_err_line4、
  sub_inline_errors1、sub_inline_if_body、sub_inline_proc_frames、we5；
  五种特性组合的 `cargo check` 与 wasm32 构建无错误。wasm 产物的检查
  （`node judge/crash-wasm.mjs` 52 项、`node crates/rtcl-wasm/test.js`
  26 项）最后一次跑是在 e46b3e7，**1da7ebe 没有重跑**（§4 T0）。
- 构建与运行：`bash -c 'ulimit -v 3400000; cargo build --release -p
  rtcl-cli -j 2'`；rtcl 跑脚本是 `target/release/rtcl -f 脚本`，tclsh 是
  `tclsh 脚本`。
- 工具都在 `target/audit/` 下，git-ignored，只在这台机器上。每个批次
  的用法：

  | 命令 | 作用 |
  |---|---|
  | `bash target/audit/quick-gates.sh <tag>` | judge、crash-gate、probes、rtcl-vm lib 测试（两种特性）、五种特性组合的 check、wasm32 构建。约 5 分钟，日志 `target/audit/quick-<tag>.log` |
  | `bash target/audit/all-gates.sh <tag>` | 上面这些加 sweep 与 workspace 测试。约 25 分钟，放后台跑，日志 `target/audit/rest-<tag>.log`，末行是 `ALLDONE` |
  | `cd crates/rtcl-wasm && bash build.sh`，再跑上面两条 `node` | 重建 wasm 产物（必须打印 `pkg/ generated`）并检查；提交前跑 |
  | `bash target/audit/bench/run.sh <tcl>` | tclsh 8.6 与 rtcl 两个引擎并排的 µs/迭代、峰值 RSS |
  | `bash target/audit/t/cmp.sh <名>` | `target/audit/t/<名>.tcl` 在 tclsh 8.6 与 rtcl 两个引擎下的输出 diff |
  | `bash judge/audit-cases/rr.sh <tcl>` | 四个解释器（含 tclsh 9.1）各跑一遍，各一行。这个在仓库里 |

  跑门禁期间不要改源码，也不要另起 cargo。机器负载常在 10–20（别人的
  任务），计时要并排比；rtcl 的 `time` 报的是整数微秒，亚微秒的东西用
  整段墙钟时间量。
- 每个批次的顺序：改 → quick-gates → all-gates（后台）→ wasm 产物与
  两项 node 检查 → 审计文档里那一行改状态 → 只 `git add` 自己改的路径 →
  提交。提交信息是中文的 `type(scope): 摘要`，正文写改法和没动的地方，
  `终态：` 一行列门禁数字，最后一行 `Co-Authored-By`。
- **同一棵工作树里有别的会话。** 提交前先 `git status --short`，不属于
  你的改动不要暂存。共享文件（比如 `TODO.md`）只暂存自己的 hunk：
  `git diff FILE > p`，从 `p` 里删掉别人的 hunk，`git apply --cached p`。
  写这一节的时候，迁移会话没提交的是：`Agent.md`、`README.md`、
  `TODO.md` 顶部「对标迁移」、`docs/PRODUCTION.md`、`judge/extract/`、
  `judge/run.sh`、`judge/probes.sh`。

### 1. 已修（22 个提交，一个里程碑一到七个提交）

- 门禁 `91bb15c`：`judge/crash-gate.sh` 与 `judge/crash-probes/`。每个
  探针钉一个脚本可触发的崩溃、挂起或静默错误，期望值取自 tclsh，在
  rtcl 的两个引擎上跑；`_` 开头的文件是工具和探针的源文件，门禁不跑。
- M1 `e513371`：12 个脚本可触发的 panic；整数字面量的语法并成一份实现。
- M2 `c298b77`、`0725bd2`：脚本重入统一成一个嵌套预算；值的字符串化
  与销毁改成迭代；解析、编译、正则的深度与编译空间上限。
- M3 `129966d`：wasm 的递归上限按实测校准（150），步数预算的旁路，
  宿主限额接口，`judge/crash-wasm.mjs`。
- M4 `47731f3`：静默错误两批——编译期边界、tailcall 按 tclsh 的机制
  重做、OO 可见性与 errorCode、interp alias、`prefix match`、Bgerror，
  以及 `expr` 里的 `#` 死循环。
- M5 `00ce332`…`979606d`：exec 的挂起、僵尸进程、错误语义和
  `@通道` 重定向；通道 close 与 exit 不再等没完的连接和管道子进程；
  file 与 socket 的 CLOEXEC。
- M6 `a1259cf`、`904a98c`、`752f5ba`：wasm 回调桥不再认脚本给的实例
  id；子解释器继承宿主的服务与限额；`interp` 命令族按 `tclInterp.c`
  重做（隐藏命令表、`-safe`、任意两个解释器之间的别名）。
- M7 性能 `f8820d5`、`6e3e451`、`c159fa7`、`20f428b`、`e46b3e7`：
  `string map` 一趟扫描（400KB 从跑不完到 2.7ms）；变量插入日志回收
  （96MB → 7.8MB）与枚举去平方；正则引擎缓存、纯文本模式走搜索、
  长文本先制表前 1024 个字符；`string length/index/range/first/last`
  按字节走（24KB 上 13–18µs → ≤1µs）；`append` 接到变量自己的缓冲区
  上（2 万次 49ms → 1.5ms），`lappend`/`dict set`/`incr` 的快路径改成
  就地改。顺带修掉 C40–C46、C48、C50。
- 变量作用域 `1da7ebe`：proc 里执行的 `namespace eval` 体有自己的
  变量帧（C49）；命名空间到全局的链（C52）；有条目没值的变量
  （C53、C18）。**这一笔是按 8.6 做的，9.1 下要改，见 §2。**

### 2. Tcl 9.1 对这一轮的影响（派活前先看）

**(a) 命名空间体里的裸名变量：8.6 回落到全局，9.1 不回落。**
`judge/audit-cases/ns-chain-86-vs-91.tcl` 两个 tclsh 不一致的行（rtcl
现在全部跟 8.6）：

| 行 | 脚本 | 8.6.17 | 9.1.0 |
|---|---|---|---|
| S1 | `set g 1; namespace eval n { set g 2 }` | 写的是全局 `g` | 建的是 `n::g`，全局不变 |
| S3 | 体里 `unset` 掉 `variable` 声明的变量，再 `set` 它 | 写全局 | 写 `n::` 的 |
| S7 | 体里读一个只有全局才有的变量 | 读到 | `can't read "h": no such variable` |
| S8 | 体里 `array get` 一个全局数组 | 给出元素 | 空 |
| S9、S14 | 全局变量上有写 trace，体里用裸名写它 | 写全局，trace 触发 | 建 `n::` 的变量，trace 不触发 |
| S11 | 体里既有 `variable` 声明又有同名全局 | 声明的读声明，别的读全局 | 别的读不到 |
| S15 | proc 里 `global gx`，再到体里 `append gx x` | 写全局 `gx` | 写 `n::gx` |

其余各行两个版本一致，其中 S5、S6 就是 C49（proc 里执行的体有自己的
变量帧），修完两个版本都对。9.1 的规则在
`.refer/tcl910/tcl9.1.0/generic/tclVar.c` 的 `TclLookupSimpleVar`：
不在全局命名空间就加 `TCL_NAMESPACE_ONLY`。

rtcl 里这条链是 `crates/rtcl-vm/src/interp/vars.rs` 的
`ns_chain_global`，调用点在 `resolve_loc`、`resolve_var` 的无帧分支、
`get_var`、`set_var`、`unset_var`、`var_exists`、`store_var`、
`remove_var`、`array_owner`；`1da7ebe` 加的 `phantom_globals` 有一半
是给这条链用的。改成 9.1 就是把这条链去掉（§4 T1），探针 d26 的
o10–o14 和 d27 的第 2、3 节再按 9.1 重钉（§4 T2）。

**(b) 复核过、不受影响的：**

- 哈希表顺序。9.1.0 的 `tclHash.c` 里 `REBUILD_MULTIPLIER` 还是 3，
  新条目插在桶首，重建时桶数乘 4，哈希函数没变；d23（45 条）在 9.1
  下全过。`interp/tclhash.rs` 的重放不用动。
- 变量读写出错的 errorCode（C50、C53、C18）。29 种里 9.1 与 8.6 只差
  `incr-write-trace-fail` 一种：9.1 报 `expected integer but got a
  list`、`TCL VALUE NUMBER`，是 9.x 数字解析的事，归迁移清单。
- 审计探针在 9.1 下第一条不成立的断言（探针遇到第一条失败就退出，
  后面可能还有）：d22、d23 全过；d20 `delete-nonempty`；d21 `c-e`
  （`interp` 的子命令清单变了）；d24 `l55`（正则编译错误的措辞）；
  d25 `idx-99999999999999999999`（下标超出机器整数）；d26 `o11`；
  d27 `o1`。

### 3. 架构评审（评审 agent 的意见；要用户拍板的在 3.8）

**总判断。** 审计里的问题按根因归堆，最重的一堆在变量这一层，其次
是指令集和第二个引擎，然后才是值模型和资源上限。下面按这个顺序
说，每条给方向、代价和建议的时机。数字都是 2026-11-25 在 1da7ebe
上量的。

#### 3.1 变量的存储与解析——最大的结构债

两天里在这一层对出来十来处静默错误（C40–C43、C48–C54），性能上
P4、P5、P14、P16 和数组读写的 3–4 倍差距也都出在这里，根因是同一
个：**rtcl 没有 tclsh 的 `Var` 这个中间物**。一个名字在哪里，要靠近
十套并行的结构一起回答——帧的槽、帧的名字表和它的插入日志、全局
平表和它的插入日志、两个数组名集合、四种链接表、无值条目表、顶层
槽缓存、`variable` 的声明表。读、写、存在、删除、数组命令、trace
各自把这条解析重走一遍，所以修好一处、漏掉另一处（C49 → C52 →
C53 就是这样一串）。列表顺序靠重放插入日志仿真，`array size` 靠
数表，trace 靠字符串键再找一遍。

方向：每个变量表（全局、帧的变量表、数组）都是一个有自己桶链的
真实容器，表里每项是一个 `Var`——值、标志（数组、未定义、链接）、
trace 表、链接目标。解析只做一次，返回 `Var` 的句柄，之后的操作
都对着句柄做。顺序是天然的，不用日志和重放；`array size` 是
常数；trace 和链接挂在 `Var` 上。前提是存储指针稳定，
`docs/PRODUCTION.md` 里存档的「指针稳定存储」方案说的就是这个。

代价是重写级：`vars.rs`（3450 行）、`array.rs`、`trace.rs` 都
要动。安全网是 judge 的 88 个文件、两个引擎的 sweep、1481 个测试、
126 次崩溃门禁，加审计探针。时机放在 T1 把 9.1 的解析规则落定之后
——那时规则最简单（proc 帧，或者当前命名空间的表，没有回落）。先
出设计再派。

#### 3.2 指令集与码表

码表该做，分三步，只有第三步是重写级。

现状：`OpCode` 是 141 个带载荷的枚举变体，每条 16 字节；宽窄两
套变体（`PushConst`/`PushConstWide`）让枚举替编码器做了宽度
选择；加一个超指令要同步七处；`exec_inner` 在每个 op 之前先做
两次分类 `match`（`begin_cmd_site`、`top_op_class`），因为
带载荷的枚举变不出下标。

- **第一步（低风险）**：一张 `OPS` 表做唯一的真源——每个 op 的
  名字、操作数的种类、有没有行号站点、是不是纯 op、超指令怎么
  展开。编码不动。分类 `match` 变成查表，反汇编、JIT lower、校验
  都读这一张表，加超指令从七处变成两处。
- **第二步（中风险）**：补宽跳转。跳转目标和下标现在是 u16，单元
  超过大约 5900 个 `for` 循环（推算约 6.5 万条指令）就整段回落
  树引擎。回落是静默的，行为也跟着变：同一条出错的命令，脚本小的
  时候 errorInfo 和 tclsh 一样，大了以后多出一层 `invoked from
  within "set x …"`（C55，`bash judge/audit-cases/big-unit.sh`）。
  要么加宽形指令，要么跳转改成相对 i32。
- **第三步（重写级）**：紧凑的线性编码（整数码加变宽操作数），
  顺带把 tclsh 的内联 `append`/`lappend`/`incr` 做进去——
  C51 的前两条就是 rtcl 没有这些内联 op 才有的。

不建议为了派发速度去做第三步。每个 op 的税现在约 20ns，tclsh 约
7ns（`docs/PRODUCTION.md` §5 的数字），但收益集中在纯算术循环；在
wasm 上，对整数码的 `match` 通常就是跳转表，真正的开销在载荷提取
和逐命令的站点记账——这一点是推断，没有量过，要先 perf 看清楚。
第三步确定的收益在正确性和维护成本，以及编译结果可以直接序列化
（浏览器端预编译，JIT 重启时不用再降一级）。

对标 9.1 本身不要求改指令集：tclsh 9.1.0 用的还是经典的字节码引擎
（一个字节码加定宽操作数，`.refer/tcl910/tcl9.1.0/generic/` 下的
`tclCompile.c`、`tclExecute.c`）。

#### 3.3 树引擎

树引擎是第二个结构债。它是 `--no-bytecode`、超过字节码上限的大
单元、若干回退路径的执行者，要求与字节码引擎逐字节一致，但 sweep
里 12 处 DIFF 长期在，T3 的复现里两个引擎的输出不一样，C55 也是
它。每修一个语义要修两遍、验两遍；它对嵌套是立方级（P10）；递归
深度的计法两边不同（C39）。

方向是只留一个引擎：`--no-bytecode` 变成字节码 VM 的一种运行方式
（不缓存、不融合超指令），大单元不再回落。能少一份代码和一半的
门禁矩阵，C39 变成单纯的调用深度问题，wasm 体积也会小。代价是要先
列全现在会落到树引擎的触发点，逐个在 VM 里补齐——这份清单是事务性
工作（T13），建议先做，再决定。

#### 3.4 值模型与 9.x 的整数

8 字节标记字加 i63 立即数这个设计是对的，不动。缺的是缓存的字符
长度：tclsh 的字符串表示里带着字节数和字符数，rtcl 每次从头数，所以
非 ASCII 的字符串操作慢 10–250 倍。加一个惰性的字符计数字段能收回
大半；难点是字符串是内部可变的，每个改写入口都要把计数作废。

9.x 的整数是任意精度，rtcl 是 i63、溢出落成浮点。对齐 9.1 就得有
大整数。自己写不划算；用 `num-bigint`（纯 Rust，no_std 加 alloc 可
用）要破「不引入新依赖」的规矩，还会增加 wasm 体积。建议等迁移清单
给出语料和探针里有多少地方依赖大整数，再定是加依赖还是记成已知
差异。

#### 3.5 资源上限

wasm 上没有栈保护区，只能自己记账。现在是一个嵌套计数把 proc 调
用、命令替换、`if`/`foreach` 的体都算进去，有效递归深度因此
只有 tclsh 的一半。方向是把调用深度和求值嵌套分开记，按每种帧的
实际栈用量记预算。树引擎收掉以后这件事会简单很多。

#### 3.6 版本策略

只做 9.1，不留 8.6 兼容开关。两套解析长期并存的维护成本比一次性
对齐大；要查 8.6 的行为，有 git 历史和 `JUDGE_ORACLE=tclsh`。

#### 3.7 多个 agent 怎么共用这个仓库

写这一节的时候有两个会话在同一棵工作树里同时改：一个构建出来的
二进制里有另一个没做完的改动，另一个的探针跑在没提交的代码上，
提交要靠手工挑 hunk。建议每个 agent 一棵 worktree
（`git worktree add ../rtcl-<名字> <分支>`），做完再合；门禁脚本
先搬进仓库（T11），别的 worktree 才用得上。

#### 3.8 要用户拍板的三件事，和建议顺序

1. 版本策略是不是只做 9.1（3.6 的建议是）。定了 T1 才能开工。
2. 树引擎的去留要不要按 3.3 的方向走。不急，T13 的清单可以现在派。
3. 大整数要不要破例加依赖（3.4）。等迁移清单的数据。

建议顺序：T0–T2 → 编译期与命名空间体的快路径（P7、P16）→ 码表
第一、二步（C55 在这里消掉）→ 变量表重写（先出设计）→ 树引擎 →
大整数。

### 4. 未完成的任务包

标注：【事务】可以直接派；【先评审】要等 §3 或条目自己的结论。
每包都过一遍 §0 的门禁顺序。

**T0 wasm 产物检查（事务，十分钟，先做）**
1da7ebe 没跑的两项：`cd crates/rtcl-wasm && bash build.sh`（要看到
`pkg/ generated`），然后 `node judge/crash-wasm.mjs` 要 `ALL PASS (52)`，
`node test.js` 要 `26 passed`。不过就报出来，别盲修。

**T1 命名空间体的变量解析改成 9.1（✅ 2026-11-26 完成，migrate/ns91）；**
版本策略已定：只做 9.1（无 8.6 兼容开关）。
去掉 `ns_chain_global` 这条链：没有 proc 帧的时候，裸名只查当前命名
空间自己的表，查不到就是没有（读报错，写建在当前命名空间）。
`phantom_globals` 留给 `has_valueless_entry`（errorCode），不再用来
改写落点。验收：`bash judge/audit-cases/rr.sh
judge/audit-cases/ns-chain-86-vs-91.tcl` 的 `vm`、`tree` 两行与 `91`
一行相同；judge（9.1）88/88；`JUDGE_ORACLE=tclsh bash judge/run.sh`
会掉文件，把掉的数量记进审计文档；crash-gate 在 T2 之后全过。
派之前先和迁移会话对一下，它的「探针 125 项 DIFF」里可能已经有人
在做同一件事。

**T2 审计探针按 9.1 重钉（✅ 2026-11-26 完成，migrate/ns91）；**
66 探针逐一与 tclsh9.1 对照：d26/d27 两步重生成；18 个探针里 40 条
8.6 钉形断言转 skip（`# skip <tag> -- 原因`，原因指向 TODO 对标迁移
工作组）；b03/d14（oracle 下状态行即死）、b09/b10（rtcl 扩展无
oracle）、d01/d02（oracle 超时）加头注释只在 rtcl 上防回归。**

`judge/crash-probes/` 下 66 个探针的期望值都来自 8.6.17。逐个用
tclsh9.1 跑，对第一条失败做判断：是 8.6 到 9.1 的语义差，就用 9.1
重新生成期望值（有源文件的：`.refer/tcl910/inst/bin/tclsh9.1
judge/crash-probes/_gen.tcl <源> <探针>`，再同样加 `--refix <探针>`
跑一遍），或者用 `# skip 标签 -- 原因` 跳过；是 rtcl 相对 9.1 的缺陷，
就修。验收：每个探针在 tclsh9.1 下要么全过，要么跳过的都有写明的原因；
rtcl 两个引擎全过。注意这是 `judge/crash-probes/`，不是迁移清单里的
`judge/probes/`。

**T3 proc 里 `info locals`/`info vars`/`info frame` 的顺序（事务；编译
局部变量集合是否与 tclsh 相同要先核对）**
复现 `judge/audit-cases/locals-order.tcl`：两个 tclsh 一致，rtcl 的
两个引擎各不相同。tclsh 的顺序是先编译局部变量（按编译器给的槽位
顺序，参数在前；`info locals` 不列链接，`info vars` 列），后运行期
建的名字（哈希顺序；删了重建的换新位置，编译局部变量回原位）。
rtcl 现在每次调用都先 `degrade_frame_all()`——把整帧降成名字存储，
之后这一帧一直走慢路径——再按建名的先后列。帧上现在有
`CallFrame.compiled`（两个引擎都有），可以不降级直接列：槽按表顺序，
其余名字的日志按 `tcl_order` 重放。

**T4 命名空间体里的变量访问提速（事务，T1 之后做）**
`judge/audit-cases/bench-ns-body.tcl`：1500 万次裸名读写，tclsh
2.6–3.5 秒，rtcl 约 40 秒（每次约 2.5µs，rtcl 在顶层是 0.19µs）。
原因是命名空间体里每一次读写都走慢路径：`current_namespace != "::"`
让 `read_var_scalar_top`/`scalar_top_write_ok` 的守卫直接放弃，然后
`canonical_global`、`redirect_flat`、`ns_var_declared` 各做一两次
`String` 分配。T1 之后解析只剩一个键（`<ns>::<名字>`，在 `globals`
里查一次），可以给这两个快路径加一条带命名空间前缀的分支：键在栈上
拼，超长或名字带 `::`、`(` 就回落；读命中即返回，写只在键已存在时
原地写，新建走慢路径。验收：这个基准到 tclsh 的 3 倍以内；顶层的
`bench/run.sh` 不回退。

**T5 C51：trace 与编译后的命令（先评审：三条各自怎么取舍）**
复现 `judge/audit-cases/var-errorcodes.tcl`，两个 tclsh 在这三条上
一致，和换 oracle 无关。(1) tclsh 把简单词的 `append a x y z` 编译成
每个值一次写，写 trace 触发三次，rtcl 一次；(2) 读 trace 报错时，
tclsh 编译后的 `incr`/`lappend` 把变量当成未定义继续（结果码 0，
errorCode 留下 `TCL READ VARNAME`），rtcl 的 `incr` 报错、`lappend`
不触发读 trace；(3) 元素的 unset trace 报过之后，`unset` 的 errorCode
tclsh 是 `TCL UNSET VARNAME`，rtcl 是 `TCL LOOKUP ELEMENT 1`；数组
写 trace 报错时 `-errorstack` 里 tclsh 记 `wa`，rtcl 记
`invokeStk1 wa`。(1)(2) 要等 §3.2 的第三步。

**T6 通道的缓冲读（✅ 2026-11-26 完成，task/t6 @ f699214；**
P9 文件读 396ms→36.7ms=1.38×、P1 TCP 597ms→5.6ms=1.17×（tclsh9.1 同窗）；
遗留：gets 循环 2.6× 在命令层（TODO 性能段新项）、9.1 坏字节严格解码归迁移。**
原任务书：**T6 通道的缓冲读（事务）**
P1：TCP 读每字节一次系统调用，3MiB 要 387ms（tclsh 5ms）；P9：文件
读逐字节，32MB 要 0.6 秒（tclsh 58ms）。都在 `channel.rs`。验收：
两个数字到 tclsh 的 3 倍以内，`judge/corpus` 里 io 相关的文件不变。

**T7 小的性能项（事务；P6 先评审）**
- P5：`array size`/`array exists` 扫整张变量表（8 万元素 0.8ms，
  tclsh 是常数），在 `commands/array.rs`。给数组记一个元素计数。
- 数组元素的读写比 tclsh 慢 3–4 倍（`set_var`/`get_var` 的数组分支，
  `vars.rs`）。
- P15 剩下的三处，在 `commands/string_cmds.rs`：`string last` 命中
  2µs（tclsh 0.1）；带空格的长模式在没有空格的文本上未命中 47µs
  （tclsh 14，标准库挑的过滤字节每个位置都命中）；非 ASCII 串上命中
  位置靠后时还要数一遍字符。
- P6：变量表的哈希不扩散，同前缀的名字退化（`interp/varmap.rs`）。
  先量出现在还成不成立。

**T8 P7 编译期（✅ 2026-11-26 定位+修复完成，task/p7-prof @ 4d9ba2a；**
热点三连：count_lines 前缀重扫（O(字节²)，88%）/add_local 线性扫（90%）/
fuse_pairs retarget 交叉积（4.7%）——LineIndex+HashMap 修掉，for5000 0.28s→
0.01s（0.5× tclsh）、set16000 0.15s→0.01s（1×）；行号 22 形状电池逐字节不变；
记录见 bench/BASELINE.md 2026-11-26 节。**
原任务书：**T8 P7 编译期（事务：先定位热点）**
2026-11-25 重量的结果：字节码上限以内，循环多的单元编译是超线性
的——3000 个 `for` 0.18 秒，4000 个 0.28 秒，5000 个 0.47 秒（tclsh
2 万个 0.10 秒）；不同变量名多的单元也是——16000 个 `set kN 1` 0.15
秒（tclsh 0.02 秒）。再往上单元整个回落树引擎，反而不编译了（C55）。
常量池的线性扫描早就改掉了，剩下的热点还没定位；先用 `perf` 找
出来。码表第二步补了宽跳转之后，这些单元会回到 VM 上编译，到时候
这个问题会从「静默回落」变成「真的很慢」，所以要在第二步之前或
同时解决。

**T9 语义遗留（逐条先评审）**
C16（只含字面量的表达式算术出错时，`expr` 帧的关键词；暂存在
`judge/probes-known/smallface_p24_open.tcl`）、C17（`lt`/`le`/`gt`/`ge`
在 tclsh 里不是运算符，rtcl 当成字符串比较；9.1 下要重看）、C28
（proc 里 `namespace eval … { tailcall … }` 的报错）、C32（带宿主
`HostExec` 的 exec 路径还是旧语义）、C33（`file attributes` 没实现、
`file normalize ~user` 不查用户）、C38（别名目标的 wrong-args 里的
命令名，tclsh 自己就不一致）、C39（有效递归深度约 496 层，tclsh
996：命令替换、`if`/`foreach` 的体都计进嵌套深度）、C47（带回引的
模式在约 1000 字符以上的文本上走到步数预算）。

**T10 安全遗留（逐条先评审；S39、S40 可以直接做）**
S21（`lrepeat 4294967296 x` 一类的大小在 32 位上回绕）、S23（wasm
栈溢出 trap 之后整个实例永久失效）、S30（`panic = "abort"`，全仓没有
`catch_unwind`，要不要留一道防线）、S38（HTTP 客户端默认没有超时、
响应体没有上限、没有 URL 审查钩子）、S39（变量表的哈希不带密钥，
可以预构造冲突键，风险低）、S40（Tk：畸形 PNG、多字节颜色串、
`image put -to` 负坐标会 panic，读码看到的，没运行过）。

**T11 清理（事务；✅ 2026-11-26 大部完成）**——门禁脚本入仓（judge/gates/，b9ef023）+ exec_cmd 死代码清除 + 空转探针普查记账（task/t11b @ 9951b16：42/437 空转已头注释、18 个候选判为有意钉形排除、exec_cmd unused 清零，rtcl-vm 警告 42→32）。**余项**：rtcl-vm 其余 ~30 条 unused 警告、clippy 棘轮（unwrap_used 等）；**objsec.tcl 墙钟探针 flake**（loop=[秒差]，空载 0s vs 1s 翻红，与改动无关——T11b 与 P7 两会话独立互证；建议加容差或归 known）。**
原任务书：**T11 清理（事务）**
- `commands/exec_cmd.rs` 里 M5 重写留下的死代码（`output_redirect`、
  `syntax_error` 等，编译警告十来条）。
- `judge/probes/` 里约 30 个探针在两个解释器上都提前死掉，等于空转：
  逐个在 oracle 下跑，看它有没有走到最后一条检查。
- `rtcl-vm` 开 clippy 的 `unwrap_used`/`indexing_slicing`/
  `arithmetic_side_effects`，按文件棘轮。
- 把 `target/audit/` 的门禁脚本搬进仓库，在 `Agent.md` 写明用法；
  `Agent.md` 的提交门槛那一行还写着 sweep 0、tests 1143，是旧的。

**T12 文档终态（事务，最后做）**
`specs/CODE-AUDIT-20261123.md`、`TODO.md`、`README.md` 的数字和状态
对齐到最后一次门禁。

**T13 列出回落到树引擎的全部触发点（✅ 2026-11-26 完成，migrate/ns91）：清单在 `specs/TREE-FALLBACK-TRIGGERS.md`——真实回落面 = F1 u16 溢出（C55）/F2 exec trace 钉回/F3 epoch 失配/F4 手动关断/F5 >256KB 不走缓存/F6 body_memo 挤出；编译器 op 白名单实际恒真（14 个非白名单 op 零发射，只服务 assemble）。**
grep `fallback`、`RTCL_NO_BYTECODE`、`bytecode_applicable`、exec
trace 相关的钉回点，加上 C55 那种超限单元，给每一个写出：触发
条件、复现脚本、在 VM 里补齐还缺什么。只要清单，不要改代码。§3.3
的去留看这份清单。

**没列成任务包的**：P10（树引擎，等 T13）、P11（建子解释器
460KB/3.4ms）、P12（JIT，暂停）、P13（Tk）。

### 5. 坑（本轮实证）

- **探针的期望值要用探针自己的行去取**。列表顺序取决于表有多满，
  生成器运行时全局表里有它自己的变量，桶数不一样，记下来的顺序在
  探针里不成立。生成是两步：`_gen.tcl <源> <探针>`，再
  `_gen.tcl --refix <探针>`（另开一个干净的 tclsh 跑探针自己的行，
  把期望值整个重写；期望值跨多行的也认）。跑生成器和 `--refix` 的
  必须是当 oracle 的那个 tclsh。
- 列表顺序这类不好移植的断言用 `# skip 标签 -- 原因`，原因要写。
- 一个 proc 里读 `m`，后面的检查接着用 `$m`：检查之间是有状态的，
  调顺序前先想清楚。
- Python 批量改代码：`str.count` 断言会被子串骗过（缩进多的行包含
  缩进少的 needle），用行级全等；按标记切片比按行号稳。
- `cargo build … 2>&1 | grep …` 的返回码是 grep 的。看有没有 `^error`
  行和 `Finished`，再看二进制的时间戳。
- `target/audit/t/` 下的对照脚本：tclsh 是 `tclsh 脚本`，rtcl 是
  `rtcl -f 脚本`，两边不一样。
- 返回 `&mut` 的取槽函数不能连着调两次（借用检查）。用闭包把槽交给
  调用方（`with_dict_slot`），或者只借某一个字段（`scope_frame_idx`）。
- `_gen.tcl` 里带引号的 `{` 要写成 `\{`，否则整个 proc 解析不了。

## 10vicesimusduodevicesimus. 2026-11-23 trv_p11 五处语义缺陷收口 + e8/e9 内层帧链式载体 + oo2 表述精确化（perf/wave2 @ c919738..）

接续崩溃中断轮。前任工作区遗留 8 文件实现（含一处**方向性错误**
——e8/e9 用 early-commit 形，被钉形否决后改链式载体重做）。终态：
**probes active 437 全绿、known 11（trv_p11 仍归档，残差 1 行）、
judge 88/88、workspace tests 1471/0**。代码两提交：7272638
（trv_p11 五处）、192d698（e8/e9 链式载体）。详录见
specs/DIVERGENCES.md 2026-11-23 节。

### 1. trv_p11（7272638，主修）

- ①② 冒号串语义：新 `split_ns_tail`/`join_var_key`（namespace.rs，
  解析面分段：`::` 起整串冒号分隔、尾单冒号属名字、尾 `::` 串 =
  空名）——canonical_global/canonical_key_for_ns/读父检查/写宿主
  预检/var_exists/which -variable/upvar 宿主检查八处对齐。关键钉
  形：`variable :` 的名字就是 `:`（键 `tnv:::`）；`::tnv:::` =
  tnv 的空名变量；`::a::b:` = ns ::a 的名字 `b:`；`::::z` 前导吞
  串 → 全局 z。
- ③④ upvar 元素别名生命周期（proc.rs + mod.rs + vars.rs）：元素
  形建链落**帧局部占位数组**（S1 基是帧局部标量建链即报 isn't
  array / S2 缺基建占位 array exists=1 / S3 全局同名不顶替）；宿
  主 unset 后读 no such variable、写 "upvar refers to element in
  deleted array"、括号写 "variable isn't array"（NEEDARRAY 先于
  dangling）。`UpvarLink::Dead` 增 `was_global` 字段——帧局部死
  链读回退不得落 globals（占位 arr(1) vs 全局 arr(1) 穿透否决）。
- ⑤ ns delete 内外区分（vars.rs `deaden_links_under(inside_tree)`
  + namespace.rs 调用点）：树内删除走 teardown 延迟（帧语境种子
  读仍通）；外部删除立即拆表 + install 期镜像清除（读/unset no
  such variable、exists 0、写 dangling-namespace）。
- 残差：探针仅 t2-vars 序行（auto_path/errorCode 转置）= oo2 族
  ghost-insert 同源，**定调并入 oo2 不单独追**。探针仍归档
  （known 11 不变），README/DIVERGENCES 已注记。

### 2. e8/e9（192d698，cases 归档拆出转绿）

`set x [[a]b]` 括号全闭合形——rtcl 缺替换词内层命令帧（词级
defer 的 pending 被外层属主 defer 覆写静默丢失）。

**方向性否决记录**：前任 early-commit 形（替换单元出口直接落账
err_pending_unit_frame 门控提交）修复了文件级形态但**穿透
catch/proc/while 边界**——tclsh 的内层单元出口帧只在**文件级退
出**兑现（catch 形三帧不含替换帧、proc 边界只落 (procedure) 帧、
while 边界只落 body 帧、flow 转换只有 converting 帧、depth-2
`set x [[ [[a]b]c]d]` 全链五帧——九钉形实测）。本轮改
`err_pending_chain` 链式载体：词级 defer / Sub 区 unwind 顶替时
移链（err_pending_supersede），error_report 先链后顶逐帧落账；
其余边界与 pending_top 同生死（catch/return/error 物化清空、
harness 直落帧 ERR_ALREADY_LOGGED 单次消费清空、eval_isolated/
trace save-restore 同步、decode_toplevel_flow flow 转换丢弃、
jit_err_clear_pending 清空）。expr 抑制臂正交性保持
（err_expr_origin 在 defer 臂消费、抑制 pending 不入链）。

残差留档：tree-walk 引擎（--no-bytecode）flow 形 converting 帧
读内层 "break"（tclsh = 外层 set 帧）——**既有缺陷与基线逐字同
形非本轮引入**；默认 bytecode 引擎正确。重开条件：tree-walk
flow defer 载荷对齐。

### 3. oo2 表述精确化（纯文档）

tcl_order "理论边界" → "**投入产出比**边界"（技术上可修、成本否
决——每收口一行可能翻红别行、无限细分），DIVERGENCES 三处同步；
auto_index 标注已修（2912c81）不变。

### 坑（本轮实证）

- 后台构建**尾随 `&` 会话回收**：`run_in_background` 里
  `( ... ) & echo started` 的子进程随前台命令退出被回收——构建
  半途而废且 task 通知误报完成，对照跑的是**旧二进制**（首次钉
  形复测结果全错）。正解 = 整条命令后台化、日志落文件后
  grep exit= 行确认。
- python 批量 patch 的**子串包含陷阱**：48 空格缩进行包含 36 空
  格 needle 作为子串，`str.count` 断言虚高——行级全等匹配替换。
- 多文件 patch 后 grep 验证照旧执行（全部命中）。
- 判定 tclsh 语义先收集**边界矩阵**再写代码：本轮先跑九钉形
  （catch/proc/while/flow/depth-2/裸命令/expr/引号拼接/多词）钉
  死"只在文件级退出兑现"，避免了按单钉形想当然的二次返工。

## 10vicesimusseptimusdecimus. 2026-11-22 globprobe 终局——parse_cmd_sub opener 栈，known 11（白盒对照 tclParse.c；perf/wave2）

probes-known 最后一条 parse 深水（globprobe 引文终点）修复转绿移回
active。终态：**active 437 全绿、known 11**。

### 白盒方法论（本轮核心差异，黑盒二分失败后的正解）

上轮（sexdecimus 节 4）黑盒二分钉不死"位置敏感的扫描栈语义差"。
本轮直接以 oracle 内部为 ground truth，三条证据链：

1. **gdb 逐层 trace**：`/usr/bin/tclsh` 无参符号（stripped）→ Python
   API 在 libtcl 加载后 `break *base+0x106c90`（`nm -D` 取
   Tcl_ParseCommand offset；gdb 默认关 ASLR，base 稳定），stop() 里读
   $rsi/$rdx/$rcx 打印每次递归入口（ptr/bytes/nested + 内存头 16 字节
   识别）。globprobe gp11 的递归树全部现形。
2. **frameprobe 逐帧重放**（/tmp/wbparse/，tcl8.6-dev 头 + libtcl8.6
   链接）：对递归树每帧的 (off, bytes, nested) 直接调
   Tcl_ParseCommand 打印 term/errorType/incomplete——关键帧 [E]
   （行 9 的 `string match {[]x]y} "]y"`）= OK term=**\n**
   （不是 `]`！）。
3. **读 .refer/tcl/generic/tclParse.c**（本地已有，无需网络）：上面
   两条证据逐行对上 ParseTokens bracket 分支（1038..1258）。

### 语义定谳（引文终点的权威规则）

- ParseTokens bracket 分支 = **逐命令循环**：`[...]` 内容按
  Tcl_ParseCommand(nested=1) 一条条解析，仅当某条命令的
  `term == ']'` 且 !incomplete 才算 bracket 闭合；否则循环继续收下
  一条命令。**引号/brace 里的 `]` 永不闭合**（`"]y"` 的 `]` 是引文
  内容）——globprobe 行 9/11 视觉上"闭合"的 `]` 全在引号内，行 9
  的 bracket 一路吞行 10/11。
- EOF 时 numBytes==0：errorType=MISSING_BRACKET，
  **term = tokenPtr->start = 当时正在收命令的那层 `[`**（
  tclParse.c:1163），即**最深未闭合 `[` 的 opener**；外层逐层原样
  上浮（`parsePtr->term = nestedPtr->term`，tclParse.c:1136）。
- 引文 = Tcl_LogCommandInfo(script, commandStart, term+1-cmdStart)
  的 150 字节 cap（tclBasic.c:5363 → tclNamesp.c Tcl_LogCommandInfo
  limit=150，纯 verbatim 前缀 + `...`，无任何修饰/闭合装饰——此前
  "display 插入 `"`"的错觉是**格式串的外层 wrap 引号**恰好贴着截断
  点）。
- term 可被 150-cap 掩盖（gp12/13 term=行 12 的 `[`，长度 165>150）；
  用 frameprobe 直读 term 而非 errorInfo 推断。

### 修复（word.rs parse_cmd_sub，单点）

rtcl 平铺 depth 计数在 EOF 失败时只能报**本调用** opener（浅 2 层，
行 9）——消费边界（skip_quoted/skip_braced 吞吐）与 tclsh 逐命令
循环逐一相同，差的只是失败层记账。修复：depth 换 **opener 栈
`Vec<usize>`**（`[` push、`]` pop），EOF 失败报 `openers.last()`。
单层退化（b1/b2/b5 `set x [abc...`）栈只剩 opener，offset=start-1
逐字节不变。

### 验证与门禁

- globprobe 双端逐字节一致（probes-known→active，git mv）；
  gp7/gp11/gp11m/gp12/gp13 前缀变体全 AGREE；gp_2（`set x [foo
  "a[<NL>bar]`）、b1/b2/b5、t2、sp1 最小形重放 AGREE。
- probes：**active 437 全绿**（fail=0）、known 11（1 pass=原 globprobe
  位）。judge 88/88、workspace tests 全绿（数字见门禁记录）。
- known 终态 11：c5_dis/cmp_unsupported×2、dbg_basic、
  cases/cases2/probe_round3、oo2、probe7、trv_p11、utf_U_parsehex。
- 新发现预存形（**本轮不修**，留档）：`set x [[a]b]`（括号全闭合、
  运行期 `invalid command name "a"`）——tclsh errorInfo 三帧
  （`a` ← `[a]b` ← `set x [[a]b]`），rtcl 缺中间帧（词求值期出错时
  替换词内层命令的帧）；与 archived cases 族同类（词求值帧链），
  与本轮 parse 修复无关（修复只动 EOF 失败 Err 路径的 offset）。
  另：objsec.tcl 是墙钟探针（clock seconds 跨秒边界），全量跑在
  构建负载下会偶发 DIFF——静默机器 6 连全绿，与既往"负载下 judge
  抖动"教训同源。

### 教训

- 黑盒二分的死角：该形态的错误面由**两层机制合成**（扫描栈决定
  term、LogCommandInfo 决定截断），输出只见合成结果——单条规则反
  推不可能。oracle 内部（gdb trace + 子进程逐帧重放）才是 ground
  truth；.refer 源码用于把实测语义钉回代码行号。
- tclsh 的 errorInfo 引文是 verbatim 前缀，任何"装饰字符"都应先
  怀疑格式串 wrap（`"..."`）与 `...`——本例 `puts ["` 的 `"` 是
  wrap，不是文本。
- stripped 二进制打断点：pending bp + libtcl 加载后用 Python API
  `break *base+offset`，寄存器取参（SysV: rdi/rsi/rdx/rcx/r8）。
- .refer/tcl 是 Tcl 9 风格源码（Tcl_Size 签名），不能直接对系统
  8.6 库编译——tcl8.6-dev 的 tcl-private 头 + 公共 API（Tcl_ParseCommand
  是导出符号）够用。

## 10vicesimussexdecimus. 2026-11-22 known 16→12 深水修复轮——ns delete 两阶段 / 词首引号 / auto_index / parse 归属 / we5 行号（perf/wave2 @ c1c1b19..）

known 归档（probes-known）中 5 条深水的修复轮。4 条全修复转绿移回
active（trv_d13/trv_p10/pidx/we5），globprobe/oo2 残差收窄留档。
终态：**active 436 全绿、known 12**、judge 88/88、workspace tests
1460/0。

### 修复（每条一 commit）

1. **9ef6f88 ns delete 两阶段拆除**（trv_d13/trv_p10）：tclsh
   Tcl_DeleteNamespace 的 NS_DYING 可见性摘除先行、实体拆表延后
   （TclTeardownNamespace 顺序：变量逐 unset → 命令逐 delete →
   最后 unlink childTable）。rtcl 加 `ns_dying` 集合（exists/require_ns
   报 gone、解析路径照常可达——TclGetNamespaceForQualName 不查
   flag）+ `var_nuke_pending` 窗口（UnsetVarStruct 的表内 undefined：
   which -variable 在 unset trace 期仍解析、读/info vars 照旧 miss、
   18.4 计数形状不回退）；dying ns 二次 delete = NS_TEARDOWN 静默
   跳过。自删路径（activationCount>0 等价）立即摘除不变。
2. **9d9e971 词首引号纪律**（pidx）：subst 括号扫描
   （run_bracket_script/check_bracket_tail）加 word_start 跟踪——词中
   `"` 是普通字符（`expr $foo([x"y])` 的下标命令名 `x"y`）；引文开放
   到 EOF 报 missing \"。
3. **2912c81 ::auto_index 惰性首建**（oo2 auto_index 行）：tclsh 默认
   unknown → auto_load 进入即建（T6/oo_ai7 钉形：查找失败也建）；
   rtcl 首建从 has_unknown 臂（只认用户 proc）移到 unknown 臂入口
   两臂共用。oo2 r4 行 auto_index 桶位与 tclsh 一致——残差仅剩
   e/errorCode ghost-insert 物化序一行（tcl_order 理论边界，留档）。
4. **41a3337 parse 错误归属最外层命令**（globprobe 归属半边）：
   ParseError 增加 cmd_start/cmd_line 章（parse_command 失败上浮逐层
   盖章）；seed_parse_error 引文起点/行号读章；skip_quoted_in_cmd 的
   missing " offset 改 opener。gp_1/gp_2/gp_3 最小形全绿。
   **残差留档**：globprobe 引文终点——tclsh term 在 ≥4 层引号⇄bracket
   互嵌套递归的最深 opener，rtcl 浅 2 层；位置敏感的扫描栈语义差，
   黑盒二分无法钉到单条规则，重开条件 = rd 扫描器嵌套栈改造。
5. **378d980 err_pending_top 单元相对行**（we5）：载荷
   (text, abs, unit_rel)——词替换 defer 位点携带体相对行 = 属主行 +
   内层日志行 - 1（err_line 此刻 = 内容单元坐标；we5 `set r [boomq]`@2
   + boomq 帧@1 → 2；ei D `set x [`@1 + nosuch@2 → 2——两形同式）；
   ns eval 体出口帧挂起在位读新字段（err_exit_frame_at），无挂起
   维持 err_line。flow 挂起位点带属主行、JIT 置 None（旧口径）。
   ei_line_attribution A-J/file_ctx_frames/loop_body_err_line×4 全绿。

### 教训

- tclsh errorLine 是"按日志事件以当前单元坐标重算"的全局——rtcl 的
  err_line 停在最内层日志单元坐标，跨单元出口帧（ns eval 体）需
  显式折算载荷；折算式（属主行 + 内容日志行 - 1）对 D/we5/ng 三形
  同一，未动 line_ctx/line_base 模型（adb25e2 口径不变）。
- 归档探针双端逐字节转绿后**移回 active**（git mv），known 目录
  只留真分歧；probes.sh 动态计数（active 436 / known 12）。

## 10vicesimusquintus. 2026-11-19 clock+package 剥离 → JS dynmod 补回（方案 a；perf/wave2 @ e2f956a..）

用户拍板候选 #1 落地形态 = **方案 a（JS dynmod）**：clock/package 从
wasm 构建剥离，JS 面注册同面命令补回——命令计数面零变化。

**盘点结论（选型依据）**：
- clock native 面 = clock.rs 2252 行（D-24/D-25 收口态：seconds/clicks/
  microseconds/milliseconds/format/scan〔-format strptime 面 + 自由
  scan 怪值收口〕/add；儒略-格里切换）。wasm 构建上 clock 全子命令在
  但是 **UTC-only stub**（clock = ["std","dep:libc"]，libc 不进 wasm：
  local_offset_name 恒 None、with_zone 恒等、-timezone 恒报 not found）。
- package native 面 = package.rs 166 行（provide/require/names/forget/
  vcompare/vsatisfies；require 的 pkgIndex 扫描走 file feature）。
  `interp.packages` map 只被 package.rs 自用；stdlib.tcl（init.tcl 移植
  的 auto_load/auto_path 面）**不调用 package 命令**——零内部依赖。
  **选型 = 纯 JS 全实现**（最小内建 + 扩展形态无必要）：registry 状态
  在 JS 模块；require 的 auto_path/pkgIndex 流程在 wrapper proc（Tcl
  面）编排——JS 命令回调无法重入解释器 source，native pkg_require 的
  逐步同构用 Tcl 写（registry get → auto_path foreach → file exists →
  catch source → re-get → can't find package）。
- JS 面优势（方案 a 的额外收益，**功能升级**）：format/scan/add 默认
  渲染 = 宿主本地时区 + `-timezone` 支持 IANA 名（Intl tzdata 求值）
  ——剥离前 wasm 是 UTC-only stub，两能力皆缺；tclsh 对齐度不降反升。

**件1 `www/clockpkgmod.js`**（JS 同面实现，~1050 行）：
- clock 面：子命令 Tcl_GetIndexFromObj 语义（精确/唯一前缀/歧义）、
  format 规定符全集（%Y %y %m %d %e %H %I %M %S %p %P %a %A %b %h %B
  %j %w %Z %z %n %t %% %s %c %D %T；未知原样）、scan -format（字段
  合成规则/%y 窗口/%p/%z 回推/%Z 缩写表优先）、自由 scan（ISO/
  YYYYMMDD/斜杠/月名/日名/D-Mon、时刻 am/pm/裸数字 HHMM、星期
  next/last/Plain、相对短语 ago 否定域、军事单字母、TimezoneTable
  全表、next|last <月名> 怪公式）、add 顺序单位 + 月末夹取 + 儒略
  切换、错误消息+errorCode 同文（__CLKERR 哨兵列表，wrapper
  `return -code error -errorcode` 解包重抛——"JS command
  '__dynmodv2:…' error:" 派发前缀不外泄；正常值 __CLKOK 包裹消除
  结果首词歧义）。
- 时区缝：zone ctx 三态（gmt/IANA/本地）；wall 求值 = Intl
  formatToParts（hourCycle h23）+ 自算 JDN 基（规避 Date.UTC 的
  0-99 年 legacy 映射）；civil→epoch 双偏移候选回验（fold 取转换
  前偏移 = DST 支〔tclsh 同〕、gap 落转换后墙钟〔glibc 同〕）。
  %Z = Intl short 名（GMT/UTC/GMT+8 风格——DIVERGENCES 定调）。
- package 面：六子命令 + `_registry_get` 内部查询面（公开面不带）；
  arity 消息随 native 的 wrong_args_with_usage **双前缀**形态
  （"package provide package provide name ?version?"）。

**件2 wasm 剥离 + 挂线**：rtcl-wasm Cargo.toml features 去掉
clock/package（rtcl-vm 侧 CMD_TABLE_CLOCK/PKG 已 cfg 门控，native
rtcl-cli 不动）；rtcl-worker.js boot 序列 encoding 块后加 clockpkg
块（`namespace which -command ::clock` 不存在 → register_module
('clockpkg') + load + CLOCKPKG_SETUP_TCL〔::clock/::package wrapper +
pkg_require 编排〕）；dynmod-test.html 同款挂线（T9 动态基线前）。

**验证（全绿）**：
- node 差分 118/118：期望值 = target/release/rtcl 探针钉定（两批
  /tmp/clock_probe*.tcl），含 DST gap/fold（1615707000/1636263000）、
  %Z 缩写表（est/z/军事单字母）、%y 窗口、儒略历 0001-01-01、
  est5edt/裸月名/裸 HHMM 怪值、package 六子命令+arity 双前缀。
- 计数面对表：worker boot 后 `info commands` 清单 99=99 **逐字相同**
  （node worker_threads 探针 dump diff；clock/package 名称在列，
  实现面换成 JS）。dynmod 页初始计数 102 不变。
- 尺寸数字链（两轨，wasm-opt 后）：

| 轨 | raw | pkg | gzip |
|---|---|---|---|
| perf 剥离前 | 2,824,454 | 2,105,384 | 783,632 |
| perf 剥离后 | 2,739,147 | 2,031,839（1.94MiB） | 756,245 |
| **Δ perf** | **−85,307**（预测 85KiB 命中） | **−73,545** | −27,387 |
| size 剥离前 | 2,390,402 | 1,731,902 | 674,592 |
| size 剥离后 | 2,324,340 | 1,675,543（1.60MiB） | 653,116 |
| **Δ size** | −66,062 | **−56,359** | −21,476 |

- 门禁：cargo test --workspace 全绿；judge interp 88/88 + judge jit
  88/88（jitsh release 双 feature）；node test.js 26/26 +
  test-worker.mjs 3/3；真机 8933 COOP/COEP + ThreadingHTTPServer：
  fs-opfs-test **41/41**（worker 路径，红线 file/io/OPFS 0 变化）+
  dynmod-test **62/62**（原 41 + 新 K1-K13 clock / P1-P8 package，
  期望值 = native 探针钉定：格式矩阵抽测/错误码面/pkgIndex 扫描
  全链路〔memory fs 写 pkgIndex.tcl → auto_path → source → require〕）。

**坑**：
- native 的 clock fold（秋拨）在同进程先跑过 gap 例后取反常值
  （with_zone 的 glibc 状态泄漏：P42 = 1636266600 vs 干净上下文
  1636263000 = tclsh 值）——JS 恒取干净值，探针钉值必须用干净上下文
  （单独脚本跑），否则差分基线本身被污染。
- `clock scan -format` + `%Z` + `-base 0`：native 61200 / tclsh
  -25200——native 既有残差（D-25 缩写表探针未覆盖该组合），JS 随
  native。
- native package 的 arity 消息是 wrong_args_with_usage 的
  command+usage 拼接 = **双前缀**（"package provide package provide
  name ?version?"）——JS 面照抄（单前缀是直觉陷阱）。
- tclsh 的 %D 是 4 位年（01/01/1970）、native 是 2 位年
  （01/01/70）——native clock.rs 的 %D 用 year%100；JS 随 native。
- parseTclInt 十进制臂必须测剥号后的 body（带号原文测 regex 会把
  负数全拒——首轮差分 -62135769600/-1 两例即此坑）。
- 文档节名顺延勘误：前人指令写 "10vicesimustertius"，但 tertius 节
  已存在（line 278）——按序列本节为 **10vicesimusquintus**。

文档同步：docs/MODULARIZATION.md（§1/§2/§4/§5 落地记录 + 数字链）、
specs/DIVERGENCES.md（2026-11-19 clock+package 外置节：%Z 定调/
require 编排/DST 消解/native 残差清单）。

## 10vicesimusquartus. 2026-11-19 ABI 版本 export + 模块化路线图 + size 轨（perf/wave2 @ e19f224..）

用户三项指令（wasi 方法层综合：①模块化 ②ABI 优化 ③侧模块化推广）
四件交付。结果层不综合的定调维持（本家 pkg 2.01MiB < wasi 2.25MB，
99 命令 + 字节码 VM），综合的是方法层。

**件1 `docs/ABI-OPT-NOTES.md`**（ABI 优化备忘录）：M1' ABI v2 面现状
（27 import + 3 global + 错误哨兵 bits==2，`abi.rs` 单一事实源）→
已做优化序（fast 调用协议五件 oo2 3.40×→1.26× / 守卫硬化 / M2
icache / M3 遮蔽感知 / wasm→wasm 直通 1.46ns）→ 候选 a-d 逐条
（内容/收益/成本/风险/时点）：a=ABI 版本 export（本轮落地）、b=
callsite_in_loop 回落协议（reopen 级，归 M5 形态）、c=链接槽出口
flush（语义承重件，挂账）、d=wasi alloc/free 面（**定调不采纳**——
本家 handle-arena+宿主 RC 更优，JIT-WASM-GC §1-2 设计理由）。

**件2 ABI 版本 export 落地**（wasi 版本协商模式移植，ABI-OPT-NOTES
§3a/§4）：模块 export `rtcl_abi_version() -> i32`，编码
`(major<<16)|minor` = v2.0（0x0002_0000；M0 三函数时代 = v1；低 16
minor/高 16 major 约定写在 `abi.rs` 常量注释）。
- 发射（`pass/assemble.rs`）：局部段末追加版本函数（复用 ty::RUN
  签名 `()->i32`；索引 fast 30/常规 28——不触碰 import::RUN/
  FAST_IMPL）+ export。
- 宿主比对：`abi::abi_version_compatible`（major 必须相等；模块
  minor 超前拒绝、更小放行）——wasmi `exec::execute` 与 wasmtime
  `register_wasmtime_abi`（顶层+proc 池共用注册体）实例化后立即
  调用，不匹配 panic 明确报错。fast/classic 池驱动不查（in-process
  发射恒匹配，真实靶 = towasm 产物与跨构建消费方，注释在案）。
- rtcl-wasm pkg 面：`#[wasm_bindgen] pub fn rtcl_abi_version() ->
  u32`（与 JIT 发射面单一版本空间——同仓同发布）；worker.js
  `await init()` 后严格相等校验，非 function/不等 → throw →
  init-error（报错文案带 `?v=` 硬刷新指引——防 HTTP 缓存旧 pkg）。
- 测试：`crates/rtcl-jit/tests/abi_version.rs` 4/4（常量编码/比对
  矩阵/发射产物 export 存在性 fast+classic/run_top 端到端）；
  `test.js` +2 断言（pkg export=0x00020000）；新增
  `test-worker.mjs`（node worker_threads 装载护栏：ready=校验过 +
  exec 探针 3/3）。

**件3 `docs/MODULARIZATION.md`**（模块化路线图）：现状盘点表
（CommandFunc 1.33MiB 结构性 retain / file+io 164KiB HostFs 保留
理由 / clock+package 85KiB 候选 / stdlib proc 非 wasm / C-float
C-lazy 已落地）→ 里程碑链（regexp-lite 97KiB→regexpmod→
spencermod→encodingmod/jsonmod→dynmod ABI v2→HostFs）→ **侧模块
ABI v1 规范正文**（数值 C ABI 五函数 `<name>_abi/_exec/alloc/
res_ptr/res_len` + envelope LE u32 记录表 tag 0/1 + 错误同文 +
崩溃面 no_std loop / std panic=abort + 独立 workspace 构建配方 +
dynmod v2 挂线协议——spencermod 120KB 逐条成文）→ 候选拆分清单
（clock+package 默认建议 JS dynmod 形态——计数面零变化；纯剔除
形态 wasm 命令计数会动，**需用户拍板**）→ 发布配方矩阵 full/slim/
core。

**件4 size 轨**（wasi 招②增量）：根 `Cargo.toml` 新
`[profile.wasm-size]`（inherits release，opt-level "s"）+
`build.sh --size` 第二产物 `pkg/rtcl_wasm_bg_size.wasm`（JS 胶水
两轨同面，部署换 .wasm 即用；两轨都过 wasm-opt -Oz）。实测数字链
（HEAD e19f224+件2）：

| 轨 | raw | pkg | gzip |
|---|---|---|---|
| perf（opt3） | 2,824,454 | 2,105,384（2.01MiB） | 783,632 |
| size（opt-s） | 2,390,402 | **1,731,902（1.65MiB）** | 674,592 |
| Δ | −15.4% | **−17.7%** | −13.9% |

运行时代价 −40-50%（wasi 线同族参照）——只用于尺寸敏感嵌入分发，
dispatcher 真机面恒走 perf 轨。

**门禁（2026-11-19）**：cargo test --workspace 全绿；judge interp
88/88 + judge jit 88/88×2（jitsh release 双 feature
jit-exec,jit-native 构建；RTCL_JIT_ENGINE=wasmtime 双轮）；真机
8933 COOP/COEP：fs-opfs-test **41/41** + dynmod-test **41/41**
（worker 启动序列含 ABI 校验，ready 正常 = 校验通过）；node
test.js 26/26 + test-worker.mjs 3/3；wasm 命令面零改动（本轮只动
export/profile，无命令增删）。

**坑**：
- worker.js 顶部的 `self.location`（?v= 参数读取）在 node
  worker_threads 链直接 ReferenceError——`?v=` 引入时的不一致
  （spencermod.js 同款用了 `globalThis.self?`）——本轮修复为
  `globalThis.self?.location?...`，node 装载护栏恢复。
- node 下 wasm-bindgen `init()` 无参走 fetch(import.meta.url) →
  undici fetch failed——worker 启动序列加 node 分支
  （readFileSync pkg 字节传入，test.js 同款）；浏览器分支原样。
- worker_threads 下 `terminate()` 使 worker 以码 1 退出——
  test-worker.mjs 的 exit 处理须以 done 标志区分成功收尾。
- 自定义 profile 名含连字符合法（`[profile.wasm-size]` +
  `cargo build --profile wasm-size` 实证）。
- 真机 dynmod 页的"初始命令计数"是动态基线（本轮 HEAD = 102，
  host-http 合并后的面）——T1/T9 按动态基线比对，wasm 初始 99 的
  历史数字不适用于现 HEAD 的绝对值断言。

## 10vicesimustertius. 2026-11-19 feat/rtcl-wasi rebase——WASI embed ABI 蓝本激活（perf/wave2 @ 0182b7c..）

用户定调：origin/feat/rtcl-wasi（WASI embed ABI 设计资产，fork 自 d71ed32d，
17 提交 2026-10-18..20）rebase 到新 master 使其成为**活体蓝本**——瘦身配方
+ abi-v2 版本协商是保留核心。rebase 在独立 worktree（/tmp/wasi-rb-wt，本地
分支 wasi-rb）执行，主 checkout 未切分支。

**rebase 结果**：17 提交 = 16 原样重放 + 1 skip + 1 新修复提交（cb55794）。
停点仅 2 处（预期 3——wasm-size profile 提交 de4ba33 的 `[profile]` 段
上下文恰好无冲突自动并入）：
1. 3de75df 根 `Cargo.toml`：members 取 master 全量列表 + 追加
   `crates/rtcl-wasi`；`Cargo.lock` git 自动合并（构建时 cargo 校正）。
2. a0999c4（http.tcl 词表同步）：master 现形是演进超集（6 参格式 +
   `namespace which` 探测，10vicesimussecundus 轮已收编同源）——**skip
   丢弃**；wasi guest 桥说明随蓝本活在 rtcl-wasi 自带的 wasi-bridge.tcl
   与 host.rs 注释里。
wasi 线自身文件（rtcl-wasi crate + tools/rtcl-wasi-smoke）与原分支**零
diff**——蓝本逐字节保留（C ABI 面 `rtcl_{abi_version,alloc,free,init,
eval,result_len,result_copy,error_len,error_copy}`、host.call 帧协议、
guest host.rs 绑定、wasm-size profile、abi-v2 feature 全部未动）。

**编译对齐点**（rtcl-core API 面几乎零漂移，主战场在 rtcl-vm 特性门）：
- `Interp::{new,eval,set_clock,load_http,register_command,set_var}`、
  `Value::{as_str,from_str,empty,as_list}`、`Error::runtime(msg,
  ErrorCode::Generic)`、`host::HostClock`（now_ms/sleep_ms + `Send`）、
  门面 features `std/regexp-lite/clock`——master 全部在位，wasi 侧
  无需改动。
- **cb55794 fix(rtcl-vm)**：10vicesimussecundus 轮记录的"rtcl-vm 纯 std
  组合既有缺口"正面修复（当时 rtcl-native 靠加 `std,file,io,json`
  绕过）——`copy_jobs` 字段/初始化随类型走 `io` 门（chan_io 模块本身
  io 门控，原 std 门漏了依赖）、`event.rs` process_events 的 chan copy
  泵送块补 `io` 门、`io.rs` fs_access 门 `file`→`std`（纯 std 实现，
  NativeFs 的 readable/writable/executable 探测需要；file⇒std，native
  构建行为不变）。修复后 rtcl-wasi guest 面（std+regexp-lite+clock）与
  rtcl-wasm 裸 std 面都能编译。

**门禁数字（2026-11-19，wasm32-wasip1）**：
- `cargo check -p rtcl-wasi --target wasm32-wasip1`：全绿（33 个警告全是
  裁剪组合下 master 既有死代码）；宿主 `cargo check -p rtcl-vm`（默认
  特性）全绿。
- release 两构建面：不带 feature 与 `--features abi-v2` 均过；产物
  rtcl_wasi.wasm 2.25MB（历史链 3.10→2.28MB 同族）。
- smoke（wasmtime 48.0.6 独立 lock，与 workspace lock 无冲突）：
  - plain：`ABI: 1` / expr=2 / puts 打印 hello-rtcl-wasi / bAnAnA /
    100k loop=4999950000（~60ms）/ error status=1 "boom" / 1M loop=
    499999500000（~583ms）——ALL PASS。
  - abi-v2：`ABI: 2` 协商过（init(2)→handle 1），行为同上（~56ms/~555ms）
    ——ALL PASS。
  - 历史基线 14ms/145ms → 现 ~60ms/~580ms（≈4x：宿主机差异与 fork 后
    解释器演进的综合漂移；**行为零分歧**）。

**蓝本状态**：origin/feat/rtcl-wasi = cb55794（force-with-lease，
e5b23c8→cb55794）。临时 worktree 已删、本地临时分支已删。

**坑**：
- `cargo … | tail` 管道吃退出码——编译失败时 tail 仍返 0，错误日志必须
  落文件再 grep `^error`。
- `--force-with-lease=<ref>:<expect>` 手写 expect 值易错（假性 rejected）；
  无参形式以远程跟踪 ref 为基线即可。
- `/home/nzinfo/src.note/rtcl-wasi` 现存 worktree 的本地 `feat/rtcl-wasi`
  仍停在旧形态 e5b23c8——要用需 `git fetch && git reset --hard
  origin/feat/rtcl-wasi`（= cb55794）。
- wasm32-wasip1 与 wasm32-unknown-unknown 构建产物同写 target/ 不同
  target 子目录，互不覆盖但都要占构建时间。

## 10vicesimussecundus. 2026-11-19 feat/host-http 合并——native 宿主服务面（HostHttp / fd 表 exec / HostJobs）契约对齐（perf/wave2 @ f9101c9..）

用户定调：合并 feat/host-http（native 侧进程自动化面）并把契约对齐到当前
master。分支 fork 自 f25d2340（jit M2 收尾），17 提交，2026-10-20 收尾；
分叉后 master 走了 ~470 提交（rtcl-core→facade、HostFs 13 键数据面、
hidden 桥注册契约、exec native `&` 收割面）。文本 merge-tree 预检"仅
Cargo.lock"是旧口径——实际冲突 8 文件 + 1 个 modify/delete。

**合并结果**：
- 冲突：`.gitignore` / `Cargo.lock`（重建）/ `Cargo.toml`（members）/
  `rtcl-cli/{Cargo.toml,main.rs}` / `rtcl-core/{Cargo.toml,lib.rs}` /
  `rtcl-data/src/yaml.rs`；
- **modify/delete**：`rtcl-core/src/host.rs`——master 已把 interp/host 迁
  rtcl-vm（git 对 interp 四文件识别 rename 自动并入，host.rs 两侧改动过大
  未识别）。分支的 exec/job/http 契约段手工移植进 `rtcl-vm/src/host.rs`
  （门面 `pub use rtcl_vm::host` 保住 `rtcl_core::host::*` 公开路径）。

**契约对齐清单**（分支假设 → master 现状）：
1. **HostFs::open 同名异构**：分支给 HostFs 加 `open(path, FdMode) ->
   Box<dyn HostFd>`；master 数据面 HostFs（13 键）已是 `open(&mut self,
   path, mode:&str) -> (Box<dyn Channel>, bool)`。**不并入、不重构宿主
   抽象**——fd 表是 host.rs 的兄弟层：新增兄弟 trait `HostFdFs`（fd 视图
   文件源），Interp 字段 `fd_fs` + `set_fd_fs`/`fd_fs`（分支的
   `fs`/`set_fs`/`host_fs` 访问器撤销，master 的 `host_fs` 字段/数据面
   原样）。docs/JOB-CONTROL.md 的 trait 清单同步改名。
2. **exec 路径**：master native 路径（std::process + `background_children`
   收割，10bis 轮）原样保留；分支 host-backed 面（`cmd_exec_host`/`job`
   命令，fd 表驱动）在 `interp.host_exec()` 注入时才分派——rtcl-cli 不
   注入，native `exec` 行为零变化。tty job prompt（SIGTSTP/SIGINT）随
   exec feature 编译进 rtcl-vm：**rtcl-vm `exec = ["std", "dep:libc"]`**
   （fork 时 libc 挂在 rtcl-core 的 target 段，随迁移归位）。
3. **net 撞名**：rtcl-vm 的 `net` = TCP socket（10ter 轮）；分支 rtcl-core
   的 `net` = HTTP 缝标记。门面 `net = ["std"]` 本地标记、**不做**
   rtcl-vm/net passthrough（注释钉死两个 net 不是同一 feature）。
4. **http.tcl 桥线格式 5→6 参**：分支加 `-headers`（缺省空串）；
   `rtcl-core/tests/http_tcl.rs` 两处断言随改。**cancel 探测漂移**：分支
   的 `::http::cancel` 用 `[info commands __rtcl_http_cancel]` 探测——
   master 已把宿主桥注册为 hidden 命令（10ter 轮修 `_require_bridge` 同
   款），native 桥下 cancel 恒报"not supported"；对齐为
   `[namespace which -command]`。net 单测 7/7 抓出后修复。
5. **feature 图**：rtcl-native 的 rtcl-core 依赖 `default-features=false +
   ["std"]` 在 master 上编不过——10quinter 解耦后 rtcl-vm 纯 std 组合是
   **master 既有缺口**（NativeFs→`io::fs_access`、chan_io 字段非门控），
   与本合并无关但被 rtcl-native 暴露。rtcl-native 显式带
   `std,file,io,json`（json = 门面新增 passthrough——合并后 http.tcl 的
   opts 用 `json encode` 构造）。exec/env/signal 门控未动，wasm 子集
   （std,file,io,clock,package,dynmod,json）不含 exec/net——`job` 命令
   不进 wasm，计数面不变（wasm 初始 99）。
6. **yaml_to_value 保留**：rtcl-data 的 wasm 宿主免 interp API，基于门面
   `bridge`（null 语义与 `cmd_yaml` 一致）；与 register_entry 宿主 API
   并行，CLI 面未动。
7. `pipe.rs`（内存有界 SPSC）随 interp 迁 rtcl-vm/src/pipe.rs（std 门控）
   + 门面 re-export 保 `rtcl_core::pipe` 路径；与 host.rs 的 `ExecIoError`
   分层（运行时原语 vs 宿主 ABI 投影），channel.rs 无同类实现，不合并。

**门禁数字（2026-11-19）**：
- `cargo test --workspace`：**1451/0**（基线 1443 + host.rs fd/job 单测 4
  + pipe.rs 单测 4；exec_cmd.rs 17 个 master 本就有）。
- `cargo test -p rtcl-native --features net`：**7/7**（feature 门控，
  workspace 默认构建不含；mock 服务器流式面）。
- judge interp：**88/88**。
- feature 组合：rtcl-vm `--no-default-features --features std,file,io,
  dynmod`（无 exec）过；rtcl-core 门面 `std,file,io` 过（门面本就无
  dynmod passthrough，master 既有状态）。
- `cargo check -p rtcl-wasm --target wasm32-unknown-unknown`：过。
- net 冒烟：`--features net` release 构建 + http 词表层装载过。

**提交链**：merge commit（含全部契约对齐——原始 merge 树不编译，对齐是
可编译性前提）+ 本节。DIVERGENCES 无新条目（无 tclsh 行为分歧，全部是
Rust 宿主 ABI 面）。

**坑**：
- merge-tree 旧式（三参）输出截断看不全冲突，用 `--write-tree --name-only`。
- git rename 检测对大改文件失效：`removed in local` 段落要人工核对落点。
- 门面缺 vm 后加 feature 的 passthrough（json）——依赖方组合报
  "does not have that feature" 时先查门面 manifest。
- rtcl-native standalone 的 feature 组合是 rtcl-vm 纯 std 编译面的唯一
  现役探针（workspace 构建被 rtcl-cli 全集掩盖）。

## 10vicesimusprimus. 2026-11-18 C-float——float 快车道全验收（jit-float-lane 默认 off；perf/wave2 @ e488372..）

docs/JIT-WASM-GC.md §5 落地。前 agent 因宿主运行时崩溃中断于发射面半
成品（工作区 ~312 行未提交），本节接续完成并实证修复九处。

### 交付面（e488372 件①基建 / a09cde9 件②发射 / fec274f 件③探针）

- 开关：cargo feature `jit-float-lane`（默认 off）→
  `EmitConfig::default().float_lane = cfg!(feature)`——全消费面
  （jitsh/rtcl-cli/fast 协议/classic 池）自动接线。**语义不依赖开关、
  性能才依赖**（off = float 全走 binary_op 路线 A 行为，探针/battery/
  judge/fuzz off 轮全绿背书）。
- 基建：`Value::as_float_rep`（Float-rep-only 读，is_imm → None、不
  shimmer——解释器零调用，语义零变化）+ `unbox_float_raw` 改走守卫
  语义 + `Lane::Float` 注记（annotate_lane 休眠 pass 的展示面落点，
  plan_guards 不消费——汇合 meet 未做）。
- 发射（assemble.rs）：Arith/Cmp/Neg 四层块 [done, host, flt, intp]，
  imm-imm → int 快臂；Float rep/混合 → unbox_float 守卫 + f64 内联 +
  box_float 边界；其余 → binary_op 宿主全语义。float_capable = 加减
  乘除 + 全比较（IEEE754 = C double）；transcendental/pow/mod/位/移
  位不进（宿主管，§5 语义清单）。

### 接续修复九处（前三枚是前 agent 半成品的硬伤）

1. **缺 End**：三个 float 分支 4 块开 3 闭 → wasm 校验拒绝（control
   frames remain）——补 host 块 End。
2. **imm 守卫 br 目标**：BrIf(1) 实落宿主区、float path 死码——br N
   = 跳到第 N 层块**末尾之后**；float path 须 br 0（退出 intp）。
3. **宿主入口目标**：操作数 fetch 落宿主写 BrIf(4)（= host label =
   **连宿主代码一起跳过**）→ dst 不写 → 返回操作数 a 原值
   （`$r+1.0` 产 1.5）——宿主代码在 flt End 之后，须 BrIf(3)。`br N`
   语义与 imm_guard_br_slow 的历史注释（「br 0 落慢路径 / br 1 跳过
   慢代码」）同构互证。
4. **imm 检查前置**：unbox_float 入参经 I32WrapI64——大 imm tagged
   值（2^54+3）截断成小偶数 → 被当句柄探进 arena → clone 越界值
   SIGSEGV（2^62-1 截成 -1 探空侥幸存活）。重排：imm 先分派，真句柄
   才 unbox。
5. **Mul 守卫深度参数化（HEAD-era 潜在 bug 首曝）**：classic 布局内
   硬编码 BrIf(1) = done——跳过宿主且 dst 不写，`3037000499*7` 产陈
   旧 dst。emit_int_inline 挂 slow_br（classic=0 / float ctx=1），
   Add/Sub 溢出、Mul 大操作数、Div/Mod 除零、Shl/Shr 移位域全部统一。
6. **Div 0/0**：float-lane 落 f64 会把 `7/0` 算成 Inf——除零守卫落宿
   主（报错全语义）；`7.0/0` 保持 IEEE Inf 内联；`0.0/0.0` → domain
   error（tclsh 拒造 NaN 除法结果，与 expr_ops.rs 同款）。
7. **比较语境 exact_only**：tclsh 对 wide/double 比较**精确裁决**
   （`9007199254740993 > 9007199254740992.0` = 1，f64 提升会收缩成邻
   值 → 恒等分歧）——比较语境 imm 提升仅限 |v|<2^53，越界落宿主；
   算术语境不设限（tclsh int→double 算术提升同样舍入）。
8. **NaN 操作数守卫**：interp binary_op 对 NaN 值操作数报错（NaN 可
   经 Inf*0.0 存在但禁再入算术；NaN 比较 = 0、Neg = NaN 不报错）——
   fa!=fa → 落宿主。
9. **fused float 臂**：Cmp 融合表命中时 float 臂 raw cmp 直写 b(br)
   （真值 ≡ 慢臂 (tagged≠1)），int 快臂/float 臂/慢臂三臂贯通。

### 验收与数字

- 门禁：float 探针 15/15×2（lane-off/on，tclsh 8.6.17 oracle）、
  jit_battery 52/52×2、judge interp 88/88 + judge jit 88/88×2、
  cargo test --workspace 全绿、fuzz seed7 双变体 0 分歧、六基准
  （默认 off）两轮无回退（rtcl 纯解释器构建不含 rtcl-jit）。
- 发射命中实证：RTCL_JIT_DUMP_PROC fast_impl f64 op 6 条 vs lane-off
  1 条。新基建：`RTCL_JIT_DUMP_PROC=<path>`（proc 模块落盘——
  RTCL_JIT_DUMP 只覆盖顶层单元，第 3/4 条修复靠它定位）。
- 探针固化：bench/scripts/jit_float_probes.sh + f01-f15（类型分派/
  混合提升/NaN-Inf/比较/循环/Neg/宽整提升/串渲染/while 条件/mod 报
  错/transcendental/int 溢出/精确比较/除零分界面/NaN 生命周期）；
  preexisting 首行标记机制与 jit_battery.sh 对齐（jit==rtcl 强制）。
- **预存分歧记录**（非本节引入，预存面归 judge/specs）：tclsh 8.6
  对 `Inf*0.0`/`Inf-Inf` 的 NaN 乘积报 domain error，rtcl interp 放
  行 NaN（除法 0/0 的 domain error interp 已对齐）——f15 按电池惯例
  标 preexisting。
- 开关保持默认 off；翻默认需 fast 路径 float 微形状基准（当前 float
  热点场景未成 bench 案例）——reopen 线索。

## 10vicesimus. 2026-11-17 C-lazy——lazy-string 全验收（rtcl-wasm 默认 on，独立于 JIT；perf/wave2 @ e81c64a/01e1431）

docs/JIT-WASM-GC.md §6 落地（两笔：件① rtcl-vm 基建 / 件② rtcl-wasm
接线）。JS 起源字符串默认保持 externref JS 原生形态，首次字节操作才
物化（JsOnly→Dual）；Tcl→JS 反向穿越缓存池索引（NativeOnly→Dual）。
即「跨边界的 shimmer」。

### 要点（详见两笔提交 body——本节为接管索引）

- choke point：`Value::as_str` 的 OnceCell 物化收窄为唯一惰性钩子入
  口——强制物化清单（list/dict parse、string map/match/…、regexp、
  expr、binary、eval）零代码成本。
- `InternalRep::HostStr(u32)` + `HostStringBacking` trait（thread_local
  注册表 = 开关；非 JS 宿主无人注册 → 零成本）+ `JsStringPool`
  （Vec<Option<Slot>> + 空闲栈 + 槽位 rc，Clone/Drop 手工实现跟随
  COW 深拷贝/回收）。
- **白名单裁决修正**：string compare 不进白名单——§6 原论证有误
  （UTF-16 码元序不保码点序，astral 混合内容次序会分叉），等 specs
  astral probe + judge 裁决后再评估；string length 照 §6 留强制物化
  侧（JS .length 数 UTF-16 码元 vs rtcl 现状三语义未实测前不冒险）。
- lone surrogate 防线：JS→Tcl 统一入口 is_valid_utf16() 一次校验，
  含孤立代理立即有损转码（U+FFFD 兜底）永不入池。
- 顺修：channel.rs std_fd_seek 补 cfg(unix) 门（713773f 起非 unix
  构建断了三天）。
- 量化（www/lazy-string-bench.html A/B 交替 3 轮中位数）：echo 回传
  12.8KB −41%、puts console 1.3KB −32% / 12.8KB −56%、1.3KB echo 持
  平（解析主导）——「转码次数 2→0」实证。
- 真机：fs-opfs-test.html 41 pass / 0 fail；dynmod-test.html 81 PASS。
- 门禁：workspace 全绿 + judge interp 88/88 + judge jit 88/88 +
  jit_battery 52/52。RTCL_DISABLE_LAZY_STRING=1 = 全转码基线回退标志
  （A/B 与保险共用）。

## 10sexter. 2026-11-13 分派面 retain 优化：wasm-opt 接入 build.sh（-8.5%）

"1.33 MiB 被 CommandFunc 表 retain"的语义澄清：elem 段本身仅 877 B
（函数索引表）；1.33 MiB 是**只能经 fn 指针间接到达**的命令函数体
——LLVM 看不到直接调用点，无法内联/常量折叠，各自独立成函数（含
错误路径/参数校验），且不可与其他命令去重。这是**尺寸现象**而非
死代码。

**性能面证据**（间接调用≠瓶颈）：
- 六基准已全部 ≤1.0x tclsh（fib 0.83/oo2 0.42/oovars 0.52，HANDOFF
  §10.2）——热路径不踩这张表：Tier-1 命令（set/if/while/foreach/
  incr/expr…）在 vm_exec 里已编译为操作码，循环体走 Inner/Special
  指令，fn 指针只服务非编译命令与动态分派。
- V8/wasmtime 对 call_indirect 做 inline cache（单一 callee 热点直接
  内联）——单态分派点的间接调用开销接近直调。
- 当前画像的真热点仍是 §10.2 五项（call_proc 帧准备/语句窗口/
  word_srcs 借用化/空循环基线/vars 存储），与分派面无关。

**尺寸优化已落**：`wasm-opt -Oz --all-features` 接入 build.sh（LLVM
之后的第二轮清理：不可达分支/内存访问模式/函数体内去重）——pkg
2,318,850 → **2,121,993 B（2.02 MiB，-196 KiB / -8.5%）**，gzip
792 → 791 KiB（bindgen 已剥 name section，压缩面收益小）。真机 26/26
全绿（--all-features 因产物含 trunc_sat/simd MVP 后指令）。profile
已是满配（lto/codegen-units=1/panic=abort），LLVM 侧无再降空间。
跳过开关：WASM_NO_OPT=1。

**后续可挖**（均需真机回归护栏）：薄包装合并（cmd_fcopy→chan_copy
式 delegator，量级 <20 KiB）；§10quater 路线的子系统外置（chart/
json/regexp-spencer 移侧模块，其分派面随之出表）。

## 10septem. 2026-11-13 子系统外置第一刀：json → JS dynmod（33/33 真机）

- **rtcl-vm**：`json` feature 门控（rtcl_std 含，native 不变；wasm
  子集剔除——json::decode/encode/json 三条目 + 模块）。净重 38 KiB
  raw（1102 行纯解析器，比 file+io 的 164KiB 小——外置收益第二梯队，
  架构演示价值优先）。
- **www/jsonmod.js**：JS 同面实现——JSON tokenizer **保留数字原文**
  （JSON.parse 会把 1.50 归一成 1.5，native 是文本保留）+ Tcl 序列化
  （对齐 serialize_list/tcl_quote：needsQuote/balanced/backslash）+
  Tcl list 解析（encode 值输入）+ schema 语法全移植（str/num/bool/
  obj/list/mixed + brace words）+ Inf/-Inf 原文输出。decode/encode/
  -index/-null/-schema 全支持。
- **dynmod 参数协议升级**（join 分隔符）：wrapper 体改为
  `[join $args "\u0001"]`——**发现 rtcl 的 {*}$args 展开对含空白
  元素有重解析拆词缺口**（`::json::decode {{...}}` 的 JSON 串被拆成
  25 碎片；Tcl 语义 {*}$args 元素应整词传递）——join 产出的单词
  任何路径都不再拆；js_dispatch 按分隔符拆开还原。**expand 拆词是
  rtcl 通用正确性 bug**（judge 未覆盖 {*}+含空白元素组合），记 OPEN
  待修（修后 wrapper 可回 [list {*}$args] 形）。
- **真机 J1-J7**（dynmod-test.html）：decode/encode/-index/-null/
  schema/嵌套 obj/数字原文保留全过，合计 **33/33**。中途真机抓出：
  apply(thisArg,args) 的 this 不进参数数组（JS handler 需 rest 参数
  `decode(...args)`）；CASES 数组被注入截断（node --check 语法护栏）。
- 回归：native 全绿（test/judge/probes/scripts）、dynmod 单测按 join
  契约更新（5/5）、wasm check 过。
- **后续**：expand 拆词修复（正解）；regexp-spencer 外置（大刀，需
  字符串 ABI v2）；OPFS worker fs 形态。

## 10tertio. 2026-11-13 ABI v2——dynmod 参数协议升级（列表值单词直传）

- **动机**：v1 join 协议（`[join $args "\u0001"]`）对**数据含
  \u0001 的参数丢界**（分隔符歧义，宿主拆分处无法区分）。
- **v2 协议**（DynAbi::V2List）：wrapper 体 `<dispatch words>
  {*}$args`——参数经 rtcl 的 {*}$args 列表展开**逐词直传**（expand
  元素感知，specs/expand.probes 双路径护栏），无分隔符——任意
  Unicode / \u0001 / 空白数据整词保真。
- **rtcl-vm**：DynAbi 枚举（V1Join/V2List）+ DynModuleDef.abi +
  register_dyn_module_v2；load 模板按 abi 分支。
- **rtcl-wasm**：register_module **默认 v2**（DynModuleDef.abi =
  V2List）；js_command 的 __dynmod 分支按 v2 wrapper 传形逐词转发
  JS fn（rest 参数契约）。
- 真机 fs-opfs-test.html 29/29 维持（O/R/S/E 全矩阵回归）。
- **共享内存零拷贝模式预留**：侧 wasm `--import-memory` 共享 rtcl
  线性内存 + (ptr,len) 直调——版本握手（abi_version）已预留，真需
  1MiB 级零拷贝时实现。
- 规约：www/ABI-V2.md。

## 10undecimus. 2026-11-13 外置第二刀 regexp + encoding 命令面 + SAB durable 桥（29/29 真机）

- **regexp 外置**（956648e + 7382ab4 + cdd3b71）：wasm 子集剔除
  regexp-lite（净重 97 KiB raw）；www/regexpmod.js = Tcl ARE → JS
  RegExp 语义子集（POSIX 类展开表、\y→\b、\m/\M/\</\> 近似、
  quoting ***=、-nocase/-line/-lineanchor/-linestop/-all/-inline/
  -indices/-start/-about 全支持）。**变量写入协议**：matchVar/
  subMatchVar/varName 在解释器帧——JS 返回 __RXCAP/__RXIDX/__RS
  结构化 Tcl 列表编码，::regexp/::regsub 语义 wrapper proc 解包 +
  upvar 写变量 + 透传返回码。**-indices 组起止已升级 V8 hasIndices
  （d flag）精确 [start,end)——嵌套 (a(b(c))) 与相邻重复 (a)(a)
  与 native Spencer 同精度（indexOf 近似曾错位）**。
- **三个高频 wrapper 错位**（真机逐一抓出，外置带变量命令的通用
  教训）：① 命名空间——JS 面 fn 装 ::模块名::fn 名（::regsub::
  regsub 是错名，正确 = ::regexp::regsub）；② __RS 解包顺序（v=
  result、返回=count 易反置）；③ POSIX 类 `[[:name:]]` 前置检测
  （外层 [ 内嵌 [:）。wrapper 定义后必须逐条真机验证。
- **encoding 命令面外置**（feature `encoding` 门控，wasm 子集不含；
  通道机制的 encoding.rs 保留）：www/encodingmod.js = 86 名宽表 →
  WHATWG label 映射 + TextDecoder/TextEncoder 数据面（非 utf-8
  encode 走反查表合成；symbol/iso2022 系/ebcdic 如实报
  unsupported——全集仍在 native）。E1-E7 真机（names/system/
  convertfrom/convertto latin-1 与 utf-8 与 cp936/dirs）。
- **SAB 严格 durable 桥**（见 10decimus 后补段）：fs-worker.js
  （OPFS 执行端，sync access handle + meta sidecar settimes）+
  rtcl-worker bridge 模式（Atomics.wait 等回执，回执在写入完成后
  发出）——terminate 后 strict 标记存活实证。
- **fs-opfs-test.html 合计 29/29**（O 区 12 + R/S 区 10 + E 区 7），
  均在 COOP/COEP 隔离服务器（/tmp/isolated_server.py，8933）。
- 回归：native 全绿（test/judge/probes/scripts——native Spencer 面
  不受影响）、wasm check 过。尺寸：wasm raw 2.84 → 2.74 MiB（-97
  KiB regexp-lite）；pkg（bindgen+wasm-opt）≈2.0 MiB。

## 10undequincesimus. 2026-11-16c JIT M3：epoch 接 Tier1 bump 信号（perf/wave2 @ 99fb254）

### 盘点定谳（epoch 生命周期表）

**bump 点**（写者，全部经 `note_tier1_mutation` 叶名过滤或
`note_tier1_sweep` 无条件）：proc 定义（proc.rs）/rename 新旧双键
（proc.rs，rename 的所有类型共用）/alias 创建（introspect）/宿主
register_command×2 + delete_command（registry）/ns import 移除+绑定
（namespace）/oo attach+detach（oo）/local proc 帧出口拆除（call.rs
两处）/ns delete + ns forget sweep（namespace 两处）——**M3 新增两
点**：`trace add execution` 成功路径 sweep（trace.rs）与
`namespace ensemble create` 叶名过滤 bump（namespace.rs ns_ensemble，
ensemble 注册在 dispatch 链 builtins **之前**，叶名撞折叠名单即从
折叠 op 下方遮蔽内建）。

**读点**（解释器）：eval.rs 单元入口门/体 memo 逐迭代门、call.rs
Site A 四门（slot + 体执行）、vm_exec exec_bytecode **逐 op** epoch
检查（083537a，bump 即从命令边界树走查重分派）、bytecode_applicable
（exec_traces 活检，与 epoch 并列）。

**读点**（JIT）：run_top_with 入口 gate（exec.rs:162）→ Fallback；
proc 钩子防御重查（proc_exec.rs:67）；classic 面宿主分派回边比对
（host.rs call_argv_raw Ok 臂，bump → STALE(5) → 驱动
jit_redispatch_after 树走查续跑）；fast 面 sh_call_dispatch 同款
（fast/shared.rs）；fast/classic 池 key 含 epoch（池命中隐含相等，
global 免 set 依据）；wasm 内 epoch global 由 refresh_epoch 在 impure
路由同步。**M3 新增**：run_top_with 与 execute_artifact_wasmi 入口
`has_exec_traces()` 门（拦「trace 先于单元注册」的嵌入式窗口；
jitsh 单文件单元里中途 add 由 bump/回边比对接住）。

### 缺口定谳（三个正确性修复）+ 收窄评估

- **件1 trace 可观察性**（876c382）：执行 trace 只在分派/树走查面
  触发，折叠 op 绕过分派——单元中途 `trace add execution <折叠名>`
  对其后折叠 op 全盲（h2b：interp 字节码漏触发；h5：JIT set 宿主臂
  set_var 直写漏触发，incr 仅靠 call_argv 置 caching=0 的副作用走
  站点 re-eval 侥幸可见）。bump 后逐 op 检查/回边比对同点重分派，
  bytecode_applicable 活检压树走查（粒度=任意执行 trace 全关折叠
  世界，与 tclsh traceCount>0 不编译同构）。
- **件2 ensemble 遮蔽**（3e717f0）：`namespace eval incr {namespace
  ensemble create}` 从折叠 op 下方遮蔽内建（h3：tclsh/树=unknown
  subcommand，interp/jit=n=6）。叶名过滤保非 Tier1 名零成本。
- **件3 遮蔽感知折叠**（8ed5e12+99fb254）：epoch 戳只防「编译后」
  遮蔽（bump 退役既有单元）；**遮蔽存在时新定义的体盖章新 epoch 过
  Site A 门**——折叠 op 绕过分派、遮蔽体不可见（h1：interp 字节码
  与 JIT proc 钩子输出 6，tclsh/树=can't read "x"）。三个体编译点
  全接（cmd_proc / apply lambda 缓存 / OO memoised_proc_def），
  编译器 skip_folds 名单内名字 compile_dyncall（分派活绑定）。
  **件3b 两重精确化**（99fb254，初版回归定谳）：内建经
  register_builtins 全体落 commands 表（register_entry 直插不
  bump）——逐叶名命中即满名单、折叠全体降级（rtcl-vm 内联帧族 3
  测 + judge(jit) 86/88 实证）。修正 = ①内建本体非遮蔽
  （registry::canonical_builtin_fn fn 指针对照，同表集同 cfg 门）；
  ②裸名解析语境限定（键 ns ∈ {全局, 当前 ns}，与 dispatch 裸名链
  同构——::tcl::dict::incr 与裸 incr 永不相交）；跨 ns 移动的
  lambda 体用 any-ns 保守口径。
- **折叠名单单源**：`rtcl-parser::compiler::TIER1_FOLD_LEAVES` 常量
  （编译器折叠 match / note_tier1_mutation bump 表 / 遮蔽探测三处
  共用）——lappend 漏网那类双表 KEEP-IN-SYNC 风险归零。
- **收窄评估（结论：不做，保守 bump 是安全默认）**：现 bump 语义 =
  「任意 Tier1 相关突变作废全部编译单元 + JIT 永久 Fallback（顶层）
  /proc 体树走查（既有体）」。精确化方向（per-leaf 13 路 epoch 向量
  或「体编译后新定义即重编译」）只在「突变 Tier1 叶名」的负载里有
  收益，judge/battery/六主基准都不含该形状——收益不显著，记 reopen
  （落点：ByteCode 加 per-leaf fold epochs 位图 + Site A 门逐叶
  比对；compiler 需Interp 遮蔽集穿透才可能重启折叠）。

### 探针与验收

- 电池 b48-b52（47→52）：proc 重定义中途遮蔽（e1 现状固化——旧头注
  vm_exec=6 已过时，083537a 重分派落地后四方=5）/rename 中途遮蔽/
  trace add execution 到折叠名/ensemble 叶名遮蔽/遮蔽后 proc 体
  （errorInfo 逐字节）。jit_epoch_probes.sh e1-e5 现状：四方一致
  （e3/e5 的 tclsh unknown 自引用死循环 = 预存 unknown-handler 分歧，
  非 epoch 面）。
- **验收**：judge interp+jit 88/88×2、jit_battery 52/52 三方逐字节、
  workspace tests 1420/0（dev）、release tests 816/0；微基准 sanity
  arith_loop jit 12ms / fib25 7ms / var_incr 13ms（jit≥interp，无
  折叠降级信号；六主基准 rtcl 侧零 JIT 天然不受影响）。顶
  rtcl-shape 探针 p516b（proc-old-5.16 独立复现）三方逐字节。
- **坑两枚**：① `cargo build -p rtcl-jit` 默认空特征**不产 jitsh**
  （bin required-features = jit-exec）——构建必须 `-p rtcl-jit
  --features jit-exec` 或 workspace 全量；拿陈旧/异特征 jitsh 跑
  探针会得到「h5 miss」的假阴性（本轮实测）。② worktree 二分
  （git worktree add /tmp/... <commit>）是「不切分支」约束下的
  干净 bisect 法——F2/F3 worktree 各 6-7 分钟构建换定谳。
- 工作树遗留观察：docs/UI-GENUI.md 有非本任务未提交改动（另一会话
  的生态调研），未动未纳入。

## 10senioquincesimus. 2026-11-16b JIT M2 件1：slot 单态缓存（perf/wave2 @ 40bcae8）

### 盘点定谳（M2 开工前）

- **M2 目标的「slot 缓存」在 fast tier 已大部存在**：C1/C4 波次落下
  的 fast-tier per-depth 内存槽（PARAMS 区）+ 零检查守卫 + lazy-flush
  脏位 + CallSite 边界 flush/resync，就是「slot 单态缓存」的内存形态
  ——fast-tier LoadVar 每读 ~9 条 wasm 指令，缓存 local 化收益 ~3 条
  ，不划算（件3 止损依据）。
- **真正的缺口在 classic tier**（顶层 slot 单元）：LoadVar 每读走
  `caching global 读 + VALUES mem load + 零检查`（~12 条），IncrVar
  ~41 条——M2 件1 落点。

### 件1 落地（40bcae8）

- classic tier per-var icache（tagged i64）+ ivalid（i32）wasm-local
  组；有效臂双门 `caching==1 && iv==1`；LoadVar/IncrVar 有效臂省
  mem load + 零检查（IncrVar 在缓存字上 untag/add/range/retag，写同
  步落 VALUES+STATE=2——保守每写落盘，写合并不做：b47 类 trace/
  别名可观察性依赖逐 op interp 面）。
- 失效完备清单：caching=1→0 的全部宿主 import（binary_op/call/
  call_argv/foreach×2/eval×2/lappend）后整表 invalidate（哨兵分支
  前——kind!=0 会 br 回循环）；caching=0 世界 icache 由 LoadVar/
  StoreVar host 臂逐 op 镜像。**防抖不需要**：缓存态是单次调用局部
  ，fill 4 条指令 vs 宿主调用 ~200ns，反复失效的额外成本 ~2%。
- 坑两枚：① wasm local 索引跨类型组连续——i64 组扩容后 i32 侧全部
  绝对基址要平移（fast_foreach_base/slot_base_local/bool_base/kind
  +fb_ic）；② br_if 极性（非零跳转）——首版有效臂漏 `i32.eqz` 被
  b16 差分电池抓出（foreach 循环变量读成垃圾→expr 空串错）。
- 新基建：battery b44-b47（命令失效/upvar 别名/global 别名/trace
  存在，43→47）；RTCL_JIT_DUMP=<path> 模块校验失败落盘。

### 验收与数字

- 门禁：judge 88/88×2、jit_battery 47/47 三方逐字节、workspace
  tests 全绿、fuzz 400（seed 7）5 分歧全部预存（基线 c889caa 与
  40bcae8 jitsh 输出逐字节一致）。
- 数字（jitsh best-of-9 交错，ms）：var_incr 13→11（−15%）、
  arith_loop 15→12（−20%）、fib25 23→22、proc_fib 10→8、
  dloop/fe/build 噪声带持平；六主基准 rtcl 侧无回退（rtcl 纯解释
  器构建不含 rtcl-jit，天然不受影响）。
- **边际判断**：var_incr/arith_loop 的 JIT-vs-interp 差距已闭合
  （jitsh 11/12ms vs interp 10/9-13ms）；件2（lane 类型化后端消除
  Cmp/Arimm imm 守卫，估再 −15-25% 仅微形状）与件3（fast-tier 臂
  ，估 ~5%）按止损规则记 reopen 线索——annotate_lane 的 out_lane
  字段是件2 的天然落点。
- jitsh oo2/oovars 3.3-4.3× tclsh 为 M2 前既有（execute_fast 每次
  proc 调用全协议 ~1.4μs vs interp ~200ns）——独立于 slot 缓存，
  reopen 归「fast 调用协议」专项。

## 10undevicesimus. 2026-11-17 JIT M4 定义即编译——用户 proc 定义点覆盖 92.7%、emit p50 19-64μs、启动税清零（perf/wave2 @ 0bf0564）

里程碑表重定位版落地（docs/JIT-WASM-GC.md §9 M4 / §7.4 Chez/Racket
CS 模式）：per-lambda 懒编译/tier-up 阈值已证伪降级，「默认 proc 定
义即编译 + 解释器兜底」转正。热点计数仅驱动可选画像增强重编译——
本节未做（reopen 线索，非默认路径）。

### 现状盘点定谳（动手前核实）

- rtcl-vm 字节码：proc 体**定义点编译早已存在**（cmd_proc →
  compile_proc_body 无条件 + M3 skip_folds；apply lambda 调用点
  memo；OO 方法体首调 memo——编译输入含调用期 owner variable 快照，
  定义期不可得，维持现状记 NYI 保留项）。
- JIT wasm 发射：**首调用**才发生（exec_proc_body → fast_try →
  compile_cached），且拒绝不缓存负项——拒绝体每次调用重跑 emit 管
  线到拒绝点。M4 的实际缺口 = 定义点发射 + 负缓存 + 预算护栏。

### 逐件（b0044ba / 0bf0564）

- 件1 负缓存：PROC_MODULES 值 Option 化——emit 对固定输入决定性，
  拒绝落缓存后调用面零重试（问号传播语义不变=留解释器）；cap 64→
  1024（预填条目在 stdlib 级定义量下会被 64 cap 冲掉）。
- 件2 定义点预编译：rtcl-vm 新钩子（运行门与 Site A 同构：
  fallback/epoch/bytecode_applicable）+ cmd_proc 定义点调用；jitsh
  默认 on（RTCL_JIT_AUTOCOMPILE env 显式值优先=逃生口；rtcl-cli
  --features jit/库默认 off——灰度先行，bench 证伪风险后再翻默认）；
  同步编译 + PRECOMPILE_OPS_LIMIT=4096 op 护栏（PLT_CS_COMPILE_
  LIMIT 的 rtcl 版；超限体保持调用时编译，语义零变化）。统计面
  RTCL_JIT_AUTOCOMPILE_STATS=1（覆盖面/耗时分布/拒绝原因，exit+
  teardown 双挂点一次性 dump）。
- 件2 内的修正（启动税实测消减）：stdlib.tcl/ui.tcl 是每进程重放
  的 rumble 层对应物——in_rt_prelude 排除后无预发射。无排除时 29
  个 stdlib proc × emit = **+2-3ms/进程启动税**（最小脚本 10ms 级
  进程被拖到 0.80×）；排除后启动双侧中位 10ms 持平。教训：Racket
  CS 的「默认全编译」不含运行时层重编——rtcl 的内嵌预置层同口径。

### 验收与数字

- 门禁：jit_battery 52/52、judge 双引擎 88/88×2、cargo test
  --workspace 全绿、fuzz 400 seed7 0 分歧（预置层排除后复跑）。
- 覆盖面（定义即编译，jitsh + STATS）：judge corpus 用户 proc
  **396/427=92.7%**、battery 10/10、jit_proc_probes 18/19、bench
  cases 1/1；预算跳过 0（1500 语句探针正确落护栏）。拒绝 NYI 清单
  （31 例）：array-element 变量引用为主（15）、stack discipline 特
  形（2）、stack depth>128（1）——拒绝=留解释器，行为零差异（门禁
  逐字节复核）。
- 编译耗时分布（定义点 emit/proc）：p50 19-64μs、p90 31-120μs、
  max 464μs——**全在 <1ms/proc 预算内**（§7.2 核算成立）。
- 基准（jitsh 交替轮 best-of-5/best-of-10 vs 基线二进制交叉）：
  六基准 + oo2/oovars 噪声带内（list_ops 10/10、var_incr 13/12、
  arith_loop 12/14、oo2 99/99 ms）；负缓存收益探针（拒绝 proc
  200k 调用）单带内（基线每调用 emit 重试 ~50-250ns）。proc 密集
  负载受益为结构性（首调零发射延迟移到定义点），稳态 wall 无可见
  变化（编译成本摊薄，符合预期）。

### 余项/reopen

- 热点计数画像增强重编译（M4 后半，可选非默认）未做；OO 方法体定
  义点编译（编译输入调用期状态）；array-element 引用进 JIT 子集；
  RTCL_JIT_AUTOCOMPILE 在 rtcl-cli 面翻默认（bench 证伪后）。

## 10duodevicesimus. 2026-11-17 fast 调用协议专项——oo_bench2 3.40×→1.26× / oovars 4.11×→1.89× tclsh（perf/wave2）

M2 验收钉死的实质回归（「jitsh 下 oo_bench2 3.32× / oovars 3.70×
tclsh」）收口。画像推翻了「协议本身 1.4μs」的单因定谳——**大头在
宿主算术回退的文本重建/重解析**，协议只占 ~7%。

### 画像（CARGO_PROFILE_RELEASE_DEBUG=1 + perf，注意 strip=true 默认吞符号）

jitsh oo_bench2 全样分布（修前）：ExprParser 系（match_op/
parse_primary/parse_equality + Vec<char> 物化）~10.5%、fmt/format/
as_str/memmove ~8%、call_proc 5.1%、execute_fast 3.8%、getenv
2.3%（exec_proc_body 每 proc 调用 3 次 getenv：RTCL_JIT_POOL×2 +
RTCL_JIT_FAST）、get_var 3.5%（参数装箱逐名）、wasmi 执行全貌
~1.8%。根因链：oo 方法实参是**字符串形态**（"7"/"3"），Add 的
imm 守卫不命中落宿主；Mod/Div 在 emit_arith 属「§4 全走宿主」类
；宿主 binary_op 把操作数 expr_quote 进文本 `format!("{sa} + {sb}")
` 再 eval_expr 全量重解析——每次宿主算术一轮 ExprParser（语义
等价由「解析器逐层就是 expr_ops 的薄调用」构造保证）。

### 逐件消减（每件 battery + judge --engine jit 验证，交替轮 best-of-5）

- 件1 env 标志缓存：RTCL_JIT_FAST/RTCL_JIT_POOL/JIT_FT/JIT_VM_TRACE
  OnceLock 一次读（进程内环境不变语义不变）。−3.5%（噪声带内，
  getenv 2.3% cycles 摘除）。
- 件2 宿主算术 expr_ops 直呼：binary_op 慢臂不再拼文本——直接调
  rtcl_vm::types::expr_ops（numeric_binop/int_mod/int_shift/
  int_bitop/op_eq/op_rel/op_pow/op_neg/…，pub 面扩容——模块 doc
  本就预留「and later the JIT's runtime calls」）；And/Or 复刻
  解析器短路结构（两侧已求值字面量，可观察面只剩 strict_bool 错
  误）。**oo 1480→676ms（3.28→1.52×），oovars 1857→1317ms
  （→2.95×）**——单件 −54%。
- 件3 参数装箱帧槽直读：rtcl-vm 新公共 API `jit_frame_param_
  values(code, n)`（slot_table 与 code 同源 + 参数段无链接别名 →
  slots[..n] 直拷，params-first 绑定序 == locals 序）；回落逐名
  get_var（apply/顶层/别名帧）。省每参数 String clone + 全量名解
  析。wasmi/wt 双驱动同改。
- 件4 协议瘦身：fast_try 槽命中快臂（FastShared 1 项 last 缓存 +
  nparams_of 登记）——免 PROC_MODULES 哈希 + Rc<Module> 克隆与
  execute_fast 内二查；入口 caching=1 set 删除（fast Store 无任何
  clearing 写点，classic 的写点在宿主闭包/wasm run() 入口，均不
  触达 fast——初值恒 1，set 本为冗余 Global 写）。fuel/depth 重置
  保留（fuel 重置承载 max_steps 计账语义：无重置时 refuel 每
  ~chunk 调用反复充值 = 计账翻倍）；epoch 重置保留（trap 后
  fast_depth 残留防线同门）。**oo →528-580ms（→1.32×），oovars
  →888-1012ms（→2.03×）**。
- 件5 Div/Mod 双 imm 快臂：emit_arith 从「§4 全走宿主」摘出
  Div/Mod——floor 语义 wasm 直算（floor_div=q-1 / floor_mod=r+vb
  修正门，除零落宿主全语义）；imm ⇒ |v|<2^62 ⇒ INT_MIN/-1 陷阱
  不可达、修正后结果恒在 i63 域免范围守卫。**oo →482-528ms
  （→1.26×），oovars →826-864ms（→1.89×）**。
- 止损留档：oovars 出口 flush 的 set_var（object `variable acc`
  经 OO prelink 成链接槽——flush 是链接落盘的承重件）与参数数字
  串装箱期强转（改值形态=语义变化）均评估后不做；PENDING_RETIRE
  AtomicBool 门（~10ns）噪声级，记 reopen 线索。

### 验收与数字

- 门禁：jit_battery 52/52（b07 两轮证伪件5 首版：模块校验失败
  （Mod 门漏 I64Eqz）→ If 嵌套倒置（符号臂被嵌进 r==0 空臂=修正
  不可达）——差分电池双杀，机械复刻两处后全绿）、judge 双引擎
  88/88×2、workspace tests 全绿、fuzz 400 seed7 **0 分歧**、
  Div/Mod 符号矩阵 392 例（±7/±9/bignum 边界/除零 catch）三方逐
  字节。
- 数字（jitsh best-of-5 交替轮，ms）：oo_bench2 1459→**482**、
  oo_bench2_vars 1907→**826**；tclsh 同轮 442/447 基准 → 比值
  **3.40×→1.26× / 4.11×→1.89×**。jitsh vs rtcl 解释器（369-381ms）
  = 1.31×/1.8×。

### ≤1.0× 目标定调（结构性，数据支撑）

oo_bench2 的 jitsh 开销余量 = 482−356(interp) = ~126ns/调用，构
成：execute_fast 协议 ~60ns + wasmi 主机↔wasm 过桥 ~45ns
（init_wasm_func_call/execute_func/args 转换，引擎下限）+ 体宿主
Add×2（字符串参数 imm 守卫不可命中，expr_ops 直呼后的强制域下限
~30ns）。而 ≤1.0× 需要 jitsh ≤ tclsh(383ms)=头部空间仅 23ms
<协议+过桥单项。拓扑定调：callsite_in_loop 回落使每方法调用全
额重入解释器分派（oo 机器 + call_proc 帧机器都是共享成本），fast
协议只能在此之上**做加法**；把整循环留在 wasm 的形态已被 v5 gate
按实测拒绝（dict_ops 23×/list_ops 9×，M2 前既有数据）。≤1.0× 归
「oo 分派自身 wasm 化/M4 形态」 reopen 线索，非本协议面可达。

## 10septendecimus. 2026-11-17 树走查直呼单元 C1 误编译修复——eval_commands 深度占位（perf/wave2，他分支移交）

**移交来源**：Gen UI 调研线（rtcl-designer worktree，ui/designer @fd70528）
capture.rs 模块注 + 会话记忆移交的 parity bug（探针 S4 定谳："平凡 proc
pp {a b} + 多行 braced 实参，$a 绑错值；VM 路径正常；ui.tcl 在
eval_commands 下挂、VM 下过"）。其分支未修（设计加载走 eval() 规避），
本节在 perf/wave2 收口。

### 复现链（三层收窄）

- **宿主暴露面**：ui.tcl 经 `ScriptUnit::parse` + 裸调
  `Interp::eval_commands` 挂载（capture 流的加载形），随后词表命令
  树走查分派——`ui::surface .s -title Demo` 落
  `dict get $catalog $type`，键读成 `Surface {整个 catalog}`（实参
  值串位）；tree-mount 侧 `::ui::define Surface {多行 braced spec}`
  的 `type` 形参直接吃到 spec 文本。VM 挂载 + eval() 调用全程正常。
- **最小形**（`crates/rtcl-vm/tests/probe_eval_commands_units.rs`
  A1）：namespace eval 内定义 `proc define {type spec}`、体为
  `dict for {opt entry} $spec { if {...} {set bad "...$type..." } }`、
  顶层裸调面调用——`$type` 读到 spec、`$opt` 读到空、`$entry` 读到
  `Surface`（**邻槽值**，帧槽表 params-first 序 vs 误装表错位）。
- **平凡多行 braced 实参本身不错位**：`pp {\nfirst\n} {\nsecond\n}`
  20 形矩阵双路径全对（错位需 proc 体走 `call.rs` `_` 臂的
  `eval(def.body)` 旁落形——移交方"多行 braced"措辞来自 ui.tcl
  define 调用形的表象）。

### 根因（RTCL_NO_BYTECODE 双跑 + has_foreach 门对照钉死）

裸调 `eval_commands` 不占 `eval_depth`（保持 0）；proc 体经 call_proc
的 `_` 臂（体编译旁落/fallback）走 `eval(def.body)` 时深度恰为
**1**——踩中 eval_impl 的 C1 门 `eval_depth == 1 && !has_foreach`，
**体单元被误编译成顶层 slot 单元**（compile_unit_toplevel）：体内
变量读写按 top_slots（C2'，单元自身 locals 序）路由，与 call_proc
按 params-first 序播种的**帧槽表**错位 → 实参/局部读值串位。eval()
面外层已占深度 1、体落深度 2 → compile_unit，故 VM 路径始终正常。
旁证：`RTCL_NO_BYTECODE=1` 双路径全对（体走树走查词法路径，无
toplevel 编译）；体里塞无害 foreach（has_foreach 门关闭 toplevel
编译）与修复同效——机制钉形 H1 入库。

### 修复（双路径语义收敛，非树走查单侧补丁）

`eval_commands` / `eval_commands_from` 占住深度第 1 位（save/占用/
restore，深度非 0 不动）：嵌套 eval ≥2 → compile_unit，与
eval()/eval_incremental（本就自带占位）语义对齐。eval_commands_from
注释先读（083535a/083537a 教训：调用方 namespace/词法/line_offset
上下文一律未动——深度位与其正交，Tier1 重分派调用方语境深度已 ≥1
时数值不变，仅对 jit 重分派直链等无占位语境兜底）。

### 验收与护栏

- `cargo test --workspace` 全绿（0 failed）；judge(interp) 88/88；
  probes **412/36**（fail 集 var/trace 组基线既有，零新增）。
- 新护栏 `crates/rtcl-vm/tests/probe_eval_commands_units.rs`（5 测）：
  最小错位形 + H1 机制对照 + 变体（B/C/D/F1/A2）+ 多行 braced 实参
  20 形矩阵（tclsh 8.6.17 oracle 逐形钉值）+ 构体内联语境 9 形
  （proc 体/ns-eval/catch/while/foreach/if/switch/try/def-in-ns）+
  ui.tcl 双挂载形（vm-mount/tree-mount + 树分派调用 + op 日志断言）。
- **移交方受益**：rtcl-designer 分支 rebase 本修复后，capture 流的
  ui.tcl `eval_commands` 挂载形自动生效（其 capture.rs 模块注
  "已知分歧：树走查直呼路径……本分支不修"条目可除名，由该分支自行
  处理）；本 checkout DIVERGENCES.md 无对应记账条目，不涉及。

## 10sexdecimus. 2026-11-16..17 JIT 线续：M2/M3/行号专项（perf/wave2 @ adb25e2）

（继 M1' 验收后的第二轮 JIT 线队列；M2/M3 各自 HANDOFF 节见
10senioquincesimus/10undequincesimus，此处补行号专项 + 总账。）

### M2 slot 单态缓存（40bcae8/8b0005a）

- 盘点定谳：**fast tier 已有等价机制**（per-depth 内存槽 + 零检查守卫
  + lazy-flush）——真正缺口在 classic tier（顶层 slot 单元，
  IncrVar ~41 条）。落地 per-var icache（tagged i64 + ivalid 双门）
  + 8 宿主 import 后整表失效清单；b47 探针实证修复「缓存臂绕过
  interp 写致 trace 丢失」。
- var_incr −15%、arith_loop −20%，**JIT-vs-interp 差距闭合**
  （11/12ms vs interp 10ms 量级）；battery 43→47（+失效矩阵）。
- reopen：lane 类型化后端（边际 −15-25% 仅微形状，目标案例已
  0.22-0.35× tclsh）；**fast 调用协议回归**（jitsh oo2 3.32×——每
  proc 调用 ~1.4μs 协议开销 vs interp ~200ns，独立专项候选）。

### M3 epoch 接 Tier1 bump（6 commits，876c382..297b9b5）

- epoch 生命周期表全盘点（bump 12 存量 + 2 新增/读点 interp+JIT
  全录）——覆盖判定：突变经宿主分派必然被回边比对同点捕获。
- 修 3 正确性缺口：trace add execution 中途盲区、ensemble 遮蔽内建、
  **遮蔽感知折叠**（epoch 戳不防「编译前已遮蔽」——shadowed 扫描 +
  skip_folds，折叠名单单源化 TIER1_FOLD_LEAVES）。收窄评估不做
  （保守 bump = 安全默认）。
- battery 47→**52**（b48-b52：重定义/rename/trace/遮蔽/遮蔽后体）；
  judge 双引擎 88/88×2；workspace 1420/0。

### 行号 provenance 专项（8f46703/adb25e2，M1' 验收遗留 5 条收口）

- **fuzz 分歧 5→0**（seed7 400 例 / seed0 500 例均 0）：词括号失败
  的体帧行号 = defer 行为基还原**体相对行**；err_from_subst 跨错误
  泄漏（catch/try 终止点/seed_parse_error/顶层 D-11 四处并入一次性
  旗标清除）。
- 固化 loop_body_err_line 四件 + DIVERGENCES 2026-11-17 节；
  probes 380/35 → **412/36**（fail 集为 var/trace 组基线既有）。

### JIT 线余项

- M4（默认 proc 定义即编译——Chez/Racket CS 模式重定位）未启动；
  fast 调用协议专项（1.4μs/调用）候选；C-float/C-lazy 独立可选。

## 10quincesimus. 2026-11-16 M1' 验收 + 语句窗口 reopen 翻案（perf/wave2 @ c3d407a，master 线 cf5e3d8 →）

### JIT M1'（87c468e）

- **盘点定谳：M1' 四件套全部已落**（abi.rs fuel/epoch globals + §3
  import 面与哨兵 tagged word、assemble.rs §4 Int 快车道全表、
  nanopass 八 pass + check.rs 校验器）——里程碑表条目已可关闭。
- 真实缺口 = 解释器 errorInfo 大改（cf5e3d8）后的宿主/驱动面漂移：
  顶层 D-11 流解码 + while/for 体帧族两板点补齐，fuzz 41→5。
- **验收：arith_loop 19.2×**（jit 13ms vs ITS 基线 bd3e169 249ms，
  门 ≥3×）；ratio 0.37× tclsh。judge 88/88×2、battery 43/43、
  workspace 1420/0、六主基准 A/B 噪声级。
- 残差：5 fuzz 分歧全部 preexist 且 jit==tclsh（vm 端词括号失败体帧行
  号 provenance）——样本留 bench/scripts/jit_fuzz_findings/，归 var/
  行号语义专项（非 JIT 面）。

### 语句窗口 reopen（e462731 + c3d407a）——上轮 +107% 翻案

- **真因不是窗口运行时**：上轮实现漏把 4 新 op 加入 compile_unit 的
  fallback 白名单 → code.fallback=true → eval_impl 拒编译执行 →
  **整脚本树走查**（插桩实证：unit 900,010 vs 8）。上轮「窗口运行时
  从未执行过」，判别实验（关分支即回基线）与根因正交——误判。
- 白名单根修后窗口首跑：**dloop −10.9%**（4335.9M→3862.4M）、
  incr-bracket 微形状 **−34.5%**、dict 形状 −11.6%。
- 语义门：28 形状探针（judge/probes/incr_window_p01..28）+ judge
  88/88 + workspace 1420/0 + probes archive DIFF 集逐项一致。
- 残留（止损接受）：full-LTO 全局重内联漂移 +0.42-0.98%（fe/oo2/
  oovars/sum）——四重证据链证明非执行路径成本；最终落位=枚举尾段
  连续 tag + 主 match 前置范围预检。
- 新基建：feature `rtcl-dbg`（RTCL_DBG=seq@N 事件流/退出汇总，默认
  剔除）。
- 教训：**新增 op 有三处强制同步点**（fallback 白名单/top_op_class/
  unfused has_fused_ops）——白名单漏配的症状是「性能」问题，实际是
  「整个优化层被静默跳过」；判别实验设计与根因必须正交验证；诊断
  前先在干净基线二进制上复现。

## 10quaterdecimus. 2026-11-14..16 subagent 串行队列七阶段——语义域清账 + 性能 + Tk fork（master 线 99f30fc → 267d49a）

（并发=1 的 7+1 阶段 agent 队列；每阶段独立提交推送。队列细节见
specs/DIVERGENCES.md 各日期节与 BASELINE 深水区节，此处只记台账级。）

### 语义域（§10.1 从 33 例清到 ~5 条记账残差）

- **errorInfo 帧组 9/9 对齐**（d596b61，7 commits）：return 完成码
  解码（-level 0/循环体穿透）、catch options 键序（根因=returnOpts
  键位模型）、errorLine 归属、-f 文件语境帧（TclEvalEx 逐命令分野）、
  uplevel ns-eval 帧保活、procedure 帧名=调用词；5 探针固化。
- **IO/通道组收口**（329060b/45e1ad4/5c9c77b）：**socket -async 落地**
  （非阻塞 connect + 惰性补齐，哈希轮 OPEN 解除）、fconfigure socket
  全面、ECONNRESET→EOF、端口形矩阵、错误面统一；4 例设计差异注记。
- **OO 可见性 14 例**（bc3e91d..98656a5）：::oo:: 核心 ns/children
  尾名哈希双态求解/元类实例表/my 独露/nextto 新实现/history 门控
  （连带修 order 组 7 挂账）/Slot 链截断；D-43⑦ 旧账推翻（误测）。
- **dict+小尾巴 22 条**（d4903f2..7f2e0df）：**D-1 CLOSED**（唯一
  OPEN 语义分歧解除）、::tcl::prefix/pkgconfig/Bgerror/CopyDirectory
  落地、expr `<<<` 移除、rep 文案 16 字符、assemble 面、EI 相邻
  残差 4 对齐 1 缩窄、var-order 物化序对齐。
- 队列后总账：judge 88/88 不回退；judge/probes **340 → 380 pass**
  （fail 52→35，余为 var/trace 组基线既有 + ghost-insert 记账）。

### 性能（aadb1cf；BASELINE 深水区节）

- **六基准真实基线重锚定**（任务旧数字是 perf/interp-arch-opt-2
  快照）：fib 0.97x 起。
- **帧池去双清落地**：fib −5.66%（0.97x→0.91x）、oo2 −2.26%、
  oovars −1.87%；word_srcs 确认已在册；空循环画像后关闭；vars 缓议
  （目标已消失）。
- 语句窗口 `incr var [script]` 完整实现后 dloop +107% → 止损回退
  （负结果留档 + reopen 线索：新旧形状 EVAL_EXPR/ts flush 序纸面
  等价但运行时不等价，需逐写点插桩对比）。
- 顺带入账 2 预存分歧（incr errorCode/i64 溢出）→ **已修**（267d49a，
  incr_widen 探针固化）。

### Tk fork 渲染侧（rtcl d12f426..9624d8f + nsh fork 663210d/02a647b）

- `-alpha` 渲染通道落地（fork SetOpacity + rtcl 钩子 + headless
  alpha_nsh 测试）；**字体栈 15% 偏差定位修复**（canvas text 从不设
  字体→137.5px vs wish 139，-1.1%）；ttk 视觉小修（scale -value/
  progressbar 填充/按钮居中）+ 主题样式系统残差交 TODO 分析；
  treeview 表头拖宽落地（runtime 共享写核 + resize_column）；
  MSVC/macOS 不可做（接线点标注）；a11y/IME 评估成文（IME 建议
  收口不实现——commit 后事件面已对齐）。
- 前置修复：menu clone 被 info commands 隐藏（VM 宿主命令默认隐藏
  vs wish 可见）→ show_command API，八遍恢复全绿。
- 门禁：specs/tk/run.sh 八遍全绿、workspace 1420/0（+alpha_nsh）、
  judge 88/88；fork nsh-core 983/0。

## 10tredecimus. 2026-11-13 Spencer 全集 wasm 侧模块——regexp 外置收官（41/41 真机）

（485a7b2 + e06bf4c）

字符串 ABI v2 的首个真实施场景：wasm 构建的 `regexp`/`regsub` 从
regexpmod.js 语义子集升级为 **native 同源 Spencer 引擎**。

### 三刀抽取

1. `crates/rtcl-unicode`：tclUtf.c/tclUniData.c 分类/大小写表平移；
   `interp::unicode` re-export（路径不变）。双端共用同一份 Tcl 语义表。
2. `crates/rtcl-regexp-spencer`：引擎全集 5001 行平移（自带 Error，
   外部依赖仅 rtcl-unicode）；`commands::regexp_spencer` re-export，
   regexp_cmds 零改动。**两 crate 刻意不继承根 workspace**——跨
   workspace 路径依赖（侧模块独立构建）需要独立包元数据。
3. `crates/rtcl-regexp-spencer-side`：同一实现再出 cdylib 面
   （独立 workspace + `[workspace]` 表，dynmod-side 同款；std +
   panic=abort + opt z，产物 120 KB）。

### 侧模块 ABI（数值 C ABI + envelope）

- 导出：`spencer_abi()`(=1)、`exec(in_ptr, in_len)`、`alloc(len)`、
  `res_ptr()`/`res_len()`（静态 RESULT——单线程 JS 安全）。
- 输入 envelope：u32 记录表，args = [pattern, subject, flags, start,
  all]（flags/start/all 十进制文本——统一 UTF-8 记录便于调试）。
- flags 位（spencer_options 同映射）：1=nocase 2=linestop 4=lineanchor
  8=expanded 16=include_end。**-line 在命令层只设 nlanch+nlstop**
  （newline 不经命令面）；expanded 引擎原生支持——JS 子集缺失项
  一并补齐。
- 输出：tag 0 = ngroups + nmatches + **字符跨度**表（端点独占；
  引擎内部 span 即字符索引，native 的 byte span 只是命令层中间量，
  用户面 -indices/string range 皆字符单位）；tag 1 = REG_* code +
  native 同文 message（node 直验 REG_EBRACK/REG_BADRPT 一致）。
- -all 循环与 native collect_matches 精确同构：from = search_start =
  p（\A 绑定扫描起点）、空匹配进一字符、regexp 首位允许 p==len、
  include_end（regsub）允许尾空匹配。

### js_dispatch v2 修复（本节根因级发现）

`__dynmod:*` 分发路径（js_command.rs）是 v1 遗留形：**只取末词按
\u0001 拆分**——v2 逐词直传被截断成单参（探针实证 rest=["e"]）。
修复：新前缀 `__dynmodv2:` 逐词透传；`__dynmod:` 保持 v1 兼容；
register_module/unregister_module 同步换前缀。教训：**v2 落地时只改
了 rtcl-vm 侧 wrapper 模板，宿主 dispatch 面没跟**——ABI v2 真机用例
当时走的是 Rust 桥（native 单测），没打到 wasm dispatch 路径。

### worker 接线（rtcl-worker.js）

- setupSpencerMod：fetch/instantiate（URL 带页面 ?v= 穿透 HTTP/
  模块缓存）、v2 dynmod 注册、SPENCER_SETUP_TCL wrapper 安装；
  wrapper 安装失败自动回退 regexpmod.js 子集（exec 抛错 → return
  false）。
- wrapper 全选项面（纯 Tcl）：选项解析（含未知选项 native 同文
  错误）、变量写入（helper 需 **upvar 2**——upvar 1 落在 ::regexp
  局部帧）、-all 变量取**末匹配**（native set_match_var_groups 同）、
  -inline/-indices、regsub \0-\9/&/\\ 替换（反斜杠字面量用
  `format %c 92` 规避 expr 层歧义）、虚拟起点 index delta。
- 装载次序：spencermod 优先 → 不可用回退 JS 子集。

### 真机与教训

- **fs-opfs-test.html 41 pass / 0 fail**（O12 + R8 + S2 + E7 +
  SP12：回引/前瞻/expanded/非 BMP 字符索引/-line 行锚/-all 末匹配
  变量/regsub 捕获换位/编译错误同文/-about）。
- 页面调试坑（复用价值）：
  - **SAB 页必须 COOP/COEP 服务器**（/tmp/isolated_server.py，8933）；
    无隔离头 → `SharedArrayBuffer is not defined`，页面脚本静默死亡。
  - **python http.server 是单线程**（HTTPServer）：并发 wasm 传输时
    后续请求排队挂起 → 用 ThreadingHTTPServer。
  - **IAB worker 配额**：strict-durable 自动重载每轮泄漏 fs-worker
    （永不 terminate）→ 页面 `new Worker` 抛异常、脚本静默死亡；
    关陈旧标签释放。
  - **JS 字符串 `\U` 不是转义**（未知转义丢反斜杠）：页面用例里
    `\U0001F201` 变成 `U0001F201` 文本 → 不匹配 → 读到前一用例残留
    的同名变量（假绿/假红都出过）——测试用例的转义与变量名要独立。
- 回归：judge 88/88、rtcl-regexp-spencer 22/22、rtcl-vm 811/811+3、
  wrapper Tcl native 桩回归全绿（R3 形状与 native 逐字对齐——
  `-indices` 变量是二元列表，外层 list 渲染 `{1 4} {1 2} {3 4}`）。
- pkg 重构建（wasm-opt -Oz ≈2.0 MiB）；spencermod.wasm 120 KB 入库
  （www/，fetch 即用）。

## 10decimus. 2026-11-13 OPFS worker 形态——文件 io 真实持久化（12/12 真机）

- **www/rtcl-worker.js**：rtcl 的 **worker 部署形态**——解释器+JS 桥
  整体运行于 module worker，主线程经消息协议投递脚本
  （{id, script} → {id, ok, result/error, output}）。这是网页
  dispatcher 的真实运行结构（UI 主线程 / 调度 worker 分离）。
- **文件数据面 = OPFS 影子权威 + write-behind**：worker 启动装载
  OPFS 树进影子索引（内容/目录/元数据）；Tcl 同步操作即时改影子，
  入持久化队列，worker 让出事件循环时异步回写 OPFS
  （createWritable 逐操作重放）。**持久化时点 = exec 返回后的首个
  微任务**；严格 durable 需 SAB+Atomics.wait 桥（v3）。元数据
  （mtime/atime）由影子持有——settimes/stat 13 键全支持（OPFS 本身
  无 utimes API，影子是唯一权威）。OPFS 不可用时回落内存 fs。
- **www/fs-opfs-test.html**：O1-O12 真机（open w/r/gets/read/seek/
  tell/append/file 面/mkdir/readdir/glob/rename/copy/delete/source/
  settimes/stat 13 键）**12/12 PASS**；**跨刷新持久化实证**——
  /persist/marker.txt 每次运行写入时间戳、下次运行读回上次值
  （页内显示 🔁 标记）。清场步骤（file delete -force /demo）保证
  可重复运行。
- 回归：native 全绿、wasm check 过。

**SAB 严格 durable 桥已实现（同日第二轮，fs-opfs-test.html 12/12）**：
- www/fs-worker.js：OPFS 执行 worker（真实 async OPFS：sync access
  handle 读写/flush/close + readdir + rename[move 回退读写删] +
  meta sidecar /__rtcl_meta.json 持久化 settimes）。
- rtcl-worker bridge 模式：Atomics.wait 阻塞等待 fs-worker 回执
  （REQ_SEQ/PAYLOAD_LEN/REPLY_SEQ SAB 布局 + MessageChannel port 传
  seq）——**回执在 OPFS 写入完成后发出**（write 返回即落盘）。契约
  走 RtclHandle::set_fs（JsFs 包装，本 worker JS 对象为数据面）。
- **真机实证**：O1-O12 全过 + **terminate 后 strict 标记存活**（写入
  后立即 terminate worker，重载后标记仍在——write-behind 形态会丢）。
- 坑（真机逐一抓出）：SAB 视图不可直接喂 TextDecoder（拷贝副本）；
  JSON.stringify 拒 BigInt（Number 归一）；Worker 对象不可入
  postMessage payload（DataCloneError → MessageChannel + transfer
  port）；module worker 消息在 eval 完成后才派发（boot 队列 +
  booted 门，fs 分支判断前重放，否则 bridge 静默降级直连）；COOP/
  COEP 头必需（SharedArrayBuffer 需 crossOriginIsolated——本地服务器
  /tmp/isolated_server.py 8933 端口）；node worker_threads 链无 OPFS
  （仅作语法/装载护栏，真机以浏览器为准）。
- 页面接线：fs-opfs-test.html = SAB 形态（严格 durable 实证区段）；
  dynmod-test.html = write-behind 形态（原样保留，两形态并存）。

**expand OPEN 项收口（同日定调）**：`{*}` 展开语义**双路径探针全绿**
——specs/expand.probes 11/11（eval/树走）+ specs/scripts/expand.tcl
10 场景（-f/编译）——元素整词传递在两条路径均正确；**[list 版]
wrapper 的 native 行为亦正确**（RTCL_EXPAND_LIST=1 实测 bridge 收
3 词整词）。wasm 时代观测的 23/25 碎片限定于 **wasm dynmod [list]
链**（wasm 特有路径，未深挖）——**join 分隔符协议为根治方案**（不
依赖该路径），native 单测按 join 契约 5/5。探针入库作护栏；wasm
[list] 链复现时再深挖。json 外置面维持 join 协议。

**regexp 外置收口教训（7382ab4 后补）**：dynmod 外置带变量命令
（regexp/regsub）的 wrapper 定义有三个高频错位点，均真机抓出——
① 命名空间：JS 面 fn 装在 `::模块名::fn名`（::regsub::regsub 是错
名，正确=::regexp::regsub）；② __RS 类结构化编码的解包顺序（v=
result、返回=count 易反置）；③ POSIX 类翻译的前置检测
（[[:name:]] = 外层 [ 内嵌 [:）。wrapper 定义后必须逐条真机验证。

## 10quinter. 2026-11-13 第五轮：file/io 外置评估——测量 + feature 图解耦（定调）

**测量**（raw 口径 = cargo 产物，bindgen 再剥 ~0.5MB → pkg 2.18MiB → gzip 774KiB）：

| 变体 | raw | 增量 |
|---|---|---|
| core `(std,dynmod)` | 2,551,625 (2.43 MiB) | — |
| +file+io | 2,715,288 | **+164 KiB（file+io 净重）** |
| full +clock+package+regexp-lite | 2,800,963 | +84 KiB |

结论：file/io 净重仅 6%（160KiB raw ≈ 56KiB gzip），**尺寸不是外置动机**；
动机是功能诚实性——浏览器里 std::fs stub 必败，这批命令当前是
"编译进但运行时死"的状态。

**外置形态定调**（与 socket 的 dynmod 形态不同档）：
- **通道协议面留原生**（io feature）：chan/fconfigure/翻译/缓冲/反射
  通道机制不动——通道状态在解释器里，搬 JS 会让整个通道语义分叉。
- **数据面走宿主服务**：`HostFs` 服务 trait（HostConsole/HostClock 同
  款缝——`set_console`/`set_clock` 已有先例）：open/file-exists/cd/pwd
  等 std::fs 直连点（io.rs 48 处/channel.rs 14 处/os.rs 1 处）路由到
  服务实现。native=std::fs（现状不变）；wasm=JsFs 桥（OPFS）→ 浏览器
  里 open/gets/puts/file 真正可用。安装：`RtclHandle::set_fs(js_obj)`。
- 尺寸收益放第二位：服务化后如果嵌入方完全不想要 file/io，feature
  已可独立关闭（本轮 event→io 耦合修复后 `(std,dynmod)` 可编译）。

**本轮已落**：feature 图解耦——`chan_not_found` 移到 feature-free
（commands/mod.rs），event.rs 不再引用 chan_io；fcopy 条目按 io 门控；
无 regexp 构建的 `parse_index_int` cfg bug 修复。`(std,dynmod)` 与
`(std,file,dynmod)` 均可编译（此前后者必败）。

**本轮已实现（HostFs 落地）**：
- `host.rs`：`HostFs` trait（数据面 18 方法：open 通道工厂/stat 简面/
  mkdir/delete/rename/copy/list_dir/cd/pwd/read_file/tempfile）+
  `NativeFs`（std::fs 逐字行为）。stat 的 unix 深水键（dev/ino/uid/…）、
  readlink、settimes 留原生直连。
- `channel.rs`：open_file 拆为 `open_file_channel`（构造，NativeFs 调
  用）+ `install_opened`（命名/config，cmd_open 与宿主通道共用）。
- 路由：cmd_file（exists/type/size/atime/mtime/delete/mkdir/rename/
  copy/owned）、cmd_open、source、glob 的 walk_dir/collect_entry、
  os.rs cd/pwd/readdir——48 处 std::fs 直连点收敛到 host_fs。
- rtcl-wasm：`fs_bridge.rs`（JsFs/JsChannel——thread_local 注册表承载
  JS 对象，Rust 侧只持 u64 id 保 Send；字节走 latin-1 字符串，二进制
  载荷 OPFS-worker 形态 v2）+ `RtclHandle::set_fs(jsObj)`。JS 方法契
  约见 fs_bridge.rs 顶部注释。坑：apply(thisArg, args) 的 this 不进
  参数数组（首版把 NULL 塞进 argv 导致参数后移一位——真机探针抓出）。
- dynmod-test.html 增设 F1-F8 区段（内存 fs 桥 makeMemoryFs）：
  open w/r/gets/read/file exists/size/type/mkdir/readdir/glob/rename/
  copy/delete/source 全链路真机 **22/22 PASS**。
- 回归：native 全绿（test/judge/probes/scripts——行为逐字保留）、
  wasm check 过、(std,dynmod)/(std,file,dynmod) 可编译。

**完整支持面补全（同日第二轮）**：
- HostFs 加宽：`FsStat`（13 键全形 struct）+ `stat_full(path, follow)`
  + `set_times(atime, mtime)` + `read_link`；NativeFs 填 std::fs 真值
  （unix 扩展键 MetadataExt 逐字；`owned` 走 getuid）。
- 路由：file stat/lstat（13 键从 FsStat 取，非 unix 无 cfg 差异）、
  file mtime name time（set_times）、readlink（错误文案对齐 tclsh 原文
  `could not read link "x": invalid argument`，io_error_text 小写）。
  原生 hfs2 差分（mtime set→读回/stat 13 键/readlink）与 tclsh 逐字同。
- JsFs：stat_full（宿主 statfull 契约；缺省 fallback 由 stat 合成）+
  set_times（JS settimes(path, atime|null, mtime|null)）+ read_link。
  内存 fs：settimes 存 atime/mtime、statfull 合成 POSIX 形值（ino 路径
  哈希/mode 按类型/blksize 4096/nlink 1/uid gid 1000）、readlink 抛
  invalid argument。真机 F9-F12 过，合计 **26/26**。
- 真机抓的坑：**JsValue::from(i64) 到 JS 是 BigInt**（settimes 数字需
  Number() 归一，as_f64 对 BigInt 返 None）；对象字面量方法内 `this.X`
  在 apply(null) 下失效（statFn 提为闭包变量）。OPFS 边界如实保留：
  同步句柄无 utimes API——set_times 在 OPFS 形态由宿主报错（内存 fs
  全支持）。

## 10quater. 2026-11-12 第四轮：dynmod——wasm `load`/`unload` 扩展模块注册表（已完成）

rtcl=网页 command dispatcher 的扩展面：wasm 里 dlopen 不存在，改为
**宿主预实例化 + 注册表挂线**（`load` 是同步命令而 instantiate 异步）。

- rtcl-vm（feature `dynmod`，入 rtcl_std 与 wasm 子集）：
  `Interp::register_dyn_module(name, DynModuleDef{commands: Vec<(tail,
  dispatch_words)>})` 声明；`load name|name.wasm|/path/to/name.so` 安装
  `::<name>::<cmd>` 包装 proc（体 = dispatch_words + `{*}$args`，与
  js_command 的 proc 包同形）；`unload name` 解包（未 load 过 → tclsh
  原文 `file "x" has never been loaded`）；路径形未登记 → 回落
  load-cmd dlopen 语义（缺文件文案逐字）。计数：原生 101、wasm 100
  （socket 仍 net 门控；load/unload 回到全平台）。
- rtcl-wasm：`register_module(name, exportsObj)` / `unregister_module`
  ——exports 的函数值属性经 `_js_dispatch` 表接为回调；示例胶水
  crates/rtcl-wasm/www/dynmod-bridge.js（数值 ABI → 字符串包装；
  字符串/字节载荷走侧模块 rtcl_alloc/rtcl_free 交接，v2）。
- 本体拆分路径：可拆=纯函数子系统（regexp-spencer/encoding 表/zlib/
  json/chart——字符串进出）；拆不动=分派/变量表/帧模型（即本体）。
- 测试：rtcl-vm dynmod 单测 5/5（host 假桥）+ **真机 smoke 全绿**
  （www/dynmod-test.html，14 用例：dynmod 全链路 8 + 平台抽查 6；
  侧模块 crates/rtcl-wasm/examples/dynmod-side 为 no_std 纯 wasm32
  crate——裸 wasm i64 参数是 BigInt，JS 胶水 BigInt/Math.trunc 交接）。
  真机抓出并修复：__dynmod:* 内部包装 proc 走 cmd_proc 路径绕过了
  register_command 的 hide 逻辑 → Interp::hide_command（trim ::）+
  register_module 注册后隐藏；计数面结论：wasm 初始 = 99（无 socket
  [net 门控] 无 exec [wasm 子集本就不含]），load 后 101，unload 回 99。
  JS 侧需本地 HTTP（ES module + fetch 不能 file://）：
  `cd crates/rtcl-wasm && python3 -m http.server 8932` →
  /www/dynmod-test.html。

## 10ter. 2026-11-12 第三轮：协程引擎 + wasm 可选门控（已完成）

- coroutine/yield/yieldto 落地（挂起链重放；specs/coroutine.probes
  23/23；AST-eval 边界内 yield 报错=唯一分歧）。rtcl 定调=网页
  command dispatcher：wasm 无法支持的特性改为可选——`net`（socket）、
  `load-cmd`（load/unload）默认仅原生；wasm info commands=98、
  原生=101（可选差异）。wasm32-unknown-unknown check 过。

## 10bis. 2026-11-12 第二轮收口：哈希/迭代序 + 命令面（已完成）

§10.1 第 3/5/6 类 + 第 7 类部分已收口（见 DIVERGENCES.md 2026-11-12 节）：
Tcl 哈希表序重放落地（`info commands` 101/101 全序一致、array names/
vars/globals/locals、mathop 23 算子、dict info、-unordered/getdef 移除、
nan/inf 词法、socket/fcopy/load/unload/zlib(crc32/adler32)/case/history
等命令面补齐）。judge 88/88、probes 全 0 DIFF（新增 specs/order.probes
48/48 + specs/expr-lex.probes 11/11）、cargo test 全绿。剩余 OPEN：
coroutine/yield/yieldto 引擎、socket -async、zlib 压缩、chan names 的
通道注册历史序、命令集再对齐时 hidden 表维护（host 桥用 namespace
which 探测，勿用 info commands）。

## 10. 原生 Tk 与性能：下一 Session 待办（2026-11-12 交接）

> perf/wave2 分支已合并回 master（`1017525`，283 提交）。原生 Tk 的
> 兼容性工作经**八遍 wish 对照**（`bash specs/tk/run.sh` 一键回归：
> 字体度量/几何/事件 % 替换/selection-虚拟/winfo/全控件子命令面/
> wm-tkwait/treeview-drag）已把 rtcl 侧纯语义差距基本清零。对照在
> 这轮累计修掉 15+ 组真 bug（字体族名解析、负 -size、-fixed 判据、
> tk scaling 默认值、%s 十进制、%k keycode 表、%A Ctrl 控制字符折叠、
> selection clear/own、wm protocol 查询、menu clone/postcascade、
> treeview 15 子命令、text wrap 显示行模型、photo 像素引擎等）。

### 10.1 语义域剩余（specs/DIVERGENCES.md，33 例需对齐 + 15 例 -f 设计差异）

> **2026-11-16b**（perf/wave2）：组 4/5 + 组 1 移交 5 残差 + var-order
> 移交项全数定调收口（judge 88/88；probes 373/41 → 378/36：翻绿
> bn13/pbr3/probe1/ivars2/tclns，固化 smallface_p24；workspace tests
> 全绿）。逐例定调见 DIVERGENCES.md 2026-11-16b 节；剩余 OPEN 见该节
> 10/11/15/22 条。

按根因分组（详细探针名见 DIVERGENCES.md 2026-11-03/11-08 节）：
1. **errorInfo/errorstack/return 帧**（~9）——**2026-11-14 收口**
   （perf/wave2 90f4ad6..3ab5a1f，6 commit；DIVERGENCES.md 2026-11-14
   节）：return -code/-level 解码（TclMergeReturnOptions 换形 +
   -level 0 物化原码）、catch options 键序（returnOpts 键位模型）、
   errorLine 归属（defer 帧带行号 + 括号行号单元化）、-f 文件执行
   语境 if/while/for 帧（in_file_script + VM unwind 最外层体限定）、
   uplevel ns-eval 帧保活（info level 计层）、procedure 帧名 = 调用词。
   9 例全绿 + 5 个新固化探针；新发现残差 5 条记 DIVERGENCES.md
   2026-11-14 节（uplevel -errorline、ARITH 关键词、expr 解析错误
   INNER/errorCode、dictGet INNER、error 三参形）。
2. **IO/通道**（~13）——**2026-11-15 收口**（perf/wave2，DIVERGENCES.md
   2026-11-15 节）：通道编号/复用、read 计数、fblocked 元数、chan copy
   方向、eofchar 均差分实证全 SAME；socket -async 惰性连接落地
   （socket 即建、I/O 补齐、失败终态 ENOTCONN）+ fconfigure
   -peername/-sockname/-connecting 选项面 + 数据面 crlf 默认 +
   ECONNRESET 归 EOF + 端口形（TclSockGetPort）+ 通道 IO 错误
   POSIX 面。新固化 judge/probes/chan_io_p23.tcl + specs/scripts
   四个回环/残余差分 harness。残余 OPEN：sockN 命名（不可复现）、
   socket -server（伺服面架构边界）。
3. **OO 可见性面**（~10）——**2026-11-16 收口**（perf/wave2，
   DIVERGENCES.md 2026-11-16 节）：::oo bootstrap 形状（9 命令 + 5 子 ns
   + Obj1..Obj10 核心 ns，children 插入序双态求解）、元类实例表、对象 ns
   只露 my + ::oo::Helpers（nextto 新实现）、methodtype/forward 自有
   作用域 + core method 形、ctor/dtor 实体（推翻 D-43⑦"恒空"）、
   deletemethod/renamemethod 提及名单、filter 拦截未导出调用、
   history 仅 stdin 注册（连带修复 order 组 7 挂账）、内部命令直调面、
   Slot listing 截断点。oo2 r4（catch var/errorCode 全局物化序）定调
   移交 var-order 组。新固化探针 judge/probes/oo21/oo22。
4. **dict 残差**（~6）——**2026-11-16b 复核定调**（perf/wave2，
   DIVERGENCES.md 2026-11-16b 节）：`dict info` 已随 tclhash 重放对齐
   （12/38 项含桶重建逐字节同）；`dict getdef` 已删（=8.6 无此命令）；
   D-12/D-13 复核为已收口；D-14 incr/lappend 坏值 EC+错误落地；
   d17 unknown 子命令 EC 补齐；`dict get` 缺键 INNER=dictGet。
5. 其余小尾巴——**2026-11-16b 收口**（perf/wave2）：expr rotate 词法
   （`<<<`/`>>>`）移除对齐（bn13）；representation 16 字符截断 +
   bignum 结果规范化（pbr3/bnprobe2）；assemble over/undefined/
   duplicate/异常语境检查 + getbytecode 装配子集 dict（cmp_assemble
   全绿）；`::tcl::prefix`/`::tcl::Bgerror`/`::tcl::CopyDirectory`
   落地 + -f 模式 ::tcl::history 移除（probe1）；`interp alias {}`
   （D-1）收口。残差 OPEN：disassemble/getbytecode 成功清单（设计
   差异）、dbg_basic（oracle SEGV 非分歧）、dbg_trace（提取残缺 +
   trace 回调链帧）、uplevel -errorline 多行 harness 差 1（基建已落，
   动 line_ctx 基线风险大）、oo2 r4 var-order 2 位置（ghost-insert
   级不可精确重放）。

已收口/移出（原七组清单）：
- **哈希/迭代序 + 命令面**（原组 3，~11）——**2026-11-12 第二轮收口**
  （§10bis / d7cc38e：Tcl 自研哈希表插入序重放落地、info commands/
  array names 全序一致、mathop 23 算子、命令面补齐；order.probes
  48/48 护栏）。
- **Unicode UCS-2**（原组 5，~5）——经定调归**设计差异**：`string
  length/index` 对增补平面按码元计 2 等走 feature/选项开关（rtcl 的
  UTF-8 内部形态是刻意选择），移入设计差异注记，不再列入对齐清单。

### 10.2 性能需提升的（bench/BASELINE.md）

六基准已全部 ≤1.0x tclsh（起点 fib 1.32 / fe 1.82 / dloop 1.01 /
oo2 1.07 / oovars 1.59 / build 1.29）；当前：fib **0.83x**、oo2
**0.42x**、oovars **0.52x**、fe/dloop/build ~持平。微套件 arith/
var_incr/proc_fib 0.6-1.0x。剩余开销点（深水区，逐项 <2% 需组合）：
1. **call_proc 帧准备四板点**：level0 每参 clone+push（~20 instr/调
   用）、slot_mode 门禁的 `Rc::clone(code)`+`slots.resize`、
   `ns_of_qualified`+两次 ns Rc clone、帧池收发与 11 项字段 clear。
   画像方法：`CARGO_PROFILE_RELEASE_DEBUG=1` + perf record → python
   自聚合（perf report 会超时，见 BASELINE Round AF）。
2. **语句窗口扩展**：`incr x [expr {...}]`、`set x [cmd ...]`（含
   bracket 结果窗口）、`[SubEnd, DynCall/Call]`——每迭代 op 数是真正
   差距（per-op 语义体已是 tclsh 的 1.1x）。
3. **`BEGIN_CMD` word_srcs 借用化**：`(*const SrcSpan, len)` 新型——
   去 p_only 画像 8.7%（op_begin_cmd 5% + drop<Vec<SrcSpan>> 3.7%）。
   unsafe，需严肃审跨嵌套 eval 的 saved 引用链。
4. **空循环基线**：dispatch + region 簿记（Vec push/pop）——候选 =
   bodies 预留 + unchecked push、`frames.last()` 缓存（~5-8%）。
5. **vars 指针稳定存储**（残差 ~1.2× 目标）：VarMap newtype +
   链接变量 `*const Value` 缓存（traced 走原路径）——方案存档
   BASELINE G9f 节。
6. JIT 已转主线；**M1' 已验收关闭**（arith_loop 19.2×，见
   10quincesimus 节）：剩 M3（epoch 接 Tier1 bump）与 M2 slot 单态
   缓存（`docs/JIT-WASM-GC.md` §4）。

### 10.3 原生 Tk 需 nsh/fork 渲染侧的九项（详见 TODO.md 新节）

`-alpha` 渲染通道、字体栈统一（渲染行宽 ~15% 偏差）、ttk 主题视觉
化（命令面已对齐）、treeview 表头拖拽视觉、a11y 深度、IME 深度、
MSVC 矩阵、macOS 平台——每项均标注 rtcl 侧已就绪的接线点。

## 8. 提交规约

- 每轮修正一个 commit，message 格式：`fix: Round N divergence burndown — <主题>`，正文列条目，尾部署名 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- 提交前必跑：`cargo test --workspace`（全绿）+ `./judge/run.sh`（无回退）。
- `git push origin HEAD`（master，已配置 ssh）。
- `.refer/`、`judge/BURNDOWN.md` 以外的新资产都可入库。

## 9. 本文件维护

每完成一轮修正或一个 JIT 里程碑：更新 §3 的数字、§4/§5 的完成状态、补充新裁决。handoff 文件自身入库（`HANDOFF.md` 在仓库根）。
