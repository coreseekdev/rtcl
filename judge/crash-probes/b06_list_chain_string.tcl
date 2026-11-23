# crash-probe: b06 list chain string
source [file join [file dirname [info script]] _chk.tcl]
set l x
for {set j 0} {$j<16000} {incr j} { set l [list $l] }
chk len [string length $l] 1
