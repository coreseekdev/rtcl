set d [dict create a 1 b 2]
set acc 0
foreach k [dict keys $d] { incr acc [dict get $d $k] }
puts "$acc [dict size $d]"
