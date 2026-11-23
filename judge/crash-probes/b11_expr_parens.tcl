# crash-probe: b11 expr parens
source [file join [file dirname [info script]] _chk.tcl]
set n 100000
set s [string repeat "(" $n]
append s 1 [string repeat ")" $n]
chk rc [catch {expr $s} e] 1
