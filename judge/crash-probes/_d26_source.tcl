# Source of judge/crash-probes/d26_append.tcl: `append` (and the other
# in-place variable writes: `lappend`, `dict set`, `incr`) and the order
# `info globals` / `info vars` list in.  `_gen.tcl` runs every line under
# tclsh and writes the probe with tclsh's outputs as the expected values.
#
# `append` rebuilt the whole string on every call — two copies of it —
# and is now an append to the variable's own buffer when nothing else
# holds the value.  The answers must not depend on the road taken: plain
# strings, values with another representation, the variable handed to
# itself, arrays, links, traces of every kind.
#
# A variable made again after `unset` takes a new place in the table
# (the head of its bucket); a write to a variable that exists keeps its
# place.  The listing used to keep the first place for both, and a write
# made by taking the value out and putting it back used to move the
# variable.
#
# Left out (rtcl differs already, for other reasons):
#   t9, t10 — the error options of a failed array write trace (the
#             `-errorstack` names the command another way);
#   t11, t12 — tclsh compiles `append` with simple words into one write
#             per value, so a write trace fires once per value; rtcl
#             fires it once, at the end;
#   o9 — inside a proc, locals made under a computed name are listed in
#        another order (the compiled locals come first in tclsh).
#   o13 — `info vars` inside a namespace lists that namespace's variables
#         and the globals, and rtcl's own start-up variables differ.
# ── plain strings ───────────────────────────────────────────────────
T a1 {set x abc; append x}
T a2 {set x abc; append x 1 2 3}
T a3 {set x 123; append x 4}
T a4 {set x [expr {1 + 2}]; append x .0}
T a5 {set x [list a b]; append x c}
T a6 {set x [dict create a 1]; append x b}
T a7 {append y1 x}
T a8 {set e ""; append e x}
T a9 {append}
T a10 {set x ""; append x ""}
T a11 {append y2 x y z; set y2}
T a12 {set x 1; append x}
T a13 {set x 1; append x [list q r]}
T a14 {set x 1; append x "é日"}
T a15 {set x "é日"; append x x}
T a16 {set x [format %.3f 1.5]; append x !}
T a17 {set x [expr {wide(1) << 62}]; append x 0}
T a18 {set x [expr {2.0 * 3}]; append x 0}
T a19 {set x [expr {1 == 1}]; append x 0}

# ── the variable given to itself, and copies ────────────────────────
T s1 {set x abc; append x $x}
T s2 {set x abc; append x $x $x x}
T s3 {set x abc; set y $x; append x d; list $x $y}
T s4 {set x abc; set y $x; append y d; list $x $y}
T s5 {set x abc; set l [list $x]; append x d; list $x $l}
T s6 {set x abc; dict set d k $x; append x d; list $x $d}
T s7 {set x abc; set y $x$z; append x d; list $x [info exists z]}
T s8 {set x ""; set y $x; append x d; list $x $y}

# ── arrays ──────────────────────────────────────────────────────────
T r1 {set arr(1) a; append arr(1) b}
# skip r2 -- the errorCode of a write to an array as a scalar is not set
T r2 {set arr(1) a; append arr 1}
T r3 {array set arr {1 a}; append arr(2) b; array get arr}
T r4 {set arr(1) a; append arr(x) y; array get arr}
T r5 {append newarr(k) v; array get newarr}
T r6 {set arr(1) a; append arr(1) $arr(1); set arr(1)}
T r7 {array set arr {1 a}; append arr(1) b c; array get arr}
T r8 {array set arr {A a B b}; append arr(A) x; array names arr}
T r9 {array set arr {A 1}; unset arr(A); set arr(A) 2; append arr(A) 3; array names arr}

