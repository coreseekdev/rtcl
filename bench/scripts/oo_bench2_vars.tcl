oo::class create C {
    variable acc
    method compute {x y} { set acc [expr {($x + $y) % 1000}]; return $acc }
}
C create c
set n 0
for {set i 0} {$i < 1000000} {incr i} { set n [c compute 7 3] }
puts $n
