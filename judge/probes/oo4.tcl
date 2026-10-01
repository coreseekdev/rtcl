proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

oo::class create D { method dm {} {return dm} }
t p1 {info object namespace D}
t p2 {namespace eval [info object namespace D] {namespace which B}}
oo::class create B { method bm {} {return bm} }
t p3 {namespace eval [info object namespace D] {namespace which B}}
t p4 {info commands [info object namespace D]::*}
t p5 {info commands ::oo::define::*}
t p6 {info commands ::oo::object::*}
t p7 {info commands ::oo::class::*}
t p8 {namespace eval [info object namespace D] {info commands ::*}}

# object namespace contents
oo::object create obj1
t p9 {info commands [info object namespace obj1]::*}
t p10 {namespace eval [info object namespace obj1] {info commands my}}
t p11 {namespace eval [info object namespace obj1] {namespace which my}}
t p12 {namespace eval [info object namespace obj1] {namespace which self}}
t p13 {namespace eval [info object namespace obj1] {namespace which next}}

# create in namespace
t q1 {namespace eval nn {oo::object create o}; info commands ::nn::*}
t q2 {oo::object create ::nosuchns::x}
t q3 {oo::class create ::alson::sub::cls; info object namespace ::alson::sub::cls}

# self in define script
oo::class create SD { }
t r1 {oo::define SD { set who [self object]; set what [self class] }}
t r2 {info class variables SD}
oo::define SD { variable who }
t r3 {info class variables SD}
t r4 {catch {oo::define SD {self foo}} e; set e}

# oo::copy <cloned> arg
oo::class create CL2 { method <cloned> {args} {return "CLONED:[llength $args]:$args"} }
CL2 create clx source-name
t s1 {oo::copy clx cly}
t s2 {catch {oo::copy clx nosuch:ns:tgt} e; set e}

# unknown method list with inherited + mixin + object methods
oo::class create B1 { method pub1 {} {return 1}; method Priv1 {} {return 2} }
oo::class create D1 { superclass B1; method pub2 {} {return 3} }
D1 create d1i
t u1 {catch {d1i nosuch} e; set e}
oo::class create M1 { method mx {} {return 4}; method Mx2 {} {return 5} }
oo::objdefine d1i mixin M1
t u2 {catch {d1i nosuch} e; set e}
oo::objdefine d1i { method objm {} {return 6}; method Objm2 {} {return 7} }
t u3 {catch {d1i nosuch} e; set e}
t u4 {lsort [info object methods d1i -all]}

# export/unexport on class, method name starting uppercase/digit
oo::class create EX { method 5five {} {return d}; method _under {} {return u} }
EX create exi
t v1 {catch {exi 5five} e; set e}
t v2 {catch {exi _under} e; set e}
t v3 {lsort [info class methods EX]}
t v4 {lsort [info class methods EX -all]}
oo::define EX export 5five _under
t v5 {list [exi 5five] [exi _under]}

# destroy semantics
oo::object create dd1
t w1 {dd1 destroy}
t w2 {catch {dd1 destroy} e; set e}
t w3 {info commands dd1}
t w4 {catch {oo::object create dd1} e; set e}
t w5 {oo::object create dd1}

# oo::object new with destroy
set n1 [oo::object new]
t w6 {catch {$n1 destroy} e; set e}
t w7 {info commands $n1}

# namespace children after object creation/deletion
t x1 {namespace children ::}
t x2 {namespace children ::oo}
t x3 {info commands ::oo::*}
