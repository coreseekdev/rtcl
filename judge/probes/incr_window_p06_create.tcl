unset -nocomplain nv
incr nv [expr {5}]
puts $nv
unset -nocomplain nv2
incr nv2 [string trim " 12 "]
puts $nv2
