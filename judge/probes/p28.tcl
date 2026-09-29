set d {a 1 b 2}
set r [dict with d {set a $b}]
puts "$r | $d"
set d {a 1 b 2}
dict with d {unset b; dict set d c 3}
puts $d
set d {a 1 b 2}
dict with d x* {puts "inner:$a"}
puts [info exists a]
set d {p {q 1} r 5}
dict with d p {puts "nested:$q"}
puts [catch {dict with nodict {zz}} m]; puts $m
