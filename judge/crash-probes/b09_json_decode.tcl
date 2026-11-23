# crash-probe: b09 json decode
source [file join [file dirname [info script]] _chk.tcl]
set s [string repeat {[} 30000]
append s [string repeat {]} 30000]
set r [catch {json decode $s} e]
chk rc $r 0
