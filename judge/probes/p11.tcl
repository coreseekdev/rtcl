foreach e {3and2 3ne2 3lt2 3in2 3eq2x 3eqx 12eq1x 3or0 3eq2.0 3eq"2"} {
    if {[catch {expr $e} m]} { puts "$e => ERR: [lindex [split $m \n] 0]" } else { puts "$e => '$m'" }
}
