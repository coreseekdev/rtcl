# crash-probe: d05 upvar0 alias
source [file join [file dirname [info script]] _chk.tcl]
set g 0
upvar 0 g q
set q 5
chk g $g 5
