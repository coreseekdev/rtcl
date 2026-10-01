proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
oo::class create C
# multi-word method with {} params and spaced body
t a1 {oo::define C method m1 {} {set x 1; return $x}}
t a2 {C create i1; i1 m1}
# multi-word method with multi-word params
t a3 {oo::define C method m2 {p q} {return "$p-$q"}}
t a4 {i1 m2 A B}
# unknown def word with args -> evaluated?
t b1 {oo::define C set zz 5}
t b2 {info exists ::oo::define::zz}
t b3 {oo::define C error foo}
t b4 {oo::define C concat}
t b5 {oo::define C}
t b6 {oo::define}
# multi-word with a body containing newlines/semicolons
t c1 {oo::define C method m3 w {set y [string toupper $w]; return $y}}
t c2 {i1 m3 ab}
# multi-word single extra arg == script form?
t d1 {oo::define C {method m4 {} {return m4}}}
t d2 {i1 m4}
# does error in multi-word form carry def-script frame?
t e1 {catch {oo::define C error foo} m; set errorInfo}
# variable decl via multi-word then method reads it
t f1 {oo::define C variable counter}
t f2 {oo::define C method m5 {} {incr counter; return $counter}}
t f3 {oo::define C constructor {} {set counter 100}}
t f4 {C create i2; i2 m5}
# objdefine multi-word eval
oo::object create oz
t g1 {oo::objdefine oz set ozv 7}
t g2 {oo::objdefine oz method getv {} {return $ozv}}
t g3 {oz getv}
t g4 {oo::objdefine oz error boom}
