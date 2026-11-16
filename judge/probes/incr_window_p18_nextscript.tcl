set s 0
for {set i 0} {$i < 6} {incr i [expr {2}]} { set s [expr {$s + $i}] }
puts "$i $s"
