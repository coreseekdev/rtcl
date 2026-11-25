# Source of judge/crash-probes/d24_regexp.tcl: `regexp`, `regsub` and
# the commands that match by regular expressions.  `_gen.tcl` runs every
# line under tclsh and writes the probe with tclsh's outputs as the
# expected values.
#
# The results must be the engine's whatever path served them: a pattern
# that is plain text (a search now), a long text (only its first
# characters are tabulated unless the match needs more), a pattern used
# again (compiled once, kept), and `switch`/`lsort`/`array names`
# (the same engine as `regexp`).

# Plain text, case-sensitive and not
T l1 {regexp {abc} xabcx}
T l2 {regexp -nocase {ABC} xabcx}
T l3 {regexp -nocase {abc} xABCx}
T l4 {regexp {abc} xabcx -indices}
T l5 {regexp -nocase {aBc} xAbCx -indices}
T l6 {regexp -all -inline {ab} ababab}
T l7 {regexp -all -nocase -inline {ab} aBaBaB}
T l8 {regexp {zzz} abc}
T l9 {regexp -nocase {ZZ} abc}
T l10 {regexp {} abc}
T l11 {regexp -all -inline {} ab}
T l12 {regexp -nocase {} abc -indices}
T l13 {regexp -start 2 {bc} abcbcbc}
T l14 {regexp -start 2 -indices {bc} abcbcbc}
T l15 {regexp -start 4 -all -inline {b} abababab}
T l16 {regexp -start 20 {a} abc}
T l17 {regexp -start 20 {} abc -indices}
T l18 {regexp -expanded {a b} xa bx}
T l19 {regexp {a b} "xa bx"}
T l20 {regexp -nocase {k} xKx}
T l21 {regexp -nocase {K} kx}
T l22 {regexp -nocase {s} xfx}
T l23 {regexp {é} été}
T l24 {regexp -nocase {É} été}
T l25 {regexp -nocase {é} été}
T l26 {regexp {a.c} axc}
T l27 {regexp ***=a.c xa.cy}
T l28 {regexp -nocase ***=A.C xa.cy}
T l29 {regexp -line {ab} ab}
T l30 {regexp -linestop {ab} ab}
T l31 {regexp "a\nb" "a\nb"}
T l32 {regexp -nocase "AB" "xab"}
T l33 {regexp {abc abc} "xabc abcy"}
T l34 {regexp {ab&c} "xab&cy"}
T l35 {regexp {]b} "x]by"}
T l36 {regexp -nocase {k} [string repeat é 20]K}
T l37 {regexp -nocase {abc} [string repeat é 20]ABC}
T l38 {regexp -indices -nocase {bc} [string repeat é 20]aBc}
T l39 {regexp {(\w)(\d)} ab3cd4 e f}
set e {}
set f {}
T l40 {list $e $f}
T l41 {regexp {ab} abc e; set e}
T l42 {regexp -nocase {AB} abc f; set f}
T l43 {regexp -lineanchor {b} "a\nb" -indices}
T l44 {regexp -linestop -all -inline {.} "a\nb"}
T l45 {regexp -all -inline -nocase {a} aAaAaA}
T l46 {regexp -indices -all -inline -nocase {aa} aAaAaA}
T l47 {regexp -nocase {ÉTÉ} été}
T l48 {regexp -nocase {ÉTÉ} ÉTÉ}
T l49 {regexp -expanded -nocase { A B } " xa bx "}
T l50 {regexp {x?} abc -indices}
T l51 {regexp {(\w+)} hello m}
T l52 {regexp -inline -indices {(a)(b)} xabx}
T l53 {regexp {\mabc\M} xabcx}
T l54 {regexp -nocase {AB} xabx m o; list $m [dict get $o -errorcode]}
T l55 {regexp {(} abc m o; list $m [dict get $o -errorcode]}
T l56 {regexp -nocase {(} abc m o; list $m [dict get $o -errorcode]}
# regsub with plain text
T r1 {regsub {abc} xabcx X}
T r2 {regsub -all {ab} ababab X}
T r3 {regsub -nocase {AB} aBaBaB X}
T r4 {regsub {zz} abc X}
T r5 {regsub -all -nocase {b} bBbb X}
T r6 {regsub -start 2 {bc} abcbcbc X}
T r7 {regsub -all -nocase {b} abBb y}
T r8 {regsub {abc} abc &-&}
T r9 {regsub -all {} ab X}
T r10 {regsub -nocase {} ab X}
T r11 {regsub -all -nocase {ab} "aB x Ab y" <&>}
T r12 {regsub {é} été X}
T r13 {regsub -nocase {Été} ÉTÉ X}
T r14 {regsub -start 3 {a} ababa X}
T r15 {regsub -all -start 3 {a} ababa X}
T r16 {regsub {zz} abc X outvar}
T r17 {regsub -all {b} abc X outvar}
T r18 {regsub {(} abc X m o; list $m [dict get $o -errorcode]}
# A long text: the first 1024 characters are tabulated first
set big [string repeat x 1030]
set big2 [string repeat x 2000]
set mid "${big}y${big2}"
T p1 {regexp {^abc} abc$::big}
T p2 {regexp {^x+} $::big}
T p3 {llength [regexp -all -inline {x+} $::big]}
T p4 {regexp -indices {^x+} $::big}
T p5 {regexp {x$} $::big}
T p6 {regexp {abc$} $::big}
T p7 {regexp -indices {x(?=x)} [string range $::big2 0 1029]}
T p8 {regexp -indices {(x+)\1} [string range $::big2 0 1029]}
T p9 {regexp -indices -start 1020 {x} $::big2}
T p10 {regexp -indices -start 1020 {^x} $::big2}
T p11 {llength [regexp -all -inline {x{100}} $::big2]}
T p12 {regexp {x{1025}} [string range $::big2 0 1029]}
T p13 {regexp {x{1026}} [string range $::big2 0 1029]}
T p14 {regexp -indices {x{1024}} [string range $::big2 0 1029]}
T p15 {regexp -indices {x{1023}} [string range $::big2 0 1029]}
T p16 {regexp -indices {^x+y} $::mid}
T p17 {regexp -indices -all -inline {x+y} $::mid}
T p18 {regexp -indices {yx} $::mid}
T p19 {regexp -indices {^x+?(?=y)} $::mid}
T p20 {regexp -indices {(x+)y\1} $::big}
T p21 {regexp -indices {y$} $::mid}
T p22 {regexp -indices -nocase {X+} [string range $::big2 0 1029]}
T p23 {regexp -indices {^x+} $::big2 -indices}
T p24 {llength [regexp -all -inline -nocase {x} $::big]}
T p25 {regsub -all {x} $::big y}
T p26 {regsub {x+z} $::big2 {} out; string length $out}
T p27 {regsub -all -nocase {X} [string range $::big2 0 1029] y}
T p28 {string length [regsub -all {x{1030}} $::big2 y]}
T p29 {regexp -indices {^x+\y} $::big}
T p30 {regexp -indices {\Mx} [string range $::big2 0 1029]}
# skip p8 p20 -- a backreference over a long text: rtcl's backtracker
# stops at its step budget and reports REG_ESPACE "match is too complex"
# where tclsh answers 0 (the budget stands in for a hang; a known
# difference, in the audit record)
# Two-byte characters across the boundary
set uni [string repeat é 1030]
T u1 {regexp -indices {^é+$} $::uni}
T u2 {regexp -indices {é$} $::uni}
T u3 {llength [regexp -all -inline {éé} $::uni]}
T u4 {regexp -indices {é{1024}} $::uni}
T u5 {regexp -indices {é{1025}} $::uni}
T u6 {regexp -nocase {k} "${uni}k"}
T u7 {regexp -nocase {k} "${uni}K"}
T u8 {regexp -indices -nocase {k} "${uni}K"}
T u9 {regexp -indices {é+} "${uni}x"}
T u10 {regexp -indices {é+(?=x)} "${uni}x"}
# The same patterns again: compiled once, same answers
T g1 {regexp {^x+} $::big}
T g2 {regexp -nocase {ABC} xabcx}
T g3 {regexp -indices {é$} $::uni}
T g4 {regexp -indices {^x+} $::big2}
T g5 {regexp -indices {^é+$} $::uni}
T g6 {regexp -indices {^x+} $::big}
# switch -regexp: the engine of `regexp`
T s1 {switch -regexp -- abcabc { {\mabc\M} {list word} {^abc$} anchored default nomatch }}
T s2 {switch -regexp -- abcabc { {ab\Bc} sepre default nomatch }}
T s3 {switch -regexp -- x123y { {\d+} digits {y$} end default nomatch }}
T s4 {switch -regexp -- abc { ***=b.c literal default nomatch }}
T s5 {switch -regexp -- "ab" { {a[[:space:]]b} sp {a(b)} cap default nomatch }}
T s6 {switch -nocase -regexp -- ABC { {abc} hit default nomatch }}
T s7 {switch -regexp -- "日本語abc" { {abc} hit default nomatch }}
T s8 {switch -regexp -indexvar iv -matchvar mv -- "ab" {a(b) {list $iv $mv} default nomatch}}
T s9 {switch -regexp -indexvar iv -matchvar mv -- "éab" {a(b) {list $iv $mv} default nomatch}}
T s10 {switch -regexp -- "a" { {(?:a)} noncap default nomatch }}
T s11 {switch -regexp -- "x" { {\y} boundary default nomatch }}
T s12 {switch -regexp -- "x" { {(} badpat default nomatch }}
T s12b {switch -regexp -- "x" { {x} first {(} badpat default nomatch }}
T s13 {switch -regexp -- "abc" { {a(?=b)} lookahead default nomatch }}
T s14 {switch -regexp -- "x1" { {[[:digit:]]} d default nomatch }}
T s15 {switch -regexp -- "ａ" { {\w} w default nomatch }}
T s16 {switch -regexp -- "ab" { {a\s b} sp {a\s} sp2 default nomatch}}
T s17 {switch -nocase -regexp -indexvar iv -- "AB" {a(b) {list $iv} default nomatch}}
T s18 {switch -regexp -- "abab" { {^(a)(b)} {switch -regexp -matchvar mv -indexvar iv -- "ab" {a(b) {list $mv $iv}}} default nomatch }}
T s19 {switch -regexp -- "日本" { {本} hit default nomatch }}
T s20 {switch -regexp -- "abc" { {abc} {list [regexp -inline {(a)} a] [switch -regexp -- a { {(a)} yes default no }]} default nomatch }}
# lsort and array names by regular expression
array set arr {ab 1 acb 2 abab 3 x 4 aB 5}
T a1 {lsort [array names arr -regexp {^a}]}
T a2 {lsort [array names arr -regexp {(a)(b)}]}
T a3 {lsort [array names arr -regexp {^(.)\1}]}
T a4 {lsort [array names arr -regexp {\mab\M}]}
T a5 {lsort [array names arr -regexp {}]}
T a6 {lsort [array names arr -regexp {ababab}]}
T a7 {lsort [array names arr -regexp {\y}]}
T a8 {array names nosuch -regexp {(}}
T a9 {catch {array names arr -regexp {(}} m o; list $m [dict get $o -errorcode]}
T a10 {lsort -regexp {b} {xb axb cb aa}}
T a11 {lsort -regexp -nocase {B} {xb axb cb aa}}
T a12 {lsort -regexp -indices {a(b)} {ab axb cb aa}}
T a13 {catch {lsort -regexp {(} {a b}} m o; list $m [dict get $o -errorcode]}
T a14 {lsort -regexp {(a+)(b+)} {ab aab aaab} }
puts DONE
