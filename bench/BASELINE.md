# bench baseline — bd3e169 2026-09-29
# Linux 7.0.0-31-generic x86_64, tclsh 8.6.17, reps=5 best-of
# Architecture review + ranked roadmap (2026-10-04): ../ARCHITECTURE-PERF.md

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

# ---------------------------------------------------------------------------
# bench — 2026-10-03, B-series: value/literal architecture (lazy from_int,
# in-place IncrVar single-lookup, inline `[...]` word compilation with
# SubMark/SubEnd regions + expr partial-emission rollback fix).
# Same machine, reps=3, taskset -c 0.
#
# rtcl now beats tclsh on the loop kernels (arith .8x, var_incr .7x) and
# sits at parity on string_build.  proc_fib (5x) is the remaining gap:
# brackets *inside braced exprs* still eval via the runtime expression
# parser (fib25 with brackets-as-words: 175ms vs 654ms expr-embedded;
# tclsh 30ms).

case               tclsh_ms    rtcl_ms    ratio
arith_loop               40         32       .8
dict_ops                  5          7      1.4
list_ops                  5          7      1.4
proc_fib                  5         25      5.0
string_build              3          3      1.0
var_incr                 41         29       .7

# ---------------------------------------------------------------------------
# bench — 2026-10-03, expr-bracket inlining: `[...]` operands of compiled
# expressions inline into the unit (ExprMark/SubEnd regions; the
# ExprSink trait lets expr_compile emit into the Compiler, byte-offset
# rebasing via the verbatim-braced word's span).  The classic
# `expr {[fib [expr {$n-1}]] + [fib [expr {$n-2}]]}` shape no longer
# runs the runtime expression parser per evaluation.  fib25 hand-probe:
# 654ms → 136ms (4.8x; tclsh 31ms).  Error framing differentially
# verified bytecode == tree-walk on 6 probe scripts; rtcl-vs-tclsh
# deltas remain the recorded divergence classes (expr/if harness
# frames, empty-operand message).
# Same machine, reps=5, taskset -c 0.

case               tclsh_ms    rtcl_ms    ratio
arith_loop               34         31       .9
dict_ops                  5          6      1.2
list_ops                  5          7      1.4
proc_fib                  4         13      3.2
string_build              3          3      1.0
var_incr                 39         29       .7

# ---------------------------------------------------------------------------
# E1 2026-10-04 (ARCHITECTURE-PERF.md item E, stage 1): VmState pooling +
# allocation-free dispatch collection.  exec_bytecode borrows its
# stack/loops/bodies/scratch Vecs from an Interp-level pool (capacity kept
# across proc calls); Call/DynCall ops DRAIN entries into a reusable
# scratch buffer (moves, no per-arg Value clone, no per-command Vec) and
# the dispatch helpers take &[Value] (the per-call command-name String is
# gone).  call_proc: frame-locals borrow hoisted out of the binding loop.
# Gates: judge 87/87, two-engine sweep artifact-only, workspace tests
# green, feature matrix green.
# Decomposition probes (taskset -c 0, best-of-3): decomp_call 115→110ms,
# decomp_var 151→142ms, fib25 134→121ms; bench table unchanged (list_ops
# 1.7 is tclsh-side noise — its reference also moved 5→4ms).
# The measured 7x binding+vars slice needs E2 (slot locals); E1 removed
# the allocation tax around it.

case               tclsh_ms    rtcl_ms    ratio
arith_loop               45         37       .8
dict_ops                  5          6      1.2
list_ops                  4          7      1.7
proc_fib                  4         13      3.2
string_build              3          3      1.0
var_incr                 40         28       .7

# ---------------------------------------------------------------------------
# E1.75 2026-10-04: dispatch maps + per-call name allocations.  commands/
# procs move from std SipHash to varmap's Fx hasher (dispatch probes them
# every DynCall); dispatch_values carries the resolved proc name as a Cow
# (plain-name hit borrows the invocation word); call_proc's frame name is
# a Cow borrowed until a tail-call rebind owns its target.  Two String
# allocations per proc call gone.  Gates: judge 87/87, sweep artifact-only
# (objmech flat=0s/1s timing flake), tests green.
# fib25 121→114ms; decomp_var/decomp_call within noise (142/106ms best-of).

case               tclsh_ms    rtcl_ms    ratio
arith_loop               45         37       .8
dict_ops                  5          6      1.2
list_ops                  4          7      1.7
proc_fib                  4         12      3.0
string_build              3          3      1.0
var_incr                 40         28       .7
# ---------------------------------------------------------------------------
# E1.9 2026-10-04 (549a716; note backfilled): dispatch_call's builtin fast
# path required `procs.is_empty()` — any script defining a single proc
# diverted EVERY compiled Call op through the full dispatch_values probe
# storm; the guard now probes `!procs.contains_key(name)`.  call_proc's
# frame teardown skips the `F{i}:` prefix format! and traced-name scan
# when no variable traces exist.  fib25 114→104ms, decomp_call ~106→104,
# decomp_var unchanged (~144).  Gates: judge 87/87, tests green.
# (Bench table unchanged from E1.75 within noise.)

