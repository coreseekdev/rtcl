menu .m -tearoff 0
.m add command -label "One" -command {puts one}
.m add command -label "Two" -command {puts two}
puts "XP|[.m xposition 0]|[.m xposition 1]|[catch {.m xposition 9} m] $m"
puts "PC|[catch {.m postcascade nosuch} m] $m"
puts "PC2|[catch {.m postcascade 1} m] $m"
.m postcascade 1
set cok [catch {.m clone .clone} m]; puts "CLONE|$cok|$m|[info commands .clone]"
set c2 ""; set cok2 [catch {.m clone clone2.m submenu} m]; puts "CLONE2|$cok2|$m"
puts "CTYPE|[.m type 0]|[.m type 1]"
puts "YP|[.m yposition 0]"
exit 0
