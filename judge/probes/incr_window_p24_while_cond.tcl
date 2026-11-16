set n 0
set i 0
while {$i < 3} { incr n [expr {2}]; set i [expr {$i + 1}] }
puts $n
