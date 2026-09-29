# list construction and traversal
set l {}
for {set i 0} {$i < 5000} {incr i} {
    lappend l $i
}
set total 0
foreach x $l { incr total $x }
puts "$total [llength $l]"
puts [lindex $l end]
