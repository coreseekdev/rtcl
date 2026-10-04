# rtcl architecture review — where the remaining order-of-magnitude lives

Consolidated view after the 2026-10-02..04 architecture series (parse cache,
bytecode VM for proc bodies and eval units, inline compilation of loop/if
bodies, `[...]` words and expr operands, lazy int values, in-place incr,
frame pool).  Bench history: `bench/BASELINE.md`; correctness gates: judge
87/87 vs live tclsh 8.6.17 + full two-engine probe sweep (bytecode ==
tree-walk byte-exact, modulo recorded divergence classes).

## Where we are (2026-10-04 post-F, taskset -c 0, best-of, noisy machine)

bench case      tclsh   rtcl    ratio      note
arith_loop         40     36      .9       compiled while + IncrVar
dict_ops            8      9     1.1       DynCall dispatch
list_ops           10     11     1.1       DynCall dispatch
proc_fib            8     16     2.0       fib20; fib25 interleaved ~80ms vs tclsh 25-39
string_build        5      6     1.2
var_incr           42     38      .9

Decomposition probes (startup-corrected, per-call):

probe (N calls)                rtcl        tclsh       ratio
trivial call (200k)            0.56 µs     0.35 µs     1.6x    call machinery
3 args + 6 local ops (100k)    1.47 µs     0.48 µs     —
  → binding+vars slice         0.92 µs     0.13 µs     ~7x     name-keyed locals

So the proc-call gap is NOT the call machinery (1.6×) — it is
**argument binding and variable access by name** (HashMap<String, Value>
per frame; `eval_var_ref` → locals map per LoadVar/StoreVar; param
binding one map insert per arg).  tclsh compiles proc locals to slots:
`LoadLocal slot` / `StoreLocal slot` in a per-frame `Vec<Value>`.  rtcl's
ISA already has these opcodes — the compiler never emits them.

## Current execution architecture (what exists)

Tiers per command site, decided at compile time:
1. **Tier-1 native ops** — set/if/while/for/expr/incr/break/continue/
   return/exit compiled to opcodes; bodies and conditions inline in the
   unit (no re-parse, no dispatch per iteration); `[...]` words and expr
   operands inline via SubMark/ExprMark regions.
2. **Tier-2 Call {cmd_id}** — known builtin id, still through
   `dispatch_call` (guards: traces off, global ns, no procs/aliases/
   ensembles → direct fn ptr).
3. **Tier-3 DynCall** — everything else, full `dispatch_values`.

Caches: parse cache (eval seam), expr-check memo, bytecode cache keyed
by script text, body memo (Rc body → Rc<ByteCode>), frame pool (32).
Values: lazy int rendering, in-place IncrVar single lookup.

Error framing is the cross-cutting invariant: every compiled construct
reproduces the tree-walk's errorInfo protocol (sites, regions,
err_fresh suppression, err_pending_top deferral) — verified by the
two-engine differential sweep over `judge/probes/`.

## Ranked remaining items (by measured leverage / risk)

### E. Slot-resolved proc locals  ←  DONE (E1 28f8484, E1.75 a26b580, E1.9 549a716, E2 2026-10-04)
Targets the measured 7× binding+vars slice and part of the 1.6× call
machinery.  Expected ≥2× on proc_fib (→ ~1.5-2× vs tclsh).
Design (staged):
- **E1 (low risk, no semantics)**: VmState pooling — exec_bytecode
  allocates 3 Vecs per call (stack/loops/bodies); take/return from an
  Interp free-list.  Plus: positional arg-bind fast path in call_proc
  when the ProcDef's compiled body exists and the signature has no
  defaults/`args` (bind straight into the frame, skip the general loop).
