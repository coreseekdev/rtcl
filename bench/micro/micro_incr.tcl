proc e {n} { set x 0; for {set i 0} {$i < $n} {incr i} { incr x }; return $x }
puts [e 1000000]
