set s 0
while {$s < 9} {
  incr s
  set x [expr {$k + -1}]
  set x $k
}
lappend s 2147483648 && "foo bar"
set x [expr {-33 != $x ^ ""}]
