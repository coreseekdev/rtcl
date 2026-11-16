# M2 失效矩阵②：upvar 别名写（proc 经 upvar 改顶层变量）——调用后
# incr 必须读到别名写的新值（invalidate + 宿主路由）。
set n 0
proc bump {} { upvar #0 n m; set m 41 }
incr n
bump
incr n
puts $n
