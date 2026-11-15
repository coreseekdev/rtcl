# oo22: OO 可见性面收口（2026-11-15）：对象 ns 只露 my、::oo::Helpers
# 重映射、::oo 内部命令直调（InfoClass/InfoObject/UnknownDefinition/
# define 词的元数与语境错误）、deletemethod/renamemethod 的提及名单、
# filter 对未导出名的拦截、nextto、history 仅 stdin 注册。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# 对象命名空间可见性（oo4 p4/p9/p12/p13）
oo::class create D { method dm {} {return dm} }
t p4 {info commands [info object namespace D]::*}
oo::object create obj1
t p9 {info commands [info object namespace obj1]::*}
t p12 {namespace eval [info object namespace obj1] {namespace which self}}
t p13 {namespace eval [info object namespace obj1] {namespace which next}}
t p12b {namespace eval [info object namespace obj1] {namespace which my}}
t p13b {namespace which ::[info object namespace obj1]::self}

# ::oo 命令面与子命名空间（oo1 a5 / oo4 x2/x3 的静态部分）
t a5 {lsort [info commands ::oo::*]}
t a5c {llength [info commands ::oo::*]}
t k1 {llength [namespace children ::oo]}
t k2 {lsort [namespace children ::oo]}
t k3 {lsort [info commands ::oo::Helpers::*]}
t k4 {lsort [info commands ::oo::define::*]}
t k5 {lsort [info commands ::oo::objdefine::*]}
t k6 {llength [info commands ::oo::InfoClass::*]}
t k7 {llength [info commands ::oo::InfoObject::*]}

# 元类实例表（oo1 a13）
t a13 {info class instances oo::class}

# 内部命令直调：InfoClass/InfoObject/UnknownDefinition
t di1 {::oo::InfoClass methods oo::object}
t di2 {catch {::oo::InfoClass} e; set e}
t di3 {catch {::oo::InfoObject} e; set e}
t di4 {catch {::oo::InfoObject isa class} e; set e}
t di5 {catch {::oo::UnknownDefinition} e; set e}
t di6 {catch {::oo::UnknownDefinition foo bar} e; set e}
t di7 {::oo::InfoClass::methods oo::object -all}
t di8 {catch {::oo::InfoClass::forward oo::object} e; set e}
t di9 {catch {::oo::InfoClass::nosuch} e; set e}

# define 词直调：元数先、语境后（tclsh d1/d5/d6/d7）
t dw1 {catch {::oo::define::method} e; set e}
t dw2 {catch {::oo::define::constructor x} e; set e}
t dw3 {catch {::oo::define::export oo::object zz} e; set e}
t dw4 {catch {::oo::objdefine::method} e; set e}
t dw5 {catch {::oo::define::variable} e; set e}
t dw6 {catch {::oo::define::filter oo::object} e; set e}

# deletemethod/renamemethod：自有限定 + 提及名单（oo7 d15/u2/n2..n4）
oo::class create RC { method m {} {return m} }
t rm1 {catch {oo::define RC deletemethod m} e; set e}
t rm2 {catch {oo::define RC deletemethod nosuch} e; set e}
t rm3 {catch {oo::define RC renamemethod nosuch nn} e; set e}
t rm4 {catch {oo::define RC unexport nosuch} e; set e}
t rm5 {catch {oo::define RC deletemethod nosuch} e; set e}
t rm6 {catch {oo::define RC renamemethod nosuch nn} e; set e}
oo::class create RD { superclass RC }
t rm7 {catch {oo::define RD deletemethod m} e; set e}

# filter 拦截未导出名（g2/g5/g6/g7）
oo::class create FB { method +f {args} {return G}; filter +f; method bar {} {return b} }
FB create fb
t f1 {fb bar}
t f2 {fb +f}
t f3 {catch {fb totally_unknown} e; set e}
oo::class create UF { method +h {args} {return H}; filter +h; method pub {} {return u} }
oo::define UF { unexport +h }
UF create uf
t f4 {uf pub}
t f5 {catch {uf +h} e; set e}
t f6 {catch {uf zzz} e; set e}

# nextto（nx1/nx3/nt1）
oo::class create NA { method m {args} {return "NA:[join $args ,]"} }
oo::class create NB { superclass NA; method m {args} {return "NB+[nextto NA]"} }
NB create nbi
t nx1 {nbi m x y}
oo::class create NC { superclass NB; method m {args} {return "NC+[nextto NB]"} }
NC create nci
t nx3 {nci m z}
oo::class create N2 { superclass NB; method m {args} {return "N2+[nextto]"} }
N2 create n2
t nt1 {catch {n2 m} e; set e}

# history 仅 stdin 注册（oo4 p8；-f 文件模式无 history）
t h1 {info commands history}
