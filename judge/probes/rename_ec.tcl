proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {rename r1}
t b {rename r1 r2 r3}
t c {rename r1 r2; set ::errorCode}
t d {apply {{} {catch [list tailcall foo]; tailcall}}}
t e {apply {{} {catch [list tailcall foo] m; set m}}}
