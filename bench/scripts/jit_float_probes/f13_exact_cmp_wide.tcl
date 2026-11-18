# wide/double 比较精确裁决（tclsh TclCompareTwoNumbers 精确路径）：
# >2^53 的 imm 提升成 f64 会收缩成邻值 → 恒等分歧。float lane 的比较
# 语境 exact_only：|v|<2^53 才提升，越界落宿主。浮点收边 + proc 编译体。
proc cmp_exact {} { expr {9007199254740993 > 9007199254740992.0} }
puts [cmp_exact]
proc cmp_eq {} { expr {9007199254740993 == 9007199254740992.0} }
puts [cmp_eq]
proc cmp_rev {} { expr {9007199254740992.0 < 9007199254740993} }
puts [cmp_rev]
proc cmp_mixed_var {} {
    set i 9007199254740993
    set d 9007199254740992.0
    expr {$i > $d}
}
puts [cmp_mixed_var]
proc cmp_inexact_ok {} { expr {4503599627370495 > 4503599627370494.0} }
puts [cmp_inexact_ok]
proc arith_rounds {} { expr {9007199254740993 + 0.0} }
puts [arith_rounds]
