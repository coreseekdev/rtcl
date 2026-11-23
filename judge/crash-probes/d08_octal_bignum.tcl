# crash-probe: d08 octal bignum
source [file join [file dirname [info script]] _chk.tcl]
set v 01000000000000000000005
chk val [expr {$v}] 9223372036854775813
