# crash-probe: b14 nested cmdsub compile
source [file join [file dirname [info script]] _chk.tcl]
set n 20000
set body "set r " + "[list " * n + "1" + "]" * n
set r [catch {proc p {} $body} e]
chk rc $r 1
