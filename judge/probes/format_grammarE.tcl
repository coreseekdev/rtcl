proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>" }
}
t i.big {format %d 99999999999999999999}
t i.big2 {format %u 99999999999999999999}
t i.wide {format %d 0x7fffffffffffffff}
t i.wide2 {format %d 0xffffffffffffffff}
t d.bool {format %d true}
t d.list {format %d {1 2}}
t c.neg {format %c -1}
t s.int {format %s 42}
t s.list {format %s {a b}}
t f.int {format %f 3}
t f.str {format %f abc}
t x.neg {format %x -255}
t x.hashneg {format {%-8x} 255}
t b.neg {format %b -5}
t o.neg {format %o -8}
t e.inf {format %e Inf}
t g.nan {format %g NaN}
t empty.prec {format {%.d} 5}
t zero.d {format {%0d} 42}
t space.d {format {% d} 42}
t xpg.dup {format {%1$d %1$d} 5}
t xpg.pct2 {format {%%%%}}
t num.after.flag {format {%-5d} 42}
t flag.after.num {format {%5-d} 42}
