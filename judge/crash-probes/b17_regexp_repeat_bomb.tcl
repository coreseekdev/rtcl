# crash-probe: b17 regexp repeat bomb
source [file join [file dirname [info script]] _chk.tcl]
chk rc [catch {regexp {((a{255}){255}){255}} a} e] 1
