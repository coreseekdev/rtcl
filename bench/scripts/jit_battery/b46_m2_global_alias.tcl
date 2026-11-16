# M2 失效矩阵③：proc 内 global 写顶层变量（循环外）——后续 incr 读新值。
set k 0
proc poke {} { set ::k 100 }
incr k
poke
incr k
incr k
puts $k
