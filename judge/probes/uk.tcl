proc unknown {args} { error "unk-fail $args" }
catch {zzz 1 2} m
puts "M:$m"
puts "EI:$::errorInfo"
