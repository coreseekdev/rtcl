set x {}
for {set i 0} {$i<100000} {incr i} { set x [list $x {}] }
puts BUILT
unset x
puts UNSET
