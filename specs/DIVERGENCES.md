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
