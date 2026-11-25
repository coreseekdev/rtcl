# Source of judge/crash-probes/d23_variable_order.tcl: what listing
# variables shows, after deletes and recreations.  `_gen.tcl` runs every
# line under tclsh and writes the probe with tclsh's outputs as the
# expected values.
#
# The insertion log that replays tclsh's hash-table order used to keep
# every key it ever saw.  It is cut back now (live keys, last occurrence)
# and a listing with nothing stale in it reads the log straight through.
# What must not change: a re-set keeps its place, a recreate after a
# delete comes last.
set ::vq 1
set ::vp 2
set ::vo 3
set ::vn 4
set ::vm 5
T g-1 {info globals v*}
unset ::vp
T g-2 {info globals v*}
set ::vp 9
T g-3 {info globals v*}
set ::vp 10
T g-4 {info globals v*}
# Enough deletes for the log to be cut back, with the live keys untouched
for {set i 0} {$i < 700} {incr i} {set ::"stale$i" $i; unset ::"stale$i"}
T g-5 {info globals v*}
T g-6 {info globals stale*}
T g-7 {list [llength [info globals v*]] [llength [info globals stale*]]}
unset ::vq ::vp ::vo
T g-8 {info globals v*}
set ::vo 1
set ::vq 1
set ::vp 1
T g-9 {info globals v*}
T g-10 {list [info exists ::vp] [set ::vp]}
# Arrays: the element keys ride the same log
array set ::A {1 x 2 y 3 z}
T a-1 {array names ::A}
unset ::A(2)
T a-2 {array names ::A}
set ::A(2) y2
T a-3 {array names ::A}
T a-4 {list [array size ::A] [array get ::A]}
for {set i 0} {$i < 700} {incr i} {set ::A("t$i") $i; unset ::A("t$i")}
T a-5 {array names ::A}
T a-6 {list [array size ::A] [lsort [array names ::A]]}
T a-7 {array exists ::A}
array unset ::A
T a-8 {list [array exists ::A] [array size ::A]}
array set ::A {b 1 a 2}
T a-9 {array names ::A}
T a-10 {array statistics ::A}
# Locals of a frame
proc Q {} {
    set qa 1
    set qb 2
    set qc 3
    set r1 [info locals q*]
    unset qb
    set r2 [info locals q*]
    set qb 9
    set r3 [info locals q*]
    for {set i 0} {$i < 700} {incr i} {set "z$i" 1; unset "z$i"}
    set r4 [info locals q*]
    return [list $r1 $r2 $r3 $r4 [llength [info locals]]]
}
T locals {Q}
proc R {a args} {
    lappend d [list [info locals] [llength [info locals]]]
    for {set i 0} {$i < 300} {incr i} {set "y$i" 1; unset "y$i"}
    lappend d [list [info locals] [llength [info locals]]]
}
T locals-args {R 1}
# Locals: a frame's own order (the proc's table of names), a local made
# again in its old place and listed once, a live link not a local
T l-args-2 {proc LA {x y args} {info locals}; LA 1 2 3}
T l-args-1 {proc LB {a args} {info locals}; LB 1 2}
T l-args-default {proc LB2 {p {q 5} args} {info locals}; LB2 1}
T l-recreate {proc LC {} {set la 1; set li 2; set r0 [info locals l*]; unset la; set r1 [info locals l*]; set la 9; list $r0 $r1 [info locals l*] [info locals]]}}
T l-array-recreate {proc LD {} {array set aa {a 1 i 2}; set e0 [array names aa]; unset aa(a); set aa(a) 9; list $e0 [array names aa]}}
set ::gv 7
T l-variable {namespace eval ns {variable nv 8}; proc LV {} {variable ::ns::nv; info locals}; LV}
proc LG {} {global gv; info locals}
T l-global {LG}
proc LU {} {upvar #0 gv y; info locals}
T l-upvar {LU}
T l-upvar-dead {proc LU3 {} {upvar #0 gv y; unset y; info locals}; LU3}
T l-apply {apply {x {y 2} args} {info locals} 1}
T l-lambda-args {apply {{a args} {info locals}} 1 2 3}
# Array subcommands with an absolute name
array set ::Q {1 x 2 y}
T q-size {array size ::Q}
T q-get {array get ::Q}
T q-unset-elem {list [catch {array unset ::Q 1} m] $m [array names ::Q]}
proc QU {} {array unset ::Q 2}
T q-unset-in-proc {list [QU] [array names ::Q]}
T q-statistics {array statistics ::Q}
# Namespaces and commands replay their own logs the same way
namespace eval ::n1 {}
namespace eval ::n2 {}
namespace eval ::n3 {}
T ns-1 {namespace children :: n*}
namespace delete ::n2
T ns-2 {namespace children :: n*}
namespace eval ::n2 {}
T ns-3 {namespace children :: n*}
proc ::zzqa {} {}
proc ::zzqb {} {}
proc ::zzqc {} {}
T c-1 {info commands zzq*}
rename ::zzqb {}
T c-2 {info commands zzq*}
proc ::zzqb {} {}
T c-3 {info commands zzq*}
rename ::zzqa ::zzqa2
T c-4 {info commands zzq*}
namespace delete ::n1 ::n2 ::n3
rename ::zzqa2 {}
rename ::zzqb {}
rename ::zzqc {}
puts DONE
