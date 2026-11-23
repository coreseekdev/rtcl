# crash-probe: b16 regexp nested groups
source [file join [file dirname [info script]] _chk.tcl]
set p [string repeat ( 6000]
append p 1 [string repeat ) 6000]
chk rc [catch {regexp $p 1} e] 1
