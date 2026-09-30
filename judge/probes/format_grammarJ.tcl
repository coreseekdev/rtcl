proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>" }
}
t star.then.xpg {format {%*d %1$d} 6 42}
t star.then.xpg2 {format {%*d %2$d} 6 42 99}
t xpg.star.xpg {format {%1$d %*d} 42 6 99}
t q.noargs {format %q}
t q.oob {format {%5$q} a b c d e}
t oob.valid {format {%5$d} a b c d e}
t xpg.cursor {format {%1$d %*d %1$d} 42 6 99}
t big.cursor {format {%2$d %1$d %1$d} 4 5}
t star.zero {format {%0*d} 6 42}
t neg.star {format {%*d} -6 42}
t pct5q {format {%5%} a b c d e}
