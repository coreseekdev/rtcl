foreach n {20000 40000 80000} {
    set s [clock seconds]
    set x {}
    for {set i 0} {$i<$n} {incr i} { set x [list $x {}] }
    puts "$n -> [expr {[clock seconds]-$s}]s"
}
