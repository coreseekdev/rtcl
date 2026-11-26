# rtcl 架构设计——变量层/指令集/单引擎/值模型（2026-11-26，架构师定稿）

> 产品目标（用户 2026-11-26 拍板）：**替代标准 Tcl**（兼容性红线 =
> judge 对 tclsh 9.1.0 oracle 逐字节），同时为 **wasm 网页应用提供最小
> 化的 command script 核心**（体积、零外部 C、防 OOM/栈、宿主挂载）。
> 本文档是 D1–D6 六项结构工作的设计与派工依据；实现按文末波次推进，
> 每项独立批次走 §0 门禁纪律。

## 0. 共同前提

- 门禁：judge 88/88（9.1 oracle）+ crash-gate 全过 + sweep 12 既有名
  单不新增 + workspace exit 0 + wasm32 构建；语料 9.1 重提取（E 项）
  完成后 judge 的语料换成 9.1 口径，其余门禁不变。
- 双引擎逐字节一致的红线在 D3 完成前仍有效。
- 每批只 `git add` 自己的路径；基准进 `bench/scripts/`，探针先钉后改。

## D1 变量表重写——Var 模型（最后实施）

**问题**（§3.1，两天 C40–C54 的共同根因）：一个名字在哪里，要靠近十
套并行结构回答（帧槽、帧名表+插入日志、全局平表+日志、两个数组名集、
四种链接表、无值条目表、顶层槽缓存、variable 声明表）；读/写/存在/
删除/数组命令/trace 各自重走解析，修一处漏一处。列表顺序靠日志重放
仿真。

**目标形状**：

- 三类表——全局表（每命名空间一张）、帧变量表（每 proc/体帧一张）、
  数组元素表（每数组一张）——都是真实哈希容器（桶链 + 插入序向量，
  顺序天然，删除留洞标记）。
- 表项 = `Var`：`value: Value`、`flags: {ARRAY, UNDEFINED, LINKED}`、
  `traces: Option<TraceSet>`、`link: Option<LinkTarget>`。
- **解析一次返回句柄**：`ResolveOutcome = Handle(VarRef) | NotFound { parent }`。
  `VarRef` = `(table_id, slot)`，slot 进 generation-guarded arena（指针
  稳定：删除只置洞+代数，不搬移）。之后的读/写/存在/删除/数组/trace
  全部对句柄操作。
- 顶层 slot 缓存（C2' 的 ts 单元）保留为句柄缓存的一种（帧局部优化），
  不再是平行存储。
- T3 的直列（槽表序+日志重放）被容器序天然替代；T7 的 `elem_counts`
  变成数组 Var 的字段；T7 平键快路径由「解析一次」替代。

**迁移路径**（每步全门禁绿后进下一步）：

1. 引入 `VarArena` + 新表结构，全局表先迁（`globals`/`phantom_globals`
   /`array_globals` 三合一），读写路径双读对拍（debug 断言旧新一致）。
2. 帧变量表迁移（槽模式保留：编译局部仍走槽，`Var` 承载槽背衬）。
3. 数组元素表迁移（`elem_counts`/搜索戳并入）。
4. trace 表挂 `Var`（`var_traces`/`elem_traces`/`trace_phantoms` 三表
   收进 `Var.traces`），trace 键字符串全部退役。
5. 删旧结构（预计净删 ~1500 行）。

**风险与红线**：errorInfo/errorCode/trace 序是字节级语义——每步用
现有 judge+probes+d28/t7_arr_elem 探针兜底；wasm 的分配记账（无栈保
护）沿用既有嵌套预算，arena 分配计数接入。

## D2 码表——OPS 表 + 宽跳转（第一、二步；第三步不做）

**第一步（唯一真源）**：`OpSpec` 静态表——每 op 的名字、操作数种类与
宽度、行号站点、纯 op、融合展开形。`exec_inner` 的两次分类 match 变查
表；反汇编/JIT lower/`unfused()`/校验读同一张表；加超指令从七处变两处。
编码不动。

**第二步（宽操作数，消 C55）**：

- 跳转改**相对 i32**（`Jump/JumpTrue/JumpFalse/JumpTable` 及融合跳转
  臂），消绝对 u16 上限；`Call/DynCall` 的 argc 宽形或拆分发射；
  `mark_index_overflow` 的三个来源（常量/局部/argc）逐一关掉。
- `PARSE_CACHE_MAX_SCRIPT`（256KB）调高或改为可配——它现在是唯一
  的大单元回落门（TREE-FALLBACK F5）。
- 验收：`judge/audit-cases/big-unit.sh` 两种大小 errorInfo 逐字节同；
  F1 从 TREE-FALLBACK 清单退役。

**第三步（线性整数码 + 变宽操作数）不做**——派发税收益未证实且集中
在纯算术循环；维护成本与 wasm 端载荷提取的开销先 perf 说清再议。

