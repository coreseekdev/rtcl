# crash-probe: a11 repeat 4g
source [file join [file dirname [info script]] _chk.tcl]
chk lr [catch {lrepeat 4294967296 x} e] 1
chk sr [catch {string repeat x 4294967296} e] 1
