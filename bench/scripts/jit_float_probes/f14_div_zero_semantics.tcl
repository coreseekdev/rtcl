# 除零分界面：int 除/模零 → 报错（宿主全语义）；float 除零 → Inf/NaN
# （IEEE）。float lane on 时除零守卫必须落宿主而非 float 路径。
if {[catch {expr {7/0}} msg]} { puts "ERR: $msg" } else { puts "NO-ERR: $msg" }
if {[catch {expr {7%0}} msg]} { puts "ERR: $msg" } else { puts "NO-ERR: $msg" }
if {[catch {expr {-7/0}} msg]} { puts "ERR: $msg" } else { puts "NO-ERR: $msg" }
puts [expr {7.0/0}]
puts [expr {-7.0/0}]
puts [expr {0.0/0.0}]
proc div_zero {} { expr {7/0} }
if {[catch {div_zero} msg]} { puts "ERR: $msg" } else { puts "NO-ERR: $msg" }
proc div_float_zero {} { expr {7.0/0} }
puts [div_float_zero]
