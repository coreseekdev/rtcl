set w [string cat "\{" x]
puts [catch {lindex $w 0} m]; puts $m
set v [string cat "\{" k " " v]
puts [catch {llength $v} m]; puts $m
