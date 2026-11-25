# Source of judge/crash-probes/d28_local_order.tcl: the ORDER of `info
# locals` / `info vars` inside a proc (T3).  tclsh's order is two
# sections: the compiled locals first — in the slot table's order,
# parameters first, an undefined one not listed — then the frame's
# dynamic (hash) table, replayed through the Tcl hash mechanics over the
# creation log.  A `global`/`variable`/`upvar` link on a compile-time
# name is itself a compiled local (tclsh compiles the link commands to
# slot ops), so it lists at its source position — with `info vars`, not
# with `info locals` (a link is not a local).  A dynamic name deleted
# and made again takes a new place; a compiled local deleted and made
# again returns to its old place (its slot).
#
# `_gen.tcl` runs every line under tclsh and writes the probe with
# tclsh's outputs as the expected values; `--refix` rewrites them from
# the probe's own state.
#
# Left out on purpose (rtcl differs, T3 边界记档):
#   b1 — tclsh registers an `unset`-only literal target as a compiled
#        local (TclCompileUnsetCmd PushVarNameWord); rtcl's unset is a
#        runtime dispatch and registers nothing, so zzu lists in the
#        hash section at its recreated place;
#   b2 — tclsh registers `catch {} var` and `dict set d k v` literal
#        targets as compiled locals; rtcl dispatches both, so zzc/zzm
#        list in the hash section;
#   info frame — rtcl's `info frame` is an rtcl-shaped extension (type
#        proc level N cmd {} locals {…}), not tclsh's frame report, so
#        there is no oracle for it.
# ── the compiled section: parameters first, then body locals ────────

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
chk s1 [T {proc P1 {b a} {return [info locals]}; P1 1 2}] {0 {b a} NONE}
chk s2 [T {proc P2 {x {y 5} args} {return [info locals]}; P2 1}] {0 {x y args} NONE}
chk s3 [T {proc P3 {args} {return [info locals]}; P3}] {0 args NONE}
chk s4 [T {proc P4 {} {set zz1 1; set zz2 2; foreach n {q1} {set $n 1}; return [info locals]}; P4}] {0 {zz1 zz2 n q1} NONE}
chk s5 [T {proc P5 {} {foreach n {zzA zzt zzq zzr} {set $n 1}; set q $zzr; return [info locals]}; P5}] {0 {n q zzr zzt zzq zzA} NONE}
chk s6 [T {proc P6 {} {foreach n {zzA zzt zzq zzi zzl zzf} {set $n 1}; incr zzi 0; lappend zzl x; for {set zzf 0} {$zzf < 0} {} {}; return [info locals]}; P6}] {0 {n zzi zzl zzf zzt zzq zzA} NONE}
chk s7 [T {proc P7 {} {foreach n {zzA zzt zzq zzj} {set $n 1}; set q [set zzj 5]; return [info locals]}; P7}] {0 {n q zzj zzt zzq zzA} NONE}
chk s8 [T {proc P8 {} {foreach n {zzA zzt zzq zzi zzj} {set $n 1}; incr zzi [set zzj 5]; return [info locals]}; P8}] {0 {n zzi zzj zzt zzq zzA} NONE}
chk s9 [T {proc P9 {} {set zx1 1; unset zx1; list [info locals] [info exists zx1] [info locals zx*]}; P9}] {0 {{} 0 {}} NONE}
chk s10 [T {proc P10 {} {return [info locals]}; P10}] {0 {} NONE}
chk s11 [T {proc P11 {} {foreach n {za zb zc zd ze zf zg zh zi zj zk} {set $n 1}; return [info locals]}; P11}] {0 {n zj zf zb zk zg zc zh zd zi ze za} NONE}
chk s12 [T {proc P12 {} {foreach n {za zb zc zd ze zf zg zh zi zj zk zm zn zo zp} {set $n 1}; return [info locals]}; P12}] {0 {n zf zg zh zi zj zk zm zn zo zp za zb zc zd ze} NONE}
chk s13 [T {proc P13 {} {foreach n {za zb zc zd ze zf zg zh zi zj zk} {set $n 1}; unset zc; set zc 9; return [info locals]}; P13}] {0 {n zc zj zf zb zk zg zh zd zi ze za} NONE}
chk s14 [T {proc P14 {} {foreach n {za zb zc} {set $n 1}; set zzp 1; return [info locals zz*]}; P14}] {0 zzp NONE}
chk s15 [T {proc P15 {} {foreach n {A a B b} {set $n 1}; set r1 [info locals]; unset A; set A 2; lappend r1 [info locals]; unset b; set b 2; lappend r1 [info locals]; return $r1}; P15}] {0 {n A b a B {n r1 A b a B} {n r1 A b a B}} NONE}
chk s16 [T {proc P16 {} {set c1 1; global lka; foreach n {aa ac} {set $n 1}; list [info locals] [info vars]}; P16}] {0 {{c1 n ac aa} {c1 lka n ac aa}} NONE}
chk s17 [T {proc P17 {} {foreach n {za zb zc} {set $n 1}; variable zzv; list [info locals] [info vars]}; P17}] {0 {{n zb zc za} {n zzv zb zc za}} NONE}
chk s18 [T {proc P18 {} {set gv0 5; upvar 0 gv0 y; list [info locals] [info vars]}; P18}] {0 {gv0 {gv0 y}} NONE}
chk s19 [T {proc P19 {} {foreach n {za zb zc} {set $n 1}; set k zzh; global $k; return [info vars]}; P19}] {0 {n k zzh zb zc za} NONE}
chk s20 [T {proc P20 {} {foreach n {za zb zc} {set $n 1}; set l2 zzh; upvar 0 nothere $l2; return [info vars]}; P20}] {0 {n l2 zzh zb zc za} NONE}
chk s21 [T {proc P21 {} {set zzg1 1; set zzg2 2; unset zzg2; list [catch {global zzg1} e1] $e1 [catch {global zzg2} e2] $e2}; P21}] {0 {1 {variable "zzg1" already exists} 0 {}} {TCL UPVAR EXISTS}}
chk s22 [T {proc P22 {} {set zzg3 1; set r [catch {upvar 0 notthere zzg3} m]; list $r $m}; P22}] {0 {1 {variable "zzg3" already exists}} {TCL UPVAR EXISTS}}
U {proc PB1 {} {foreach n {zzA zzt zzq zzu} {set $n 1}; unset zzu; set k zzu; set $k 9; return [info locals]}; PB1}
U {proc PB2 {} {foreach n {zzA zzt zzq} {set $n 1}; catch {} zzc; dict set zzm k 1; return [info locals]}; PB2}

# 24 lines; the chk ones are the checks.
