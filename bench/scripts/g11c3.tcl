proc e2 {x} { expr {$x + 1} }
set i 0
set t0 [clock milliseconds]
while {$i < 300000} { set r [e2 $i]; incr i }
set t1 [clock milliseconds]
puts "1arg+expr: [expr {$t1-$t0}]ms"
