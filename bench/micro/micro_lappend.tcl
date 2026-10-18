proc e {n} { set L {}; for {set i 0} {$i < $n} {incr i} { lappend L $i }; return [llength $L] }
puts [e 1000000]
