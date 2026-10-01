proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
oo::class create MM { method mm {} {return mm} }
oo::class create HOST { mixin MM; method hh {} {return hh} }
t e1 {HOST create hi1; info class instances MM}
t e2 {info class mixins HOST}
t e3 {list [hi1 mm] [hi1 hh]}
t e4 {catch {hi1 nosuch} e; set e}
t e5 {MM destroy; info commands hi1}
t e6 {info commands HOST}
t e7 {catch {hi1 mm} e; set e}

# multiple inheritance resolution order
oo::class create MA3 { method who {} {return MA3}; method onlyA {} {return a} }
oo::class create MB3 { method who {} {return MB3} }
oo::class create MC3 { superclass MA3 MB3 }
t m1 {MC3 create mci3; mci3 who}
t m2 {mci3 onlyA}
oo::class create MD3 { superclass MC3 MA3 }
t m3 {catch {MD3 create mdi3} e; set e}
t m4 {info class superclasses MD3}

# diamond with next
oo::class create DA2 { method m {} {return DA2} }
oo::class create DB2 { superclass DA2; method m {} {return "DB2<[next]>"} }
oo::class create DC4 { superclass DA2; method m {} {return "DC4<[next]>"} }
oo::class create DD2 { superclass DB2 DC4; method m {} {return "DD2<[next]>"} }
t n1 {DD2 create ddi; ddi m}

# unexported inherited method callable via my
oo::class create UA { method Secret {} {return s} }
oo::class create UB { superclass UA; method callit {} {my Secret} }
t o1 {UB create ubi; ubi callit}
t o2 {catch {ubi Secret} e; set e}

# export state change visibility
oo::class create ES { method m1 {} {return 1} }
ES create esi
t p1 {oo::define ES unexport m1; catch {esi m1} e; set e}
t p2 {oo::define ES export m1; esi m1}

# mixin method conflicts with own class method
