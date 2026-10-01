proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# oo-4.1 verbatim
set result {}
set o [oo::object new]
oo::objdefine $o method Foo {} {lappend ::result Foo; return}
t v1 {lappend result [catch {$o Foo} msg] $msg}
t v2 {info object methods $o}

# per-object method listed after export?
oo::object create po2
oo::objdefine po2 method hide2 {} {}
t v3 {catch {po2 nosuch} e2; set e2}
t v4 {catch {po2 hide2} e2; set e2}

# class-method unexported listing
oo::class create UQ { method uh {} {return 1} }
UQ create uqi
t v5 {catch {uqi nosuch} e2; set e2}

# mixin'd object as instance?
oo::class create MA2 { method am {} {return am} }
oo::class create A2 { method bar {} {return bar} }
oo::objdefine [A2 create at1] mixin MA2
t w1 {info class instances MA2}
t w2 {info class instances A2}
t w3 {info object isa class at1}
t w4 {info object mixins at1}
t w5 {info object class at1}
# destroy mixin class -> what happens to at1?
t w6 {MA2 destroy; info commands at1}

# oo-14.1 verbatim reconstruction
oo::class create Ac {
}
oo::define Ac method bar {} {lappend ::r "[self object] in bar"}
oo::class create Bc {
}
oo::define Bc method boo {} {lappend ::r "[self object] in boo"}
oo::objdefine [Ac create ft1] mixin Bc
oo::objdefine [Ac create ft2] mixin Bc
set r {}
catch {ft1 ?} em
t x1 {set em}
t x2 {info class instances Bc}
t x3 {info class instances Ac}
t x4 {Bc destroy; info commands ft*}
t x5 {info commands ft*}

# class destroy cascade
oo::class create CB {}
oo::class create CD { superclass CB }
oo::class create CE { superclass CD }
CD create ci1
CB create ci2
t y1 {CB destroy; list [info commands ci1] [info commands ci2] [info commands CD] [info commands CE]}
t y2 {info commands CB}

# next at end of chain
oo::class create NA2 { method m {} {return "NA2:[next]"} }
t z1 {NA2 create na; catch {na m} e3; set e3}
t z2 {catch {next} e3; set e3}

# self subcommands
oo::class create SA { method m {} { list [catch {self} e] $e [catch {self foo} e2] $e2 [catch {self object extra} e3] $e3 [self object] [self class] [self namespace] } }
t s1 {SA create sai; sai m}
t s2 {catch {self} e; set e}
t s3 {catch {my} e; set e}

# errorInfo in method body
oo::class create EA { method m {} {error boom} }
t ei1 {EA create eai; catch {eai m}; set errorInfo}

# wrong-args through my
oo::class create WA { method m {a b} {return x} }
t wa1 {WA create wai; catch {wai m 1} e; set e}
t wa2 {catch {wai m 1 2 3} e; set e}
t wa3 {catch {my m 1} e; set e}

# define with separate args vs script
t df1 {catch {oo::define SA error boom} e; set e}
t df2 {catch {oo::define SA {error boom}} e; set e}
t df3 {set errorInfo}
t df4 {catch {oo::define SA error boom; error boom2} e; set e}
t df5 {set errorInfo}
t df6 {catch {oo::define SA nonexistentword} e; set e}
t df7 {catch {oo::define nosuchclass {}} e; set e}
t df8 {catch {oo::define} e; set e}
t df9 {catch {oo::objdefine nosuchobj {}} e; set e}
t df10 {catch {oo::define SA method} e; set e}
t df11 {catch {oo::define SA method nm} e; set e}
t df12 {catch {oo::define SA method nm {a b}} e; set e}

# class create with definition script errors
t cr1 {catch {oo::class create badcls {error bar}} e; set e}
t cr2 {set errorInfo}
t cr3 {catch {oo::class create badcls} e; set e}

# create on non-class
t cn1 {catch {sai create zz} e; set e}
t cn2 {catch {oo::object create} e; set e}
t cn3 {catch {oo::object destroy zz} e; set e}
t cn4 {catch {oo::class destroy zz} e; set e}
t cn5 {catch {oo::objdefine} e; set e}
t cn6 {catch {oo::define SA} e; set e}
t cn7 {oo::object create newobj; oo::objdefine newobj destroy; info commands newobj}
