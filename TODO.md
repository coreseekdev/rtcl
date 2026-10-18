# TODO — 待优化/待完成任务清单

> **同步约定**：本清单与 `bench/BASELINE.md` 的台账保持同步——每完成
> 一项，在此勾销并在 BASELINE 记录实证数字；每新增一项，先在 BASELINE
> 补分析与方案再入列。条目按预期收益排序。

## 性能优化（剩余杠杆）

- [x] **name-keyed 超指令（Round G，本批次提交）**：全局/eval 级循环的
  `CmpVar*Jump*`/`AddVar*`/`LappendVarVar`/`IncrVarPop`/`StoreLocalPop`；
  `ByteCode::unfused()` 桥保 JIT 全绿（`docs/JIT-OPCODE-DELTA.md`）。
  dloop 0.69x、sum_only 0.72x、fe 1.29x（指令数比）。
- [ ] **JIT 原生 lower（可选，JIT 侧）**：接受 A 族并直接 lower，免去
  `unfused()` 展开（清单见 `docs/JIT-OPCODE-DELTA.md` §4）。
- [x] **nanopass 融合（Round F，已提交 3964c97）**：`fuse_slot_pairs` → 
  `fuse_pairs` pass + 融合 op（cmp+jcc ×4、lappend/incr pop 版）——
  build-only 1.29x→0.85x 指令。
- [x] **fe 求和迭代（p 段，Rounds G–K 收尾）**：p 循环 9 op→**2 op**/迭代
  （`SET_SS_ADD` + `FOREACH_NEXT_CLOSE`），foreach_bind 快路径、body-mark
  折叠、融合分支直取布尔——fe 0.96x 指令 / 0.85x wall、build 0.76x。
- [ ] **表示层重构（用户已授权，Tcl byte-2-byte 语义为红线）**——deep-dive 批次：
  1. **`Value` 字符串表示层可共享**：`ValueInner.string` 从 `OnceCell<String>`
     改为可共享句柄（如 `StrRep::Shared(Rc<str>) | Owned(Box<str>)`）+
     `Value::str_handle() -> Option<Rc<str>>`：常量池/派发站点名共享同一句柄，
     命令名/变量名的比较走 **ptr 快路径**（oovars/dloop 画像里 memcmp ~8%
     + hashbrown 探测的字符串比较部分是目标）。注意 `append`/字符串改写走
     COW；JIT 不触 Value 内部（仅 ABI）✓。
  2. **`VarMap/VarSet` 键 `Rc<str>` + ptr-first Eq**（`Borrow<str>` 保 &str
     查询）：link/prelink/OO memo 路径用共享句柄插查，探针 eq 去 memcmp；
     插入时用已有的 memoized `Rc`（而非新 String）。
  3. `call_proc` 帧准备缓存（按 ProcDef 预置 slots/绑定顺序）——fib 画像
     call_proc 12%。
  4. 空循环基线（dispatch + region 簿记）：`bodies` 预留 + unchecked push、
     `VmState` 帧指针缓存（unsafe 面小、收益 ~5-8%）。
- [x] **OO prelink 去税 + 变量语义修正（Rounds Q/R）**：注册一次性、
  `(qualified,ns_part)` 预计算、去冗余 seed、memo 身份补 ns —— oovars
  0.69x→**0.58x** 指令 / 0.96x→0.70x wall；新增 judge 回归
  `oo_instance_vars.tcl`（88/88）。
- [x] **dloop 画像结论（0.68x）**：剩余成本 = 派发式 `incr acc [dict get …]`
  的名字扫描（`split_array_ref`/`redirect_flat`，每次访问 2-3 扫）与 dict
  插入（IndexMap rehash/alloc，摊销固有）——进一步需下表的 VarMap 键表示层。
- [x] **rtcl-vm 边界 unfused()**（Round P）：`execute()` 入口展开融合 op
  （与 JIT 同桥）；canonical 集之外的面（lappend 族）维持其既有显式拒绝。
