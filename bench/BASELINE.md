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

# ---------------------------------------------------------------------------
# C 2026-10-04 (ARCHITECTURE-PERF.md item C): command-resolution cache.
# Root cause first (instrumented): stdlib.tcl's `namespace ensemble create`
# inside `namespace eval ::tcl::tm` means `ensembles` is NEVER empty in a
# real session — dispatch_call's fast-path guard never fired, so EVERY
# compiled Call/DynCall walked dispatch_values' full chain: ~7 procs probes
# + find_ensemble + commands chain, including two heap `format!("::{}",
# name)` keys per dispatched command (~1M dispatches in the dict bench;
# profile: fmt machinery ~6%, VarHasher 5.3%, memcmp 5%).  Fix: tclsh-style
# resolved-command tokens — `cmd_cache: VarMap<VarMap<CachedCmd>>` keyed by
# (current namespace, invocation name), entries carry the command-table
# generation; every commands/procs/ensembles/import_aliases mutation bumps
# `cmd_generation` (hooks at all 35+ sites: proc/rename/namespace sweep/
# import/forget/ensemble create/oo attach+detach/alias/interp/registry).
# Outcomes cached: builtin fn ptr (Call ops now dispatch through ONE
# generation-checked probe — the CmdId::from_name memcmp chain is gone)
# and proc KEY (the def is re-fetched per call: statics write-back replaces
# the map entry via Rc::make_mut, so caching the def would go stale — the
# 5 statics unit tests caught exactly that).  Ensemble/unknown/unresolved
# names re-run the chain (uncached).
# Gates: judge 87/87; two-engine sweep 8 known artifacts / 0 new; workspace
# tests 1143 green; dloop (300k dict set + 300k foreach dict get/incr):
# rtcl 598→437ms best-of-3 vs tclsh 249ms (2.2x → 1.76x); post-C profile
# shows the fmt/format entries gone, allocator/memmove dominant.  fe_bench
# (foreach-inline) unchanged at 15.0-15.1ms as expected.  fib25 interleaved
# A/B vs pre-C HEAD: <=3-5%, within the day's noise band (a co-running
# session's test load inflated all engines; tclsh fib25 swung 26→44ms).
# Known pre-existing divergence noticed while testing (NOT C's doing, not
# corpus-covered): `rename` of a name a proc shadows removes the BUILTIN,
# leaving the proc (tclsh removes the resolved command — the proc replaced
# the table entry at definition).  Recorded as a follow-up.

case               tclsh_ms    rtcl_ms    ratio
arith_loop               33         35      1.0
dict_ops                  5          6      1.2
list_ops                  5          7      1.4
proc_fib                  5         11      2.2
string_build              4          4      1.0
var_incr                 42         31      0.7

# ---------------------------------------------------------------------------
# F1 2026-10-04 (ARCHITECTURE-PERF.md item F, part 1): lazy execution-trace
# context in call_proc.  F was re-scoped from measurement, not assumption:
# the roadmap text assumed return-with-value allocates, but Error::ret moves
# the Value inline into the enum (allocation-free) and compiled loops
# already jump directly for break/continue.  The symbolized fib25 profile
# said the real per-call costs were call_proc self 18.3% (its FIRST act was
# resolve_command_key — the 7-arm probe chain + a format! qualifier walk —
# plus two String allocs and an exec_step_stack push) — while the only
# reader, exec_step_begin (trace.rs), checks exec_traces.is_empty() first:
# in an untraced session (every real one) all of it was dead work.  Fix:
# `traced_entry` gate — the resolve/push/leave-context happen only when
# execution traces exist; the tailcall rebind and the exit pop/leave-fire
# are guarded by the same flag (the rebind would otherwise clobber the
# CALLER's context).
# Semantics: probed tclsh 8.6.17 — enterstep/leave traces registered on an
# ALREADY-RUNNING proc do not instrument that invocation; rtcl's eager
# context DID fire them (two divergences, /tmp/trv_mid2.tcl + trv_leave.tcl
# shapes).  After F1 rtcl matches tclsh byte-for-byte on both probes.
# Gates: judge 87/87; two-engine sweep 8 known artifacts / 0 new (objquad
# flaked once — it prints clock-seconds deltas, timing not semantics; 3/3
# clean re-runs); workspace tests 1143 green.  Interleaved A/B vs pre-F1
# HEAD build under load ~4-5: dloop 568-629 → 452-460ms (~20%; the loop
# calls a proc per iteration, each paying the resolve) vs tclsh 267 (1.7x);
# fe_bench 0.14-0.17 → 0.12s; fib25 96.8-112.6 → 93.1-96.6ms (~4% — call
# overhead is a smaller slice of fib's recursive profile).  Standard table
# (5 reps best-of, noisy machine): ratios improved across the board vs the
# post-C row (dict_ops 1.2→1.0, list_ops 1.4→0.9, proc_fib 2.2→2.1).

case               tclsh_ms    rtcl_ms    ratio
arith_loop               41         40      0.9
dict_ops                  8          8      1.0
list_ops                 10          9      0.9
proc_fib                  7         15      2.1
string_build              6          8      1.3
var_incr                 51         34      0.6

# ---------------------------------------------------------------------------
# F2 2026-10-04 (ARCHITECTURE-PERF.md item F, part 2): namespaces as Rc<str>.
# The F1 profile's next cost: call_proc cloned 3-4 namespace Strings per call
# (prev_namespace, frame.call_ns, ns_of_qualified's result, def_ns for the
# frame) — all "::" (2 chars, one malloc each) for global-scope procs, plus
# ns_of_qualified's rfind showed as the profile's ReverseSearcher.  Change:
# `current_namespace: Rc<str>` (+ `ns_root: Rc<str>` cached root handed out
# per global-proc call), `frame.ns`/`frame.call_ns: Option<Rc<str>>`,
# ns_of_qualified returns Rc<str> reusing ns_root; names built fresh
# (qualify, ns eval, oo::define) move their String in via `Rc::from`
# (reuses the allocation).  A global-scope proc call now allocates ZERO
# namespace strings (was 4-5).  ~60 sites touched, all cold paths
# (namespace/info commands, upvar targets, unknown-handler walk);
# comparisons became `.as_ref() ==`, map probes `.get(x.as_ref())`.
# Gates: judge 87/87; two-engine sweep 8 known artifacts / 0 new; workspace
# tests 1143 green; namespace smoke vs tclsh byte-exact (proc-in-ns,
# relative resolution, ensemble, upvar/uplevel, apply ::ns, rename across
# ns, variable, tailcall-in-ns, info level 0) + F1's trace probes still
# byte-match tclsh.  Interleaved A/B vs F1 build: fib25 89.7-99.0 →
# 80.4-83.1ms (~10-15%; recursion clones namespaces per call); dloop
# 0.41-0.47 → 0.43-0.44s (~3%, noise-adjacent); fe_bench neutral (foreach
# body dominates, not the call seam).

case               tclsh_ms    rtcl_ms    ratio
arith_loop               40         36      0.9
dict_ops                  8          9      1.1
list_ops                 10         11      1.1
proc_fib                  8         16      2.0
string_build              5          6      1.2
var_incr                 42         38      0.9

# ---------------------------------------------------------------------------
# F-followup 2026-10-04: numeric fast path — lazy bignum widening.
# The fib25 profile's remaining numeric bucket (as_int + int_rep/to_big
# ~6%) traced to eager conversion: numeric_binop's I64×I64 arm computed
# ia.to_big()/ib.to_big() (two BigInt allocations) BEFORE the checked
# arithmetic, paying them on every non-overflowing op — i.e. every `+`,
# `-`, `*`, `/` in loop/recursion code — and using them only on the
# overflow-widen branch.  int_bitop had no i64 path at all: `&`/`|`/`^`
# widened both operands to BigInt unconditionally.  Fix: widen lazily
# (overflow arms convert inline); bitop gains an I64×I64 arm (bitwise on
# i64 never overflows).  Side effect: bitop results of i64 operands now
# carry the int rep instead of the string rep (string form identical).
# Gates: judge 87/87; sweep 8 known / 0 new; tests 1143; numeric smoke
# vs tclsh byte-exact (i64 overflow widen, MIN/-1, shifts, bitwise,
# radix literals, float mixing, div-by-zero text).  fib25 interleaved vs
# F2: neutral-to-marginal (the eager conversions were a smaller slice
# than the profile bucket suggested); standard table under light load:
# proc_fib 2.0 → 1.8, nothing regressed.

case               tclsh_ms    rtcl_ms    ratio
arith_loop               35         36      1.0
dict_ops                  8         10      1.2
list_ops                  9         10      1.1
proc_fib                  8         15      1.8
string_build              6          6      1.0
var_incr                 44         34      0.7

# ---------------------------------------------------------------------------
# G1 2026-10-04: foreach zero-copy iteration + hoisted iteration count.
# New profile round (perf, cpu-clock, symbolized) on the >1.2x-gap loads
# found the compiled foreach paying per-iteration copies the two-engine
# design never intended: (a) ForeachNext RE-COMPUTED the group iteration
# count on every iteration (groups × lists zip + div_ceil + max — 7.3% of
# the proc-foreach profile); (b) ForeachStart strict-listed every varlist
# into an OWNED Vec<Value> — a 200k-element list cost 200k Rc bumps at
# start and 200k drops at end (Vec<Value> clone 5.5% + drop 6.1%);
# (c) foreach_bind cloned each bound value twice (owned get + the slot
# write's by-value argument).  Fix: iteration count fixed once at
# ForeachStart (ForeachFrame.iters); a varlist already carrying a list
# internal rep is held BY REFERENCE (ForeachList::Rep — one Rc bump, the
# tclsh model of a refcounted list read in place; COW keeps mid-loop
# `lappend` iterating the start snapshot, probed byte-exact); only
# string/dict-rep sources materialise an owned Vec (same strict-parse
# errors).  frame_slot_write takes &Value so the slot path bumps once
# and the name fallback moves the value.
# Gates: judge 87/87; sweep 8 known + objquad (clock-seconds boundary
# flake, value output identical — bytecode now finishes the 40000 case
# just under the second boundary more often than the tree engine);
# tests 1143; foreach smoke vs tclsh byte-exact incl. uneven multi-group,
# lmap, break/continue, mid-loop lappend on the iterated list, dict
# flatten, in-proc error framing; both engines byte-identical.
# fe_bench (proc-foreach, time {p $L} 5, best of interleaved): 26.3ms
# → 14.9-16.3ms per iteration (~1.7x).  dloop 441 → 405-429ms (the
# dict-keys foreach borrows too).  fib25 neutral (74-102ms band, no
# foreach in the hot path).

# ---------------------------------------------------------------------------
# G2 2026-10-04: call-path allocation removal (borrowed args, pooled
# level0, word-src singleton).  The same profile round put ~4.6%
# allocator traffic (malloc+free) + 8.4% memmove on the fib25 path;
# three per-call copies tracked down: (a) call_proc opened with
# `args.to_vec()` — one Vec malloc + memcpy of every invocation word per
# proc call, its buffer then moved into `level0_src` and DROPPED at
# frame teardown; (b) exec_bytecode swapped cur_cmd_word_srcs for a
# fresh `Rc::new(Vec::new())` per proc call; (c) `info level 0`'s
# invocation words were Option<Vec<Value>> reallocated per call.
# Fix: args borrow the caller's slice for the whole call (a tail-call
# target materialises an owned vector — same shape the proc name's Cow
# already had); CallFrame.level0 is a plain pooled buffer (cleared, not
# dropped — capacity rides the frame pool; each loop iteration rewrites
# it exactly as before: the apply override first, else this iteration's
# words, tailcall rebinds included); exec_bytecode bumps a shared
# `word_srcs_nil` Rc.
# Gates: judge 87/87; sweep 8 known (objquad's clock-seconds boundary
# flake did not fire this round); tests 1143; call-path smoke vs tclsh
# byte-exact (info level 0 plain/args/defaults/tailcall-rebind/apply,
# arity error text, level 1, 400-deep pool reuse), engines identical.
# fib25 interleaved best-of-5: 79.0 -> 75.5ms (~4%); fe/dloop neutral.

