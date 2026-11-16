incr m
set k 0
while {$k < 6} {
  incr k
  set m [expr {~($x)}]
}
set x 0
set a $i
set a [expr {$i || -1152921504606846976 + $k}]
