oo::class create C { method compute {x y} { expr {($x + $y) % 1000} } }
C create c
set o c; set m compute
set n 0
for {set i 0} {$i < 1000000} {incr i} { set n [$o $m 7 3] }
puts $n
