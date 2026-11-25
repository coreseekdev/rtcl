# Source of judge/crash-probes/d27_ns_body.tcl: variables in a
# `namespace eval` body.  `_gen.tcl` runs every line under tclsh and
# writes the probe with tclsh's outputs as the expected values; the
# `--refix` pass then takes them from the probe's own lines.
#
# 1. A body evaluated from inside a proc.  tclsh gives it its own
#    variable frame: the proc's locals are invisible in it and a bare
#    name is the namespace's variable.  rtcl only switched the current
#    namespace, so the body read and wrote the proc's locals —
#    `set x secret; namespace eval n { set x 1 }` left x at 1 — and never
#    saw the namespace's variables.
# 2. The namespace chain at the top level: a global read or written from
#    inside a body by its bare name.  Writes went to the global, but its
#    traces did not fire and the array subcommands looked only under the
#    namespace's own name.
# 3. A global that is in the table without a value: a trace on it, a
#    proc's `global`/`upvar #0` link to it, a `variable` declaration.
#    Such an entry takes the write; rtcl made the namespace's variable.
#
# Left out (rtcl differs already, for other reasons):
#   `info vars` inside a body lists the globals of the running
#   interpreter, and rtcl's start-up variables are not tclsh's.
proc nop {a b c} {}
set ::log {}
proc tl {a b c} { lappend ::log [list $a $b $c] }

