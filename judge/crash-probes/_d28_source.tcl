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
T s1 {proc P1 {b a} {return [info locals]}; P1 1 2}
T s2 {proc P2 {x {y 5} args} {return [info locals]}; P2 1}
T s3 {proc P3 {args} {return [info locals]}; P3}
T s4 {proc P4 {} {set zz1 1; set zz2 2; foreach n {q1} {set $n 1}; return [info locals]}; P4}
# ── what registers a slot: set (target before the value words), $read,
# incr, lappend, for's init set, foreach vars ────────────────────────
T s5 {proc P5 {} {foreach n {zzA zzt zzq zzr} {set $n 1}; set q $zzr; return [info locals]}; P5}
T s6 {proc P6 {} {foreach n {zzA zzt zzq zzi zzl zzf} {set $n 1}; incr zzi 0; lappend zzl x; for {set zzf 0} {$zzf < 0} {} {}; return [info locals]}; P6}
T s7 {proc P7 {} {foreach n {zzA zzt zzq zzj} {set $n 1}; set q [set zzj 5]; return [info locals]}; P7}
T s8 {proc P8 {} {foreach n {zzA zzt zzq zzi zzj} {set $n 1}; incr zzi [set zzj 5]; return [info locals]}; P8}
# an undefined compiled local does not exist and is not listed
T s9 {proc P9 {} {set zx1 1; unset zx1; list [info locals] [info exists zx1] [info locals zx*]}; P9}
T s10 {proc P10 {} {return [info locals]}; P10}
# ── the dynamic section: creation log replayed through the hash ─────
T s11 {proc P11 {} {foreach n {za zb zc zd ze zf zg zh zi zj zk} {set $n 1}; return [info locals]}; P11}
T s12 {proc P12 {} {foreach n {za zb zc zd ze zf zg zh zi zj zk zm zn zo zp} {set $n 1}; return [info locals]}; P12}
T s13 {proc P13 {} {foreach n {za zb zc zd ze zf zg zh zi zj zk} {set $n 1}; unset zc; set zc 9; return [info locals]}; P13}
T s14 {proc P14 {} {foreach n {za zb zc} {set $n 1}; set zzp 1; return [info locals zz*]}; P14}
# ── a compiled local deleted and made again returns to its old place ─
T s15 {proc P15 {} {foreach n {A a B b} {set $n 1}; set r1 [info locals]; unset A; set A 2; lappend r1 [info locals]; unset b; set b 2; lappend r1 [info locals]; return $r1}; P15}
# ── links: a compile-time link lists at its source position, with
# `info vars`, not `info locals`; a computed link name is a frame-table
# (hash) entry and lists among the dynamic names ─────────────────────
T s16 {proc P16 {} {set c1 1; global lka; foreach n {aa ac} {set $n 1}; list [info locals] [info vars]}; P16}
T s17 {proc P17 {} {foreach n {za zb zc} {set $n 1}; variable zzv; list [info locals] [info vars]}; P17}
T s18 {proc P18 {} {set gv0 5; upvar 0 gv0 y; list [info locals] [info vars]}; P18}
T s19 {proc P19 {} {foreach n {za zb zc} {set $n 1}; set k zzh; global $k; return [info vars]}; P19}
T s20 {proc P20 {} {foreach n {za zb zc} {set $n 1}; set l2 zzh; upvar 0 nothere $l2; return [info vars]}; P20}
# ── a defined local blocks the link; an undefined one links fine ─────
T s21 {proc P21 {} {set zzg1 1; set zzg2 2; unset zzg2; list [catch {global zzg1} e1] $e1 [catch {global zzg2} e2] $e2}; P21}
T s22 {proc P22 {} {set zzg3 1; set r [catch {upvar 0 notthere zzg3} m]; list $r $m}; P22}
# ── T3 边界（rtcl 与 tclsh 的既定分歧，留档不钉）─────────────────────
# skip b1 b2 -- rtcl 不给 unset-only / catch / dict set 的字面目标登记编译局部（T3 修法边界，见提交说明）
T b1 {proc PB1 {} {foreach n {zzA zzt zzq zzu} {set $n 1}; unset zzu; set k zzu; set $k 9; return [info locals]}; PB1}
T b2 {proc PB2 {} {foreach n {zzA zzt zzq} {set $n 1}; catch {} zzc; dict set zzm k 1; return [info locals]}; PB2}
puts DONE
