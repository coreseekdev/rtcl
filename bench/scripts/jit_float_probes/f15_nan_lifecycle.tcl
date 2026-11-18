# jit-battery: preexisting（interp vs tclsh：tclsh 8.6 对 Inf*0.0 的
# NaN 乘积报 domain error，rtcl interp 放行——预存缺口归 judge/specs；
# 本例只强制 jit==rtcl。NaN 生命周期其余语义见下）
# NaN 生命周期（interp 语义对齐面）：NaN 值可经 Inf*0.0 产生，但
# (a) 禁再入算术（"can't use non-numeric floating-point value..."报错）
# (b) 比较 = 0（IEEE 恒假）  (c) Neg = NaN  (d) 0.0/0.0 = domain error。
# float lane 的操作数 NaN 守卫 + Div 0/0 守卫的固化回归面。
proc make_nan {} { expr {[expr {1.0/0.0}] * 0.0} }
set x [make_nan]
puts $x
if {[catch {expr {$x + 1.0}} m]} { puts "ERR: $m" } else { puts "RES: $m" }
if {[catch {expr {$x * 2.0}} m]} { puts "ERR: $m" } else { puts "RES: $m" }
if {[catch {expr {$x > 1e300}} m]} { puts "ERR: $m" } else { puts "RES: $m" }
if {[catch {expr {$x == $x}} m]} { puts "ERR: $m" } else { puts "RES: $m" }
if {[catch {expr {-$x}} m]} { puts "ERR: $m" } else { puts "RES: $m" }
if {[catch {expr {0.0/0.0}} m]} { puts "ERR: $m" } else { puts "RES: $m" }
proc div00 {} { expr {0.0/0.0} }
if {[catch {div00} m]} { puts "ERR: $m" } else { puts "RES: $m" }
proc infdiv {} { expr {1.0/0.0} }
puts [infdiv]
