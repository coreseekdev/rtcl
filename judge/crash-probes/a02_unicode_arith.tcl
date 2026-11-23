# crash-probe: a02 unicode arith
source [file join [file dirname [info script]] _chk.tcl]
chk add [catch {expr {"中a" + 1}} e] 1
chk mod [catch {expr {2 % "aé"}} e] 1
chk max [catch {expr {max("中", "a")}} e] 1
