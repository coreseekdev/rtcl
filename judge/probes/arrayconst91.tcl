# array default / array for / const / info constant|consts (tclsh 9.1)
# Sources: doc/array.n (TIP 508 default + array for), tclVar.c
# Tcl_ConstObjCmd + VAR_CONSTANT guards, tclInfo.c info constant/consts.
# ── array default face ──────────────────────────────────────────────
catch {array default} m; puts "1:$m :: $errorCode"
catch {array default gorp ary} m; puts "2:$m :: $errorCode"
catch {array default get nosuch} m; puts "3:$m :: $errorCode"
catch {array default set} m; puts "4:$m"
catch {array default exists} m; puts "5:$m"
catch {array default exists a b} m; puts "6:$m"
array set a {x 1}
catch {array default get a} m; puts "7:$m :: $errorCode"
array default set a foo
puts "8:[array default get a] [array default exists a]"
array default unset a
puts "9:[array default exists a]"
catch {array default get a} m; puts "10:$m :: $errorCode"
catch {array default exists nosuch} m; puts "11:$m :: $errorCode"
array default set fresh 5
puts "12:[array exists fresh] [info exists fresh] [array size fresh] [array default get fresh]"
catch {set fresh 1} m; puts "13:$m"
# ── array default read semantics ────────────────────────────────────
set r {}
array set ary {a 3}
array default set ary 7
lappend r $ary(a) $ary(b) [info exist ary(a)] [info exist ary(b)] [array default get ary]
array default unset ary
lappend r $ary(a) [catch {set ary(b)}]
array default set ary grill
lappend r [array size ary] [info exist ary(x)]
append ary(x) abc
lappend r [array size ary] $ary(x)
array default unset ary
append ary(x) def
append ary(y) ghi
lappend r [array size ary] $ary(x) $ary(y)
array default unset ary
array set ary2 {a 3}
array default set ary2 7
incr ary2(x) 11
lappend r [array size ary2] $ary2(x)
array default unset ary2
array default set ary {x y}
dict lappend ary(p) x z
lappend r [array get ary]
unset -nocomplain ary
array set ary {a 3}
array default set ary 7
apply {{} {
    upvar 1 ary ary ary(c) c
    global r
    lappend r $ary(a) $ary(b) $c
    lappend r [info exist ary(a)] [info exist ary(b)] [info exist c]
}}
lappend r [array default get ary]
puts "14:$r"
# default dies with unset of the array
array set d2 {k 1}
array default set d2 dv
unset d2
array set d2 {k 2}
lappend r15 [catch {array default get d2}]
puts "15:$r15"
# ── array for ───────────────────────────────────────────────────────
catch {array for {k v} c d e {}} m; puts "16:$m :: $errorCode"
catch {array for {k v} {}} m; puts "17:$m :: $errorCode"
catch {array for {k} a {}} m; puts "18:$m :: $errorCode"
catch {array for {k v w} a {}} m; puts "19:$m :: $errorCode"
catch {array for {k v} nosuch {}} m; puts "20:$m :: $errorCode"
array set f1 {b 2 a 1}
set out {}
array for {k v} f1 {lappend out $k=$v}
puts "21:[lsort $out]"
array for {k v} f1 {break}
puts "22:done"
catch {array for {k v} f1 {unset f1(b)}} m; puts "23:$m :: $errorCode"
# ── const face ──────────────────────────────────────────────────────
catch {const} m; puts "24:$m :: $errorCode"
catch {const X} m; puts "25:$m :: $errorCode"
catch {const X 1 2} m; puts "26:$m :: $errorCode"
set Y 1
catch {const Y 2} m; puts "27:$m :: $errorCode"
array set arr {x 1}
catch {const arr 1} m; puts "28:$m :: $errorCode"
catch {const arr(x) 1} m; puts "29:$m :: $errorCode"
const Z 5
catch {const Z 6} m; puts "30:$m"
const Z 7
puts "31:$Z"
catch {set Z 9} m; puts "32:$m :: $errorCode"
catch {unset Z} m; puts "33:$m :: $errorCode"
unset -nocomplain Z
puts "34:$Z"
catch {append Z abc} m; puts "35:$m :: $errorCode"
catch {lappend Z x} m; puts "36:$m :: $errorCode"
catch {incr Z} m; puts "37:$m :: $errorCode"
proc pp {} { const P 1; return $P }
puts "38:[pp]"
proc pq {} { const Q 1; unset Q }
catch {pq} m; puts "39:$m :: $errorCode"
catch {apply {{} {
    const W 1
    set W 2
}}} m; puts "40:$m :: $errorCode"
# const via a variable holding the command name
catch {apply {c {
    $c X2 123
    set X2 gorp
}} const} m; puts "41:$m :: $errorCode"
# write trace does not fire a const value change? (creation-set fires)
set tracefired {}
proc tw {args} { lappend ::tracefired $args }
trace add variable T write tw
const T 1
puts "42:$tracefired"
# ── info constant / info consts ─────────────────────────────────────
catch {info constant} m; puts "43:$m"
catch {info constant foo bar} m; puts "44:$m"
catch {info constant foo} m; puts "45:$m :: $errorCode"
set out2 {}
lappend out2 [lsort [info consts]] [info constant CX]
const CX 1
lappend out2 [lsort [info consts]] [info constant CX]
const CY 2
lappend out2 [lsort [info consts]]
catch {info consts a*} m2
puts "46:$out2 :: $m2"
catch {info consts a b} m; puts "47:$m"
apply {{} {
    const IN 1
    global out3
    lappend out3 [info consts] [info constant IN]
}}
puts "48:$out3"