# ---------------------------------------------------------------------------
# G3 2026-10-04: literal value pool + negative small-int cache.  The
# symbolized profile round showed every PushConst/PushInt re-allocated:
# `Value::from_str` = one Rc malloc per literal push (dispatch command
# names, data-list words, operator strings), and from_int covered only
# [0,256) so decrementing loops' PushInt(-1) malloced per iteration.
# Fix: Interp.const_pool (HashMap<code-addr, ConstPoolEntry{ Rc<ByteCode>
# keepalive, Rc<[Value]> constants }>, bound 512 clear-on-overflow,
# inserted at the four compile seams: eval's two compile_unit sites, proc
# definition, lambda cache) pre-materialises a unit's constants; the
# executor takes ONE pool probe per exec_bytecode call into VmState
# (st.consts: Option<Rc<[Value]>>) and PushConst/PushConstWide become a
# slice index + Rc bump, zero per-push malloc/copy (a pool miss falls
# back to from_str — same bytes, correctness never depends on
# residency).  CACHED_INTS extended to [-128,256): the -1/-N step
# operands push a cached singleton instead of Rc::new per iteration.
# SWEEP ROOT CAUSE FOUND (user-directed): the historical "8 known
# artifacts" were never engine divergences — every one (ivars2,
# namespace_varmodel2, oo2, p-uplevel-ns, q114b, trv_p11, trv_p8, u72)
# diffs ONLY in an extra `env(RTCL_NO_BYTECODE)` element: the tree arm
# was selected by exporting the env var, which materialises into
# $env(*) at startup, and those probes enumerate variables.  Fix at the
# launcher level (user directive): rtcl gains `--no-bytecode`
# (rtcl_core::interp::set_bytecode_disabled — an AtomicBool consulted
# by bytecode_applicable before the env OnceLock; env var still works)
# and sweep2.sh selects the tree arm by FLAG, leaving the process env
# identical between arms.  Result: sweep 0 diffs, strict — no known-
# artifact allowance any more (objquad's clock-seconds boundary flake
# remains possible under load; bn14 once flapped on the sweep's old
# 60s bytecode-arm timeout under load — timeouts now 120s both arms).
# Gates: judge 87/87; sweep 0; tests 1143; G3 smoke vs tclsh
# byte-exact on string-form preservation (007/+5/-0/1.50/.5/1e3, octal
# expr 010->8), negative ints through the cache edges (-128 cached,
# -129 lazy), proc defaults, apply lambdas, pool overflow at 512 (>
# 520 distinct units then literal re-check), post-overflow literals,
# pooled-literal mutation independence (lappend COW); engines identical.
# Bench (interleaved, machine loaded — ranges wide): literal-heavy
# micro (proc mk {} { list <16 literals> }, 300k calls) 253ms -> 169ms
# (~1.5x); fib25 73.5 vs 73.3 (neutral); fe/dloop neutral in noise.

# ---------------------------------------------------------------------------
# G4 2026-10-04: var fast-path empty-set guards (G4a) + dict Fx hashing with
# fmix64 finalizer (G4b).  G4a: get_var/set_var/take_var_fast/incr_var_fast
# guarded their upvar/array sidecar probes with set-emptiness checks (an
# empty set can't contain the name — pure equivalence), removing 1-2
# sidecar hash probes per var access on common frames.  G4b: DictMap's
# Ordered/Unordered moved from std SipHash to a local Fx multiply-mix
# (rtcl-vm std-only, dependency direction forbids importing varmap's).
# COLLISION POST-MORTEM (why G4b carries an fmix64 finalizer): the first
# cut returned the raw multiply-mix hash, and hashbrown picks buckets from
# the LOW bits — which a multiply-mix leaves dependent only on the first
# ~2 bytes.  dloop's keys all share the 3-byte prefix "key" (digits start
# at bit 24), so all 300k keys were congruent mod the 2^19 table mask:
# every insert into one probe chain, insert_full 30% of profile, dloop
# 0.85s vs 0.53s (a REGRESSION, caught by the interleaved A/B gate).
# The variable tables never hit this (small tables, early-distinct short
# names).  Fix: `finish` runs murmur3 fmix64 (bijection, full avalanche —
# low bits depend on ALL key bytes; ~2 multiplies).  Iteration order is
# unaffected: Ordered keeps insertion order, Unordered's was never
# specified (`dict create -unordered` is an rtcl extension).
# Gates: judge 87/87; sweep 0 diffs (strict); tests 1143; feature matrix
# green (embedded + no-default + std variants); dict smoke vs tclsh
# byte-exact three-way (create/get/set/unset order incl. overwrite-position
# and reinsert-at-end, nested set/unset, dict with/lappend/incr/append,
# merge/filter/map/replace/update, dict for order, keys/values/size,
# 40-key insertion order, nested-list values, error texts+errorInfo) —
# and byte-identical against the G3 binary (pre-change oracle).
# Bench (interleaved best-of-5, load ~7): dloop 0.50 -> 0.44s (~12%,
# G4b's target); g3_lit 0.33 -> 0.28 (~10%); fib25/fe_bench neutral;
# dict_ops 11 -> 10ms, list_ops 11 -> 10, var_incr 47 -> 44, proc_fib
# 16 -> 15 (all same-or-better within granularity).

# ---------------------------------------------------------------------------
# G5 2026-10-04: dispatch-path probe cleanup, all from a symbolized fib25
# profile on the G4 build: (a) cmd_cache root fast path — resolution at
# global scope ("::", every plain proc call) probed a two-level map, the
# outer probe re-hashing "::" per dispatched command (3.11%); a dedicated
# cmd_cache_root map makes it one probe (shares cmd_cache_len, the gen
# ageing, and the overflow clear).  (b) const_pool identity hasher — the
# pool is keyed by ByteCode address (unique, well-spread) but hashed with
# std's keyed SipHash (RandomState::hash_one 1.51% per exec_bytecode
# probe); PtrHasher (write_usize = move) removes it.  Embedded builds
# keep BTreeMap (cfg-split field).  (c) "::" substring scans off the hot
# paths: str::contains/rfind("::") pays StrSearcher::new setup for a
# two-byte needle on 1-4 byte names — has_ns_sep (windows(2)) in
# get_var/set_var/take_var_fast/incr_var_fast, rfind_ns_sep in
# ns_of_qualified (per proc call).  StrSearcher::new went 4.35% -> ABSENT,
# cmd_cache_get 3.11% -> 0.58%, hash_one 1.51% -> ABSENT.
# Gates: judge 87/87; sweep 0; tests 1143; feature matrix green (incl.
# embedded); dict+g3 smokes byte-exact vs tclsh.  fib25 x10 batch: 819
# median -> ~779-809ms (~3% wall; the profile deltas are the reliable
# signal — 80ms granularity per run hides it).  dloop/fe neutral.
# Remaining fib hotspots for the next profile round: call_proc 24.9%
# (share inflated by the shrunken rest), memmove 3.9%, bignum int_rep
# 2.5%, as_int 2.5%, pop_val 2.4%.

# ---------------------------------------------------------------------------
# G6 2026-10-05: OO method-body compilation (memo) + bare `self` parity.
# exec_chain_entry rebuilt the synthetic method ProcDef per call — two
# full-body String copies (link_prefix push_str + Rc::from), a fresh
# params Vec, compiled: None (call_proc then eval'd through the bytecode
# cache's full-text probe), for EVERY method invocation.  tclsh compiles
# a method body once and keeps it on the method record.  Change:
# `MethodDef.proc_memo: Rc<RefCell<Option<(String, Rc<ProcDef>)>>>` —
# filled on first call when the defining owner declares NO `variable`s
# (the assembled body is then def.body verbatim, a pure function of
# def + invoked method name; keyed by the typed name because params[0]
# feeds the arity-usage text and `info level 0`).  Owners WITH variables
# keep the rebuild (the prefix follows the live variable lists).
# Invalidation is structural: (re)definition installs a fresh MethodDef
# (set_method), `export` toggling is the only in-place mutation and
# never enters the memo; the shared ProcDef is never written back (OO
# statics are always empty, so call_proc's Rc::make_mut statics
# write-back cannot fire) and its compiled form goes through call_proc's
# per-call epoch/applicability gate exactly like a named proc's — plus
# it unlocks the E2 slot-locals binding for method bodies.
# Also fixed (found by the smoke, pre-existing, both engines): `self`
# with no arguments now returns the object's qualified name (tclsh
# parity; it was `wrong # args: should be "self subcommand"`).  Probes
# never caught it — the sweep is engine-vs-engine and both arms erred.
# Gates: judge 87/87; sweep 0 (incl. new probe oo17); tests 1143/0;
# feature matrix: std variants + rtcl-vm no-default + wasm32 green
# (rtcl-core `embedded` no_std is broken PRE-EXISTING — 2231 errors at
# G5 too — unchanged, not this round's doing).  OO smoke (oo17.tcl,
# 15 sections: memo/rebuild paths, my/next, redefinition mid-flight,
# export toggling after memo fill, forward, ctor/dtor, oo::copy,
# recursion through the shared ProcDef, tailcall in a memoised method,
# arity text, info level 0, error framing) byte-exact vs tclsh 8.6.17.
# Bench (1M method calls, interleaved best-of-3, load ~3): memo path
# 1543 -> 1317ms (~15%); vars path 2652 -> 2543 (unchanged path, noise);
# tclsh 393/449 — OO call gap 3.9x -> 3.3x.  fib25/dloop neutral.
# Remaining OO per-call cost for the profile round: build_chain rebuilds
# the resolution order per call and ActiveMethod clones the whole chain
# (chain.to_vec() — every ChainEntry's params Vec + body String) per
# invocation; oo::object dispatch is ~3.3x tclsh while method BODIES now
# match named-proc speed.

# ---------------------------------------------------------------------------
# G7 2026-10-05: the profile round (frame-pointer symbolized builds at
# /tmp/g7sym, callchains via `perf script` frame-walking), plus the two
# certain fixes it found in the expr layer.
#
# fib25 (5k samples): exec_op 14.9% + exec_bytecode 13.1% + call_proc
# 12.1% (the call machinery is ~40% of fib); memmove 9.5% (mostly with
# dispatch_values as the direct caller — the per-call args/level0 Vec
# copies); dispatch_values self 4.2%; FxHasher::write 3.9%;
# numeric_binop 3.0%; from_int 2.9%; as_float 2.8%; cmd_cache_get 2.7%;
# as_int 2.4%; memcmp 2.3%; pop_val 2.0%; int_rep 1.5%; numeric_cmp
# 1.3%.  dloop: dict IndexMap ops ~33% (600k hashed dict set/get —
# inherent work); the VARIABLE layer ~15% (globals get 4.1% + globals
# get_mut 2.6% [symbol ICF-folded with the procs map instantiation] +
# array_globals VarSet contains_key 4.9% + set_var/get_var self +
# split_array_ref 0.8% + canonical_global_in 0.8%) — the array_globals
# probe runs on every global-scope var op because env/tcl_platform make
# that set permanently non-empty.  fe_bench: exec_op 32.9% dominant;
# frame_slot_write 3.6%; foreach_bind 3.0%; Rc<ValueInner>::new 3.0%
# (from_int outside the small-int cache — inherent, tclsh allocates per
# result too); drop_glue 1.6%.
#
# Two fixes from this (both semantics-preserving, probed against tclsh):
# (a) numeric_binop's NaN guard ran as_float on BOTH operands BEFORE the
#     integer fast path — a string-rep operand (every bracket result in
#     `expr {[fib ...] + [fib ...]}`) was float-parsed, then int-parsed:
#     double parse per `+`.  The guard now sits after the both-int arm
#     (an integer never carries NaN; Int + "nan" skips the int arm and
#     still errors exactly as before).
# (b) numeric_cmp allocated TWO BigInts per comparison (to_big on both
#     int reps even for I64×I64) — every loop condition `$i < $n` paid
#     two allocations per iteration.  I64×I64 now compares directly.
# Bench (interleaved x10 batches, best-of-3, load ~7): fib25 0.76 ->
# 0.70s (~8%); fe_bench 1.01 -> 0.94 (~7%); dloop neutral (no cmp/binop
# in its loop — dict get/incr only).  Gates: judge 87/87; sweep 0
# (incl. new probe numcmp1: NaN operand orders, i64 overflow widen,
# i64::MIN /-1 and %-1, hex/octal/binary operands, cross-rep equality,
# string operands, div-by-zero texts) byte-exact vs tclsh 8.6.17;
# tests 1143/0.
#
# Ranked next-round candidates (all profiled, none taken this round):
# 1. Call-site command tokens in ByteCode (tclsh's model): the compiled
#    Call op would carry its resolved target + epoch, folding
#    cmd_cache_get (2.7%) + the dispatch_values probe chain + the
#    procs.get re-fetch (~19% of fib all told).
# 2. Array marker inside the globals/locals tables (rep variant or a
#    tagged base entry): kills the array_globals contains probe and the
#    second table probe per global var op (dloop var layer ~15% ->
#    ~7%).  Architecture change — the base-key Value::empty() marker
#    and every loc_* path would move.
# 3. Zero-copy frame level0/args plumbing (fib memmove 9.5%): the
#    dispatch scratch already holds the words; call_proc rewrites
#    level0 from it per call.
# 4. OO chain caching (G6 follow-up): build_chain + chain.to_vec()
#    per method call is the remaining OO dispatch gap (3.3x tclsh).

