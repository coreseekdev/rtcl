# TODO — 待优化/待完成任务清单

> **同步约定**：本清单与 `bench/BASELINE.md` 的台账保持同步——每完成
> 一项，在此勾销并在 BASELINE 记录实证数字；每新增一项，先在 BASELINE
> 补分析与方案再入列。条目按预期收益排序。

## 性能优化（剩余杠杆）

- [ ] **fe 求和迭代 2.8× → 收尾**：foreach_bind + 9 ops 的 per-op 权重
  （9ns/op vs tclsh 3.5ns）。`bench/scripts/fe_bench.tcl`、
  `bench/scripts/sum_only.tcl`。依赖 foreach 路由对齐（下条）+ 每op税。
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

## 语义对齐（已知分歧，5/4072）

- [ ] rename 遮蔽过程后旧名解析（probe：`judge/probes/`）
- [ ] `interp alias {} name {} target` 创建形式
- [ ] 3 条错误信息帧计数微差（gen_* 语料可定位具体用例）

## 基础设施

- [ ] rtcl-core `embedded` no_std 构建修复（G5 起破损，~2200 错误；
  OnceCell/线程本地 freelist 的 no_std 替代是主因）
- [ ] perf 依赖：`kernel.perf_event_paranoid` 每次重启回 4——写入
  sysctl.d 或 CI 环境预设
- [ ] JIT（rtcl-jit）暂停中：若重启，先评估 M1 里程碑
  （见 HANDOFF.md §5；解释器为主要引擎的前提下优先级最低）

## 已完成（近三轮，供对照）

- [x] G22 槽内存减半（Vec<Value> + UNSET 哨兵）
- [x] G21 槽超指令融合（Add/Cmp 的 slot 变体）
- [x] G20/G19 Fx 哈希（expr_check/namespaces/code/bytecode/OO 表/备忘录）
- [x] G18 arrays_env_only 标志
- [x] G17 LappendVar 内联快速路径
- [x] G16 执行器栈裸 Value 化
- [x] G12+G14 内联整数 Value + 全算术/比较立即数快速路径
- [x] G13 链接键缓存（单次 globals 探测）
