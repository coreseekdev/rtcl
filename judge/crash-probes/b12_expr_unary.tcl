# crash-probe: b12 expr unary
source [file join [file dirname [info script]] _chk.tcl]
set s [string repeat - 50000]
append s 1
chk rc [catch {expr $s} e] 1
