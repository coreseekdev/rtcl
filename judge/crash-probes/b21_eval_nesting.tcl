# crash-probe: b21 eval nesting
source [file join [file dirname [info script]] _chk.tcl]
set s [string repeat {eval } 2000]
append s 1
chk rc [catch {uplevel #0 $s} e] 1
