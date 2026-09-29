set x {}
puts [catch {set x(1) a} m]; puts $m
puts [catch {set nosuch(1) a} m]; puts $m
array set a {1 one}
set a(2) two
puts $a(2)
