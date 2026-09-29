puts [expr {[string range "1.50x" 0 3]}]
puts [expr {[string range "123abc" 0 2]}]
puts [expr {1 ? "1e15" : 0}]
puts [expr {[concat "2.50"]}]
puts [expr {[string toupper "abc"]}]
puts [expr {[format %s 1e15]}]
set x "1e15"
if {$x} {puts yes}
