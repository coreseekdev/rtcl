# incr / append / unset / info
set i 0
incr i
puts $i
incr i 5
puts $i
incr i -2
puts $i
set s "ab"
append s "cd" "ef"
puts $s
puts [info exists i]
unset i
puts [info exists i]
puts [info exists nonexistent_var]
# info commands subset check
set cmds [info commands]
puts [expr {[llength $cmds] > 0}]
# global
set g 1
proc bumpg {} { global g; incr g; return $g }
puts [bumpg]
puts $g
