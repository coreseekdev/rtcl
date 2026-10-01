proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
proc foo {} {return F}
t a {apply {{} {catch [list tailcall foo] m; list $m [catch [list tailcall foo] m2]}}}
t b {apply {{} {catch [list tailcall]; list [catch [list tailcall] m] $m}}}
t c {apply {{} {catch [list tailcall foo] m; return got:$m}}}
t d {apply {{} {set c [catch [list tailcall foo] m o]; list $c $m $o}}}
t e {tailcall}
t f {apply {{} {tailcall}}}
t g {catch {tailcall foo} m; list [catch {tailcall foo} m2] $m2}
