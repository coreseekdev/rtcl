oo::class create C {
    method compute {x y} { expr {($x + $y) % 1000} }
}
C create c
set n 0
for {set i 0} {$i < 1000000} {incr i} { set n [c compute 7 3] }
puts $n
