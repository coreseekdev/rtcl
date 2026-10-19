set i ""
set x 59
set i 0
while {$i < 10} {
  incr i
  proc p3 {b x} {
  for {set k 0} {$k < 5} {incr k 2} {
  set a foo bar
  if {$k >= 4} break
}
}
p3 2147483648 false
}
