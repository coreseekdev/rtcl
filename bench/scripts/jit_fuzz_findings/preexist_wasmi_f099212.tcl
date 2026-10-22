set s 0
while {$s < 11} {
  incr s
  proc p1 {x} {
  set i $i
  incr i -9
  set k $x
}
p1 123456789
}
incr a
set x 0
while {$x < 7} {
  incr x
  incr s
  puts "$s"
}
set s 0
while {$s < 7} {
  incr s
  set x 1
}
lappend a $b
