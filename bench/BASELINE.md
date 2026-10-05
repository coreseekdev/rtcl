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
