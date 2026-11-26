# lpop / lremove / ledit (tclsh 9.1) — behavior pins.
# Sources: doc/lpop.n doc/lremove.n doc/ledit.n + tclCmdIL.c
# Tcl_LpopObjCmd / Tcl_LremoveObjCmd / Tcl_LeditObjCmd.
# ── lpop ────────────────────────────────────────────────────────────
set l {a b c d}
puts [lpop l]
puts $l
puts [lpop l 0]
puts $l
set nested {{a b c} {d e f} {g h i}}
puts [lpop nested 1 2]
puts $nested
catch {lpop} m; puts $m
catch {lpop nope} m; puts "$m :: $errorCode"
set e {}
catch {lpop e} m; puts "$m :: $errorCode"
set l2 "x y"
catch {lpop l2 -1} m; puts "$m :: $errorCode"
catch {lpop l2 5} m; puts "$m :: $errorCode"
set l3 "x y"
catch {lpop l3 end+1} m; puts "$m :: $errorCode"
catch {lpop l {{a b}c} } m; puts "$m :: $errorCode"
# lpop on a non-list variable errors from the strict list parse
set nl "{a b}c"
catch {lpop nl} m; puts "$m :: $errorCode"
# variable is rewritten even when popped via sublist
set deep {a {b {c d}}}
puts [lpop deep 1 0]
puts $deep
# ── lremove ─────────────────────────────────────────────────────────
puts [lremove {a b c d e} 2]
puts [lremove {a b c d e} end-1 1]
puts [lremove {a b c d e} 2 end-2]
puts [lremove {a b c d e}]
catch {lremove} m; puts $m
puts [lremove {a b c d e} 99 -5]
puts [lremove {a b c d e} end]
catch {lremove {a b c} foo} m; puts "$m :: $errorCode"
# ── ledit ───────────────────────────────────────────────────────────
set lst {c d e f g}
puts [ledit lst -1 -1 a b]
puts $lst
set lst {c d e f g}
puts [ledit lst end+1 end+1 x]
puts $lst
set lst {c d e f g}
puts [ledit lst 1 2 X Y Z]
puts $lst
set lst {c d e f g}
puts [ledit lst 2 1 i]
puts $lst
set lst {c d e f g}
puts [ledit lst 0 end]
puts $lst
catch {ledit} m; puts $m
catch {ledit onlytwo} m; puts $m
catch {ledit nope 0 0} m; puts "$m :: $errorCode"
set lst {c d e f g}
catch {ledit lst foo 0} m; puts "$m :: $errorCode"
set lst {c d e f g}
catch {ledit lst 0 end+2} m; puts "$m :: $errorCode"
# ledit returns the variable's new value; with no values it deletes
set lst {a b c}
puts [ledit lst 1 1]
puts $lst
