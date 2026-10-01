proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t q1 {info object}
t q2 {info class}
t q3 {info object isa}
t q4 {info class x}
t q5 {info object namespace}
t q6 {info object namespace a b}
t q7 {info class superclasses}
t q8 {info obj oo::object}
t q9 {info object me oo::object}
t q10 {info object isa oo::object}
t q11 {info object isa class oo::object nosuch}
t q12 {info object methods oo::object -all -private}
t q13 {info class methods oo::object -private}
t q14 {info class subclasses oo::object}
t q15 {info class definition oo::class}
t q16 {info class call oo::object destroy}
t q17 {info object vars nosuchobj}

# constructor failure cleanup
oo::class create CF { constructor {x} {error "bad ctor: $x"} }
t c1 {catch {CF create cf1 zz} e; set e}
t c2 {info commands cf1}
t c3 {info class instances CF}

# destructor chain without next
oo::class create PA { destructor {lappend ::d2 PA} }
oo::class create PB { superclass PA; destructor {lappend ::d2 PB} }
oo::class create PC2 { superclass PB }
t d1 {set ::d2 {}}
t d2 {PC2 create pci; pci destroy}
t d3 {set ::d2}
# destructor without next but with explicit one missing
oo::class create PD { destructor {lappend ::d3 PD; next} }
t d4 {set ::d3 {}}
t d5 {PD create pdi; pdi destroy}
t d6 {set ::d3}

# class-level mixin instances
oo::class create MM { method mm {} {return mm} }

oo::class create HOST { mixin MM; method hh {} {return hh} }
t e1 {HOST create hi1; info class instances MM}
t e2 {info class mixins HOST}
t e3 {list [hi1 mm] [hi1 hh]}
t e4 {MM destroy; info commands hi1}

# variable visibility across chain
oo::class create VA2 { variable vx; constructor {} {set vx 1} }
oo::class create VB2 { superclass VA2; variable vy; constructor {} {next; set vy 2}; method getboth {} {list $vx $vy} }
t f1 {VB2 create vbi}
t f2 {vbi getboth}
t f3 {info class variables VB2}
oo::objdefine vbi { variable vz }
t f4 {catch {vbi getboth} e; set e}

# method param defaults and args collection
oo::class create MP { method m {a {b 5} args} {list $a $b $args} }
t g1 {MP create mpi}
t g2 {mpi m 1}
t g3 {mpi m 1 2}
t g4 {mpi m 1 2 3 4}
t g5 {catch {mpi m} e; set e}
t g6 {info args mpi m}
t g7 {info default mpi m b dv; set dv}
t g8 {info body mpi m}

# info level inside method
oo::class create IL { method m {x} {info level 0} }
t h1 {IL create ili; ili m zz}
oo::class create IL2 { method m {} {namespace current} }
t h2 {IL2 create il2i; il2i m}
t h3 {namespace eval [info object namespace il2i] {namespace current}}

# objdefine script error info
t i1 {catch {oo::objdefine nosuch {}} e; set e}
oo::object create oe1
t i2 {catch {oo::objdefine oe1 {error boomdef}} e; set e}
t i3 {set errorInfo}

# destroy during cascade with destructor on instance class
oo::class create KB { destructor {lappend ::klog "KB-inst"} }
oo::class create KC { superclass KB; destructor {lappend ::klog "KC-inst"} }
t j1 {set ::klog {}}
t j2 {KC create kci}
t j3 {KB destroy}
t j4 {set ::klog}

# create returns and namespace tails
t k1 {namespace eval deepns {oo::class create cls; info commands ::deepns::*}}
t k2 {oo::object create {weird name}}
t k3 {info commands ::weird*}
t k4 {catch {{weird name} destroy} e; set e}

# forward in define (class-level)
oo::class create FW2 { forward ff ::lappend ::flog; method callff {args} {my ff {*}$args} }
t l1 {FW2 create fwi}
t l2 {fwi callff 1 2}
t l3 {set ::flog}
t l4 {info class methods FW2}
