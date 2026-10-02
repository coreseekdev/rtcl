proc a1 {} {
    if {1} { nosuch }
}
catch {a1}
puts "arm: [info line nosuch] / err=[lindex [split [set ::errorInfo] \n] end-1]"
catch {a1} m
puts $::errorInfo
puts ===
proc a2 {} {
    if {0} { } else { nosuch }
}
catch {a2} m
puts $::errorInfo
