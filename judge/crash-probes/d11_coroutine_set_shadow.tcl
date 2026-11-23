# crash-probe: d11 coroutine set shadow
source [file join [file dirname [info script]] _chk.tcl]
proc p {} { set x 1; return $x }
coroutine set apply {{} {yield 99}}
chk rc [catch {p} e] 1
