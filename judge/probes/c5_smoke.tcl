proc sum {n} {
    set acc 0
    set i 0
    while {$i < $n} {
        if {$i % 2 == 0} {
            set acc [expr {$acc + $i}]
        } else {
            incr acc $i
        }
        incr i
    }
    return $acc
}
puts [sum 10]
proc boom {} { error "kaboom" }
catch {boom} m
puts "catch: $m"
puts "info: [info script]"
proc frames {} { nosuchcmd }
catch {frames} m2
puts "f: $m2"
proc pdemo {} {
    while {1} {
        break
    }
    foreach x {1 2} { append r $x }
    return $r
}
puts [pdemo]
proc tail {n acc} {
    if {$n == 0} { return $acc }
    return [tail [expr {$n - 1}] [expr {$acc + $n}]]
}
puts [tail 100 0]
proc undef {x} { return $nothere }
catch {undef 1} m3
puts "u: $m3"
