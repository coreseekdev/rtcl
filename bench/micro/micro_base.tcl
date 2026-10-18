proc e {n} { set i 0; for {set i 0} {$i < $n} {incr i} {}; return $i }
puts [e 1000000]
