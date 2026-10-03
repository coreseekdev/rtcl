# top-level: the expr frame appends (no enclosing word to defer)
catch {expr {[nosuch a]}} m
puts $::errorInfo
puts ===
# empty bracket operand
catch {set q [expr {[] + 1}]} m2
puts $m2
puts $::errorInfo
puts ===
# if-condition with bracket operands (compiled condition)
set s abc
if {[string length $s] > 2} { puts big } else { puts small }
# while condition re-runs its bracket every iteration
set L {}
set n 0
while {[llength $L] < 3} { lappend L [incr n] }
puts $L
