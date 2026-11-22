proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# --- colon-name semantics ---
t tail-a-colons {list [namespace tail a:::] [namespace tail a:b] [namespace tail a::]}
t qual-a-colons {list [namespace qualifiers a:::] [namespace qualifiers a:b]}
catch {namespace delete ::tnv}
namespace eval tnv { variable : X1 }
t which-colon {namespace eval tnv {namespace which -variable :}}
t set-abs-colon {set ::tnv:::}
t abs-colon-eq {set ::tnv:::}
t which-colon-abs {namespace eval tnv {namespace which -variable ::tnv:::}}
t exists-abs-colon {info exists ::tnv:::}
catch {namespace delete ::t2}
namespace eval t2 {}
t set-trailing2 {set ::t2:: 5}
t get-trailing2 {set ::t2::}
t set-colon-run {set ::t2::: 6}
t get-colon-run {set ::t2:::}
t t2-vars {namespace eval t2 {info vars}}

# --- parent-ns error for plain (non-::) set inside ns ---
namespace eval pns {}
t set-rel-trailing {namespace eval pns {set sub:: 1}}
t set-rel-qual {namespace eval pns {set sub::x 1}}
t unset-missing-ns {catch {unset ::nope4::zz}}

# --- upvar to plain global; unset then write ---
set g1 1
proc pG {} { upvar 0 g1 gg; unset g1; set gg 3 }
t upvar-scalar-unset-then-write {pG}
t upvar-scalar-recreated {set g1}
set g2 1
proc pG2 {} { upvar 0 g2 gg; unset g2; set gg }
t upvar-scalar-read-after-unset {pG2}
set g3 1
proc pG3 {} { upvar 0 g3 gg; unset g3; info exists gg }
t upvar-scalar-exists-after-unset {pG3}

# --- element alias after unset array (1.17 shape) ---
set arr(1) 2
proc pA {} { upvar 0 arr(1) foo; lappend r [catch {set foo} m] $m; unset arr; lappend r [catch {set foo 3} m2] $m2; lappend r [catch {set foo(3) 3} m3] $m3; lappend r [info exists foo] }
t upvar-elem {pA}

# --- variable alias: read/unset after ns delete (1.15 extensions) ---
catch {namespace delete ::dn}
namespace eval dn { variable fv 9 }
proc pV {} {
    variable ::dn::fv
    set r {}
    lappend r [set fv]
    namespace delete ::dn
    lappend r [catch {set fv} m1]
    lappend r [catch {unset fv} m2]
    lappend r [info exists fv]
    set r
}
t var-alias-after-del {pV}

# --- global command: relative qualified name ---
catch {namespace delete ::gq}
namespace eval gq { variable v 55 }
proc pGlobalRel {} { global gq::v; set v }
t global-rel-qualified {pGlobalRel}
proc pGlobalAbs {} { global ::gq::v; set v }
t global-abs-qualified {pGlobalAbs}

# --- ns delete: command traces vs variable unset traces order ---
set olog {}
proc ocmd {o n op} { lappend ::olog "cmd:$o" }
proc ovar {n i o} { lappend ::olog "var:$n" }
catch {namespace delete ::oz}
namespace eval oz { proc pr {} {} }
set ::oz::vv 1
trace add command ::oz::pr delete ocmd
trace add variable ::oz::vv unset ovar
t oz-del {namespace delete ::oz}
t oz-log {set ::olog}
