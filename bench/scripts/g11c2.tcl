proc e1 {x} {}
set i 0
set t0 [clock milliseconds]
while {$i < 300000} { e1 5; incr i }
set t1 [clock milliseconds]
puts "1arg-proc: [expr {$t1-$t0}]ms"
