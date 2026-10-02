# errorInfo frame shapes through the VM path
proc errbody {} {
    while {$q < 3} { incr q }
}
if {[catch {errbody} m]} { puts [set ::errorInfo] }
puts ---
proc errcond {} {
    while {zz} { puts hi }
}
catch {errcond} m
puts $::errorInfo
puts ---
proc elser {} {
    if {0} { puts a } else { nosuch }
}
catch {elser} m
puts $::errorInfo
puts ---
proc subser {} { set x [boom2] }
catch {subser} m
puts $::errorInfo
