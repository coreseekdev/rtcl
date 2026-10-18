oo::class create C {
    method a {n} { if {$n <= 0} { return 0 }; return [my b [expr {$n - 1}]] }
    method b {n} { if {$n <= 0} { return 0 }; return [my a [expr {$n - 1}]] }
}
set c [C new]
set r 0
for {set i 0} {$i < 100000} {incr i} { set r [$c a 20] }
puts $r
