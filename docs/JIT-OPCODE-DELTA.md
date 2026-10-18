# rtcl-ir opcode 增量说明（给 JIT 侧）

> **crate 布局（2026-10-18 重构后）**：解释器（`interp/`、`types/`、
> `channel/command/host/parser`）已迁入 **`rtcl-vm`**；`rtcl-core` 退为
> **facade**（`rtcl_core::interp/...` 等路径与 feature 名全部保留 → 下游
> import 零改动）。依赖方向：`rtcl-vm` → `rtcl-parser`/`rtcl-ir`；
> `rtcl-core` → `rtcl-vm`（仅 re-export）。`VmContext`（JIT/wasm ABI 回调
> `charge_step` 的 trait）留在 `rtcl-vm` ✓。新增代码请直接用 `rtcl_vm::...`。

> **状态**：`perf/interp-arch-opt` 分支的解释器性能批次（Round F/G）新增了
> 一批「解释器原生超指令」，由 `rtcl-ir` 的纳米趟（nanopass）融合 pass
> `ByteCode::fuse_pairs` 产出。本文件是这些 op 的**语义索引与建议 lower
> 路径**：JIT 的 `pass::classify` 接受集与 `pass/lower.rs` 需要按此更新，
> 否则含这些 op 的单元会整体回退到解释器（「编译拒绝」粒度，见设计文档
> §2）。
>
> 关联：`crates/rtcl-ir/src/opcode.rs`（每个变体的文档）、
> `crates/rtcl-ir/src/bytecode.rs`（`fuse_pairs` 窗口表 + 跳转靶点规则）、
> `bench/BASELINE.md` Round F/G 数字。

## 1. 背景：融合窗口从哪来

编译器为每条命令产出的「生产者 + 消费者」相邻窗口由 `fuse_pairs` 折叠成
一个 op（纯函数重写；窗口尾项是跳转靶点时拒绝融合；`peephole()` 驱动迭代
至收敛，`strip_nops` 负责压缩与靶点重映射）。动机：每个 op 一次派发 + 栈
往返 + 引用计数进出，在循环体里占 ~30–50% 的时间。实测（指令数比
rtcl/tclsh）：build 循环 1.29x → 0.85x、全局 while 循环 12→8 op/迭代。

窗口 → 融合 op 表：

| 窗口 | 融合 op | 类别 |
|---|---|---|
| `[LoadVar a, LoadVar b, ADD/CMP]` | `AddVarVar` / `CmpVarVar` | **JIT 可见** |
| `[LoadVar a, PushInt k, ADD/CMP]` | `AddVarImm` / `CmpVarImm` | **JIT 可见** |
| `[PushInt k, LoadVar b, CMP]` | `CmpVarImm(b, k, cc 镜像)` | **JIT 可见** |
| `[CmpVarVar/Imm, JumpTrue/False]` | `CmpVarVar/ImmJumpTrue/False` | **JIT 可见** |
| `[LoadVar s, LappendVar d, Pop]` | `LappendVarVar` | **JIT 可见** |
| `[IncrVar x, Pop]` | `IncrVarPop` | **JIT 可见** |
| `[LoadLocal s, LappendLocal d, Pop]` | `LappendSlotSlot` | 仅 locals-mode（JIT 不可见） |
| `[BeginCmd s, SubMark]` | `BeginCmdSubMark` | **JIT 可见** |
| `[ExprMark, BeginCmd s]` | `ExprMarkBeginCmd` | **JIT 可见** |
| `[BeginCmd s, PushConst i]` | `BeginCmdPushConst` | **JIT 可见** |
| `[SubEnd, StoreVarPop i]` | `SubEndStoreVarPop` | **JIT 可见** |
| `[SubEnd, Return]` | `SubEndReturn` | **JIT 可见** |
| `[Jump(t=下一个op), NextMark]` | `SwapToNextMark` | **JIT 可见** |
| `[Jump(t=下一个op), ForeachNext]` | `ForeachNextClose` | **JIT 可见** |
| `[NextMark, BeginCmd s]` | `NextMarkBeginCmd` | **JIT 可见** |
| `[BeginCmd s, IncrVarPop i k]` | `BeginCmdIncrVarPop` | **JIT 可见** |
| `[SubEnd, StoreLocalPop d]` | `SubEndStoreLocalPop` | 仅 locals-mode |
| `[BeginCmd s, IncrLocalPop x k]` | `BeginCmdIncrLocalPop` | 仅 locals-mode |

### Round I/M 增补（语句窗口族）

