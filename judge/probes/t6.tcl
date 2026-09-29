proc f {} {return -level 0 -code ok v; puts unreached}
puts [f]
