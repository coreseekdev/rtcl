# 宽整值提升与守卫拒绝：imm 上限 → convert 舍入；超 i63 句柄 → 宿主
puts [expr {4611686018427387903 + 0.0}]
puts [expr {123456789012345678 * 1.0}]
set big 9223372036854775807
puts [expr {$big * 1.0}]
puts [expr {$big / 2}]
