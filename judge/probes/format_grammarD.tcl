proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>|[string length $r]" }
}
t pct.arg {format {%5%} x}
t pct.arg2 {format {%%%} x}
t c.big {format %c 1114112}
t c.big.bytes {binary encode hex [format %c 1114112]}
t hs {format {%hs} ab}
t hhd {format {%hhd} 5}
t hh {format {%hh} 5}
t o.hash0 {format {%#o} 0}
t x.hash0 {format {%#X} 0}
t utf.w {format {%4s} éé}
t utf.p {format {%.2s} ééé}
t utf.c {format %c 233}
t utf.cw {format {%3c} 233}
t s.prec0 {format {%.0s} abc}
t s.wide.prec {format {%6.2s} abcdef}
t d.plus.minus {format {%-+5d} 42}
t xpg.plus {format {%1$+d} 5}
t xpg.width {format {%2$5d} 1 2}
t xpg.star {format {%1$*2$d} 42 6}
t i.big {format %d 99999999999999999999}
t e.prec {format %.2e 12345}
t g.x {format %g 0.0001}
t g.y {format %g 0.00001}
t ll.x {format {%llx} 305419896}