| 窗口 | 融合 op | 类别 |
|---|---|---|
| `[BeginCmdSubMark s1, BeginCmd s2, AddSlotSlot a b, SubEndStoreLocalPop d]` | `SlotSetAddSS{..}` | 仅 locals-mode |
| `[BeginCmdSubMark s1, BeginCmd s2, AddVarVar a b, SubEndStoreVarPop d]` | `VarSetAddVV{..}` | **JIT 可见** |
| `[PushConst i, SubMark]` | `PushConstSubMark` | **JIT 可见** |
| `[SubMark, BeginCmd s]` | `SubMarkBeginCmd` | **JIT 可见** |
| `[ExprMark, BeginCmd s, PushConst i]` | `ExprMarkBeginPushConst` | **JIT 可见** |
| `[LoadLocal a, PushInt k, Sub]` / `[LoadLocal a, LoadLocal b, Sub]` | `SubSlotImm` / `SubSlotSlot` | 仅 locals-mode |
| `[SubEnd, DynCall n]` / `[SubEnd, Call{cmd,argc}]` / `[SubEnd, Add]` | `SubEndDynCall` / `SubEndCall` / `SubEndAdd` | **JIT 可见** |

语句窗口（`SlotSetAddSS`/`VarSetAddVV`）是 4 个原 op 的**逐字合并**：站点设置→
Sub region 开→expr 站点→读/加→region 关→写入（move），任一步失败的
站点/region/错误状态与原窗口逐步相同；`unfused()` 各自直落 7/8 op 规范形。
`SubEndDynCall/SubEndCall` 内部的 dispatch 站点缓存按 `st.pc-1` 键控，首次
执行回填融合 op 的 pc（自愈，无额外协议）。
| `[IncrLocal x, Pop]` | `IncrLocalPop` | 仅 locals-mode |
| `[StoreLocal x, Pop]` | `StoreLocalPop` | 仅 locals-mode |
| `[CmpSlotSlot/Imm, JumpTrue/False]` | `CmpSlotSlot/ImmJumpTrue/False` | 仅 locals-mode |

「仅 locals-mode」的槽族：JIT 目前只经 `Compiler::compile_unit`（name-keyed）
取字节码（`crates/rtcl-jit/src/exec.rs`），proc 体的 locals 编译不会送进
JIT，因此这几族当前对接受集不可见；A 族（name-keyed）会出现在 JIT 消费的
单元里，**必须更新接受集/降级路径**。

## 2. A 族逐 op 语义与建议 lower

所有变量读走 `eval_var_ref`（与 `LoadVar` 完全同路径：数组引用、链接间接、
canonical 错误），写走 `take_var_fast`/`set_var`（与 `StoreVar`/`LappendVar`
同路径）。`cc` 编码沿用既有的超级指令约定：
`0=< 1=> 2=<= 3=>= 4=== 5=!=`。

| op（载荷） | 语义 | 建议 lower（L2/L3 词汇） |
|---|---|---|
| `AddVarVar(a, b)` | push `read(a) + read(b)`（数值语义 `numeric_binop('+')`） | `LoadVar(a) → r1`, `LoadVar(b) → r2`, `Arith Add {r1,r2}` |
| `AddVarImm(a, k)` | push `read(a) + k` | `LoadVar(a) → r1`, `Const k → r2`, `Arith Add` |
| `CmpVarVar(a, b, cc)` | push bool（`cmp_values` 语义：数值优先、NaN/字符串规则与 `Lt/Gt/…/Eq/Ne` 一致） | `LoadVar`×2 + `Cmp` |
| `CmpVarImm(a, k, cc)` | 同上（立即数操作数） | `LoadVar(a)` + `Const k` + `Cmp` |
| `CmpVarVarJumpTrue/False(a,b,cc,t)` | 比较后**直接分支**，不落布尔值：条件成立（want=true 时真 / want=false 时假）→ `pc=t`；否则落空且**开 body region** | `LoadVar`×2 + `Cmp` + `BoolTest/BoolOfBool` + `BrIf`（与现有 `JumpTrue/JumpFalse` 的 Cmp 特化同构） |
| `CmpVarImmJumpTrue/False(a,k,cc,t)` | 同上（立即数操作数） | 同上 |
| `LappendVarVar(src, dst)` | 读 `src` 变量；`lappend` 进 `dst` 变量；**丢弃结果**（`Pop` 已被折叠） | `LoadVar(src) → r` + `LappendVar { var: dst, src: r }` + 丢弃（不做 `SetResult`） |
| `IncrVarPop(idx, k)` | `incr` 变量、丢弃结果 | `IncrVar { keep:false }`（或 `IncrVar` + 丢弃） |

### 2b. meta/循环族（第二批次）的语义要点

- 这些 op 都是「簿记 + 簿记/簿记 + 值操作」的逐字合并，错误/站点/region 状态
  在任一步失败时与原序列**逐步相同**；`unfused()` 各自展开回原 op 对
  （含 `SubEndStoreLocalPop → [SubEnd, StoreLocal, Pop]` 这种必须直落规范形的
  展开，避免再产出融合 op）。