# ---------------------------------------------------------------------------
# G8 2026-10-05: the four-angle optimization round (architecture
# re-audit / flamegraph tooling / data-path copies / Rc-vs-String
# sharing).  Root causes were established byte-precisely before any
# change: DWARF-callchain perf (`--call-graph dwarf`) attributed the
# fib memmove bucket to pool-pop/frame-push/frame-pop/pool-push, and
# disassembling the memmove call sites read the size registers —
# `mov $0x108,%edx` = 264B (CallFrame), `mov $0xa8,%edx` = 168B
# (VmState) — so the copies were the POOL ELEMENTS themselves, four
# moves per proc call (CallFrame) and two per exec_bytecode (VmState).
#
# Changes (all semantics-preserving, gates below):
# (a) frames/frame_pool as Vec<Box<CallFrame>>: the pool moves 8-byte
#     pointers; a box is allocated only at a new recursion depth.
# (b) VmPool as Vec<Box<VmState>>: same (the 168B take/give memcpys).
# (c) Call-site command tokens (G7 candidate 1, tclsh's resolved
#     `Command *` model): each compiled Call/DynCall op resolves
#     through a per-op slot on the const-pool entry (keyed by op pc,
#     binary-searched) instead of cmd_cache_get in dispatch_call AND
#     dispatch_values (two Fx probes) + the 7-arm chain.  Validity =
#     command-table generation + invocation name + resolving ns —
#     the exact cmd_cache key; ensemble/unknown never cache; exec
#     traces and pool eviction degrade to the full path; backfill
#     from what the full path cached after every miss.  Proc defs
#     are still re-fetched per call (statics write-back freshness).
# (d) MethodDef { params: Rc<[(String, Option<String>)]>,
#     body: Rc<str> }: build_chain/chain clones and constructor
#     synthesis become Rc bumps.
#
# Flamegraph tooling (kept for the next rounds): /tmp/flamegen.py —
# `perf script` -> collapsed stacks + SVG (usage: `python3 flamegen.py
# IN.script OUT.svg --collapsed OUT.collapsed`); profile builds need
# CARGO_PROFILE_RELEASE_STRIP=false (workspace release profile strips)
# + `-C force-frame-pointers=yes`; DWARF mode for exact attribution
# when FP unwinding misattributes through non-FP helper frames.
#
# Profile delta (fib25x25, 1303 samples vs G7's 1800 = ~27% less CPU
# for the same work): cmd_cache_get 2.7% -> 0.00%; memmove 9.5% ->
# 0.38% (background); dispatch_values/dispatch_dynamic (was the
# dominant dispatch layer) -> 4.07% each (first-call resolution +
# backfill only — the token fast path sits directly under exec_op);
# fib is now arithmetic-bound: arith 11.7%, numeric_binop 8.9%,
# rel 8.3%, as_int 6.5%, find_inner 6.0% (the single procs.get
# re-fetch), Rc clone/inc_strong ~4.4% each (inherent refcounting).
#
# Bench (interleaved batches, best-of-3, load ~3.5): fib25 x10-batch
# 746 -> 609ms (-18%; tclsh 357ms — gap 2.09x -> 1.71x); oo_bench2
# 1337 -> 1235ms (-7.6%); fe_bench / dloop neutral (both non-
# call-bound — the tokens/pools never fire in their hot loops).
# Gates: judge 87/87 (final binary re-confirmed); sweep 0 diffs;
# tests 1143/0 (the two `unused import: Rc` warnings are pre-existing
# test-module imports, oo.rs:2557 / namespace.rs:2048); feature
# matrix: rtcl-core std variants x6 + rtcl-vm no-default + wasm32
# green; OO smoke byte-exact vs tclsh 8.6.17.
#
# G7 ranked-candidates ledger: #1 (call-site tokens) DONE above.
# #3 (zero-copy level0) is OBSOLETE — the memmove was the frame
# struct itself, now boxed; level0's pooled buffer was already
# copy-free.  #2 (array marker inside the var tables, dloop var
# layer ~15% -> ~7%) and #4 (OO chain caching, build_chain rebuilds
# per call) remain open.

# ---------------------------------------------------------------------------
# G8b 2026-10-05: OO chain memo — G7 candidate 4.  The oo_bench2 profile
# (post-G8) showed build_chain at 33.0% of the whole benchmark (linearize
# alone 19.6%: a C3 merge allocating a Vec<String> per class PER CALL,
# plus per-class mixins.clone() and Owner::Class String compares), and
# exec_chain_entry's ActiveMethod push cloned the chain again per call.
#
# Change: OoState gains mutation_ctr + chain_memo ((key, method,
# include_private) -> (stamp, Rc<Vec<ChainEntry>>), capped 512
# clear-on-overflow) — the cmd_generation/cmd_cache pattern applied to
# method resolution.  invoke_method serves every hit without walking;
# ActiveMethod.chain is now the shared Rc (next's invocation-time
# snapshot semantics unchanged, minus the to_vec).  Every structural
# mutation bumps the stamp: the four method-table helpers (set_method /
# set_export / take_method / delete_method), the seven define arms
# (mixin, superclass, constructor, destructor, unexport-hidden,
# variable, filter), class/instance create, oo::copy, and destroy.
# Audit: `interp.oo` is written ONLY in oo.rs (crate-wide grep); the
# two init-time inserts precede any possible memo.  Redefinition-mid-
# flight, export/unexport, mixin add/remove, superclass redefinition,
# next-after-redefine, deletemethod/renamemethod, destroy cascade, and
# unexport-of-inherited all verified byte-exact vs tclsh 8.6.17 (new
# probe judge/probes/oo18.tcl, 11 sections).
#
# Profile: build_chain 33.0% -> 0.00%, linearize 19.6% -> 0.00%
# (chain_for's memo probe costs 3.0%); oo_bench2 samples 1235 -> 799
# for the same work.  Bench (interleaved best-of-3, load ~3):
# oo_bench2 1311 -> 765ms (-42%; tclsh 393ms — OO call gap 2.9x ->
# 1.95x, from 3.3x at G6); oo_bench2_vars 2555 -> 1895ms (-26%);
# fib25 neutral.  Gates: judge 87/87; sweep 0; tests 1143/0; feature
# matrix green (std variants, rtcl-vm no-default, wasm32).
#
# Remaining open candidates: #2 array marker inside the var tables
# (dloop var layer ~15% -> ~7%); oo dispatch's cmd_oo_object entry
# (typed/key String allocs per call) now the visible OO cost.

# ---------------------------------------------------------------------------
# G8c 2026-10-05: vars-path memo unification + a pre-existing multi-
# variable link_prefix bug found BY the new bench gate.
#
# The worst remaining gap was oo_bench2_vars (class methods whose owner
# declares `variable`s): exec_chain_entry rebuilt + recompiled the
# synthetic ProcDef EVERY call (fresh ProcDef has no compiled form, so
# the body also ran tree-walked) — the memo only covered the no-vars
# case because the prefix "follows the live variable lists".  But the
# assembled body is a pure function of (def, method, owner vars): the
# memo now keys on the typed method name PLUS a snapshot of the live
# `variable` list (a `variable` declaration change diverges the
# snapshot and rebuilds; the define word also bumps mutation_ctr,
# retiring the chain memo).  owner_variables now returns a borrowed
# slice (no Vec clone per call).
#
# BUG (pre-existing, found while validating the memo): link_prefix
# emitted `variable a b;` for the names [a, b] — at the Tcl level that
# is the PAIRED declare-with-initial-value form, so `acc` was clobbered
# with the literal string "extra" and `$acc` read "extra" (probed
# tclsh: methods link each name separately).  Any class with TWO
# declared variables broke arithmetic on the first one (old binary:
# `expected integer but got "log"`).  link_prefix now emits one
# `variable <name>;` per name; ctor/dtor paths share the fix.  New
# probe oo19.tcl (7 vars-memo sections + ctor/dtor with two variables)
# byte-exact vs tclsh 8.6.17.
#
# Sweep flake resolved: judge/probes/objquad prints measured wall time
# (clock seconds deltas around an 80k list-nest loop sitting near the
# 1s print boundary) — under sustained load it flapped engine-vs-engine
# 5/8 rounds, BOTH directions (captured artifacts /tmp/sweep_flaky/).
# It and objtime (same pattern, ms granularity) are perf probes, not
# semantics probes: both are now excluded from the differential sweep,
# alongside pw/objmech etc.  The sweep itself is finally committed as
# judge/sweep.sh (was a /tmp session artifact; 8-round capture loop
# confirmed objquad was the ONLY flaky probe — zero other diffs).
#
# Bench (per-run ms, interleaved best-of-3, load ~2): oo_bench2_vars
# 2590 -> 1682 (-35%; tclsh 421 — gap 6.2x -> 4.0x; remaining cost is
# the per-call `variable` link execution + degraded name-keyed frame,
# tclsh compiles the link into the frame); oo_bench2 1313 -> 772
# (1.97x tclsh); fib25 72 -> 55ms/run (gap 2.0x -> 1.53x); dloop
# 396 -> 370 (-7%, secondary).  Gates: judge 87/87; sweep 0
# (judge/sweep.sh); tests 1143/0; feature matrix green (std variants,
# rtcl-vm no-default, wasm32).

# ---------------------------------------------------------------------------
# G8d 2026-10-05: `variable`-link resolution memo.  The oo_bench2_vars
# DWARF profile at G8c: cmd_variable 24.2% (under it qualify 11.4% +
# format 10.7% + normalise 9.7%) — a hot method body re-declares its
# `variable` links EVERY call (the link prefix is part of the body
# text), and each declaration re-ran the qualified-key + parent-ns
# computation: three string allocations + scans per variable per call.
# The resolution is a pure string function of (current namespace,
# declared name) — memoised in Interp::var_link_cache (nested
# VarMap, the cmd_cache shape; bounded 1024 clear-on-overflow; NO
# invalidation needed).  The live checks stay per-call: parent-exists
# probe, the owning namespace's variable-set insert, the per-frame
# upvar link + degrade.  (UpvarLink::Global stays String — Rc<str>
# would touch every consumer for one clone.)
#
# Bench: oo_bench2_vars 1682 -> 1513ms (-10%; cumulative vs G8 base
# 2632 -> 1513 = -43%, gap 6.2x -> 3.6x tclsh); oo_bench2 742ms
# (1.9x); fib25 53ms/run (1.47x).  Remaining vars-path cost: the
# per-call upvar degrade forces the whole compiled body name-keyed
# (frame_slot_* fallbacks + upvar indirection + set_var's array
# probes) — the linked-slot model (slot cell = Value | Link) is the
# structural follow-up, same class as the array-marker refactor.
# Gates: judge 87/87; sweep 0; tests 1143/0; feature matrix green.

# ---------------------------------------------------------------------------
# G8e 2026-10-05: linked slots — the whole-frame upvar degrade is gone
# for link commands.  Installing a link (`upvar` / `global` /
# `variable`) flushed EVERY slot value into the name map and dropped
# the slot table, demoting the whole frame to name-keyed lookups — in
# an OO method body whose `variable acc;` link re-installs per call,
# every x/y/param op paid map hashing for the sake of one linked name.
# Now: degrade_frame_link moves ONE cell into `locals` and sets a
# parallel `slot_aliased` bool; the three frame_slot_* ops and
# CallFrame::slot_value consult it (an indexed read, no hashing) and
# defer to the name path, which resolves the link through its existing
# (judge-proven) upvar machinery.  The name-path fast paths already
# self-guard on `upvars.contains_key`, so no other site needed edits.
# Scope: only the three link commands alias per-name (their storage
# change is name-scoped); trace/array-ification/unset keep the
# wholesale flush (frame-wide storage-kind change, cold paths), and
# upvar-to-a-CALLER-frame local keeps it too — the target frame has no
# upvar entry for its own name, so per-name aliasing there would let
# the caller's compiled stores resurrect the flushed cell.  Enumeration
# sites (info vars/locals/frame) already degrade wholesale first, so
# the linked name is visible exactly as before.
#
# Profile (oo_bench2_vars): cmd_variable 24.2% -> 12.9% (the remaining
# cost is the live per-call checks: parent-exists, ns variable-set
# insert, upvar install — all semantics); set_var 16.4% / hashing 13%
# / resolve_object_key 7.5% are the structural floor of the current
# design (tclsh's compiledLocals hold direct pointers, no hashing).
# Bench: oo_bench2_vars 1513 -> 1407ms (cumulative vs G8 base 2632 ->
# 1407 = -46%, gap 6.2x -> 3.3x tclsh); oo_bench2/fib/dloop unchanged.
# Gates: judge 87/87; sweep 0; tests 1143/0 (the test_slot_* battery
# covers upvar/global/variable/array/trace degrades); all 7 var/OO
# probes byte-exact vs tclsh 8.6.17.

# ---------------------------------------------------------------------------
# G8f 2026-10-05: the OO dispatcher's entry tax.  oo_bench2 profile at
# G8e: cmd_oo_object 65% inclusive, under it resolve_object_key 13.4%
# (object_key_candidates 10.9%: `qualify` + `format!("::{}")` allocate
# candidate Strings per call, then two owned-String table probes) —
# plus chain_for's flat (String, String) memo key allocating two
# Strings per PROBE, plus cmd_oo_object's own typed/method .to_string()
# and a Vec clone of the call args.
#
# Changes: resolve_object_key memoised (OoState::key_memo, nested
# ns->name maps probed by &str, valid under mutation_ctr — every
# objects/classes mutation bumps it per the G8b audit, and `rename`
# does not touch the oo tables, so the pure-function argument is
# closed); chain_for split into nested chain_memo/chain_memo_priv
# (zero-alloc hit); both dispatchers borrow the typed name, method
# word and call-arg slice (nothing needs to outlive the dispatch — the
# ActiveMethod snapshot owns its copies).  New probe oo20 (memoized
# miss -> object created, destroy -> recreate as another class,
# qualified form).  NOTE (pre-existing, both engines diverge from
# tclsh): rtcl objects do not follow `rename` (tclsh: renamed object
# command keeps working) — old and new binaries agree; recorded as a
# follow-up, not this round's business.
#
# Bench: oo_bench2 787 -> 688ms (-13%; cumulative vs G8 base 1521 ->
# 688 = -55%, gap 3.9x -> 1.76x tclsh); vars/other benches unchanged.
# Gates: judge 87/87; sweep 0; tests 1143/0; feature matrix green.

