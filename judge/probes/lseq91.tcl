# lseq (tclsh 9.1, TIP 746 — expr behavior removed; pure numeric series)
# Source of truth: doc/lseq.n + tclCmdIL.c Tcl_LseqObjCmd + tclArithSeries.c.
# Argument-shape decode (arg_key): n / n n / n n n / n to n / n count n /
# n by n / n to n n / n count n n / n n by n / n to n by n / n count n by n
puts [lseq 3]
puts [lseq 3 0]
puts [lseq 10 .. 1 by -2]
puts [lseq 0 -5]
puts [lseq 1 count 5 by 0]
puts [lseq 3 to 9 by 0]
puts [lseq 6 to 1 by 2]
puts [lseq 5 to 1]
puts [lseq 1 to 5]
# doubles render per maxObjPrecision of the args (ObjPrecision = fraction
# digits of the arg text; e-notation args contribute 0)
puts [lseq 0 0.5 by 0.1]
puts [lseq 0 2 by 1.0]
puts [lseq 0 .. 1 by 0.1]
puts [lseq 2.0]
# integer series with a double that fits an integer exactly (bug
# f4a4bd7f1070): 3.0 allowed as integer start, 3.1 errors
puts [lseq 3.0]
catch {lseq 3.1} m; puts $m
# count must be an integer
catch {lseq 1 count 3.1} m; puts $m
# count with a double that is integral converts
puts [lseq 2 count 3.0]
# length computation: int((end-start)/step)+1, direction mismatch -> {}
puts [lseq 1 to 9 by 2]
puts [lseq 9 to 1 by -2]
puts [lseq 1 to 9 by -2]
puts [lseq 9 to 1 by 2]
# single value for step 0
puts [lseq 3 to 9 by 0]
puts [llength [lseq 5 10]]
puts [lseq 10 count 3]
puts [lseq 10 count 3 by 2]
# syntax errors (Tcl_WrongNumArgs "n ??op? n ??by? n??")
catch {lseq} m; puts $m
catch {lseq 1 2 3 4 5 6 7} m; puts $m
catch {lseq 1 gorp 2} m; puts $m
catch {lseq 1 to} m; puts $m
catch {lseq 1 to 2 gorp} m; puts $m
catch {lseq 1 count 2 by} m; puts $m
# unknown operation word reports via Tcl_GetIndexFromObj "operation"
catch {lseq 1 gorp} m; puts "$m :: $errorCode"
# indexing/range over a series behaves like a list
set s [lseq 5]
puts [lindex $s 2]
puts [lrange $s 1 3]
puts [llength $s]
# huge range -> invalid arithmetic series parameter values
catch {lseq 0 1e20} m; puts "$m :: $errorCode"
# series are consumed by list commands as lists
puts [lseq -5 0]
puts [join [lseq 0 to 0.6 by 0.2] ,]

# ── 2026-11-28 T1 余项批增钉（tclsh9.1 逐字节冻结）──────────────────
# 惰性 SeriesRep 验收面：lindex/llength/lrange/foreach/lmap/lsearch/
# lsort 在巨序列上索引不物化（SERIES_EAGER_MAX=4096 以下走 eager 双路）。
# 端点 bignum → ARITH IOVERFLOW；NaN/Inf 域分诊：start NaN → domain
# （len 循环首测即败落 post-check），end NaN → estimate-length 文案
# （Tcl_PrintDouble 小写 "nan"），step NaN/Inf → domain（0*Inf=NaN），
# Inf 端点 → max length exceeded。
# 注：count 为 double（lseq 1e20）的报文随 tclsh 字面量 rep 的编译双路
# 而异——直接编译形 const-fold 成 double rep 报 IOVERFLOW，catch/自包含
# 形是 string rep 报 expected integer。本探针统一 catch 形钉 string-rep
# 路径，与 judge 自包含语料风格一致（直接编译形记为已知偏差）。
foreach c {{lindex [lseq 0x7fffffff] 0x80000000} {lindex [lseq 0x7fffffff] end-1} {lindex [lseq 0 1e20] 0} {catch {lseq 0 0x8000000000000000} m; set m} {catch {lseq 0x8000000000000000 10} m; set m} {catch {lseq 0 10 0x8000000000000000} m; set m} {catch {lseq 5 count 0x8000000000000000} m; set m} {catch {lseq 5 count 1e20} m; set m} {catch {lseq 5 count 2.5} m; set m} {catch {lseq 0 10 nan} m; set m} {catch {lseq NaN 5} m; set m} {catch {lseq 0 nan} m; set m} {catch {lseq nan nan} m; set m} {catch {lseq 0 inf} m; set m} {catch {lseq 0 10 inf} m; set m} {lindex [lseq 10 0x7fffffffffffffff] 5} {lsearch [lseq 0 1000000] 42} {lrange [lseq 0x7fffffff] 100 102} {lmap x [lseq 1 5] {expr {$x * 2}}} {foreach x [lseq 1 5] {incr t $x}; set t} {string length [lseq 0 9]} {join [lseq 0 4] ,}} {
    if {[catch $c m]} { puts "$c => ERR: $m" } else { puts "$c => '[string range $m 0 40]' len=[llength $m]" }
}
