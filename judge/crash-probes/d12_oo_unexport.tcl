# crash-probe: d12 oo unexport
source [file join [file dirname [info script]] _chk.tcl]
oo::class create A { method m {} { return 1 } }
set o [A new]
oo::objdefine $o unexport m
chk rc [catch {$o m} e] 1