# ---------------------------------------------------------------------------
# G8g 2026-10-06: const-pool eviction had no way back.  The dloop profile
# at G8f showed the FULL dispatch chain (dispatch_site -> dispatch_call ->
# dispatch_dynamic -> dispatch_values -> cmd_foreach) at ~35% — the
# call-site tokens were never filling because st.cmd_sites was None: the
# const_pool (cap 512, clear-on-overflow) is filled by every compile
# seam INCLUDING the stdlib load's several hundred proc bodies, and the
# clear wiped the top-level unit's entry AFTER its insert; cached units
# never re-inserted, so every PushConst re-materialised its literals and
# every dispatch took the full chain FOR THE REST OF THE SESSION.
# (fib25x25 was unaffected — few units — which is why the G8 profile
# looked clean while dloop paid.)
#
# Fix: exec_bytecode takes &Rc<ByteCode> (all three callers hold one)
# and re-inserts on a pool miss — any eviction heals at the unit's next
# execution; the cap also rises to 2048 (a stdlib load alone is worth
# several hundred entries).  dloop 407 -> 364ms (gap 1.7x -> 1.51x
# tclsh); oo_bench2 688 -> 659; fib/fe unchanged.
# Gates: judge 87/87; sweep 0; tests 1143/0; feature matrix green
# (spot: no-default + wasm32).

# ---------------------------------------------------------------------------
# G9 2026-10-06: VPtr — the free-list is back.  jimtcl's Jim_Obj carries
# free-list pointers (the Rust port's comment explicitly dropped them in
# favour of the allocator); the fib/fe/dloop profiles kept showing the
# allocator's share (~10-25%: Rc::new + drop glue + tcache paths) for a
# value population that is overwhelmingly create-consume-drop within one
# interpreter step.  Value now holds VPtr, a hand-rolled Rc<ValueInner>
# whose dead boxes go to a thread-local free list (cap 8192 boxes):
# clone/drop/make_mut/strong_count/ptr_eq reproduce the Rc semantics,
# Deref keeps every accessor, Value stays 8 bytes, !Send/!Sync as Rc
# was — the change is fully contained in rtcl-vm/src/value.rs (the only
# file touching ValueInner; make_mut/strong_count had no callers
# outside).  Payloads (string buffers, list/dict vecs) still drop
# normally; only the BOX recycles.
#
# BUG the judge caught mid-round (and the process lesson): the worktree
# judge had been running a STALE binary all day — judge/sweep "87/87"
# verdicts for G8c..G8g were the Oct-5 G8 binary passing (the rsync
# excludes target/).  The freshly-synced binary failed gen/gen_dict
# + gen/gen_obj with SIGSEGV: VPtr::drop freed the payload with
# drop_in_place and then, on the freelist-full path, did
# drop(Box::from_raw(p)) — Box's own drop ran drop_in_place a SECOND
# time.  Double free, firing only above the 8192-box cap (the 10000-
# element dict-24.2x corpus cases), which is why every in-process test
# (small populations) stayed green.  Fix: bare
# alloc::dealloc with the ValueInner layout.  Worktree discipline is
# now: rsync sources, cp target/release/rtcl into the worktree, then
# judge.
#
# Bench: fib25 58 -> 51ms/run (gap 1.6x -> 1.42x); dloop 343 -> 324
# (1.42x -> 1.34x); fe_bench 88 -> 82 (2.9x -> 2.65x); oo_vars 1407 ->
# 1281 (3.3x -> 3.0x); oo_bench2 ~flat (load noise).  Memory: churn
# test RSS stable (freelist cap bounds retention at ~0.6MB).  Gates:
# judge 87/87 (real binary); sweep 0; tests 1143/0; probes oo18/19/20
# byte-exact.

# ---------------------------------------------------------------------------
# G9b 2026-10-06: the call-site token carries the proc def.  The fib
# profile at G9 showed the per-call `procs[key]` re-fetch at ~14%
# inclusive (hash + key compare + find_inner) — the token stored the key
# but re-resolved the def every call because the statics write-back
# replaced the map entry WITHOUT bumping cmd_generation (the one
# unbumped `procs` mutation).  Both write-backs now bump (one u64 per
# write-back — statics-bearing procs are never hot-loop material), and
# CmdSite gains `def: Option<Rc<ProcDef>>`: a proc verdict rides the
# token straight into call_proc, the probe runs on the cold backfill
# path only.
#
# Bench: oo_bench2 696 -> 638ms (every proc call benefits, OO method
# bodies included); fib25 54-55 vs 70ms old (1.6x tclsh).  Gates:
# judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G9c 2026-10-06: three per-call constant costs.  (a) VmState remembers
# the unit whose consts/cmd_sites it holds (identity compare) —
# recursion re-borrows its own state, so the per-call const-pool hash
# (~7% of fib) disappears on the hot path; eviction healing unchanged.
# (b) the E2 slot-gate's param/table alignment moved to COMPILE time
# (ByteCode::params_aligned, computed where the table is seeded — the
# runtime gate used to re-verify with a zip walk per call).  (c) the
# call-site token's namespace check takes an Rc::ptr_eq fast path (the
# root namespace is one shared Rc).
#
# Bench: oo_bench2 638 -> 607ms (1.55x tclsh); fib neutral-to-slightly
# better (53-55 vs 70ms old).  Gates: judge 87/87; sweep 0; tests
# 1143/0.

# ---------------------------------------------------------------------------
# G9d 2026-10-06: the OO variable prelink.  The vars-method profile's
# remaining cmd_variable share (12.9%) was the dispatched `variable`
# command per name per call — the link prefix in the assembled body.
# The prefix text is GONE: memoised_proc_def now returns the link list
# (owner vars minus param-bound names), exec_chain_entry hands it to
# call_proc via frame_prelink, and frame setup installs the exact same
# triple cmd_variable performs (namespace variable-set insert, link-only
# slot aliasing, upvar entry, seed-if-exists) via the shared
# resolve_var_link memo.  Unreachable-error note: declaration names are
# validated by the define word and object namespaces always exist, so
# the parent-exists check cannot fail on this path.  Consumed once per
# call (taken after the recursion guard — no leak into unrelated
# calls), and the tailcall loop's upvar clear matches the old
# prefix-inside-the-body lifecycle.
#
# Bench: oo_vars 1281 -> 1209ms (2.87x tclsh); oo_bench2 within noise.
# Gates: judge 87/87; sweep 0; tests 1143/0; all 8 var/OO probes
# byte-exact vs tclsh 8.6.17.

# ---------------------------------------------------------------------------
# G9e 2026-10-06 (NEGATIVE RESULT, reverted): fused slot-arithmetic
# superinstructions.  The peephole window [LoadLocal, LoadLocal, Add,
# StoreLocal] -> AddSlotSlot never fires in real Tcl: the accumulator
# pattern is always `set s [expr {$s + $i}]`, and the bracket word
# compiles to SubMark .. SubEnd AROUND the inner expr ops — the
# markers are the error-framing regions (they defer the outer `set`
# command's errorInfo frame exactly like the tree-walk's CommandSub
# boundary), so the four ops are never adjacent and removing them
# would change error attribution.  Where the window could fire, the
# bench showed nothing (fe/fib flat; a 3M-iteration micro was within
# noise).  Reverted before commit; the negative result is the
# record — the remaining fe_bench gap lives in per-op bookkeeping and
# value lifecycle, not in op fusion.

# ---------------------------------------------------------------------------
# G9f 2026-10-06 (design note, parked): global-scope foreach inlining.
# The locals_mode gate on foreach/lmap is the per-iteration body-eval
# overhead at top level (fe_bench/dloop outer loops).  The tree engine
# ALREADY routes inlinable-shape foreach through foreach_lexical
# (compiled semantics) at global — BUT the routing is context-
# conditional at RUNTIME (foreach_inline_shape consults
# cur_cmd_word_srcs; probe-pinned: catch-wrapped lmap at global stays
# dispatched-shaped), while a compiler inline is unconditional.  Doing
# this right needs either a runtime bail in ForeachStart (re-evaluating
# the body from its site span, with framing parity) or exact mirroring
# of the context-conditional routing in the executor.  Both are
# next-major-project sized; parked, not attempted.

# ---------------------------------------------------------------------------
# G9 state board (2026-10-06, post-G9d): micro suite at parity or better
# (arith .8x, dict .8x, list .8x, string_build 1.0x, var_incr .7x —
# BELOW 1 means rtcl is faster; proc_fib micro 1.3x).  Heavy benches vs
# tclsh 8.6.17: fib25 48ms/run ~1.4x; dloop 333ms 1.38x; oo_bench2
# 638ms 1.63x; fe_bench 85ms 2.7x; oo_bench2_vars 1229ms 2.9x.  The arc
# from the session start (10-85x at bd3e169): every remaining gap has
# been decomposed to per-call semantic operations (hash probes,
# refcounts, error-frame bookkeeping) — tclsh's residual edge is
# pointer-stable compiledLocals (zero hashing on linked variables),
# inline-expr object reuse, and four decades of constant-factor tuning.
#
# The mapped next-major projects, in expected-value order:
# 1. Inline-int Value (tagged pointer / NaN-box): makes numbers
#    immediate — zero alloc, zero refcount.  The wall is as_str(&self)
#    -> &str for an immediate (needs Cow migration across every
#    consumer, thousands of sites) or a render-ring footgun.  Biggest
#    single lever left.
# 2. Pointer-stable variable storage (slotmap-style tables with
#    hash-carrying lookups): zeroes the hash chains on linked/global
#    variable access (vars-bench remaining cost).  Touches VarMap's
#    every consumer.
# 3. Global foreach inlining (see the parked design note): kills the
#    per-iteration body-eval machinery at top level (fe/dloop).

# ---------------------------------------------------------------------------
# G10 2026-10-06: the release profile was optimizing for SIZE.  The
# workspace [profile.release] carried opt-level = "s" from its original
# setup — the interpreter loop (exec_op's giant match, value ops, the
# call path) was size-tuned, not speed-tuned.  perf stat on a 3M-
# iteration while+add loop: rtcl 5550M instructions vs tclsh 2028M
# (2.74x) — the instruction COUNT gap said "more work", and opt-level 3
# attacks the codegen half of it.
#
# Bench (O3 vs s, same sources): fib25 51 -> 44ms (gap 1.3x tclsh);
# dloop 360 -> 325 (1.17x!); oo_bench2 623 -> 572 (1.33x);
# oo_bench2_vars 1214 -> 1049 (2.37x); fe_bench flat (noise band).
# Binary 3.6 -> 4.5MB (+25%).  The wasm size target: rebuild with
# CARGO_PROFILE_RELEASE_OPT_LEVEL="z" (cargo env override) when code
# size matters — the profile keeps speed for native.
# Gates: judge 87/87; sweep 0; tests 1143/0; wasm32 check green.

# G10 verified board (same-session tclsh references, load ~2):
# fib 45/39 = 1.15x; dloop 297/253 = 1.17x; oo 562/416 = 1.35x;
# fe 96/46 = 2.1x; vars 1027/448 = 2.29x.  Micro suite: arith .8x,
# var_incr .6x, string_build .9x, proc_fib .9x, dict/list 1.1x.

# ---------------------------------------------------------------------------
# G10b 2026-10-06: latent foreach-target corruption + the global-inline
# verdict.  The experiment (drop foreach's locals_mode gate, inline at
# top level) crashed immediately — not from the routing problem mapped
# in G9f, but from strip_nops: its branch-target remap table (Jump /
# JumpTrue / JumpFalse / LoopEnter / CatchStart) predates the inline
# foreach and never remapped ForeachStart { next, end } / ForeachNext
# { body }.  ANY constant fold ahead of an inlined foreach shifted the
# loop's entry/exit by the strip count — latent since the inline
# foreach landed (proc bodies never folded ahead of a loop).  Fixed
# with the two missing remap arms.
#
# The global-inline verdict itself: TCLSH DOES NOT INLINE foreach at
# eval level (compiledLocals are proc-context) — the probe-pinned
# dispatched shape at top level IS tclsh's behavior, the original
# locals_mode gate was correct, and the tree's foreach_lexical routing
# is correctly scoped to lexical bodies.  Gate reverted; the sweep
# differential (bytecode-inline vs tree-dispatched) proved it in one
# run — exactly what the two-engine gate exists for.
# Gates: judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G11 2026-10-06: inline-int Value — feasibility PROVEN, migration
# scoped, deferred as its own effort.  The tagged representation
# (u64 word: bit0=1 -> immediate i63 via arithmetic shift, bit0=0 ->
# heap VPtr; |n| >= 2^62 falls back to a heap Int-rep box, full i64
# range preserved) was implemented and compiles clean through
# rtcl-vm (value.rs + execute.rs).  The repr makes from_int/set_int_rep
# allocation-free, as_int an untag, clone/drop copy/noop for the i63
# range; heap semantics (COW, freelist, singletons) unchanged.
#
# The migration wall, measured: flipping as_str to Cow<'_, str> yields
# 879 error sites in rtcl-core; iterating a span-driven auto-fixer
# (.as_ref() insertion at `expected &str, found Cow` labels) clears
# ~300 but the remainder splits into classes needing individual
# judgment: 286 `expected Cow, found &str` (re-wrap sites), 273
# Cow-method-not-found, 106 E0599, plus borrowck ripples.  Grinding
# those auto-mechanically risks subtle hot-site into_owned()
# regressions — deferred to a dedicated pass with per-file review.
# The proven core is preserved at bench/next/inline-int-value-core.rs.txt
# (compile-ready; drop into rtcl-vm/src/value.rs and run the fixer at
# /tmp/cow_fix.py pattern — span-driven E0308 label matching).
#
# Why it matters (evidence): the leanest while+incr loop profile puts
# as_int 12.5% + strict_bool 10.7% + drop 14.3% on value lifecycle;
# tclsh allocates its add results too, so immediates are the
# BEAT-tclsh move, not the parity move.  Per-op tax today ~20ns vs
# tclsh ~7ns across all ops.
#
# Also resolved this round: the build loop (for+lappend, 200k iters)
# decomposition — while+incr 2.4x tclsh, lappend adds 4.4x; diffuse
# per-component overhead (lappend 19%, incr 19%, set_var 17%, allocs
# 31%), no single lever.  var-bench pointer-stable storage remains
# the vars fix (VarMap is a bare alias; encapsulation first).

