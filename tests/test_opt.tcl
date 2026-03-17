# Optimization end-to-end tests

# --- Short-circuit && ---
set x 0
set r1 [expr {$x != 0 && 10 / $x > 1}]
puts "sc_and_false: $r1"
# expected: 0 (no divide-by-zero)

set y 5
set r2 [expr {$y != 0 && 10 / $y > 1}]
puts "sc_and_true: $r2"
# expected: 1

# --- Short-circuit || ---
set r3 [expr {1 || 10 / 0 > 1}]
puts "sc_or_true: $r3"
# expected: 1 (no divide-by-zero)

set r4 [expr {0 || 1}]
puts "sc_or_false_then_true: $r4"
# expected: 1

# --- Float arithmetic ---
set pi 3.14
set r5 [expr {$pi * 2}]
puts "float_mul: $r5"
# expected: 6.28

set r6 [expr {1.0 + 2.5}]
puts "float_add: $r6"
# expected: 3.5

# --- Nested short-circuit ---
set a 0
set b 1
set r7 [expr {$a && ($b && 10 / $a > 1)}]
puts "nested_sc: $r7"
# expected: 0

# --- Mixed || and && ---
set r8 [expr {1 || (0 && 10 / 0)}]
puts "mixed_sc: $r8"
# expected: 1

# --- Tail call optimization ---
# factorial via tailcall — should NOT overflow the stack
proc fact_iter {n acc} {
    if {$n <= 1} {
        return $acc
    }
    tailcall fact_iter [expr {$n - 1}] [expr {$n * $acc}]
}
set r9 [fact_iter 10 1]
puts "tco_fact: $r9"
# expected: 3628800

# mutual tail-call: even/odd
proc tc_even {n} {
    if {$n == 0} { return 1 }
    tailcall tc_odd [expr {$n - 1}]
}
proc tc_odd {n} {
    if {$n == 0} { return 0 }
    tailcall tc_even [expr {$n - 1}]
}
set r10 [tc_even 100]
puts "tco_even: $r10"
# expected: 1

set r11 [tc_odd 99]
puts "tco_odd: $r11"
# expected: 1

# deep tailcall — would overflow without TCO
proc countdown {n} {
    if {$n <= 0} { return done }
    tailcall countdown [expr {$n - 1}]
}
set r12 [countdown 5000]
puts "tco_deep: $r12"
# expected: done (no stack overflow)

puts "all tests done"
