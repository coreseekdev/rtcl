proc e0 {} {}
set i 0
set t0 [clock milliseconds]
while {$i < 300000} { e0; incr i }
set t1 [clock milliseconds]
puts "empty-proc: [expr {$t1-$t0}]ms"
