proc b2 {x} {
    if {$x} { puts a } else { nosuch }
}
catch {b2 0} m
puts $::errorInfo