# ---------------------------------------------------------------------------
# G12 2026-10-06: INLINE-INT VALUE — the tagged repr landed.  Value is a
# Cell<u64> word: low bit set = an immediate i63 ([-2^62, 2^62); the
# extremes fall back to a heap Int-rep box, full i64 preserved), low
# bit clear = the recycled heap box (VPtr semantics unchanged).  The
# migration wall (879 as_str->Cow sites) DISSOLVED: as_str keeps its
# `-> &str` signature — an immediate whose string is first demanded
# materializes through the Cell into a heap box and borrows from it.
# Soundness: the upgrade is one-way and idempotent; every other &self
# method is read-only; clones bump the box count, so a borrowed &str
# outlives its origin correctly; PhantomData<*mut _> keeps Value
# !Send/!Sync.  from_int/set_int_rep/as_int are now allocation- and
# refcount-free on the i63 path; the small-int cache is subsumed
# (removed).
#
# Two use-after-free bugs caught by the crash gates during development
# (both "grab the box pointer without taking the rc" mistakes —
# empty()'s singleton and as_str's fresh-box temporary; both fixed
# with from_heap_clone / mem::forget ownership transfer) and one
# semantic break: as_list/as_dict on an immediate must DELEGATE to the
# string parse (an int's rendering is its list form — the old Int-rep
# `_` arm parsed), not short-circuit None (binary format c0 caught it).
#
# Bench: fe_bench 89 -> 66ms (gap 2.0x -> 1.5x tclsh); build loop
# 34 -> 27ms (was 160ms at G8); arith_loop micro 0.7x (FASTER than
# tclsh); fib 1.14x; dloop/oo flat.  Memory: materialization churn
# RSS stable 6.1MB.  Gates: judge 87/87; sweep 0; tests 1143/0;
# i64-extremes probe byte-exact (heap-fallback range); feature matrix
# green (no-default, rtcl-vm no-default, wasm32).

# ---------------------------------------------------------------------------
# G13 2026-10-07: the linked-slot key cache — Project 2's safe core.
# Aliased slots holding GLOBAL links (`variable`/`global`/upvar-to-
# global) now carry the canonical target key (CallFrame::link_keys,
# parallel to slots, installed by degrade_frame_link from the link
# kind — frame/dead links keep None and keep their name paths).  Slot
# ops on those names resolve through ONE globals probe with the cached
# key instead of the name path's two hashes (upvars probe + globals
# probe); incr mutates in place through the same single probe.  No
# unsafe, no generation counter — the cached key is immutable.  (The
# full pointer-stable storage remains available if the last hash ever
# matters; the safe subset covered the measured gap.)
#
# Bench: vars 1119 -> 791ms (2.45x -> 1.81x tclsh); dloop 284 vs 277 —
# AT PARITY (1.03x); fe ~2.0x, fib 1.3x, oo 1.28x (same-session
# references, both arms drifted up with load).  Gates: judge 87/87;
# sweep 0; tests 1143/0.

# G13 CORRECTION (2026-10-07): the /tmp bench files were wiped between
# sessions — several post-G13 readings ran against missing files (error
# exits swallowed by >/dev/null).  The TRUE board with recreated files:
# fe 111/46 = 2.4x; fib 49/28 = 1.75x; dloop 270/246 = 1.10x; oo
# 643/399 = 1.61x; vars 815/438 = 1.86x; build 30/17 = 1.76x.  (The
# pre-wipe "fib 1.26x / oo 1.28x" claims were optimistic.)  dloop is
# the one AT-PARITY bench.  Bench scripts now belong in the repo, not
# /tmp.

# G13b (2026-10-07): proc-call overhead BEATS tclsh.  Layer
# decomposition (300k-iteration loops, wall clock, load-noisy ±15%):
# empty-proc loop rtcl 73ms vs tclsh 88 (0.83x); 1-arg proc 49 vs 94
# (0.52x — 1.9x FASTER); 1-arg+expr 77 vs 106 (0.73x).  The
# dispatch_site token + frame pool + slot binding + exec_bytecode
# pipeline is now cheaper per call than tclsh's INST_START_CMD path.
# fib's remaining 1.75x is therefore per-op weight on the ~12 body ops
# (condition/subs/add), not call overhead — and profiling is currently
# BLOCKED (kernel.perf_event_paranoid=4), so the next per-op steps
# (inline-int arithmetic ops are in; the dispatch-loop slimming is the
# mapped item) proceed by wall-clock decomposition only.
#
# CAVEAT for future rounds: kernel.perf_event_paranoid=4 blocks perf
# entirely this session; earlier G9/G10 profiles were taken when it
# was permissive.

# fe decomposition (final, G13 state): build 30 vs 17ms (1.76x) +
# 5x sum 16 vs 6.4ms each (2.5x) — the sum iteration is ~9 ops
# (ForeachNext + bind + LL/LL/Add/SL + Pop) at ~9ns/op vs tclsh's
# ~3.5ns/instruction; build is lappend-dispatch heavy.  Both are the
# per-op weight item — the mapped interpreter-core slimming (direct
# threading / stack-entry slimming) is the remaining lever, currently
# blind without perf.

# ---------------------------------------------------------------------------
# G14 2026-10-07: immediate×immediate fast paths for every arithmetic
# and comparison entry point.  With immediates landed (G12), the
# compare/binop paths still paid nan_pair (six accessor calls), the
# bignum int_rep parsing, and heap-box results for ints.  numeric_binop,
# op_rel, op_eq, op_ne now untag-and-go when both operands are
# immediates: checked arithmetic retags in place (no box, no refcount),
# overflow/out-of-i63-range widens through the same BigInt arms, div-
# by-zero and MIN/-1 hit the identical errors.  op_rel/op_eq/op_ne
# untag-and-compare (immediates are ints — no NaN possible).
#
# BUG the judge corpus caught in the first cut: the immediate tag holds
# i63 but checked_mul is an i64 check — 11^18 (5.6e18) passed checked,
# then wrapped NEGATIVE in the tag shift.  Results now range-check
# against i63 and widen otherwise (gen_expr expr-23.51/52/53).
#
# perf stat (3M while+incr): instructions 2652M -> 2090M (-21%),
# cycles 501M -> 359M (-28%).  Bench: fe 66 -> 61 (1.74x); fib 43 -> 36
# (1.06-1.29x — AT PARITY within noise); build 27 -> 22 (1.57x);
# vars 800; dloop/oo flat.  Micro suite: arith 0.6x, var_incr 0.5x,
# proc_fib 0.7x, dict 0.8x, list/string 1.0x.
# Gates: judge 87/87; sweep 0; tests 1143/0; g11num/g11ext (overflow,
# widening, mixed-rep, i64-extremes) byte-exact vs tclsh.

# ---------------------------------------------------------------------------
# G16 2026-10-07: the executor stack stores plain Values (8-byte words)
# — the Entry enum (Val | Expanded(Vec)) is gone.  ExpandList now
# flattens: the elements join the stack individually, and the
# CallExpand/DynCallExpand ops collect the whole `expand_base..` range
# (the per-expansion grouping was never needed — source order IS stack
# order).  Stack push/pop move 8 bytes instead of 16; pop_val/top_val/
# collect_args_into_scratch simplify.  Gates green; the definitive
# bench needs settled load (external gcc jobs were running — tclsh
# references drifted 1.4x mid-measurement).

# G16 DEFINITIVE BOARD (settled load 2.0, 5-rep references):
# fe 77/34 = 2.26x; fib 49/36 = 1.36x; dloop 281/267 = 1.05x (parity);
# oo 526/402 = 1.31x; vars 775/524 = 1.48x; build 15/12 = 1.25x —
# the G16 stack slimming halved the build loop (30 -> 15ms).  fe's sum
# iteration stands at 2.8x (12.4ms vs 4.4ms per 200k pass) — foreach
# bind + per-op weight; the next levers remain the two documented
# projects (foreach routing parity, pointer-stable storage).

# ---------------------------------------------------------------------------
# G17 state (2026-10-07): the fib profile at G17 is FULLY DIFFUSE — the
# top item is arith at 4.7%, then a long tail of 2-4% components
# (drop_glue, rel, Rc<str> traffic from the per-call source/ns swaps,
# strict_bool, slot ops, region pushes, stack Entry machinery).  No
# lever above 5% remains: call overhead already beats tclsh (0.52x),
# arithmetic/comparison runs on immediates, the frame/const pools are
# identity-cached.  The residual fib gap (1.1-1.3x) is the SUM of the
# per-op dispatch weight — the interpreter-core slimming (direct
# threading) is the only remaining structural item and is
# rewrite-grade.  fe (~2x) retains one mapped lever (foreach routing
# parity, G9f); vars (~1.5-1.8x) retains pointer-stable storage.

# ---------------------------------------------------------------------------
# G18 2026-10-07: the arrays_env_only flag.  array_globals permanently
# holds the startup arrays (env, tcl_platform), so every per-op guard
# (`!is_empty() && contains(name)`) ran a real probe.  A flag — true
# while the set is exactly the startup arrays, maintained at every
# mutation site (4 insert/remove sites + init) — lets the fast-path
# guards skip the probe when the answer is statically false.  Error-
# path membership checks (loc_is_array) keep the probe.
# Bench within today's noise band; the guard probes were ~10% of
# var-heavy profiles.  Gates: judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G19 2026-10-07: the OO memos hash with Fx, not SipHash.  The G8f
# chain_memo/key_memo used plain std HashMap (RandomState ~20ns/probe)
# — the oo profile showed make_hash<RandomState> + hash_one at ~16%
# combined.  All three memos (chain_memo, chain_memo_priv, key_memo,
# including their nested inner maps) now hash with the interpreter's
# unkeyed Fx (~2ns).  Gates: judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G20 2026-10-07: four more hot tables hash with Fx, not SipHash —
# expr_check_cache (consulted on EVERY expr eval), namespaces (the
# parent-exists probe in every variable-link install), code_cache and
# bytecode_cache (probed on every eval and body eval).  A generic
# FxHashedMap<K, V> alias joined varmap.rs.  Bench deferred — an
# external node job spiked the load mid-measurement (tclsh references
# doubled).
# Gates: judge 87/87; sweep 0; tests 1143/0.

# G20 DEFINITIVE BOARD (settled load 2.0): fe 69/32 = 2.16x; vars
# 770/445 = 1.73x; dloop 281/256 = 1.10x; fib 42/36 = 1.17x; oo
# 518/405 = 1.28x; build 22/16 = 1.38x.  Micro suite (settled): arith
# 0.6x, var_incr 0.6x, string_build 0.8x, dict_ops 0.9x, list_ops
# 1.0x, proc_fib 1.0x — the micro suite is at parity or faster
# throughout.

# ---------------------------------------------------------------------------
# G21 2026-10-07: slot superinstructions — the loop-unrolling pass.  The
# peephole now fuses the accumulator/condition triples EVERYWHERE
# (including inside [expr ...] brackets — the earlier attempt required a
# StoreLocal suffix and never fired; the window is the contiguous
# [LoadLocal, LoadLocal/PushInt, ARITH-or-CMP] triple):
#   [LL, LL, Add]        -> AddSlotSlot(a, b)
#   [LL, PushInt, Add]   -> AddSlotImm(a, k)
#   [LL, LL, CMP]        -> CmpSlotSlot(a, b, cc)   (cc: 4 orders + eq/ne)
#   [LL/PushInt, CMP]    -> CmpSlotImm (mirrored cc for swapped operands)
# The fused ops replicate LoadLocal's exact fallback (eval_var_ref on
# the table name — globals fallback, linked-slot resolution, canonical
# errors) inside themselves; comparisons run through op_rel/op_eq so
# string operands are identical.  perf stat (3M while+incr):
# instructions 2652M -> 2351M (-11%; the while body fuses only its
# condition — the sum loop fuses 3 ops -> 1).  Gates: judge 87/87;
# sweep 0; tests 1143/0; numeric probes byte-exact.  Definitive bench
# deferred (external node load oscillating).

