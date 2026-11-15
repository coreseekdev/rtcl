# oo21: methodtype/forward/constructor/destructor 的自有作用域与形态
# （2026-11-15 OO 可见性组收口：own-only + core method 形 + 元数错误）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
oo::class create A { method m {x} {return A:$x}; forward fw ::set k; method _p {} {return p} }
oo::class create B { superclass A; method m {x y} {return B:$x$y} }
B create b1
t mt1 {info class methodtype A fw}
t mt2 {info class methodtype A _p}
t mt3 {catch {info class methodtype B fw} e; set e}
t mt4 {catch {info class methodtype B m} e; set e}
t mt5 {catch {info class methodtype B nosuch} e; set e}
t mt6 {catch {info class methodtype -private A _p} e; set e}
t mt7 {info class methodtype oo::object destroy}
t mt8 {info class methodtype oo::class create}
t mt9 {catch {info class methodtype oo::object create} e; set e}
t mt10 {catch {info object methodtype b1 m} e; set e}
t mt11 {catch {info object methodtype b1 destroy} e; set e}
t mt12 {catch {info object methodtype b1 nosuch} e; set e}
oo::object create ob
t mt13 {catch {info object methodtype ob destroy} e; set e}
t fw1 {catch {info class forward A} e; set e}
t fw2 {info class forward A fw}
t fw3 {catch {info class forward A nosuch} e; set e}
t fw4 {catch {info class forward B fw} e; set e}
oo::objdefine ob forward gf ::set q
t fw5 {info object forward ob gf}
t fw6 {catch {info object forward ob nosuch} e; set e}
oo::class create C { variable v; constructor {a {b 2}} {set v $a}; destructor {my z}; method z {} {} }
t ct1 {info class constructor C}
t ct2 {info class destructor C}
oo::class create D {}
t ct3 {info class constructor D}
t ct4 {info class destructor D}
t ct5 {info class constructor oo::object}
# oo::Slot 面：自有方法、methodtype、实例表
t sl1 {info class methods oo::Slot}
t sl2 {lsort [info class methods oo::Slot -all]}
t sl3 {catch {::oo::Slot -clear} e; set e}
t sl4 {catch {::oo::Slot nosuch} e; set e}
t sl5 {lsort [info class instances oo::Slot]}
t sl6 {info object class oo::Slot}
t sl7 {info object class ::oo::define::variable}
t sl8 {info class methodtype oo::Slot -clear}
oo::class create E {}
t sl9 {info class methods E -all}
oo::class create Y2 { superclass oo::Slot; method yy {} {} }
t sl10 {lsort [info class methods Y2 -all]}
