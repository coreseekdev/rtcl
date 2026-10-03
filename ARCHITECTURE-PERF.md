# rtcl architecture review — where the remaining order-of-magnitude lives

Consolidated view after the 2026-10-02..04 architecture series (parse cache,
bytecode VM for proc bodies and eval units, inline compilation of loop/if
bodies, `[...]` words and expr operands, lazy int values, in-place incr,
frame pool).  Bench history: `bench/BASELINE.md`; correctness gates: judge
87/87 vs live tclsh 8.6.17 + full two-engine probe sweep (bytecode ==
tree-walk byte-exact, modulo recorded divergence classes).

## Where we are (2026-10-04, taskset -c 0, best-of)

bench case      tclsh   rtcl    ratio      note
arith_loop         34     31      .9       compiled while + IncrVar
dict_ops            5      6     1.2       DynCall dispatch
list_ops            5      7     1.4       DynCall dispatch
proc_fib            4     13     3.2       fib20; fib25 hand-probe 134ms vs 31ms (4.3x)
string_build        3      3     1.0
var_incr           39     29      .7

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

### D. Inline foreach/lmap/switch/catch bodies  ←  next
foreach currently DynCalls cmd_foreach per iteration set with per-body
eval_body_value.  Compiling the body inline (LoopEnter per list
element) attacks the 1.2-1.4× data-op gap.  Medium risk: foreach's
var-list shapes (multi-var, {a b} pairs, `continue`/`break` semantics)
and lmap's result accumulation.

### C. dispatch_values probe-storm slimming
Each DynCall walks namespace-resolution chains with successive map
probes.  A CmdId-indexed builtin table + procs fast map already exists
partially (dispatch_call guards); extend to the resolution order so the
common case (plain global proc or builtin) is ≤2 probes.  Small (few %)
but cheap.

### F. ControlFlow rework
`Error::ControlFlow { code, level, ... }` is constructed/decoded on
every return/break/continue crossing; return-with-value allocates.  A
dedicated completion enum carried in VmState would remove per-iteration
costs in loop-heavy code that mixes dispatch and inline constructs.

### JIT (HANDOFF §5, M0-M4) — after the interpreter plateaus
Tcl → wasm per-proc modules, interpreter stays as fallback.  The slot-
locals work (E2) is a prerequisite in spirit: the JIT's frame layout is
exactly the dense-slots repr, and the locals-table builder becomes the
JIT's front-end analysis.

## Decision

E1 → E1.75 → E1.9 → E2 landed (proc_fib 3.0→1.8×, decomp_var 2.7→1.8×).
Next: D (foreach/lmap inline), then C, re-baseline, then decide JIT M0.
Every step gated on: build
(ulimit 3.4G) → judge 87/87 → two-engine probe sweep → workspace tests
→ bench delta recorded in BASELINE.md.
