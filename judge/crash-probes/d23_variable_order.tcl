# crash-probe: d23 what lists variables, after deletes and recreations
# Every scope keeps a log of the keys it saw, in order, to replay tclsh's
# hash-table iteration order (`info globals`, `array names`, …).  Nothing
# ever left that log: a loop making and unsetting variables grew it
# without end (300000 × 3 rounds: 96MB held against tclsh's 5MB), and
# every listing walked all of it — `array names` on 80000 elements took
# 600ms once the log had stale entries.  The log is cut back to the live
# keys now; what must read the same is the order a re-set keeps and the
# place a recreated key takes.

source [file join [file dirname [info script]] _chk.tcl]
proc T {script} {
    # A success leaves the previous error's errorCode standing: start
    # each check from NONE.
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    return [list $c $r [expr {[info exists ::errorCode] ? $::errorCode : {}}]]
}
# A skipped check still builds the state the later ones sit on: its
# script runs, unchecked.
proc U {script} {
    catch {uplevel 1 $script}
}
set ::errorCode NONE
set ::vq 1
set ::vp 2
set ::vo 3
set ::vn 4
set ::vm 5
chk g-1 [T {info globals v*}] {0 {vm vn vo vp vq} NONE}
unset ::vp
chk g-2 [T {info globals v*}] {0 {vm vn vo vq} NONE}
set ::vp 9
chk g-3 [T {info globals v*}] {0 {vm vn vo vp vq} NONE}
set ::vp 10
chk g-4 [T {info globals v*}] {0 {vm vn vo vp vq} NONE}
for {set i 0} {$i < 700} {incr i} {set ::"stale$i" $i; unset ::"stale$i"}
chk g-5 [T {info globals v*}] {0 {vm vn vo vp vq} NONE}
chk g-6 [T {info globals stale*}] {0 {} NONE}
chk g-7 [T {list [llength [info globals v*]] [llength [info globals stale*]]}] {0 {5 0} NONE}
unset ::vq ::vp ::vo
chk g-8 [T {info globals v*}] {0 {vm vn} NONE}
set ::vo 1
set ::vq 1
set ::vp 1
chk g-9 [T {info globals v*}] {0 {vm vn vo vp vq} NONE}
chk g-10 [T {list [info exists ::vp] [set ::vp]}] {0 {1 1} NONE}
array set ::A {1 x 2 y 3 z}
chk a-1 [T {array names ::A}] {0 {1 2 3} NONE}
unset ::A(2)
chk a-2 [T {array names ::A}] {0 {1 3} NONE}
set ::A(2) y2
chk a-3 [T {array names ::A}] {0 {1 2 3} NONE}
chk a-4 [T {list [array size ::A] [array get ::A]}] {0 {3 {1 x 2 y2 3 z}} NONE}
for {set i 0} {$i < 700} {incr i} {set ::A("t$i") $i; unset ::A("t$i")}
chk a-5 [T {array names ::A}] {0 {1 2 3} NONE}
chk a-6 [T {list [array size ::A] [lsort [array names ::A]]}] {0 {3 {1 2 3}} NONE}
chk a-7 [T {array exists ::A}] {0 1 NONE}
array unset ::A
chk a-8 [T {list [array exists ::A] [array size ::A]}] {0 {0 0} NONE}
array set ::A {b 1 a 2}
chk a-9 [T {array names ::A}] {0 {a b} NONE}
chk a-10 [T {array statistics ::A}] {0 {2 entries in table, 4 buckets
number of buckets with 0 entries: 2
number of buckets with 1 entries: 2
number of buckets with 2 entries: 0
number of buckets with 3 entries: 0
number of buckets with 4 entries: 0
number of buckets with 5 entries: 0
number of buckets with 6 entries: 0
number of buckets with 7 entries: 0
number of buckets with 8 entries: 0
number of buckets with 9 entries: 0
number of buckets with 10 or more entries: 0
average search distance for entry: 1.0} NONE}
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
chk locals [T Q] {0 {{qa qb qc} {qa qc} {qa qb qc} {qa qb qc} 8} NONE}
proc R {a args} {
    lappend d [list [info locals] [llength [info locals]]]
    for {set i 0} {$i < 300} {incr i} {set "y$i" 1; unset "y$i"}
    lappend d [list [info locals] [llength [info locals]]]
}
chk locals-args [T {R 1}] {0 {{{a args} 2} {{a args d i} 4}} NONE}
chk l-args-2 [T {proc LA {x y args} {info locals}; LA 1 2 3}] {0 {x y args} NONE}
chk l-args-1 [T {proc LB {a args} {info locals}; LB 1 2}] {0 {a args} NONE}
chk l-args-default [T {proc LB2 {p {q 5} args} {info locals}; LB2 1}] {0 {p q args} NONE}
chk l-recreate [T {proc LC {} {set la 1; set li 2; set r0 [info locals l*]; unset la; set r1 [info locals l*]; set la 9; list $r0 $r1 [info locals l*] [info locals]]}}] {0 {} NONE}
chk l-array-recreate [T {proc LD {} {array set aa {a 1 i 2}; set e0 [array names aa]; unset aa(a); set aa(a) 9; list $e0 [array names aa]}}] {0 {} NONE}
set ::gv 7
chk l-variable [T {namespace eval ns {variable nv 8}; proc LV {} {variable ::ns::nv; info locals}; LV}] {0 {} NONE}
proc LG {} {global gv; info locals}
chk l-global [T LG] {0 {} NONE}
proc LU {} {upvar #0 gv y; info locals}
chk l-upvar [T LU] {0 {} NONE}
chk l-upvar-dead [T {proc LU3 {} {upvar #0 gv y; unset y; info locals}; LU3}] {0 {} NONE}
chk l-apply [T {apply {x {y 2} args} {info locals} 1}] {1 {namespace "::args" not found} {TCL LOOKUP NAMESPACE ::args}}
chk l-lambda-args [T {apply {{a args} {info locals}} 1 2 3}] {0 {a args} NONE}
array set ::Q {1 x 2 y}
chk q-size [T {array size ::Q}] {0 2 NONE}
chk q-get [T {array get ::Q}] {0 {1 x 2 y} NONE}
chk q-unset-elem [T {list [catch {array unset ::Q 1} m] $m [array names ::Q]}] {0 {0 {} 2} NONE}
proc QU {} {array unset ::Q 2}
chk q-unset-in-proc [T {list [QU] [array names ::Q]}] {0 {{} {}} NONE}
chk q-statistics [T {array statistics ::Q}] {0 {0 entries in table, 4 buckets
number of buckets with 0 entries: 4
number of buckets with 1 entries: 0
number of buckets with 2 entries: 0
number of buckets with 3 entries: 0
number of buckets with 4 entries: 0
number of buckets with 5 entries: 0
number of buckets with 6 entries: 0
number of buckets with 7 entries: 0
number of buckets with 8 entries: 0
number of buckets with 9 entries: 0
number of buckets with 10 or more entries: 0
average search distance for entry: 0.0} NONE}
namespace eval ::n1 {}
namespace eval ::n2 {}
namespace eval ::n3 {}
chk ns-1 [T {namespace children :: n*}] {0 {::n2 ::n3 ::ns ::n1} NONE}
namespace delete ::n2
chk ns-2 [T {namespace children :: n*}] {0 {::n3 ::ns ::n1} NONE}
namespace eval ::n2 {}
chk ns-3 [T {namespace children :: n*}] {0 {::n2 ::n3 ::ns ::n1} NONE}
proc ::zzqa {} {}
proc ::zzqb {} {}
proc ::zzqc {} {}
chk c-1 [T {info commands zzq*}] {0 {zzqa zzqb zzqc} NONE}
rename ::zzqb {}
chk c-2 [T {info commands zzq*}] {0 {zzqa zzqc} NONE}
proc ::zzqb {} {}
chk c-3 [T {info commands zzq*}] {0 {zzqa zzqb zzqc} NONE}
rename ::zzqa ::zzqa2
chk c-4 [T {info commands zzq*}] {0 {zzqa2 zzqb zzqc} NONE}
namespace delete ::n1 ::n2 ::n3
rename ::zzqa2 {}
rename ::zzqb {}
rename ::zzqc {}

# 86 lines; the chk ones are the checks.
