proc p {} {
  lappend g a
  lappend g b
  return $g
}
puts [p]
