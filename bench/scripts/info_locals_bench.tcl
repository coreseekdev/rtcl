# info_locals_bench.tcl — `info locals` in a proc used to degrade the
# whole frame to name-keyed storage for the rest of its life; after T3
# the listing reads the slot storage directly.  Three probes:
#   list     — `info locals` in a fresh frame per call (the listing cost
#              itself, no degrade left to piggyback on);
#   after    — one `info locals`, then a slot-hot loop (set/incr on
#              compiled locals): the degrade used to make every later
#              variable op in the frame take the slow name path;
#   plain    — the same loop with no listing (the floor).
# Prints "<name> <µs/iter>" lines for judge/gates/bench/run.sh.
proc list1 {k} {
    set r [info locals]
    set a $k
    return $r
}
proc after1 {k} {
    info locals
    set acc 0
    for {set i 0} {$i < 40} {incr i} {
        incr acc $i
        set tmp $acc
    }
    return [expr {$acc + $tmp + $k}]
}
proc plain1 {k} {
    set acc 0
    for {set i 0} {$i < 40} {incr i} {
        incr acc $i
        set tmp $acc
    }
    return [expr {$acc + $tmp + $k}]
}
proc rep {name script n} {
    $script 1
    set t0 [clock microseconds]
    for {set i 0} {$i < $n} {incr i} { $script 1 }
    set dt [expr {[clock microseconds] - $t0}]
    puts "$name [expr {$dt / double($n)}]"
}
rep list list1 20000
rep after after1 20000
rep plain plain1 20000
puts DONE
