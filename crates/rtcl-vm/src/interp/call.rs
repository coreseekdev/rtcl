//! Procedure call and tail-call optimisation for [`Interp`].

use std::borrow::Cow;
use super::PrelinkEntry;

use super::{CallFrame, Interp, ProcDef, UpvarMap, VarMap, VarSet};
use super::util::rfind_ns_sep;
use crate::error::{Error, Result};
use crate::value::Value;

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeSet as HashSet;

use super::Rc;

impl Interp {
    /// Call a user-defined procedure.
    ///
    /// `ns_override` pins the frame's namespace for `apply {p b ::ns}`
    /// (the lambda runs in `::ns` regardless of its synthetic name).
    pub(crate) fn call_proc(&mut self, proc_def: &ProcDef, args: &[Value], proc_name: &str, ns_override: Option<Rc<str>>) -> Result<Value> {
        if self.call_depth > self.max_call_depth {
            return Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            ));
        }

        // The def's params/body/compiled are read through a reference for
        // the whole call; only a tail-call rebind needs an owned def (the
        // loop's `tc_def`) — the three per-call Rc clones are gone.
        let mut tc_def: Option<Rc<ProcDef>> = None;
        // The invocation's words: borrowed from the caller's slice for the
        // whole call (zero-copy binding — the common case never rebinds),
        // materialised into an owned vector only by a tail-call target,
        // which then serves every later iteration.
        let mut owned_args: Option<Vec<Value>> = None;
        // Static variables ride the def's shared map: the common (empty)
        // case is one refcount bump, not a map clone per call; write-backs
        // COW via `Rc::make_mut`.
        let mut current_statics: Rc<HashMap<String, Value>> = Rc::clone(&proc_def.statics);
        // Borrowed until a tail-call rebind owns its target's name: plain
        // global proc calls pay no per-call String for the frame name.
        let mut current_proc_name = Cow::Borrowed(proc_name);
        let mut ns_override = ns_override;

        // Execution traces (tclsh 8.6.17): `enter` fires before the frame
        // exists and its callback's error REPLACES this invocation
        // (trace-40.1); `leave` fires after the frame is gone.  The
        // resolved key doubles as this frame's enterstep context (the
        // body's commands resolve their enterstep traces through it).
        // The context is only READ while execution traces exist
        // (`exec_step_begin`'s first check), so the untraced path — every
        // proc call in a normal session — skips the resolve (a multi-probe
        // + format! walk) and the push/pop entirely.  tclsh agrees: traces
        // (enterstep, leave) registered on a proc that is ALREADY running
        // do not instrument that invocation (probed 8.6.17: no enterstep,
        // no leave output; rtcl's former eager context fired them — a
        // divergence this fixes).
        let traced_entry = !self.exec_traces.is_empty();
        let mut leave_ctx: Option<(String, String)> = None;
        if traced_entry {
            let exec_key = super::commands::proc::resolve_command_key(self, proc_name)
                .unwrap_or_else(|| proc_name.to_string());
            let exec_cmdtext = Value::from_list(args).as_str().to_string();
            self.exec_fire_enter(&exec_key, &exec_cmdtext)?;
            self.exec_step_stack.push(exec_key.clone());
            leave_ctx = Some((exec_key, exec_cmdtext));
        }

        self.call_depth += 1;

        // A proc executes in the namespace where it was defined (the
        // qualifiers of its registered name), not the caller's namespace.
        let prev_namespace = self.current_namespace.clone();
        // Apply's one-shot `info level 0` word (consumed even when the
        // arity check fails, so it can't leak into a later call).
        // Same guard as the tailcall marker: `take()` on the (24-byte)
        // Option is a full replace; `apply` frames are the only ones that
        // ever arm it.
        let level0_override = if self.frame_level0_args.is_some() {
            self.frame_level0_args.take()
        } else {
            None
        };
        // The OO dispatcher's one-shot variable links (consumed here for
        // the same leak reason; installed at frame setup below).
        let prelink = self.frame_prelink.take();
        let mut prelink_done = prelink.is_none();

        // Push a new call frame (a recycled one when the pool has one —
        // the maps keep their allocation and capacity)
        let mut frame = match self.frame_pool.pop() {
            Some(mut f) => {
                f.locals.clear();
                f.array_locals.clear();
                f.upvars.clear();
                f.local_procs.clear();
                f.deferred_scripts.clear();
                f.slots.clear();
                f.slot_aliased.clear();
                f.link_keys.clear();
                f.has_aliased = false;
                f.slot_table = None;
                f
            }
            None => Box::new(CallFrame {
                locals: VarMap::default(),
                array_locals: VarSet::default(),
                upvars: UpvarMap::default(),
                ns: None,
                call_ns: None,
                local_procs: Vec::new(),
                deferred_scripts: Vec::new(),
                // Small head-room: a fresh frame is created per call once
                // the pool is exhausted (deep recursion), and the slot
                // fill would otherwise realloc on its first resize.
                slots: Vec::with_capacity(8),
                slot_aliased: Vec::new(),
                has_aliased: false,
                link_keys: Vec::new(),
                slot_table: None,
                tailcall: None,
                level0: Vec::new(),
                ns_depth: 0,
            }),
        };
        // `frame.ns`/`frame.call_ns` are *retained* across the pool
        // round-trip (not reset here, not cleared at the epilogue): the
        // assignments below are identity-checked, so a hot proc in one
        // namespace pays neither a drop nor a clone per call.  The frame
        // is only read after the definition namespace is (re)established.
        if !matches!(&frame.call_ns, Some(n) if Rc::ptr_eq(n, &prev_namespace)) {
            frame.call_ns = Some(Rc::clone(&prev_namespace));
        }
        frame.tailcall = None;
        frame.level0.clear();
        frame.ns_depth = self.ns_level0.len();
        self.frames.push(frame);

        let final_result = loop {
            // The def this iteration runs: the called one, or a tail-call
            // target's (owned by `tc_def`).
            let def: &ProcDef = tc_def.as_deref().unwrap_or(proc_def);
            // ── E2 slot-locals gate (re-checked every iteration: a tailcall
            // rebind swaps the compiled body, and the shape must follow) ──
            // The conditions are the execution gate below plus the slot
            // shape: no statics (they inject through the name-keyed store)
            // and the table's params-first seeding aligning positionally —
            // the first `params.len()` table entries must BE the params in
            // order.  Duplicate parameter names dedup in `add_local` and
            // non-candidate names (qualified/array) are skipped by the
            // seeding, both shrinking the table out of alignment (tclsh
            // rejects duplicate proc params at definition; rtcl accepts
            // them with map semantics).  When the gate fails the frame
            // stays name-keyed and the body's slot ops fall back to their
            // name paths (which is also what a mid-run degrade leaves
            // behind).
            //
            // LOAD-BEARING for the OO prelink slots: the `slot_table`
            // installed here is the SAME compiled unit the OO memo
            // precomputed its `PrelinkEntry::slot` indices against
            // (oo.rs builds both from one `memoised_proc_def`), so
            // `degrade_frame_link_at` may trust the index with only a
            // debug_assert on the name.  Changing what this gate accepts
            // changes that invariant.
            let slot_mode = match &def.compiled {
                Some(code)
                    if !code.fallback
                        && code.epoch == self.tier1_epoch
                        && super::vm_exec::bytecode_applicable(self)
                        && current_statics.is_empty()
                        && !code.locals().is_empty()
                        && code.params_aligned == Some(def.params.len()) =>
                {
                    let table = Rc::clone(code);
                    let n = code.locals().len();
                    let frame = self.frames.last_mut().unwrap();
                    frame.slots.clear();
                    frame.slots.resize(n, Value::unset());
                    // The alias/`Global`-link side vectors build lazily:
                    // `degrade_frame_link` sizes them on first use, every
                    // reader probes with `.get(slot)` — no per-call resize
                    // of either.
                    frame.slot_aliased.clear();
                    frame.has_aliased = false;
                    frame.link_keys.clear();
                    frame.slot_table = Some(table);
                    true
                }
                _ => {
                    let frame = self.frames.last_mut().unwrap();
                    frame.slots.clear();
                    frame.slot_aliased.clear();
                    frame.has_aliased = false;
                    frame.link_keys.clear();
                    frame.slot_table = None;
                    false
                }
            };

            // ── Enter the definition namespace (also on tail-call switch) ──
            // The override (apply's ::ns element) applies to the first frame
            // only; tail-call targets recompute from their own names.
            {
                let def_ns = ns_override
                    .take()
                    .unwrap_or_else(|| ns_of_qualified(self, &current_proc_name));
                // Identity-checked writes: in a hot proc the namespace is
                // the same allocation across calls (`Rc<str>` drops were
                // 8% of `call_proc` self), so both the interpreter's
                // current namespace and the frame's skip the churn.
                if !Rc::ptr_eq(&self.current_namespace, &def_ns) {
                    self.current_namespace = Rc::clone(&def_ns);
                }
                let frame = self.frames.last_mut().unwrap();
                if !matches!(&frame.ns, Some(n) if Rc::ptr_eq(n, &def_ns)) {
                    frame.ns = Some(def_ns);
                }
                frame.locals.clear();
                frame.array_locals.clear();
                frame.upvars.clear();
                // `info level 0` shows the invocation as dispatched: the
                // as-typed command word plus the evaluated arguments
                // (tclsh 47.1: `ns a b c` → `::ns::a b c`).  `apply`
                // overrides with `apply {<term>} <args...>`.  The words
                // are stored raw and rendered only if asked.  The buffer
                // is the frame's own (pool-carried capacity): each
                // iteration rewrites it — the override wins, otherwise
                // the binding step fills this iteration's words.
                frame.level0.clear();
                if let Some(l0) = &level0_override {
                    frame.level0.clone_from(l0);
                }
            }

            let has_args = def.params.last().map(|(p, _)| p.as_str()) == Some("args");

            let regular_params = if has_args {
                &def.params[..def.params.len() - 1]
            } else {
                &def.params[..]
            };

            // This iteration's invocation words: the borrowed caller
            // slice, or the tail-call target's owned vector.
            let cur: &[Value] = owned_args.as_deref().unwrap_or(args);

            // ── Arity check (Tcl: wrong # args) ────────────────────
            // Tcl binds positionally: any parameter left without an
            // argument must have a default (or be the trailing `args`).
            {
                let num_args = cur.len().saturating_sub(1);
                let mut arity_ok = has_args || num_args <= regular_params.len();
                if arity_ok {
                    for (i, (_, d)) in regular_params.iter().enumerate() {
                        if i >= num_args && d.is_none() {
                            arity_ok = false;
                            break;
                        }
                    }
                }
                if !arity_ok {
                    // Tcl list-quotes the command name ("{}", "{a b  c}")
                    // but appends parameter descriptors verbatim ("?arg ...?").
                    let name_word = if current_proc_name == "apply lambdaExpr" {
                        // Synthetic apply frame: Tcl renders this prefix literally.
                        current_proc_name.to_string()
                    } else {
                        crate::value::tcl_quote(&current_proc_name)
                    };
                    let mut usage = name_word;
                    for (i, (p, d)) in def.params.iter().enumerate() {
                        usage.push(' ');
                        if has_args && i == def.params.len() - 1 {
                            usage.push_str("?arg ...?");
                        } else if d.is_some() {
                            usage.push('?');
                            usage.push_str(p);
                            usage.push('?');
                        } else {
                            usage.push_str(p);
                        }
                    }
                    // tclsh stamps errorCode `TCL WRONGARGS` on proc
                    // arity errors, like builtin wrong-#-args errors.
                    super::commands::list::set_error_code(self, "TCL WRONGARGS");
                    break Err(Error::Msg(format!(
                        "wrong # args: should be \"{}\"",
                        usage
                    )));
                }
            }

            // Positional binding: hoist the frame borrow — a slot-compiled
            // frame binds argument i into slot i (the table's params-first
            // seeding is the positional contract), the name-keyed frame
            // inserts into the map.
            {
                let frame = self.frames.last_mut().unwrap();
                for (i, (param, default)) in regular_params.iter().enumerate() {
                    let value = if i + 1 < cur.len() {
                        cur[i + 1].clone()
                    } else if let Some(d) = default {
                        Value::from_str(d)
                    } else {
                        Value::empty()
                    };
                    if slot_mode {
                        frame.slots[i] = value;
                    } else {
                        frame.locals.insert(param.clone(), value);
                    }
                }
            }

            if has_args {
                let remaining_start = regular_params.len() + 1;
                // $args 变量必须持有一个**规范** Tcl list（tclsh 同源）：
                // 元素按 TclScanElement/TclConvertElement 规则加引号。
                // 此前的手写 join 只对含空白的词加 {}，首字符为引号或花
                // 括号的词（如行级 JSON `{"name":"x"}`）会被去引号落地，
                // 之后任何 `lassign/lindex` 式重解析都会碎裂——tclsh 的
                // `$args` 可无限往返。这里复用 Value::from_list 的序列化
                // （与 `[list]` 输出同一实现），保证 round-trip 性质。
                let remaining: Vec<Value> = if remaining_start < cur.len() {
                    cur[remaining_start..].to_vec()
                } else {
                    Vec::new()
                };
                let list_str = Value::from_list(&remaining).as_str().to_string();
                let frame = self.frames.last_mut().unwrap();
                if slot_mode {
                    // `args` is the table's last entry — index
                    // `regular_params.len()` (params seeded in order, all
                    // candidates under the slot gate).
                    frame.slots[regular_params.len()] = Value::from_str(&list_str);
                } else {
                    frame.locals.insert("args".to_string(), Value::from_str(&list_str));
                }
            }

            // Hand the invocation words to the frame for a potential
            // `info level 0` — refcount bumps into the pooled buffer, no
            // per-call Vec (the old code moved a fresh to_vec allocation
            // in here and dropped it at frame teardown).
            let frame = self.frames.last_mut().unwrap();
            if frame.level0.is_empty() {
                frame.level0.extend(cur.iter().cloned());
            }

            // ── OO variable prelink ─────────────────────────────────
            // The method dispatcher's `variable` declarations installed
            // here, at frame setup, instead of executing a `variable`
            // command per name per call.  Consumed once: a tail-call
            // rebind clears upvars anyway (the loop clears them every
            // iteration), matching the old prefix-inside-the-body
            // lifecycle.
            if !prelink_done {
                prelink_done = true;
                let links: &[PrelinkEntry] = prelink.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
                let frame_idx = self.frames.len() - 1;
                for entry in links.iter() {
                    let name = &entry.name;
                    let (qualified, ns_part) = (&entry.qualified, &entry.ns_part);
                    // The definition namespace always exists (the object
                    // created it), so the parent-exists check the
                    // `variable` command performs cannot fail here.  The
                    // registration runs ONCE per memo (a flag beside the
                    // entry) instead of probing the namespace's variable
                    // set on every call; the entry is retired with the
                    // chain memo, which the object namespace's lifetime
                    // bounds.  The `(qualified, ns_part)` pair is
                    // precomputed there too (pure string math).
                    if !entry.registered.get() {
                        if let Some(info) = self.namespaces.get_mut(&**ns_part) {
                            if !info.variables.contains(&**qualified) {
                                info.variables.insert(qualified.to_string());
                            }
                        }
                        entry.registered.set(true);
                    }
                    // The alias name leaves the slot model (link-only
                    // aliasing — the frame's other slots stay fast).
                    let glink = crate::interp::UpvarLink::Global(Rc::clone(qualified));
                    self.degrade_frame_link_at(frame_idx, name, entry.slot, &glink);
                    self.frames[frame_idx].upvars.insert(
                        Rc::clone(name),
                        crate::interp::UpvarLink::Global(Rc::clone(qualified)),
                    );
                    // No seed: live reads go through the upvar link; the
                    // seed copied the global and allocated a String key
                    // per call. A frame whose target namespace dies
                    // mid-method gets a dead-time copy instead
                    // (`materialize_frame_links`).
                }
            }

            // ── Inject static variables into the frame ─────────────
            for (sname, sval) in current_statics.iter() {
                self.frames.last_mut().unwrap().locals.insert(sname.clone(), sval.clone());
            }

            // ── Execute body ───────────────────────────────────────
            // A proc body's lines are body-relative (tclsh: `(procedure
            // "p" line 1)` is the body's first line) regardless of any
            // caller rebase in flight — an inlined foreach/if body leaves
            // line_offset rebased to its absolute position, and both body
            // executors (tree-walk eval, exec_bytecode's entry_offset)
            // would otherwise inherit it and mis-number the callee's
            // frames (min4: `(procedure "P" line 3)` instead of line 1).
            let saved_body_offset = self.line_offset;
            self.line_offset = 0;
            // A named proc's body compiled once at `proc` time runs
            // op-by-op here — same errors, same frames, no per-call
            // re-parse.  Everything else (non-parsing bodies, trace-
            // observed frames, `RTCL_NO_BYTECODE`, non-whitelisted ops,
            // a Tier1 command shadowed since compilation) stays on the
            // tree-walk.
            let mut result = match &def.compiled {
                Some(code)
                    if !code.fallback
                        && code.epoch == self.tier1_epoch
                        && super::vm_exec::bytecode_applicable(self) =>
                {
                    // M2 seam: the process-level JIT hook (if installed)
                    // gets first shot at this body; None keeps the
                    // bytecode executor.  All downstream decoding (ret /
                    // (procedure …) frames / break conversion) is shared.
                    match super::vm_exec::proc_body_executor(self, code) {
                        Some(r) => r,
                        None => super::vm_exec::exec_bytecode(self, code),
                    }
                }
                // The tree-walked body IS the proc's unit when a compiled
                // form exists but was bypassed (stale Tier1 epoch, exec
                // traces, RTCL_NO_BYTECODE, fallback ops): the unit the
                // compiler built inlines loop/if bodies and brackets, so
                // the tree-walked twin must keep the compiled-context
                // semantics through those constructs.  Bodies with NO
                // compiled form (apply lambdas, OO synthetics — and
                // non-parsing bodies) stay dispatched: in the bytecode
                // engine they run through `eval`'s fast path as plain
                // compile_units, and the two engines must agree byte-exact
                // (tclsh compiling lambda/method bodies like procs is a
                // recorded follow-up, not this change).
                _ => {
                    if def.compiled.is_some() {
                        self.next_eval_lexical = true;
                    }
                    self.eval(&def.body)
                }
            };
            self.line_offset = saved_body_offset;

            // A tailcall whose completion an inner catch consumed leaves
            // the frame marker armed: it still fires at body end and
            // replaces the result (tclsh tailcall-12.3a1:
            // `catch [list tailcall foo]; tailcall` → {}).
            if !matches!(&result, Err(e) if e.is_tail_call()) {
                if let Some(frame) = self.frames.last_mut() {
                    // `is_some` guard: the bare `take()` is a full
                    // `mem::replace` of the (24-byte) Option every call —
                    // on the overwhelmingly common unarmed frame the
                    // discriminant check is all that's needed.
                    if frame.tailcall.is_some() {
                        if let Some(tc_args) = frame.tailcall.take() {
                            if !tc_args.is_empty() {
                                result = Err(Error::tail_call(tc_args));
                            }
                        }
                    }
                }
            }

            // ── Check for tail-call signal ─────────────────────────
            match result {
                Err(e) if e.is_tail_call() => {
                    // Write back statics before switching to tail-call target
                    if !current_statics.is_empty() {
                        if let Some(frame) = self.frames.last() {
                            let snames: Vec<String> = current_statics.keys().cloned().collect();
                            for sname in &snames {
                                if let Some(val) = frame.locals.get(sname) {
                                    Rc::make_mut(&mut current_statics).insert(sname.clone(), val.clone());
                                }
                            }
                        }
                        if let Some(pdef) = self.procs.get_mut(current_proc_name.as_ref()) {
                            Rc::make_mut(pdef).statics = Rc::clone(&current_statics);
                            // The map entry was replaced: age every
                            // call-site token / resolution-cache entry
                            // holding a def clone.  Statics-bearing procs
                            // are never hot-loop material — one u64 bump.
                            self.note_cmd_mutation();
                        }
                        current_statics = Rc::new(HashMap::new());
                    }
                    let tc_args = match self.frames.last_mut().unwrap().tailcall.take() {
                        Some(a) => a,
                        None => e.into_tail_call_args().unwrap(),
                    };
                    if tc_args.is_empty() {
                        // `tailcall` with no command: the frame yields the
                        // empty completion value.
                        break Ok(Value::empty());
                    }
                    let cmd_name = tc_args[0].clone();
                    if let Some(new_proc) = self.procs.get(&cmd_name).cloned() {
                        // Tail-call to another proc — reuse the frame (no depth increase)
                        current_statics = Rc::clone(&new_proc.statics);
                        tc_def = Some(new_proc);
                        current_proc_name = Cow::Owned(cmd_name.clone());
                        owned_args = Some(tc_args.into_iter().map(|s| Value::from_str(&s)).collect());
                        // The reused frame's enterstep context now names the
                        // tail-call target (corpus-free: no extra enter/leave
                        // fires for the switched-to command).  Only a traced
                        // entry pushed one — without the guard this would
                        // clobber the CALLER's context.
                        if traced_entry {
                            let new_key =
                                super::commands::proc::resolve_command_key(self, &cmd_name)
                                    .unwrap_or_else(|| cmd_name.clone());
                            if let Some(top) = self.exec_step_stack.last_mut() {
                                *top = new_key;
                            }
                        }
                        continue;
                    } else {
                        // Target is a built-in — evaluate and return
                        let tc_values: Vec<Value> =
                            tc_args.iter().map(|s| Value::from_str(s)).collect();
                        break self.invoke_builtin_or_eval(&tc_values);
                    }
                }
                other => break other,
            }
        };

        // A break/continue that escaped the body had no enclosing loop to
        // stop it: tclsh converts it to an error at the proc boundary
        // (proc-old-5.14/5.15) — a caller's loop never sees it.  The
        // conversion happens where the loop instructions are absent, so no
        // command position is logged for it: errorInfo starts with the bare
        // message and the `(procedure "p" line 1)` frame reports line 1
        // (the interp's error line stays at its reset value), with
        // errorCode `TCL RESULT UNEXPECTED`.
        let final_result = match final_result {
            Err(e) if e.is_break() || e.is_continue() => {
                let msg = if e.is_break() {
                    "invoked \"break\" outside of a loop"
                } else {
                    "invoked \"continue\" outside of a loop"
                };
                self.err_info = Some(msg.to_string());
                self.err_fresh = false;
                self.err_line = 1;
                super::commands::list::set_error_code(self, "TCL RESULT UNEXPECTED");
                Err(Error::Msg(msg.to_string()))
            }
            other => other,
        };

        // `(procedure "name" line N)` frame when the body errored — the
        // line is the erroring command's line within the body, recorded by
        // the body's script harness.  Lambda bodies (apply) get the
        // `(lambda term ...)` tag from cmd_apply instead.
        if let Err(e) = &final_result {
            if self.err_is_error(e) && current_proc_name != "apply lambdaExpr" {
                let tag = format!("procedure \"{}\"", current_proc_name);
                self.err_exit_frame(&tag);
            }
        }

        // Write back static variables to the proc definition
        if !current_statics.is_empty() {
            if let Some(frame) = self.frames.last() {
                let mut updated = Rc::clone(&current_statics);
                for sname in updated.keys().cloned().collect::<Vec<_>>() {
                    if let Some(val) = frame.locals.get(&sname) {
                        Rc::make_mut(&mut updated).insert(sname, val.clone());
                    }
                }
                if let Some(pdef) = self.procs.get_mut(current_proc_name.as_ref()) {
                    Rc::make_mut(pdef).statics = updated;
                    // See the tail-call write-back: the entry was
                    // replaced, so cached def handles must age out.
                    self.note_cmd_mutation();
                }
            }
        }

        // Execute deferred scripts (from `defer` command) in reverse order
        if let Some(frame) = self.frames.last() {
            let scripts: Vec<String> = frame.deferred_scripts.clone();
            for script in scripts.iter().rev() {
                let _ = self.eval_isolated(script);
            }
        }

        // Clean up local procs (created by `local` command)
        if let Some(frame) = self.frames.last() {
            let procs_to_delete: Vec<String> = frame.local_procs.clone();
            for name in &procs_to_delete {
                super::vm_exec::note_tier1_mutation(self, name);
                self.procs.remove(name);
                self.commands.remove(name);
                self.aliases.remove(name);
            }
            if !procs_to_delete.is_empty() {
                self.note_cmd_mutation();
            }
        }

        // Frame teardown (tclsh): the proc's locals die with the frame —
        // on any completion, error unwinds included — and `unset` traces
        // registered on them fire then, after the values are gone
        // (proc-old-5.16: the trace fires while the body's error is
        // propagating out, and a trace error stays a background error that
        // leaves the in-flight errorInfo untouched).  Whole-array traces
        // fire once for the array; element-only registrations do not.
        // The per-call `F{i}:` prefix allocation only happens when there
        // are traces at all to find.
        if !self.var_traces.is_empty() {
            let teardown_prefix = format!("F{}:", self.frames.len() - 1);
            let traced: Vec<String> = self
                .var_traces
                .keys()
                .filter(|k| k.starts_with(&teardown_prefix))
                .map(|k| k[teardown_prefix.len()..].to_string())
                .collect();
            if !traced.is_empty() {
                if let Some(frame) = self.frames.last_mut() {
                    frame.locals.clear();
                    frame.array_locals.clear();
                }
                // Trace callbacks must not disturb the error (if any) that is
                // unwinding through this frame.
                let saved_info = self.err_info.take();
                let saved_fresh = std::mem::take(&mut self.err_fresh);
                let saved_raised = std::mem::take(&mut self.err_code_raised);
                let saved_pending = self.err_pending_top.take();
                for name in traced {
                    let _ = self.fire_traces(&name, None, "unset");
                }
                self.err_info = saved_info;
                self.err_fresh = saved_fresh;
                self.err_code_raised = saved_raised;
                self.err_pending_top = saved_pending;
            }
        }

        // Pop the frame and leave the definition namespace; the emptied
        // frame goes to the pool for the next call (its maps keep their
        // allocation).  Pool depth is naturally bounded by recursion.
        let mut popped = self.frames.pop().unwrap();
        popped.locals.clear();
        popped.array_locals.clear();
        popped.upvars.clear();
        popped.local_procs.clear();
        popped.deferred_scripts.clear();
        popped.slots.clear();
        popped.slot_aliased.clear();
        popped.has_aliased = false;
        popped.link_keys.clear();
        popped.slot_table = None;
        // Kept (cleared, with capacity) — the pooled frame reuses the
        // buffer for its next invocation's `info level 0` words.
        popped.level0.clear();
        if popped.tailcall.is_some() {
            popped.tailcall = None;
        }
        // `popped.ns`/`popped.call_ns` are retained (see the prologue):
        // the next invocation's identity checks decide whether they are
        // replaced; the pooled frame keeps at most two namespace `Rc`s.
        if self.frame_pool.len() < 32 {
            self.frame_pool.push(popped);
        }
        // Frame-local trace tables keyed F{idx}:... must not leak into the
        // next call that reuses this frame index.  (The retain only runs
        // when a table is non-empty — the format! below was a per-call
        // allocation that `p` on an empty table didn't pay for in tclsh.)
        if !self.var_traces.is_empty()
            || !self.elem_traces.is_empty()
            || !self.trace_phantoms.is_empty()
        {
            let fkey = format!("F{}:", self.frames.len());
            self.var_traces.retain(|k, _| !k.starts_with(&fkey));
            self.elem_traces.retain(|k, _| !k.starts_with(&fkey));
            self.trace_phantoms.retain(|k, _| !k.starts_with(&fkey));
        }
        self.current_namespace = prev_namespace;
        self.call_depth -= 1;

        // Execution traces: the frame is gone; `leave` fires with the
        // completion code and result (errors are background errors).
        // Only a traced entry pushed a context (and has the key/cmdtext
        // to report); a trace registered mid-body does not fire for the
        // already-running invocation (tclsh-probed above).
        if traced_entry {
            self.exec_step_stack.pop();
            if !self.exec_traces.is_empty() {
                if let Some((exec_key, exec_cmdtext)) = &leave_ctx {
                    let pair = match &final_result {
                        Ok(v) => Some(("0", v.as_str().to_string())),
                        Err(e) if self.err_is_error(e) => {
                            Some(("1", e.message_text().to_string()))
                        }
                        _ => None,
                    };
                    if let Some((code, res)) = pair {
                        self.exec_fire_leave(exec_key, exec_cmdtext, code, &res);
                    }
                }
            }
        }

        match final_result {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_return() {
                    match &e {
                        Error::ControlFlow { level, value, error_info, error_code, .. } => {
                            // Propagate -errorcode to the global variable;
                            // -errorinfo only rides along with an actual
                            // error completion (a `return -code ok
                            // -errorinfo ...` leaves ::errorInfo alone).
                            if let Some(code) = error_code {
                                let _ = self.set_var("::errorCode", Value::from_str(code));
                                self.err_code_raised = true;
                            }
                            let val = value.clone().unwrap_or_default();
                            match *level {
                                0 => {
                                    // Plain return (level=0 means just return the value)
                                    Ok(val)
                                }
                                n if n < 0 => {
                                    // Explicit -level N encoded as -(N+1): any of
                                    // these leaves this proc with its value
                                    // (rtcl has no multi-level proc return).
                                    Ok(val)
                                }
                                1 => {
                                    // return -code error "msg" → propagate as error,
                                    // preserving any -errorinfo / -errorcode options.
                                    // A -errorinfo option REPLACES the auto-built
                                    // `msg\n    while executing` base of the
                                    // errorInfo (tclsh TclProcessReturn); the
                                    // enclosing-command frames still append to it.
                                    if let Some(info) = error_info {
                                        self.err_info = Some(info.clone());
                                        self.err_fresh = false;
                                    }
                                    if let Some(info) = error_info {
                                        let _ = self.set_var("::errorInfo", Value::from_str(info));
                                    }
                                    Err(Error::ControlFlow {
                                        kind: crate::error::ControlFlow::Error,
                                        value: Some(val),
                                        level: 1,
                                        error_info: error_info.clone(),
                                        error_code: error_code.clone(),
                                    })
                                }
                                3 => {
                                    // return -code break: the break that
                                    // reaches the caller carries the
                                    // return's value (tclsh: `return -code
                                    // break x` in a proc → `catch r` m=x).
                                    Err(Error::ControlFlow {
                                        kind: crate::error::ControlFlow::Break,
                                        value: Some(val),
                                        level: 1,
                                        error_info: None,
                                        error_code: None,
                                    })
                                }
                                4 => {
                                    // return -code continue: value carried
                                    // the same way.
                                    Err(Error::ControlFlow {
                                        kind: crate::error::ControlFlow::Continue,
                                        value: Some(val),
                                        level: 1,
                                        error_info: None,
                                        error_code: None,
                                    })
                                }
                                _ => Ok(val),
                            }
                        }
                        _ => Ok(Value::empty()),
                    }
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Invoke a command from tail-call args. Tries builtins first, falls back to eval.
    fn invoke_builtin_or_eval(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Ok(Value::empty());
        }
        let cmd_name = args[0].as_str();
        if let Some(f) = self.commands.get(cmd_name).cloned() {
            self.call_depth += 1;
            let result = f(self, args);
            self.call_depth -= 1;
            self.fill_wrong_args(cmd_name, result)
        } else {
            // Fallback: build script string and eval。
            // 同上：词序列化必须走规范 list 形式（含引号词才会往返），
            // 否则拼出的脚本会把 `{"a":1}` 拆成碎的词。
            let script = Value::from_list(args).as_str().to_string();
            self.eval(&script)
        }
    }
}

