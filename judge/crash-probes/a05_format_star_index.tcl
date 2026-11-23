# crash-probe: a05 format star index
source [file join [file dirname [info script]] _chk.tcl]
chk xpg [catch {format %3$*.*d 5 6 7} e] 1
