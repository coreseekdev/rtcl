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

# ---------------------------------------------------------------------------
# bench — C5+C6 2026-10-03: bytecode VM live (proc bodies at the `proc`
# definition site + every `eval()` unit via the bytecode cache).  Same
# machine, reps=5 best-of.  d69ca08 table kept as the C5 comparison base.
# Remaining gaps are dispatch-bound (foreach/dict/lappend DynCalls +
# substituted-word EvalScripts), the JIT milestone's territory.

case               tclsh_ms    rtcl_ms    ratio   vs-d69ca08
arith_loop               36        188     5.22        1.62x
dict_ops                  8         90    11.25        1.06x
list_ops                  7         42     6.00        0.98x
proc_fib                  8         41     5.12        1.83x
string_build              5          9     1.80        0.89x
var_incr                 41         98     2.39        2.80x

# ---------------------------------------------------------------------------
# bench — 74f9449 2026-10-03, after the A-series architecture batch
# (A1 var fast paths, A3 zero-copy list reads, A4 in-place mutation,
# A5 loop-body memo + call-proc overhead, A2 for-inline + return
# -level 0 semantics).  for now compiles inline like tclsh: the
# per-iteration ExprParser run is gone.
# Linux 7.0.0-34-generic x86_64, tclsh 8.6.17, reps=5

case               tclsh_ms    rtcl_ms    ratio
arith_loop               33         53      1.6
dict_ops                  5          7      1.4
list_ops                  5          8      1.6
proc_fib                  5         27      5.4
string_build              3          4      1.3
var_incr                 41         44      1.1
