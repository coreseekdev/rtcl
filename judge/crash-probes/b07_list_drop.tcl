# crash-probe: b07 list drop
source [file join [file dirname [info script]] _chk.tcl]
set l x
for {set j 0} {$j<160000} {incr j} { set l [list $l] }
unset l
chk drop 1 1
