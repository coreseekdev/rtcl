proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>" }
}
t xpg.mix1 {format {%d %1$d} 1 2}
t xpg.mix2 {format {%1$d %d} 1 2}
t xpg.oob {format {%5$d} x}
t xpg.oob2 {format {%2$d} 1}
t xpg.ok {format {%2$s-%1$s} a b}
t xpg.d {format {%2$d %1$x} 10 20}
t star.negw {format {%*d} -5 42}
t star.negp {format {%.*f} -2 3.14159}
t star.p {format {%.*f} 2 3.14159}
t star.wz {format {%*d} 5 42}
t star.both {format {%*.*f} 8 2 3.14159}
t star.nonint {format {%5d} x}
t h.d {format %hd 65535}
t h.neg {format %hd -65536}
t h.x {format %hx -2}
t h.u {format %hu 70000}
t h.l {format %ld 5}
t h.ll {format %lld 5}
t h.only {format %l 5}
t h.only2 {format %h 5}
t z.s {format {%5%}}
t b.hash {format {%#b} 5}
t o.hash {format {%#o} 5}
t x.hash {format {%#x} 0}
t s.zero {format {%05s} ab}
t s.zeroneg {format {%-05s} ab}
t c.w {format {%5c} 65}
t f.hash0 {format {%#.0f} 9.5}
t f.hash {format {%#f} 9.5}
t e.hash0 {format {%#.0e} 9.5}
t g.hash0 {format {%#.0g} 9.5}
t d.wide {format %d 0x10}
t d.oct {format %d 010}
t i.hex {format %i 0x10}
t u.neg {format %u -1}
t c.big {format %c 1114112}
t pct.mid {format {%%d} 5}
t trail.digit {format {%3} 5}
t trail.dot {format {%.} 5}
t trail.flag {format {%-} 5}
t empty.spec {format {%}}
t d.str {format %d 2a}
t d.float {format %d 3.7}
t c.str {format %c abc}
t x.str {format %x zz}
