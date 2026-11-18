# NaN/Inf 是合法值（IEEE754 = tclsh；不用哨兵）
puts [expr 0.0/0.0]
puts [expr 1.0/0.0]
puts [expr -1.0/0.0]
puts [expr {0.0/0.0 == 0.0/0.0}]
puts [expr {0.0/0.0 < 1.0}]
puts [expr {1.0/0.0 > 0.0}]
puts [expr 1e308 * 10]
puts [expr -1e308 * 10]
