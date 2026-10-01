proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {winfo name $zz}
t b {info commands winfo}
t c {set x 1; list [catch {incr x 1a} msg] $msg $::errorInfo}
t d {lsort -ascii [list a\x00a a]}
t e {lsort -ascii -nocase [list a\x00a a]}
t f {string compare a\x00a a}
t g {lsort -ascii [list bb b]}
t h {lsort -nocase [list BB b]}
t i {scan [lsort -ascii [list a\x00a a]] %c%c%c}
