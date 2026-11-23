# crash-probe: a12 pack neg offset
source [file join [file dirname [info script]] _chk.tcl]
set v {}
chk pack [catch {pack v 1 le 0 -1} e] 1