# ---------------------------------------------------------------------------
# G22 2026-10-07: slot memory halved — `slots: Vec<Value>` with an
# in-band unset sentinel.  `Cell<u64>` has no niche, so Option<Value>
# was 16 bytes; bits==0 is unreachable for real values (immediates
# have bit0 set, heap pointers are non-null), making it the natural
# unset state: slots are now 8-byte words, halving slot memory traffic
# for every slot op.  Value::UNSET / Value::is_unset are the API.
#
# CRASH GATE lesson (again, sharper): the first cut segfaulted —
# Drop/Clone dispatched on `bits & 1` (immediate vs heap) and the
# sentinel's bits==0 fell into the heap branch, dereferencing null.
# The judge caught it as a 35-file wipe.  Clone/Drop now early-return
# on bits==0, and every slot access is guarded by is_unset() before
# touching the payload.  (The workspace test-compile also briefly
# reported "tests: 0" — a stale-binary race, not a compile break.)
# Gates: judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G23 2026-10-07: the OO tables and memo keys go Fx/Rc.  OoState's
# objects/classes/my_commands hash with the interpreter's unkeyed Fx
# (they were std RandomState — resolve_object_key's two candidate
# probes paid SipHash per call).  The chain/key memos key on Rc<str>
# (probes still by &str), and the memo hit returns an Rc clone — the
# dispatcher's canonical key is now zero-alloc end-to-end (the
# define-time callers materialize Strings; they are cold).
# Gates: judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G23b (NEGATIVE RESULT, reverted): Rc-keyed chain memos + Rc ActiveMethod.
# Converting the chain/key memo keys (and ActiveMethod's obj/method) from
# String to Rc<str> — to hand the dispatcher zero-alloc canonical keys —
# measured 498 -> 617ms on oo_bench2 (+23%): the hit path needed the
# stored key Rcs, and recovering them (memo.keys().find over the memo's
# entries) cost more than the two String allocations it saved.  Reverted;
# the Fx-hashed String-keyed memos (G19/G23) stand.  (Shell-state resets
# between bench invocations also produced phantom 1ms readings — measure
# with freshly-defined helpers in the same command.)

# ---------------------------------------------------------------------------
# G24 2026-10-07: framed foreach — the global inline now reproduces the
# dispatched tclsh shape.  ForeachInfo gains `framed` (true when the
# foreach compiles OUTSIDE a proc body — tclsh's compiledLocals gating),
# ForeachStart pushes Region::BodyFramed for those, and the error
# protocol gains the dispatched shape: body errors crossing the framed
# region append the innermost frame, the `("foreach"/"lmap" body line N)`
# exit frame, AND the foreach command's own `invoked from within`
# harness (all immediate — the pending-top deferral loses the frame to
# inner catch); loop-var write failures gain the
# `(setting foreach loop variable "x")` decoration before the harness.
# All five foreach-shape pins (loops.rs + vm_exec foreach_inline_tests)
# pass; gen_foreach foreach-1.14 byte-exact.
#
# This completes the foreach global inline (the gate opened in G21's
# experiment): the compiled engine now matches the tree at every scope.
# Gates: judge 87/87; sweep 0; tests 1143/0.

# ---------------------------------------------------------------------------
# G25 2026-10-07 (FINAL optimization round): pooled args + method-head
# cache.  The OO dispatch arm assembles [typed, method, call_args...] per
# call — three allocations (the Vec + two from_str Values).  The Vec now
# comes from OoState.args_pool (take/put, cap 8), and the head Values are
# cached on MethodDef.head_cache keyed by the typed name (recycled on
# call-site match).  oo_bench2 558 -> 508ms.  Gates: judge 87/87; sweep
# 0; tests 1143/0; wasm32 build + node suite 18/18.
#
# == OPTIMIZATION PHASE CLOSED (production marking) — the definitive
# settled-load board: oo 1.19x, vars 1.52x, fe 2.2x, fib 1.21x, dloop
# 1.15x, build 1.43x; micro suite 0.6-1.0x (arith/var_incr/string_build
# FASTER than tclsh).  wasm32: build green, node suite 18/18, 1.6MB.
# The remaining levers are the two documented projects (foreach routing
# parity, pointer-stable storage) plus direct threading — all deferred
# with full playbooks above.
# rtcl-data micro baseline — f809dbe 2026-10-07
# Linux 7.0.0-34-generic x86_64, release, best-of-3
# harness: crates/rtcl-data/src/bin/data_bench.rs (lib-level, no interp), fixture bench/fixture.yaml

bench            per-op
yaml_decode_4kb  74.2 us
url_normalize    629 ns
sha256_64b       203 ns


## perf/interp-arch-opt — 解释器热径优化（2026-10-18）

分支 `perf/interp-arch-opt`（不合并 master，JIT M2 期间独立验证）。画像工具：
`setarch -R perf record -F 4000 -g` + 专用构建（frame pointers + debug info），
`perf script` 文本自聚合（本机 `perf report` 超时不可用）；开发反馈以
`perf stat -e instructions:u`（±0.01% 稳定）为准，wall 用交错 best-of-N。

| 轮次 | 改动 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars |
|---|---|---|---|---|---|---|
| 起点（master f25d234） | — | 1.32x | 1.82x | 1.01x | 1.07x | 1.59x |
| A（6892c48+da44359） | exec_op 热径内联 hot lane（共享 #[inline] 语义体） | 0.97x | 1.71x | 0.98x | 1.07x | 1.52x |
| B1（c7e8f57） | `as_int/as_float` 补 Bool rep 快路径（循环条件不再字符串解析 "1"/"0"） | — | 1.47x | — | — | 1.49x |
| B2（fb931c3） | Frame.has_aliased 帧级短路（slot 别名探测） | — | — | — | — | — |
| B3（本提交） | Value::drop 内联两层（tag/refcount）+ as_int/as_float 拆分快慢 + strict_bool 内联 + op 边界检查单次化 | 1.03x | 1.23x | 1.01x | 0.97x | 1.47x |

指令数比（rtcl/tclsh，越低越好）：fib 1.04x、fe 1.51x、dloop 0.86x、oo2 0.73x、
oovars 1.05x、build-only 1.29x。fe 的余量集中在 `build`（lappend 循环，1.22x）；
oovars 是内存停顿主导（指令 1.05x 但 wall 1.47x：cache-refs 346M vs tclsh 74M）。

## Round D/E — 分配与扫描去除（2026-10-18，继续）

| 轮次 | 改动 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars |
|---|---|---|---|---|---|---|
| D1+D2（bd71f9e） | OO `objects/classes/my_commands`、namespace `variables` 换 Fx 哈希（去 SipHash）；VarHasher 尾块免 memcpy | — | — | — | 0.67x | 0.96x |
| D3（2cd2669） | 派发链名字 `Rc<str>` 共享（ActiveMethod/chain memo 键/对象 ns）、prelink 记忆化（ProcMemo 携带链接表 Rc）、upvar 链接 `Rc<str>` + upvars 键 `Rc<str>` | — | — | — | 0.60x | 0.81x |
| E1（本提交） | 短名 `(` 扫描内联字节循环（util::find_paren/strip_paren）、`dead_link_error`/`check_parent_ns` 空表/无 `::` 快速短路 | 1.02x | 1.51x | 0.70x | 0.54x | 0.75x |

以上为指令数比（rtcl/tclsh）。wall 对应：oo2 0.63x、oovars 1.04x、dloop 0.77x、fib 0.87x、fe 1.33x。

**新目标（wasm 视角）**：指令数比 0.5~0.6x 为目标，1.2x 为下限。当前最差点：
- fe_bench 1.51x 指令 / 1.33x wall —— `build`（lappend 循环，build-only 1.29x 指令）是唯一还大幅落后的热点
- fib25 1.02x 指令（proc 调用路径）
- oovars 0.75x 指令但 wall 1.04x（内存停顿：IPC 比 ≈0.72）

## Round F — nanopass 融合：比较/分支与槽对窗口（2026-10-18，继续）

背景：JIT 侧（master `24ed853`）的发射器已是 nanopass 管线（L0→L4 +
层间 `check.rs` 校验）。本轮把解释器的 opcode 构造侧同样推进为「独立纯
函数 pass、单向重写」：新增 `ByteCode::fuse_slot_pairs`（由 `peephole()`
驱动，与既有折叠 pass 同轮迭代；`locals_mode` 门控），按跳转靶点位图
逐窗口重写——窗口内部（head 之外）是靶点则拒绝融合（融合 op 整体替换
窗口，源自 consumer 的入口会丢掉 producer）；`strip_nops` 同步重映射
融合 op 自带的 `t`。

| 窗口 | 融合 op |
|---|---|
| `[CmpSlotSlot, JumpTrue/JumpFalse]` | `CmpSlotSlotJumpTrue/False(a,b,cc,t)` |
| `[CmpSlotImm, JumpTrue/JumpFalse]` | `CmpSlotImmJumpTrue/False(a,k,cc,t)` |
| `[LoadLocal, LappendLocal, Pop]` | `LappendSlotSlot(src,dst)` |
| `[IncrLocal, Pop]` | `IncrLocalPop(s,k)` |

语义与兼容保障：
- 融合 op 只出现在 locals-mode 单元（JIT 只编译 name-keyed 单元，接受集
  不受扰）；`rtcl-vm` 并行 VM 显式拒绝，`ByteCode.fallback` 白名单同步
  补入（语义标志不变）。
- `OpCode` 尺寸簿记测试（≤16B，dispatch 每 op 拷贝一次枚举）——新
  payload 不得顶出信封。
- 分支融合的 backward-charge 以「原 jump op 索引 +1」为界（`t == pc+1`
  与派发版 JumpFalse 同判）；fall-through 即 `st.pc` 本身（融合后布局）。
- 拾获两个真 bug：① `peephole()` 早于 `locals_mode` 赋值（pass 空跑）；
  ② 融合臂 fall-through 多加 1，吞掉紧随的 `BEGIN_CMD`——judge
  `gen_proc-old` 抓到（body 错误帧与 proc 帧从 errorInfo 消失），已修 +
  回归测试 `fused_branch_fallthrough_keeps_site`。
- 同批：`LappendLocal` fallback 名的 `.to_string()` 惰性化（此前每次迭代
  一次 malloc+memcpy ≈13% build 时间）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| 起点（本轮前） | 1.02x | 1.51x | 0.70x | 0.54x | 0.75x | 1.29x |
| F（本提交） | 1.00x | 1.40x | 0.70x | 0.54x | 0.75x | 0.86x |

指令数比（rtcl/tclsh）。wall（3 次交错 best-of-N）：fe 1.18x、dloop 0.78x、
oo2 0.56x、oovars 0.99x、fib 1.00x。build 循环每迭代 op 数 12→8。

## Round G — name-keyed 超指令 + de-fusion 桥（2026-10-18，JIT 约束解除后）

方向更新（用户裁决）：wasm 性能目标优先，**允许 break JIT**——JIT 侧靠
文档 + 一个展开钩子跟进，不再以「JIT 接受集」反向限制解释器的 op 集合。

- `fuse_pairs` 新增 name-keyed 窗口（eval 级单元 = 全局/namespace 变量
  代码，也是 wasm 嵌入场景的主力形态）：
  `[LoadVar,LoadVar,ADD/CMP] → AddVarVar/CmpVarVar`、
  `[LoadVar,PushInt k,ADD/CMP] → AddVarImm/CmpVarImm`（含镜像序
  `[PushInt,LoadVar,CMP]`）、`[LoadVar,LappendVar,Pop] → LappendVarVar`、
  `[IncrVar,Pop] → IncrVarPop`、`[CmpVar*,JumpT/F] → CmpVar*JumpT/F`
  （4 个融合分支）。
- `StoreLocalPop`（[StoreLocal,Pop]）：值 move 进 slot，去 clone+drop；
  `frame_slot_write_move` 与其克隆版同款守卫。
- `ByteCode::unfused()`：全部融合 op 展开回**融合前**规范窗口（含靶点
  重映射、全族覆盖测试 `unfused_covers_every_family`）。JIT 编译边界调用
  之（`exec.rs` + golden/roundtrip/microbench 测试助手），**JIT 接受集与
  golden 全绿、零回退损失**；原生 lower 列为可选优化。
- `docs/JIT-OPCODE-DELTA.md`：逐 op 语义、窗口表、建议 lower 路径、降级
  保真三件事（回边计步/站点 region/值语义）与「已完成/可选」接线清单。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| F 后 | 1.00x | 1.40x | 0.70x | 0.54x | 0.75x | 0.86x |
| G（本提交） | 1.00x | 1.29x | 0.69x | 0.53x | 0.73x | 0.85x |

补充（指令数比）：sum_only 0.78x→0.72x（全局 while + 5×foreach 求和）；
全局 while 循环 12→8 op/迭代；oo2 4863M→4737M、oovars 7.09G→6.90G。
wall（3 次交错 best-of-N）：fe 1.13x、dloop 0.73–0.81x、oo2 0.57x、
oovars 0.96x、sum_only 0.68x、fib 1.03x（噪声带内）。

## Round H — meta op 与循环控制融合（2026-10-18，继续）

数据驱动：per-op 语义体 ≈ tclsh 的 1.1x，真正差距在**每迭代 op 数**（错误
簿记的 meta op 税）。逐项增量测量（空 for 循环基线）显示 rtcl 534 instr/迭代
vs tclsh 337。本轮把编译器的高频簿记相邻对交给 `fuse_pairs`：

