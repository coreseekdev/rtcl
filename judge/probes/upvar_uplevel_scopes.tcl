proc t {label script} { set c [catch {uplevel 1 $script} r]; set ec [catch {set ::errorCode} e2]; puts "$label|$c|$r|EC=$e2" }
t l1 {apply {{} { upvar a b c }}}
t l2 {apply {{} { upvar 0 x }}}
t l3 {apply {{} { upvar 1 }}}
t l4 {apply {{} { uplevel 1 }}}
t l5 {apply {{} { uplevel }}}
t l6 {uplevel}
t l7 {uplevel 9 {}}
t l8 {uplevel 0 {}}
t l9 {uplevel #0 {set zz1 5}}
t l10 {apply {{} { uplevel #-1 {} }}}
t l11 {apply {{} { uplevel #[expr -1] {} }}}
t l12 {apply {{} { uplevel #0xffffffff {} }}}
t l13 {apply {{} { uplevel #0.2 {} }}}
t l14 {apply {{} { uplevel #[expr 0.2] {} }}}
t l15 {apply {{} { uplevel .2 {} }}}
t l16 {apply {{} { uplevel #.2 {} }}}
t l17 {apply {{} { uplevel #[expr .2] {} }}}
t l18 {uplevel #0 { uplevel { set y 222 } }}
t l19 {apply {{} { uplevel 2 {} }}}
t l20 {uplevel set q1 7; puts "q1=[set q1]"}
t l21 {apply {{} { set r {}; uplevel 0 { lappend r in0 }; lappend r $r }}}
t l22 {namespace eval nx { variable nv 3; proc tp {} { upvar nv v; set v 9 }; tp; puts "nv=$nv" }}
t l23 {namespace eval ny { variable nw 1; namespace eval nz { upvar nw w; set w 2 }; puts "nw=$nw" }}
t l24 {apply {{a b} { upvar 1 a x b y; set x 1; set y 2; list $a $b }} 10 20}
t l25 {proc outer {} { set s1 A; inner; return $s1 }; proc inner {} { upvar s1 z; set z B }; outer}
t l26 {upvar #1 x y}
t l27 {apply {{} { uplevel 1 {} }}}
t l28 {uplevel #1 {}}
t l29 {apply {{} { upvar 1 {x} y }}}
t l30 {apply {{} { set v1 C; upvar v1 alias1; set alias1 D; set v1 }}}
