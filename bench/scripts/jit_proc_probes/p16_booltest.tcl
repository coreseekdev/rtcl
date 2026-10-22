proc p {c} {while $c {}}
catch {p foo} m
set m
