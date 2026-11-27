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

## Round AI — call_proc 批次 3 实验（2026-10-18，结论：回退）

把「位置绑定 + `args` 尾 + `info level 0` 填词」的三次
`self.frames.last_mut()` 借用合并为一次（args 尾先算成 `Option<String>`，
帧借用期间不再重借）。画像里 `last_mut`/`as_mut_slice`/`non_null<Box>`
合计 ≈4.2% of call_proc self，预期 ~0.5%——实测**净负**：fib +0.71%、
oo2 +0.42%、oovars +0.31%（合并块更大、跨 `extend` 的生命周期拉长）。
按纪律回退；item 4 以 AF+AH 收口。

## Round AH — call_proc 批次 1+2（2026-10-18，4 号收口）

批次 1（守卫两个每调用 `take()`）：`frame.tailcall.is_some()` 与
`frame_level0_args.is_some()` 守卫——裸 `take()` 是对 24 字节 Option 的全量
`mem::replace`（画像里合计 4.6% of call_proc self）。fib −0.24%、oo2
−0.13%、oovars −0.10%。

批次 2（ns 身份写 + 池化帧保留 ns/call_ns）：`prev_namespace` 桥接的
`frame.call_ns`、`frame.ns`、`interp.current_namespace` 三处赋值改为
**指针相等即跳过**；`frame.ns = None`（序）/`popped.ns = None`
（收尾）**删除**——池化帧保留至多两个命名空间 `Rc`，复用时由身份检查决定
是否替换。读点核对：`call_ns` 仅 `upvar_target` 的 Bottom 分支读（同值
语义不变）；`frame.ns` 只在进入定义命名空间后读取。fib −1.31%、oo2
−0.60%、oovars −0.43%。

**item 4 合计（AF+批次 1+2）**：fib25 507.46M→**493.75M（−2.70%）**，
比值 0.85x→**0.83x**；oo2 −1.38%（0.42x）；oovars −1.01%（0.52x）。
门禁：tests 1256/0、judge 88/88、sweep 0。

## Round AG — my 双槽派发缓存（2026-10-18，5 号配套实测）

结构：对象派发**单槽**（`obj_cache`）＋ `my` 派发**双槽**（`my_cache[2]` +
交替填充游标）。验证不变（`cache_entry_live`：stamp + ns 指针 + typed/method
分配身份）；`my` 路径仍逐调用跑 `frame_in_object_ns` 门禁（obj_ns 随条目缓存）。

实测（确定性指令数）：
- `my_bench`（a↔b 交替递归，2.1M 次 `my`）17.466G→**11.827G（−32.3%）**
  ——Round AB 单槽冲刷问题的正解。
- `dispatch_dynamic`（`$o $m` 变量名派发）4.448G 持平（对象单槽已覆盖）。
- oo2 3809.1M / oovars 4915.2M 持平（**回退过程留档**：把对象派发也做双槽
  会因第二槽探测 +7 instr/调用使 oo2/oovars 各 +0.18%，故拆成单/双槽；
  展开手写探针救不回这 7 instr，成本在条目本身）。
- 板面六项与 AF 相同（fib 0.84x / fe 0.92x / dloop 0.68x / oo2 0.42x /
  oovars 0.52x / build 0.72x）。门禁：tests 1256/0、judge 88/88、sweep 0。

## Round AF — call_proc 去 Rc 流量（2026-10-18，4 号第一刀）

**落地**（call.rs）：`params`/`body`/`compiled` 的每调用三连 `Rc::clone`
改为「全程经引用读 def；仅尾调用重绑时取 owned def（`tc_def:
Option<Rc<ProcDef>>`，循环头 `let def = tc_def.as_deref().unwrap_or(proc_def)`）」，
`statics` 仍为每调用一次 clone（COW 写回需要 owned）。行为等价：
尾调用分支只是把三个赋值换成一个 `tc_def = Some(new_proc)`。

实测（指令数，确定性）：
- fib25 507.46M→**501.52M（−1.17%）**，比值 0.85x→**0.84x**
- oo2 3834.1M→**3809.1M（−0.65%）**，**0.42x**
- oovars 4939.2M→**4915.2M（−0.49%）**，0.52x
- fe/dloop/build 持平。门禁：tests 1256/0、judge 88/88、sweep 0。

**同批否决**（两组合并实验，均回退留档）：
1. `bytecode_applicable` 每迭代一次化 + 槽向量「resize+重写」原位重置：
   合并净负（fib +0.45%、oo2 +0.75%、oovars +0.8%）——未拆分单因，
   但两者都不留。
2. 全批（上述+去 Rc）曾测得 fib −0.65%/oo2 +0.13%/oovars +0.36%；
   拆分后确认收益全部来自去 Rc，且单留它比全批更好（fib −1.17%）。
   教训：**小改动必须拆分 A/B**，合并测量会把负项混进正项。

## Round AE — call_proc 行级归因（2026-10-18）

工具路径三连挫（已踩，勿重复）：
1) `perf script -F +srcline` 全量：逐帧 addr2line，4.4M 帧 → 超时（900s）。
2) `perf report --symbols=call_proc --sort=srcline`：本机版本该组合无输出。
3) 小样本重录（`-F 4000`×12）+ `-F +srcline`：重录仅得 6 样本（record 未采到
   子进程），且本机 perf 不认 `+srcline` 字段（133 行无行号）。
→ **正确工具是 `perf annotate`**：`perf annotate -i /tmp/fib25.perf --stdio -s call_proc`
（实测输出如下，逐行占比为 call_proc 的 self 内部）：

```
(perf annotate 无输出)
```

（原始建议保留）`perf annotate`：`perf annotate -i /tmp/fib25.perf --stdio
   -s call_proc`（单符号源码/汇编注记，只对一个符号做 addr2line，成本可控）。
（下列为路径 2 的空输出记录）：

```
(perf report --symbols 过滤无输出)
```

## Round AD — fib25 符号化画像（2026-10-18，4 号定位）

方法：`CARGO_TARGET_DIR=/tmp/opencode/rtcl-prof` + `CARGO_PROFILE_RELEASE_DEBUG=1`
+ `-C force-frame-pointers=yes` 构建 → `setarch -R perf record -F 40000 -g`
（30 次 fib25，38,698 样本）→ `perf script` 导出 + python 自聚合。

**self(叶) top**：`exec_inner` 12.91% · **`call_proc` 10.73%** ·
`[unknown]` 7.63% · `numeric_binop` 3.09% · `op_begin_cmd` 2.13% ·
`dispatch_site` 2.13% · `__memcmp_evex_movbe` 1.96% · `get<u64>` 1.95% ·
`drop` 1.81% + `drop<str>` 1.78% · `branch<Value,Error>` 1.51% ·
`exec_bytecode` 1.37% · `resize<Value>` 1.30% · `op_cmp_si_jump` 1.29%。
**inclusive**：`call_proc` 89.37%、`exec_bytecode`/`exec_inner` 91.24%。

结论（4 号可动性）：`call_proc` self = 10.73%（fib 全调用都在其下），按
243K 次调用折算 ≈ **224 instr/调用**的纯帧准备/收尾——与 TODO 的候选清单
（level0/槽 resize/ns/池收发）量级吻合；砍掉其 30-40% ≈ fib 3-4%。
下一步：**行级归因**。注意：`perf script -F +srcline` 会对每帧跑 addr2line，
全量 4.4M 帧会超时（已踩）——改用小样本（`-F 2000`、3-5 次运行）或
`perf report --stdio --sort=srcline` 定向看 `call_proc` 内部行。
另注：`[unknown]` 7.63% 与 `drop`/`resize`/`get<u64>` 的 5-6% 是新线索
（帧准备里的容器 clea/resize 与引用计数释放），画像下一轮一并归因。

## Round AC — 4/5 号研究与设计（2026-10-18，不动物码）

- 4 号（call_proc 帧准备）：查台账无近期 fib 画像可直接引用（F1 为 10-04
  旧案）；审读 call.rs 后判定现候选逐项 <1%，**必须先画像**再动。可执行
  配方（含 debug 构建、perf record、perf script 自聚合）已写入 TODO。
- 5 号（Value 句柄共享）：设计稿 `docs/DESIGN-value-handles.md` 落盘——
  S1 名字构造点显式共享（低风险，先做）/ S2 短串线程本地驻留（需 micro
  证明数据串无回退）/ S3 表示层统一（暂不计划）；红线与 JIT 契约写明。

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



# ---------------------------------------------------------------------------
# bench — perf/wave2 cc188db 2026-10-24, JIT fast-tier milestone
# (tag jit-fast-tier @ 6db3724 + wave2 fixes), Linux 7.0.0-38-generic,
# tclsh 8.6.17.  进程内 `time {}`（消除 ~5.5ms 双方共担的进程开销——
# 墙钟口径在 fib 这种 <100ms 的 case 上会严重失真）+ 钉核 best-of-5 墙钟参考。

进程内 time（µs/次）          tclsh      rtcl(interp)   jitsh(fast-tier)
fib25  (242785 proc calls)  113797        71494           52541
arith  (1M-loop)            701975       394291          337144
forloop (100k, proc)          9413        19979           13301
foreach (20k×10)                1.8          6.0            6.0

钉核墙钟 best-of-5 (ms)      tclsh      rtcl           jitsh
fib25                          119          88             85
arith1M                       817         263            229

要点：
- **proc 密集（fib）**：fast-tier 直呼快过 interp 1.36×（进程内）；
  vs tclsh 2.2×。forloop 同理 1.5×。
- **无 proc 调用的纯循环（arith）**：jitsh 走解释器同速路径+序言
  重置的微增益，1.17× vs interp；vs tclsh 2.1×。
- **foreach**：与 interp 持平（foreach 家族宿主路由 = interp 自身
  分派成本；tclsh 差距是解释器层面的，非 JIT 回归）。fast 门扩展
  （循环/变量写/foreach 内联）是后续收益面。
- 墙钟 fib 表观差距远小于进程内（~5.5ms spawn/init 开销占 fib25
  墙钟的 6-10%）；arith 墙钟与进程内方向一致。

# ---------------------------------------------------------------------------
# perf/wave2 fast-tier v3 系列 @ 6bc32bc 2026-10-25（快照机态：闲时高频，
# 与上表不可比——当日 tclsh fib25 仅 24.4ms vs 台账时 113.8ms，机态差
# ~4.7×；比例口径为准）。进程内 time + 钉核 best-of-5 交错。

形状（µs/次）                 tclsh      rtcl(interp)   jitsh(fast v3.1)
fib25 (242785 proc calls)     24356        14495           15553   （≈持平 1.02-1.08×）
forloop (100k, proc+incr)      4438*         2162            3730   （1.73× 落后）
* fl.tcl 钉核当日值

本系列四提交：8864903（IncrVar 增量 tag 修复——untag 折半族：
inc1/gi3/fl 全系值错单根因）、ebca5ce（v3.1 循环进 fast + cur_site
全局 6→7 撞位修复 + resolve_proc 哨兵传播——uv2 关键）、aad1b63
（lazy-flush 槽脏位掩码：切片末字 SLOT_MASK_OFF=120，gate 16→15 槽；
fl 9145→6979）、6bc32bc（slot_base 缓存 + fast 免 SetSite + 纯读单元
免 flush——fl 6979→3730，fib 回持平）。

要点：
- **fib 持平**：fast 协议开销（wrapper/CallSite 门）与 interp 分派
  相抵；纯读单元免 flush 后无额外税。
