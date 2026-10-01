proc t {label script} { set c [catch {uplevel 1 $script} r]; set ec [catch {set ::errorCode} e2]; puts "$label|$c|$r|EC=$e2" }
t u1 {upvar a b c}
t u2 {upvar 0 x}
t u3 {upvar 1}
t u4 {upvar}
t u5 {upvar 99 x y}
t u6 {upvar #-1 x y}
t u7 {upvar #1 x y}
t u8 {upvar #0.2 x y}
t u9 {upvar #x y z}
t u10 {upvar # y z}
t u11 {upvar 01 x y}
t u12 {upvar 1a x y}
t u13 {upvar 2.5 x y}
t u14 {upvar +2 x y}
t u15 {upvar 0.2 x y}
t u16 {upvar -1 x y}
t u17 {upvar 0xffffffff x y}
t u18 {upvar #010 x y}
t u19 {upvar #0xffffffff x y}
t u20 {apply {{} { upvar 0 b b }}}
t u21 {upvar 0 zz zz}
t u22 {apply {{} { set a 33; upvar b a }}}
t u23 {apply {{} { trace add variable a write foo; upvar b a }}}
t u24 {apply {{} { trace add variable a(1) write foo; upvar b a }}}
t u25 {apply {{} { upvar 0 a b; upvar 0 b a }}}
t u26 {set w1 5}
t u27 {apply {{} { upvar 2 x y }}}
t u28 {upvar #0}
t u29 {upvar + x y}
t u30 {upvar 0x x y}
t u31 {upvar - x y}
t u32 {upvar +0 x y}
