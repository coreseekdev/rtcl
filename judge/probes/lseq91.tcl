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
