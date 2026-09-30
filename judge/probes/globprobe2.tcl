proc t {p s} {if {[catch {string match $p $s} m]} {puts "ERR: $m"} else {puts "$p vs $s => $m"}}
t {a[} {a[}
t {a[b} {a[b}
t {[]x]y} {]y}
t {[a-]z} {-z}
t {[\]]z} {]z}
t {[a\]z} {az}
t {[a-z]*} {zz9}
t {[z-a]x} {yx}
t {a\\*b} {a\xy zb}
puts [info patchlevel]
