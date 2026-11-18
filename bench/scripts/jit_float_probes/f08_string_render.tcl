# float→string 永远宿主（shortest-round-trip；JIT 只产值不格式化）
puts [expr 0.1 + 0.2]
puts [expr 1.0 / 3.0]
puts [expr 2.0]
puts [expr 1e21]
set v [expr 0.1 + 0.2]
puts [string length $v]
