proc build {n} {
    set L {}
    for {set i 0} {$i < $n} {incr i} { lappend L $i }
    return $L
}
proc p {L} {
    set sum 0
    foreach i $L { set sum [expr {$sum + $i}] }
    return $sum
}
set n 200000
set L [build $n]
set t [time {p $L} 5]
puts "proc-foreach: $t"
