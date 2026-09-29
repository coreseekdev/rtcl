set t0 [clock clicks]
set x {}
for {set i 0} {$i<100000} {incr i} { set x [list $x {}] }
set t1 [clock clicks]
unset x
set t2 [clock clicks]
puts "loop=[expr {($t1-$t0)/1000000}]ms unset=[expr {($t2-$t1)/1000000}]ms"
