set a(1) 0
incr a(1) [expr {5}]
puts $a(1)
if [catch {incr nosuch(2) [expr {1}]} msg] { puts "E:$msg" }
