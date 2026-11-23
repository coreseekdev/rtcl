# crash-probe: a07 clock format neg
source [file join [file dirname [info script]] _chk.tcl]
set r [catch {clock format -100000000000000000} e]
chk neg17 $r 0
set r [catch {clock format -2305843009213693952} e]
chk neg2_61 $r 0
