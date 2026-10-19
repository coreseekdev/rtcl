proc p {} {
  set s 0
  for {set i 0} {$i < 3} {incr i} {
    set s [expr {$s + $i}]
    if {$i == 1} {error "mid loop"}
  }
  return $s
}
catch {p} m
set m
set ::errorInfo
