set s0 [clock seconds]
set y {}
for {set i 0} {$i<100000} {incr i} { set y [list a b] }
puts "flat=[expr {[clock seconds]-$s0}]s"
set s1 [clock seconds]
set x {}
for {set i 0} {$i<10000} {incr i} { set x [list $x {}] }
puts "nested10k=[expr {[clock seconds]-$s1}]s"
