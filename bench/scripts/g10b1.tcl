proc build {n} { set L {}; for {set i 0} {$i < $n} {incr i} { lappend L $i }; return $L }
set L [build 200000]
puts [llength $L]
