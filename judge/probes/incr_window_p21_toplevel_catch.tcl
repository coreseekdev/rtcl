set acc 0
set rc [catch {incr acc [expr {5}]} r]
puts "$rc $r $acc"
set rc2 [catch {incr acc [bogus]} r2]
puts "$rc2 $r2"
puts $errorInfo
