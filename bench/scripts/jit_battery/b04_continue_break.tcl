set s 0
for {set i 0} {$i < 10} {incr i 2} {
  if {$i == 4} { continue }
  if {$i == 8} { break }
  set s [expr {$s + $i}]
}
puts $s
