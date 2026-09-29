# arithmetic-heavy loop: expr dispatch + variable access
set sum 0
for {set i 0} {$i < 100000} {incr i} {
    set sum [expr {$sum + $i * 2}]
}
puts $sum
