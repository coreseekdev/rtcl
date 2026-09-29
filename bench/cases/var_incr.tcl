# variable access + incr in tight while loop
set i 0
set n 200000
while {$i < $n} {
    incr i
}
puts $i
