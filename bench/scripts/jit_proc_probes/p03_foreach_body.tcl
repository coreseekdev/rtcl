proc p {} {
  set s 0
  foreach x {1 2 3} {set s [expr {$s + $x}]}
  return $s
}
puts [p]
