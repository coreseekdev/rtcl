proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t d1 {oo::class create Aclass {method bar {} {return "[self object] in bar"}}; oo::class create Bclass {method boo {} {return "[self object] in boo"}}; oo::define Aclass mixin Bclass; Aclass create fooTest; set r [list [catch {fooTest ?} m] $m]; lappend r [catch {fooTest bar} m] $m; lappend r [catch {fooTest boo} m] $m; set r}
t d2 {lappend r [Bclass destroy] [info commands Aclass]; set r}
t e1 {oo::class create Aclass; oo::define Aclass method test {} {lappend ::result [self object]->test}; Aclass create Ainstance; set result {}; Ainstance test; oo::copy Ainstance Binstance; Binstance test; Ainstance test; Ainstance destroy; set result}
t e2 {namespace eval foo {oo::copy Binstance Cinstance}; Cinstance test; set result}
t e3 {Aclass destroy; namespace delete foo; lappend result [info commands Binstance]; set result}
