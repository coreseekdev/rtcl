set acc 0
foreach k {1 2 3 4 5} {
    if {$k == 2} { incr acc [continue] }
    if {$k == 4} { incr acc [break] }
    incr acc $k
}
puts $acc
proc q {} { set a 0; foreach k {1 2 3} { incr a [break] }; return $a }
puts [q]
