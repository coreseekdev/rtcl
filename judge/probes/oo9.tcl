proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
oo::class create K { method m {} {return 1} }
K create ki
t a1 {ki}
t a2 {oo::object}
t a3 {oo::class}
t a4 {K}
t a5 {set k2 {}; catch {$k2 m} e; set e}
t b1 {K create ki m extra}
t b2 {oo::object create oo::object}
t b3 {info object isa class oo::object}
t b4 {info object isa object oo::object}
t b5 {info object isa metaclass oo::class}
t b6 {info object isa metaclass oo::object}
t b7 {info object isa typeof ki K}
t b8 {info object isa mixin ki K}
t b9 {info object isa bogus ki}
t b10 {info object isa}
t b11 {info class methods nosuchcls}
t b12 {info class instances nosuchcls}
t b13 {info object class nosuchobj}
t b14 {info object mixins nosuchobj}
t c1 {oo::class create E1 {error boom}; info commands E1}
t d1 {catch {K create nosuch:ns:tgt} e; set e}
t d2 {K create ::nn::sub::obj; info object namespace ::nn::sub::obj}
oo::object create oz
t e1 {catch {oo::objdefine oz {mixin nosuch}} e; set e}
t e2 {catch {oo::objdefine oz {superclass K}} e; set e}
oo::class create W1 { method m {} {return W1m} }
oo::objdefine W1 { method ObjM {} {return om} }
t f1 {catch {W1 new ObjM} e; set e}
t f2 {set o [W1 new]; $o ObjM}
t f3 {info object class $o}
t g1 {oo::object create sub; oo::objdefine sub {method m2 {a} {return x-$a}}; sub m2 yy}
t h1 {info object methods oo::object}
t h2 {info object isa class plainstring}
t h3 {info object namespace plainstring}
