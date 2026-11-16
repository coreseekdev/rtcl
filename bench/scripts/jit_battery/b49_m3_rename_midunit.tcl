# M3 失效矩阵②：Tier1 命令单元中途改名（rename lappend lapp）。
# rename 对新旧键各 bump 一次（proc.rs rename 通用路径）；折叠
# lappend 的单元在同一重分派点退回动态解析——lappend 不复存在，
# 报 invalid command（遮蔽移除方向的 rename 亦由此覆盖）。
set x {}
rename lappend lapp
lappend x 1
puts $x
