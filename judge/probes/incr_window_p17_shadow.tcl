proc p {} { set a 0; incr a [expr {1}]; return $a }
puts [p]
rename incr myincr
rename myincr incr
puts [p]
rename incr {}
puts [catch {p} m]
proc incr2 {v amt} { return [myincr $v $amt] }
