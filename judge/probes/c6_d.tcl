set i 0
while {$i < 5} {
  incr i
  if {$i == 3} break
}
puts "i=$i"