- **循环 1.73× 落后 = wasmi 指令解释地板**：fl 稳态 ~100 wasm 指令/
  迭代 × wasmi 分派 vs interp 原生 ~21ns/迭代。Arith/Cmp 已全内联
  （含 tagged 单调比较免 untag），进一步融合空间 <30%。
- **下一档杆 = wasmtime fast 驱动**：exec_wasmtime 现仅经典 run()
  （stub 注释：fast 驱动未实现）；fl@wasmtime-classic 已 5401µs，
  fast+wasmtime（Liftoff 原生编译）预期 <interp。FastHost/linker/
  resync/dirty 机器全 wasmi 特化，移植是独立工作量。
- 正确性：形状 17 项 vs tclsh 全一致（rd/rp/dp 余项为 tclsh 措辞
  差、rtcl==jitsh）；fuzz 三种子 1100 例 0 分歧；rtcl-vm 809 全过。

# ---------------------------------------------------------------------------
# perf/wave2 wasmtime fast 驱动 @ 888ff1e 2026-10-27（闲时任务 #145）。
# 同机态同窗交错钉核 best-of-5（快照口径；机态漂移 4×+，比例为准）。

形状（µs/次）        tclsh    rtcl(interp)  jitsh@wasmi  jitsh@wasmtime
fib25                 24828       14849        15101          3203
forloop                2564        2209         3110          1280

比例（wasmtime fast ÷ interp）：fib **0.22×（4.6× 快于解释器）**、
fl **0.58×（1.7× 快于解释器）**——「不弱于解释器」目标在 proc 形状
达成；vs tclsh：fib 7.8×、fl 2.0×。

本节提交：9fef47c（Phase A 循环臂融合：Cmp→BoolOfBool 直写 + IncrVar
零检查消除 + LoadVar 寻址补漏——wasmi fl -18%）→ 888ff1e（Phase B
wasmtime fast 全量移植 exec_fast_wt.rs + p05 循环控制流修复：fast
CallSite resolve 臂 kind1/2 补内层出口 br，深度=经典 d+1/d+2 再 +1
——resolve 点只多 $done 一层）。

要点：
- **wasmi fast 的循环 1.46× 落后是指令解释地板**，wasmtime native
  编译一票翻盘——两引擎共享发射器与 Host 路由，引擎侧仅 Caller/
  Global/Table 类型差异（exec_fast_wt.rs 与 proc_exec.rs 同构）。
- p05 教训：fast CallSite 臂的 kind 分支深度是发射点块结构的函数，
  动块结构必须 WAT 块栈实测（$done 在 resolve 点仍在、$fallback 已闭）。
- 双引擎门禁全绿：probes 18/18×2、battery 33/33×2、fuzz 两种子
  1600 例 0 分歧×2、rtcl-vm 809。
- 下一档杆：arith（无 proc 纯循环）走顶层 classic——fast 化顶层单元
  或 interpreter 内联是独立课题；foreach 内联依旧 OPEN。

# ---------------------------------------------------------------------------
# 补遗（同日，闲时任务 #145 收尾）：judge conformance 全满 + 两修复
# @ a237487。judge --engine jit：8b63ae3 基线 81/88（7 败）→ 88/88
# 双引擎全满。两修复均为 v2/v3.1 语义缺口（非移植引入）：
# 1. dbd9239：resolve_proc 非 proc 命令分派后无条件 dirty——eval 型
#    命令（catch/apply body）内 lappend/set 改调用方帧变量，仅跨帧
#    哨兵会漏（fast 槽 stale → catch_error）。
# 2. a237487：fast_take_fail 补 kind2→5 esc 转换 + CallSite kind-5
#    出口臂（eval 型 continue 逃逸，gen_for）。
# perf 复测（Speed 引擎 e29fd6a）：fib best 2952µs（4.9× interp）、
# fl ~1.3-1.5ms（1.5-1.7× interp）——与 888ff1e 表一致。

# ---------------------------------------------------------------------------
# 闲时任务 #794（foreach 内联 + esc 臂校验）@ 9d1a830 2026-10-31。
# 机态：当日 tclsh fib25 ~25ms（闲时高频档），比例口径为准。

## Phase 0：esc 臂深度 WAT 实测（gen_for for-8.2 apply 单元）

kind-5 逃逸 Br(esc_depth+3) 目标 = 经典 Br(esc_depth+2) 同语义 if 块
（fast 块栈多 $done 一层），无函数层溢出；judge 88/88×2 背书。
结论：纸面推导 +1 关系成立，p05 教训的 WAT 实测法复核通过。

## foreach/lmap 进 fast：语义全绿、perf 止损回摆

- 语义修复三件（judge 88/88×2 下 foreach/lmap 全转绿）：
  final_depth vm 对齐（通用 stack_effect foreach=(0,0) 与
  alloc_locals info 感知表分叉 ngroups-1 格 → 单元末 SetResult 错位，
  lmap 双组返 iters、foreach-8.1 crash 返 "z"——ae44a98 整单元拒绝
  掩盖的原始 bug 族）；bind 协议（Start/Bind 前 flush、Start 后定向
  resync——Start 宿主 bind(0) 写迭代 0 循环变量）；布局（foreach i64
  对基址 nv+3，fast_foreach_base 是 i32 侧公式勿混用）。
- perf 实测（fe_bench foreach 20k 求和，钉核交错）：
  rtcl(interp 内联 foreach) 0.32ms；fast(wasmi/wasmtime) 4.1/4.8ms
  ——每迭代 flush+bind+sync 三次宿主调用 ~200ns vs interp 直写槽
  ~16ns。**gate 收回**（foreach 默认回落 exec_bytecode 解释 = 3.25ms
  ——仍比 HEAD 的 6.94ms 快 2.1×：escape 拒绝解除后单元得以编译、
  回落从树遍历变为字节码解释）。RTCL_JIT_FE_FAST=1 重新放行。
- 预外发现：jitsh 非 fast 回落 vs rtcl 纯解释在 for+lindex 形状差
  2.2×（4751 vs 10480µs）——jitsh 解释路径预存税（hook 探测/记账），
  foreach 形状 10×（3310 vs 353）——独立 OPEN 项。
- 回归 spot-check：fib25 wt 4.0ms（≈interp 4.9×）、fl wt 1.9ms
  （1.6×）——lower.rs final_depth 改动无 perf 回归。

## 门禁终态（9d1a830）

judge 88/88×2、battery 33/33×2、probes 18/18×2、fuzz 800 例 0 分歧
×2、fe/lm/mg 10 形状 ×2 模式（默认+FE_FAST=1）全对 tclsh、
rtcl-vm 809。

# ---------------------------------------------------------------------------
# 会话续（同日）：jitsh 回落税两根因修复 + foreach arena 决策 @ 51928dd。
# 墙钟含 ~4ms spawn（best-of-3 钉核），比例口径为准。

## 回落税两根因（21ae920）

1. classic 池从未兑现 opt-in：exec_proc_body 对非 fast 体无条件走
   classic wasm（注释声称仅 RTCL_JIT_POOL=1）——逐命令宿主分派比
   exec_bytecode 慢 3-7×。修复后默认回落 exec_bytecode；
   foreach 形状 6.9×→1.05×。
2. gate v5：循环体内 CallSite 拒绝（callsite_in_loop 静态分析）——
   fast 的每迭代 CallSite 协议（flush+分派+dbd9239 无条件 dirty→
   resync）~500ns/iter vs interp ~190ns，tc lindex 循环 2.7× 净亏。
   拒绝后 exec_bytecode 平价。fib 递归不在循环内不受影响（wt 5×）。

## 顶层 classic 同款（51928dd）

run_top_with 对 callsite_in_loop 的顶层单元回 interp（module.l3 复
用）：dict_ops 162→7ms、list_ops 55→7ms（墙钟，与 rtcl 平价）；
arith_loop 11/8ms（wt，赢 1.3-1.8×）与 var_incr 保留。
bench/cases 六例 jitsh 全部 ≥ rtcl 平价。

## foreach arena 决策（不做）

fast foreach 转正的唯一路径（bind 纯 wasm 化：Start 预展开 tagged
元素到 arena + base 打包返回 + Bind wasm 直写槽）评估后放弃——
interp 内联 foreach 的 bind 是原生槽直写 ~16ns/iter，wasm 侧即使
零宿主也 ~20-100ns/iter（wt 指令数下限/wasmi 解释地板），结构上
不可能赢过 interp；foreach 形状的正确终态 = exec_bytecode 回落
（已平价）。RTCL_JIT_FE_FAST=1 实验档保留（语义全绿背书仍在）。

## 门禁终态（51928dd）

judge 88/88×2、battery 33/33×2、probes 18/18×2、fuzz 三种子 1200 例
0 分歧×2、17 形状×2 引擎（timing 脚本剥首行）、rtcl-vm 809；
fib wt 2.95ms（5.0× interp）、fl wt 1.3ms（1.7×）、bench/cases 六例
全 ≥ 平价。

# ---------------------------------------------------------------------------
# aggressive_tier 目标感知策略 @ 75f47d7（用户指令「接近即保留」）。

v5 回落决策全部基于 native 口径。wasm32 target（rtcl-wasm）下基线
反转：解释器自身跑在宿主 wasm 引擎里（每 op 间接分派），JIT 产物
由宿主引擎原生编译——native 平价/小亏形状在 wasm32 反超。

aggressive_tier()：wasm32 自动启用；native 用 RTCL_JIT_AGGRESSIVE=1
覆盖。三决策点：gate v5 callsite_in_loop 拒绝 / 顶层 v5 回落 / 非
fast 体 exec_bytecode 回落——aggressive 下全部保留 JIT 路径。
实证：tc 的 p 默认 reject:callsite-in-loop → AGGRESSIVE Some(1)（值
正确）；两模式 judge 88/88×2、battery/probes 全绿、fuzz 900 例
0 分歧（保留路径正确性由 native AGGRESSIVE 档背书）。

wasm32 编译：rtcl-jit（jit-exec）/ rtcl-wasm（jit）cargo check 干净。
注：wasm32 上 fast 层现经 wasmi-in-wasm 执行（rtcl-wasm jit =
jit-exec）；「产物交宿主引擎原生编译」（js-sys 桥出字节）是兑现
aggressive 红利的后续工作——策略开关先行，浏览器 bench 是下一步。

# ---------------------------------------------------------------------------
# 目标对齐（2026-11-01 用户拍板）+ 双线落地 @ 3813969。

**对齐结论**：双线并行——in-process JIT（Lane A，架构合并）与 towasm
产物（Lane B，独立）。依据：用户最初指令「目标产物性能」（MoonBit
式产物观）+ wasm32 部署论证（jit 本地执行 vs rtcl 解释退化）。

## Lane B（towasm，8a76420）

rtcl-towasm crate（lib+bin）：compile_script 纯编译（不需解释器实例）
+ wasmparser 解析产物字节的 ABI 契约清单 + CLI（-o/--manifest/
--verify）+ 跨引擎回环门禁（解释器 vs wasmi 产物 vs wasmtime 产物
三方对值，5 形状）。
**roundtrip 揪出真缺口**：classic 端际缓存回写——caching=1 期间
StoreVar 只写 wasm 值栈缓存且随 Store 丢弃，interp 帧从未回写（单
单元 judge 全经 set_result/puts 宿主出口不可见；跨单元读帧 = REPL
多次 eval/towasm 消费方/rtcl-wasm 嵌入读到陈旧值）。修复 = 两引擎
execute 尾部逐 var 查 STATE(=2) → set_var_raw 回写。
**架构事实**：产物仅含顶层单元（proc 体运行时编译 = jiterpreter
模式，清单如实标注）——AOT 模块集 = towasm 路线图首位。

