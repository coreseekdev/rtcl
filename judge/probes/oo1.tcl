proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t a1 {oo::object create o1}
t a2 {info object namespace o1}
t a3 {info object namespace ::o1}
t a4 {namespace children ::}
t a5 {info commands ::oo::*}
t a6 {info commands ::o1}
t a7 {namespace which ::o1}
t a8 {info object methods oo::object}
t a9 {info object methods oo::object -all}
t a10 {info class methods oo::object}
t a11 {info class methods oo::object -all}
t a12 {info class superclasses oo::object}
t a13 {info class instances oo::class}
t a14 {info object class o1}
t a15 {info object isa class oo::class}
t a16 {oo::object create ::one::two::three}
t a17 {info object namespace ::one::two::three}
t a18 {namespace children ::one}
t a19 {info commands ::one::*}
t a20 {rename [info object namespace o1]::my renamedMy; info commands renamedMy}
t a21 {o1 destroy; info commands o1}
t a22 {info commands renamedMy}
t a23 {namespace which renamedMy}

# method definition basics
t b1 {oo::class create C1}
t b2 {oo::define C1 { method m {a b} { return "$a-$b" } }}
t b3 {C1 create inst1}
t b4 {inst1 m x y}
t b5 {info object methods inst1}
t b6 {info class methods C1}
t b7 {catch {inst1 m x} e; set e}
t b8 {catch {inst1 m x y z} e; set e}
t b9 {info object class inst1}
t b10 {info class instances C1}
t b11 {oo::define C1 export m}
t b12 {oo::define C1 unexport m}
t b13 {catch {inst1 destroy} e; set e}
t b14 {info commands inst1}

# self / my / namespace current in methods
oo::class create C2 { method m {} {
    list [self object] [self class] [namespace current] [self namespace]
} }
t c1 {C2 create i2}
t c2 {i2 m}
t c3 {catch {self} e; set e}
proc helperAtGlobal {} { catch {self object} e; return $e }
oo::class create C3 { method m {} { helperAtGlobal } }
t c4 {C3 create i3; i3 m}
t c5 {catch {my nosuch} e; set e}
oo::class create C4 {
    method pub {} { return "pub" }
    method priv {} { return "priv" }
    method callboth {} { list [my pub] [my priv] }
    export pub
}
t c6 {C4 create i4; i4 callboth}
t c7 {catch {i4 priv} e; set e}
t c8 {i4 pub}
t c9 {catch {my pub} e; set e}

# unknown method message
t d1 {catch {i4 nosuch} e; set e}
t d2 {catch {i2 m extra} e; set e}

# method visibility / export lists
t e1 {info class methods C4}
t e2 {lsort [info class methods C4 -all]}
t e3 {info object methods i4}
t e4 {lsort [info object methods i4 -all]}

# inheritance
oo::class create B0 { method bm {} {return bm0} }
oo::class create D0 { superclass B0; method dm {} {return dm0} }
t f1 {D0 create d0i}
t f2 {d0i bm}
t f3 {d0i dm}
t f4 {info class superclasses D0}
t f5 {lsort [info class superclasses D0 -all]}
t f6 {info class instances B0}
t f7 {info object isa class D0}
t f8 {info object isa class d0i}

# destructor order
oo::class create DA { destructor {lappend ::dlog "DA"} }
oo::class create DB { superclass DA; destructor {lappend ::dlog "DB"; next} }
t g1 {set ::dlog {}}
t g2 {DB create dbi}
t g3 {dbi destroy}
t g4 {set ::dlog}

# constructor
oo::class create DC { variable v; constructor {x} {set v $x} method get {} {return $v} }
t h1 {DC create dci hello}
t h2 {dci get}
t h3 {catch {DC create} e; set e}
t h4 {info class variables DC}
