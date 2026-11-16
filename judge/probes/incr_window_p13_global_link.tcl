set g 0
proc p {} { global g; incr g [expr {5}]; return $g }
puts [p]
proc up {} { upvar #0 h v; incr v [string trim " 4 "]; return $v }
puts [up]
