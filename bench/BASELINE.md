# bench baseline — bd3e169 2026-09-29
# Linux 7.0.0-31-generic x86_64, tclsh 8.6.17, reps=5 best-of

case               tclsh_ms    rtcl_ms    ratio
arith_loop               43        249      5.7
dict_ops                  8         91     11.3
list_ops                  7        114     16.2
proc_fib                  8         73      9.1
string_build              6          9      1.5
var_incr                 44        194      4.4

# ---------------------------------------------------------------------------
# bench — d69ca08 2026-10-02, after the interpreter fast-path series
# (parse cache + Rc<ProcDef>, dispatch probe-storm fixes, Rc AST source
# sharing, check_expr memo).  bd3e169 table above is kept as the JIT
# milestone gate reference (M1: arith_loop ≥3× vs ITS baseline).
# NOTE: between bd3e169 and the fast-path series, the fix rounds
# (86c1d3b..2ab7165) added errorInfo/expr-check machinery that REGRESSED
# wall times ~2x (interim measurement: arith_loop 586ms, proc_fib 172ms
# on this machine); the series clawed most of that back.
# Linux 7.0.0-34-generic x86_64, tclsh 8.6.17, reps=5 best-of

case               tclsh_ms    rtcl_ms    ratio
arith_loop               36        285      7.9
dict_ops                  8         89     11.1
list_ops                  7         42      6.0
proc_fib                  9         76      8.4
string_build              6         10      1.6
var_incr                 47        269      5.7
