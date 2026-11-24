# crash-probe: d11 coroutine set shadow
# gate: skip-tree
# (The tree engine cannot suspend a yield inside an apply body at all —
# a standing engine difference; the defect here is the bytecode fold.)
source [file join [file dirname [info script]] _chk.tcl]
proc p {} { set x 1; return $x }
coroutine set apply {{} {yield 99}}
chk rc [catch {p} e] 1
