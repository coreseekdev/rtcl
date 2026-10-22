proc p {l} {
  return [lmap x $l {expr {$x * 2}}]
}
puts [p {1 2 3}]
