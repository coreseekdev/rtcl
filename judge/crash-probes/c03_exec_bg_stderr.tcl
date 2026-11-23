# crash-probe: c03 exec bg stderr
# gate: max-ms 4000
source [file join [file dirname [info script]] _chk.tcl]
# gate: skip-tree
set p [exec sh -c {head -c 300000 /dev/zero | tr '\0' E >&2} &]
after 1500
chk gone [catch {exec kill -0 $p}] 1