- [ ] **OO 派发 per-call 字符串面（Round O 画像，oovars 0.69x）**：
  1. prelink 索引预计算：链 memo 构建时把 `(slot_idx, key, name)` 预先算好
     （编译体 locals 已知），每调用安装省去 `resolve_var_link` memo 探测 +
     `slot_index_of` 线性扫描（≈5% oovars）。
  2. `ChainEntry` 携带 owner 变量表 `Rc`（9 个构建点），去每调用
     `owner_variables` 的表查找（≈2%）。
  3. 派发站点级 OO memo（`ResolvedCmd` 增 OoMethod 变体 + backfill 探测），
     去 KeyMemo/ChainMemo 的每调用哈希+memcmp（≈4-6%，改动大）。
  4. `head_cache` 的 typed 比较 → 与调用站点共享 `Rc<str>` 身份（需
     `Value` 字符串 rep 可暴露 Rc，或站点级持有）。
- [ ] **下一批（按预期收益，Round L+ 候选）**：
  1. `BEGIN_CMD` 的 `word_srcs` 借用化：`cur_cmd_word_srcs` 改 `(*const
     SrcSpan, len)` 新type（site/单元生命周期内有效；save/restore 只拷指针），
     去每命令 rc++/--——p_only 画像里 `op_begin_cmd` 5% + `drop<Rc<Vec<SrcSpan>>>`
     3.7%；预期 fe/fib ~5%。**unsafe，需严肃审 lifetimer**（跨嵌套 eval 的
     saved 引用链：调用者的单元在其 `exec_bytecode` 帧存活期间一直有效）。
  2. fib 调用路径：`call_proc`（画像 11%）——参数绑定零拷贝化、`slot_mode`
     门的检查合并、`ns_of_qualified` 的每调用开销。
  3. 空循环基线 524 vs 337 instr/迭代：dispatch + region 簿记（Vec push/pop）；
     候选 = `bodies` 预留 + unchecked push（unsafe）、`frames.last()` 缓存
     （VmState 帧指针 + generation）——深水区。
  4. 更多语句窗口：`incr x [expr {...}]`、`set x [cmd ...]`（含 dispatch 的
     bracket 结果窗口）、`[SubEnd, DynCall/Call]`。
- [ ] **foreach 路由对齐**（G9f 存档，G24 已落 framed 双模式的一半）：
  全局编译 foreach 已工作（G21 实验 + G24 framed）✅ **已完成的
  部分：全局内联 + framed 错误协议**。剩余：`uplevel`/`namespace eval`
  上下文的 `foreach_lexical` 路由镜像验证（`foreach_inline_shape`
  的 word-srcs 条件）——用 sweep 逐上下文验证。
- [ ] **vars/oo 链接变量 1.7-1.8× → 指针稳定存储**：VarMap newtype
  （封装 insert/remove/clear/retain，自动 generation bump）+
  链接变量的 `*const Value` 指针缓存（traced 变量走原路径）。
  方案存档于 BASELINE G9f 节。预期 vars/oo 收敛至 ~1.2×。
- [ ] **dloop 收尾 1.05×**：cmd_dict 的 IndexMap 查找为固有大头
  （48%）；`get_index_of` 字符串键 → 可尝试 Rc<str> 键 + 指针缓存。
  `bench/scripts/dloop.tcl`。
- [ ] **oo 1.19× 收尾**：[typed, method] 头 Values + args Vec 已池化
  （G25）✅。剩余：`invoke_method`/`exec_chain_entry` 的分层调用
  （3 层函数）可合并；`resolve_object_arg`（String 化）的调用点
  梳理。`bench/scripts/oo_bench2.tcl`。

## 性能候选（评估结论存档，未排期）

> 来自 clippy 归零批次（type_complexity alias 化）途中的性能评估——
> 本轮只记录不实施；bench 显示热点后再入「性能优化」。

- [ ] **`Rc<Vec<ChainEntry>>` 双重间接 → `Rc<[ChainEntry]>`**：切掉一层
  Vec 头部（oo 链 memo 与 `ActiveMethod` 快照共用该形状）。机械小改，
  列为第一个 bench 候选（`bench/scripts/oo_bench2.tcl`）。
