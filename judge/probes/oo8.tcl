proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# constructor chain without next
oo::class create CA { constructor {} {lappend ::clog CA} }
oo::class create CB2 { superclass CA; constructor {} {lappend ::clog CB2} }
t a1 {set ::clog {}}
t a2 {CB2 create cbi}
t a3 {set ::clog}
t a4 {catch {CA new} e; set e}

# <cloned> argument value
oo::class create CL3 { method <cloned> {args} {return "ignored"} }
CL3 create cl3
set ::clarg {}
oo::class create CL4 { method <cloned> {args} {set ::clarg $args; return "ignored"} }
CL4 create cl4src
t b1 {oo::copy cl4src cl4dst}
t b2 {set ::clarg}

# copy of class object, copy onto proc name
proc someproc {} {}
t c1 {catch {oo::copy cl3 someproc} e; set e}
t c2 {catch {oo::copy cl3 cl3} e; set e}
t c3 {catch {oo::copy DC nosuchobj} e; set e}

# self method/call/next subcommands
oo::class create SL { method m {} {list [catch {self method} e1] $e1 [catch {self call} e2] $e2 [catch {self next} e3] $e3 [catch {self target} e4] $e4 [catch {self caller} e5] $e5 [catch {self filter} e6] $e6} }
t d1 {SL create sli; sli m}
# self inside constructor
oo::class create SC2 { constructor {} {lappend ::sclog [self object]} }
t d2 {set ::sclog {}; SC2 create sci; set ::sclog}

# next with args changes params
oo::class create NX1 { method m {a} {return "NX1:$a"} }
oo::class create NX2 { superclass NX1; method m {a} {return "NX2:[next zzz]"} }
t e1 {NX2 create nxi; nxi m qq}

# object destroy removes from instances list
oo::class create IR { }
t f1 {IR create iri; info class instances IR}
t f2 {iri destroy; info class instances IR}

# unexport inherited (overwrite in subclass)
oo::class create UA2 { method m {} {return 1} }
oo::class create UB2 { superclass UA2; unexport m }
UB2 create ub2i
t g1 {catch {ub2i m} e; set e}
t g2 {info class methods UB2}

# methodtype / definition / call info
t h1 {catch {info class methodtype UA2 m} e; set e}
t h2 {info class methodtype UA2 m}
t h3 {info object methodtype oo::object destroy}
t h4 {catch {info class definition UA2 m} e; set e}
t h5 {info class definition UA2 m}
t h6 {catch {info class definition UA2 nosuch} e; set e}
t h7 {info class call ub2i m}

# creating object inside method; destroy self inside method
oo::class create MK { method mk {} {return [my makeone]} method makeone {} {return [info object class [self object]]} }
t i1 {MK create mki; mki mk}

# namespace current inside constructor
oo::class create NC2 { constructor {} {set ::nc [namespace current]} }
t j1 {NC2 create nci2; set ::nc}
