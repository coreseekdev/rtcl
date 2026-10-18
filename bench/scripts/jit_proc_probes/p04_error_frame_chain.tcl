proc p {} {error "boom in p"}
catch {p} m
set m
set ::errorInfo
