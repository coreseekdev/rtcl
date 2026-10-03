# multi-line bracket inside an expr: line attribution
proc p {} {
    set x [expr {1 +
        [nosuch b]}]
}
catch {p} m
puts $::errorInfo
