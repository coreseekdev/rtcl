set x 1
set n 0
while {$x} { incr n; if {$n > 4} { set x 0 } }
if {$n != 5} { error "n=$n" }
