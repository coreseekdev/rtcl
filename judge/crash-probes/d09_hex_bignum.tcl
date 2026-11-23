# crash-probe: d09 hex bignum
source [file join [file dirname [info script]] _chk.tcl]
set v 0xFFFFFFFFFFFFFFFF
chk val [expr {$v + 0}] 18446744073709551615
