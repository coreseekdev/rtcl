set x "{"
puts [catch {expr {"a" in $x}} m]
puts $m
