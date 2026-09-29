foreach e {3eq 2 3 eq2 eq2 3eq2 3neq2 3eq7 3in7 7in3 3is2x 3andx} {
    if {[catch {expr $e} m]} { puts "$e => ERR: [lindex [split $m \n] 0]" } else { puts "$e => '$m'" }
}
