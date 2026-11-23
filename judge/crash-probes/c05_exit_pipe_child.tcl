# crash-probe: c05 exit pipe child
# gate: max-ms 1500
source [file join [file dirname [info script]] _chk.tcl]
# gate: skip-tree
set f [open "|sleep 3"]
exit 0
