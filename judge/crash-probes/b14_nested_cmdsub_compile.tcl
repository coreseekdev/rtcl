# crash-probe: b14 nested cmdsub compile
source [file join [file dirname [info script]] _chk.tcl]
# 20000 nested `[list [`: the parser refuses the nesting (tclsh's
# evaluation limit) instead of letting the compiler recurse per level.
set n 20000
set body "set r [string repeat {[list } $n]1[string repeat {]} $n]"
chk define [catch {proc p {} $body} e] 0
chk call [catch {p} e] 1
# Within the limit everything still works, inline or nested-eval path.
set m 600
set body2 "set r [string repeat {[list } $m]1[string repeat {]} $m]"
proc q {} $body2
chk q [catch {q} e] 0
# Quoted-word brackets go through the nested-eval path at every level.
set s "set v x[string repeat {[string cat } $m]y[string repeat {]} $m]"
chk nested-eval [catch {uplevel #0 $s} e] 0
chk nested-val $v xy
