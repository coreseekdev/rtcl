incr s 5
foreach x {-1 -5 4 -1 3} {
  proc p3 {s k} {
  for {set a 0} {$a < 4} {incr a 3} {
  set m -987654321
  if {$a >= 3} break
}
  set i $a
  incr i -7
  return $x
}
p3 0 2147483648
}
puts [expr {$b << 0 * -987654321}]
