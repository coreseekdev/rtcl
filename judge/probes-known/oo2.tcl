proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# constructor
oo::class create DC {
    variable v
    constructor {x} {set v $x}
    method get {} {return $v}
}
t h1 {DC create dci hello}
t h2 {dci get}
t h3 {catch {DC create} e; set e}
t h4 {info class variables DC}
t h5 {info class variables D0}

# object new naming
t i1 {set o [oo::object new]}
t i2 {info object namespace $o}
t i3 {catch {oo::object create $o} e; set e}

# unknown-method list rule with per-object methods
oo::object create po
oo::objdefine po { method hidden {} {return h} }
t j1 {catch {po nosuch} e; set e}
oo::objdefine po export hidden
t j2 {catch {po nosuch} e; set e}
t j3 {po hidden}
oo::objdefine po unexport hidden
t j4 {catch {po nosuch} e; set e}

# class chain unexported
oo::class create PC { method hidden {} {return h} }
PC create pci
t k1 {catch {pci nosuch} e; set e}

# forward
oo::object create fw
oo::objdefine fw { forward f ::lappend log }
t l1 {fw f a b}
t l2 {set log}
t l3 {info object methods fw}
t l4 {catch {fw f} e; set e}
t l5 {catch {oo::objdefine fw forward} e; set e}
t l6 {catch {oo::objdefine fw forward g nosuchcmd} e; set e}
t l7 {fw g}
t l8 {set log}

# oo::copy
oo::class create CC { variable n; constructor {x} {set n $x}; method getn {} {return $n} }
CC create c1 seven
t m1 {oo::copy c1 c2}
t m2 {c2 getn}
t m3 {catch {oo::copy c1 c2} e; set e}
t m4 {catch {oo::copy nosuch tgt} e; set e}
t m5 {oo::copy c1}
t m6 {info object class c3}
t m7 {namespace eval nn {oo::copy c1 deep}; info commands ::nn::*}
t m8 {::nn::deep getn}
t m9 {catch {oo::copy c1 ::abs::x} e; set e}
t m10 {info commands ::abs::*}

# <cloned> method on copy
oo::class create CL { method <cloned> {args} {lappend ::clog CLONED-[llength $args]} }
CL create cl1
t n1 {oo::copy cl1 cl2}
t n2 {set ::clog}

# mixins
oo::class create MA { method am {} {return am} }
oo::class create MB { method bm {} {return bm} }
oo::class create MC { mixin MA MB; method cm {} {return cm} }
t o1 {MC create mci}
t o2 {list [mci am] [mci bm] [mci cm]}
t o3 {info class mixins MC}
t o4 {info object mixins mci}
t o5 {catch {mci nosuch} e; set e}
oo::class create MD { method bm {} {return bm-of-D} }
oo::objdefine mci mixin MD
t o6 {list [mci am] [mci bm]}
t o7 {info object mixins mci}
oo::objdefine mci mixin
t o8 {list [mci am] [mci bm]}
t o9 {catch {oo::objdefine mci mixin NoSuch} e; set e}
t o10 {catch {oo::define MC mixin NoSuch} e; set e}

# next
oo::class create NA { method m {} {return "NA"} }
oo::class create NB { superclass NA; method m {} {return "NB+[next]"} }
t p1 {NB create nbi; nbi m}
t p2 {catch {NA create nai; nai m} e; set e}
oo::class create NC { superclass NB; method m {} {return "NC+[next a1 a2]"} }
oo::class create ND { superclass NC; method m {} {return ND} }
t p3 {ND create ndi; ndi m}

# filters
oo::class create FA {
    method foo {} {return foo}
    filter filt
    method filt {args} {lappend ::flog [self call]; return "F:$args"}
}
t q1 {FA create fai}
t q2 {catch {fai foo} e; set e}
t q3 {info class filters FA}

# variable declarations in methods
oo::class create VA {
    variable x y
    constructor {} {set x 1; set y 2}
    method getx {} {return $x}
    method setx {v} {set x $v; return [my getx]}
}
t r1 {VA create vai}
t r2 {vai getx}
t r3 {vai setx 9}
t r4 {info vars}
t r5 {catch {oo::define VA variable a:b} e; set e}
t r6 {catch {oo::define VA variable a(1)} e; set e}
t r7 {catch {oo::define VA variable {}} e; set e}
