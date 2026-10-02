proc b1 {} {
    if {0} { puts a } else { nosuch }
}
catch {b1} m
puts $::errorInfo
