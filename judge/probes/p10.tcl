foreach e {3eq2 3e 3e2 3e+2 3x2 5mod3 1%(1<<63) -7%3 7%-3 -7%-3} {
    if {[catch {expr $e} m]} { puts "$e => ERR: $m" } else { puts "$e => '$m'" }
}
