set acc 0
incr acc [expr {1}]
incr acc [expr {1}]
puts [info exists acc]
set lst {}
lappend lst [incr acc [expr {10}]]
puts $lst
