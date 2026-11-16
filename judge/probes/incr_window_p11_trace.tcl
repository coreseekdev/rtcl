set x 0
proc tw {args} { puts "trace:[lindex $args 0]" }
trace add variable x write tw
incr x [expr {5}]
trace remove variable x write tw
proc tr {args} { puts "rtrace:[lindex $args 0]" }
trace add variable x read tr
puts "read:[incr x 0]"
trace remove variable x read tr
