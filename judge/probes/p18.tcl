foreach v {-0x1234 {-0x1234} 0b101 +0x10} { puts "$v -> [expr {$v == 0}] int:[expr int($v)]" }
