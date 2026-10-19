proc p {x} {incr x}
catch {p abc} m
set m