/// The namespace part of a qualified proc name: `::foo::p` → `::foo`,
/// `::p` → `::`, bare `p` → `::`.  The root case hands out the shared
/// [`Interp::ns_root`] — a global-scope proc call allocates nothing here.
fn ns_of_qualified(interp: &Interp, name: &str) -> Rc<str> {
    match rfind_ns_sep(name) {
        Some(0) => Rc::clone(&interp.ns_root),
        Some(pos) => Rc::from(format!("::{}", name[..pos].trim_start_matches("::"))),
        None => Rc::clone(&interp.ns_root),
    }
}

impl Interp {
    /// JIT 直呼快路径（call_argv 直连路由的 dispatch 免除层）：
    /// cmd_cache 命中 proc 且形状门过 → 精简帧机器（call_proc 的
    /// 无循环/无 trace/无 statics 版；epilogue 语义逐项复刻——
    /// break/continue 转错、(procedure …) 帧、deferred/local-proc/
    /// var-trace 兜底、return -code 解码、帧池归还）。形状门不过或
    /// 非 proc → None（调用方退回 dispatch_values 全链）。
    ///
    /// 不支持的形态（在门或兜底里保持正确）：
    /// - 执行 trace：入口门排除（enter/leave 语境依赖全链）
    /// - statics：入口门排除（statics 注入/回写依赖 call_proc 循环）
    /// - tailcall：出口检测——弹帧后按重分派语义交 dispatch_values
    ///   （同深度：帧已弹，目标帧顶替；无 (procedure "P") 帧——
    ///   与 call_proc 的帧复用语义一致）
    /// - apply/OO：level0_override/prelink 仅由那些入口武装——
    ///   本路径不读（cmd_cache 的 proc 键不含它们）
    pub fn jit_call_proc_direct(&mut self, name: &str, args: &[Value]) -> Option<Result<Value>> {
        if !self.exec_traces.is_empty() {
            return None;
        }
        let def = match self.cmd_cache_get(name) {
            Some(super::ResolvedCmd::Proc(key)) => self.procs.get(key.as_ref()).cloned()?,
            _ => return None,
        };
        if !def.statics.is_empty() {
            return None;
        }
        // 名字借用调用方切片（call_proc 的 Cow::Borrowed 同款——每调用
        // 一次 Rc<str> 堆分配即 ~40ns，fib A/B 实证）。
        let key: &str = name;
        if self.call_depth > self.max_call_depth {
            return Some(Err(Error::runtime(
                "maximum recursion depth exceeded",
                crate::error::ErrorCode::StackOverflow,
            )));
        }
        self.call_depth += 1;

        // 定义命名空间（与 call_proc 同源：注册名的限定前缀）。
        let prev_namespace = self.current_namespace.clone();
        let mut frame = match self.frame_pool.pop() {
            Some(mut f) => {
                f.locals.clear();
                f.array_locals.clear();
                f.upvars.clear();
                f.local_procs.clear();
                f.deferred_scripts.clear();
                f.slots.clear();
                f.slot_aliased.clear();
                f.link_keys.clear();
                f.has_aliased = false;
                f.slot_table = None;
                f
            }
            None => Box::new(CallFrame {
                locals: VarMap::default(),
                array_locals: VarSet::default(),
                upvars: UpvarMap::default(),
                ns: None,
                call_ns: None,
                local_procs: Vec::new(),
                deferred_scripts: Vec::new(),
                slots: Vec::with_capacity(8),
                slot_aliased: Vec::new(),
                has_aliased: false,
                link_keys: Vec::new(),
                slot_table: None,
                tailcall: None,
                level0: Vec::new(),
                ns_depth: 0,
            }),
        };
        if !matches!(&frame.call_ns, Some(n) if Rc::ptr_eq(n, &prev_namespace)) {
            frame.call_ns = Some(Rc::clone(&prev_namespace));
        }
        frame.tailcall = None;
        frame.level0.clear();
        frame.ns_depth = self.ns_level0.len();
        self.frames.push(frame);

        // slot-locals 门 + 种子（call_proc 同款——params 对齐 + 无
        // statics；OO prelink 槽的 LOAD-BEARING 不变量同源）。
        let slot_mode = match &def.compiled {
            Some(code)
                if !code.fallback
                    && code.epoch == self.tier1_epoch
                    && super::vm_exec::bytecode_applicable(self)
                    && !code.locals().is_empty()
                    && code.params_aligned == Some(def.params.len()) =>
            {
                let table = Rc::clone(code);
                let n = code.locals().len();
                let frame = self.frames.last_mut().unwrap();
                frame.slots.clear();
                frame.slots.resize(n, Value::unset());
                frame.slot_aliased.clear();
                frame.has_aliased = false;
                frame.link_keys.clear();
                frame.slot_table = Some(table);
                true
            }
            _ => {
                let frame = self.frames.last_mut().unwrap();
                frame.slots.clear();
                frame.slot_aliased.clear();
                frame.has_aliased = false;
                frame.link_keys.clear();
                frame.slot_table = None;
                false
            }
        };

        // 定义命名空间入帧（identity-checked 写，call_proc 同款）。
        {
            let def_ns = ns_of_qualified(self, &key);
            if !Rc::ptr_eq(&self.current_namespace, &def_ns) {
                self.current_namespace = Rc::clone(&def_ns);
            }
            let frame = self.frames.last_mut().unwrap();
            if !matches!(&frame.ns, Some(n) if Rc::ptr_eq(n, &def_ns)) {
                frame.ns = Some(def_ns);
            }
            frame.locals.clear();
            frame.array_locals.clear();
            frame.upvars.clear();
        }

        let has_args = def.params.last().map(|(p, _)| p.as_str()) == Some("args");
        let regular_params = if has_args {
            &def.params[..def.params.len() - 1]
        } else {
            &def.params[..]
        };

        // arity 检查（tclsh 同款报错形状）。
        {
            let num_args = args.len().saturating_sub(1);
            let mut arity_ok = has_args || num_args <= regular_params.len();
            if arity_ok {
                for (i, (_, d)) in regular_params.iter().enumerate() {
                    if i >= num_args && d.is_none() {
                        arity_ok = false;
                        break;
                    }
                }
            }
            if !arity_ok {
                let name_word = crate::value::tcl_quote(&key);
                let mut usage = name_word;
                for (i, (p, d)) in def.params.iter().enumerate() {
                    usage.push(' ');
                    if has_args && i == def.params.len() - 1 {
                        usage.push_str("?arg ...?");
                    } else if d.is_some() {
                        usage.push('?');
                        usage.push_str(p);
                        usage.push('?');
                    } else {
                        usage.push_str(p);
                    }
                }
                super::commands::list::set_error_code(self, "TCL WRONGARGS");
                let e = Error::Msg(format!("wrong # args: should be \"{}\"", usage));
                // 帧清理走共享出口（self.frames 已 push）。
                self.jit_frame_epilogue_pop(prev_namespace);
                self.call_depth -= 1;
                return Some(Err(e));
            }
        }

        // 位置绑定 + args 包（call_proc 同款）。
        {
            let frame = self.frames.last_mut().unwrap();
            for (i, (param, default)) in regular_params.iter().enumerate() {
                let value = if i + 1 < args.len() {
                    args[i + 1].clone()
                } else if let Some(d) = default {
                    Value::from_str(d)
                } else {
                    Value::empty()
                };
                if slot_mode {
                    frame.slots[i] = value;
                } else {
                    frame.locals.insert(param.clone(), value);
                }
            }
        }
        if has_args {
            let remaining_start = regular_params.len() + 1;
            let remaining: Vec<Value> = if remaining_start < args.len() {
                args[remaining_start..].to_vec()
            } else {
                Vec::new()
            };
            let list_str = Value::from_list(&remaining).as_str().to_string();
            let frame = self.frames.last_mut().unwrap();
            if slot_mode {
                frame.slots[regular_params.len()] = Value::from_str(&list_str);
            } else {
                frame.locals.insert("args".to_string(), Value::from_str(&list_str));
            }
        }
        {
            let frame = self.frames.last_mut().unwrap();
            if frame.level0.is_empty() {
                frame.level0.extend(args.iter().cloned());
            }
        }

        // 体执行（Site A 钩子优先；gate 与 call_proc 一致）。
        let result = match &def.compiled {
            Some(code)
                if !code.fallback
                    && code.epoch == self.tier1_epoch
                    && super::vm_exec::bytecode_applicable(self) =>
            {
                match super::vm_exec::proc_body_executor(self, code) {
                    Some(r) => r,
                    None => super::vm_exec::exec_bytecode(self, code),
                }
            }
            _ => {
                if def.compiled.is_some() {
                    self.next_eval_lexical = true;
                }
                self.eval(&def.body)
            }
        };

        // ── Epilogue（call_proc 语义复刻）────────────────────────────
        // tailcall：弹帧后重分派（帧顶替、同深度、无 P 帧）。
        let result = match result {
            Err(e) if e.is_tail_call() => {
                let tc_args = match self.frames.last_mut().unwrap().tailcall.take() {
                    Some(a) => a,
                    None => match e.into_tail_call_args() {
                        Some(a) => a,
                        None => Vec::new(),
                    },
                };
                self.jit_frame_epilogue_pop(prev_namespace);
                self.call_depth -= 1;
                if tc_args.is_empty() {
                    return Some(Ok(Value::empty()));
                }
                let vals: Vec<Value> =
                    tc_args.iter().map(|s| Value::from_str(s)).collect();
                return Some(self.dispatch_values(&vals));
            }
            r => r,
        };

        // break/continue 逃逸转错（proc 边界，tclsh proc-old-5.14）。
        let result = match result {
            Err(e) if e.is_break() || e.is_continue() => {
                let msg = if e.is_break() {
                    "invoked \"break\" outside of a loop"
                } else {
                    "invoked \"continue\" outside of a loop"
                };
                self.err_info = Some(msg.to_string());
                self.err_fresh = false;
                self.err_line = 1;
                super::commands::list::set_error_code(self, "TCL RESULT UNEXPECTED");
                Err(Error::Msg(msg.to_string()))
            }
            other => other,
        };

        // `(procedure "name" line N)` 出口帧。
        if let Err(e) = &result {
            if self.err_is_error(e) {
                let tag = format!("procedure \"{}\"", key);
                self.err_exit_frame(&tag);
            }
        }

        // deferred / local procs / var-trace 兜底（无则零成本）。
        {
            let (scripts, procs_to_delete) = match self.frames.last() {
                Some(frame) => (
                    frame.deferred_scripts.clone(),
                    frame.local_procs.clone(),
                ),
                None => (Vec::new(), Vec::new()),
            };
            if !scripts.is_empty() {
                for script in scripts.iter().rev() {
                    let _ = self.eval_isolated(script);
                }
            }
            if !procs_to_delete.is_empty() {
                for pname in &procs_to_delete {
                    super::vm_exec::note_tier1_mutation(self, pname);
                    self.procs.remove(pname);
                    self.commands.remove(pname);
                    self.aliases.remove(pname);
                }
                self.note_cmd_mutation();
            }
        }
        if !self.var_traces.is_empty() {
            let teardown_prefix = format!("F{}:", self.frames.len() - 1);
            let traced: Vec<String> = self
                .var_traces
                .keys()
                .filter(|k| k.starts_with(&teardown_prefix))
                .map(|k| k[teardown_prefix.len()..].to_string())
                .collect();
            if !traced.is_empty() {
                if let Some(frame) = self.frames.last_mut() {
                    frame.locals.clear();
                    frame.array_locals.clear();
                }
                let saved_info = self.err_info.take();
                let saved_fresh = std::mem::take(&mut self.err_fresh);
                let saved_raised = std::mem::take(&mut self.err_code_raised);
                let saved_pending = self.err_pending_top.take();
                for tname in traced {
                    let _ = self.fire_traces(&tname, None, "unset");
                }
                self.err_info = saved_info;
                self.err_fresh = saved_fresh;
                self.err_code_raised = saved_raised;
                self.err_pending_top = saved_pending;
            }
        }

        self.jit_frame_epilogue_pop(prev_namespace);
        self.call_depth -= 1;

        // return -code 解码（call_proc 同款）。
        Some(match result {
            Ok(v) => Ok(v),
            Err(e) => {
                if e.is_return() {
                    match &e {
                        Error::ControlFlow { level, value, error_info, error_code, .. } => {
                            if let Some(code) = error_code {
                                let _ = self.set_var("::errorCode", Value::from_str(code));
                                self.err_code_raised = true;
                            }
                            let val = value.clone().unwrap_or_default();
                            match *level {
                                0 => Ok(val),
                                n if n < 0 => Ok(val),
                                1 => {
                                    if let Some(info) = error_info {
                                        self.err_info = Some(info.clone());
                                        self.err_fresh = false;
                                    }
                                    if let Some(info) = error_info {
                                        let _ = self.set_var("::errorInfo", Value::from_str(info));
                                    }
                                    Err(Error::ControlFlow {
                                        kind: crate::error::ControlFlow::Error,
                                        value: Some(val),
                                        level: 1,
                                        error_info: error_info.clone(),
                                        error_code: error_code.clone(),
                                    })
                                }
                                3 => Err(Error::ControlFlow {
                                    kind: crate::error::ControlFlow::Break,
                                    value: Some(val),
                                    level: 1,
                                    error_info: None,
                                    error_code: None,
                                }),
                                4 => Err(Error::ControlFlow {
                                    kind: crate::error::ControlFlow::Continue,
                                    value: Some(val),
                                    level: 1,
                                    error_info: None,
                                    error_code: None,
                                }),
                                _ => Ok(val),
                            }
                        }
                        _ => Ok(Value::empty()),
                    }
                } else {
                    Err(e)
                }
            }
        })
    }

