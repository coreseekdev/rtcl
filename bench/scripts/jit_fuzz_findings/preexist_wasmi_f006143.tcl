set s abc
set a 0
while {$a < 8} {
  incr a
  set m [expr {~($s)}]
  lappend k $s
}
foreach i {2 4 -5 -5} {
  set n $i
}
incr n 9
set m 0
incr i -9
set s $n
set a 0
set b [expr {1152921504606846976}]
