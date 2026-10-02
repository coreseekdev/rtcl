proc b3 {} {
    if {0} { puts a } else {
        nosuch
    }
}
catch {b3} m
puts $::errorInfo
