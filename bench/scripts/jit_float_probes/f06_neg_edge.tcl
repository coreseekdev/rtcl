# Neg：float → f64.neg（含 -0.0/Inf 语义），imm int → int neg
puts [expr -1.5]
puts [expr -1]
puts [expr {-(0.0)}]
puts [expr {-1.0/0.0 * 1.0}]
set z 0.0
puts [expr {-$z}]
puts [expr -[expr 3]]
