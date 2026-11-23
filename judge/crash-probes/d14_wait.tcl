# crash-probe: d14 wait
source [file join [file dirname [info script]] _chk.tcl]
set p [exec sleep 0.3 &]
after 800
set r [catch {wait $p} e]
chk rc $r 0
chk status [lindex $e 1] 0
