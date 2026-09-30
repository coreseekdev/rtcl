proc t {p s} {if {[catch {string match $p $s} m]} {puts "ERR: $m"} else {puts "$p vs $s => $m"}}
t {[x]]z} {x]z}
t {[a]]z} {a]z}
t {[\]]z} {z}
t {[\]]z} {\]z}
t {[a\]b]z} {a]bz}
t {[a\]b]z} {abz}
t {[a\]b]z} {a]z}
t {[\a]z} {az}
t {[a-b-c]z} {-z}
t {[a-b-c]z} {cz}
t {[^]a]z} {z}
t {[^a]z} {bz}
puts [info patchlevel]
