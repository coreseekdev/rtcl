# M2 失效矩阵④：变量 trace 存在（trace add = 命令 → 缓存窗口关闭，
# 每次写都过 interp → 回调计数与解释器一致）。
set n 0
set count 0
proc tr {args} { incr ::count }
trace add variable n write tr
set n 1
incr n
incr n
puts "n=$n count=$count"
