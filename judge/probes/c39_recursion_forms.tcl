# C39（9.1 对齐批 B1，2026-11-26）：递归深度口径——调用深度与求值嵌套
# 分开记账。oracle（tclsh 9.1.0）实测 @默认 limit 1000：
# - 命令替换/直接尾递归（每层 = proc 调用 1）：完成 998 层后报
#   too many nested evaluations（== 边界也要拒）；
# - eval 体递归（每层 = proc 1 + eval 1）：499；
# - 内联循环体（while/for/foreach——tclsh 把体内联编译进调用者单元，
#   体不是求值层）：与直接形同 998（字节码语境 rtcl 同 998）。
# 纯树走查语境（RTCL_NO_BYTECODE/回退单元）的循环体递归仍是资源界
# ~499（每层 Rust 栈 ~8KB+，998 层爆 8MB 栈——树走查保留 per-dispatch
# 计数当栈防线；不进本探针，记账到 D3 树引擎删除时消掉）。
set ::depth 0
set ::max 0
proc A1 {n} { incr ::depth; if {$::depth > $::max} {set ::max $::depth}; if {$n <= 0} { return 0 } ; set r [A1 [expr {$n-1}]]; incr ::depth -1; return $r }
set c [catch {A1 5000} m]
puts "expr-form: c=$c max=$::max msg=[expr {$m eq {too many nested evaluations (infinite loop?)}}]"
set ::depth 0; set ::max 0
proc E1 {n} { incr ::depth; if {$::depth > $::max} {set ::max $::depth}; if {$n <= 0} { return 0 }; set r [eval {E1 [expr {$n-1}]}]; incr ::depth -1; return $r }
set c [catch {E1 5000} m]
puts "eval-form: c=$c max=$::max"
set ::depth 0; set ::max 0
proc D1 {n} { incr ::depth; if {$::depth > $::max} {set ::max $::depth}; if {$n <= 0} { return 0 }; set r [D1 [expr {$n-1}]]; incr ::depth -1; return $r }
set c [catch {D1 5000} m]
puts "direct: c=$c max=$::max"
# 字节码语境的循环体递归（与直接形同界）：经 interp 别名无关路径——
# 直接在脚本里定义 + 调用（VM 主语境里 while/for/foreach 内联）。
set ::depth 0; set ::max 0
proc W1 {n} { incr ::depth; if {$::depth > $::max} {set ::max $::depth}; while {$n > 0} { set r [W1 [expr {$n-1}]]; incr ::depth -1; return $r ; break } ; return 0 }
set c [catch {W1 5000} m]
puts "while-form: c=$c max=$::max"
set ::depth 0; set ::max 0
proc F1 {n} { incr ::depth; if {$::depth > $::max} {set ::max $::depth}; if {$n <= 0} { return 0 }; foreach _ {1} { set r [F1 [expr {$n-1}]]; incr ::depth -1; return $r } }
set c [catch {F1 5000} m]
puts "foreach-form: c=$c max=$::max"
