proc p {} { set acc 0; incr acc [expr {5}]; incr acc [string trim " 7 "]; return $acc }
puts [p]
proc q {base} { incr base [expr {3}]; return $base }
puts [q 10]
