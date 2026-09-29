foreach e {{::tcl::mathfunc::abs -0} {isqrt({rubbish})} {{-0x1234}}} {
    if {[catch {expr $e} m]} { puts "$e => ERR: $m" } else { puts "$e => '$m'" }
}
set x NaN; puts "NaN==NaN: [expr {$x == $x}]"
set a [list one two three]
puts "list-eq: [expr {$a eq {}}]"
