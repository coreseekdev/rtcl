# crash-probe: d07 regex budget
source [file join [file dirname [info script]] _chk.tcl]
set s [string repeat a 2000]
chk match [regexp {a+()\1x} ${s}x] 1
