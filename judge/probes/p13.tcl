foreach e {3gt2 3le2 3ge2 3eq2 3ne2 3in2} {
    if {[catch {expr $e} m]} { puts "$e => ERR: [lindex [split $m \n] 0]" } else { puts "$e => '$m'" }
}
