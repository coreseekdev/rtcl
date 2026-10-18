proc p {x} {return [expr {$x * 2}]}
puts [p 3]
rename p q
puts [q 4]