    /// 精简帧出口的弹帧+池归还+trace 键清理（epilogue 共用；命名空间
    /// 恢复由调用方持有 prev_namespace）。
    fn jit_frame_epilogue_pop(&mut self, prev_namespace: Rc<str>) {
        let mut popped = self.frames.pop().unwrap();
        popped.locals.clear();
        popped.array_locals.clear();
        popped.upvars.clear();
        popped.local_procs.clear();
        popped.deferred_scripts.clear();
        popped.slots.clear();
        popped.slot_aliased.clear();
        popped.has_aliased = false;
        popped.link_keys.clear();
        popped.slot_table = None;
        popped.level0.clear();
        if popped.tailcall.is_some() {
            popped.tailcall = None;
        }
        if self.frame_pool.len() < 32 {
            self.frame_pool.push(popped);
        }
        if !self.var_traces.is_empty()
            || !self.elem_traces.is_empty()
            || !self.trace_phantoms.is_empty()
        {
            let fkey = format!("F{}:", self.frames.len());
            self.var_traces.retain(|k, _| !k.starts_with(&fkey));
            self.elem_traces.retain(|k, _| !k.starts_with(&fkey));
            self.trace_phantoms.retain(|k, _| !k.starts_with(&fkey));
        }
        self.current_namespace = prev_namespace;
    }
}
