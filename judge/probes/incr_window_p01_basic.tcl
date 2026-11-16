set acc 0
incr acc [expr {5}]
incr acc [expr {7}]
puts "$acc [incr acc [expr {1}]]"
