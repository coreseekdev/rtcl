set x 40
proc p {} { return $x }
puts "p=[p]"
puts "direct=[apply {{} {return $x}}]"