- meta 对：`[BeginCmd,SubMark]→BeginCmdSubMark`、
  `[ExprMark,BeginCmd]→ExprMarkBeginCmd`、`[BeginCmd,PushConst]→
  BeginCmdPushConst`、`[SubEnd,StoreLocalPop/StoreVarPop/Return]→
  SubEndStoreLocalPop/…VarPop/SubEndReturn`。
- 循环控制：`[Jump(t=下一op),NextMark]→SwapToNextMark`、
  `[Jump(t=下一op),ForeachNext]→ForeachNextClose`、
  `[NextMark,BeginCmd]→NextMarkBeginCmd`、
  `[BeginCmd,IncrLocal/VarPop]→BeginCmdIncrLocal/VarPop`。
- **class-A 靶点重定向**：被消费的 `NextMark`/`ForeachNext` 常是 `continue`
  的落点——融合时把指向它的靶点重定向到融合 op 头（展开为 `[Nop,原op]`，
  trampoline 语义）；执行器按 `LoopFrame` 归一化 region 栈（与 `op_continue`
  同款），双入口等价、计步平价。
- 两个自查 bug：`ForeachNextClose` 的 `body` 靶点漏进 `strip_nops` 重映射
  （judge 8 例 + 1 panic 抓到）；class-A 融合需重定向 else 跳过归一化
  （judge control_flow / gen_lmap 抓到）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| G 后 | 1.00x | 1.29x | 0.69x | 0.53x | 0.73x | 0.85x |
| H（本提交） | 0.99x | 1.23x | 0.69x | 0.52x | 0.72x | 0.84x |

dloop 的 0.45x 假象说明：坏 remap 下循环少干活（judge 未绿时的数字不可用，
已剔除）。空循环基线 534→517 instr/迭代。

## Round I — 语句级融合（2026-10-18，继续）

`set <slot> [expr {$a + $b>}]` / `set <var> [expr {$a + $b>}]` 的整段窗口
（`[BeginCmdSubMark, BeginCmd, AddSlotSlot/AddVarVar, SubEndStoreLocal/VarPop]`）
折叠为单 op `SlotSetAddSS` / `VarSetAddVV`（payload 14B，保持 ≤16 信封）；
逐字复刻各步（站点、Sub region、读、加、关 region、move 写入），错误/region
状态在任一步失败时与原窗口一致；`unfused()` 直落 7/8 op 规范形。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| H 后 | 0.99x | 1.23x | 0.69x | 0.52x | 0.72x | 0.84x |
| I（本提交） | 0.99x | 1.10x | 0.69x | 0.52x | 0.72x | 0.84x |

fe 的 `p` 循环每迭代 6 op → 3 op（BODY_MARK / SET_SS_ADD /
FOREACH_NEXT_CLOSE）；wall（3 次交错）：fe 1.10x、dloop 0.81x、oo2 0.65x、
oovars 1.01x、fib 0.97x。

## Round J — foreach 绑定快路径 + body-mark 折叠（2026-10-18，继续）

- `foreach_bind` 常见形（单 group、单变量、slot 目标）直绑：跳过双层迭代器
  与 `Value::empty()` 的 TLS 查找（只在越界分支取用）——p_only −8%。
- `[Jump(t=下一op), ForeachNext]` 融合把 **body region 的建设收进
  FOREACH_NEXT_CLOSE**：body 入口指向 `BODY_MARK` 之后（首轮仍经
  `ForeachStart` 落穿执行标记，后续迭代由融合 op 按 `framed/lmap` 开
  region）——p 循环 3 op → 2 op（SET_SS_ADD / FOREACH_NEXT_CLOSE）。
  非内联形状（body 非 BODY_MARK）保持原 op 对。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| I 后 | 0.99x | 1.10x | 0.69x | 0.52x | 0.72x | 0.84x |
| J（本提交） | 0.99x | **0.99x** | 0.69x | 0.52x | 0.72x | 0.84x |

**六个基准全部 ≤1.0x**（起点：fib 1.32 / fe 1.82 / dloop 1.01 / oo2 1.07 /
oovars 1.59 / build 1.29）。

## Round AB — my 派发快径实验（2026-10-18，结论：回退）

对 `cmd_oo_my` 复用派发缓存（第 2 槽）+ `frame_in_object_ns` 免 `format!`：
- `my` 快径（单槽）：交替递归微基准（a↔b 互调，2.1M 次 my 调用）
  **−2.89%**（17.55G→18.06G）。根因：单槽缓存被交替方法名冲刷，
  每次调用 = 验证失败 + 慢径 + 填充（object_ns_of/Rc clones），
  而慢径时省下的（qualify/Rc::from/两哈希）已被摊销得比预期小。
- 仅免 `format!` 门禁：**−0.01%**（噪声），保留价值不足。
结论：两者均回退。`my` 派发的进一步优化需要**按方法词身份索引的多槽缓存**
（如 4 槽，idx = 词指针低位），且板面不覆盖 `my`——记录为低优先级。
（对照：对象派发单槽即可，因热点循环形状是同一方法重复调用——Round AA。）

## Round AA — OO 派发站点级 memo（2026-10-18）

`OoCallCache`（OoState 单槽）：缓存 `(ns, stamp, typed, method, key,
method_rc, chain)`；命中判定 = 变异戳相等 + 当前命名空间 `Rc` 指针相等 +
typed/method 两词 `same_allocation`（常量池字面量/槽值 clone 的分配稳定）
——四项合起来蕴含 `resolve_object_key` 必得同 key、`chain_for` 必 memo 命中，
于是两次哈希探测全免，缓存链直进 `exec_chain_entry`。`my`
（include_private）路径既不查也不填（帧上下文门禁语义另议）。

实测：oo2 4284M→**3834M（−10.5%）0.48x→0.43x**；oovars 5389M→**4939M
（−8.3%）0.57x→0.52x**；其余板持平。两板均进入 0.5-0.6 目标带（oo2 更低）。
门禁：tests 1256/0、judge 88/88、sweep 0。

## Round Z — while 体尾融合实验（2026-10-18，结论：回退）

新 op `IncrPopJumpLocal/Var`：`[BeginCmdIncr*Pop(site,x,k), Jump(t)]` 单 op
（`while {..} {incr i}` 的体尾形状；`for`/最小值脚本已在 IR 层验证融合触发）。
实测（`/tmp/while_incr.tcl`，3M 次 while+incr）：融合后**净增** 36.0M 指令
（2.336G→2.372G，**+1.54%**，≈+12 instr/迭代）；六板与 micro 板持平（该形状
不在其中）。根因判定：热 op 派发面再加两个臂后，小循环（2 op/迭代）的派发/
布局成本上升，吃掉并超过省下的一次派发——与「热 op 加分支反复净负」的先例
一致。另外 micro_incr 的 `for` 体尾是 **fall-through**（体尾无 Jump，直接落入
advance 块），本窗口对其根本无效。
决策：回退（无板面收益 + 目标形状变慢 + 增加 JIT 增量面），实验记录留档。

## Round Y — OO 派发快径两连（2026-10-18）

1. **head_cache 身份比较**：`MethodDef.head_cache` 键从 `(String, Value, Value)`
   改 `(Value, Value, Value)`，命中判定 `Value::same_allocation`（ptr 比较，
   免 memcmp、免 miss 路径的 `to_string` 分配）。`invoke_method`/
   `exec_chain_entry` 增 `typed_val: &Value` 参数（`next`/`<cloned>` 冷路径
   传临时 Value）。oovars 5499M→5482M，oo2 4394M→4376M。
2. **ChainEntry 携带 owner 变量表**：链构建点（构建时表项在手，clone 即得）
   预取 owner 的 `variables`/`obj_variables` Rc；`memoised_proc_def` 的 memo
   键检查不再每调用做 `objects.get`/`classes.get` 哈希探测。
   健全性：该字段唯一写点（`oo::define variable`）换 Rc 且必 bump
   `mutation_ctr`，而链 memo 由 ctr 盖章——盖章未变则 hint == live。
   `next`/`<cloned>` 路径经 `fresh_chain=false` 走原 live 探测（方法体内
   声明 `variable` 后 `next` 的观测语义逐字保持）。
   oovars 5482M→5389M（**0.58x→0.57x**），oo2 4376M→4284M（**0.49x→0.48x**）。

门禁：tests 1256/0、judge 88/88、sweep 0。

## 快照 — perf/interp-arch-opt-2 收盘（2026-10-18）

指令数比（rtcl/tclsh）/ wall（3 次交错 best-of）：

| bench | 指令比 | wall |
|---|---|---|
| fib25 | 0.85x | 0.88x |
| fe_bench | 0.92x | 0.79x |
| dloop | 0.68x | 0.72x |
| oo_bench2 | 0.49x | 0.56x |
| oovars | 0.58x | 0.66x |
| build-only | 0.72x | — |
| micro_base | 1.08x | — |
| micro_incr | 1.33x | — |

本轮（评审合并后新分支）五个轮次：U 骨架融合修复（靶点 remap）、V 死推入
消除、W 站点惰性审计（关闭）、X prelink 槽位预计算；S/T 为回退实验记录。
下一批候选（定界已入 TODO）：head_cache 身份比较、ChainEntry 携带 owner
变量表、OO 派发站点级 memo、call_proc 帧准备、`Value` 字符串句柄共享。

## Round X — OO prelink 槽位预计算（2026-10-18）

`PrelinkEntry.slot`：链 memo 构建时（拿到编译体之后）预计算每个链接名在
`locals` 表中的槽位；安装路径改走 `degrade_frame_link_at(frame_idx, name,
slot, link)`——命中预计算槽位时跳过 `slot_index_of` 的线性名字扫描
（debug_assert 校验槽位与名字一致）。实测 oovars 5536M→**5499M**；
oo2/fe/fib/dloop/build 持平。门禁：tests 1256/0、judge 88/88、sweep 0。

## 审计记录 — 站点惰性化（Round W，结论：不做）

想要「`BEGIN_CMD` 只写 `cur_site`，文本/行/word_srcs 延迟到错误路径解析」。
先做读取点审计（`cur_cmd_text/cur_cmd_line/cur_cmd_word_srcs/line_offset`）：

| 读者 | 何时运行 | 惰性化影响 |
|---|---|---|
| vm_exec 错误处理 + unwind_subs + 各 Region 臂 | 编译单元出错 | 可从 `code.sites[st.cur_site]` 直接解析 ✓（本可行） |
| `loops.rs:295` foreach/lmap 内联形状门 | 编译单元**派发** foreach/lmap 时 | ✗ 需要 BEGIN_CMD 写的 word_srcs（派发者无 code） |
| `control.rs:32`/`loops.rs:27,358,387`（`line_offset + cur_cmd_line`） | 派发 while/for/if 的 tree-walk 体求值 | ✗ 需要 BEGIN_CMD 的字段 |
| `namespace.rs:336`（`cur_cmd_text.slice`） | 派发辅助命令 | ✗ 同上 |
| `eval.rs:308/333/356`（word 读取） | tree-walk 词求值（自带 save/restore 写字段） | ○ 非 BEGIN_CMD 来源 |

结论：**收益 ~15-20 instr/命令，代价是给 8 个外部读点 + 错误路径引入 (unit, site)
惰性解析**——且 Round L 之后 `op_begin_cmd` 本就只剩「守卫 + site 查表 +
4-5 个廉价存储」≈25 instr（word_srcs 已是 ptr+len 切片，无 Rc）。风险/收益
不成立，**关闭该项**。

## Round V — 死推入消除 + 骨架成本归因（2026-10-18）

`[PushEmpty/PushTrue/PushFalse/PushInt/PushConst(Wide), Pop]` → `Nop, Nop`
（语义空操作：被丢弃的字面量不可观测；空体/常量体构造的常见形）。

- `micro_base`（空 for 体）430.9M→**365.9M（1.28x→1.08x）**——空体的
  `PUSH_EMPTY+POP` 每迭代 ~28 instr 归零。
- 全板：fe 0.92x、build 0.72x、其余持平（该形只在空体/常量体构造出现）。

**micro 成本归因**（perf 叶表，回答「为何 1.3x」）：

| micro | 每迭代 | 构成（叶%） |
|---|---|---|
| micro_base | 366 instr / 2 op | exec_inner 19.9（派发 ~72/迭代）、条件 cmp_bool 13.3+op_cmp_ss_jump 10.2+slot_read 6.0（≈108）、尾 op 9.6+incr 路径 ~11（≈74）、region drop/clone ~7 |
| micro_incr | 519 instr / 3 op | exec_inner 16.0（~83）、条件 ~18（~93）、体 `incr x`（op_begin_cmd 6.5 + incr_local_value 6.5 + set_int_rep 5.2 + frame_slot_incr 4.3 + get<u64> 5.6 ≈ 150）、尾 ~120 |

结论：**单 op ≈170-180 instr**（tclsh ≈48/指令），主因 = 每命令 `BEGIN_CMD`
站点存储（~34）、通用槽访问（frames.last+检查+clone，每次 ~20×5）、
region 簿记（~40/迭代）、派发（~36/op）与 Result/helper 管道——tclsh 的
等价循环没有运行时站点/region 簿记（errorInfo 走静态行表）且指令更扁平。
这是架构地板；后续只剩「体+尾」合并省一次派发（~36/迭代，payload 受限）
等小项。

