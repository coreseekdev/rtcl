set s0 [clock seconds]
set x {}
for {set i 0} {$i<100000} {incr i} { set x [list $x {}] }
set s1 [clock seconds]
unset x
set s2 [clock seconds]
puts "loop=[expr {$s1-$s0}]s unset=[expr {$s2-$s1}]s"
