set x [expr {2**62}]
incr x [expr {2**62}]
puts $x
set y 9223372036854775807
incr y [expr {1}]
puts $y
