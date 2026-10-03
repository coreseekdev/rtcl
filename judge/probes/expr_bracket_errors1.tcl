# the classic fib shape: brackets as expr operands
proc fib n {
    if {$n < 2} {
        return $n
    }
    expr {[fib [expr {$n - 1}]] + [fib [expr {$n - 2}]]}
}
puts [fib 15]
set s 0
for {set i 0} {$i < 5} {incr i} {
    incr s [expr {[fib $i] % 7}]
}
puts $s
