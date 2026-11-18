# int 溢出守卫落宿主（wide/bignum 全语义），float lane on 也同文。
# 历史：Mul 守卫深度硬编码（classic 布局 br 1 = done，跳过宿主且 dst
# 不写）——3037000499*7 实证产出陈旧 dst；C-float 参数化后回归面。
proc mul_big {} { expr {3037000499 * 7} }
puts [mul_big]
proc mul_wide {} { expr {4611686018427387903 * 2} }
puts [mul_wide]
proc add_overflow {} { expr {4611686018427387903 + 4611686018427387903} }
puts [add_overflow]
proc sub_underflow {} { expr {-4611686018427387904 - 1} }
puts [sub_underflow]
proc shl_wide {} { expr {1 << 62} }
puts [shl_wide]
proc mul_var {} { set x 7; expr {3037000499 * $x} }
puts [mul_var]
