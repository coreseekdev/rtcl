namespace eval n { variable v 1 }
incr n::v [expr {5}]
puts $n::v
proc p {} { incr n::v [string trim " 2 "]; return $n::v }
puts [p]
