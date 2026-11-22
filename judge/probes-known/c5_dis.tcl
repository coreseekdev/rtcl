proc b1 {} {
    if {0} { puts a } else { nosuch }
}
puts [::tcl::unsupported::disassemble proc b1]