## Lane A（合并计划，3813969）

docs/JIT-ENGINE-MERGE.md：引擎镜像盘点（fast-tier wasmi↔wasmtime
~800 行逐函数镜像 + classic linker ×2）、目标布局（engine/ 模块树 +
host.rs 迁出）、EngineOps trait 草图（收敛 7 条已定谳 API 不对称）、
五期计划（P1 纯搬迁→P5）每期全门禁纪律、与 towasm 的 pub 接口约束
（emit::compile + execute_artifact_* 不得破坏）。

## 门禁终态

towasm 6/6、judge 88/88×2、battery 33/33、probes 18/18×2、fuzz 800
例 0 分歧×2、rtcl-vm 809。

## Lane A P3-lite（e1679ac，2026-11-01）

fast-tier wasmi↔wasmtime 镜像合并：FastShared 共享层 + sh_* 逻辑体
10 函数单点化，两引擎闭包瘦身（胶水+委托）。1656+887 → 1441+680+314
行；本会话 p05/dirty/sync/kind5 系列「每个修复双处落地」的痛点解除。
perf 无回归（fib wt 3.0ms/5.0×、fl wt 1.4ms/1.6×、wasmi fib 平价）。
门禁双引擎全绿（judge/battery/probes/fuzz/towasm/rtcl-vm）。

## Lane A 收官（f003957，2026-11-01）

P1 物理布局 + P2 Host 迁出完成，合并治理收官：
- proc_exec.rs 1656 → **101 行**（纯钩子：install + 策略 + 引擎分派）；
  exec.rs 1673 → **557**（纯顶层驱动）；Host → host.rs（1176，叶模块）；
  fast/{shared,wasmi,wt} + classic/{mod,wasmi,wt} 各归其位。
- P4 classic linker 合并评估后放弃：逻辑已 100% 单点化（Host 方法），
  剩余为注册胶水（类型使然），trait 成本 > 收益。
- perf 无回归；门禁双引擎全绿（judge 88/88×2、battery、probes、
  fuzz 800×2、towasm 6/6、rtcl-vm 809）。

## A 线深挖（2026-11-01 续：f66f5f1 + 9976009）

1. **默认引擎翻转**（f66f5f1）：jit-native 在位时 engine_from_env
   默认 wasmtime——fib25 默认 15.0→3.0ms（5.0×）、fl 3.2→1.4ms
   （2.3×）直达默认路径；纯 jit-exec 构建（rtcl-cli）保持 wasmi；
   显式 RTCL_JIT_ENGINE 恒优先。
2. **稳态指令谱定谳**（9976009）：fib fast 模块 WAT——wrapper 58
   指令/调用 + fast_impl 稳态 ~57，合计 ~115 ÷ 12.4ns ≈ 2.7 指令/ns
   = Cranelift Speed 代码质量地板；残余可瘦点（dirty 门/slot_base/
   fuel）各 ≤5% 且均带语义，不动。
3. **fast_mask_set 直排**：i64.store 栈序地址在前，tmp64 中转消除
   （-2 指令/次 × 每迭代 2 次）——fl 对 interp 1.6-1.7× → **1.82×**
   （0.55×，同窗 best5 均值）。
4. **基线修正**：arith 顶层 4 引擎全平价（80ns/iter，name-keyed 变量
   查表主导）——顶层循环的真正杠杆 = slot 化（top-level locals_mode，
   独立大项，OPEN）。

## 对齐修正（b3fe4a2，2026-11-01）：默认引擎回归 wasmi

用户对齐：**JIT 的进程内执行 target = wasmi**（纯 Rust 全平台可用，
含主 target wasm32-unknown-unknown 的 wasmi-in-wasm 部署路径）；
wasmtime = 显式 opt-in（RTCL_JIT_ENGINE=wasmtime + jit-native，
native bench 档）。f66f5f1 的默认翻转撤销，fib 5.0×/fl 1.82× 保留
为 wasmtime 档数字（bench 用 RTCL_JIT_ENGINE=wasmtime 复现）。

产品层分层（既状确认）：
- rtcl 默认构建 = 纯解释器（零 JIT 依赖）
- rtcl --features jit = 解释器默认 + JIT 特性开关（fast-only 策略）
- jitsh = JIT 引擎执行体/差分 oracle（hook 全量）

## top-level slot 化 C1（fcaa578，2026-11-01）

顶层单元 locals_mode 编译落地（设计 docs/TOPLEVEL-SLOTS.md）：
- Compiler：compile_unit_toplevel 新入口（locals_mode + toplevel_slots）；
  framed 解耦（顶层 slot 单元保留错误帧）；foreach/lmap 内联门收窄
  （顶层保持分派——bind 写帧槽背衬 = C2）。
- eval 深度门：仅最外层 eval slot 编译；嵌套体 eval（dict update/
  with 别名）保持非 slot。
- JIT gate 拒绝 toplevel_slots 单元（C4 范围：全局别名语义未覆盖
  ——params_aligned=Some(0) 意外满足 fast 门，foreach body 行号
  错位实证）。
- C1 零 perf 变化（LoadLocal 走 fallback 名键读同速）；C2（tslots
  背衬 + 分派边界 flush/reload）为提速层（预期 94→~30ns/iter）。
- 实验路径：盲切 → judge 85/88 → 深度门 88/88 → rtcl-vm 4 测试 +
  fuzz 12 分歧（foreach 装饰/catch 形状/顶层 fast 误纳）→ 三层修复
  全绿。教训：locals_mode 一开，foreach/catch/错误的语义面全动——
  fail-safe 白名单 + 全门禁矩阵缺一不可。

## C4——顶层单元进 fast tier（e992d61，2026-11-01）

顶层 slot 单元（C1 locals_mode 编译）走 fast tier 执行：gate 放行 +
run_top_with 接线（execute_fast_top 引擎分派 + JitRun 映射 = finish
同款 rc 语义）。既有机制全覆盖顶层全局别名：emit_slot_flush 前置、
分派后无条件 dirty→resync、wrapper epilogue exit flush、ts 作用域
take/restore（嵌套 proc 执行时悬挂）。

实测（裸顶层 arith 300k，墙钟含 ~4ms spawn）：
- jitsh@wasmi 25ms（interp 31 = 1.24×）
- jitsh@wasmtime **11-15ms（interp 31 = 2.5×）**
- 值正确。

意义：顶层循环（Tcl 最常见的性能热点）脱离解释瓶颈——C1 编译 +
C4 fast 执行的组合让**整个脚本**（不只 proc 体）享受编译执行。

门禁双引擎全绿：judge 88/88×2、battery 33/33×2、probes 18/18×2、
fuzz 800 例 0 分歧×2、towasm 6/6、rtcl-vm 809。

## fuzz f000137 定性（359e76d 后复查，2026-11-03）

preexist_wasmi_f000137（lmap 体 $n 读）：三方 = tclsh 静默成功 /
rtcl 报 can't read n（rtcl 自身的 lmap 体词法作用域 quirk，preexist
rtcl≠tclsh）/ jitsh 静默成功 = **与 tclsh 一致（更正确）**。fuzz 门
的 jit==rtcl 判据把该例计为分歧——实为 jit 修正了 rtcl 的 quirk。
非本系列回归（C1/C4 前后均为 rtcl 错、jit 对）。

附带发现：`time {...}` 体是嵌套 eval（深度≥2）→ 非 slot 编译——
所有包 time 的基准从未测过顶层 slot 路径（测量方法学修正：裸循环
文件墙钟）。

## C2'——解释器 tslots + C4 gate 重开（76b7056/c3c7e32，2026-11-03）

C2 双写者根因（顶层变量=全局别名，ts 缓存+名路径双写）证伪后重做：
**ts = 私有缓存**（TopSlots{names,values,aliased}），名路径只触全局，
边界同步。三层模型下解释器是产品默认——本项把 C1 顶层槽编译的收益
落到 rtcl 默认构建。

### 机制
- top_op_class 穷举三分类：Pure(0) 栈/算术/槽 op+全融合族 / FlushOnly(1)
  名键读 / FlushInvalidate(2) 分派·名写·数组化；新 op 编译期强制分类
- 名路径 ts 臂 7 处（get_var/set_var/incr_var_fast/lappend_slot_fast/
  frame_slot_*），直通咽喉（scalar_top read/write）由 class 1/2 边界
  背书
- **悬挂同步**（e.tcl lappend 丢失实证）：exec_bytecode/ns eval 的
  take 前 flush + restore 后 invalidate（嵌套单元直写全局，恢复副本
  stale——flush 会反向覆盖，只能失效）。EvalExpr(class 1) 括号内嵌套
  单元是盲区来源：class-1 不失效，靠悬挂点同步兜底
- 边界降级（trace/dead 中途注册 → flush+永久 ts=None）；aliased 初值
  =array_globals；set_var 数组化标记 ts.aliased
- 融合 op 白名单（SubEndStoreLocalPop/ForNextIncrLocal 等）：漏列即
  每迭代 flush+invalidate（440ns/iter → 132ms 回归实证）

### 性能（裸顶层循环 300k，墙钟）
- rtcl interp：31ms → **10ms（-68%）**——裸顶层 arith 与 jitsh@wasmtime
  同档（11-15ms），解释器默认构建首次追平 Cranelift 顶层形状
- jitsh@wasmi 25ms → 顶层循环相对 wasmi 反超（10 vs 25）——wasmi
  顶层 fast tier 的存在意义需重估（见 OPEN）

### 门禁
judge 88/88×2（interp + jit 三向）、battery 33/33、probes 18/18、
fuzz 500×2 引擎 0 分歧、rtcl-vm 809、towasm 6/6。
C4 gate 重开独立复核：f000137 三方 rc=0 一致（has_foreach 编译侧排
除后分歧源结构性消失）+ lmap 括号逃逸形状（continue/break/过滤）
三方一致。

### preexisting 缺口（本轮发现，非 C2' 引入）
ns eval 体 `set` **已存在的全局裸名**写全局（tclsh 解析链含 global
fallback——`set v9 100` in ns 体覆写 ::v9），rtcl 一律建 ns 表
（::demo::v9）。全新名两侧一致建 ns 表。ts 悬挂期（名路径纯全局）
与本分歧正交——名字解析规则专项待办。

### 教训
- 悬挂 take/restore 的同步语义要在**建立悬挂时**一次想全：flush 的
  方向性（pending→全局 vs 全局→槽）在两端相反
- 探针定位分歧：RTCL_DBG 环境门 eprintln 逐写点插桩（flush/fill/
  fsw/fswm/set_var-arm），一轮跑出完整读写序——比 stash 二分快
- 单次执行 ≤10min 硬规矩（构建 -j 2 ~2min15s，后台+短轮询）

## 名字解析专项 + ts ::-leaf 根限定（ce1d9fc，2026-11-03）

tclsh oracle 探全（nr1-nr4）：eval 级变量解析链两级——**当前 ns 表 →
全局；父 ns 不在链上**（a::b 体 set x 建 a::b::x；读 a::y 报 no
such）。写命中链上首个已存在者，创建落当前 ns；variable 声明记录
（含 valueless）截链。

