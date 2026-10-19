for {set s 0} {$s < 6} {incr s 2} {
  foreach m {-4} {
  set a -78
  proc p2 {n} {
  set a $m
  set m $n
}
p2 2147483648
}
  if {$s >= 5} break
}
foreach k {1 0} i {3 0 -4 2} {
  incr b 9
  proc p3 {x} {
  error "e10"
  lappend i $a
  incr n 4
  return $n
}
p3 -2147483648
}
set x [expr {abc >= 1152921504606846976}]
proc p1 {i} {
  set k -2147483648
  incr m -3
  incr n 10
  return $s
}
p1 1152921504606846976
error "e18"
set m 2147483648
