proc p {n} {
  set s 0
  for {set i 0} {$i < $n} {incr i} {
    if {$i == 2} continue
    if {$i > 3} break
    set s [expr {$s + $i}]
  }
  return $s
}
puts [p 10]
