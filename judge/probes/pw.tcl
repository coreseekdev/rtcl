foreach e {268435456 268435455} {
    foreach b {3 0 1 -1 2} {
        if {[catch {expr {$b**$e}} r]} { puts "b=$b e=$e ERR: $r" } else { puts "b=$b e=$e OK: [string range $r 0 20]" }
    }
}
foreach x {2.0**268435456 2**-1 2.0**-3 0.0**-1.0 0**-1 0.0**0 -3**268435455} {
    if {[catch {expr $x} r]} { puts "$x ERR: $r" } else { puts "$x OK: [string range $r 0 20]" }
}
puts [expr {3**268435455 == 3**268435455}]
