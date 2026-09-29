proc f {} {return -level 2 -code error E}
proc g {} {catch {f} m; set m}
puts [g]
