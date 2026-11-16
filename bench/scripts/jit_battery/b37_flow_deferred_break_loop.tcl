# deferred break 落调用方循环：循环出口语义（非顶层解码路径）。
proc p {} {return -code continue}
set n 0
while {$n < 2} {incr n; p}
puts "loop $n"
