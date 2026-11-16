set acc 0
incr acc [expr 5 + 1]
puts $acc
incr acc [expr {$acc + 2}]
puts $acc