# ── 1. a body run from inside a proc ────────────────────────────────
namespace eval n1 { variable v 1 }
proc N1 {} {
    set x secret
    namespace eval n1 { set x 1; set y $x }
    return [list $x [catch {set ::n1::y} m] $m]
}
T n1 {N1}
proc N2 {} {
    set x secret
    foreach v {1 2} { append acc $v }
    namespace eval n1 { foreach v {a b} { append acc $v } }
    return [list $x $v $acc]
}
T n2 {N2}
proc N3 {} { namespace eval n1 { for {set i 0} {$i < 3} {incr i} { append s $i } ; return $s } }
T n3 {N3}
proc N4 {} {
    set r old
    namespace eval n1 { lappend r x ; incr c ; dict set d k 1 ; array set ar {1 x} }
    return [list $r [catch {set ::n1::c} m] $m [catch {set ::n1::d} m2] $m2 [catch {array get ::n1::ar} m3] $m3]
}
T n4 {N4}
proc N5 {} { set lv 5; namespace eval n1 { lappend lv 9 ; incr lv ; append lv x }; return $lv }
T n5 {N5}
proc N6 {} { namespace eval n1 { append v 4 } }
T n6 {N6}
T n7 {set n1::v}
proc N8 {} { namespace eval n1 { list [set v] [info exists v] [namespace current] [info locals] } }
T n8 {N8}
proc N9 {} { namespace eval n1 { set k2 7 } }
T n9 {N9}
T n10 {info exists n1::k2}
proc NA {} { namespace eval n1 { variable z 9 } }
T na {NA}
T nb {set n1::z}
proc NC {} { namespace eval n1 { array set ar2 {1 x} } }
T nc {NC}
T nd {array get n1::ar2}
proc NE {} { set local 5; namespace eval n1 { list [catch {set local} m] $m } }
T ne {NE}
proc NF {} { namespace eval n1 { namespace eval n1 { append v 0 } } }
T nf {NF}
T ng {list [set n1::v] [catch {set n1::n1::v} m] $m}
proc NH {} { namespace eval n1 { apply {{} { append v 0 }} } }
T nh {NH}
proc NI {} { namespace eval n1 { info level } }
T ni {NI}
proc NJ {} { namespace eval n1 { uplevel 1 { info level } } }
T nj {NJ}
proc NK {} { namespace eval n1 { dict set d2 k 1 } }
T nk {NK}
T nl {set n1::d2}
proc NL {} { namespace eval n1 { global gb1; set gb1 5 } }
T nm {NL}
T nn {list [info exists ::gb1] [info exists n1::gb1] [catch {set n1::gb1} m] $m}
set ::gsrc 7
proc NN {} { namespace eval n1 { upvar 1 gsrc ly } }
T no {NN}
T np {list [catch {set n1::ly} m] $m}
proc NO {} { namespace eval n1 { upvar #0 gsrc g3; append g3 1 } }
T nq {NO}
T nr {list $::gsrc [catch {set n1::g3} m] $m}
proc NP {} { namespace eval n1 { upvar 0 zz zz9 } }
T ns {NP}
proc NQ {} { namespace eval n1 { upvar 1 ::gsrc g4; append g4 1 } }
T nt {NQ}
T nu {list $::gsrc [catch {set n1::g4} m] $m}
proc NR {} { namespace eval n1 { append nosuch 1 } }
T nv {NR}
proc NS {ns body} { namespace eval $ns $body }
T nw {NS n1 { append v 5 }}
T nx {set n1::v}
proc NT {} {
    set x outer
    set body { set x inner }
    namespace eval n1 $body
    return [list $x [catch {set ::n1::x} m] $m]
}
T ny {NT}
set topslot T
proc TP {} { namespace eval n1 { set topslot N } }
T nz {TP}
T o1 {list $topslot [catch {set n1::topslot} m] $m}
proc ER {} { namespace eval n1 { set msg "from ns"; error "$msg boom" } }
T o2 {catch ER m}
T o3 {list $m [lindex [split $::errorInfo \n] 0] [lindex [split $::errorInfo \n] 1] [lindex [split $::errorInfo \n] 2]}
T o4 {namespace eval n1 { set msg2 hi; set ::saved $msg2; unset msg2; list [info exists msg2] $::saved }}
proc EU {} { namespace eval n1 { unset nosuchvar } }
T o5 {EU}
proc EV {} { namespace eval n1 { array set ea {1 x}; list [catch {set ea} m] $m [catch {set ea(9)} m2] $m2 [catch {set ea(1)} m3] $m3 } }
T o6 {EV}

# ── 2. the chain at the top level ───────────────────────────────────
namespace eval n2 {}
set g 1
trace add variable g write tl
T c1 {namespace eval n2 { set g 2 }}
T c2 {list $::g $::log}
array set a {1 x 2 y}
trace add variable a write tl
T c3 {namespace eval n2 { set a(3) z }}
T c4 {list [array get a] [lsort [array names a]] [array exists n2::a] $::log}
T c5 {namespace eval n2 { append g 3 }}
T c6 {list $::g $::log}
T c7 {namespace eval n2 { incr g }}
T c8 {namespace eval n2 { lappend g 4 }}
T c9 {list $::g [llength $g] [llength $::log]}
T c10 {namespace eval n2 { unset g }}
T c11 {list [info exists ::g] [info exists n2::g] [llength $::log]}
set h 1
T c12 {namespace eval n2 { unset h }}
T c13 {list [info exists ::h] [info exists n2::h]}
array set b {1 x}
T c14 {namespace eval n2 { array get b }}
T c15 {namespace eval n2 { array size b }}
T c16 {namespace eval n2 { array names b }}
T c17 {namespace eval n2 { info exists b(1) }}
T c18 {namespace eval n2 { array exists b }}
T c19 {namespace eval n2 { set b(2) y }}
T c20 {array get b}
T c21 {namespace eval n2 { array unset b 1 }}
T c22 {array get b}
T c23 {namespace eval n2 { list [catch {set b} m] $m [set b(3)] }}
T c24 {array get b}
set ::tt 1
trace add variable tt read tl
T c25 {namespace eval n2 { set tt }}
T c26 {lindex $::log end}
set k 1
T c27 {namespace eval n2 { list [catch {set k} m] $m }}
array set ka {1 x}
T c28 {namespace eval n2 { list [catch {set ka} m] $m [catch {unset ka} m2] $m2 }}
T c29 {array get ka}
namespace eval n2 { variable own 1 }
set ::own 2
T c30 {namespace eval n2 { append own x }}
T c31 {list [set n2::own] $::own}
namespace eval n2 { upvar #0 h hh; set h 9 }
T c32 {list [catch {set ::h} m] $m}

# ── 3. in the table without a value ─────────────────────────────────
variable q
T p1 {namespace eval n2 { set q 1 }}
T p2 {list [catch {set ::q} m] $m [info exists n2::q]}
namespace eval n2 { variable r }
T p3 {namespace eval n2 { set r 1 }}
T p4 {list [info exists ::r] [catch {set n2::r} m] $m]}
trace add variable s write nop
trace remove variable s write nop
T p5 {namespace eval n2 { set s 1 }}
T p6 {list [info exists ::s] [info exists n2::s]]}
trace add variable t write tl
T p7 {namespace eval n2 { set t 1 }}
T p8 {list [catch {set ::t} m] $m [info exists n2::t] [lindex $::log end]}
unset -nocomplain ::t
T p9 {namespace eval n2 { set t 2 }}
T p10 {list [catch {set ::t} m] $m [info exists n2::t]}
trace add variable w write tl
T p11 {namespace eval n2 { info exists w }}
T p12 {namespace eval n2 { array exists w }}
T p13 {list [info exists ::w] [namespace eval n2 { info exists w }]}
trace add variable z unset tl
set z 1
unset z
T p14 {namespace eval n2 { set z 5 }}
T p15 {list [catch {set ::z} m] $m [info exists n2::z]}
trace add variable y read tl
T p16 {namespace eval n2 { catch {set y} m }}
T p17 {list $m [catch {namespace eval n2 { set y 3 }} m2] $m2 [catch {set ::y} m3] $m3}
trace add variable pe(1) write tl
T p18 {namespace eval n2 { set pe(1) 9 }}
T p19 {list [catch {set ::pe(1)} m] $m [lindex $::log end]}
proc pg1 {} { global gx; namespace eval n2 { append gx x } }
T p20 {pg1}
T p21 {list [catch {set ::gx} m] $m [info exists n2::gx]]}
proc pu1 {} { upvar #0 ux ul; namespace eval n2 { append ux x } }
T p22 {pu1}
T p23 {list [catch {set ::ux} m] $m [info exists n2::ux]}
proc pg2 {} { global gy; namespace eval n2 { set gy 1 } }
T p24 {pg2}
T p25 {list [catch {set ::gy} m] $m [info exists n2::gy]}
proc pg3 {} { global gz; namespace eval n2 { info exists gz } }
T p26 {pg3}
T p27 {list [catch {pg3} m] $m [info exists ::gz]}
proc pg4 {} { global gw; catch {namespace eval n2 { unset gw }} m; return $m }
T p28 {pg4}
set pv 1
trace add variable pv unset tl
T p29 {namespace eval n2 { unset pv }}
T p30 {list [info exists ::pv] [info exists n2::pv] [lindex $::log end]}

# ── the proc's own scope is unchanged ───────────────────────────────
proc PA {} {
    set x 1
    set r [namespace eval n1 { set x 2 }]
    return [list $x $r [catch {set n1::x} m] $m]
}
T a1 {PA}
proc PB {} {
    global gb
    set gb 3
    namespace eval n1 { append gb x }
    return [list $gb [catch {set ::gb} m] $m]
}
T a2 {PB}
proc PC {} {
    variable v
    append v q
    namespace eval n1 { append v w }
    return [list $v]
}
T a3 {PC}
T a4 {set n1::v}

# ── 4. reading and unsetting an entry without a value ──────────────
# The message is that of a missing variable; tclsh found the Var all the
# same, and its errorCode says so (READ / UNSET, where a name no table
# knows is LOOKUP with the name).
trace add variable u1 read nop
T v1 {catch {set u1} m}
T v2 {list $m $::errorCode}
trace add variable u2 write nop
T v3 {catch {set u2} m}
T v4 {list $m $::errorCode}
proc LU {} { set x 1; unset x; set x }
T v5 {catch LU m}
T v6 {list $m $::errorCode}
proc LUU {} { if 0 { set y 1 }; set y }
T v7 {catch LUU m}
T v8 {list $m $::errorCode}
proc LU2 {} { set v dd; catch {set $v} m e; list $m [dict get $e -errorcode] }
T v9 {LU2}
set g1 1
unset g1
T v10 {catch {set g1} m}
T v11 {list $m $::errorCode}
proc G2 {} { global gg; catch {set gg} m e; list $m $e }
T v12 {G2}
trace add variable uu read nop
T v13 {catch {namespace eval n2 { set uu }} m}
T v14 {list $m $::errorCode}
trace remove variable uu read nop
T v15 {catch {set uu} m}
T v16 {list $m $::errorCode}
proc PG4 {} { global gw; catch {namespace eval n2 { unset gw }} m e; list $m $e }
T v17 {PG4}
T v18 {catch {unset ::gw} m}
T v19 {list $m $::errorCode}
proc PG5 {} { global gww; unset gww }
T v20 {catch PG5 m}
T v21 {list $m $::errorCode}
variable dq
T v22 {catch {set dq} m}
T v23 {list $m $::errorCode}
T v24 {catch {unset dq} m}
T v25 {list $m $::errorCode}
T v25b {catch {set dq} m}
T v25c {list $m $::errorCode}
namespace eval n2 { variable nq }
T v26 {catch {namespace eval n2 { set nq }} m}
T v27 {list $m $::errorCode}
trace add variable qe(1) read nop
T v28 {catch {set qe(1)} m}
T v29 {list $m $::errorCode}

puts DONE
