set L {}
foreach i {1 2 3} {
    lappend L [expr {$i * 2}]
}
puts $L
set s 0
for {set i 0} {$i < 5} {incr i} { set s [expr {$s + $i}] }
puts $s
proc fib n { expr {$n < 2 ? $n : [fib [expr {$n-1}]] + [fib [expr {$n-2}]]} }
puts [fib 10]
set r [catch {set x [nosuchcmd a b]} m]
puts [list $r $m]
puts [info errorinfo]
