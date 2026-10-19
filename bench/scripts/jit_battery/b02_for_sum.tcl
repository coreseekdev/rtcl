set s 0
for {set i 0} {$i < 100} {incr i} {set s [expr {$s + $i * 2}]}
puts $s