- **E2 (the real change)**: compiler builds a per-proc locals table
  (params + body-discovered plain-name `set`/`incr` targets);
  LoadLocal/StoreLocal/IncrLocal for table names (slot operand, name
  recoverable from `ByteCode::locals()` for the fallback), expr `$var`
  operands routed through `ExprSink::var_read`.  Frame storage = slots
  as the canonical store + every name-keyed var path consulting
  `slot_index_of` first (tclsh's compiledLocals/var-table aliasing);
  degrade slots→map (in slot order = tree-walk insertion order) on
  upvar/global/variable link install, trace registration,
  array-ification, and whole-frame for `info locals`/`info vars`/
  `info frame`.  Runtime gate (per tailcall iteration): compiled +
  epoch-valid + applicable + no statics + all params slot candidates +
  non-empty table; failures keep the name-keyed frame, slot ops fall
  back through name paths.  Landed: decomp_var 144→96ms, fib25 104→87,
  proc_fib 3.0→1.8× (BASELINE.md "E2" section for the full table).
Risk: high (var traces, upvar, info locals, tailcall); gate every step
with judge + two-engine sweep.

### D. Inline foreach/lmap + proc-context catch/apply bodies  ←  DONE (2026-10-04)
foreach DynCall'd cmd_foreach per iteration set with per-body
eval_body_value; proc-foreach bench was 28× tclsh.  Landed in three
coherent pieces (all gated judge 87/87 + two-engine sweep clean):

- **D-bytecode**: `ForeachStart/Next/Collect/End` ops with a
  `ForeachInfo` shape table (groups of `VarTarget{slot, name_idx}`) —
  strict-parsed data lists, slot-or-name binding per iteration,
  compiled break/continue through the ordinary loop machinery, lmap
  collects.  Fold gate mirrors tclsh: locals-mode unit + literal
  varlists + braces-only verbatim body (`compiler.rs`
  `"foreach"/"lmap" if self.locals_mode`).
- **D-tree (lexical twin)**: `interp.lexical_body` /
  `next_eval_lexical` — cmd_foreach/cmd_lmap/cmd_while/cmd_for/if-arms
  run a value-level twin (`foreach_lexical`) gated by value-level
  mirrors of the compiler gates (`body_would_compile`,
  `word_is_plain_literal`, `foreach_inline_shape`), so the tree engine
  (RTCL_NO_BYTECODE) produces byte-identical output to the bytecode
  engine.  Arming sites mirror the compiler's inline decisions exactly
  (eval_word_ctx whole-word brackets, expr operand brackets, loop/if
  bodies, call.rs tree path).
- **D-context (catch/apply)**: tclsh compiles apply lambda bodies like
  proc bodies and inlines braced catch bodies into proc-context units;
  everything else (top level, eval'd strings) stays dispatched with the
  `(setting foreach loop variable …)` decorated shapes.  rtcl mirrors:
  `ByteCode::locals_mode` → `Interp::in_locals_unit` (set by
  exec_bytecode for locals units, cleared by `eval` at fresh-unit
  boundaries), `cmd_apply` compiles lambda bodies (memoised by term
  string in `lambda_code_cache`), `cmd_catch` runs braces-only bodies
  through `eval_lexical_script` when `(lexical_body ||
  in_locals_unit)`.  This closed judge gen_lmap lmap-4.15 (tclsh's
  compiled-context var-write shape) while keeping foreach-1.14's
  top-level decoration.

Residual (recorded): OO method bodies still `compiled: None` (tclsh
compiles them; uncovered by the corpus — engines agree on the
dispatched shape).  Catch bodies run tree-walked (lexical) rather than
compiler-inlined — perf follow-up is true catch inlining at the
compiler level (CatchStart/CatchEnd reserved).  proc-foreach bench:
57ms → 14-19ms vs tclsh 5.2ms (gap 28× → ~3×).

### C. dispatch_values probe-storm slimming  ←  DONE (2026-10-04, c849b39)
Root cause (instrumented, not guessed): stdlib's `namespace ensemble
create` in `namespace eval ::tcl::tm` keeps `ensembles` non-empty in
every real session, so `dispatch_call`'s fast path NEVER fired — every
compiled Call/DynCall walked the full chain: ~7 procs probes +
find_ensemble + the commands chain, with two heap `format!("::{}", name)`
keys per dispatched command (~1M dispatches in the dict bench; profile:
fmt ~6%, VarHasher 5.3%, memcmp 5%).  Fix: tclsh-style resolved-command
tokens — `cmd_cache[(namespace, name)] -> CachedCmd{gen, target}`,
generation bumped at every commands/procs/ensembles/import_aliases
mutation (35+ sites).  Builtins cache the fn ptr (one generation-checked
probe replaces both the guard storm and the CmdId memcmp chain); procs
cache the resolved KEY and re-fetch the def per call (statics write-back
replaces the map entry via `Rc::make_mut` — caching the def goes stale;
the statics unit tests caught it).  Ensemble/unknown/unresolved stay
uncached.  dloop 598→437ms vs tclsh 249 (2.2×→1.76×); gates green
(judge 87/87, sweep 8 known, tests 1143).  Full notes in BASELINE.md "C".

### F. Call-path rework  ←  DONE (2026-10-04, F1 bb7de53 + F2)
Re-scoped from measurement, not the roadmap's original text (the
ControlFlow-allocates assumption was wrong: `Error::ret` moves the Value
inline; compiled loops already jump directly).  The measured costs were
per-call dead work in call_proc:
- **F1 — lazy execution-trace context**: call_proc's first act was
  resolve_command_key (7-arm probe chain + format! walk) + 2 String
  allocs + an exec_step_stack push, whose only reader gates on
  `exec_traces.is_empty()` — dead in every untraced session.  Gated
  behind `traced_entry` (tailcall rebind + exit pop/fire carry the same
  guard).  tclsh-probed: traces registered on an already-running proc do
  NOT instrument that invocation — rtcl's eager context DID (two
  divergences fixed; probes now byte-match).  dloop ~20%, fib25 ~4%.
- **F2 — namespaces as `Rc<str>`**: `current_namespace`/`frame.ns`/
  `frame.call_ns` were Strings cloned 3-5× per call (all "::" for global
  procs) and `ns_of_qualified`'s rfind was the profile's
  ReverseSearcher.  Now Rc bumps + a cached `ns_root`; fresh names move
  in via `Rc::from(String)` (reuses the allocation).  A global-scope
  proc call allocates zero namespace strings.  fib25 another ~10-15%
  (89.7-99.0 → 80.4-83.1ms interleaved vs F1); dloop/fe_bench neutral.
Full notes in BASELINE.md "F1"/"F2".

### JIT (HANDOFF §5, M0-M4) — after the interpreter plateaus
Tcl → wasm per-proc modules, interpreter stays as fallback.  The slot-
locals work (E2) is a prerequisite in spirit: the JIT's frame layout is
exactly the dense-slots repr, and the locals-table builder becomes the
JIT's front-end analysis.

## Decision

E1 → E1.75 → E1.9 → E2 landed (proc_fib 3.0→1.8×, decomp_var 2.7→1.8×).
D landed (foreach/lmap inline + lexical twin + catch/apply proc-context;
proc-foreach 28× → ~3×).
C landed (resolution cache; dloop 2.2×→1.76×).
F landed (F1 lazy trace context — dloop ~20% and two trace divergences
fixed; F2 Rc<str> namespaces — fib25 ~10-15% more; fib25 journey
post-E2 87 → ~80ms best under load, vs tclsh 25-39 same conditions).
Next: re-baseline, then decide JIT M0.  Remaining
follow-ups from D: compiler-level catch-body inlining; OO method-body
compilation; lambda-code cache on the Value rep.  New follow-up from C's
testing: `rename` of a name a proc shadows removes the builtin and leaves
the proc (tclsh removes the resolved command — pre-existing, recorded).
Also recorded: numeric fast path (Value::as_int through bignum machinery
~6% on fib — the next measured target if more interpreter headroom is
wanted before JIT).
Every step gated on: build
(ulimit 3.4G) → judge 87/87 → two-engine probe sweep → workspace tests
→ bench delta recorded in BASELINE.md.
