proc p {n} {
  set s 0
  while {$n > 0} {
    set s [expr {$s + $n}]
    set n [expr {$n - 1}]
  }
  return $s
}
puts [p 5]
