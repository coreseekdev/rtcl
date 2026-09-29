foreach e {{false} {true} {--5} {+--++36} {3eq2} {entier(1e+22)} {round(9.2233720368547758e+18)} {isqrt(1,2)} {isqrt(-1)} {abs(-0x0)} {500000000000000<<28} {1%(1<<63)}} {
    if {[catch {expr $e} m]} { puts "$e => ERR: $m" } else { puts "$e => '$m'" }
}