# ── links ───────────────────────────────────────────────────────────
set ::g 1
proc lg1 {} {global g; append g 2}
T l1 {lg1; set ::g}
proc lg2 {} {upvar #0 g gg; append gg 3}
T l2 {lg2; set ::g}
namespace eval ns { variable v 1 }
proc lg3 {} {variable v; append v 2}
T l3 {namespace eval ns { lg3 }; set ns::v}
proc lg4 {} { upvar 1 x xx ; append xx 9 }
T l4 {set x 5; lg4; set x}
proc lg5 {} { global gone; catch {append gone 1} m e; list $m $e [info exists ::gone] }
T l5 {lg5}
proc lg6 {} { upvar #0 nope n; catch {append n 1} m e; list $m $e [info exists ::nope] }
T l6 {lg6}
T l7 {append ::g 7; set ::g}
T l8 {append ns::v 4; set ns::v}
T l9 {append nosuchns::v 1}
proc lg7 {} { upvar 1 arr a; append a(1) z }
T l10 {array set arr {1 a}; lg7; array get arr}
proc lg8 {} { variable v; append v 5; append v 6; set v }
T l11 {namespace eval ns { lg8 }; set ns::v}
proc lg9 {} { global g; append g $g; set g }
T l12 {lg9; set ::g}
proc lg10 {} { set x a; upvar 0 x y; append y b; list $x $y }
T l13 {lg10}

# ── traces ──────────────────────────────────────────────────────────
set tv 1
set ::log {}
proc tw {name1 name2 op} { global log ; lappend log [list $name1 $name2 $op] }
trace add variable tv write tw
T t1 {append tv 2}
T t2 {set ::log}
set ::log {}
proc tr {name1 name2 op} { global log ; lappend log [list r $name1 $op] }
trace add variable tv read tr
T t3 {append tv 3}
T t4 {set ::log}
trace remove variable tv read tr
trace remove variable tv write tw
proc wr {name1 name2 op} { uplevel 1 [list set $name1 traced] }
trace add variable tv write wr
T t5 {set tv 1; append tv 2}
T t6 {set ::tv}
trace remove variable tv write wr
proc ra {name1 name2 op} { return -code error "no read" }
trace add variable tv read ra
T t7 {set tv 1; catch {append tv 2} m e; list $m $e $tv}
trace remove variable tv read ra
proc wa {args} { return -code error "no write" }
trace add variable tv write wa
T t8 {set tv 1; catch {append tv 2} m e; list $m $e $tv}
trace remove variable tv write wa
array set ta {1 a}
trace add variable ta write wa
# skip t9 -- the error options of a failed array write trace differ
T t9 {catch {append ta(1) 2} m e; list $m $e}
trace remove variable ta write wa
trace add variable ta(1) write wa
# skip t10 -- same as t9
T t10 {catch {append ta(1) 2} m e; list $m $e}
trace remove variable ta(1) write wa
set ::log {}
trace add variable tv write tw
# skip t11 t12 -- one write per value under tclsh's compiled append
T t11 {append tv x y z}
T t12 {set ::log}
trace remove variable tv write tw
set ::log {}
proc tu {name1 name2 op} { global log ; lappend log [list u $name1 $op] }
trace add variable tv unset tu
T t13 {unset tv; set ::log}
trace remove variable tv unset tu
proc trw {name1 name2 op} { uplevel 1 [list set $name1 A]; return [set $name1]B }
trace add variable tv write trw
T t14 {set tv 1; append tv 2}
trace remove variable tv write trw

# ── loop variables ──────────────────────────────────────────────────
T f1 {foreach v {1 2 3} { append acc $v }; set acc}
T f2 {for {set i 0} {$i < 3} {incr i} { append s $i }; set s}
T f3 {foreach {a b} {1 2 3 4} { append p $a$b }; set p}
proc fl {} {
    set s ""
    foreach v {x y z} { append s $v }
    for {set i 0} {$i < 4} {incr i} { append s $i }
    set l {}
    foreach v {x y z} { lappend l $v }
    return [list $s $l]
}
T f4 {fl}

# ── the order of a listing ──────────────────────────────────────────
# Names 32 apart share a bucket of a 4-bucket table.
foreach n {A a B b C c D d} { set ::$n 1 }
T o1 {info globals {[-a-dA-D]}}
append A 2
append C 2
T o2 {info globals {[-a-dA-D]}}
T o3 {info vars {[-a-dA-D]}}
lappend L1 1
lappend L2 2
dict set DS k 1
T o4 {list [info globals L*] [info globals DS] [info vars {[-a-dA-D]}] [info globals {[-a-dA-D]}]}
unset B
set B 1
T o5 {info globals {[-a-dA-D]}}
incr a
T o6 {info globals {[-a-dA-D]}}
unset a
set a 1
T o7 {info globals {[-a-dA-D]}}
unset C
set C 1
append C 9
incr C
lappend C 7
dict set C k 2
T o8 {info globals {[-a-dA-D]}}
proc ol {} {
    foreach n {A a B b C c} { set $n 1 }
    set r [info locals]
    lappend r [info vars]
    unset A
    set A 2
    lappend r [info locals] [info vars]
    return $r
}
# skip o9 -- see the top of this file
T o9 {ol}
namespace eval nso { variable A 1; variable a 1; variable B 1; variable b 1 }
T o10 {info vars nso::*}
namespace eval nso { unset A; set A 2 }
T o11 {info vars nso::*}
T o12 {info vars ::nso::*}
# skip o13 -- see the top of this file
T o13 {namespace eval nso { info vars }}
T o14 {namespace eval nso { info vars A }}

# ── loops at the top level and in a proc ────────────────────────────
set top ""
for {set i 0} {$i < 50} {incr i} { append top $i }
T p1 {string length $top}
set tl {}
for {set i 0} {$i < 50} {incr i} { lappend tl $i }
T p2 {llength $tl}
proc pl {n} {
    set s ""
    set l {}
    set m [dict create]
    for {set i 0} {$i < $n} {incr i} {
        append s $i
        lappend l $i
        dict set m k$i $i
    }
    return [list [string length $s] [llength $l] [dict size $m] [lindex $l end] [dict get $m k0]]
}
T p3 {pl 200}
T p4 {pl 2000}
proc pg {n} {
    global gs
    set gs ""
    upvar #0 gl gl
    set gl {}
    for {set i 0} {$i < $n} {incr i} { append gs x; lappend gl x }
    return [list [string length $gs] [llength $gl]]
}
T p5 {pg 300}
puts DONE
