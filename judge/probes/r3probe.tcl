puts "V4:[catch {namespace eval foo {set x 1}; set ::foo::x} m]; puts $m"
puts "V6:[catch {uplevel 1 {set q 9}} m2]; puts $m2"
proc f5 {} {return -level 0 -code ok v; puts unreached}
puts "C5:[f5]"
proc f6 {} {return -code return inner}
puts "C6:got=[f6]"
puts "E5:[catch {expr {010}} m3]; puts $m3"
puts "E7:[catch {expr {abc}} m4]; puts $m4"
puts "E10:[catch {expr {1 | 2.5}} m5]; puts $m5"
puts "E11:[catch {expr {1 >> -1}} m6]; puts $m6"
set x "{"
puts "E12:[catch {expr {"a" in $x}} m7]; puts $m7"
proc fv args {return $args}
puts "V3:[fv "a}b" c]"
