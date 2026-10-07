# rtcl ↔ tclsh 8.6.17 已知分歧清单

> 格式：行为声明 | Tcl 8.6 行为 | rtcl 行为 | 复现 | 严重度。
> 修复后标 `[FIXED YYYY-MM-DD]`。复核日期：**2026-10-07**（实测逐字节比对）。

## OPEN

### D-1 `interp alias` 的空解释器参数形式
- **行为**：`interp alias {} myalias {} puts` —— 用空字符串占位源/目标
  解释器创建别名。
- **Tcl 8.6**：接受，创建 `myalias` → `puts` 的别名。
- **rtcl**：`wrong # args: should be "interp interp"`（interp 命令的
  alias 子命令未实现空解释器参数形式）。
- **复现**：`interp alias {} myalias {} puts`
- **严重度**：低（`interp alias <ns> ...` 带名形式已支持；空形式罕见）。

## 非分歧（tclsh 侧上下文失败，rtcl 严格对齐）

### gen_namespace-old 2.3 / 6.10 / 6.13 / 6.16 / 6.19（5 例）
- **现象**：tclsh 8.6.17 独立运行语料时自身 FAIL；rtcl 逐字节复现相同
  失败输出（全文件 diff 为空）。
- **根因**：提取器未包含前序 case 的跨 case setup——6.12 重定义
  `trigger` 并创建 `test_ns_cache1::test_ns_cache_var`（6.13/6.16/6.19
  依赖）；2.3 依赖被跳过的 `test_ns_simple::test_ns_x/y` setup。官方
  套件带 harness 运行亦有 3 例 FAIL（8.6.17 自身状态）。
- **结论**：rtcl 与 tclsh 严格对齐（含失败行为）；非 rtcl 缺陷。
  若追求语料自洽，可在 extract.py 补跨 case setup 提取（协议变更，
  需谨慎）。

## FIXED

### [FIXED 2026-10-07] 整数溢出 f64 提升（vs Tcl bignum）
- 原残留：整数溢出提升为 f64；现与 Tcl 一致提升为**精确 BigInt**
  （`numeric_binop` 的 wide 臂 + 立即数路径的 i63 范围守卫宽化）。
- 验证：`bench/scripts/` 的数值探针 + gen_expr expr-23.5x 大数幂
  用例逐字节一致。

### [FIXED 2026-10-07] gen_namespace-old 的 5 个 case 级失败
- 原状态（2026-10-03）：namespace/lsort/variable/proc 边缘组合 5 例。
- 现状态：**逐字节一致**（2026-10-07 实测 diff 为空；HANDOFF §3 的
  5-case 计数已过时，judge case 级 4072/4072）。
