proc show {s} {
  set r {}
  for {set j 0} {$j < [string length $s]} {incr j} {
    lappend r [format %x [scan [string index $s $j] %c]]
  }
  puts "len=[string length $s] vals=$r"
}
proc dump {label s} { puts -nonewline "$label "; show $s }
set x0 "\U00020000"
dump case0 $x0
set x1 "\U0000110000"
dump case1 $x1
set x2 "\U000100000000"
dump case2 $x2
set x3 "\U0000FFFFFFFF"
dump case3 $x3
set x4 "\U00011000"
dump case4 $x4
set x5 "\U0000FFFF"
dump case5 $x5
set x6 "\U0010FFFF"
dump case6 $x6
set x7 "\U00110000"
dump case7 $x7
set x8 "\UFFFFFFFF"
dump case8 $x8
set x9 "\U10000000"
dump case9 $x9
set x10 "\U0001F600"
dump case10 $x10
set x11 "\U00D842"
dump case11 $x11