## Round U — 循环骨架融合（修复版，2026-10-18）

Round T 失败**根因已定位并修复**：`ForNextIncrLocal/Var` 携带的跳转靶点
漏进 `strip_nops` 重映射与 `collect_jump_targets`——复杂脚本里前段折叠
产生的 nop 一经 strip 就把条件靶点移错（循环不退出）；简单脚本无 nop
故自测通过。补上两处后：

- `[SwapToNextMark, BeginCmdIncrLocal/VarPop(site,x,k), Jump(t)]`
  → `ForNextIncrLocal/Var`（k 需 fits-i32；`jump_pc = st.pc + 1` 计步平价）；
  entries 在头部（continue 落点）执行同一序列。
- 实测：`micro_base` 449.9M→**430.9M（1.33x→1.28x）**、`micro_incr`
  533.9M→518.9M（1.36x→1.33x）、`build-only` 3485M→**3371M（0.75x→0.72x）**、
  `fe_bench` 0.93x→**0.92x**；fib/dloop/oo2/oovars 持平。
- 门禁：tests **1256/0**、judge **88/88**（含 control_flow/gen_expr）、sweep 0。

## 实验记录 — 循环骨架融合（Round T，已回退）

消融定位（新增 micro）：**`while` 形态 1.09x vs `for` 形态 1.33x**
（`micro_base` tclsh 337M vs rtcl 450M，1M 迭代）——差距集中在 `for` 专属的
next-script 转换（`SWAP_TO_NEXT_MARK` + Next region + 一跳派发 ≈107 instr/迭代），
而非通用循环机制。

实现（两窗口，均逐字复刻）：
- 3-op `[SwapToNextMark, BeginCmdIncrLocal/VarPop(site,x,k), Jump(t)]`
  → `ForNextIncrLocal/Var`（k 需 fits-i32；entries 在头部=continue 落点等价）；
- 2-op `[BeginCmdIncrLocal/VarPop, Jump]` → `Incr*PopJump`。

实测：`micro_base` 449.9M→**430.9M**（1.33x→1.28x）、`micro_incr` 533.9M→
518.9M、`m_while` 342.9M→337.9M；但 **judge 破防**：2-op 窗口令
`control_flow` 的 `for` 不终止；移除 2-op 后 3-op 仍令 `gen_expr`（`expr-23.49`
的 `for {set tval 0} {$tval <= 66} {incr tval}`）不终止——两窗口均曾复现
「循环不退出」，根因未在预算内收敛（自测小脚本 f1/f2/f3、cf_min 均通过，
长脚本多构造交互处失败 → 疑似与 `strip/remap` 的 `cont/brk` 靶点交互）。

**整批回退**（分支保留 master 基线）。结论：骨架融合必须配「循环终止性」
专项矩阵（break/continue/next-script 逃逸 × 嵌套 × strip 交互）才能安全落地，
列入下一批（完整工作段 + 专项探针）。

## 实验记录 — VarMap RC 键表示层（Round S，已回退）

按计划试做 `VarMap/VarSet/UpvarMap` 键 → `Name(Rc<str>)`（ptr-first `Eq` +
`Borrow<str>`，178 处机械修复全部由编译器引导落地；并把
`UpvarLink::Global` 的**句柄身份**贯通到 `VarLoc::Global(Name)`：
link 首次写入用同一 `Rc` 入表，槽-link 探针（`frame_slot_value/write/incr`
的 `get_mut`）改为 `Name` 句柄查询 → 命中时 ptr 比较、跳过 memcmp）。

实测（与基线逐项对比）：

| bench | 基线 | 本次 | 结论 |
|---|---|---|---|
| fib25 | 508.1M | 507.0M | 持平 |
| fe_bench | 577.5M | 577.8M | 持平 |
| dloop | 2977M | 3030M | **+1.8% 回退** |
| oo_bench2 | 4358M | 4355M | 持平 |
| oovars | 5508M | 5517-5536M | 略负 |
| build-only | 3467M | 3473M | 略负 |

结论：**中性偏负，回退**。原因：省下的 memcmp（短名 ~10-20 instr）抵不过
探针仍需的字符串 hash（未keyed Fx 也要 ~20-30 instr）与句柄克隆/释放
（4-8 instr），且 `Name` 键让 hot 表的行宽/代码布局变动带来 dloop 的净负。
**此实验关闭该项**：变量表键表示层在此架构下不成立；若未来做
`Value` 字符串句柄共享（派发名/常量池），可重估「散列按身份缓存」的第三条路
（哈希值随句柄记忆化，需不动用不安全代码的 memo 结构）。

## Round R — OO 变量语义修正 + 大赢（2026-10-18，继续）

两个语义发现（探针 + judge 语料联合作证）：

1. **prelink 的「seed 局部副本」移除**：安装时把 namespace 变量的现值拷进
   `frame.locals`（每次调用一次全局探测 + String 分配）。Tcl 的 `variable`
   链接语义是活链接（读写直达 ns 变量，无副本）——upvar 链接已提供该路径，
   seed 纯属冗余。移除后 oovars 6051M→**5515M**（−8.9%）。
2. **方法 memo 身份补 ns**（我预计算引入的跨实例串扰，探针抓到）：一个
   `MethodDef` 服务类的所有实例，而各实例的 `variable` 链接要在**自己的
   ns**（`::oo::ObjN`）解析——memo 元组加入 def ns 的 `Rc` 身份比较；
   新增 judge 语料 `oo_instance_vars.tcl` 作回归（tclsh 差分通过）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| Q 后 | 0.85x | 0.93x | 0.68x | 0.48x | 0.64x | 0.74x |
| R（本提交） | 0.85x | 0.93x | 0.68x | 0.48x | **0.58x** | 0.74x |

## Round Q — OO prelink 条目预计算（2026-10-18，继续）

OO 方法每次调用的 prelink 安装原本做 4 个哈希/字符串探测（`resolve_var_link`
memo、namespace 变量集注册、slot 扫描、upvars 插入）。两步去化：

- **注册一次性**：`PrelinkEntry.registered: Cell<bool>` —— namespace 变量集
  的注册只跑一次（条目的失效域 = 链 memo = 对象 ns 生命周期）；
- **`(qualified, ns_part)` 预计算**：`resolve_var_link` 是纯字符串函数
  （其 memo 可替换、结果不可变）——在 `memoised_proc_def` 构建时（用对象
  ns，与安装时同值）算好存入条目。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| P 后 | 0.85x | 0.93x | 0.68x | 0.48x | 0.69x | 0.74x |
| Q（本提交） | 0.85x | 0.93x | 0.68x | 0.48x | **0.64x** | 0.74x |

## Round P — rtcl-vm 即 VM（crate 重构，2026-10-18）

- 删除未使用的演示 VM（`execute.rs`）与死 API `Interp::eval_compiled`；
  `VmContext` 保留（JIT/wasm ABI 经它回调 `charge_step`）。
- `interp/types/channel/command/host/parser` + stdlib `.tcl` 数据整体迁入
  **`rtcl-vm`**；`rtcl-core` 变 facade（路径/feature 全透传，下游零改动）。
  指令数比无回退：fib 0.85 / fe 0.93 / dloop 0.68 / oo2 0.48 / oovars 0.69 /
  build 0.74（四道门全绿）。

## Round K — 融合分支直取布尔（2026-10-18，继续）

`expr_ops::cmp_bool(left, right, cc)`：比较的**真值**直返（`op_rel/op_eq/
op_ne` 改为其薄包装，语义单一来源）；融合分支 op（`CmpSlot*/CmpVar*Jump*`）
经新 `jump_if_bool` 直接消费，去掉每条件一次的
`Value::from_bool`（缓存单例句柄）+ `strict_bool`（重强制）往返回。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| J 后 | 0.99x | 0.99x | 0.69x | 0.52x | 0.72x | 0.84x |
| K（本提交） | **0.97x** | **0.96x** | 0.69x | 0.52x | 0.72x | **0.76x** |

（起点 master：fib 1.32 / fe 1.82 / dloop 1.01 / oo2 1.07 / oovars 1.59 /
build 1.29。）

## Round O — OO 变量表 Rc 化（2026-10-18，继续）

`OoObject/OoClass.variables`（+ `obj_variables`）从 `Vec<String>` 改为
`Rc<Vec<String>>`：`variable` 声明**整体替换**向量（指针即版本），
`memoised_proc_def` 的每调用校验从「方法名 + 变量表内容逐项比较」变为
**一次指针比较**（`owner_variables` 返回 `Option<&Rc<..>>`，无所有者用
`None` 身份）。oovars 6614M→6563M（0.69x）。

画像（oovars 3978 样本）：memcmp 9.5%、hashbrown 探测 ~12%、call_proc 6.4%；
per-call 字符串面 = KeyMemo/ChainMemo 两次哈希+memcmp、`owner_variables` 的
object 表查找、`head_cache` 的 typed 比较、prelink 安装（`resolve_var_link`
memo + `slot_index_of` 线性扫描 + `Upvars.insert`）——**下一批候选**（见
TODO）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| N 后 | 0.85x | 0.93x | 0.68x | 0.49x | 0.70x | 0.73x |
| O（本提交） | 0.85x | 0.93x | 0.68x | 0.49x | **0.69x** | 0.73x |

（起点 master：fib 1.32 / fe 1.82 / dloop 1.01 / oo2 1.07 / oovars 1.59 /
build 1.29。）

## Round N — 调用路径去分配（2026-10-18，继续）

- `current_statics` 从 `HashMap` clone/调用 → `Rc<HashMap>`（默认零成本，
  写回 COW）；`slot_aliased`/`link_keys` 侧向量懒化（`degrade_frame_link`
  按需扩容，读者全走 `.get`）——每调用去一次 map clone 与两次 resize。
- 实验记录（已回退，两次）：
  ① int 原位快车道（`slot_int`）全量铺开——fib −7M 但 fe/oovars +20M；
  ② 同一快车道**只加在 `SET_SS_ADD`**（fe 热点 op）——仍 fe +9M（577.5M
  →586.5M，+1.6%），其余不变。
  结论：该架构下热 op 内增加分支/重试路径会牵动 LLVM 寄存器分配与布局，
  净负且与收益面无关；per-op 语义体已到该架构的实用地板（~80-100
  instr/op），进一步需要更激进的结构（寄存器式执行器/专用循环形）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| M 后 | 0.90x | 0.93x | 0.68x | 0.51x | 0.70x | 0.73x |
| N（本提交） | **0.85x** | 0.93x | 0.68x | **0.49x** | 0.70x | 0.73x |

（起点 master：fib 1.32 / fe 1.82 / dloop 1.01 / oo2 1.07 / oovars 1.59 /
build 1.29。）

## Round M — fib 参数括号窗口族（2026-10-18，继续）

fib 每次递归调用 26 op：新增 8 个窗口 op 折叠参数括号族——
`PushConstSubMark`（命令名 push + bracket region）、`SubMarkBeginCmd`、
`ExprMarkBeginPushConst`、`SubSlotImm/SubSlotSlot`（`$n-1` 形）、
`SubEndDynCall/SubEndCall`（bracket 结果的 dispatch）、`SubEndAdd`——每次
调用 26→17 op。dispatcher 的 site 缓存按 `st.pc-1` 键控：融合后自愈
（首次 miss 回填融合 op 的 pc，之后命中）。

热径布局调优（实测）：仅 `PushConstSubMark/SubSlotImm/SubEndDynCall`
（fib ×2/调用）留在热径 match，其余走 `exec_op`——热径 match 保持小，
fe/build 不因新 op 变慢（全放热径：fib 0.90x 但 fe/build +0.9%；全放
exec_op：fe/build 恢复但 fib 0.92x；折中两者兼得）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| L 后 | 0.95x | 0.93x | 0.68x | 0.51x | 0.70x | 0.73x |
| M（本提交） | **0.90x** | 0.93x | 0.68x | 0.51x | 0.70x | 0.73x |

（起点 master：fib 1.32 / fe 1.82 / dloop 1.01 / oo2 1.07 / oovars 1.59 /
build 1.29。）

## Round L — word_srcs 借用切片（2026-10-18，继续）

`Interp::cur_cmd_word_srcs` 从 `Rc<Vec<SrcSpan>>` 改为借用新类型
`util::WordSrcs`（`(*const SrcSpan, len)`，`Deref<[SrcSpan]>`）：`BeginCmd`
每次命令不再 rc++/--（`strip` 后 op_begin_cmd 画像 4-5% + Rc 释放 3.7%）。
安全性契约：所有读者（词跨度读取、`foreach/lmap` 内联形状门）都在
owner（编译单元/AST）被栈借用的活动命令内运行；嵌套 save/restore 只
拷贝指针对。JIT 不受影响（同一 `site.word_srcs` Rc 仍在 ByteCode 里）。

| 轮次 | fib25 | fe_bench | dloop | oo_bench2 | oo_bench2_vars | build-only |
|---|---|---|---|---|---|---|
| K 后 | 0.97x | 0.96x | 0.69x | 0.52x | 0.72x | 0.76x |
| L（本提交） | **0.95x** | **0.93x** | 0.68x | **0.51x** | **0.70x** | **0.73x** |

（起点 master：fib 1.32 / fe 1.82 / dloop 1.01 / oo2 1.07 / oovars 1.59 /
build 1.29。）

