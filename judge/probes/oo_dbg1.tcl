proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t c11 {oo::object create foo; oo::objdefine foo {method bar args {global result; lappend result {*}$args; return [llength $args]}}; set r {}; lappend r [foo bar a b c]; lappend r [foo destroy] [info commands foo]; set r}
t c120 {oo::object create obj; rename [info object namespace obj]::my ::AGlobalName; obj destroy; info commands ::AGlobalName}
t c111 {oo::object create foo; set result [list [catch {oo::object create foo} msg] $msg]; lappend result [foo destroy] [oo::object create foo] [foo destroy]; set result}
t c112 {oo::class create bar; bar create foo; set result [list [catch {bar create foo} msg] $msg]; lappend result [bar destroy] [oo::object create foo] [foo destroy]; set result}
t c166 {oo::object create foo; set result [list [info object methods foo]]; oo::objdefine foo method bar {...}; lappend result [info object methods foo] [foo destroy]; set result}
t c182 {catch {oo::define oo::object error foo} m; set m}
t c183 {catch {oo::class create foo {error bar}} m; set m}
t c143 {oo::class create Aclass; oo::define Aclass method bar {} {lappend ::result "[self object] in bar"}; Aclass create ft; catch {ft ?} m; set m}
t c147 {info command infocmdtest*; info commands infocmdtest*}
t c152 {oo::object create foo; oo::objdefine foo {method m x {lappend ::result [self object] >$x<}; forward f ::lappend ::result fwd}; set result {}; foo m 1; foo f 2; lappend result [oo::copy foo bar]; foo m 3; foo f 4; bar m 5; bar f 6; lappend result [foo destroy]; bar m 7; bar f 8; lappend result [bar destroy]; set result}
t c151 {oo::class create Aclass; oo::define Aclass method test {} {lappend ::result [self object]->test}; Aclass create Ainstance; set result {}; Ainstance test; oo::copy Ainstance Binstance; Binstance test; Ainstance test; Ainstance destroy; namespace eval foo {oo::copy Binstance Cinstance; Cinstance test}; Aclass destroy; lappend result [info commands Binstance]; set result}
t c142 {oo::class create Aclass {method bar {} {return "x"}}; oo::class create Bclass {method boo {} {return y}}; oo::define Aclass mixin Bclass; Aclass create fooTest; set r [list [catch {fooTest ?} m] $m]; lappend r [catch {fooTest bar} m] $m; lappend r [catch {fooTest boo} m] $m; lappend r [Bclass destroy] [info commands Aclass]; set r}
