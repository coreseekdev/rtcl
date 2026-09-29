# dict commands
set d [dict create a 1 b 2]
puts [dict get $d a]
puts [dict keys $d]
puts [dict size $d]
puts [dict exists $d b]
puts [dict exists $d z]
dict set d c 3
puts [dict get $d c]
puts [dict size $d]
dict unset d a
puts [dict exists $d a]
puts [dict keys $d]
# nested
set nd [dict create outer [dict create inner "deep"]]
puts [dict get $nd outer inner]
dict set nd outer inner2 "deep2"
puts [dict get $nd outer inner2]
# dict with / incr / append
dict with d { puts "b=$b c=$c" }
