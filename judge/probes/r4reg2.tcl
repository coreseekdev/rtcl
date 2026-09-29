set x 40
apply {{} { set y 5 }}
puts "apply-set-y=[catch {set y} m]; m=$m"
puts "g=[apply {{} {return $x}}]"
proc p {} { return $x }
puts "p=[p]"
