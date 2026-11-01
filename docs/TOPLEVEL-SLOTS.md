# top-level slot 化设计（Lane A/解释器——2026-11-01 定稿）

> 动机（实测）：顶层 name-keyed 循环 80-94ns/iter vs proc slot 形状
> 22-25ns——差额 = 变量哈希探测（FxHash ~8-10ns/次 × 5-6 次/迭代）+
> 守卫链。标量快路径（c8dbe4d）已省扫描部分（-14%）；哈希探测本体
> 只能靠槽位语义消除。tclsh 同构建议即「把循环放进 proc」——本设计
> 把 proc 的槽位待遇带给顶层。

## 语义核心：顶层变量 = 全局变量（别名，非隔离）

顶层 `set x 5` 写的是**全局**（proc 可经 `global x` 读、upvar 可链、
`namespace eval` 内的解析规则不同）。槽位化不能改变别名事实——
设计 = **合成帧 + flush/reload 纪律**（JIT lazy-flush 的解释器版）：

- 单元执行开始：压**合成帧**（slots = locals.len() 全 unset，
  slot_table = 本单元 code，无 upvars/locals map）。
- LoadLocal：slot 有效 → 直读；unset → 读全局（名字 = locals[slot]）
  并填槽（cache-clean）。
- StoreLocal/IncrLocal：写槽 + 标脏。
- **分派边界**（任何可执行 Tcl 代码或按名 touch 变量的 op 之前）：
  flush 脏槽 → 全局；**reload 全部槽 ← 全局**（分派内的写入——proc
  `global x; set x 7`、`set a(k) v`——落全局，reload 拉回）。
- 单元退出：flush 脏槽（REPL/下一条 eval 必须看见）。

### 纯 op 白名单（无边界动作）

LoadLocal/StoreLocal/IncrLocal(+Pop)/PushConst/PushInt/PushEmpty/
PushConstWide/Arith 族/Cmp 族（slot+var 形式）/Jump/Pop/Dup/Lt.../
ForNextIncrVar。**白名单外一律 flush+reload**（fail-safe：新增 op
默认安全）。实现 = 每 opcode 一位的静态表 + 执行循环 O(1) 查表。

## 边界清单（每项的处置）

| 边界 | 处置 |
|---|---|
| proc 内 `global x` 写全局 | reload-after：分派后槽从全局拉回 |
| 数组化（`set a(1) 2`，a 已槽位） | degrade_frame_local_here 迁移既有机制 + 分派边界 flush/reload 覆盖 |
| 变量 trace（读/写） | 编译/执行守卫：var_traces 非空 → 不启用槽模式（bytecode_applicable 同款） |
| namespace eval 体 | current_namespace != "::" → 不启用（resolve 规则不同） |
| upvar 链接顶层变量 | 链接目标 = 全局表；reload-after 拉回变化 |
| `unset x` | 分派 → flush（x 若脏先落）+ reload（unset 后读报 not-found ✓） |
| exec_traces（调试） | bytecode_applicable 已拒 → 不进 slot 模式 |

## 实施切片（每片独立绿）

1. **C1 编译器**：eval.rs 顶层编译改走 locals_mode（params=[]，
   aligned=Some(0)）——ByteCode 侧零新机制（add_local 按需已有）。
2. **C2 执行**：exec_bytecode 的顶层 slot 分支（合成帧 push/pop +
   白名单边界表 + LoadLocal/StoreLocal/IncrLocal 的 tslots 臂）。
3. **C3 门禁**：既有 judge/battery/probes/fuzz 全量 + 新形状（顶层
   循环纯 op / 分派混合 / 数组混合 / trace 关闭断言）。
4. **C4（可选）**：顶层单元 params_aligned=Some(0) 天然满足 fast
   gate 前置——顶层 fast 化（fast-tier 驱动跑顶层单元）作为后续。

## 预期收益

顶层 arith 循环 94 → ~30ns/iter（**3×**，解释器直受益）；顶层
loop-only 42 → ~25ns。与 JIT 无关——纯解释器性能（用户指令「解释器
性能也非常重要」）。JIT 侧（C4）为衍生收益。
