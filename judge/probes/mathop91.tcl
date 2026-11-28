# 9.1 mathop 钉形（2026-11-28，T1 余项批，对 tclsh9.1 逐字节冻结）：
# - 单操作数 `-`/`/`：负化/倒数（`- 5`→-5、`/ 5`→0.2、`/ 1`→1.0 实数除）
# - tclBasic.c mathOpCmds 形状：`!=`/`ne`/`in`/`ni`/`%`/`<<`/`>>`/`~`/`!`
#   严格定参（TclSingleOpCmd），`< <= > >= == eq lt le gt ge` nary 链
#   （TclSortingOpCmd，0 参空真 1）
# - usage 回显调用点字面（`::` 有无随调用）
foreach c {{::tcl::mathop::- 5} {::tcl::mathop::/ 5} {::tcl::mathop::/ 1} {::tcl::mathop::- 0} {::tcl::mathop::/ 0} {::tcl::mathop::/ -5} {::tcl::mathop::- 2.5} {::tcl::mathop::/ 2.5} {::tcl::mathop::/ 5 2} {::tcl::mathop::/ 5 2.0} {::tcl::mathop::/ 5 0} {::tcl::mathop::/ 7 0.0} {::tcl::mathop::- 1 2 3} {::tcl::mathop::!= 1} {::tcl::mathop::ne 1} {::tcl::mathop::ne 1 2 3} {::tcl::mathop::ne a a} {::tcl::mathop::eq 1 2 3} {::tcl::mathop::eq} {::tcl::mathop::<} {::tcl::mathop::==} {::tcl::mathop::lt 1 2 3} {::tcl::mathop::lt 2 10} {::tcl::mathop::ge 2 10} {::tcl::mathop::in 1 {1 2}} {::tcl::mathop::ni 3 {1 2}} {::tcl::mathop::in} {::tcl::mathop::ni a b c} {::tcl::mathop::<<} {::tcl::mathop::>> 1} {::tcl::mathop::~} {::tcl::mathop::!} {::tcl::mathop::% 5} {tcl::mathop::in} {tcl::mathop::ne 1} {::tcl::mathop::max 1 5}} {
    if {[catch $c m]} { puts "$c => ERR: $m" } else { puts "$c => '$m'" }
}
