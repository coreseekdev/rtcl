proc cb2 {a b c} { puts "CB2 $a|$b|$c" }
set r [catch {::namespace inscope :: cb2 alpha {} delete} m]
puts "r=$r m=$m"
proc cb3 {a b c} { puts "CB3 $a|$b|$c" }
set r2 [catch {namespace inscope :: cb3 alpha {} delete} m2]
puts "r2=$r2 m2=$m2"
