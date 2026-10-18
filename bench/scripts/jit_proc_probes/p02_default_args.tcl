proc p {a {b 10}} {return [expr {$a + $b}]}
puts [p 5]
puts [p 5 20]