- [ ] **嵌套 memo 复合键**（`chain_memo`/`key_memo`，现 `ChainMemo`/
  `KeyMemo`）：双层 `FxMapK<Rc<str>, …>` = 每次查找两次哈希 + 两次
  Rc 间接。候选：单层复合键 `(Rc<str>, Rc<str>)`（(ns, name) 对，一次
  哈希）。**优先级低**——这些是冷路径 memo（miss 才付出），bench 显示
  OO 热点后再做。
- [ ] **offset+len 替代 `Rc<str>`（用户提议，评估为大手术）**：当前
  `Rc<str>` 已通过 refcount 免拷贝（parse_cache 以整段源码为键，word
  级无重复分配）；改 span 模型需动 parser 的 Word 表示与缓存键设计。
  收益点在 wasm 侧（减少分配器往返）。前置条件：bench 显示源码分配
  占比 >5%。

## 语义对齐（与 specs/DIVERGENCES.md 同步，复核 2026-10-07）

- [x] **case 级 4072/4072 严格对齐达成**：全量审计确认 0 项 rtcl 引起
  的语义分歧；gen_namespace-old 的 5 例 FAIL 为 tclsh 独立运行时的
  上下文失败（缺跨 case setup），rtcl 逐字节复现（非 rtcl 缺陷，
  见 DIVERGENCES.md"非分歧"节）
- [ ] 可选：extract.py 补跨 case setup 提取，使 5 例语料自洽
  （tclsh 侧协议变更，谨慎）
- [ ] `interp alias {} name {} target` 创建形式（空解释器占位参数；
  唯一 OPEN 语义分歧，严重度低——DIVERGENCES.md D-1）
- [x] 整数溢出 f64 提升 —— 已改为精确 BigInt 宽化（gen_expr 大数幂
  用例逐字节一致）
- [x] rename 遮蔽过程后旧名解析 —— 复核为两引擎一致（原记录有误）

## 基础设施

- [ ] rtcl-core `embedded` no_std 构建修复（G5 起破损，~2200 错误；
  OnceCell/线程本地 freelist 的 no_std 替代是主因）
- [ ] perf 依赖：`kernel.perf_event_paranoid` 每次重启回 4——写入
  sysctl.d 或 CI 环境预设
- [ ] JIT（rtcl-jit）暂停中：内存管理/编译管线设计已定稿（路线 C，
  `docs/JIT-WASM-GC.md`）；任务包已就绪（`TASK-JIT-WASM.md`，重启时
  直接派发给新 agent）；若重启，按新文档 M1'（ABI v2 + Int 快车道）
  执行（见 HANDOFF.md §5/§6；解释器为主要引擎的前提下优先级最低）
- [ ] JIT 侧步数预算：A1 eval 步数预算（`set_max_steps`）只覆盖解释器
  与 vm_exec 回边——JIT 编译的 proc 内纯控制流循环（`while {1} {}`
  编进 wasm 原生循环）绕过全部计步点。**方案已定**：fuel/epoch
  imported mutable globals（`docs/JIT-WASM-GC.md` §3），随 M1' 落地。
- [ ] lazy-string（独立于 JIT 的 wasm 边界优化，可先行，设计见
  `docs/JIT-WASM-GC.md` §6）：`InternalRep::HostStr(u32)` +
  `HostStringBacking` trait（choke point = `value.rs` 的 as_str 物化
  入口）；rtcl-wasm 默认 on。前置：string.allium 加 astral probe 裁决
  `string length` 三方语义。

## 已完成（近三轮，供对照）

- [x] G22 槽内存减半（Vec<Value> + UNSET 哨兵）
- [x] G21 槽超指令融合（Add/Cmp 的 slot 变体）
- [x] G20/G19 Fx 哈希（expr_check/namespaces/code/bytecode/OO 表/备忘录）
- [x] G18 arrays_env_only 标志
- [x] G17 LappendVar 内联快速路径
- [x] G16 执行器栈裸 Value 化
- [x] G12+G14 内联整数 Value + 全算术/比较立即数快速路径
- [x] G13 链接键缓存（单次 globals 探测）
