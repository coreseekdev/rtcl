# crash-probe: a06 sleep invalid
source [file join [file dirname [info script]] _chk.tcl]
chk inf [catch {sleep inf} e] 1
chk nan [catch {sleep nan} e] 1
chk big [catch {sleep 1e30} e] 1
chk neg [catch {sleep -1} e] 1
