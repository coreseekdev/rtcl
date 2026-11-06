# 编译特化命令的 INNER 操作码名（tclsh 8.6.17 指令名）。
# 覆盖 lexical（proc 局部槽）与 stk（eval/uplevel 脚本）两套命名：
#   set→storeScalar/Array(1|4|Stk)  incr→incrScalar/Array(1|Stk)(Imm)
#   lappend→lappendScalar/Array(1|4)/lappendList(Array)Stk
#   append→appendScalar/Array(1|4)/(Array)Stk  unset→unsetScalar/Array/(Array)Stk
#   regexp 字面命令的 pattern 编译失败 → INNER 为裸命令词列表（无操作码）。
proc show {label script {mode proc}} {
    if {$mode eq "proc"} {
        proc p {} $script
    } elseif {$mode eq "eval"} {
        proc p {} "eval {$script}"
    } else {
        proc p {} "uplevel #0 {$script}"
    }
    catch {p} m o
    set es [expr {[dict exists $o -errorstack] ? [dict get $o -errorstack] : "<none>"}]
    puts "$label => $m || $es"
}
show regexp-literal {regexp {(} x}
show regexp-dynamic {set r regexp; $r {(} x}
show regexp-eval {regexp {(} x} eval
show regexp-space {regexp {(} {a b}}
show set-scalar-on-array {set a(1) 1; set a 2}
show set-array-on-scalar {set a 1; set a(1) 2}
show set-eval {set a 1; set a(1) 2} eval
show incr-scalar-nonint {set x abc; incr x}
show incr-scalar-nonint2 {set x abc; incr x 2}
show incr-scalar-var {set x abc; set d 2; incr x $d}
show incr-array-el {set a(1) abc; incr a(1)}
show incr-array-el-var {set a(1) abc; set d 2; incr a(1) $d}
show incr-eval {set x abc; incr x} eval
show lappend-scalar-badlist {set x "a \{"; lappend x b}
show lappend-array-on-scalar {set a 1; lappend a(1) x}
show lappend-eval {set x "a \{"; lappend x b} eval
show append-scalar-on-array {set a(1) 1; append a x}
show append-array-on-scalar {set a 1; append a(1) x}
show append-eval {set a(1) 1; append a x} eval
show unset-scalar {unset nope}
show unset-array {unset q(1)}
show unset-multi {unset n1 n2}
show unset-eval {unset q(1)} eval
show unset-dynamic {set u unset; $u nope}
show incr-arity {incr a 1 2}
show set-arity {set a b c}
