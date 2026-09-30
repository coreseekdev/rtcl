foreach x {2.0**268435456 2**-1 2.0**-3 0.0**-1.0 0**-1 0.0**0} {
    if {[catch {expr $x} r]} { puts "$x ERR: $r" } else { puts "$x OK: $r" }
}
puts "268435455-check: [catch {expr {(-2)**268435455}} m] [string range $m 0 30]"
puts "double-big: [catch {expr {2.0**10000}} m] [string range $m 0 30]"
