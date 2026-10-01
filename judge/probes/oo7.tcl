proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a1 {oo::class base}
t a2 {oo::object base}
t a3 {catch {oo::object new} e; set e}
oo::class create DC { method m {} {return 1} }
t a4 {catch {DC base} e; set e}
t a5 {catch {DC new} e; set e}
t a6 {info commands [namespace eval :: [oo::class new]]}
t a7 {catch {oo::class create X arg1 arg2} e; set e}

# destructor error leaves object?
oo::class create PD2 { destructor {lappend ::d3 PD2; next} }
t b1 {catch {PD2 create pdi2} e; set e}
t b2 {catch {pdi2 destroy} e; set e}
t b3 {info commands pdi2}

# define on non-class, objdefine on class
oo::object create plainobj
t c1 {catch {oo::define plainobj {}} e; set e}
t c2 {catch {oo::objdefine DC {}} e; set e}
t c3 {oo::objdefine DC {}; info object class DC}

# definition command arity in script form
t d1 {catch {oo::define DC {method}} e; set e}
t d2 {catch {oo::define DC {method nm}} e; set e}
t d3 {catch {oo::define DC {method nm {a b}}} e; set e}
t d4 {catch {oo::define DC {method nm {a b} {} {extra}}} e; set e}
t d5 {catch {oo::define DC method} e; set e}
t d6 {catch {oo::define DC forward} e; set e}
t d7 {catch {oo::define DC forward nm} e; set e}
t d8 {catch {oo::define DC constructor {a}} e; set e}
t d9 {catch {oo::define DC destructor {a} {b}} e; set e}
t d10 {catch {oo::define DC superclass} e; set e}
t d11 {catch {oo::define DC variable} e; set e}
t d12 {catch {oo::define DC export} e; set e}
t d13 {catch {oo::define DC export nosuchmeth} e; set e}
t d14 {catch {oo::define DC unexport nosuchmeth} e; set e}
t d15 {catch {oo::define DC deletemethod nosuchmeth} e; set e}
t d16 {catch {oo::define DC method nm {a b} {} {extra}} e; set e}

# superclass/mixin validation
oo::object create notaclass
t e1 {catch {oo::define DC superclass notaclass} e; set e}
t e2 {catch {oo::define DC mixin notaclass} e; set e}
t e3 {catch {oo::define DC superclass DC} e; set e}
t e4 {catch {oo::define DC mixin DC} e; set e}
t e5 {oo::define DC mixin nosuch}
t e6 {catch {oo::define DC mixin plainobj nosuch} e; set e}
t e7 {info class mixins DC}

# variable name check order
t f1 {catch {oo::define DC variable a::b(1)} e; set e}
t f2 {catch {oo::define DC variable a(1)::b} e; set e}
t f3 {catch {oo::define DC variable good bad::var} e; set e}
t f4 {info class variables DC}

# self in define, no args
oo::class create SD2 { }
t g1 {oo::define SD2 {set x [self]; set x}}
t g2 {catch {oo::define SD2 {self}} e; set e}

# objdefine variable / export etc
oo::object create ov1
oo::objdefine ov1 { variable ov }
t h1 {oo::objdefine ov1 {method getov {} {return $ov}}}
t h2 {catch {ov1 getov} e; set e}
oo::objdefine ov1 {method setov {v} {set ov $v}}
t h3 {ov1 setov 42}
t h4 {ov1 getov}

# method redefinition export state
oo::class create RD { method m1 {} {return 1} }
oo::define RD unexport m1
oo::define RD method m1 {} {return 2}
t i1 {catch {RD create rdi; rdi m1} e; set e}

# chain resolution: per-object method shadows class method
oo::class create SH { method m {} {return class} }
SH create shi
oo::objdefine shi { method m {} {return object} }
t j1 {shi m}

# mixin vs superclass precedence
oo::class create SM { method who {} {return mixin} }
oo::class create SC { method who {} {return class} }
oo::class create SU { superclass SC; mixin SM }
t k1 {SU create sui; sui who}
