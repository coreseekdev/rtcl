# crash-probe: b08 dict string
source [file join [file dirname [info script]] _chk.tcl]
set l x
for {set j 0} {$j<16000} {incr j} { set l [dict create k $l] }
set r [catch {string length $l} e]
chk rc $r 0
