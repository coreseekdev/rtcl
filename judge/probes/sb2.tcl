proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t flags {subst -nov -nob -noc {abc $x [expr {1 + 2}] \\\x41}}
t ifret {subst {foo [if 1 { return {x}; bogus code }] bar}}
t evalret {subst {[eval {return hi}] there}}
t parseerr {set a {}""}
t brk12x {set x unset; set y unset; catch {subst {[set x 1;break;incr x][set y $x]}}}; list
t ifcont {subst {foo [if 1 { continue; bogus code}] bar}}