## D3 树引擎收敛——单执行器

按 TREE-FALLBACK-TRIGGERS 的 F 系列顺序补齐，然后删树遍历执行器：

1. **F3**：epoch 失配重编译（失效缓存 + 再编译），不再回落。
2. **F5**：大脚本直接编译入口（绕过 parse-cache 键）。
3. **F6**：body_memo 容量策略（或并入 F5 的按请求编译）。
4. **F2**：exec trace 事件进 VM——`BeginCmd`/`Call` 站点发 before/
   after 事件，支持 result/命令词改写与删除；trace 挂着时不再钉回树。
   （最大头，单独批次；事件协议对齐 tclsh 9.1 的 exec trace 语义。）
5. **F4**：`--no-bytecode` = VM 的 no-cache/no-fusion 模式位（诊断用）。
6. 删 `interp/eval.rs` 的树遍历执行器与 `loops.rs` 回退体；sweep 的
   比对对象改为「VM vs VM+trace 模式」或退役（12 处既有 DIFF 名单
   随之消案，逐条在 specs/DIVERGENCES.md 记账）。

C39（递归深度口径）在树引擎删除后变成单纯的调用深度问题，按 §3.5
把调用深度与求值嵌套分开记账解决。

## D4 Value 字符长度缓存

- 字符串 rep 加惰性 `char_len`（`Cell<Option<usize>>` 或等价位；8 字
  节标记字的余量先行审计，放不下就进堆侧 rep）。
- **第一步是清点全部字符串改写入口**（append/str_replace/str_map/
  lower/upper/trim 族/insert/replace/bin translate…），每个入口置无效。
  清单不全就是错字形——用「改写后 length 变化」的 fuzz 探针兜底。
- 计数函数复用 T7 的 SWAR 实现（3.1µs/24KB 是无缓存地板，缓存命中后
  O(1)）。
- 验收：非 ASCII `string index/range/first/last` 基准到 tclsh9.1 的
  3 倍以内；P15 hit/hit-uni 两项退役或达标。

## D5 num-bigint 集成（已授权）

- 依赖：`num-bigint = { version = "0.4", default-features = false }`
  （alloc 特性，no_std/wasm 兼容；不引入 rand）。
- Value 整数状态：i63 立即数仍是快路径；溢出/超界落 `Rc<BigInt>` 堆
  箱（flags 加 BIG 分支）。算术全族（`+ - * / % ** << >> & | ^ ~`、
  `incr`、比较族、`int()/wide()/entier()`）接 BigInt；除法/取模按
  tclsh 9.1 的整除语义。
- **9.1 口径**：`format`/`scan` 无 size 修饰符的 32 位截断规则照
  TODO 对标迁移 T1 执行（BigInt 先折算再截断）；`int()` 不再截 word。
- wasm 体积实测入账（预计 +200–400KB；`opt-s` 轨另测）；判官风险：
  溢出错形（ARITH IOVERFLOW）语料在 9.1 下变 bignum 结果——语料重
  提取（E）统一重钉。
- 语义先钉：溢出族探针（incr_widen/bnprobe2 已有，补 `int()`/`**`/
  位运算族）重钉 9.1。

## D6 wasm 最小核心（产品方向，不在本轮实施）

- **Core Profile** 概念：wasm 构建的 feature 分层——`core`（最小命
  令集：核心语言 + expr + list/dict/string + channel 内存面 + OO）与
  现行 `std`（全量）。36 项扩展已是独立 feature；`encoding`/`clock`/
  `file`/`exec` 已可选——Core Profile 把它们整体关掉并固定体积上限
  （目标 ≤1.5MB，现 2.1MB）。
- D5 的 num-bigint 在 core 里保留（语言核心）；D4 的缓存保留。
- 每批 wasm 构建体积入账 BASELINE；体积回退是门禁。

## 实施波次

| 波 | 内容 | 依赖 |
|---|---|---|
| W-A | D5 num-bigint；E 语料 9.1 重提取；T9/T8 预存分歧按 9.1 对齐批 | 无（文件不相交） |
| W-B | D4 字符长度缓存 | W-A（Value 内部不并行） |
| W-C | D2 码表一二步 | 无强依赖（可并行 W-A 后） |
| W-D | D3 树引擎收敛（F3/F5/F6 → F2 → 删树） | W-C（宽跳转先行） |
| W-E | D1 变量表重写（五步） | 所有语义批次稳定后（最后） |
| W-F | D6 Core Profile | 其余落定后 |

T10 安全项处置：S38（HTTP 超时/响应上限）、S21（lrepeat/repeat 大小
守卫）、S39（密钥哈希）随 W-A 一批做（小而明确）；S23（wasm trap 恢
复）挂 D3/D6 设计注记；S30（panic=abort 防线）保持 abort（wasm 体积
与不可恢复语义），宿主边界加 `catch_unwind` 另议。