# ---------------------------------------------------------------------------
# E2 2026-10-04 (ARCHITECTURE-PERF.md item E, stage 2): slot-resolved proc
# locals.  `proc` compiles its body with a locals table (params seed the
# first slots in order, plain-name `set`/`incr` targets append at compile
# time — `compile_unit_locals`); plain-name reads/writes of table names
# compile to LoadLocal/StoreLocal/IncrLocal (slot operand; expr `$var`
# operands route through ExprSink::var_read so loop conditions hit slots
# too).  call_proc binds arguments positionally into the slots when the
# slot gate holds (compiled + epoch-valid + bytecode applicable + no
# statics + the table's params-first seeding aligning positionally with
# the parameter list + non-empty table; re-gated per tailcall iteration —
# the alignment zip also catches duplicate parameter names, which dedup
# in the table and would shift every later slot; tclsh rejects such
# procs at definition, rtcl accepts them with map semantics (last
# binding wins; tclsh is first-wins — a recorded divergence class)).
# Slots are the canonical store and
# every name-keyed path ALIASES them (tclsh's compiledLocals/var-table
# trick): get_var/resolve_var/set_var/store_var/remove_var/
# loc_base_exists/incr_var_fast/take_var_fast consult
# `CallFrame::slot_index_of` first.  Degrade (slots → map, in slot order
# — the tree-walk's insertion order, keeping unsorted enumerations
# byte-exact) on: upvar/global/variable link install (both the linking
# frame and a frame-target's frame), trace registration, array-ification
# (set_var element branch + mark_array), and whole-frame for `info
# locals`/`info vars`/`info frame`.  Unset empties the cell; slot ops on
# degraded/statics frames fall back through their name paths.
# Gates: judge 87/87; two-engine sweeps — judge corpus 0 diffs (87
# files) and full probe sweep 394 files with only the 11 known
# artifacts (8 env-dump, bnprobe2 pointers, objmech/objsec timing);
# workspace tests 1132 green (+13 slot_locals tests) covering
# basics/aliasing/degrades/tailcall/epoch/enumeration/duplicate-params.
# Decomposition probes (taskset -c 0, best-of-3): decomp_var 144→96ms
# (tclsh 54; gap 2.7→1.8x), decomp_call 104→96ms (tclsh 74), fib25
# 104→87ms (tclsh 31).  Bench table: proc_fib 12→9ms (3.0→1.8x);
# everything else already at/below parity and unchanged.

case               tclsh_ms    rtcl_ms    ratio
arith_loop               32         31       .9
dict_ops                  4          6      1.5
list_ops                  4          7      1.7
proc_fib                  5          9      1.8
string_build              3          4      1.3
var_incr                 40         27       .6

# ---------------------------------------------------------------------------
# D 2026-10-04 (ARCHITECTURE-PERF.md item D): inline foreach/lmap in compiled
# proc bodies (ForeachStart/Next/Collect/End + slot-or-name binding) with the
# lexical tree-walk twin (lexical_body arming mirrors the compiler's inline
# decisions) and the proc-context catch/apply work: tclsh compiles apply
# lambda bodies like proc bodies and inlines braces-only catch bodies into
# proc-context units — ByteCode::locals_mode → Interp::in_locals_unit, apply
# compiles+memoises (lambda_code_cache), catch routes braced bodies through
# eval_lexical_script.  This closed judge gen_lmap lmap-4.15 (compiled-context
# plain var-write) while keeping foreach-1.14's top-level decoration.
# Gates: judge 87/87, two-engine sweep 8 known artifacts / 0 new, workspace
# tests 785+ (incl. foreach-inline + catch-context + apply-compile suites).
# The standard table doesn't exercise foreach — the dedicated microbench
# (taskset -c 0, /tmp/fe_bench.tcl shape) carries the D number:
#   proc-foreach: rtcl 14.9-15.4ms vs tclsh 5.2-5.5ms (~2.9x; was 28x pre-D).
# proc_fib 9→11ms is reps-3 noise (E2's reps-5 run recorded 9).

case               tclsh_ms    rtcl_ms    ratio
arith_loop               34         34      1.0
dict_ops                  5          7      1.4
list_ops                  5          8      1.6
proc_fib                  5         11      2.2
string_build              3          4      1.3
var_incr                 39         31       0.7
