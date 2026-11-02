incr s
lappend n -4611686018427387904
set a 1152921504606846976
lmap m {1 2 3 4} {
  set k $n
  set a "on"
  continue
  expr {$m * 2}
}
