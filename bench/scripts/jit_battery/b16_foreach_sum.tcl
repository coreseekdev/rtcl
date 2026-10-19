set s 0
foreach x {1 2 3 4} {set s [expr {$s + $x}]}
puts $s