- **`SwapToNextMark` / `ForeachNextClose` 的双入口归一化**：一个被消费的
  `NextMark`/`ForeachNext` 可能同时是 `continue` 的落点（`LoopEnter.cont` /
  `ForeachStart.next`）。融合时 `fuse_pairs` 会把「指向被消费 op 的靶点」
  **重定向到融合 op 头**（`retargets` 表，见 `bytecode.rs`）——展开时这两个 op
  是 `[Nop, 原op]`（trampoline Jump 语义上本是无操作跳转）。执行器用
  `LoopFrame`（`bodies_len`/`owner`）归一化 region 栈，与 `op_continue` 同款，
  因此两个入口行为一致、计步平价。
- JIT 若原生 lower：`BeginCmdSubMark/NextMarkBeginCmd/ExprMarkBeginCmd` 即
  对 `SetSite`（+ 各自 region 记号）的组合；`BeginCmdPushConst` = `SetSite` +
  `Const`；`SwapToNextMark`/`ForeachNextClose` = 循环回边区（落 fuel 表口径
  与 `Jump` 回边表一致——它们本身不产生回边，回边在随后的 `Jump` 上）；
  `SubEndReturn` = 转 `Return`。

## 3. 降级时必须保真的三件事

1. **回边计步（fuel）平价**：融合分支把「条件比较 + 跳转」合成一条，JIT
   必须让**原跳转位置**的回边判定不变——即把融合 op 视作**位于窗口头部的
   跳转**：`t < 融合 op 索引 + 1` 判回边（与原 `JumpFalse` 等价）；`cont`/
   `brk` 目标若指向窗口尾部，压缩已把靶点重映射到融合 op 的下一条 op，
   `strip_nops` 的 `forward` 表是唯一权威。
2. **站点/region 语义**：not-taken 路径必须与原 `JumpTrue/JumpFalse` 一样
   开 `Region::Body(cur_site)`（首个 `BeginCmd` 失败的无 region 特例、
   body-of-own-construct 的 frameless 传播都依赖它）；错误帧文本/行号取
   当前 site，不因融合而变。JIT 侧对应 `SetSite` + fail 协议。
3. **值语义**：`*Pop` 形态与 `Lappend*` 融合不物化被丢弃的结果（语义等价，
   引用计数中间态不可观测）；`Add/Cmp` 的非数值/错误路径必须走与
   `numeric_binop`/`cmp_values` 完全相同的宿主回退（字符串解析、bignum、
   报错文案），不得在快车道里自行降级。

## 4. JIT 侧的接线（已做）与可选优化

**已接线（保持全量 JIT 覆盖，零回退损失）**：rtcl-ir 提供
`ByteCode::unfused()` ——把融合 op 展开回**融合前的规范窗口**（含跳转靶点
重映射；测试 `unfused_covers_every_family` 保证所有族可展开）。JIT 的编译
边界已调用它：

- `crates/rtcl-jit/src/exec.rs`（运行路径的 `Compiler::compile_unit(...)`
  之后追加 `.unfused()`）
- `crates/rtcl-jit/tests/{golden,roundtrip,microbench}.rs` 的 `compile_unit`
  助手（golden 的逐行断言因此保持原样）

因此 **JIT 的接受集/降级路径当前无需任何修改**；`classify` 看到的仍是
融合前的 op 形状。

**可选优化（JIT 侧按需，收益 = 免去展开、直接利用融合 op）**：

- [ ] `pass/mod.rs::classify`：原生接受 A 族（§2 表）——`CmpVar*Jump*` 为
      `Flow::Cond(t, want)`，其余 `Flow::Fall`；随后 `escape`/`alloc_locals`
      按 §2 的建议登记读写。
- [ ] `pass/lower.rs`：按 §2 建议映射为现有 `Instr`；`LappendVarVar` 走
      现有 `LappendVar` 宿主路由；`IncrVarPop` 复用 `IncrVar` 并丢弃结果。
- [ ] 完成后再移除 `.unfused()` 接线（golden 会显示 op 形状差异，作为
      评审界面）。
- [ ] `judge --engine jit` 三向差分全绿（含
      `judge/corpus/gen/gen_proc-old.tcl` 的 errorInfo 用例——融合分支的
      fall-through/站点语义由此回归）。

## 5. 附：不变量与工具

- `OpCode` 尺寸簿记测试（`rtcl-ir`，`assert size_of::<OpCode>() <= 16`）：
  新增变体的载荷不得顶出 16 字节信封（dispatch 每 op 拷贝一次枚举）。
- `ByteCode::fallback` 白名单（`rtcl-parser/src/compiler.rs`）已同步收录
  全部新 op——融合不得改变该标志的语义。
- `rtcl-vm`（并行 VM，无 slot 机制）对新族显式拒绝（
  `"lappend ops are not supported here"`），沿用既有的非支持集处理。
- 复现窗口形状：`cargo test -p rtcl-core dump_*`（临时探针）或
  `ByteCode` 的 `Display`（`BEGIN_CMD/CMP_VI_JF/LAPPEND_VV/INCR_VAR_POP`）。
