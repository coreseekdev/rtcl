# crash-probe: b10 json encode
source [file join [file dirname [info script]] _chk.tcl]
set l x
for {set j 0} {$j<20000} {incr j} { set l [list $l] }
set r [catch {json encode $l} e]
chk rc $r 0
