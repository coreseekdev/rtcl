proc p {L} { set sum 0; foreach i $L { set sum [expr {$sum + $i}] }; return $sum }
set L {}
set i 0
while {$i < 200000} { lappend L $i; incr i }
set z 0
while {$z < 5} { p $L; incr z }
