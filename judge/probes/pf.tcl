foreach e { {abs(-5)} {(0)} {(-5)} {abs(5)} {int(3.7)} {-5} {abs(-5)+1} {2*(3+4)} {min(1,2)} } {
    if {[catch {expr $e} m]} { puts "ERR <<$e>>: $m" } else { puts "OK  <<$e>>: $m" }
}
