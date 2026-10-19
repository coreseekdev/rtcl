proc p {a b} {expr {$a + $b}}
catch {p 1 xyz} m
set m
