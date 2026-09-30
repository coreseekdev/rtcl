proc e {label script} {
    set c [catch {uplevel 1 $script} r o]
    if {$c} { puts "$label EC=[dict get $o -errorcode] | $r" } else { puts "$label OK <$r>" }
}
e trailing {format ab%}
e trailing2 {format %}
e notenough {format %d}
e badspec {format %q x}
e endedmid {format {%.} 5}
e mixing {format {%d %1$d} 1 2}
e oob {format {%5$d} x}
e int {format %d 2a}
e nan {format %f NaN}
e widthstar {format {%*d} x 3}
e minus2 {format {a%.-2sa} foobarbaz}
