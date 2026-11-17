# rtcl 工程 Handoff — 多轮修正 → JIT 全任务交接

> **状态横幅（2026-10-07）**：语义修正与优化阶段已收敛——judge 87/87
> 文件（case 级 4072/4072，原 5 例分歧已修复）、双引擎差分 0 分歧、
> 性能微套件反超/持平 tclsh、生产就绪标记（见 `docs/PRODUCTION.md`）。
> 当前待办见 `TODO.md`；性能台账见 `bench/BASELINE.md`；分歧台账见
> `specs/DIVERGENCES.md`（已重建）。本文件 §1 基础设施/§2 方法论仍然
> 有效；§3 以后的数字是 2026-10-03 的历史快照（当前数字以 PRODUCTION
> 与 BASELINE 为准）；JIT 保持暂停（2026-10-17 内存管理/编译管线设计
> 定稿 = 路线 C，见 `docs/JIT-WASM-GC.md`，实现仍暂停）。
>
> 最后更新：2026-10-17（横幅）；正文完成于 2026-10-03
> 用途：任一 agent 读本文件即可接管全部剩余工作，无需对话历史。

## 0. 项目与仓库

- 仓库：`git@github.com:coreseekdev/rtcl.git`，master 工作树 = `/home/nzinfo/src.note/rtcl-master`（持久盘；2026-10-03 从 /tmp tmpfs 迁出——/tmp 重启即失，工作成果必须及时 commit+push），分支 `master`。
- 定位：Rust 实现的 Tcl 解释器（jimtcl 血统的轻量定位），target = native + **wasm32-unknown-unknown（预期主路径）** + wasm32-wasip1 + embedded no_std。
- crate 结构：`rtcl-parser`（递归下降解析 + ByteCode 编译）、`rtcl-ir`（OpCode 三层：primitive / `Call(CmdId)` 0..127 stdlib 128+ 扩展 / `DynCall`）、`rtcl-vm`（execute.rs dispatch 循环 + `VmContext` trait + Value/Error 唯一实现）、`rtcl-core`（Interp + 内建命令，`value.rs`/`error.rs` 只是 re-export）、`rtcl-cli`、`rtcl-wasm`、`rtcl-expect`、`rtcl-jit`（ByteCode → wasm 发射器，JIT M0 起）。
- Tcl 官方源码（行为 oracle 的测试语料）：`/home/nzinfo/src.note/rtcl/.refer/tcl`（已 gitignore；如被删重新 `git clone --depth 1 https://github.com/tcltk/tcl.git .refer/tcl`）。
- 系统 oracle：`/usr/bin/tclsh` = Tcl 8.6.17。**一切语义以 tclsh 实测为准，不凭记忆。**

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
  oracle：echo '<script>' | tclsh（Tcl 8.6.17），一切语义以 tclsh 实测为准。
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
