proc e {n} { set sum 0; for {set i 0} {$i < $n} {incr i} { set sum [expr {$sum + $i}] }; return $sum }
puts [e 1000000]
