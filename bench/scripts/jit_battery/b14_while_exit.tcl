set x 1
while {$x} { set x [expr {$x + 1}]; if {$x > 3} { break } }
puts $x
