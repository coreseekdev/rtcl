# error framing: expr-region appends, word-sub defers
proc p {} {
    set q [expr {[nosuch a]}]
}
catch {p} m
puts "M:$m"
set ei $::errorInfo
puts [join [lrange [split $ei \n] 0 5] \n]
proc r {} {
    set s [expr {1 + [if {1} {error mid}]}]
}
catch {r} m2
puts "M2:$m2"
set ei2 $::errorInfo
puts [join [lrange [split $ei2 \n] 0 5] \n]
