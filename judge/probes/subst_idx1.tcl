proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {proc tstProc {} { # comment
 # this is a bogus comment
 # this is a bogus comment
 # this is a bogus comment
 }
 set msg {}
 list [catch tstProc msg] $msg}
t b {set x 1a; list [catch {incr x 1a} msg] $msg $::errorInfo}
t c {catch {return -code error -errorcode {{}a} eek} m; set m}
t d {scan [lsort -ascii -nocase [list a\x00a a]] %c%c%c%c%c}
t e {scan a\x00a %c%c%c}
