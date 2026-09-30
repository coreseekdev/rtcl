proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t s55 {set a 0; list [catch {subst {[set a 1}} msg] $a $msg}
t s61 {catch {unset a}; catch {subst {[concat foo] $a}}}
t s75 {subst -nocommands {abc $x [expr {1 + 2}] \\\x41}}
t s81 {subst {foo [return {x}; bogus code] bar}}
t s101 {subst {foo [break; bogus code] bar}}
t s111 {subst {foo [continue; bogus code] bar}}
t s123 {set x 0; catch {subst "\[incr x;"} m; list $x $m}
