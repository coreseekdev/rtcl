# control flow through expr-embedded brackets
set out {}
for {set i 0} {$i < 4} {incr i} {
    if {[expr {$i % 2}] == 0} { lappend out even-$i ; continue }
    lappend out odd-$i
}
puts $out
# continue in an expr bracket in a for BODY routes to the next script
set hits 0
for {set i 0} {$i < 3} {incr i} {
    set t [expr {[continue] + 1}]
    incr hits
}
puts $hits
# return through an expr bracket
proc r {} { expr {1 + [return 42]} }
puts [r]
# error through nested expr brackets, caught
proc deep n { expr {[expr {[nosuch $n]}] + 1} }
catch {deep 3} m
puts $::errorInfo
