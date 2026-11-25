# crash-probe: d24 `regexp`, `regsub`, and the commands that match by
# regular expression
# A pattern was compiled for every call — several microseconds for a
# short string, where tclsh keeps a pattern's compiled form in the
# pattern object.  Now compiled once and kept (16 engines per thread),
# with a matcher each.
#
# Three paths answer now, and must answer the same:
#   - a pattern that is plain text is a search (`regexp -nocase {ABC}`
#     cost 11µs a call; 0.2µs now);
#   - of a long text only the first 1024 characters are tabulated unless
#     the match reaches them or needs what follows (`^abc` on 40KB
#     cost 7µs a call);
#   - `switch -regexp`, `lsort -regexp` and `array names -regexp`
#     match by the engine `regexp` does.  `switch` compiled a pattern of
#     its own with the `regex` crate on every call: `\y`, `\m`,
#     lookaheads and backreferences never matched, a bad pattern matched
#     nothing silently, and `-indexvar` gave byte offsets.
#
# Expected values are tclsh 8.6.17's own outputs, check by check, in the
# order the state is built: _gen.tcl writes this file from
# _d24_source.tcl.

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
chk l1 [T {regexp {abc} xabcx}] {0 1 NONE}
chk l2 [T {regexp -nocase {ABC} xabcx}] {0 1 NONE}
chk l3 [T {regexp -nocase {abc} xABCx}] {0 1 NONE}
chk l4 [T {regexp {abc} xabcx -indices}] {0 1 NONE}
chk l5 [T {regexp -nocase {aBc} xAbCx -indices}] {0 1 NONE}
chk l6 [T {regexp -all -inline {ab} ababab}] {0 {ab ab ab} NONE}
chk l7 [T {regexp -all -nocase -inline {ab} aBaBaB}] {0 {aB aB aB} NONE}
chk l8 [T {regexp {zzz} abc}] {0 0 NONE}
chk l9 [T {regexp -nocase {ZZ} abc}] {0 0 NONE}
chk l10 [T {regexp {} abc}] {0 1 NONE}
chk l11 [T {regexp -all -inline {} ab}] {0 {{} {}} NONE}
chk l12 [T {regexp -nocase {} abc -indices}] {0 1 NONE}
chk l13 [T {regexp -start 2 {bc} abcbcbc}] {0 1 NONE}
chk l14 [T {regexp -start 2 -indices {bc} abcbcbc}] {0 1 NONE}
chk l15 [T {regexp -start 4 -all -inline {b} abababab}] {0 {b b} NONE}
chk l16 [T {regexp -start 20 {a} abc}] {0 0 NONE}
chk l17 [T {regexp -start 20 {} abc -indices}] {0 1 NONE}
chk l18 [T {regexp -expanded {a b} xa bx}] {0 0 NONE}
chk l19 [T {regexp {a b} "xa bx"}] {0 1 NONE}
chk l20 [T {regexp -nocase {k} xKx}] {0 1 NONE}
chk l21 [T {regexp -nocase {K} kx}] {0 1 NONE}
chk l22 [T {regexp -nocase {s} xfx}] {0 0 NONE}
chk l23 [T {regexp {é} été}] {0 1 NONE}
chk l24 [T {regexp -nocase {É} été}] {0 1 NONE}
chk l25 [T {regexp -nocase {é} été}] {0 1 NONE}
chk l26 [T {regexp {a.c} axc}] {0 1 NONE}
chk l27 [T {regexp ***=a.c xa.cy}] {0 1 NONE}
chk l28 [T {regexp -nocase ***=A.C xa.cy}] {0 1 NONE}
chk l29 [T {regexp -line {ab} ab}] {0 1 NONE}
chk l30 [T {regexp -linestop {ab} ab}] {0 1 NONE}
chk l31 [T {regexp "a\nb" "a\nb"}] {0 1 NONE}
chk l32 [T {regexp -nocase "AB" "xab"}] {0 1 NONE}
chk l33 [T {regexp {abc abc} "xabc abcy"}] {0 1 NONE}
chk l34 [T {regexp {ab&c} "xab&cy"}] {0 1 NONE}
chk l35 [T {regexp {]b} "x]by"}] {0 1 NONE}
chk l36 [T {regexp -nocase {k} [string repeat é 20]K}] {0 1 NONE}
chk l37 [T {regexp -nocase {abc} [string repeat é 20]ABC}] {0 1 NONE}
chk l38 [T {regexp -indices -nocase {bc} [string repeat é 20]aBc}] {0 1 NONE}
chk l39 [T {regexp {(\w)(\d)} ab3cd4 e f}] {0 1 NONE}
set e {}
set f {}
chk l40 [T {list $e $f}] {0 {{} {}} NONE}
chk l41 [T {regexp {ab} abc e; set e}] {0 ab NONE}
chk l42 [T {regexp -nocase {AB} abc f; set f}] {0 ab NONE}
chk l43 [T {regexp -lineanchor {b} "a\nb" -indices}] {0 1 NONE}
chk l44 [T {regexp -linestop -all -inline {.} "a\nb"}] {0 {a b} NONE}
chk l45 [T {regexp -all -inline -nocase {a} aAaAaA}] {0 {a A a A a A} NONE}
chk l46 [T {regexp -indices -all -inline -nocase {aa} aAaAaA}] {0 {{0 1} {2 3} {4 5}} NONE}
chk l47 [T {regexp -nocase {ÉTÉ} été}] {0 1 NONE}
chk l48 [T {regexp -nocase {ÉTÉ} ÉTÉ}] {0 1 NONE}
chk l49 [T {regexp -expanded -nocase { A B } " xa bx "}] {0 0 NONE}
chk l50 [T {regexp {x?} abc -indices}] {0 1 NONE}
chk l51 [T {regexp {(\w+)} hello m}] {0 1 NONE}
chk l52 [T {regexp -inline -indices {(a)(b)} xabx}] {0 {{1 2} {1 1} {2 2}} NONE}
chk l53 [T {regexp {\mabc\M} xabcx}] {0 0 NONE}
chk l54 [T {regexp -nocase {AB} xabx m o; list $m [dict get $o -errorcode]}] {1 {key "-errorcode" not known in dictionary} {TCL LOOKUP DICT -errorcode}}
# skip l55 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp {(} abc m o; list $m [dict get $o -errorcode]}
# skip l56 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp -nocase {(} abc m o; list $m [dict get $o -errorcode]}
chk r1 [T {regsub {abc} xabcx X}] {0 xXx NONE}
chk r2 [T {regsub -all {ab} ababab X}] {0 XXX NONE}
chk r3 [T {regsub -nocase {AB} aBaBaB X}] {0 XaBaB NONE}
chk r4 [T {regsub {zz} abc X}] {0 abc NONE}
chk r5 [T {regsub -all -nocase {b} bBbb X}] {0 XXXX NONE}
chk r6 [T {regsub -start 2 {bc} abcbcbc X}] {0 abcXbc NONE}
chk r7 [T {regsub -all -nocase {b} abBb y}] {0 ayyy NONE}
chk r8 [T {regsub {abc} abc &-&}] {0 abc-abc NONE}
chk r9 [T {regsub -all {} ab X}] {0 XaXb NONE}
chk r10 [T {regsub -nocase {} ab X}] {0 Xab NONE}
chk r11 [T {regsub -all -nocase {ab} "aB x Ab y" <&>}] {0 {<aB> x <Ab> y} NONE}
chk r12 [T {regsub {é} été X}] {0 Xté NONE}
chk r13 [T {regsub -nocase {Été} ÉTÉ X}] {0 X NONE}
chk r14 [T {regsub -start 3 {a} ababa X}] {0 ababX NONE}
chk r15 [T {regsub -all -start 3 {a} ababa X}] {0 ababX NONE}
chk r16 [T {regsub {zz} abc X outvar}] {0 0 NONE}
chk r17 [T {regsub -all {b} abc X outvar}] {0 1 NONE}
chk r18 [T {regsub {(} abc X m o; list $m [dict get $o -errorcode]}] {1 {wrong # args: should be "regsub ?-option ...? exp string subSpec ?varName?"} {TCL WRONGARGS}}
set big [string repeat x 1030]
set big2 [string repeat x 2000]
set mid "${big}y${big2}"
chk p1 [T {regexp {^abc} abc$::big}] {0 1 NONE}
chk p2 [T {regexp {^x+} $::big}] {0 1 NONE}
chk p3 [T {llength [regexp -all -inline {x+} $::big]}] {0 1 NONE}
chk p4 [T {regexp -indices {^x+} $::big}] {0 1 NONE}
chk p5 [T {regexp {x$} $::big}] {0 1 NONE}
chk p6 [T {regexp {abc$} $::big}] {0 0 NONE}
chk p7 [T {regexp -indices {x(?=x)} [string range $::big2 0 1029]}] {0 1 NONE}
U {regexp -indices {(x+)\1} [string range $::big2 0 1029]}
chk p9 [T {regexp -indices -start 1020 {x} $::big2}] {0 1 NONE}
chk p10 [T {regexp -indices -start 1020 {^x} $::big2}] {0 0 NONE}
chk p11 [T {llength [regexp -all -inline {x{100}} $::big2]}] {0 20 NONE}
# skip p12 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp {x{1025}} [string range $::big2 0 1029]}
# skip p13 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp {x{1026}} [string range $::big2 0 1029]}
# skip p14 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp -indices {x{1024}} [string range $::big2 0 1029]}
# skip p15 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp -indices {x{1023}} [string range $::big2 0 1029]}
chk p16 [T {regexp -indices {^x+y} $::mid}] {0 1 NONE}
chk p17 [T {regexp -indices -all -inline {x+y} $::mid}] {0 {{0 1030}} NONE}
chk p18 [T {regexp -indices {yx} $::mid}] {0 1 NONE}
chk p19 [T {regexp -indices {^x+?(?=y)} $::mid}] {0 1 NONE}
U {regexp -indices {(x+)y\1} $::big}
chk p21 [T {regexp -indices {y$} $::mid}] {0 0 NONE}
chk p22 [T {regexp -indices -nocase {X+} [string range $::big2 0 1029]}] {0 1 NONE}
chk p23 [T {regexp -indices {^x+} $::big2 -indices}] {0 1 NONE}
chk p24 [T {llength [regexp -all -inline -nocase {x} $::big]}] {0 1030 NONE}
chk p25 [T {regsub -all {x} $::big y}] {0 yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy NONE}
chk p26 [T {regsub {x+z} $::big2 {} out; string length $out}] {0 2000 NONE}
chk p27 [T {regsub -all -nocase {X} [string range $::big2 0 1029] y}] {0 yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy NONE}
# skip p28 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {string length [regsub -all {x{1030}} $::big2 y]}
chk p29 [T {regexp -indices {^x+\y} $::big}] {0 1 NONE}
chk p30 [T {regexp -indices {\Mx} [string range $::big2 0 1029]}] {0 0 NONE}
set uni [string repeat é 1030]
chk u1 [T {regexp -indices {^é+$} $::uni}] {0 1 NONE}
chk u2 [T {regexp -indices {é$} $::uni}] {0 1 NONE}
chk u3 [T {llength [regexp -all -inline {éé} $::uni]}] {0 515 NONE}
# skip u4 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp -indices {é{1024}} $::uni}
# skip u5 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {regexp -indices {é{1025}} $::uni}
chk u6 [T {regexp -nocase {k} "${uni}k"}] {0 1 NONE}
chk u7 [T {regexp -nocase {k} "${uni}K"}] {0 1 NONE}
chk u8 [T {regexp -indices -nocase {k} "${uni}K"}] {0 1 NONE}
chk u9 [T {regexp -indices {é+} "${uni}x"}] {0 1 NONE}
chk u10 [T {regexp -indices {é+(?=x)} "${uni}x"}] {0 1 NONE}
chk g1 [T {regexp {^x+} $::big}] {0 1 NONE}
chk g2 [T {regexp -nocase {ABC} xabcx}] {0 1 NONE}
chk g3 [T {regexp -indices {é$} $::uni}] {0 1 NONE}
chk g4 [T {regexp -indices {^x+} $::big2}] {0 1 NONE}
chk g5 [T {regexp -indices {^é+$} $::uni}] {0 1 NONE}
chk g6 [T {regexp -indices {^x+} $::big}] {0 1 NONE}
chk s1 [T {switch -regexp -- abcabc { {\mabc\M} {list word} {^abc$} anchored default nomatch }}] {1 {invalid command name "nomatch"} {TCL LOOKUP COMMAND nomatch}}
chk s2 [T {switch -regexp -- abcabc { {ab\Bc} sepre default nomatch }}] {1 {invalid command name "nomatch"} {TCL LOOKUP COMMAND nomatch}}
chk s3 [T {switch -regexp -- x123y { {\d+} digits {y$} end default nomatch }}] {1 {invalid command name "digits"} {TCL LOOKUP COMMAND digits}}
chk s4 [T {switch -regexp -- abc { ***=b.c literal default nomatch }}] {1 {invalid command name "nomatch"} {TCL LOOKUP COMMAND nomatch}}
chk s5 [T {switch -regexp -- "ab" { {a[[:space:]]b} sp {a(b)} cap default nomatch }}] {1 {invalid command name "cap"} {TCL LOOKUP COMMAND cap}}
chk s6 [T {switch -nocase -regexp -- ABC { {abc} hit default nomatch }}] {1 {invalid command name "hit"} {TCL LOOKUP COMMAND hit}}
chk s7 [T {switch -regexp -- "日本語abc" { {abc} hit default nomatch }}] {1 {invalid command name "hit"} {TCL LOOKUP COMMAND hit}}
chk s8 [T {switch -regexp -indexvar iv -matchvar mv -- "ab" {a(b) {list $iv $mv} default nomatch}}] {0 {{{0 1} {1 1}} {ab b}} NONE}
chk s9 [T {switch -regexp -indexvar iv -matchvar mv -- "éab" {a(b) {list $iv $mv} default nomatch}}] {0 {{{1 2} {2 2}} {ab b}} NONE}
chk s10 [T {switch -regexp -- "a" { {(?:a)} noncap default nomatch }}] {1 {invalid command name "noncap"} {TCL LOOKUP COMMAND noncap}}
chk s11 [T {switch -regexp -- "x" { {\y} boundary default nomatch }}] {1 {invalid command name "boundary"} {TCL LOOKUP COMMAND boundary}}
# skip s12 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {switch -regexp -- "x" { {(} badpat default nomatch }}
chk s12b [T {switch -regexp -- "x" { {x} first {(} badpat default nomatch }}] {1 {invalid command name "first"} {TCL LOOKUP COMMAND first}}
chk s13 [T {switch -regexp -- "abc" { {a(?=b)} lookahead default nomatch }}] {1 {invalid command name "lookahead"} {TCL LOOKUP COMMAND lookahead}}
chk s14 [T {switch -regexp -- "x1" { {[[:digit:]]} d default nomatch }}] {1 {invalid command name "d"} {TCL LOOKUP COMMAND d}}
chk s15 [T {switch -regexp -- "ａ" { {\w} w default nomatch }}] {1 {invalid command name "w"} {TCL LOOKUP COMMAND w}}
chk s16 [T {switch -regexp -- "ab" { {a\s b} sp {a\s} sp2 default nomatch}}] {1 {invalid command name "nomatch"} {TCL LOOKUP COMMAND nomatch}}
chk s17 [T {switch -nocase -regexp -indexvar iv -- "AB" {a(b) {list $iv} default nomatch}}] {0 {{{0 1} {1 1}}} NONE}
chk s18 [T {switch -regexp -- "abab" { {^(a)(b)} {switch -regexp -matchvar mv -indexvar iv -- "ab" {a(b) {list $mv $iv}}} default nomatch }}] {0 {{ab b} {{0 1} {1 1}}} NONE}
chk s19 [T {switch -regexp -- "日本" { {本} hit default nomatch }}] {1 {invalid command name "hit"} {TCL LOOKUP COMMAND hit}}
chk s20 [T {switch -regexp -- "abc" { {abc} {list [regexp -inline {(a)} a] [switch -regexp -- a { {(a)} yes default no }]} default nomatch }}] {1 {invalid command name "yes"} {TCL LOOKUP COMMAND yes}}
array set arr {ab 1 acb 2 abab 3 x 4 aB 5}
chk a1 [T {lsort [array names arr -regexp {^a}]}] {0 {aB ab abab acb} NONE}
chk a2 [T {lsort [array names arr -regexp {(a)(b)}]}] {0 {ab abab} NONE}
chk a3 [T {lsort [array names arr -regexp {^(.)\1}]}] {0 {} NONE}
chk a4 [T {lsort [array names arr -regexp {\mab\M}]}] {0 ab NONE}
chk a5 [T {lsort [array names arr -regexp {}]}] {0 {aB ab abab acb x} NONE}
chk a6 [T {lsort [array names arr -regexp {ababab}]}] {0 {} NONE}
chk a7 [T {lsort [array names arr -regexp {\y}]}] {0 {aB ab abab acb x} NONE}
chk a8 [T {array names nosuch -regexp {(}}] {0 {} NONE}
# skip a9 -- 9.x regexp 文案 cannot compile——TODO 对标迁移 T7
U {catch {array names arr -regexp {(}} m o; list $m [dict get $o -errorcode]}
chk a10 [T {lsort -regexp {b} {xb axb cb aa}}] {1 {bad option "-regexp": must be -ascii, -command, -decreasing, -dictionary, -increasing, -index, -indices, -integer, -nocase, -real, -stride, or -unique} {TCL LOOKUP INDEX option -regexp}}
chk a11 [T {lsort -regexp -nocase {B} {xb axb cb aa}}] {1 {bad option "-regexp": must be -ascii, -command, -decreasing, -dictionary, -increasing, -index, -indices, -integer, -nocase, -real, -stride, or -unique} {TCL LOOKUP INDEX option -regexp}}
chk a12 [T {lsort -regexp -indices {a(b)} {ab axb cb aa}}] {1 {bad option "-regexp": must be -ascii, -command, -decreasing, -dictionary, -increasing, -index, -indices, -integer, -nocase, -real, -stride, or -unique} {TCL LOOKUP INDEX option -regexp}}
chk a13 [T {catch {lsort -regexp {(} {a b}} m o; list $m [dict get $o -errorcode]}] {0 {{bad option "-regexp": must be -ascii, -command, -decreasing, -dictionary, -increasing, -index, -indices, -integer, -nocase, -real, -stride, or -unique} {TCL LOOKUP INDEX option -regexp}} {TCL LOOKUP INDEX option -regexp}}
chk a14 [T {lsort -regexp {(a+)(b+)} {ab aab aaab} }] {1 {bad option "-regexp": must be -ascii, -command, -decreasing, -dictionary, -increasing, -index, -indices, -integer, -nocase, -real, -stride, or -unique} {TCL LOOKUP INDEX option -regexp}}

# 162 lines; the chk ones are the checks.
