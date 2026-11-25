# 空转（T11b 普查 2026-11-26）：末条检查 `expr {1 << 99999999}` 两边都超 15s 挂死（前 4 条检查已打印，cnt_big 从未返回）
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t cnt_f {expr {1 << 2.5}}
t cnt_s {expr {1 << "abc"}}
t both_f {expr {1.5 << 2.5}}
t cnt_neg {expr {1 >> -1}}
t cnt_big {expr {1 << 99999999}}
