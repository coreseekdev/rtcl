# crash-probe: b15 nested cmdsub eval
source [file join [file dirname [info script]] _chk.tcl]
set n 20000
set s [string repeat {[string cat } $n]
append s x [string repeat {]} $n]
set r [catch {uplevel #0 $s} e]
chk rc $r 1