### 修复面
- A：ns 体 set 已存在全局裸名 → 全局（原建 ns 表）
- E：ns 体 unset 全局裸名 → 全局（原报 no such variable）
- L：ns 体数组元素 → 全局基（原建 ns 数组）
- ::-leaf ts 臂劫持（C2' 回归）：$d7::s 的 leaf "s" 撞顶层槽 s，
  读/写劫持成槽值。leaf 撞名仅对根限定 ::leaf 成立——两臂改
  strip_prefix("::") 且 rest 无 ::/(。I 形状（variable 声明后 set）
  全程由此造成，variable 命令无辜（种子 d7::s=9 本就正确）

### 复核关闭
lmap 体作用域 quirk（f000137 定性遗留）：proc 内体读外层局部/体写
变量体外可见/双变量列表/体读全局四形状三方一致——原分歧源随
has_foreach 编译侧排除结构性消失。专项关闭。

### 教训
- ts 臂的「同名等价」假设要精确到限定形式：leaf 撞名 ≠ 同一变量
  （rsplit 惰性——根限定才可劫持）
- divergent-shape 探针先行（nr1-nr4 oracle 表）再动代码：一次定准
  规则（两级链、无父链、声明截链），避免按直觉实现父链回退

### 门禁
battery 33/33、probes 18/18、judge 88/88×2、fuzz 500×2 引擎 0 分歧、
rtcl-vm 809、towasm 6/6；本机差分 11/11；bare.tcl 10ms 持平。

## 深水区检测（allium 规约驱动，1dd2d81，2026-11-03）

用户指令：借助 allium skill 检测深水区正确性。规约先行——
specs/variables.allium 深水节 16 rule（ns×proc 帧隔离/upvar 交叉/
variable 链/delete/which/ts 桥），specs/variables-deep.probes 17 探针
经 specs/probe.sh 三方差分，oracle 定性后修 rtcl。

### 修复（3 真 bug）
- **upvar 0 ::a(e) x 元素链**：parse_level("0")=Rel(0) → Frame 臂 →
  链接 = 字面名 "::a(e)" 的帧局部。Frame 臂根限定目标切全局链 +
  set_var 元素形式 loc 键重入（**良构守卫 ends_with(')')**——
  set-old-4.5 半开名 a(bcd 是标量，裸 contains('(') 递归 = 栈溢出，
  judge gen_set-old 拦截）
- **死父 ns 建链**：can't access "::d::v": parent namespace doesn't
  exist（三臂统一——GlobalNs/eval/Frame）
- **variable 瞬态链接**：eval 级 variable 链接移 ns_transient（体帧
  作用域，ns eval 入口 take/退出恢复）；upvar eval 别名留
  flat_aliases 持久（var-3.10）。教训：两类链接生灭语义相反——
  save/restore flat_aliases 整表曾把 var-3.10 一并杀掉（judge 回归
  拦截），分表才对

### 定性澄清
- ns 内定义 proc 顶层不可见 = **伪分歧**（首轮 probe.sh 的 rc 列
  误读；catch 值形复核三方一致）
- 自删形状 = 真分歧 → DIVERGENCES.md D-2 OPEN（帧视图 vs 表复活）

### 门禁
judge 88/88×2、battery 33/33、probes 18/18、fuzz 500×2 引擎 0 分歧、
rtcl-vm 809、towasm 6/6、深水 16/17、nr 11/11、bare 10ms 持平。

## D-2 关闭（d72465c，2026-11-03）

体中自删的帧视图语义：oracle sd1-sd3 探全后修复两处——
- get_var/var_exists **限定名死父 ns 守卫**：限定读 → can't read:
  no such variable、exists → 0（体内体外一致；裸名 keep-alive 链接
  不受影响）。教训：守卫的父键须先剥前导 ::（根 ns 下 canonical 原样
  返回 '::d::x'，直接 rsplit 拼 '::::d'——gen_namespace-46.3/深水
  3 探针回归拦截）
- ns_eval **退出清扫**：死者前缀的 globals/array_globals 残留随体
  消亡（post-delete 写入 + keep-alive；体中重建该 ns 则跳过）

意外收货：工作树出现三份新深水探针（dict/error/matching-deep，
21+21+23 行，同格式）——随 D-2 提交入库并跑差分，挖出三个新语义域
的真分歧（dict incr 非整数、catch options 保真、正则惰性量词等，
见 DIVERGENCES.md 后续）。

门禁：judge 88/88×2、battery 33/33、probes 18/18、fuzz 500×2 引擎
0 分歧、rtcl-vm 809、towasm 6/6、深水 17/17、本机差分 14/14。

## dict/error 域深水修复（1b80c20，2026-11-03，并行会话协作期）

dict-deep 15/20、error-deep 13/20、matching-deep 13/22、variables-deep
17/17。修复清单见提交（catch options 语义族/exit 穿透/throw tclsh 化/
dict 前缀缩写族/\\u 校验）。

**并行会话协作注意**：工作树出现共享在途编辑（defer_flow_frame/
eval/vm_exec/channel——io 域）。gen_join/gen_ioCmd/gen_split/b12 四例
门禁回归经干净 HEAD A/B 定性**归属并行在途编辑**（join 错误对象在
fresh interp 下已非 WrongNumArgs——与 catch 域改动无因果），移交该
会话。多会话同树协作纪律：提交前 A/B 干净 HEAD、按文件归属切分。

剩余 OPEN（记 DIVERGENCES.md）：dict info（hash internals 不可复现）/
dict getdef（rtcl 多出的 8.7 特性）/errorstack+errorline+options 键序
（catch {error boom} vs catch {nosuchcmd} 两种键序）/try 体 errorInfo
帧/正则惰性量词组语义（Henry Spencer 最短整体匹配——(a+?)(a*) →
"a a {}"）/lookahead 的 crate→bt 回退缺口/\\u 外转义文本。

# ---------------------------------------------------------------------------
# perf/wave2 深水区组合拳（2026-11-16，基线 7f2e0df）
#
# 起点重锚定：HANDOFF §10.2 的六基准数字（fib 0.83x 等）是
# perf/interp-arch-opt-2 分支快照（Round AH 收口时点）。wave2 语义轮
# （errorInfo/IO/OO 可见性/dict/expr，367 提交）落在其后，热径有新增
# 簿记——本分支今日实测（指令数，确定性口径）：
#
# | bench        | rtcl instr    | tclsh instr   | 指令比 | wall 比 |
# |---|---|---|---|---|
# | fib25        | 578.67M       | 598.04M       | 0.97x  | 0.98x   |
# | fe_bench     | 613.49M       | 624.01M       | 0.98x  | 0.83x   |
# | dloop        | 4116.54M      | 4355.88M      | 0.95x  | 1.08x   |
# | oo_bench2    | 5839.91M      | 9018.07M      | 0.65x  | 0.64x   |
# | oo_bench2_vars| 7109.97M     | 9490.53M      | 0.75x  | 0.90x   |
# | build_loop   | 128.36M       | —             | —      | —       |
# | micro_base   | 392.72M       | 337.31M       | 1.16x  |         |
# | micro_incr   | 557.73M       | 391.52M       | 1.42x  |         |
#
# 六基准指令比仍全 ≤1.0x；语义基线门禁：judge 88/88、probes 380/35
# （同机态 A/B 复核锚定）。
#
# ## 板点 1（call_proc 帧准备）——落地（458c80e）
#
# 画像（fib25，perf script 自聚合 + perf annotate 行级）：call_proc
# self ≈4.5%，其中池收发面（slot_table=None/upvars.clear/Vec len++）
# ≈2.5%。落地「帧池不变式去双清」：两个 push 站点全清后才入池
# （coroutine drain 不回池，审计确认）→ pool-pop 路径 11 项 clear
# 全删；尾调用重绑门控（first_iter）——首遍 locals/order/array/upvars/
# level0/slots/slot_aliased/link_keys 清除由不变式背书跳过，重绑遍
# 保留；slots 改 resize 直填。jit_call_proc_direct 同款。
#
# 实测：fib25 **−5.66%**（578.67M→545.97M，0.97x→0.91x）；oo2
# −2.26%；oovars −1.87%；fe/dloop/build 持平（proc 调用少/无 proc，
# 符合改动面）。门禁：tests 1419/0、judge 88/88、probes 381/34
# （唯一翻转 objquad = 坑清单在册的 15s 超时抖动探针，fail 集逐项
# diff 同集）。
#
# ## 板点 2（语句窗口扩展 incr var [script]）——回退（负结果留档）
#（**2026-11-16 重开翻案：真因 = fallback 白名单缺失致整脚本树走查，
# 非窗口运行时成本；修复后落地，见下方「板点 2 reopen」节。**)
#
# 实现：4 新 op（IncrVarStack/IncrLocalStack + [SubEnd,*] 融合对）+
# compile_incr 括号分支（try_compile_sub 内联 + 栈上增量）+
# fuse_pairs 窗口 + unfused 展开 + top_op_class 分类 + INNER hint 臂。
# 窗口发射确认（disassemble：8 op vs 原 10 op，去 incr 分派），语义
# 探针 17 例对 tclsh 全同（含 errorCode/trace/errorInfo 归属），judge
# 88/88、workspace 全绿、probes 381/34。
#
# **实测净负**：dloop 4116.5M→8513.2M（**+107%**）。三变体定位：
# `incr acc [expr {$k}]` 861.4M→1835.9M（+113%）、`incr acc [string
# trim …]` 979.1M→1962.2M（+100%）、无括号 `incr acc $k`（分派路径）
# 持平——回归锁死在窗口发射的运行时。判别实验：仅关编译器分支（新
# op 与热径臂保留）即回到基线 861.37M——排除热径加臂成本，坐实窗口
# 运行时。带符号画像（新路径）见 ExprParser/match_op、Vec<char> lexer
# （expr 逐迭代解析）、redirect_flat、collect_trace_scripts 与
# malloc/free 风暴——但同款 EVAL_EXPR 与 ts flush/invalidate 序在旧
# 形状同样存在，根因未收敛（两次尝试止损，按 Round Z/T 先例回退）。
# ** reopen 线索**：下一步用 RTCL_DBG 环境门逐写点插桩（top_flush/
# top_slot_fill/incr_var_fast→None 计数）对比新旧每迭代 flush 序。
# 顺带发现（预存分歧，与本窗口无关，三路径同现）：incr 非 int 旧值
# 的 errorCode（tclsh TCL VALUE INTEGER vs rtcl NONE）与 i64 溢出的
# bignum 提升（tclsh 9223372036854775808 vs rtcl 回绕）。（这两项已于
# 267d49a 收口。）
#
# ## 板点 2 reopen（RTCL_DBG 插桩 + 白名单根修）——落地（2026-11-16 重开轮）
#
# 上轮回退的真因（RTCL_DBG 逐写点插桩钉死，feature `rtcl-dbg` + env
# `RTCL_DBG[=seq[@N]]`，默认构建零剔除）：**不是窗口运行时**。重建的最
# 小原型（4 op + compile_incr 括号分支 + fuse_pairs）精确复现 +101.3%
# （dloop 4335.9M→8727.9M，同机同日基线 87c468e），插桩对比显示形状级
# 翻倍的真身：`T unit 900010 vs 8`（每迭代 3 次 exec_bytecode 重进入）
# + `T eval_expr 300001 vs 0` + ts flush/invalidate 全消——**整脚本被
# 树走查**。根修点 = 新 op 未入 `Compiler::compile_unit` 的 fallback
# 白名单 → `code.fallback=true` → eval_impl 门（`!code.fallback`）拒编
# 译执行 → eval_commands 树走查（cond/body/next 三片段每迭代各自重解
# 析重编译、ts 永不激活、expr 逐迭代重解析——上轮画像的 ExprParser/
# Vec<char> lexer/redirect_flat/malloc 风暴全部是树走查特征）。上轮
# 「判别实验仅关编译器分支即回基线」的机制 = 关分支→无新 op→无
# fallback→编译恢复；**上轮的窗口运行时从未执行过**。教训入册：新增
# op 必须同步入 fallback 白名单（与 top_op_class 同为穷举强制点，建议
# 未来以编译期清单合并三处）。
#
# 白名单修复后窗口首次真跑（最终落位：枚举尾段连续 tag +
# exec_op 主 match 前置范围预检 + 阶梯实现居 vars.rs
# `incr_var_fast_stack`/`incr_local_slot_stack`）：
#
# | bench        | base(87c468e) | 窗口真跑      | 指令比 |
# |---|---|---|---|
# | dloop        | 4335.9M       | 3862.4M       | **0.891** |
# | fib25        | 545.97M       | 548.86M       | 1.005 |
# | fe_bench     | 613.49M       | 616.29M       | 1.005 |
# | oo_bench2    | 6415.1M       | 6478.1M       | 1.010 |
# | oo_bench2_vars| 7684.1M      | 7746.1M       | 1.008 |
# | sum_only     | 616.2M        | 619.0M        | 1.005 |
# | micro_incr   | 557.7M        | 585.7M        | 1.050 |
# | v_expr 变体  | 2669.6M       | 1747.7M       | **0.655** |
#
# 窗口形状收益（同机 3 连测最优）：dloop **-10.9%**（热循环
# `incr acc [dict get $d $k]`）、incr-bracket 微形状 -34.5%、100k dict
# 形状 -11.6%、`[string trim]` 形状 -1.4%、无括号形状持平。
#
# 语义门：新增 28 形状探针（judge/probes/incr_window_p01..28，含
# return/break/continue 出口、创建、bignum 提升、数组/命名空间/链接、
# trace、tier1 中途重定义 e1-e3 类、next-script、`{*}` 展开与拼接形
# 不入窗守卫）：25 例与 tclsh 8.6.17 全同，3 例失败为**预存分歧**
# （基线同样失败：errorInfo INNER 帧归属、proc 内 ns 限定名 incr、
# incr 读 trace）——非本窗口引入。judge 88/88、jitsh 差分 88/88、
# workspace tests 1420/0、probes archive（含新 28）DIFF 集**与基线逐
# 项一致**。
#
# 残留（已定位、止损接受）：**代码 gen 漂移**。窗口可执行分派代码的
# 任何落位都使 fe/oo2/oovars/sum +0.42-0.58%、micro 矩阵 +2.1-5.0%
# （micro_incr +2 分支/迭代 = 主 match 决策树形状改写）。证据链：本
# 基准字节码逐字节同、RTCL_NO_WINDOW 关分支后漂移同在、stub-
# unreachable 臂零漂移、惰性 canary（50 行 inline(never) 死代码）零
# 漂移、RTCL_DBG 事件计数逐一相同——非执行路径成本，是 full-LTO
# （codegen-units=1）下 5MB text 的全局重内联布局效应。六种落位
# （热径臂/表分派臂/inline(never)/阶梯迁 vars.rs/枚举尾/预检门）漂移
# 逐位相同 → 两次落位尝试 + 预检门变体后止损，接受为已知残留；wall
# best-of-5 噪声（±5-8%，fe -5.1%~+7.9% 随构建翻转）掩盖其量级。尾段
# 连续 tag + 预检门约定使未来窗口族可复用同一落位（主 match 保持基
# 线形状）。
#
# ## 板点 3（word_srcs 借用化）——已在册关闭
#
# Round L（2026-10-18）已落地：`util::WordSrcs（*const SrcSpan, len）`
# 借用型在位（interp/mod.rs:779、util.rs:181）。§10.2 条目陈旧。
#
# ## 板点 4（空循环基线）——画像后关闭（不动）
#
# micro_base 新画像（1M 迭代 ×5，392.7M = 1.16x tclsh）：每迭代仅
# 2 op（CMP_VI_JF + FOR_NEXT_INCR_V，Round U/H 融合已吃净）。残项
# 叶占比：exec_inner 派发地板 ≈14%（pc/界检/epoch/ts 门/jump 表）、
# Value::is_unset 5.7%（槽读 unset 门，语义必需）、frames.last 2.06%、
# Region push(ptr::write) 1.58%。台账的「5-8%」估计系 Round V 时代
# 数字（空循环 517→393 instr/迭代已消化大半）；剩余两项候选——
# bodies unchecked push（省 ~1-2 instr/push，噪声级）与 VmState 帧指
# 针缓存（需在全部 frames.push/pop 站点维护代数，unsafe 面 + 跨分派
# realloc 正确性）——收益各 <2.5% 且风险结构不符，**不动作，关闭**。
#
# ## 板点 5（vars 指针稳定存储）——缓议（局面已变）
#
# 目标数字已消失：oovars 0.75x、oo2 0.65x（均 <1.0x，G13 链接槽键
# 缓存 + Round Q/R/X/AA/Y/AG 已吃掉链接变量哈希面）。残余价值集中在
# dloop 名扫描（split_array_ref/redirect_flat）与 dict IndexMap 固有
# 大头——VarMap newtype 全消费面改造（Round S 实测 178 处机械修复）
# 在当前板面下性价比为负，**缓议**，待 dloop/vars 重新放大后再评估。

# ---------------------------------------------------------------------------
# M1' 收口：顶层 D-11 流解码 + while/for 体帧族对齐（perf/wave2，2026-11-16）
#
# 盘点：M1' 四件套在树内均已落地（ABI v2 import 面 + fuel/epoch globals =
# abi.rs/exec{,_wasmtime}.rs/fast,classic；Int 快车道 = assemble.rs §4 全表
# （比较免 untag、+/- 范围检查、* 高 32 位检查、div/mod 宿主、位运算、
# IncrVar/IncrLocal）；nanopass 链 = pass/ 八 pass + check 校验器）。本
# 轮 = 验收缺口的收口：解释器 errorInfo 大改（cf5e3d8 全包含）后 JIT 宿
主/驱动面的语义漂移。
#
# ## 板点 A（顶层 D-11 流解码）——落地
#
# b12_bare_break 电池红（jit≠rtcl）揪出：裸 break/continue 与 deferred
# `return -code error/break/continue` 在顶层是错误（c3bc477 D-11），
# JIT 驱动 finish() 仍走旧 driver_report（输出裸 "break"/"return"）。
# 修复面：
# - FlowExit 新 IR（裸 break/continue 出口先 rtcl.fail 记点再返回
#   BREAK/CONTINUE；alloc_locals 给单元级 break/continue 烘焙 fail 快照）
# - finish() 三臂分流：BREAK/CONTINUE rc、case-a/case-b pending 中的
#   deferred return → Host::decode_top_flow（vm defer_flow_frame 同款
#   无条件覆写帧 = last_fail 的 site/bodies[0]）→ decode_toplevel_flow
# - CallSite kind-1/2 无循环臂：单元 BREAK/CONTINUE（原落 ERROR 且
#   pending 已被 take_fail 早退消费 = 空输出，f7/f10 实证）
# - take_fail case-b deferred-return 早退（vm = defer_flow_frame，无帧
#   机械；f9 的裸消息守卫同 build_case_a_report/fuzz f001026）
# - jitsh：-f Fallback 分支 decode、-c 整面镜像 rtcl-cli run_command
#   （eval_incremental——JIT 差分门禁全走 -f）
# 23 形状 sweep（裸 break/continue、deferred break/continue/error/
# return、词括号、if 体、eval 体、catch rc、循环内 deferred）+ -c 9 形
# 状三方逐字节一致。
#
# ## 板点 B（while/for 体帧族）——落地（fuzz 41→5）
#
# fuzz seed0 500 例：HEAD 41 分歧 → 5。cf5e3d8 的解释器体帧语义落地后
# JIT unwind 的 Body/Next 臂是空转（{}），fuzz 门在 HEAD 未复跑。修复面：
# - unwind/proc-branch/sh_take_fail 三 sink 复刻 vm unwind_subs 的文件
#   语境体帧（("while"/"for" body line N)、("for" loop-end command)、
#   仅最外层体区；proc 体 = call_proc 强制编译语境，旗标自然 false）
# - jitsh arm_file_script + jit_err_set_file_script（JIT 顶层单元不经
#   eval_impl，arm 无消费点——直设与 rtcl-cli 消费 arm 后同状态）
# - rtcl-vm jit_err hooks：in_file_script/get/set、exit_frame_raw、
#   has_info、append_info_raw、has_pending、jit_err_line
# - alloc_locals LoopExit 体区截断（环后 op 的 fail 快照携带已闭合环
#   区——体帧臂对环外错误误触发，f000357 族；对 for-8.x eval-break 的
#   不匹配 LoopExit 保持旧容忍——panic 实证）
# 残余 5 分歧 = 全部 preexist（rtcl≠tclsh 形状，jit==tclsh）：体帧行号
# 在词括号失败时的 provenance——vm 的 Sub defer 不动 err_line、而 jit
# 驱动 harness 更新镜像——需要 vm 端行号语义专项（reopen 线索：
# bench/scripts/jit_fuzz_findings/preexist_wasmi_f000{054,162,213,246,
# 272}.tcl，family = ("while"/"for"/"foreach" body line N) 行差 1-2）。
#
# ## 验收数字（同窗钉核 best-of-5，含 spawn）
#
# | case        | tclsh | rtcl(interp) | jit@wasmi | jit@wt | vs bd3e169(249ms) |
# |---|---|---|---|---|---|
# | arith_loop  | 35    | 10           | 13        | 13     | 19.2×（门 ≥3×，<83ms） |
# | var_incr    | 42    | 11           | 13        | 12     | — |
#
# jit@wasmi 13ms = 0.37× tclsh（门 ≤1.9×）。A/B（HEAD cf5e3d8 同窗）：
# interp/jit 全部噪声级（oo 1434→1404、dloop 300→280、fib 15→15、
# arith 11→10）——无回退。六主基准 + 微套件同窗复测无回退。
#
# 门禁：judge 88/88×2、battery 43/43（+b34-b43 流/体帧固化）、fuzz
# seed0 500 例 5 分歧（HEAD 41；残余见上）、workspace tests 1420/0、
# roundtrip（jit-exec）5/5。

## 审计后续轮 perf wave——T6 通道缓冲 + P7 编译期三连（f699214/4d9ba2a，2026-11-26，子代理施工）

### T6 通道缓冲读（task/t6 @ f699214）
- P9 文件 32MB `read`：396ms → **36.7ms**（tclsh9.1 同窗 26.6ms，**1.38×**；原 14.1×）。
- P1 TCP 3MiB `read`：597ms → **5.6ms**（tclsh9.1 4.8ms，**1.17×**；原 127×）。
- 做法：Channel trait 加 `fill_buf`/`buf_consume`（FileChannel/OsStdin/PipeChannel 委托
  自身 BufReader；TcpChannel 自带 64KB in_buf）；read/gets 块快路径（utf-8/字节编码、
  无 eofchar、管线无残留时整片扫描 EOL 翻译；片尾悬置 CR 不消费交慢路径裁决）；
  chan copy 同编码整片快路。**试错教训**：cfg 级预读块会脱同步 tell/seek-Current/r+
  写回卷（tell 25 vs 8 实测）——缓冲必须在通道内，不进 config。
- 基准脚本入仓：`bench/scripts/channel_read_bench.sh` + file/tcp-server/tcp-client tcl。
- 遗留：gets 循环 61 万行 920ms vs tclsh 345ms（2.6×）——瓶颈在每行 Value 装箱/
  `set_var` 命令层，非通道层，未动（新台账项）；9.1 对坏字节 `read` 报严格 utf-8
  错误（rtcl 仍 8.6 逐字节出字符）——归 TODO 对标迁移 encoding 面。

### P7 编译期三连（task/p7-prof @ 4d9ba2a，先 perf 定位后修复）
- 定位（perf 可用，paranoid=-1，样本 211–522/形状）：for 形 **88% 在 count_lines
  前缀重扫**（每内联体从源码 0 字节重数换行 → O(字节²)，字节缩放实验锁死）；set
  异名形 **90% 在 add_local 线性扫**（`add_const_wide` 早有索引、局部表漏了——同名
  A/B 0.01s vs 异名 0.15s）；fuse_pairs retarget 交叉积 4.7%（修完 #1 后的主要超
  线性项，一并修）。行号站点存储侧排除（emit 摊销 O(1)，profile 无热点）。
- 修复：LineIndex O(n) 前缀表 + partition_point 二分（**22 形状行号电池修复前后
  errorInfo 逐字节 diff 为空**，第一版把换行计数误当偏移被电池当场抓住）；add_local
  加 `local_index: HashMap`（locals Vec 插入顺序不动——槽号契约）；retargets →
  HashMap（from 值每窗口唯一，五臂 apply 语义不变）。
- 数字（同窗 best-of-3，负载 4.4–8.6）：for5000 0.28s → **0.01–0.02s**（tclsh9.1
  0.03–0.04s，~7× → **0.5×**）；set16000 异名 0.12–0.15s → **0.01–0.02s**（~11–15×
  → **1×**）；字节缩放 108→222KB 修复后恒 0.01s，二次型消失。
- 顺带定性：PARSE_CACHE_MAX_SCRIPT=256KB 封住大单元可观测窗口（for20000=764KB、
  set30000=270KB 均走树，见 specs/TREE-FALLBACK-TRIGGERS.md F5）；body_inlinable
  同体解析两遍（线性 ~0.7%，未动）。
- 门禁：judge 88/88（9.1 oracle）、crash-gate 126/126、sweep 12 既有名单不新增、
  probes 315/122 不新增、quick-gates 全 rc=0、workspace exit 0。objsec 探针曾单轮
  123（墙钟秒数跨 1s 边界，负载 ~13 抖动；同二进制 3/3 复跑 MATCH + 修复前二进制
  复核 123 名单确认唯一波动）。

## T3 info locals/vars 直列——降级税消除（task/t3 @ af0f95d，2026-11-26，子代理施工）

- 语义：proc 里 `info locals`/`info vars`/`info frame` 改为按 tclsh 顺序直列
  （编译段槽表序 + 哈希段 `tcl_order` 重放），`degrade_frame_all` 不再被列举
  触发（函数成死代码，已删）。`rr.sh locals-order.tcl` 四方逐字节一致
  （86=91=vm=tree）；另造 9 个对照脚本全对齐。
- 连带语义修正（tclsh 钉形）：compile_set/compile_incr 目标槽先登记（原值词
  先登记，编译段次序反）；`global`/`variable`/`upvar` 的字面参数按 tclsh 登记
  为编译局部槽（NSUPVAR/VARIABLE/UPVAR 槽操作码，`info locals` 排除、`info
  vars` 列）；`global`/`upvar` 对已有值局部报 `variable "x" already exists`。
- 记档不修（边界）：unset-only/`catch {} var`/`dict set` 字面目标不登记编译
  局部（需新编译形态）；append 未编译（T5 范围）。
- 数字（bench/scripts/info_locals_bench.tcl，µs/iter，vs 39bf618）：
  list 1.66→**0.52**（3.2×）；after（列举后槽热循环）9.44→**4.71**（2.0×，
  ≈无列举底噪 4.53——降级税消除）；plain 4.60→4.53 不变。tclsh9.1 参照
  list 0.17 / after 1.44 / plain 1.28。
- 新增 `judge/crash-probes/d28_local_order.tcl`（两步流程生成），crash-gate
  126→**128/128**。judge 88/88、sweep 12 既有名单不新增、probes fail=122
  基线、workspace rc=0、vm-lib 两特性 rc=0。

## T7 性能小项四连（task/t7 @ 96e81cd/acd9db6/3d2ffc3/2c68059，2026-11-26，子代理施工）

### P5 array size/exists O(1)（96e81cd）
- Interp/CallFrame 记 `elem_counts`（set_var 元素写两分支、unset 元素删、mark_array、
  loc_remove_array、ns delete/oo destroy 直删、env/tcl_platform 播种维护）。
- 8 万元素 300 次/段：`array size` 2,023,992→**248µs**（常数）；exists 未命中
  3,221,210→261µs。tclsh9.1 的 array size 实测也非常数（8k 44µs/80k 449-1253µs）
  ——rtcl 现在更快。

### 数组元素读写零分配直取（acd9db6）
- get_var/set_var/var_exists/incr_var_fast/cmd_append 五处数组分支加守卫径：平键
  与给定名逐字相同（帧 array-local 或 :: 顶层全局）且无死链/平别名/ns 瞬态/无
  trace → 一次哈希直取。语义先钉后动（judge/probes/t7_arr_elem.tcl 46 行，改造
  前后对 tclsh9.1 逐字节一致）。
- **顺带发现**：stdlib init 后 `ns_transient` 残留 `auto_execs_arr` 自映射一条
  （预存泄漏）；粗粒度 is_empty() 守卫会永久失活——改按基名精确匹配。
- 数字（50k-100k 次/段，比值=rtcl/tclsh9.1）：write 2.05x、read 2.02x、incr
  **0.51x（反超）**、append 0.83x、read-50k 1.29x、write-new 2.09x、write-old
  1.12x——全部 ≤2.1x（达标线 ≤3x）。

### P15 string last 三处（3d2ffc3）
- 完整反向候选走（rmemchr 按字反跳+字内精化；Horspool 移位表被暴力对照证伪——
  覆盖论证只对前向成立，反向漏右端 1935/200000，已弃）+ SWAR 连续字节计数
  （24KB 12µs→3.1µs）+ ASCII 分类三次扫合并。
- last-miss-sp 11786→1275（**0.34x 反超**）；last-hit 1733→651（4.97×）、
  last-hit-uni 7157→1629（11.16×）**未达标——架构下界**：hit 余量是 string
  ensemble 分派底（0.23µs/次 vs tclsh 对象内缓存 0.1-0.2µs 全程），uni 余量是
  字符长度无缓存（SWAR 3.1µs 是本机地板，5 种写法实测同）——根治 = Value 层
  字符长度缓存（审计 §3.4 已记根因），立后续项，未盲修。

### P6 变量表哈希塌缩（2c68059）——先量后修，实测成立且严重
- Python 逐位复刻 VarHasher：6 万个 `k000000..kN` 只占 131072 桶的 **32 个**
  （链长中位 1856）；端到端同前缀族比变长族慢 45%。此前"顺带好了"不成立。
- `finish()` 加 murmur3 fmix64 雪崩（与 value.rs 字典键 FxHasher 既有修复同
  处方）；修后四族全 4.8 万桶、链长≤6；6 万变量整表读回 -20~-40%，族间差
  1.45x→1.09x。哈希值变 = 全部变量/命令表迭代序变——judge/probes/sweep/
  tests 全绿实证迭代序对输出无承载（var_order 重放设计成立）。

### 门禁（合并主树后复验 02:50）
judge 88/88、crash-gate 128/128（d28 在内）、sweep 12 既有、probes **316**/122
（t7_arr_elem 新探针入列，无新 DIFF）、workspace rc=0。

## T4 命名空间体裸名快路径——反超 tclsh（task/t4 @ a8e4f2a，2026-11-26，子代理施工+主会话收尾）

- 读侧三臂（read_var_scalar_top）：:: 顶层裸名臂原样；ns 体裸名键
  `<ns>::<名字>` 栈上拼（64 字节封顶回落）单次探测；根限定单叶 `::leaf`
  直读 :: 表。get_var/incr 热径与 scalar 直写守卫同构造；写只覆写已存在键。
  expr_compile bare-word 字符集补 `:`——`$ns::v` 编进 LoadVar 免逐迭代重解析。
- 顺带补齐 T1 的 S8 读侧：get_var 元素读残留的 8.6 全局数组回落按 9.1 移除
  （探针钉形）；read_var 两 trace 表全空时直走 get_var。
- **数字（bench-ns-body 1500 万次，同窗交错三轮）**：rtcl 1.9-2.9s vs
  tclsh9.1 2.7-4.4s——**反超 0.65-0.72×**（基线 ~40s，约 15×；P16 达标线 3×）。
- 行为零变化：t4_ns_body_var 探针（错误文本/errorCode/trace 序/variable 声明/
  ns delete/数组路由/超长带 :: 带 ( 回落）对 tclsh9.1 逐字节一致，vm/tree
  三方一致。
- 门禁：quick-gates 十一项 rc=0、crash-gate 128/128、sweep 12 既有、probes
  **317**/122（t4 探针入列）、workspace cargo exit 0。
- 施工注记：子代理两次被机器 OOM 杀（进程级），主会话接手收尾——改动本身
  一次成型，验证链（探针/基准/门禁）由主会话补齐。

## D5 num-bigint 任意精度集成（task/bigint @ f7a3241，2026-11-26，子代理施工）

- Value：`InternalRep::Big(Rc<BigInt>)`——i63 快路径零改动；溢出落 Big 箱；
  Clone 是 Rc 份额复制；`as_str` 惰性渲染 canonical 十进制。杠杆点一处：
  `bignum::to_value` 从字符串箱改 Big 箱，全部算术 widen 臂自动落新表示。
- 语义修正三处（探针实证）：`int()` 不再截 word（9.1）；`op_pow` 补 bignum
  底/指数臂；`incr` 接任意精度 + errorCode 9.1 分工。**勘误**：9.1 除法/
  取模是地板除（`-7/2=-4`），非向零除——设计稿已更正，现状实现本就正确。
- 探针：bn_int91/bn_pow91/bn_bits91/bn_incr91 四个新入仓，oracle/vm/tree
  三方逐字节一致。
- 门禁：judge 88/88（旧口径）/合并后新语料 case 级 **4157/4849=85.7%**
  （基线 4152，bigint 清 5 case）、crash-gate 128/128、probes **326/117**
  （溢出族 5 个 8.6 钉形转 9.1 一致，DIFF 122→117）、sweep 12 既有、
  workspace exit 0、wasm32 过。
- wasm 体积：perf 轨 −1114B；size 轨 +2567B（+0.14%）——num-bigint 本在
  链接链，alloc-only 开关自抵。
- 基准：arith_loop/var_incr ±5% 内（快径零改动）；bignum 链收益：阶乘
  400×30 轮 58ms→**9ms**、2^63 加链 10 万次 47ms→**18ms**。

## B2 对齐与加固批（task/b2-execfile @ 708d29b/0ae5636/caebf8d/4246531/d6654b6，2026-11-26，子代理施工）

- **C32**：宿主 HostExec exec 路径按 9.1 重写（开关表同原生路径、重定向
  连写、TclCleanupChildren 分类收尾）；15 个 mock-host 单元测试与 d19
  同钉形。宿主面余项登记：-ignorestderr stderr 丢弃、CHILDKILLED 无
  宿主 ABI 对应物。
- **C33**：`file attributes` 按 9.1 实现（unix 属性集 `-group/-owner/
  -permissions`、权限 `0o%05o`、errno 文案）；`file normalize` 去 ~ 展开
  （9.0 变更）+ `file tildeexpand`/`file home` 补齐；38 条探针逐字节一致。
- **S38**：HTTP 连接超时默认 30s、总超时 300s（显式化 reqwest 隐式 30s
  ——审计的"无超时"实为该无声值，>30s 的 SSE 流会被它切断）、响应体
  64MiB 上限、URL scheme 白名单 + 宿主校验钩子（`set_url_guard`）；
  env（`RTCL_HTTP_*`）与逐请求 opts 可覆盖；11/11 测试。
- **S21**：`lrepeat`/`string repeat` 分配前守卫按 9.1 钉形（TIP 626 64 位
  count、NEGARG、LIST_MAX 1152921504606846970、TCL MEMORY/ARITH IOVERFLOW）；
  超物化预算段按 9.1 错误形状可 catch 拒绝（tclsh 走惰性 rep 即刻成功，
  rtcl 急切物化的容量差已登记）。19 条探针逐字节一致。
- **objsec flake 根治**：墙钟断言改容差形（≤2s；三次独立会话实证 ±1s 抖动）。
- 门禁：judge 58/95（新语料基线持平）、crash-gate 128/128、sweep 12 既有、
  probes 327/118（objsec flake 态）→ 修复后 328/117 口径、workspace exit 0、
  rtcl-native(net) 11/11、wasm32 过、五特性 check 过。

## B3/B2 对齐与加固（task/b3-commands + task/b2-execfile，2026-11-26）

B3（timer/regexp/regsub/lsearch/process/混合包，部分子代理+主会话接手）：
- tcl::process ensemble（autopurge/list/purge/status，空态钉形）
- regsub -command（TIP 463 回调替换：前缀+匹配+捕获组按值求值）
- regexp 编译错误文案 couldn't→cannot（9.1 全族）
- lsearch 错误清单补 -stride、REG_BADRPT 措辞序 invalid quantifier operand
- binary f32 字段窄化走最短十进制字符串按 f32 RN 解析（位级 cast 在
  半点值上与 tclsh 分叉——binary-53.22/53.25 钉形）
- 事件 id 基 after#1→after#0（9.x 口径）；timer/timerate ensemble
- case 级：4157→4531/4849（93.4%），gen_lfilter/lpop/lseq/timer/
  dict/subst/array/const/regexp/lsearch/binary/process 十余文件转绿

B2（exec 宿主路径/file/HTTP/大小守卫）：见 BASELINE 2026-11-26 B2 节。

## N 数字字面量 9.1 语法批（task/digits91 5 提交，2026-11-26 合并，子代理施工）

- **12 个解析点统一到 9.1 文法**：共享扫描器 scan_number（bignum.rs，
  tclStrToD.c acceptPoint 镜像）——0o/0O、**新增 0d/0D**、0b、0x、裸
  前导 0=十进制（017=17、09=9、018 不再报错）、`_` 分隔符、±号形；
  **删除 is_invalid_octal**（9.1 无此错误面）。覆盖：parse_int/int_rep、
  parse_float_str、expr 词法与预解析（radix hint 0b→binary/0o→octal，
  oracle 钉形）、索引扫描、lsort、string is（类界重钉：integer 任意宽
  度含 bignum、wideinteger 64 位）、format、scan（**核实不改**——自有
  C 文法与 9.1 一致）、编译期折叠（**核实不改**——新形式正确 bail）。
- **关键归组更正**：gen_expr 族 95 失败无一由数字文法引起——26+29+12
  条是 expr 错误消息 9 口径簇（另一批）、12 条 mathfunc（已落地）、9 条
  TIP582 expr 注释（另一批）。真实翻绿在散布点：gen_cmdIL 混进制排序、
  gen_util 索引 0d 形、gen_obj 2→0。
- 探针 digits91.tcl（90+ 形，oracle 逐字节一致）+ 8 个 scratch 矩阵。
- rewrite 期间探针抓回三缺陷（hex 字位/前缀区间越界 panic/空尾数）。
- 门禁（合并后）：judge 76/95、probes **363/100**（DIFF 117→100：翻绿
  bn2-bn4/ec2/expr_lead1-2/incr_frame/ix3/s_probe/v_err 等 10 个，零新
  增）、crash-gate 128/128、sweep 12 既有、rtcl-vm 870/0、workspace
  1496/0、wasm32 过。

## B1 语义对齐批（task/b1-91align 13 提交，2026-11-26 合并，子代理施工）

- **C16**：expr 折叠/运行期双日志模型（折叠常量子树错误 = `invoked
  from within`、含运行期值 = `while executing`；布尔语境全常量才折叠）
  + 9.1 词形（cannot use non-numeric string "a" as left/right operand
  of "+"、cannot use a list、ARITH DOMAIN list、expected boolean →
  TCL VALUE NUMBER、~4.0 报错）。实现：operand_class_error 统一 +
  逐节点 runtime-events 折叠纯度 + 新 op `ExprFoldError`（载荷缩回池
  索引，OpCode 回 ≤16B）+ 条件语境 `EvalExprCond`。探针
  c16_expr_fold_frames 50 例双绿。连带修 3 处（词级错误出口泄漏
  cur_source 跨单元越界 panic、Neg 臂 `?` 绕过命令帧、mathop 折叠起
  点首非法操作数 left 归属）。
- **C28**：ns-eval 体不是 proc 帧——`frame.ns_depth` 判定，tailcall
  报 `tailcall can only be called from a proc, lambda or method` +
  TCL TAILCALL ILLEGAL。探针 5 例。
- **C38**：别名 wrong-args 命令名 9.1 统一为别名调用词；带前缀实参
  到名字为止、无前缀带参数表；只改写 proc 目标（内建保自我用词）。
  探针 13 例。
- **C39**：循环体 builtin 分派不再占 call_depth（tclsh 内联体不是求
  值层）；递归门 >→>=；**rtcl-cli 解释器挪 64MB 栈线程**（树走查每
  层 Rust 栈 8-17KB，8MB 主线程在 ~998 层循环体递归即栈溢出）。
  @limit 1000 完成 998 层 = 9.1 钉形。探针 5 例。wasm 150 校准与宿
  主接口不变。
- **C47**：回溯步数预算 2M→24M（覆盖 ~3300 字符回引扫描族，bail
  ~2s 护栏保留）。探针 10 例。
- **array unset** 不存在变量静默（9.1）；**trace 元素 unset name1**
  = unset 写下的全名（ta(k)/::tc(k)），整数组扫描保持裸名。
- 门禁（合并树）：judge **79/95**（gen_compExpr-old/gen_while-old/
  gen_while 转绿；18 fail ⊂ 基线 21 无新增）、crash-gate 128/128、
  probes **382/88**（净 fail -22：bn6/7/9/10/12/13/14、
  expr_bracket_top、numcmp1、p32/p33、verify 等转绿 + 1 known 翻绿）、
  sweep 12 既有、rtcl-vm 871/0、wasm build ✓（opt 2.32MB）。
- 遗留登记：-errorstack INNER 的 runtime 操作数形（TIP 356 深水面）、
  mathop 单操作数词形未钉、wasm crash-wasm 50/52（string repeat/
  lrepeat 2^32 TRAP 为 wave2 基线既有 S21 wasm 面，见 BASELINE B2 节
  与 TODO）。

### D2 合并解回归与修复（vm_exec 递归门，2026-11-27）

D2 合并（3545ffe）的 vm_exec 解取 D2 侧时回退了 B1 的 C39 递归门
`>=`（D2 分支基线 b6e674c 早于 c4e4029）——递归边界晚一帧：c39
probe VM=999 vs tree/oracle=998，sweep 13（12 既有 + 该探针）。已修
复（`>=` 恢复，probe 双引擎 998 与 oracle 一致，sweep 回 12）。教训：
跨分支合并解时，对含行为语义的小 hunk（门比较式）要逐个 diff 核对，
不能只看冲突区域——自动合并也可能因基线差丢 hunk。objquad.tcl 墙钟
断言同批容差化（与 objsec 同款）。

## E2/E3 语义对齐批（task/e2-ooinfo + task/e3-exprfmt，2026-11-27 合并，子代理施工）

### E2（oo/info/ns/uplevel/lseq，gen_oo 17→0、gen_info 3→0、gen_namespace-old 3→0、gen_basic 1→0、gen_uplevel 7→0）
- gen_oo 17：creationid/definitionnamespace/self/private 词/Slot 引导态×11
  （oo 单测对象编号 Obj11→Obj15 随引导态）；gen_info 3：cmdtype 的 ::
  前缀归一；gen_namespace-old 3：自删体帧窗口的复活语义；gen_basic 1：
  自删 ns 复活窗口；gen_uplevel 7：层级词解析对齐 TclObjGetFrame
  （upvar_levels 探针转绿）。lseq-4.11 记账（arithseries 惰性 rep 需
  Value 层 series 表示，归 Value 模型批）。
- **主树发现并登记**：rtcl OO bootstrap 少 9 个对象（根因 3b82bc1，
  首个用户 OO 对象 = ::oo::Obj20 的裁定 37907d0 + specs 登记 6bd2b19）。

### E3（format/expr/incr，gen_format 15→0、gen_expr 11→0、gen_expr-old 5→0、gen_parseExpr 1→0、gen_incr 2→0、gen_incr-old 1→0、gen_get 2→0）
- format：9.1 截断口径（无 size 修饰符 %b/%d/%o/%x/%X/%u 截 32 位、h
  16 位、l/lq/j/z/t 64 位、ll/L 全精度 bignum、%llu 负值
  TCL FORMAT BADUNSIGNED）；`#` 前缀 9 化（%#x 恒小写 0x、%#o→0o、
  %#b→0b、%#d→0d、零值无前缀、%p 恒 0x）；精度对整数生效且压 0 旗
  标；零填充位置（符号+前缀后）；%c 全 Unicode（E1 侧超集保留：
  bignum ARITH IOVERFLOW + [INT_MIN,UINT_MAX] 界）；`%*d` 星宽
  errorCode → TCL VALUE NUMBER。
- expr：**TIP582 注释**（`#` 到行尾词法等价空白，token 边界均可，双
  引擎同步）；一元 ± 语义修正（-"a" 报 non-numeric operand of "-"、
  一元 + 数值断言、±NaN 浮点类）；布尔不 trim（`! " true "` 报错）；
  mathfunc 报错分族（浮点族 expected floating-point number、min/max
  not enough arguments）；数组下标非法字符（`(" { }` → invalid
  character in array index，28 字节 display 对齐）。
- incr：列表样值报 expected integer but got a list（incr-2.32/2.33 帧
  保留）。
- **附**：E3 发现并修复 expr_compile.rs cfg(test) 自基线（8908749）
  起编译红（D2 宽操作数的测试模式字面）——非其引入，主树同红。

### 门禁（合并树终态，全部独立复验）
judge **94/95**（余 gen_lseq 1 = arithseries rep 记账项）、crash-gate
128/128、probes **398/76**（known 2/10：objsec/objquad 容差化后转绿）、
sweep 12 既有、workspace **cargo exit 0**（rtcl-vm 871/0）、wasm32 ✓、
五特性 ✓。案例链：语料 9.1 重提取基线 4157 → 各批累计 **4817/4849=99.3%**。

## D1 步骤 1——VarArena + 全局表迁移（task/d1-vars1 @ 499acc6，2026-11-27，子代理施工）

- **Arena 形状**：VarRef{table,slot,gen} 句柄 + generation-guarded 槽
  arena（删除置洞+代数+1，槽地址永不搬移，空洞复用靠代数防 ABA）+
  Fx 桶链键索引 + 插入序向量（删除留洞跳洞枚举→「每活键恰一次、末
  次创建位」）= tclsh 表序。Var{value,flags(ARRAY/UNDEFINED/LINKED),
  elem_count,traces,link} 16B；traces/link 占位恒 None（步骤 3/4 迁）。
  embedded cfg 退 BTreeMap（本模块零错误）。
- **三合一迁移**：globals 平表 + phantom_globals + array_globals（含
  全局 elem_counts）→ 每命名空间一张 VarTable。读写面全路由
  （vars/vm_exec/namespace/oo/misc/array/eval/control/proc/package/
  interp_cmd/introspect/list/dict）；var_slot_fast 两处 &mut 逃逸改
  闭包形（with_var_slot_res/link_res）。
- **对拍**：debug 双读逐读对拍（旧平表路径保留至步骤 5）+ release
  judge 对 oracle 逐字节。**抓到 1 个真漏点**：启动数组标记先于基键
  落表 → `info globals` 序错位——insert_moving（无值条目上值=新建序
  位）修复。
- 门禁（合并树）：judge **95/95 全绿**——**gen_lseq-4.11 顺势转绿**
  （arena 复用语义与 tclsh arithseries 面对齐，记账项清除）；crash-gate
  128/128；probes 398/76；sweep 12 既有；rtcl-vm **884/0**（对拍常开）；
  五特性/wasm32 ✓。
- 基准：var_incr/dict/list/string_build ±噪声内；元素微基准 create
  **-5%（反超）**/read +2.5%；array size O(1) 保持；ns 体反超方向保持。
- wasm 体积 +25.6KB/+0.83%（arena 机制+路径；步骤 5 删镜像结构预期
  回收）。行为序变化面（登记）：`array unset <pattern>` 逐元素 trace
  触发序、oo copy 复制序从 HashMap 迭代序换 arena 插入序——judge 语料
  证实两处旧序未被钉。

## D1 步骤 2——帧变量表迁入 VarArena（task/d1-vars2 @ e808a36，2026-11-28，子代理施工）

- **帧表形状**：VarTable 驻 CallFrame.vt（不驻 arena vec——协程排水把
  整帧 Box<CallFrame> 移入 record、恢复重推，表必须随帧走）；池化
  归还/尾调重绑/trace 清场三处清空收敛为 reset_var_state（整表清空+
  逐槽代数+1——旧句柄全灭，容量保留 O(槽数)）；表实例携带
  TableId::Frame(帧栈下标)（推栈点+coro resume 刷新，池化帧可换深度
  复用），句柄读写归属校验。
- **迁移面**：咽喉层 frame_local_insert/remove（单探测+有值域返回）、
  get/get_mut/contains/is_array/mark、resolve/store/remove/loc_* 五件、
  get/set 快径、incr 元素快径、upvar/global/append/array 元素扫描/
  GC 根快照。编译局部槽、upvars/drop_frame_global_links/dead 链不动
  （步骤 3/4 面）。帧域无 UNDEFINED phantom（链接条目只入日志——
  步骤 3/4 面）。
- **对拍抓 2 个真漏点**：①frame_local_remove 对 flag-only 占位返回
  Some（改有值域）；②占位不入日志时 array_elements 快径守卫可误列
  （占位入日志恢复「日志 ⊇ vt 条目」不变式）。T3 直列输出与步骤 1
  基线逐字相同（一处 arr 位置分歧为预存，两版 rtcl 一致）。
- 门禁（合并树）：judge **95/95**、crash-gate 128/128、probes 398/76、
  sweep 12 既有、rtcl-vm **886/0**（debug 对拍常开 +2 帧表测试）、
  workspace exit 0、wasm32 ✓、四特性 check ✓。
- 基准：var_incr/dict/proc_fib/list/string_build 持平；帧面放大基准
  199→198ms（池化重置成本可控）；T3 info locals 持平、after/plain 反
  超方向保持；t7 数组 ±4% 内；ns 体反超方向保持；RSS 持平。
- wasm 体积 **−395B**（帧表机制+镜像维护退场相抵）。
- **遗留登记**：debug 判官的预存对拍缺口——全局域 arena UNDEFINED 位
  与 phantom_globals 镜像在 unset 走 global_remove 的重叠态分歧（步骤
  1 遗留，基线 36269a7 逐字复现，常设门禁不覆盖）——随步骤 3/4
  （链接/phantom 面迁移）收口。

## D1 步骤 3——数组元素表迁入 VarArena（task/d1-vars3 @ eaffbb1，2026-11-28，子代理施工）

- **子表形状**：元素表 = 独立 VarTable 驻宿主 Var.elements: Option<Box<VarTable>>，
  身份 TableId::Array(Box<宿主VarRef>)；ARRAY 旗标收编进 Var.flags
  （sync_elements 全写槽路径维护 is_array⟺elements.is_some 不变式）；
  生命周期随宿主（数组 unset 子表整体回收、帧池 reset 清空、再建新句柄）。
  elem_count 两域统一挂宿主 Var。
- **路由收口**：咽喉元件感知（global_*/frame_local_* 的元素形键臂）——
  T7 五面快径自动换道；land_element（覆盖写零分配/新键单分配）+
  loc_elem_* 直访（免平键构建/再拆）；array 命令全族枚举走子表插入序；
  搜索戳键换宿主 VarRef（ns delete 反查+死柄弃置）；启动 seeding 重写
  （env/tcl_platform 容器先建）；oo copy/collect 根快照扫子表。trace 三
  表仍字符串键（步骤 4 面）。
- **对拍漏点 0**（路由一次到位）；顺序语义：unset 重建换位、ns/帧/
  upvar 面 10 项压力脚本对 oracle 逐字节同；四探针（t7_arr_elem 等）
  对 oracle 逐字节同。
- 门禁（合并树）：judge **95/95**、crash-gate 128/128、probes 398/76、
  sweep 12 既有、rtcl-vm **889/0**、workspace exit 0、wasm32 ✓、五特性 ✓。
- 基准（ABBA 12×4 轮中位）：t7 数组全案 ±5% 内（size-80k O(1) 保持
  0.26ms vs tclsh 90ms）；micro-read **-12.2%**、micro-incr -4.4%；帧/
  全局面 string 三案 -3.9~-42.9%、info_locals -3.9%、ns_body -6.2%；
  **RSS 50k/80k 元素负载 -17%/-22%**（12812/39360 vs 15500/50520 KB）。
- wasm 体积 +5269B/+0.27%（size 轨）——子表机制成本，步骤 5 预期回收。
- 预存登记（未盲修）：`upvar 0 a(e) x; set x 5; info exists a` =0 vs
  oracle=1（帧域 flag-only 标记无基键值——baseexists 面归步骤 4）；`variable a(e)` 不报错（元素变量面批）；array statistics history 压缩
  时机换子表触发形（BASELINE 登记）；步骤 1 的 phantom 对拍缺口随
  步骤 4 收口。

## D1 步骤 4——trace 挂 Var + phantom/链接面收口（task/d1-vars4，2026-11-28，子代理施工）

- **TraceSet 形状**：`Var.traces: Option<Box<TraceSet>>`（`Var` 24B→32B——
  一个指针宽的增量，只有被追踪的 Var 付堆侧成本）。`TraceSet{var: 整变
  量族, var_linger: 整变量键在册位, elem: 元素族}`：整变量族挂名字宿主
  Var（无值占位宿主就地确保——bare-:: 与旧 note_global_entry 同形
  UNDEFINED；限定名/ns 体裸名同形扩展，legacy `trace variable` 为 plain
  占位——oracle 实测注册即建 Var、ns delete 不发 unset 火钉形）；元素族
  挂**数组基宿主** Var（生命周期语义：元素摘除后 trace 仍在、phantom
  元素同域——旧 elem_traces 逐字同）。`var_linger` = 旧 var_traces 字符
  串键的 lingering 语义（摘净后 upvar-8.7 仍拦——probe8c 基线钉形；元素
  族不置位——upvar_levels u24 oracle 钉形）。
- **触发序**：在席火（resolve_loc→宿主收集）+ 快照火（unset 四臂/ns
  delete/帧清场先 take_trace_set 再触发——oracle 实测记录在自身 unset
  触发期间对 trace info 不可见，基线字符串表的火中可见为预存分歧，随迁
  移收敛）。守卫键（VAR_TRACE_ACTIVE）保持 stamp 串形；帧清场序 = vt
  插入序（旧 HashMap 随机序——未钉）。触发序红线（T2T1/A1E1/D-36
  重读/D-37③⑤/name1 全名形）逐字节复核 + 10 个 trace 族探针 oracle
  逐字节同。
- **快径守卫**：三表空判换 `n_trace_hosts`（持有 trace 集的宿主 Var
  数——与旧「键 linger」守卫语义逐字同：注册建集 +1、摘记录不复位、
  宿主消亡 −1）。elem 族宿主计入守卫——修掉基线 append 元素快径的
  elem trace 盲区（oracle 发、基线不发；未钉路径，收敛）。
- **对拍漏点**：步骤 1-2 登记的 phantom 重叠态缺口**收口**（global_remove
  整条摘除时镜像 phantom 位同步——debug 判官 gen_upvar/gen_trace/
  gen_set-old panic 形消除，最小形 `proc p {} { global z; set z 5 }; unset z`
  实证）。**对拍零新漏点**；语义新发现 4 项（基线预存分歧，未钉路径）：
  ① unset 火中 trace info 可见性（基线可见/oracle 不可见——随快照火收敛
  oracle）；② 元素 trace 随元素 unset 存活（基线存活/oracle 摘除——基线
  持平保留）；③ 整数组 unset 的 phantom 元素火（oracle 发/基线不发——
  保留）；④ 整数组 unset 的整变量先于元素序（oracle 整组先/基线元素先
  ——保留）。
- **链接面**：drop_frame_global_links 改 trace 感知（目标仍承载 trace
  时只撤 UNDEFINED 位、条目随 trace 留存——值less 可见性与 trace 存活
  分刻）；upvar-8.7 检查换 var_linger 位。`dead_flat`/`flat_aliases`/
  `ns_transient`/upvars 本体**保留**（步骤 5 面）；步骤 3 登记的帧域
  `upvar 0 a(e) x; set x 5; info exists a` baseexists 分歧**未修**
  （oracle=1/rtcl=0——链接元素写不落基键值，归链接面独立批）。
- 门禁（合并树，最终源）：judge **95/95**；crash-gate **128/128**；
  probes **398/76**（DIFF 名单与基线逐项相同——零新增）；sweep **12 既有**
  无新增；workspace exit 0；rtcl-vm lib **889/0**（std+nostd 双特性）；
  五特性 check ✓；wasm32 ✓；**debug 判官全语料 95/95**（对拍断言常开，
  缺口消除实证）。
- 基准（A/B 对 /tmp 双二进制，perf cycles 交错取中位 + 自计时 ABBA，
  负载 2-3 窗口）：var_incr −3.5%、dict_ops +2.7%、proc_fib −6%；t7
  size-80k 1.000/exists-80k 1.002/read-50k +2.5%/write-new −8.7%/
  write-old −2.5%；帧面 info_locals list −10%/after −6.5%/plain −1%
  （`frame_slot_incr` 补 `#[inline]`——大 diff 触发的内联失守实测
  +9%、补 hint 后回位，符号化构建对拍 0.99-1.005）；trace 路径
  trace_bench（新增 bench/scripts/trace_bench.tcl）：reg-rm −8.8%/
  fire-w +5.7%/fire-elem +1.1%/notrace-set +4.2%。
- wasm 体积 **−3001B/−0.09%**（3168702→3165701，release 轨）——三张
  字符串键表退场 > TraceSet 机制成本。
