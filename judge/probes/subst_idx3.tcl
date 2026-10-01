proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {proc p {} {global a; set tst $a([winfo name $zz])}; list [catch p m] $m}
t b {proc p2 {} {global a; set tst $a([undefined_cmd x])}; list [catch p2 m] $m}
t c {proc p3 {} {global a; set tst $a(nosuch)}; list [catch p3 m] $m}
t d {lsort -nocase [list a\x00a a]}
t e {string compare -nocase a\x00a a}
t f {lsort -ascii -nocase [list bb\x00b b]}
t g {lsort -dictionary [list a\x00a a]}
