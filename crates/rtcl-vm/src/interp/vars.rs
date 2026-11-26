//! Variable access methods on [`Interp`].
//!
//! All variable reads/writes are scope-aware: at global level they use
//! `Interp::globals`; inside a proc they use the current `CallFrame`'s
//! locals (or follow upvar links to globals / other frames).

use super::util::{has_ns_sep, is_array_element_name, split_array_ref};
use super::{tclhash, CallFrame, Interp, Rc, UpvarLink};
use crate::error::{Error, ErrorCode, Result};
use crate::value::Value;

impl Interp {
    /// Insert into the global var table, appending the key to the
    /// Tcl-order log when the insert was NEW (a re-set keeps its
    /// position; a recreate after delete appends anew — Tcl hash-entry
    /// semantics).
    pub(crate) fn global_insert(&mut self, key: String, v: Value) {
        if self.globals.insert(key.clone(), v).is_none() {
            self.var_order.push(key);
        }
        // A log grown far past the table (variables made and unset in a
        // loop) is brought back to the live entries.
        if self.var_order.len() > 2 * self.globals.len() + 64 {
            let live = |k: &str| self.globals.contains_key(k);
            compact_order(&mut self.var_order, &live);
        }
    }

    /// Remove from the global var table (log entry becomes stale; the
    /// iteration walk filters by table liveness and dedups to the last
    /// occurrence).
    pub(crate) fn global_remove(&mut self, key: &str) -> Option<Value> {
        self.globals.remove(key)
    }
}

/// Insert into a frame's locals, maintaining the frame's Tcl-order log.
pub(crate) fn frame_local_insert(f: &mut CallFrame, key: String, v: Value) {
    if f.locals.insert(key.clone(), v).is_none() {
        f.locals_order.push(key);
    }
    if f.locals_order.len() > 2 * f.locals.len() + 64 {
        let live = |k: &str| f.locals.contains_key(k);
        compact_order(&mut f.locals_order, &live);
    }
}

// ── T4 命名空间体快路径的键拼装 ─────────────────────────────────
//
// 体里裸名的解析键 = `<ns>::<名字>`（`canonical_global_in` 平名分支
// `format!("{}::{}", ns, name)` 后剥 `::` 的产出）。快路径把键拼在
// 调用方的栈缓冲里——每次读写零堆分配；两段都已是 UTF-8，拼接仍必
// 是 UTF-8。64 字节封顶：更长的名字回落全链（正确性不变，慢一点）。

/// 拼出 `<ns>::<name>`（`ns` 恒规整形 `::x`——与 ns_var_declared 的
/// `&self.current_namespace[2..]` 同一拆法），返回入缓冲的切片。
fn stack_ns_key<'b>(ns: &str, name: &str, buf: &'b mut [u8]) -> Option<&'b str> {
    let base = &ns[2..];
    let len = base.len() + 2 + name.len();
    if len > buf.len() {
        return None;
    }
    buf[..base.len()].copy_from_slice(base.as_bytes());
    buf[base.len()..base.len() + 2].copy_from_slice(b"::");
    buf[base.len() + 2..len].copy_from_slice(name.as_bytes());
    Some(std::str::from_utf8(&buf[..len]).expect("utf8 by construction"))
}

/// 数组读的键对：宿主 `<ns>::<base>` 与元素 `<ns>::<base>(<idx>)`，
/// 同一栈缓冲拼出（先写宿主段，再补 `(idx)`）。宿主切片与元素切片
/// 同源于 buf（写完后取共享视图）。
fn stack_ns_elem_keys<'b>(
    ns: &str,
    base: &str,
    index: &str,
    buf: &'b mut [u8],
) -> Option<(&'b str, &'b str)> {
    let stripped = &ns[2..];
    let host_len = stripped.len() + 2 + base.len();
    let elem_len = host_len + 1 + index.len() + 1;
    if elem_len > buf.len() {
        return None;
    }
    let mut n = 0;
    buf[n..n + stripped.len()].copy_from_slice(stripped.as_bytes());
    n += stripped.len();
    buf[n..n + 2].copy_from_slice(b"::");
    n += 2;
    buf[n..n + base.len()].copy_from_slice(base.as_bytes());
    buf[host_len] = b'(';
    buf[host_len + 1..host_len + 1 + index.len()].copy_from_slice(index.as_bytes());
    buf[elem_len - 1] = b')';
    let host = std::str::from_utf8(&buf[..host_len]).expect("utf8 by construction");
    let elem = std::str::from_utf8(&buf[..elem_len]).expect("utf8 by construction");
    Some((host, elem))
}

/// Remove from a frame's locals (log entry goes stale, as above).
pub(crate) fn frame_local_remove(f: &mut CallFrame, key: &str) -> Option<Value> {
    f.locals.remove(key)
}

/// Log a frame-table entry whose creation did not go through
/// [`frame_local_insert`] — a `global`/`upvar`/`variable` link with
/// nothing to mirror: tclsh's frame table holds the entry all the same
/// (`global nosuch; info vars` lists nosuch), and the hash-order replay
/// needs it in the creation sequence.  A name already logged keeps its
/// entry (tclsh: a re-link keeps the table position).
pub(crate) fn frame_log_link(f: &mut CallFrame, key: &str) {
    if !f.locals_order.iter().any(|o| o == key) {
        f.locals_order.push(key.to_string());
    }
}

/// Bring an insertion log back to what the replay reads of it: the keys
/// still in the table, one entry each.  What remains replays to the same
/// order; the stale entries and the other occurrences were skipped anyway.
/// They were kept for ever: a loop making and unsetting variables grew the
/// log without end, and every listing walked all of it.
///
/// The entry kept is the last occurrence — a key made again after a
/// delete takes a new place in the table.  A frame local too: the listing
/// replays the creation sequence through the hash mechanics, so the
/// recreated key must sit at its recreated place (a compiled local is not
/// in this replay at all — it lists at its slot position).
fn compact_order(order: &mut Vec<String>, live: &dyn Fn(&str) -> bool) {
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(order.len());
    let mut keep = vec![false; order.len()];
    // Walking back from the end, the first occurrence met is the last.
    for (i, k) in order.iter().enumerate().rev() {
        if live(k) && seen.insert(k.as_str()) {
            keep[i] = true;
        }
    }
    drop(seen);
    let mut i = 0;
    order.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}

/// [`Interp::var_frame_idx`] over the frame stack and the number of open
/// `namespace eval` bodies.
fn scope_frame_idx(frames: &[Box<CallFrame>], ns_open: usize) -> Option<usize> {
    let idx = frames.len().checked_sub(1)?;
    match frames.get(idx) {
        Some(f) if f.ns_depth >= ns_open => Some(idx),
        _ => None,
    }
}

/// Where a resolved variable lives, after following upvar links and
/// `::`-qualification to its *owning* scope.
#[derive(Debug, Clone)]
enum VarLoc {
    /// Global slot: `Interp::globals[<name>]` (name canonical, no `::`).
    Global(String),
    /// Frame slot: `frames[i].locals[<name>]`.
    Frame(usize, String),
}

impl VarLoc {
    fn base_name(&self) -> &str {
        match self {
            VarLoc::Global(n) | VarLoc::Frame(_, n) => n,
        }
    }

    /// Flat key for an array element in this scope.
    fn element_key(&self, index: &str) -> String {
        format!("{}({})", self.base_name(), index)
    }
}

/// Classify an error produced while reading a variable: Tcl treats a
/// missing variable differently from a type conflict in several commands
/// (`incr` starts from 0 for missing, but propagates type errors).
pub(crate) fn is_type_conflict(err: &Error) -> bool {
    let msg = err.to_string();
    msg.contains("variable is array") || msg.contains("variable isn't array")
}

/// Canonical identity key for a variable location (scope-aware).
fn stamp_key(loc: &VarLoc) -> String {
    match loc {
        VarLoc::Global(n) => format!("G:{}", n),
        VarLoc::Frame(i, n) => format!("F{}:{}", i, n),
    }
}

impl Interp {
    // ── internal helpers ────────────────────────────────────────

    // ── E2 slot-locals aliasing ───────────────────────────────────
    //
    // tclsh's compiledLocals array and its locals hash table alias the
    // same Var entries.  rtcl models that with slots as the canonical
    // store of table names plus these consults on every name-keyed path:
    // an uncompiled writer (foreach var, lappend, catch result, `global`
    // mirror…) observes the same variable the compiled ops touch.

    /// Value of slot `slot` in the current frame, when it holds one
    /// (unset slots and name-keyed frames both miss — the callers fall
    /// back to the name path).
    /// 顶层 slot 边界（C2'）：ts set 槽 → 全局（分派读/链接写见最新）。
    pub(crate) fn top_flush(&mut self) {
        let flush: Vec<(String, Value)> = {
            let Some(ts) = self.top_slots.as_ref() else { return };
            (0..ts.names.len())
                .filter(|&i| !ts.values[i].is_unset())
                .map(|i| (ts.names[i].clone(), ts.values[i].clone()))
                .collect()
        };
        crate::interp::dbgtrace::event("flush", &flush.len().to_string());
        for (name, v) in flush {
            self.global_insert(name, v);
        }
    }

    /// 顶层 slot 边界（C2'）：全槽 invalidate（分派可能改全局——槽读
    /// 回落全局拉新）。数组名标记 aliased（槽写回落名路径 → is-array
    /// 错误归属全链）。
    pub(crate) fn top_invalidate(&mut self) {
        crate::interp::dbgtrace::event("invalidate", "");
        let Some(ts) = self.top_slots.as_mut() else { return };
        for i in 0..ts.names.len() {
            ts.aliased[i] = !self.array_globals.is_empty()
                && self.array_globals.contains(&ts.names[i]);
            ts.values[i] = Value::unset();
        }
    }

    /// 顶层 slot 单元（C2'）：单槽失效（纯 op 回落名路径写全局后——
    /// 下一次槽读拉回全局真值）。
    pub(crate) fn top_slot_invalidate_one(&mut self, slot: usize) {
        crate::interp::dbgtrace::event("inv1", "");
        let Some(ts) = self.top_slots.as_mut() else { return };
        if let Some(cell) = ts.values.get_mut(slot) {
            *cell = Value::unset();
        }
    }

    /// 顶层 slot 单元（C2'）：名路径读后填槽（cache-fill）。
    pub(crate) fn top_slot_fill(&mut self, slot: usize, value: &Value) {
        crate::interp::dbgtrace::event("fill", "");
        let Some(ts) = self.top_slots.as_mut() else { return };
        let Some(cell) = ts.values.get_mut(slot) else { return };
        if cell.is_unset() {
            *cell = value.clone();
        }
    }

    /// The frame that bare-name variable operations act on, and its
    /// index: the innermost proc frame — unless a `namespace eval` body is
    /// open above it, which is `None`.  tclsh gives the body its own
    /// variable frame (`TclPushStackFrame` on the namespace, not a call
    /// frame): the proc's locals are not visible in the body, and a bare
    /// name is the namespace's variable.  A frame's `ns_depth` records how
    /// many bodies were open when it was pushed; one more open now means
    /// the body is the innermost scope.  The proc frame still counts as a
    /// call level for `info level` and `uplevel`, which keep reading
    /// `frames` itself.
    ///
    /// This used to be `frames.last()` everywhere: a body evaluated from
    /// inside a proc read and wrote the proc's locals —
    /// `proc Q {} { set x secret; namespace eval n { set x 1 } }` left
    /// `x` at 1 — and never saw the namespace's variables.
    pub(crate) fn var_frame_idx(&self) -> Option<usize> {
        scope_frame_idx(&self.frames, self.ns_stack.len())
    }

    pub(crate) fn var_frame(&self) -> Option<&CallFrame> {
        self.var_frame_idx().map(|i| &*self.frames[i])
    }

    pub(crate) fn var_frame_mut(&mut self) -> Option<&mut CallFrame> {
        match self.var_frame_idx() {
            Some(i) => self.frames.get_mut(i).map(|f| &mut **f),
            None => None,
        }
    }

    pub(crate) fn frame_slot_value(&self, slot: usize) -> Option<Value> {
        // 顶层 slot 单元（C2'）：ts 权威读；unset → 名路径（全局拉取/
        // 错误归属）。
        if let Some(ts) = &self.top_slots {
            let v = ts.values.get(slot)?;
            if v.is_unset() || ts.aliased.get(slot).copied().unwrap_or(false) {
                return None;
            }
            return Some(v.clone());
        }
        let f = self.frames.last()?;
        if f.has_aliased && f.slot_aliased.get(slot).copied().unwrap_or(false) {
            // Global link: ONE probe with the canonical key (the name
            // path would pay upvars + globals = two hashes).
            let key = f.link_keys.get(slot)?.as_ref()?;
            return self.globals.get(key.as_ref()).cloned();
        }
        let v = f.slots.get(slot)?;
        if v.is_unset() {
            return None;
        }
        Some(v.clone())
    }

    /// Write `value` into slot `slot` of the current frame.  `false` when
    /// the frame is not slot-compiled (or has degraded) — the caller then
    /// takes the name path.  Takes the value by reference: the refcount
    /// bump happens only on the write, so a caller whose fallback consumes
    /// the value pays nothing extra on the attempt.
    /// Whether the compiled local at `slot` is const-marked in the
    /// current frame (the VM's slot stores route such writes to `set_var`,
    /// whose guard owns the can't-set error).  `None` consts — the common
    /// frame — pays one Option probe.
    pub(crate) fn frame_local_is_const(
        &self,
        slot: usize,
        code: &rtcl_parser::ByteCode,
    ) -> bool {
        let Some(cs) = self.frames.last().and_then(|f| f.consts.as_ref()) else {
            return false;
        };
        code.locals()
            .get(slot)
            .map(|n| cs.iter().any(|c| c == n))
            .unwrap_or(false)
    }

    pub(crate) fn frame_slot_write(&mut self, slot: usize, value: &Value) -> bool {
        // 顶层 slot 单元（C2'）：ts 写（数组化槽 → 名路径，is-array
        // 错误归属全链）。
        if let Some(ts) = self.top_slots.as_mut() {
            let Some(cell) = ts.values.get_mut(slot) else { return false };
            if ts.aliased.get(slot).copied().unwrap_or(false) {
                return false;
            }
            *cell = value.clone();
            return true;
        }
        let aliased = self
            .frames
            .last()
            .map(|f| f.has_aliased && f.slot_aliased.get(slot).copied().unwrap_or(false))
            .unwrap_or(true);
        if aliased {
            // D-37⑧：链接写（global/upvar 别名）在有 trace 时回落名路径
            // ——set_var 以别名名触发 trace 并承担写后重读语义（tclsh
            // name1=别名名）。被 trace 的帧局部在注册时已移出槽模型，
            // 此处只有链接直写需要拦。move 变体的 Err(value) 就是调用方
            // 名路径的值句柄。
            if !self.var_traces.is_empty() {
                return false;
            }
            // Global link: write the target directly through the cached
            // canonical key (one probe; creation goes through the name
            // path — the link install seeded the variable, so the hot
            // case is always the hit).
            let key = self
                .frames
                .last()
                .and_then(|f| f.link_keys.get(slot))
                .and_then(|k| k.clone());
            if let Some(key) = key {
                if let Some(cell) = self.globals.get_mut(key.as_ref()) {
                    *cell = value.clone();
                    return true;
                }
                let name = self.frames.last().and_then(|f| {
                    f.slot_table.as_ref().and_then(|t| t.locals().get(slot).cloned())
                });
                if let Some(name) = name {
                    let _ = self.set_var(&name, value.clone());
                    return true;
                }
            }
            return false;
        }
        match self.frames.last_mut().and_then(|f| f.slots.get_mut(slot)) {
            Some(cell) => {
                *cell = value.clone();
                true
            }
            None => false,
        }
    }

    /// The `StoreLocalPop` storage arm: same fast path and delegation
    /// points as [`Self::frame_slot_write`], but the caller's only
    /// remaining handle to `value` moves into the cell — the discarded
    /// result never pays a clone.  `Err(value)` returns that handle to
    /// the caller (which then takes the name path, exactly the `false`
    /// branch of the cloning write).
    pub(crate) fn frame_slot_write_move(
        &mut self,
        slot: usize,
        value: Value,
    ) -> core::result::Result<(), Value> {
        if let Some(ts) = self.top_slots.as_mut() {
            let Some(cell) = ts.values.get_mut(slot) else {
                return Err(value);
            };
            if ts.aliased.get(slot).copied().unwrap_or(false) {
                return Err(value);
            }
            *cell = value;
            return Ok(());
        }
        let aliased = self
            .frames
            .last()
            .map(|f| f.has_aliased && f.slot_aliased.get(slot).copied().unwrap_or(false))
            .unwrap_or(true);
        if aliased {
            // D-37⑧：链接写（global/upvar 别名）在有 trace 时回落名路径
            // ——set_var 以别名名触发 trace 并承担写后重读语义（tclsh
            // name1=别名名）。被 trace 的帧局部在注册时已移出槽模型，
            // 此处只有链接直写需要拦。
            if !self.var_traces.is_empty() {
                return Err(value);
            }
            // Global link: write the target directly through the cached
            // canonical key (one probe; creation goes through the name
            // path — the link install seeded the variable, so the hot
            // case is always the hit).
            let key = self
                .frames
                .last()
                .and_then(|f| f.link_keys.get(slot))
                .and_then(|k| k.clone());
            if let Some(key) = key {
                if let Some(cell) = self.globals.get_mut(key.as_ref()) {
                    *cell = value;
                    return Ok(());
                }
                let name = self.frames.last().and_then(|f| {
                    f.slot_table.as_ref().and_then(|t| t.locals().get(slot).cloned())
                });
                if let Some(name) = name {
                    let _ = self.set_var(&name, value);
                    return Ok(());
                }
            }
            return Err(value);
        }
        match self.frames.last_mut().and_then(|f| f.slots.get_mut(slot)) {
            Some(cell) => {
                *cell = value;
                Ok(())
            }
            None => Err(value),
        }
    }

    /// `lappend` on a frame slot's list — the compiled-op fast path.
    /// Appends `value` to the slot's list in place when every guard the
    /// real `lappend` enforces holds: no traces, not aliased/linked, the
    /// cell holds a list rep.  `false` = take the dispatched command
    /// (creation, string-rep parse, link write-through — its exact
    /// semantics).
    pub(crate) fn lappend_slot_fast(&mut self, slot: usize, value: &Value) -> bool {
        if !self.var_traces.is_empty() {
            return false;
        }
        // 顶层 slot 单元（C2'）：ts 读改写——**仅 list rep 直推**。
        // unset/string-rep 槽一律回落名路径（边界 invalidate 后全局是
        // 最新——lappend 的 CREATE 从零建表会丢全局累积元素，while
        // llength 恒 1 死循环实证）；名路径读写全局后槽保持 unset，
        // 下一次 LoadLocal 再拉回 ✓。
        if let Some(ts) = self.top_slots.as_mut() {
            let Some(cell) = ts.values.get_mut(slot) else { return false };
            if ts.aliased.get(slot).copied().unwrap_or(false) {
                return false; // 链接退化 → 名路径（链接语义归属）
            }
            if cell.is_unset() {
                return false; // invalidate 后：全局是最新 → 名路径
            }
            if let Some(items) = cell.as_list_mut() {
                items.push(value.clone());
                return true;
            }
            return false; // string rep：名路径（严格解析错误归属）
        }
        let Some(f) = self.frames.last_mut() else {
            return false;
        };
        if f.has_aliased && f.slot_aliased.get(slot).copied().unwrap_or(false) {
            return false;
        }
        let Some(cell) = f.slots.get_mut(slot) else {
            return false;
        };
        if cell.is_unset() {
            return false; // unset: creation is the dispatched command's job
        }
        let Some(items) = cell.as_list_mut() else {
            return false; // non-list rep: the strict parse owns the errors
        };
        items.push(value.clone());
        true
    }

    /// `incr` on a frame slot, mutating the int rep in place
    /// (the slot analogue of [`Interp::incr_var_fast`]; `None` falls back
    /// to the real `incr`, which owns creation and the exact errors).
    pub(crate) fn frame_slot_incr(&mut self, slot: usize, amount: i64) -> Option<Value> {
        if !self.var_traces.is_empty() {
            return None;
        }
        if let Some(ts) = self.top_slots.as_mut() {
            let cell = ts.values.get_mut(slot)?;
            if cell.is_unset() || ts.aliased.get(slot).copied().unwrap_or(false) {
                return None;
            }
            let current = cell.as_int()?;
            let next = current.checked_add(amount)?;
            cell.set_int_rep(next);
            return Some(cell.clone());
        }
        {
            let f = self.frames.last()?;
            if f.has_aliased && f.slot_aliased.get(slot).copied().unwrap_or(false) {
                // Global link: one probe, in-place int rewrite when the
                // stored value is unshared/immediate.
                let key = f.link_keys.get(slot)?.as_ref()?;
                let cell = self.globals.get_mut(key.as_ref())?;
                let current = cell.as_int()?;
                let next = current.checked_add(amount)?;
                cell.set_int_rep(next);
                return Some(cell.clone());
            }
        }
        let cell = self.frames.last_mut()?.slots.get_mut(slot)?;
        if cell.is_unset() {
            return None;
        }
        let current = cell.as_int()?;
        // Overflow（含 bignum 现值）属于真 `incr`——cmd_incr 的
        // BigInt 精确和（D5：9.1 任意精度默认，不回绕）。
        let next = current.checked_add(amount)?;
        cell.set_int_rep(next);
        Some(cell.clone())
    }

    /// Degrade `frames[frame_idx]` back to the name-keyed store — but
    /// only when `name` actually lives in its slot table (otherwise the
    /// exceptional structure being installed coexists with the slots
    /// fine, and the frame keeps its fast paths).  Slot values migrate in
    /// slot order: params first, then `set`-discovered names in
    /// compilation order — the same sequence in which the tree-walk
    /// inserts them at runtime, so hash iteration order (and anything
    /// derived from it after a later degrade) matches too.
    pub(crate) fn degrade_frame_local(&mut self, frame_idx: usize, name: &str) {
        let base = super::util::strip_paren(name);
        let Some(f) = self.frames.get_mut(frame_idx) else { return };
        if f.slots.is_empty() || f.slot_index_of(base).is_none() {
            return;
        }
        if let Some(table) = f.slot_table.take() {
            for (i, name) in table.locals().iter().enumerate() {
                if let Some(cv) = f.slots.get_mut(i) {
                    if !cv.is_unset() {
                        let taken = std::mem::replace(cv, Value::unset());
                        f.locals.insert(name.clone(), taken);
                        if !f.locals_order.iter().any(|o| o == name.as_str()) {
                            f.locals_order.push(name.as_str().to_string());
                        }
                    }
                }
            }
        }
        f.slots.clear();
        f.slot_aliased.clear();
        f.link_keys.clear();
        f.has_aliased = false;
    }

    /// Link-only aliasing of ONE slot: move the cell's value into
    /// `locals` and mark the slot aliased, so slot ops on that name defer
    /// to the name path (which resolves the link) while every OTHER slot
    /// keeps its compiled fast path.  Used by `upvar` / `global` /
    /// `variable` — the links a hot method body re-installs per call —
    /// where the old whole-frame flush demoted every variable of the
    /// frame to name-keyed lookups.  Traces / array-ification / unset
    /// keep [`Self::degrade_frame_local`]'s wholesale flush (their
    /// storage-kind change is frame-wide by nature, and they are cold).
    pub(crate) fn degrade_frame_link(&mut self, frame_idx: usize, name: &str, link: &crate::interp::UpvarLink) {
        self.degrade_frame_link_at(frame_idx, name, None, link)
    }

    /// [`Self::degrade_frame_link`] with a precomputed slot index (the OO
    /// dispatcher's prelink knows it from the compiled body's locals
    /// table, and the frame it installs into is that same body's — the
    /// scan is skipped; a stale/foreign index falls back).
    pub(crate) fn degrade_frame_link_at(
        &mut self,
        frame_idx: usize,
        name: &str,
        slot: Option<u32>,
        link: &crate::interp::UpvarLink,
    ) {
        let base = super::util::strip_paren(name);
        let Some(f) = self.frames.get_mut(frame_idx) else { return };
        if f.slots.is_empty() {
            return;
        }
        let i = match slot {
            Some(s) if (s as usize) < f.slots.len() => {
                debug_assert_eq!(
                    f.slot_table
                        .as_ref()
                        .and_then(|t| t.locals().get(s as usize))
                        .map(|l| l.as_str()),
                    Some(base),
                    "prelink slot must name the same local"
                );
                s as usize
            }
            _ => match f.slot_index_of(base) {
                Some(i) => i,
                None => return,
            },
        };
        if !f.slots[i].is_unset() {
            let taken = std::mem::replace(&mut f.slots[i], Value::unset());
            f.locals.insert(base.to_string(), taken);
            if !f.locals_order.iter().any(|o| o == base) {
                f.locals_order.push(base.to_string());
            }
        }
        if f.slot_aliased.len() != f.slots.len() {
            f.slot_aliased.clear();
            f.slot_aliased.resize(f.slots.len(), false);
        }
        f.slot_aliased[i] = true;
        f.has_aliased = true;
        // Global links get the direct-key fast path: slot ops on this
        // name resolve through ONE globals probe with the canonical key
        // (frame/dead links keep None — their name paths handle the
        // indirection).
        if f.link_keys.len() != f.slots.len() {
            f.link_keys.clear();
            f.link_keys.resize(f.slots.len(), None);
        }
        f.link_keys[i] = match link {
            // The cached `Rc<str>` target, by refcount.
            crate::interp::UpvarLink::Global(g) => Some(std::rc::Rc::clone(g)),
            _ => None,
        };
    }

    /// Frame-local names in tclsh's listing order, read straight off the
    /// slot storage — the frame is NOT degraded (the old
    /// `degrade_frame_all` on every listing demoted each proc frame that
    /// ever asked for one to the name-keyed store for the rest of its
    /// life, for nothing: the slots plus the compiled locals table are a
    /// complete view on their own).
    pub(crate) fn list_frame_locals(&self, include_links: bool) -> Vec<String> {
        match self.var_frame_idx() {
            Some(i) => self.list_frame_locals_at(i, include_links),
            None => Vec::new(),
        }
    }

    /// [`Self::list_frame_locals`] for an arbitrary frame (`info frame`
    /// takes a level).  The order is tclsh's `AppendLocals`: the compiled
    /// locals first — in the slot table's order, parameters first, an
    /// undefined one not listed — then the frame's dynamic (hash) table,
    /// replayed through the Tcl hash mechanics over the creation log.
    /// A `global`/`upvar`/`variable` link is a hash-table entry, so with
    /// `include_links` it sorts among the dynamic names by hash — never
    /// with the compiled section.
    pub(crate) fn list_frame_locals_at(&self, idx: usize, include_links: bool) -> Vec<String> {
        let Some(frame) = self.frames.get(idx) else {
            return Vec::new();
        };
        // Any kind of upvar link is a link: `info locals` skips it
        // (tclsh `TclIsVarLink`), `info vars`/`info frame` list it.
        let linked = |name: &str| frame.upvars.contains_key(name);
        let mut names: Vec<String> = Vec::new();
        // 1. Compiled locals, in the slot table's order.  Listed while a
        //    value is attached: the slot cell, an aliased slot's
        //    name-keyed cell, an array container, or (in a non-slot
        //    frame — tree walk, degraded unit) the plain map entry.
        let compiled: &[String] = frame
            .compiled
            .as_ref()
            .map(|t| t.locals())
            .unwrap_or(&[]);
        for (i, name) in compiled.iter().enumerate() {
            if !include_links && linked(name) {
                continue;
            }
            let exists = if frame.has_aliased
                && frame.slot_aliased.get(i).copied().unwrap_or(false)
            {
                // Link-aliased slot: the cell moved to the name-keyed
                // store (`degrade_frame_link_at`); the link itself is
                // the existence (tclsh lists `global nosuch` with no
                // value at all).
                frame.locals.contains_key(name.as_str())
                    || frame.upvars.contains_key(name.as_str())
            } else {
                match frame.slots.get(i) {
                    Some(v) => !v.is_unset(),
                    // Name-keyed frame (tree walk / degraded unit): the
                    // map holds what exists, and a link exists by itself.
                    None => {
                        frame.locals.contains_key(name.as_str())
                            || frame.array_locals.contains(name.as_str())
                            || frame.upvars.contains_key(name.as_str())
                    }
                }
            };
            if exists {
                names.push(name.clone());
            }
        }
        // 2. The frame's dynamic table: replay the creation log through
        //    the Tcl hash mechanics.  Array elements live in their
        //    array's own table, never in the frame's, and compiled-local
        //    names list in the compiled section (a recreated one returns
        //    to its old place) — both stay out of the replay, which must
        //    otherwise see the same population tclsh's table held
        //    (rebuild thresholds are entry-counted).  A link with
        //    nothing to mirror is a table entry all the same (logged at
        //    link time), and the link filter applies to the replayed
        //    output, not to the replayed population.
        let is_compiled: std::collections::HashSet<&str> =
            compiled.iter().map(|s| s.as_str()).collect();
        let in_table = |k: &str| {
            frame.locals.contains_key(k) || frame.upvars.contains_key(k)
        };
        let dyn_keys: Vec<&str> = frame
            .locals_order
            .iter()
            .map(|s| s.as_str())
            .filter(|k| {
                !is_array_element_name(k) && !is_compiled.contains(k) && in_table(k)
            })
            .collect();
        for k in tclhash::tcl_order(dyn_keys) {
            if include_links || !linked(k) {
                names.push(k.to_string());
            }
        }
        names
    }

    /// [`Self::degrade_frame_local`] on the current frame (no-op at the
    /// global level, where there is nothing to degrade).
    pub(crate) fn degrade_frame_local_here(&mut self, name: &str) {
        if let Some(i) = self.var_frame_idx() {
            self.degrade_frame_local(i, name);
        }
    }

    /// Resolve the owning scope of a variable name (upvar links followed).
    fn resolve_loc(&self, name: &str) -> VarLoc {
        if let Some(gname) = Self::split_global(name) {
            // Leading `::` is absolute: eval-level bare-name aliases
            // (`variable`/`upvar` at namespace-eval level) never rename a
            // root-qualified target (`set ::x` must see the global x even
            // when a namespace body linked its bare `x`).
            return VarLoc::Global(gname);
        }
        if let Some(idx) = self.var_frame_idx() {
            let frame = &self.frames[idx];
            if let Some(link) = frame.upvars.get(name) {
                return match link {
                    UpvarLink::Global(gname) => {
                        VarLoc::Global(self.redirect_flat(gname.to_string()))
                    }
                    UpvarLink::Frame { frame_index, var_name } => {
                        VarLoc::Frame(*frame_index, var_name.clone())
                    }
                    // A dead link reads (and exists) against the alias's
                    // seeded local copy; writes are rejected in set_var.
                    UpvarLink::Dead { .. } => VarLoc::Frame(idx, name.to_string()),
                };
            }
            return VarLoc::Frame(idx, name.to_string());
        }
        // No proc variable frame: the current namespace's own table —
        // Tcl 9 resolved the bare name with TCL_NAMESPACE_ONLY whenever
        // the current namespace is not :: (tclVar.c
        // TclLookupSimpleVar), so there is no fallback to the :: table.
        // Everything that asks where a variable lives goes through
        // here, so the array commands and the trace tables follow the
        // same single rule.
        VarLoc::Global(self.redirect_flat(self.canonical_global(name)))
    }

    /// Follow an eval-level `upvar` alias (global level): the alias's flat
    /// key maps to the target's flat key.  `variable` 命令的瞬态链接
    /// （ns_transient——体帧作用域）同一重定向语义。
    fn redirect_flat(&self, key: String) -> String {
        for (k, t) in &self.flat_aliases {
            if *k == key {
                return t.clone();
            }
        }
        for (k, t) in &self.ns_transient {
            if *k == key {
                return t.clone();
            }
        }
        key
    }

    /// Whether the :: table has an entry for the bare global `key`, with
    /// a value or without.  One without a value is there from the moment
    /// a trace is put on the variable ([`Interp::note_global_entry`]) or
    /// the `variable` command declares it in `::`; tclsh keeps such a Var
    /// undefined but present.  Asked by the valueless-entry errorCode
    /// probe for bare names at the :: level: reads and `info exists` of
    /// a valueless entry still miss.
    fn global_entry_exists(&self, key: &str) -> bool {
        if self.globals.contains_key(key) {
            return true;
        }
        if key.contains('(') || key.contains("::") {
            return false;
        }
        self.phantom_globals.contains(key)
            || self
                .namespaces
                .get("::")
                .is_some_and(|i| i.variables.contains(key))
    }

    /// The :: table's key for a name that is a scalar of the global
    /// namespace and is being looked at from outside any proc — `::x`, or
    /// `x` at the top level — and `None` for every other name (a proc's
    /// local, an element, another namespace's variable).
    fn global_scalar_key<'a>(&self, name: &'a str) -> Option<&'a str> {
        if name.contains('(') || self.var_frame_idx().is_some() {
            return None;
        }
        if let Some(k) = name.strip_prefix("::") {
            if k.contains("::") {
                return None;
            }
            return Some(k);
        }
        if self.current_namespace.as_ref() == "::" {
            Some(name)
        } else {
            None
        }
    }

    /// Record an entry without a value in the :: table (see
    /// [`Self::global_entry_exists`]) for a name looked at from outside
    /// any proc.
    pub(crate) fn note_global_entry(&mut self, name: &str) {
        if let Some(key) = self.global_scalar_key(name) {
            self.note_global_key(key);
        }
    }

    /// The same for a key known to be a scalar of the :: table — the
    /// target of a proc's `global` or `upvar #0` link.
    pub(crate) fn note_global_key(&mut self, key: &str) {
        if !key.contains('(') && !key.contains("::") && !self.globals.contains_key(key) {
            self.phantom_globals.insert(key.to_string());
        }
    }

    /// Drop such an entry: the variable was unset, or its last trace went
    /// while it still had no value.
    pub(crate) fn forget_global_entry(&mut self, name: &str) {
        if let Some(key) = self.global_scalar_key(name) {
            self.phantom_globals.remove(key);
        }
    }

    /// tclsh：`global`/`upvar #0` 链接只在链接存活期间保住目标的未定义
    /// Var——链接随帧消亡时，仍无值的目标连条目一起离表（d27 p21：proc
    /// 返回后再读 `::gx` 是普通 LOOKUP miss，不是 READ）。帧弹出前对
    /// 本帧的 Global 链接调用；仍被其他存活帧链接的目标留下。
    pub(crate) fn drop_frame_global_links(&mut self, frame_idx: usize) {
        let mut keys: Vec<String> = Vec::new();
        if let Some(f) = self.frames.get(frame_idx) {
            for link in f.upvars.values() {
                if let UpvarLink::Global(g) = link {
                    // 无值才谈得上瞬态条目；不在 phantom 表的键没有要
                    // 清的东西（有值的变量本来就该留下）。
                    if !self.globals.contains_key(g.as_ref())
                        && self.phantom_globals.contains(g.as_ref())
                        && !keys.iter().any(|k| k == g.as_ref())
                    {
                        keys.push(g.to_string());
                    }
                }
            }
        }
        for k in keys {
            let still_linked = self.frames.iter().enumerate().any(|(i, f)| {
                i != frame_idx
                    && f
                        .upvars
                        .values()
                        .any(|l| matches!(l, UpvarLink::Global(g) if g.as_ref() == k.as_str()))
            });
            if !still_linked {
                self.phantom_globals.remove(&k);
            }
        }
    }

    /// Dead-link write check (tclsh 8.6.17): only WRITES through an alias
    /// whose target was destroyed error; reads/`info exists`/unset keep the
    /// pre-deletion behavior against the seeded local copy.  Returns
    /// `Some(is_element_error)` when the write must fail.
    fn dead_link_error(&self, name: &str) -> Option<bool> {
        // Every `set_var` pays this first: nothing linked (the common
        // case) means nothing can be dead — skip the paren scan AND the
        // upvar hash probe entirely.
        if let Some(frame) = self.var_frame() {
            if frame.upvars.is_empty() {
                return None;
            }
            let base = super::util::strip_paren(name);
            match frame.upvars.get(base) {
                Some(UpvarLink::Dead { array, .. }) => Some(*array),
                _ => None,
            }
        } else {
            if self.dead_flat.is_empty() {
                return None;
            }
            let key = self.canonical_global(super::util::strip_paren(name));
            if self.dead_flat.contains(&key) {
                Some(false)
            } else {
                None
            }
        }
    }

    /// Is the variable at `loc` an array?  (Authoritative registry; the
    /// empty base-key value is only an enumeration marker.)
    fn loc_is_array(&self, loc: &VarLoc) -> bool {
        match loc {
            VarLoc::Global(n) => self.array_globals.contains(n),
            VarLoc::Frame(i, n) => self
                .frames
                .get(*i)
                .map(|f| f.array_locals.contains(n))
                .unwrap_or(false),
        }
    }

    /// Does a variable slot exist at `loc` at all (scalar marker, array
    /// marker, or otherwise)?
    fn loc_base_exists(&self, loc: &VarLoc) -> bool {
        match loc {
            VarLoc::Global(n) => self.globals.contains_key(n),
            VarLoc::Frame(i, n) => self
                .frames
                .get(*i)
                .map(|f| f.locals.contains_key(n) || f.slot_value(n).is_some())
                .unwrap_or(false),
        }
    }

    /// Get a flat key in the scope owning `loc`.
    fn loc_get(&self, loc: &VarLoc, key: &str) -> Option<&Value> {
        match loc {
            VarLoc::Global(_) => self.globals.get(key),
            VarLoc::Frame(i, _) => self.frames.get(*i).and_then(|f| f.locals.get(key)),
        }
    }

    /// Insert a flat key in the scope owning `loc`.
    fn loc_insert(&mut self, loc: &VarLoc, key: String, value: Value) {
        match loc {
            VarLoc::Global(_) => {
                self.global_insert(key, value);
            }
            VarLoc::Frame(i, _) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    frame_local_insert(f, key, value);
                }
            }
        }
    }

    /// Remove a flat key in the scope owning `loc`; returns previous presence.
    fn loc_remove(&mut self, loc: &VarLoc, key: &str) -> bool {
        match loc {
            VarLoc::Global(_) => self.global_remove(key).is_some(),
            VarLoc::Frame(i, _) => self
                .frames
                .get_mut(*i)
                .map(|f| frame_local_remove(f, key).is_some())
                .unwrap_or(false),
        }
    }

    /// Mark the variable at `loc` as an array.
    fn loc_mark_array(&mut self, loc: &VarLoc) {
        // Container generation bump (see loc_remove_array): recreation
        // counts as a container change for in-flight `array get` reads.
        self.array_generation = self.array_generation.wrapping_add(1);
        match loc {
            VarLoc::Global(n) => {
                if n != "env" && n != "tcl_platform" {
                    self.arrays_env_only = false;
                }
                self.array_globals.insert(n.clone());
                // (Re)creation starts the live element count at 0 (P5).
                self.elem_counts.insert(n.clone(), 0);
            }
            VarLoc::Frame(i, n) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    f.array_locals.insert(n.clone());
                    f.elem_counts.insert(n.clone(), 0);
                }
            }
        }
    }

    /// Adjust the array at `loc`'s live element count by `delta` (P5:
    /// `array size`/`array exists` read the count instead of filtering
    /// the scope's whole table).  A no-op when no count is tracked —
    /// the query then falls back to the scan, so an unmaintained entry
    /// degrades performance, never correctness.
    fn loc_elem_count_bump(&mut self, loc: &VarLoc, delta: i64) {
        match loc {
            VarLoc::Global(n) => {
                if let Some(c) = self.elem_counts.get_mut(n) {
                    *c = ((*c as i64) + delta).max(0) as usize;
                }
            }
            VarLoc::Frame(i, n) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    if let Some(c) = f.elem_counts.get_mut(n) {
                        *c = ((*c as i64) + delta).max(0) as usize;
                    }
                }
            }
        }
    }

    /// Remove an entire array: all element keys, the array marker, and the
    /// base key (tclsh: `unset arr` kills the array itself).
    fn loc_remove_array(&mut self, loc: &VarLoc) {
        // (generation bump at the end of this function)
        let prefix = format!("{}(", loc.base_name());
        match loc {
            VarLoc::Global(_) => {
                let keys: Vec<String> = self
                    .globals
                    .keys()
                    .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                    .cloned()
                    .collect();
                for k in keys {
                    self.globals.remove(&k);
                }
                let base = loc.base_name().to_string();
                self.array_globals.remove(&base);
                self.elem_counts.remove(&base);
                // A deleted array's default dies with it (probe: unset
                // then recreate -> no default).
                if !self.array_defaults.is_empty() {
                    self.array_defaults.remove(&stamp_key(loc));
                }
                self.arrays_env_only = self
                    .array_globals
                    .iter()
                    .all(|k| k == "env" || k == "tcl_platform");
                self.globals.remove(&base);
            }
            VarLoc::Frame(i, _) => {
                if let Some(f) = self.frames.get_mut(*i) {
                    let keys: Vec<String> = f
                        .locals
                        .keys()
                        .filter(|k| k.starts_with(&prefix) && k.ends_with(')'))
                        .cloned()
                        .collect();
                    for k in keys {
                        f.locals.remove(&k);
                    }
                    let base = loc.base_name().to_string();
                    f.array_locals.remove(&base);
                    f.elem_counts.remove(&base);
                    f.locals.remove(&base);
                }
                if !self.array_defaults.is_empty() {
                    self.array_defaults.remove(&stamp_key(loc));
                }
            }
        }
        // Container generation bump: a destroyed array's snapshot entries
        // are invisible to an in-flight `array get` even when a callback
        // recreates same-named elements (tclsh holds the old Var).
        self.array_generation = self.array_generation.wrapping_add(1);
    }


    /// Resolve a variable name, following upvar links.
    /// Returns `Some(&Value)` if found.
    /// A leading `::` qualifies a variable as global regardless of the
    /// current call frame (Tcl namespace-qualified variable access).
    fn split_global(name: &str) -> Option<String> {
        let rest = name.strip_prefix("::")?;
        if rest.is_empty() {
            return None;
        }
        // tclsh 前导规则：`::` 后**整串冒号**一并吞掉（`::::z` → 全局
        // `z`——TclGetNamespaceForQualName 的 leading skip）；随后按
        // 变量面分段（split_ns_tail）：分隔 = `::` 串、尾段单冒号属
        // 名字（`::a::b:` → ns ::a 的 `b:`）、尾 `::` 串 = 空名变量
        // （`::t4::` ≡ `::t4:::` → t4 的空名——双端实测钉形）。旧
        // normalise+ends_with 修补对混合形（`::a::b:`）把空名错安到
        // `::a::b` 上。
        let rest = rest.trim_start_matches(':');
        let (quals, tail) = super::commands::namespace::split_ns_tail(rest);
        Some(super::commands::namespace::join_var_key(quals, tail))
    }

    /// Canonical key for a variable name reached at the global frame
    /// level: a name (plain or relative-qualified) inside a namespace is
    /// that namespace's variable (`set x` in `namespace eval n` writes
    /// `n::x`, visible as `set n::x` — namespace-old-5.5); colon runs
    /// collapse.  Plain names concatenate verbatim (a single `:` is a
    /// name character — var-1.14); names at the global level pass
    /// through.
    pub(crate) fn canonical_global(&self, name: &str) -> String {
        self.canonical_global_in(&self.current_namespace.clone(), name)
    }

    /// [`Self::split_global`] for callers outside `vars.rs` (`global`'s
    /// link targets follow the same normalization as `set`).
    pub(crate) fn global_key(name: &str) -> Option<String> {
        Self::split_global(name)
    }

    /// [`Interp::canonical_global`] against an explicit namespace (the
    /// caller's context — a proc's own `current_namespace` is its
    /// definition namespace, not the `namespace eval` it was called
    /// from).
    pub(crate) fn canonical_global_in(&self, ns: &str, name: &str) -> String {
        if ns == "::" {
            return name.to_string();
        }
        let qualified = if name.contains("::") {
            // tclsh 分段结构拼接：quals 并入 ns 路径、tail 逐字（尾 `::`
            // 串 = 空名变量——`set q::` 在 ns w7 建的是 w7::q 的空名，
            // 键 `w7::q::`；qualify 的整体 normalise 会把尾段吞成
            // `w7::q`）。
            let (quals, tail) = super::commands::namespace::split_ns_tail(name);
            let ns_path = if name.starts_with("::") {
                super::commands::namespace::normalise(quals)
            } else if quals.is_empty() {
                ns.to_string()
            } else {
                super::commands::namespace::qualify(ns, quals)
            };
            let base = ns_path.strip_prefix("::").unwrap_or(&ns_path);
            if base.is_empty() {
                tail.to_string()
            } else {
                format!("{}::{}", base, tail)
            }
        } else {
            format!("{}::{}", ns, name)
        };
        qualified.strip_prefix("::").unwrap_or(&qualified).to_string()
    }

    fn resolve_var(&self, name: &str) -> Option<&Value> {
        if let Some(gname) = Self::split_global(name) {
            return self.globals.get(gname.as_str());
        }
        if let Some(frame) = self.var_frame() {
            if let Some(link) = frame.upvars.get(name) {
                return match link {
                    // A `variable`-linked name whose namespace variable was
                    // deleted mid-proc still reads the local copy the
                    // `variable` command seeded (tclsh: the linked Var is
                    // refcounted; 46.8 `info exist x` → 1 after
                    // `namespace delete [namespace current]`).
                    UpvarLink::Global(gname) => self
                        .globals
                        .get(self.redirect_flat(gname.to_string()).as_str())
                        .or_else(|| frame.locals.get(name)),
                    UpvarLink::Frame { frame_index, var_name } => {
                        // Link installation degrades the target frame, so
                        // the map probe is the live path; the slot consult
                        // is defense in depth.  An ELEMENT link's miss is
                        // answered by the host array's default — but only
                        // on reads (get_var); var_exists must stay 0 (the
                        // default never materializes), so the fallback
                        // lives in get_var's tail, not here.
                        self.frames.get(*frame_index)
                            .and_then(|f| f.slot_value(var_name.as_str())
                                .or_else(|| f.locals.get(var_name.as_str())))
                    }
                    UpvarLink::Dead { target, was_global, .. } => {
                        // 全局死链回退 globals（目标已 unset 的 `variable`
                        // 链读穿全局）；帧局部死链**不得**回退——占位元素
                        // 宿主已亡，同名全局元素不属于这条链（trv_p11
                        // ③④：帧占位 arr(1) vs 全局 arr(1)=2）。
                        if *was_global {
                            self.globals
                                .get(target.as_str())
                                .or_else(|| frame.locals.get(name))
                        } else {
                            frame.locals.get(name)
                        }
                    }
                };
            }
            // Slotted name: the slot is the canonical store (unset slots
            // fall through — the map never holds a table name).
            // 相对 ns 限定名(n::v,非 :: 开头)不属于 proc 局部——
            // tclsh 按全局解析(deep::r2 内 $n::v 读 ::n::v 钉形);
            // 字面 frame 查找不适用,variable 链接已由 upvars 臂处理。
            if name.contains("::") {
                let key = self.redirect_flat(name.to_string());
                return self.globals.get(key.as_str());
            }
            frame
                .slot_value(name)
                .or_else(|| frame.locals.get(name))
        } else {
            // Outside procs an unqualified name resolves ONLY in the
            // current namespace's own table (Tcl 9: TclLookupSimpleVar
            // passes TCL_NAMESPACE_ONLY whenever the current namespace
            // is not :: — the :: table is no longer on the chain).  A
            // valueless `variable` declaration reads as absent, and a
            // same-named global is invisible from here (S7: reading a
            // global-only name inside a namespace body errors).
            let key = self.redirect_flat(self.canonical_global(name));
            self.globals.get(key.as_str())
        }
    }

    /// Is `name` (plain, no `::`) declared by the current namespace's
    /// `variable` command?
    fn ns_var_declared(&self, name: &str) -> bool {
        let key = format!("{}::{}", &self.current_namespace[2..], name);
        self.namespaces
            .get(self.current_namespace.as_ref())
            .map(|info| info.variables.contains(&key))
            .unwrap_or(false)
    }

    /// Set a variable in the current scope, following upvar links.
    pub(crate) fn store_var(&mut self, name: &str, value: Value) {
        if let Some(gname) = Self::split_global(name) {
            self.global_insert(gname, value);
            return;
        }
        if self.var_frame_idx().is_none() {
            // Bare names in a namespace eval body write the current
            // namespace's own table (Tcl 9: no fallback to :: — an
            // existing global of the same name is not written; S1
            // `set g 2` in a body creates ::n::g).  A new name is
            // created there too (`variable y` + `set y 9` → ::n::y).
            let key = self.redirect_flat(self.canonical_global(name));
            self.global_insert(key, value);
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    // A link write lands on the SAME global every time:
                    // probe in place first (one hash, no key churn) — the
                    // unconditional insert dropped and re-stored the key
                    // String per write.
                    let key = self.redirect_flat(gname.to_string());
                    if let Some(slot) = self.globals.get_mut(&key) {
                        *slot = value;
                    } else {
                        self.global_insert(key, value);
                    }
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        frame_local_insert(f, var_name, value);
                    }
                }
                // Writes through dead links error in set_var before
                // reaching here; keep the write local for safety.
                UpvarLink::Dead { .. } => {
                    frame_local_insert(
                        &mut self.frames[frame_idx],
                        name.to_string(),
                        value,
                    );
                }
            }
        } else if name.contains("::") {
            // 相对 ns 限定名写全局(tclsh:proc 内 `set n::v 9` 写 ::n::v,
            // 顶层 $n::v 可见钉形);局部名不含 `::`,字面 frame 写不适用。
            let key = self.redirect_flat(name.to_string());
            self.global_insert(key, value);
        } else {
            let f = &mut self.frames[frame_idx];
            match f.slot_index_of(name) {
                // Slotted name: the slot is the canonical store.
                Some(i) => f.slots[i] = value.clone(),
                None => {
                    frame_local_insert(f, name.to_string(), value);
                }
            }
        }
    }

    /// Does `name` resolve (through upvar/flat aliases) to an ELEMENT slot
    /// rather than an array base?  A link whose target keeps the `a(e)`
    /// shape addresses a scalar element even though the written name has
    /// no parentheses (set-old-8.38.3: `upvar 0 a(e) x; array set x {}`
    /// → "variable isn't array").
    pub(crate) fn resolves_to_element(&self, name: &str) -> bool {
        if name.contains('(') {
            return true;
        }
        if let Some(frame) = self.var_frame() {
            match frame.upvars.get(name) {
                Some(UpvarLink::Global(g)) => g.contains('('),
                _ => false,
            }
        } else {
            self.flat_aliases
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, t)| t.contains('('))
                .unwrap_or(false)
        }
    }

    /// Drop the seeded mirror copies for every frame's alias to a scalar
    /// global that was just unset, KEEPING the links (set-old-7.7..7.9
    /// probed semantics: after the target's deletion a `global` link
    /// dangles — reads/`info exists` see nothing (7.7), but a WRITE
    /// through the still-live link recreates the global, visible outside
    /// (7.8/7.9) — so only the stale seed must go, not the link).
    fn drop_global_seeds(&mut self, key: &str) {
        for f in self.frames.iter_mut() {
            let dead: Vec<String> = f
                .upvars
                .iter()
                .filter_map(|(n, l)| match l {
                    UpvarLink::Global(g) => {
                        // Mirror resolve_loc's redirect so aliased flat
                        // keys (global-level `upvar`) compare equal too.
                        let flat = self
                            .flat_aliases
                            .iter()
                            .find(|(k, _)| k.as_str() == &**g)
                            .map(|(_, t)| t.as_str())
                            .unwrap_or(&**g);
                        (flat == key).then(|| n.to_string())
                    }
                    _ => None,
                })
                .collect();
            for n in dead {
                f.locals.remove(&n);
                f.array_locals.remove(&n);
            }
        }
    }

    /// Remove a variable from the current scope, following upvar links.
    fn remove_var(&mut self, name: &str) {
        if let Some(gname) = Self::split_global(name) {
            self.globals.remove(gname.as_str());
            return;
        }
        if self.var_frame_idx().is_none() {
            let key = self.redirect_flat(self.canonical_global(name));
            self.globals.remove(key.as_str());
            // Unsetting also forgets the `variable` declaration (tclsh:
            // `namespace which -variable` afterwards is empty).  Tcl 9:
            // a later read does NOT fall through to a same-named global
            // — the name is simply absent from the namespace's table.
            if let Some(info) = self.namespaces.get_mut(self.current_namespace.as_ref()) {
                info.variables.remove(&key);
            }
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    let gname = self.redirect_flat(gname.to_string());
                    self.globals.remove(gname.as_str());
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.remove(&var_name);
                    }
                }
                // Unset through a dead alias keeps its pre-deletion
                // behavior (removes the seeded local copy).
                UpvarLink::Dead { .. } => {
                    self.frames[frame_idx].locals.remove(name);
                }
            }
        } else {
            let f = &mut self.frames[frame_idx];
            match f.slot_index_of(name) {
                // Unset of a slotted name empties the cell (tclsh: the
                // compiledLocal's Var is cleared, the table entry stays).
                Some(i) => f.slots[i] = Value::unset(),
                None => {
                    f.locals.remove(name);
                }
            }
        }
    }

    // ── public API ─────────────────────────────────────────────

    pub fn get_var(&self, name: &str) -> Result<&Value> {
        crate::interp::dbgtrace::event("get_var", name);
        // 顶层 slot 单元（C2'）：ts 分支——标量直读 / 数组-标量归属 /
        // ::-资格读（顶层槽 = 全局，::leaf 同名可见）。
        if let Some(ts) = &self.top_slots {
            if !has_ns_sep(name) {
                if let Some((base, _)) = split_array_ref(name) {
                    // 数组读：base 在 ts 且 set（标量）→ is-array 错误。
                    if let Some(i) = ts.names.iter().position(|n| n == base) {
                        if !ts.aliased[i] && !ts.values[i].is_unset() {
                            return Err(Error::runtime(
                                format!("can't read \"{}\": variable isn't array", name),
                                crate::error::ErrorCode::Generic,
                            ));
                        }
                    }
                } else if !name.contains('(') {
                    // 标量槽名：直读（unset → 落全局链）。
                    if let Some(i) = ts.names.iter().position(|n| n == name) {
                        if !ts.aliased[i] {
                            let v = &ts.values[i];
                            if !v.is_unset() {
                                return Ok(v);
                            }
                        }
                    }
                }
            } else {
                // ::-资格读：仅**根限定**（::leaf——leaf 与槽同名 = 同一
                // 全局变量）→ ts 值。非根 ns::leaf 是别的变量，leaf 撞名
                // 不得劫持（nr4 P1 实证：$d7::s 读成槽 s 的值 1）。
                if let Some(leaf) = name
                    .strip_prefix("::")
                    .filter(|rest| !rest.contains("::") && !rest.contains('('))
                {
                    if let Some(i) = ts.names.iter().position(|n| n == leaf) {
                        if !ts.aliased[i] {
                            let v = &ts.values[i];
                            if !v.is_unset() {
                                return Ok(v);
                            }
                        }
                    }
                }
            }
        }
        // T4 vars：根限定单叶 `::leaf`（无内层 `::`、无 `(`）——任何语
        // 境直读 :: 表（split_global 臂，无重定向；限定名父 ns 检查对
        // 单叶是空转）。命中即读；miss 与数组标记都留给下方全链
        // （valueless 条目的 errorCode、is-array 文本、父 ns 检查归
        // 它）。ts 分支在其上（槽叶优先，语义原样）。
        if let Some(leaf) = name
            .strip_prefix("::")
            .filter(|rest| !rest.contains("::") && !rest.contains('('))
        {
            if !self.array_globals.is_empty() && self.array_globals.contains(leaf) {
                // 数组语义（is-array 错误文本归属全链）。
            } else if let Some(v) = self.globals.get(leaf) {
                return Ok(v);
            }
        }
        // 限定名父 ns 不存在 → 变量不可见（D-2：ns delete——含体中自
        // 删——后的帧视图对限定访问不可见，sd1-B/sd3 实证；裸名
        // keep-alive 链接不受影响）。
        if name.contains("::") {
            // 宿主 ns 由 tclsh 分段给出（`::t4:::` 的宿主是 ::t4——尾
            // 串冒号 = 空名变量；旧 rsplit_once 把宿主算成 `::t4:` 恒
            // miss——读在查表前就报 no such variable，trv_p11 ②根因）。
            // 相对限定名的解析根是全局(tclsh:deep::r2 内 `$n::v` 读
            // ::n::v 钉形)——不拼 current ns(ns 语境裸名的 canonical
            // 在 resolve_var/store_var 的非 frame 分支处理)。
            let key = name.strip_prefix("::").unwrap_or(name);
            let (quals, _) = super::commands::namespace::split_ns_tail(key);
            if !quals.is_empty() {
                let containing = super::commands::namespace::normalise(quals);
                if containing != "::" && !self.namespaces.contains_key(&containing) {
                    return Err(Error::runtime(
                        format!("can't read \"{}\": no such variable", name),
                        ErrorCode::NotFound,
                    ));
                }
            }
        }
        if let Some((array_name, index)) = split_array_ref(name) {
            // Fast path (T7 vars): a plain base+index whose flat element
            // key IS the name as spelled — a frame array-local, or a
            // global array at "::" — with nothing that reroutes
            // resolution (dead marks, flat/ns aliases, ts slots are all
            // handled above or guarded).  One hash probe, zero
            // allocation; every guard miss keeps the full resolution
            // below (same errors, same traces).
            if self.dead_flat.iter().all(|k| k != array_name)
                && self.flat_aliases.iter().all(|(k, _)| k != array_name)
                && self.ns_transient.iter().all(|(k, _)| k != array_name)
            {
                if let Some(frame) = self.var_frame() {
                    if frame.array_locals.contains(array_name)
                        && (frame.upvars.is_empty()
                            || !frame.upvars.contains_key(array_name))
                    {
                        // array default（TIP 508）：元素**缺**且宿主数
                        // 组带默认值 -> 返回默认（不物化：info exists/
                        // array size 不动；append/incr/lappend 的读同获
                        // 默认）。已有元素永远优先。
                        if let Some(v) = frame.locals.get(name) {
                            return Ok(v);
                        }
                        if let Some(dv) = self.array_default_ref(array_name) {
                            return Ok(dv);
                        }
                        return Err(Error::runtime(
                            format!(
                                "can't read \"{}\": no such element in array",
                                name
                            ),
                            ErrorCode::NotFound,
                        ));
                    }
                } else if self.current_namespace.as_ref() == "::"
                    && self.array_globals.contains(array_name)
                {
                    if let Some(v) = self.globals.get(name) {
                        return Ok(v);
                    }
                    if let Some(dv) = self.array_default_ref(array_name) {
                        return Ok(dv);
                    }
                    return Err(Error::runtime(
                        format!("can't read \"{}\": no such element in array", name),
                        ErrorCode::NotFound,
                    ));
                } else if self.current_namespace.as_ref() != "::"
                    && !has_ns_sep(array_name)
                    && self.flat_aliases.is_empty()
                    && self.ns_transient.is_empty()
                {
                    // T4 vars：ns 体平键数组读——宿主键与元素键栈上拼
                    // （与 resolve_loc/element_key 逐字同）。宿主数组在
                    // ns 表才进：miss = 元素缺（宿主已确证数组，全链同
                    // 错文本）；宿主缺席时全链的 alt 判定（全局数组可
                    // 见性——S8 已收敛为 ns-only）原样保留。
                    let mut buf = [0u8; 96];
                    if let Some((host, elem)) =
                        stack_ns_elem_keys(self.current_namespace.as_ref(), array_name, index, &mut buf)
                    {
                        // 宿主不是死链别名（deaden 后的裸名读不得穿到旧
                        // 目标——T7 守卫同款，对 canonical 键再查一遍）。
                        if self.dead_flat.iter().all(|k| k != host)
                            && self.array_globals.contains(host)
                        {
                            if let Some(v) = self.globals.get(elem) {
                                return Ok(v);
                            }
                            if let Some(dv) = self.array_default_ref(host) {
                                return Ok(dv);
                            }
                            return Err(Error::runtime(
                                format!(
                                    "can't read \"{}\": no such element in array",
                                    name
                                ),
                                ErrorCode::NotFound,
                            ));
                        }
                    }
                }
            }
            let loc = self.resolve_loc(array_name);
            // Tcl 9（S8 读侧收尾）：裸名数组基只落当前 ns 的表——元素
            // 读与元素写/info exists 同一落点（后两者已是 ns-only），
            // 同名全局数组在体里不可见（tclVar.c：TCL_NAMESPACE_ONLY）。
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                if self.loc_get(&loc, &key).is_none() {
                    // Element miss: the host array's default answers the
                    // read (owned ref into the defaults table — a borrow
                    // of self, same shape as the table entries).
                    if let Some(dv) = self.array_default_ref(array_name) {
                        return Ok(dv);
                    }
                    return Err(Error::runtime(
                        format!("can't read \"{}\": no such element in array", name),
                        ErrorCode::NotFound,
                    ));
                }
                return Ok(self.loc_get(&loc, &key).unwrap());
            } else if self.loc_base_exists(&loc) {
                Err(Error::runtime(
                    format!("can't read \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ))
            } else {
                Err(Error::var_not_found(name))
            }
        } else {
            // Fast path: unqualified scalar in the innermost frame (no
            // `::` qualifier, no upvar link) — the proc-local and
            // loop-variable case.  One hash probe, no allocation; the
            // slow path below keeps the full scope-chain semantics.
            // The upvar/array sidecar probes are guarded by emptiness:
            // an empty set can't contain the name, and the common frame
            // (no upvars, no local arrays) then pays probes only for
            // `locals` itself.
            if !has_ns_sep(name) {
                if let Some(frame) = self.var_frame() {
                    if frame.upvars.is_empty() || !frame.upvars.contains_key(name) {
                        // Slotted name: the slot is the canonical store (a
                        // table name is never in the map).  An empty cell is
                        // the unset state — the map probe below must not run.
                        if let Some(i) = frame.slot_index_of(name) {
                            let sv = &frame.slots[i];
                            if sv.is_unset() {
                                return Err(Error::var_undefined(name));
                            }
                            return Ok(sv);
                        }
                        if let Some(v) = frame.locals.get(name) {
                            if !frame.array_locals.is_empty()
                                && frame.array_locals.contains(name)
                            {
                                return Err(Error::runtime(
                                    format!("can't read \"{}\": variable is array", name),
                                    ErrorCode::Generic,
                                ));
                            }
                            return Ok(v);
                        }
                    }
                } else if self.current_namespace.as_ref() == "::" && self.flat_aliases.is_empty() {
                    if let Some(v) = self.globals.get(name) {
                        if !self.array_globals.is_empty() && self.array_globals.contains(name) {
                            return Err(Error::runtime(
                                format!("can't read \"{}\": variable is array", name),
                                ErrorCode::Generic,
                            ));
                        }
                        return Ok(v);
                    }
                } else if self.current_namespace.as_ref() != "::"
                    && self.flat_aliases.is_empty()
                    && self.ns_transient.is_empty()
                {
                    // T4 vars：ns 体裸名读（cmd_set/incr/append 的命令
                    // 径）——键栈上拼（与 read_var_scalar_top 同构造），
                    // 命中即读；miss 与数组标记命中都留给下方全链
                    // （valueless 条目的 errorCode、is-array 文本）。
                    let mut buf = [0u8; 64];
                    if let Some(key) =
                        stack_ns_key(self.current_namespace.as_ref(), name, &mut buf)
                    {
                        if let Some(v) = self.globals.get(key) {
                            if !self.array_globals.is_empty() && self.array_globals.contains(key)
                            {
                                return Err(Error::runtime(
                                    format!("can't read \"{}\": variable is array", name),
                                    ErrorCode::Generic,
                                ));
                            }
                            return Ok(v);
                        }
                    }
                }
            }
            let loc = self.resolve_loc(name);
            if self.loc_is_array(&loc) {
                return Err(Error::runtime(
                    format!("can't read \"{}\": variable is array", name),
                    ErrorCode::Generic,
                ));
            }
            match self.resolve_var(name) {
                Some(v) => Ok(v),
                None => {
                    // An element LINK's miss (`upvar ary(c) c`) reads the
                    // host array's default; the link target is the flat
                    // element name (var-24.15).
                    if let Some(frame) = self.var_frame() {
                        if let Some(link) = frame.upvars.get(name) {
                            let target = match link {
                                UpvarLink::Frame { var_name, .. } => {
                                    Some(var_name.to_string())
                                }
                                UpvarLink::Global(g) => Some(g.to_string()),
                                _ => None,
                            };
                            if let Some(t) = target {
                                if let Some((base, _)) = split_array_ref(&t) {
                                    if let Some(dv) = self.array_default_ref(base) {
                                        return Ok(dv);
                                    }
                                }
                            }
                        }
                    }
                    Err(self.read_error(name))
                }
            }
        }
    }

    /// The error for a read that found no value: the same message either
    /// way, another errorCode.  A variable whose entry is in its table
    /// without a value is `TCL READ VARNAME` (tclsh finds the Var); a name
    /// no table knows is `TCL LOOKUP VARNAME <name>`.
    fn read_error(&self, name: &str) -> Error {
        if self.has_valueless_entry(name) {
            Error::var_undefined(name)
        } else {
            Error::var_not_found(name)
        }
    }

    /// Does `name` have an entry in its table that holds no value — a
    /// traced variable, the target of a link, a `variable` declaration
    /// without one, a proc's compiled local before its first `set` or
    /// after `unset`?  Such an entry is found (tclsh keeps an undefined
    /// Var), which shows in the errorCode of a read or of an `unset`.
    fn has_valueless_entry(&self, name: &str) -> bool {
        if name.contains('(') {
            false
        } else if let Some(idx) = self.var_frame_idx() {
            let frame = &self.frames[idx];
            frame.is_compiled_local(name) || frame.upvars.contains_key(name)
        } else if name.contains("::") {
            // That namespace's own table.
            Self::split_global(name).is_some_and(|key| {
                key.contains("::")
                    || self.phantom_globals.contains(&key)
                    || self
                        .namespaces
                        .get("::")
                        .is_some_and(|i| i.variables.contains(&key))
            })
        } else if self.current_namespace.as_ref() == "::" {
            self.global_entry_exists(name)
        } else {
            // Inside a namespace: only that namespace's own entries
            // count (Tcl 9: the :: table is not on the chain).
            self.ns_var_declared(name)
        }
    }


    /// tclsh: a WRITE to a namespace-qualified variable requires the
    /// variable's namespace to exist (`can't set "bogus::x": parent
    /// namespace doesn't exist`, errorCode TCL LOOKUP VARNAME).  A trailing
    /// colon run names the EMPTY variable in that namespace — the namespace
    /// to check is then the qualifiers part itself (`set ::pns::` is legal,
    /// `set ::nope2::` is not — var-1.12).  Reads, `info exists`, and unset
    /// stay plain misses.
    pub(crate) fn check_parent_ns(&mut self, name: &str) -> Result<()> {
        // Fast reject: a name with no `::` anywhere cannot name a
        // namespace-scoped variable (stripping an element suffix can't
        // add one either) — the scan below is for qualified names only.
        if !super::util::has_ns_sep(name) {
            return Ok(());
        }
        let var = super::util::strip_paren(name);
        if !super::util::has_ns_sep(var) {
            return Ok(());
        }
        // 变量宿主 ns 按 tclsh 分段（split_ns_tail）：quals 为空 = 无
        // ns 部件（单冒号名 `a:b` 是当前 ns 的普通名字）；否则 = 分段
        // 的 quals 路径（尾 `::` 串 → 空名的宿主就是该 ns；尾单冒号
        // 属名字、宿主在更浅一层——旧 ends_with(':') 二分把 `::a::b:`
        // 的宿主错算成 `::a::b:`，trv_p11 ①族）。绝对名根全局、相对名
        // 沿用现基（与 qualify 现行为一致——帧内相对限定名的全局根由
        // store_var/resolve_var 承担，本预检语义不变）。
        let (quals, _) = super::commands::namespace::split_ns_tail(var);
        if quals.is_empty() {
            return Ok(());
        }
        let ns_to_check = if var.starts_with("::") {
            super::commands::namespace::normalise(quals)
        } else {
            super::commands::namespace::qualify(&self.current_namespace, quals)
        };
        if ns_to_check == "::" || self.namespaces.contains_key(&ns_to_check) {
            return Ok(());
        }
        crate::interp::commands::list::set_error_code(
            self,
            &format!("TCL LOOKUP VARNAME {}", name),
        );
        Err(Error::runtime(
            format!("can't set \"{}\": parent namespace doesn't exist", name),
            ErrorCode::Generic,
        ))
    }

    pub fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        crate::interp::dbgtrace::event("set_var", name);
        // const 守卫（9.1 VAR_CONSTANT）：先于一切路径（ts 槽写也算写）。
        if let Some(e) = self.const_write_err(name) {
            return Err(e);
        }
        // 顶层 slot 单元（C2'）：ts 分支——标量写 / 数组-标量归属 /
        // ::-leaf 写（顶层槽 = 全局）。
        if let Some(ts) = self.top_slots.as_mut() {
            if !has_ns_sep(name) {
                if let Some((base, _)) = split_array_ref(name) {
                    if let Some(i) = ts.names.iter().position(|n| n == base) {
                        if !ts.aliased[i] && !ts.values[i].is_unset() {
                            return Err(Error::runtime(
                                format!("can't set \"{}\": variable isn't array", name),
                                crate::error::ErrorCode::Generic,
                            ));
                        }
                    }
                } else if !name.contains('(') {
                    if let Some(i) = ts.names.iter().position(|n| n == name) {
                        if !ts.aliased[i] {
                            ts.values[i] = value.clone();
                            return Ok(value);
                        }
                    }
                }
            } else {
                // ::-资格写：仅**根限定**（::leaf）→ ts 写（同读臂——
                // 非根 ns::leaf 是别的变量，leaf 撞名不得劫持）。
                if let Some(leaf) = name
                    .strip_prefix("::")
                    .filter(|rest| !rest.contains("::") && !rest.contains('('))
                {
                    if let Some(i) = ts.names.iter().position(|n| n == leaf) {
                        if !ts.aliased[i] {
                            ts.values[i] = value.clone();
                            return Ok(value);
                        }
                    }
                }
            }
        }
        // A write through an alias whose target namespace/array was
        // destroyed errors (var-1.15/1.16/1.17); reads keep working.
        // 括号形分流（E1/var-1.15 钉形）：死**元素**链上的括号写 =
        // 别名被当数组名 → `variable isn't array`（NEEDARRAY 先于
        // dangling——元素 Var 已随宿主消亡、别名退化为未定义标量）；
        // 死 ns 变量链（元素/裸名）一律 dangling-namespace（Var 仍在、
        // 宿主 ns 失效——tclsh 的 DANGLINGVAR 先于数组形态判定）。
        if let Some(elem) = self.dead_link_error(name) {
            if elem && name.contains('(') {
                return Err(Error::runtime(
                    format!("can't set \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ));
            }
            let what = if elem {
                "element in deleted array"
            } else {
                "variable in deleted namespace"
            };
            return Err(Error::runtime(
                format!("can't set \"{}\": upvar refers to {}", name, what),
                ErrorCode::Generic,
            ));
        }
        self.check_parent_ns(name)?;
        if let Some((array_name, index)) = split_array_ref(name) {
            // 顶层 slot 单元（C2'）：ts 名数组化 → 槽标记 aliased（后续
            // 槽写回落名路径——`set x 7` 的 is-array 错误归属与 tclsh
            // 一致；标记漏发会让槽写静默覆盖）。（快径同做——base 命中
            // ts 名表才标，未命中零成本。）
            if let Some(ts) = self.top_slots.as_mut() {
                if let Some(i) = ts.names.iter().position(|n| n == array_name) {
                    ts.aliased[i] = true;
                }
            }
            // Fast path (T7 vars): OVERWRITE of an existing element whose
            // flat key is the name as spelled (frame array-local, or a
            // global array at "::") — one get_mut probe, zero
            // allocation.  Creation, rerouted resolution (links, dead
            // marks, aliases) and the error shapes keep the full path
            // below; degrade is a no-op here (the base already carries
            // this scope's array marker, so its slot left the frame when
            // the marker was set).
            if self.dead_flat.iter().all(|k| k != array_name)
                && self.flat_aliases.iter().all(|(k, _)| k != array_name)
                && self.ns_transient.iter().all(|(k, _)| k != array_name)
                && !has_ns_sep(name)
            {
                let mut wrote = false;
                if let Some(frame) = self.var_frame_mut() {
                    if frame.array_locals.contains(array_name)
                        && (frame.upvars.is_empty()
                            || !frame.upvars.contains_key(array_name))
                    {
                        if let Some(slot) = frame.locals.get_mut(name) {
                            *slot = value.clone();
                            wrote = true;
                        }
                    }
                } else if self.current_namespace.as_ref() == "::"
                    && self.array_globals.contains(array_name)
                {
                    if let Some(slot) = self.globals.get_mut(name) {
                        *slot = value.clone();
                        wrote = true;
                    }
                }
                if wrote {
                    // Phantom cleanup is a no-op on an empty map — skip
                    // the stamp-key allocation with it (state identical).
                    if !self.trace_phantoms.is_empty() {
                        self.clear_phantom(array_name, index);
                    }
                    // Same tail as the slow path: traces fire after the
                    // write; a fired write trace returns the variable's
                    // new value (D-36).
                    let fired = self.fire_traces(array_name, Some(index), "write").map_err(
                        |e| {
                            Error::runtime(
                                format!("can't set \"{}\": {}", name, e),
                                ErrorCode::Generic,
                            )
                        },
                    )?;
                    if fired {
                        return Ok(
                            self.get_var(name).map(|v| v.clone()).unwrap_or_default()
                        );
                    }
                    return Ok(value);
                }
            }
            // Array-ification of a slotted name migrates the frame back to
            // the name-keyed store (element keys are flat map entries; the
            // slot model has no element form).
            self.degrade_frame_local_here(array_name);
            let base = array_name.to_string();
            let idx = index.to_string();
            // 数组基解析走 resolve_loc（Tcl 9：裸名只落当前 ns 的表，
            // 不再链到全局；元素读写同键）。
            let loc = self.resolve_loc(array_name);
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                let existed = self.loc_get(&loc, &key).is_some();
                self.loc_insert(&loc, key, value.clone());
                if !existed {
                    // New element: element-set mutation invalidates searches.
                    self.bump_stamp(&loc);
                    self.loc_elem_count_bump(&loc, 1);
                }
            } else if self.loc_base_exists(&loc) {
                let base = name.split('(').next().unwrap_or(name);
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", base),
                );
                return Err(Error::runtime(
                    format!("can't set \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ));
            } else {
                // Create the array: marker + base key (for enumeration and
                // `info exists`) + the first element.
                let base_key = loc.base_name().to_string();
                self.loc_mark_array(&loc);
                self.loc_insert(&loc, base_key, Value::empty());
                let key = loc.element_key(index);
                self.loc_insert(&loc, key, value.clone());
                self.loc_elem_count_bump(&loc, 1);
            }
            self.clear_phantom(&base, &idx);
            // Write traces fire after the write lands; a trace error is
            // reported as the set's failure (trace-0.0) but the write
            // itself persists.
            self.fire_traces(&base, Some(&idx), "write")
                .map_err(|e| {
                    Error::runtime(format!("can't set \"{}\": {}", name, e), ErrorCode::Generic)
                })?;
        } else {
            // Fast path: unqualified scalar write hitting an existing
            // variable in the innermost frame (or the global table at
            // "::" top level) — no upvar/dead link, not an array.  One
            // get_mut probe, zero allocations; creation, links, arrays
            // and qualified names keep the slow path below (same errors,
            // same trace firing).
            if !has_ns_sep(name) && self.dead_flat.is_empty() {
                let mut wrote = false;
                if let Some(frame) = self.var_frame_mut() {
                    if (frame.upvars.is_empty() || !frame.upvars.contains_key(name))
                        && (frame.array_locals.is_empty() || !frame.array_locals.contains(name))
                    {
                        match frame.slot_index_of(name) {
                            // Slotted name: write the cell (creation of an
                            // unset slot reaches here via the slow path's
                            // store_var, which consults the same table).
                            Some(i) => {
                                frame.slots[i] = value.clone();
                                wrote = true;
                            }
                            None => {
                                if let Some(slot) = frame.locals.get_mut(name) {
                                    *slot = value.clone();
                                    wrote = true;
                                }
                            }
                        }
                    }
                } else if self.current_namespace.as_ref() == "::"
                    && self.flat_aliases.is_empty()
                    && (self.arrays_env_only || !self.array_globals.contains(name))
                {
                    if let Some(slot) = self.globals.get_mut(name) {
                        *slot = value.clone();
                        wrote = true;
                    }
                } else if self.current_namespace.as_ref() != "::"
                    && self.flat_aliases.is_empty()
                    && self.ns_transient.is_empty()
                {
                    // T4 vars：ns 体裸名覆写（命令径）——键栈上拼，键已
                    // 存在才原地写；新建（含 variable 声明无值目标）落
                    // 点归下方全链（store_var 的 9.1 落点）。
                    let mut buf = [0u8; 64];
                    if let Some(key) =
                        stack_ns_key(self.current_namespace.as_ref(), name, &mut buf)
                    {
                        if !self.array_globals.is_empty() && self.array_globals.contains(key) {
                            // 数组语义（is-array 错误文本归属全链）。
                        } else if let Some(slot) = self.globals.get_mut(key) {
                            *slot = value.clone();
                            wrote = true;
                        }
                    }
                }
                if wrote {
                    if !self.var_traces.is_empty() {
                        let fired = self.fire_traces(name, None, "write").map_err(|e| {
                            Error::runtime(format!("can't set \"{}\": {}", name, e), ErrorCode::Generic)
                        })?;
                        // tclsh：write trace 改写后返回改写值（D-36）。
                        if fired {
                            return Ok(self.get_var(name).map(|v| v.clone()).unwrap_or_default());
                        }
                    }
                    return Ok(value);
                }
            }
            // Tcl 9：裸名写只落当前 ns 的表（无全局链；同名全局不写、
            // 其上的写 trace 也不触发——S9/S14）。落点由 resolve_loc 决定。
            let loc = self.resolve_loc(name);
            // 链接/别名目标为元素形式（深水 DIFF-6：upvar 0 ::a(e) x 的
            // 链接写原按裸键直插，数组标记/元素路由全失）——以限定元素
            // 名重入，走数组元素写全链，与直接 `set ::a(e)` 同形。良构
            // 守卫：须以 ) 收尾（set-old-4.5 的 `a(bcd` 半开名是标量，
            // 递归即无穷）。
            if let VarLoc::Global(k) = &loc {
                if k.ends_with(')') && k.contains('(') {
                    let q = format!("::{}", k);
                    return self.set_var(&q, value);
                }
            }
            if self.loc_is_array(&loc) {
                return Err(Error::runtime(
                    format!("can't set \"{}\": variable is array", name),
                    ErrorCode::Generic,
                ));
            }
            self.store_var(name, value.clone());
            let fired = self.fire_traces(name, None, "write").map_err(|e| {
                Error::runtime(format!("can't set \"{}\": {}", name, e), ErrorCode::Generic)
            })?;
            // tclsh：write trace 改写变量后 set 返回改写后的值（trace
            // 落地后重读；trace 若删了变量则回空串）。
            return Ok(if fired {
                self.get_var(name).map(|v| v.clone()).unwrap_or_default()
            } else {
                value
            });
        }
        Ok(value)
    }

    /// The slot a change to the scalar `name` writes, for making the
    /// change in place (tclsh: mutate the refcnt==1 object in the
    /// variable's own table entry).  Returns `None` unless every guard
    /// for the mutation fast path holds: unqualified scalar name, no
    /// upvar/dead link, not an array, no variable traces registered
    /// anywhere, and the variable currently exists.  Nothing is taken out
    /// of a table — a value lifted out and put back used to leave a
    /// second entry in the scope's insertion log, and a listing replays
    /// a second entry as a variable made anew: it jumped to the head of
    /// its bucket.
    ///
    /// The guards matter for more than speed: with a trace registered the
    /// write has to fire it, and the callers fall back to the commands'
    /// own paths, which go through `set_var`.
    pub(crate) fn var_slot_fast(&mut self, name: &str) -> Option<&mut Value> {
        if has_ns_sep(name)
            || name.contains('(')
            || !self.var_traces.is_empty()
            || !self.dead_flat.is_empty()
        {
            return None;
        }
        // 顶层 slot 单元（C2'）：名路径——ts 权威。
        if let Some(ts) = self.top_slots.as_mut() {
            if self.frames.is_empty() && self.current_namespace.as_ref() == "::" {
                if let Some(i) = ts.names.iter().position(|n| n == name) {
                    if ts.aliased[i] {
                        return None;
                    }
                    let cell = &mut ts.values[i];
                    return if cell.is_unset() { None } else { Some(cell) };
                }
                // 非槽名 → 落下方全局路径
            }
        }
        // The predicate over the two fields, not [`Self::var_frame_idx`]:
        // this function returns a reference into one table or the other,
        // and a method call on the whole interpreter does not borrow that
        // way.
        let frame = scope_frame_idx(&self.frames, self.ns_stack.len())
            .and_then(|i| self.frames.get_mut(i));
        if let Some(frame) = frame {
            if !frame.upvars.is_empty() && frame.upvars.contains_key(name) {
                return None;
            }
            if !frame.array_locals.is_empty() && frame.array_locals.contains(name) {
                return None;
            }
            match frame.slot_index_of(name) {
                // Slotted name: the slot is the canonical store; an unset
                // cell is the undefined variable.
                Some(i) => {
                    let cell = frame.slots.get_mut(i)?;
                    if cell.is_unset() {
                        return None;
                    }
                    Some(cell)
                }
                None => frame.locals.get_mut(name),
            }
        } else {
            if !self.flat_aliases.is_empty() || !self.ns_transient.is_empty() {
                return None;
            }
            if self.current_namespace.as_ref() == "::" {
                if !self.arrays_env_only && self.array_globals.contains(name) {
                    return None;
                }
                return self.globals.get_mut(name);
            }
            // T4 vars：ns 体裸名（incr 热径）——键栈上拼，与 scalar
            // 直写守卫同一构造；数组语义与超长回落真 incr（其建名、
            // 非整数与溢出错误文本归它）。
            let mut buf = [0u8; 64];
            let key = stack_ns_key(self.current_namespace.as_ref(), name, &mut buf)?;
            if !self.arrays_env_only && self.array_globals.contains(key) {
                return None;
            }
            self.globals.get_mut(key)
        }
    }

    /// [`Self::var_slot_fast`] where plain frame-local travel won't do:
    /// the proc-level `variable` link (`UpvarLink::Global` — the alias a
    /// `variable` declaration installs to the owning namespace's
    /// variable), qualified names (`::m …`), and bare/relative-qualified
    /// names at eval level under the namespace's canonical write key.
    /// Same guards; nothing is taken out of a table (see
    /// [`Self::var_slot_fast`]).
    ///
    /// The store keys here mirror `store_var`/`resolve_loc` exactly
    /// (`redirect_flat` over the split/canonical forms).
    pub(crate) fn var_slot_fast_link(&mut self, name: &str) -> Option<&mut Value> {
        if name.contains('(')
            || !self.var_traces.is_empty()
            || !self.dead_flat.is_empty()
        {
            return None;
        }

        // (0) Leading-`::` names are frame-independent: even inside a
        //     proc they address the global table (resolve_loc's
        //     split_global arm short-circuits the frame walk).
        if let Some(gname) = Self::split_global(name) {
            let key = gname;
            if (!self.arrays_env_only && self.array_globals.contains(key.as_str())) {
                return None;
            }
            return self.globals.get_mut(key.as_str());
        }

        // (1) Proc level: only the `variable` command's namespace link.
        //     Unlinked proc locals are handled by var_slot_fast.
        if let Some(frame) = self.var_frame() {
            let target = match frame.upvars.get(name) {
                Some(UpvarLink::Global(g)) => g.to_string(),
                _ => return None,
            };
            // The link target is the owning namespace's qualified name;
            // keep the eval-level flat-alias redirect in step with
            // resolve_var's read path.
            let key = self.redirect_flat(target);
            if (!self.arrays_env_only && self.array_globals.contains(key.as_str())) {
                return None; // element/array target: scalar writes take the generic path
            }
            // Release the `variable` command's seeded local hold as well:
            // the seed shares the same Value rep as the global, and
            // leaving it in place pins the old rep so every in-place
            // write COW-copies the whole dict (the O(n)-per-op hotspot in
            // hot `variable`+`dict set` loops).  Reads refresh from the
            // global (resolve_var consults the global first); the seed
            // only keeps a mid-proc-deleted namespace variable readable.
            if let Some(frame) = self.var_frame_mut() {
                frame.locals.remove(name);
            }
            return self.globals.get_mut(key.as_str());
        }

        // (2) Eval level: qualified (`::m`), relative-qualified (`sub::m`)
        //     and bare names all canonicalize to the namespace's
        //     canonical WRITE key (store_var's eval arm), through the
        //     flat-alias redirect exactly as resolve_loc computes it.
        let key = self.redirect_flat(self.canonical_global(name));
        if (!self.arrays_env_only && self.array_globals.contains(key.as_str())) {
            return None;
        }
        self.globals.get_mut(key.as_str())
    }

    /// Run `f` on the slot `dict set` changes in place — an ordinary
    /// local or global, else through a `variable` link or a qualified
    /// name (see [`Self::var_slot_fast`] and [`Self::var_slot_fast_link`])
    /// — and return what it returned; `None` when no guard let the change
    /// in place, or when `f` declined (it returns `None`).
    pub(crate) fn with_dict_slot<R>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Value) -> Option<R>,
    ) -> Option<R> {
        let slot = self.var_slot_fast(name);
        let slot = match slot {
            Some(v) => v,
            None => self.var_slot_fast_link(name)?,
        };
        f(slot)
    }

    /// The live keys of the :: table in Tcl hash-table order — bare
    /// globals and namespace variables (`n::x`), array elements left out:
    /// what `info globals` and `info vars` list from, and what the
    /// element keys of an array ride along with.  The insertion log is
    /// replayed keeping a key's LAST occurrence: in a Tcl hash table a
    /// variable made again after a delete takes a new place (the head of
    /// its bucket), unlike a plain re-set, which never touches the log.
    pub(crate) fn global_var_order(&self) -> Vec<&str> {
        let log = &self.var_order;
        // An element key `n::a(k)` or `a(k)`: the array's name has a
        // parenthesised tail.
        let not_element = |k: &str| {
            !k.rsplit("::").next().is_some_and(|last| last.contains('('))
        };
        if log.len() == self.globals.len() {
            // No stale entry, so no duplicate either: the log is the table
            // in insertion order, and no key needs a lookup.
            return log
                .iter()
                .map(String::as_str)
                .filter(|k| not_element(k))
                .collect();
        }
        let mut seen: std::collections::HashSet<&str> =
            std::collections::HashSet::with_capacity(self.globals.len());
        let mut order: Vec<&str> = Vec::with_capacity(self.globals.len());
        for k in log.iter().rev() {
            let k = k.as_str();
            if not_element(k) && self.globals.contains_key(k) && seen.insert(k) {
                order.push(k);
            }
        }
        order.reverse();
        order
    }

    /// `incr` fast path: read-modify-write an integer scalar in one slot
    /// lookup, mutating the stored value in place when it is unshared
    /// (tclsh: TclIncrObj on a refcnt==1 object — no allocation, no string
    /// rendering).  `None` whenever any guard fails — the caller falls
    /// back to the real `incr`, which owns the exact errors (undefined
    /// variable, non-integer value, overflow), creation, and trace
    /// semantics.
    pub(crate) fn incr_var_fast(&mut self, name: &str, amount: i64) -> Option<Value> {
        let r = self.incr_var_fast_impl(name, amount);
        if r.is_some() {
            crate::interp::dbgtrace::event("fast_ok", "");
        } else {
            crate::interp::dbgtrace::event("fast_none", name);
        }
        r
    }

    /// 语句窗口四 op 的执行体（exec_op 的单一分派臂转入）。整段实现
    /// 屈居 vars.rs 而非 vm_exec——后者是热径分派单元，代码增量会挪动
    /// LLVM 的内联预算（micro 矩阵 +2-5% 指令数漂移的定位与修复，见
    /// BASELINE 深水区节）。VmState 的 stack/bodies/cur_site 已
    /// pub(crate) 化以支撑此布局。
    pub(crate) fn exec_window_stmt(
        &mut self,
        code: &rtcl_parser::ByteCode,
        st: &mut super::vm_exec::VmState,
        op: &rtcl_parser::OpCode,
    ) -> Result<Value> {
        use rtcl_parser::OpCode::*;
        // SubEnd 前置（融合形）：关括号区域并还原属主命令现场——
        // op_sub_end 的逐字（vars.rs 布局下 st 字段已可见）。
        if matches!(op, SubEndIncrVarStack(_) | SubEndIncrLocalStack(_)) {
            if let Some(region) = st.bodies.pop() {
                st.cur_site = region.site();
            }
        }
        let amount = st.stack.pop().unwrap_or_else(Value::empty);
        let v = match op {
            IncrVarStack(idx) => {
                let name = code.get_const(*idx).unwrap_or("");
                self.incr_var_fast_stack(name, amount)?
            }
            SubEndIncrVarStack(idx) => {
                let name = code.get_const(u32::from(*idx)).unwrap_or("");
                self.incr_var_fast_stack(name, amount)?
            }
            IncrLocalStack(slot) => {
                self.incr_local_slot_stack(*slot as usize, amount, code)?
            }
            SubEndIncrLocalStack(slot) => {
                self.incr_local_slot_stack(u32::from(*slot) as usize, amount, code)?
            }
            _ => unreachable!("exec_window_stmt"),
        };
        st.stack.push(v);
        Ok(Value::empty())
    }

    /// 语句窗口 `incr var [script]` 的名路径阶梯（`IncrVarStack` op 体
    /// 屈居 vars.rs 而非 vm_exec——vm_exec 是热径分派单元，代码增量会
    /// 挪动 LLVM 内联预算，micro 矩阵实测 +2-5% 指令数）。快径 = 小整
    /// 数量 + `incr_var_fast` 守卫集；其余一律委托真 `incr`——创建、
    /// bignum 提升、trace、`(reading increment)` 帧与精确错误全归它，
    /// 与 DynCall 形的分派逐字同。
    pub(crate) fn incr_var_fast_stack(&mut self, name: &str, amount: Value) -> Result<Value> {
        if let Some(k) = amount.as_int() {
            if let Some(v) = self.incr_var_fast(name, k) {
                return Ok(v);
            }
        }
        // dispatch 的 builtin 臂先复位（fresh error state）；回落同款。
        self.err_code_raised = false;
        let args = [Value::from_str("incr"), Value::from_str(name), amount];
        super::commands::misc::cmd_incr(self, &args)
    }

    /// [`Self::incr_var_fast_stack`] 的槽孪生（`IncrLocalStack` op 体）：
    /// 同 `IncrLocal` 的回落阶梯（槽快径 → 降级帧名快径 → 真 `incr`），
    /// 增量取自栈上括号结果。
    pub(crate) fn incr_local_slot_stack(
        &mut self,
        slot: usize,
        amount_v: Value,
        code: &rtcl_parser::ByteCode,
    ) -> Result<Value> {
        let name = || code.locals().get(slot).map(String::as_str).unwrap_or("");
        if self.frame_local_is_const(slot, code) {
            self.err_code_raised = false;
            let args = [Value::from_str("incr"), Value::from_str(name()), amount_v];
            return super::commands::misc::cmd_incr(self, &args);
        }
        if let Some(k) = amount_v.as_int() {
            if let Some(v) = self.frame_slot_incr(slot, k) {
                return Ok(v);
            }
            if let Some(v) = self.incr_var_fast(name(), k) {
                // 名路径写全局 → ts 槽失效。
                self.top_slot_invalidate_one(slot);
                return Ok(v);
            }
        }
        self.err_code_raised = false;
        let args = [Value::from_str("incr"), Value::from_str(name()), amount_v];
        super::commands::misc::cmd_incr(self, &args)
    }

    fn incr_var_fast_impl(&mut self, name: &str, amount: i64) -> Option<Value> {
        // const 标记名走真 incr（cmd_incr 顶部的 const 守卫持有其错误形）。
        if !self.const_globals.is_empty() && self.const_globals.iter().any(|c| c == name) {
            return None;
        }
        if let Some(f) = self.frames.last() {
            if f.consts.is_some() && f.consts.as_ref().unwrap().iter().any(|c| c == name) {
                return None;
            }
        }
        // T7 vars: element fast path — existing element, flat key is the
        // name as spelled, no traces registered (the command's read is
        // then silent either way), integer value.  Everything else
        // (missing element, non-int current, overflow, links, namespaces)
        // falls back to `incr` itself, which produces the same errors
        // from the same read (cmd_incr's missing-element creation, TCL
        // VALUE INTEGER, bignum widening all replay unchanged).
        if let Some((array_name, index)) = split_array_ref(name) {
            if has_ns_sep(name)
                || !self.var_traces.is_empty()
                || self.dead_flat.iter().any(|k| k == array_name)
                || self.flat_aliases.iter().any(|(k, _)| k == array_name)
                || self.ns_transient.iter().any(|(k, _)| k == array_name)
            {
                return None;
            }
            let next;
            if let Some(frame) = self.var_frame_mut() {
                if !(frame.array_locals.contains(array_name)
                    && (frame.upvars.is_empty()
                        || !frame.upvars.contains_key(array_name)))
                {
                    return None;
                }
                let cell = frame.locals.get_mut(name)?;
                let current = cell.as_int()?;
                next = current.checked_add(amount)?;
                *cell = Value::from_int(next);
            } else {
                if self.current_namespace.as_ref() != "::"
                    || !self.array_globals.contains(array_name)
                {
                    return None;
                }
                let cell = self.globals.get_mut(name)?;
                let current = cell.as_int()?;
                next = current.checked_add(amount)?;
                *cell = Value::from_int(next);
            }
            if !self.trace_phantoms.is_empty() {
                self.clear_phantom(array_name, index);
            }
            return Some(Value::from_int(next));
        }
        let slot = self.var_slot_fast(name)?;
        let current = slot.as_int()?;
        // Overflow（含 bignum 现值）属于真 `incr`——cmd_incr 的
        // BigInt 精确和（D5：9.1 任意精度默认，不回绕）。
        let next = current.checked_add(amount)?;
        slot.set_int_rep(next);
        Some(slot.clone())
    }

    pub fn unset_var(&mut self, name: &str) -> Result<()> {
        // const 守卫（tclVar.c 2547：unset 路径的 ISCONST）。
        if let Some(e) = self.const_unset_err(name) {
            return Err(e);
        }
        if let Some((array_name, index)) = split_array_ref(name) {
            let base = array_name.to_string();
            let idx = index.to_string();
            // 数组基解析走 resolve_loc（Tcl 9：裸名只落当前 ns 的表）。
            let loc = self.resolve_loc(array_name);
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                if self.loc_remove(&loc, &key) {
                    // Removed element: mutation invalidates searches.
                    self.bump_stamp(&loc);
                    self.loc_elem_count_bump(&loc, -1);
                    self.clear_phantom(&base, &idx);
                    // Unset traces fire after deletion; errors are
                    // discarded (tclsh: the unset still succeeds).
                    let _ = self.fire_traces(&base, Some(&idx), "unset");
                    Ok(())
                } else {
                    // tclsh：对缺失元素的 unset 仍触发 unset trace（且抹
                    // 该元素的记录——trace-14.17 族，D-37⑤）。
                    let sk = stamp_key(&loc);
                    let _ = self.fire_traces(&base, Some(&idx), "unset");
                    if let Some(map) = self.elem_traces.get_mut(&sk) {
                        map.remove(&idx);
                        if map.is_empty() {
                            self.elem_traces.remove(&sk);
                        }
                    }
                    self.trace_phantoms
                        .entry(sk)
                        .or_default()
                        .remove(&idx);
                    crate::interp::commands::list::set_error_code(
                        self,
                        &format!("TCL LOOKUP ELEMENT {}", idx),
                    );
                    Err(Error::runtime(
                        format!("can't unset \"{}\": no such element in array", name),
                        ErrorCode::NotFound,
                    ))
                }
            } else if self.loc_base_exists(&loc) {
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", base),
                );
                Err(Error::runtime(
                    format!("can't unset \"{}\": variable isn't array", name),
                    ErrorCode::Generic,
                ))
            } else {
                crate::interp::commands::list::set_error_code(
                    self,
                    &format!("TCL LOOKUP VARNAME {}", base),
                );
                Err(Error::runtime(
                    format!("can't unset \"{}\": no such variable", name),
                    ErrorCode::NotFound,
                ))
            }
        } else {
            let given = name.to_string();
            // Tcl 9：裸名 unset 只看当前 ns 的表（无全局链）。
            let loc = self.resolve_loc(name);
            if self.loc_is_array(&loc) {
                // tclsh：整数组 unset 逐元素触发 {a k unset}（D-37③，
                // per-Var 销毁序——元素 trace 先于整变量 trace）。元素名
                // 必须在删除前枚举。
                let elems = self.array_element_names(&given);
                for e in elems {
                    let _ = self.fire_traces_elem_only(&given, &e, "unset");
                }
                self.loc_remove_array(&loc);
                // Element aliases into this array now error on write
                // (var-1.17: `upvar 0 arr(1) foo; unset arr; set foo(3)` →
                // "element in deleted array").
                self.deaden_element_links(&loc);
                // Whole-array unset kills its searches and stamp. Unset
                // traces fire after the deletion, so drop the tables after
                // firing them.
                let sk = stamp_key(&loc);
                self.array_searches.remove(&sk);
                self.array_stamps.remove(&sk);
                let _ = self.fire_traces(&given, None, "unset");
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                Ok(())
            } else if self.loc_base_exists(&loc) {
                self.remove_var(name);
                // set-old-7.7..7.9: removing the target retires every
                // frame's seeded mirror (stale reads) while the links
                // stay live so writes recreate the global.
                if let VarLoc::Global(k) = &loc {
                    let k = k.clone();
                    self.drop_global_seeds(&k);
                    // The entry goes with its traces (a traced variable
                    // that was never set leaves the table).
                    self.phantom_globals.remove(&k);
                }
                // The key from where the variable WAS: by name it would
                // no longer resolve there (the chain needs the variable).
                let sk = stamp_key(&loc);
                let _ = self.fire_traces_key(&sk, &given, None, "unset", true);
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                Ok(())
            } else if self.current_namespace.as_ref() != "::" && self.ns_var_declared(&given) {
                // `variable`-declared but valueless (or declared with the
                // value living in the namespace's own slot): unset removes
                // the declaration and succeeds (tclsh 8.6.17).
                self.remove_var(&given);
                Ok(())
            } else {
                // tclsh deletes a missing variable's trace records too
                // (`unset` on an untraced-existence variable still wipes
                // its registrations — trace-14.17/14.19/33.1) AND fires
                // the unset traces on the way (D-37⑤).
                // An entry without a value was found — it goes with this
                // `unset` — and tclsh's code is then the bare
                // `TCL UNSET VARNAME`; a name no table knows is
                // `TCL LOOKUP VARNAME <name>`.
                let found_entry = self.has_valueless_entry(name);
                let _ = self.fire_traces(&given, None, "unset");
                let sk = stamp_key(&loc);
                self.var_traces.remove(&sk);
                self.elem_traces.remove(&sk);
                self.trace_phantoms.remove(&sk);
                // The failed `unset` still takes the valueless entry with
                // it, traces and declaration alike: the next read or
                // `unset` of the name is a lookup failure.  By where the
                // name resolved to, not by the name: the entry may be a
                // global reached from inside a namespace body.
                if let VarLoc::Global(k) = &loc {
                    let k = k.clone();
                    self.phantom_globals.remove(&k);
                    if let Some(info) = self.namespaces.get_mut("::") {
                        info.variables.remove(&k);
                    }
                }
                let code = if found_entry {
                    "TCL UNSET VARNAME".to_string()
                } else {
                    format!("TCL LOOKUP VARNAME {}", name)
                };
                crate::interp::commands::list::set_error_code(self, &code);
                Err(Error::runtime(
                    format!("can't unset \"{}\": no such variable", name),
                    ErrorCode::NotFound,
                ))
            }
        }
    }

    pub fn var_exists(&self, name: &str) -> bool {
        // 限定名父 ns 不存在 → 不存在（D-2：同 get_var 守卫——tclsh
        // `info exists e::y` 对已删 ns 恒 0，sd1-B/C/sd2 实证）。宿主
        // ns 按 tclsh 分段（`::t4:::` 的宿主是 ::t4——旧 rsplit 把
        // verbatim 键的尾串切出 `::t4:` 恒 0，trv_p11 ①族）。
        if name.contains("::") {
            let key = self.canonical_global(name);
            let key = key.strip_prefix("::").unwrap_or(&key);
            let (quals, _) = super::commands::namespace::split_ns_tail(key);
            if !quals.is_empty() {
                let containing = super::commands::namespace::normalise(quals);
                if containing != "::" && !self.namespaces.contains_key(&containing) {
                    return false;
                }
            }
        }
        if let Some((array_name, index)) = split_array_ref(name) {
            // Fast path (T7 vars): same flat-key-is-the-name guard set as
            // get_var/set_var — one contains probe, zero allocation
            // (this sits on `incr`'s hot path via cmd_incr).
            if !has_ns_sep(name)
                && self.dead_flat.iter().all(|k| k != array_name)
                && self.flat_aliases.iter().all(|(k, _)| k != array_name)
                && self.ns_transient.iter().all(|(k, _)| k != array_name)
            {
                if let Some(frame) = self.var_frame() {
                    if frame.array_locals.contains(array_name)
                        && (frame.upvars.is_empty()
                            || !frame.upvars.contains_key(array_name))
                    {
                        return frame.locals.contains_key(name);
                    }
                } else if self.current_namespace.as_ref() == "::"
                    && self.array_globals.contains(array_name)
                {
                    return self.globals.contains_key(name);
                }
            }
            // Tcl 9: a bare array base is the current namespace's own
            // (resolve_loc already lands there; no global fallback).
            let loc = self.resolve_loc(array_name);
            if self.loc_is_array(&loc) {
                let key = loc.element_key(index);
                self.loc_get(&loc, &key).is_some()
            } else {
                // Element of a scalar / of a missing variable: never exists.
                false
            }
        } else {
            self.resolve_var(name).is_some()
        }
    }

    /// `incr varName ?amount?` — Tcl reads first: a missing variable starts
    /// at 0, but a type conflict (scalar where element expected, or array
    /// read as scalar) is an error.
    pub fn incr_var(&mut self, name: &str, amount: i64) -> Result<Value> {
        let current: i64 = match self.get_var(name) {
            Ok(v) => v
                .as_int()
                .ok_or_else(|| Error::type_mismatch("integer", v.as_str()))?,
            Err(e) => {
                if is_type_conflict(&e) {
                    return Err(e);
                }
                0
            }
        };
        self.set_var(name, Value::from_int(current + amount))
    }

    /// Is `name` (whole-name form) an array in the scope that owns it?
    /// Quiet existence check (scalar or array; no read traces fired) —
    /// the `array default` subcommands' name probe.
    pub(crate) fn var_exists_quiet(&self, name: &str) -> bool {
        let loc = self.resolve_loc(name);
        self.loc_base_exists(&loc)
    }

    pub(crate) fn is_array_semantic(&self, name: &str) -> bool {
        let loc = self.resolve_loc(name);
        self.loc_is_array(&loc)
    }

    /// Canonical identity of the array at `name` (scope-aware), used to key
    /// search lists and mutation stamps.
    pub(crate) fn array_stamp_key(&self, name: &str) -> String {
        stamp_key(&self.resolve_loc(name))
    }

    /// The `array default` value of the array `name` resolves to (TIP
    /// 508).  Consulted on element-miss reads only — the default is
    /// never materialized, so `info exists`/`array size` stay untouched.
    pub(crate) fn array_default_ref(&self, base: &str) -> Option<&Value> {
        if self.array_defaults.is_empty() {
            return None;
        }
        self.array_defaults.get(&stamp_key(&self.resolve_loc(base)))
    }

    pub(crate) fn array_default_of(&self, base: &str) -> Option<Value> {
        self.array_default_ref(base).cloned()
    }

    pub(crate) fn set_array_default(&mut self, base: &str, value: Value) {
        self.array_defaults
            .insert(stamp_key(&self.resolve_loc(base)), value);
    }

    pub(crate) fn unset_array_default(&mut self, base: &str) {
        if !self.array_defaults.is_empty() {
            self.array_defaults.remove(&stamp_key(&self.resolve_loc(base)));
        }
    }

    /// Whether `name` is const-marked in the scope a write would land in
    /// (frame-scoped marks or the global set).  Frames without declared
    /// constants (`consts == None`) and an empty global set pay one
    /// branch each.
    pub(crate) fn is_const_marked(&self, name: &str) -> bool {
        if !self.const_globals.is_empty() && self.const_globals.iter().any(|c| c == name) {
            return true;
        }
        if let Some(frame) = self.var_frame() {
            if let Some(consts) = &frame.consts {
                return consts.iter().any(|c| c == name);
            }
        }
        false
    }

    /// A write (`set`/`append`/`lappend`) onto a constant variable's
    /// error (tclsh tclVar.c ISCONST via the "set" path).
    pub(crate) fn const_write_err(&mut self, name: &str) -> Option<Error> {
        if self.is_const_marked(name) {
            crate::interp::commands::list::set_error_code(self, "TCL WRITE VARNAME");
            return Some(Error::runtime(
                format!("can't set \"{}\": variable is a constant", name),
                ErrorCode::Generic,
            ));
        }
        None
    }

    /// `unset` onto a constant variable (tclVar.c line 2547: the "unset"
    /// path reports ISCONST with TCL UNSET CONST).
    pub(crate) fn const_unset_err(&mut self, name: &str) -> Option<Error> {
        if self.is_const_marked(name) {
            crate::interp::commands::list::set_error_code(self, "TCL UNSET CONST");
            return Some(Error::runtime(
                format!("can't unset \"{}\": variable is a constant", name),
                ErrorCode::Generic,
            ));
        }
        None
    }

    /// `incr` onto a constant variable (tclVar.c line 2310: the "incr"
    /// path reports ISCONST with TCL WRITE CONST).
    pub(crate) fn const_incr_err(&mut self, name: &str) -> Option<Error> {
        if self.is_const_marked(name) {
            crate::interp::commands::list::set_error_code(self, "TCL WRITE CONST");
            return Some(Error::runtime(
                format!("can't incr \"{}\": variable is a constant", name),
                ErrorCode::Generic,
            ));
        }
        None
    }

    /// The element keys ever inserted into the array at `name` since its
    /// (re)creation, for `array statistics`: tclsh's element table keeps
    /// the buckets it grew into as elements were deleted, and starts over
    /// when the array is unset and made again.
    pub(crate) fn array_element_history(&self, name: &str) -> Vec<String> {
        self.array_elements(name, false)
    }

    /// Locate the scope that owns the array `name` refers to, following
    /// upvar links to the ultimate owner: returns `(frame_index, base)`
    /// where `frame_index = None` means the global table and `base` is
    /// the array's key there (`upvar a x` at eval level names the
    /// aliased target, ns-qualified).  Callers enumerate elements with
    /// `base(`-prefixed keys in the returned table.
    pub(crate) fn array_owner(&self, name: &str) -> (Option<usize>, String) {
        // An absolute name (`::a`) is the global table's, under the key it
        // goes by there — it used to be taken as spelled, so `array size
        // ::a` and `array get ::a` looked for `::a(` and found nothing.
        let (mut scope, mut base) = match Self::split_global(name) {
            Some(key) => (None, key),
            None => match self.var_frame_idx() {
                Some(frame) => (Some(frame), name.to_string()),
                // Tcl 9: a bare array name is the current namespace's.
                None => (
                    None,
                    self.redirect_flat(self.canonical_global(name)),
                ),
            },
        };
        // Follow upvar links transitively (with a hop bound so a link
        // cycle can't spin).
        for _ in 0..=self.frames.len() {
            let next = match scope {
                Some(fi) => self
                    .frames
                    .get(fi)
                    .and_then(|f| f.upvars.get(base.as_str()))
                    .cloned(),
                None => self
                    .flat_aliases
                    .iter()
                    .find(|(k, _)| *k == base)
                    .map(|(_, t)| UpvarLink::Global(Rc::from(t.as_str()))),
            };
            match next {
                Some(UpvarLink::Global(g)) => {
                    scope = None;
                    base = g.to_string();
                }
                Some(UpvarLink::Frame {
                    frame_index,
                    var_name,
                }) => {
                    scope = Some(frame_index);
                    base = var_name;
                }
                // A dead link terminates the chain (var-1.17 semantics:
                // the alias stands on its own seeded copy).
                Some(UpvarLink::Dead { .. }) => break,
                None => break,
            }
        }
        (scope, base)
    }

    /// Fire variable traces for `op` on (base, elem). Callback scripts run
    /// in the current scope with `name1 name2 op` appended (tclsh passes
    /// name2 as an empty string for whole-variable ops).
    ///
    /// Returns whether any script actually fired (callers use this to
    /// re-read the written value, tclsh: a write trace that modifies the
    /// variable changes `set`'s result, D-36).
    ///
    /// Re-entrancy guard (tclsh VAR_TRACE_ACTIVE): while a target's
    /// traces fire, a nested fire on the SAME target is skipped — a
    /// callback writing its own traced variable must not recurse into
    /// the trace machinery (native stack overflow without this, D-36).
    /// The guard is per exact target: whole-var fires and element fires
    /// guard independently (tclsh guards per Var record), so a whole-var
    /// write inside an element's callback still fires the whole-var
    /// traces, and vice versa.
    pub(crate) fn fire_traces(&mut self, base: &str, elem: Option<&str>, op: &str) -> Result<bool> {
        self.fire_traces_impl(base, elem, op, true)
    }

    /// Element-only fire：不触发整变量组（tclsh 整数组 unset 的逐元素
    /// 扫描只发元素 Var 的 trace——2026-11-05 实测：trace on a(unset)
    /// 在 `array unset a`/`unset a` 时恰一次 {a {} unset}、来自末尾的
    /// 整变量 fire，元素扫描不发整组，D-37③）。
    #[allow(dead_code)]
    pub(crate) fn fire_traces_elem_only(
        &mut self,
        base: &str,
        elem: &str,
        op: &str,
    ) -> Result<bool> {
        self.fire_traces_impl(base, Some(elem), op, false)
    }

    fn fire_traces_impl(
        &mut self,
        base: &str,
        elem: Option<&str>,
        op: &str,
        include_whole: bool,
    ) -> Result<bool> {
        self.fire_traces_key(&self.array_stamp_key(base), base, elem, op, include_whole)
    }

    /// [`Self::fire_traces`] with the key of the variable's table given:
    /// for a fire that comes after the variable itself is gone, when the
    /// name would no longer resolve to it (an `unset` of a global from
    /// inside a namespace body).
    fn fire_traces_key(
        &mut self,
        key: &str,
        base: &str,
        elem: Option<&str>,
        op: &str,
        include_whole: bool,
    ) -> Result<bool> {
        let guard = match elem {
            Some(e) => format!("E\x00{}\x00{}", key, e),
            None => format!("W\x00{}", key),
        };
        if !self.trace_active.insert(guard.clone()) {
            return Ok(false);
        }
        let fired = self.fire_traces_inner(key, base, elem, op, include_whole);
        self.trace_active.remove(&guard);
        fired
    }

    fn fire_traces_inner(
        &mut self,
        key: &str,
        base: &str,
        elem: Option<&str>,
        op: &str,
        include_whole: bool,
    ) -> Result<bool> {
        let (whole, elems) = self.collect_trace_scripts(key, elem, op);
        let whole: Vec<String> = if include_whole { whole } else { Vec::new() };
        let fired = !(whole.is_empty() && elems.is_empty());
        // Most-recent-registered fires first within each group (tclsh
        // prepends to the trace list: T2 T1 for two write traces), and
        // the whole-variable group fires BEFORE the element group
        // (tclsh order A1E1, D-37).  The script string gets the args
        // appended (so scripts can end in `;#` to swallow them) and is
        // evaluated as a script.
        let q = |v: &str| Value::from_list(&[Value::from_str(v)]).as_str().to_string();
        for s in whole.into_iter().rev().chain(elems.into_iter().rev()) {
            let cmd = format!("{} {} {} {}", s, q(base), q(elem.unwrap_or("")), op);
            // The callback writes another traced variable, which fires
            // again from inside this frame: one nesting level per fire.
            self.enter_nested()?;
            let r = self.eval(&cmd);
            self.leave_nested();
            r?;
        }
        Ok(fired)
    }

    /// Trace scripts registered for `op` on (base-key `key`, optional
    /// element): (whole-var scripts, element scripts), each in stored
    /// (oldest-first) order.
    fn collect_trace_scripts(
        &self,
        key: &str,
        elem: Option<&str>,
        op: &str,
    ) -> (Vec<String>, Vec<String>) {
        let mut whole: Vec<String> = Vec::new();
        if let Some(trs) = self.var_traces.get(key) {
            for t in trs {
                if t.ops.iter().any(|o| o == op) {
                    whole.push(t.script.clone());
                }
            }
        }
        let mut elems: Vec<String> = Vec::new();
        if let Some(e) = elem {
            if let Some(map) = self.elem_traces.get(key) {
                if let Some(trs) = map.get(e) {
                    for t in trs {
                        if t.ops.iter().any(|o| o == op) {
                            elems.push(t.script.clone());
                        }
                    }
                }
            }
        }
        (whole, elems)
    }

    /// Read one element for `array get`: fire its read traces one script
    /// at a time (pt1: `array get x` fires `{x a read} {x b read}`) with
    /// tclsh's interleaved checks (probes P1-P7, trace-1.11..14):
    ///  - the array container gone right after a script → the whole get
    ///    aborts `can't read "base(elem)": no such variable` and later
    ///    traces never run (1.13/1.14: the set-foo trace after unset-x
    ///    never fires, x stays gone);
    ///  - the container destroyed AND recreated by the callbacks (fresh
    ///    base-key value) → the snapshot element is skipped even when a
    ///    same-named element exists again (P2: recreated `bar 9` invisible);
    ///  - a callback error is swallowed and skips the element (P6:
    ///    `error BOOM` trace → get returns {}) without firing the rest;
    ///  - otherwise the element's CURRENT value is used (P5: trace set
    ///    `bar 5` → get shows 5), a simply-unset element is skipped (P1).
    pub(crate) fn array_get_element(
        &mut self,
        owner_fi: Option<usize>,
        base: &str,
        elem: &str,
    ) -> std::result::Result<Option<Value>, Error> {
        let key = self.array_stamp_key(base);
        let gen0 = self.array_generation;
        let alive = |t: &Self| match owner_fi {
            Some(i) => t.frames.get(i).map(|f| f.locals.contains_key(base)),
            None => Some(t.globals.contains_key(base)),
        };
        let (whole, elems) = self.collect_trace_scripts(&key, Some(elem), "read");
        let q = |v: &str| Value::from_list(&[Value::from_str(v)]).as_str().to_string();
        let mut recreated = false;
        // 元素读取 trace：整变量组先、元素组后（A1E1 同序），组内
        // latest-first（tclsh per-Var 触发、Tcl 列表序）。
        for s in whole.into_iter().rev().chain(elems.into_iter().rev()) {
            let cmd = format!("{} {} {} {}", s, q(base), q(elem), "read");
            if self.eval(&cmd).is_err() {
                return Ok(None);
            }
            match alive(self) {
                None | Some(false) => {
                    crate::interp::commands::list::set_error_code(self, "TCL READ VARNAME");
                    return Err(Error::runtime(
                        format!("can't read \"{}({})\": no such variable", base, elem),
                        ErrorCode::Generic,
                    ));
                }
                Some(true) => {}
            }
            if self.array_generation != gen0 {
                recreated = true;
            }
        }
        if recreated {
            return Ok(None);
        }
        let ekey = format!("{}({})", base, elem);
        Ok(match owner_fi {
            Some(i) => self.frames.get(i).and_then(|f| f.locals.get(&ekey)).cloned(),
            None => self.globals.get(&ekey).cloned(),
        })
    }

    /// A trace on an element was satisfied: the element now has a real
    /// value, so drop its phantom marker.
    fn clear_phantom(&mut self, base: &str, elem: &str) {
        let key = self.array_stamp_key(base);
        if let Some(s) = self.trace_phantoms.get_mut(&key) {
            s.remove(elem);
        }
    }

    pub(crate) fn bump_stamp_by_name(&mut self, name: &str) {
        let loc = self.resolve_loc(name);
        self.bump_stamp(&loc);
    }

    /// Mark every upvar alias pointing INTO the array at `loc` (any
    /// element of it) dead: writes through those aliases now error with
    /// "upvar refers to element in deleted array" (var-1.17).
    fn deaden_element_links(&mut self, loc: &VarLoc) {
        let (scope_idx, base) = match loc {
            VarLoc::Global(n) => (None, n.clone()),
            VarLoc::Frame(i, n) => (Some(*i), n.clone()),
        };
        let elem_prefix = format!("{}(", base);
        for f in self.frames.iter_mut() {
            for link in f.upvars.values_mut() {
                match link {
                    UpvarLink::Frame { frame_index, var_name }
                        if Some(*frame_index) == scope_idx
                            && var_name.starts_with(&elem_prefix) =>
                    {
                        *link = UpvarLink::Dead {
                            array: true,
                            was_global: false,
                            target: var_name.clone(),
                        };
                    }
                    UpvarLink::Global(g) if scope_idx.is_none() && g.starts_with(&elem_prefix) => {
                        *link = UpvarLink::Dead {
                            array: true,
                            was_global: true,
                            target: g.to_string(),
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    /// `namespace delete <prefix>`: every alias (frame upvar or eval-level
    /// flat alias) whose target lives under the deleted namespace turns
    /// dead — reads keep the seeded copy, writes error (var-1.15/1.16).
    /// Variables a `variable` command declared at eval level are exempt:
    /// the declaring scope still references them.
    /// Seed every frame's `variable`/`global` link under `var_prefix`
    /// with the target's live value — the prelink path installs links
    /// WITHOUT an install-time seed (a per-call copy), so this dead-time
    /// materialisation is what reads after the purge fall back to
    /// (tclsh keeps the linked Var refcounted; 46.8).  Callers purge the
    /// namespace's globals right after (namespace delete) or destroy the
    /// object's storage (OO destroy).  Prefix forms with or without the
    /// leading `::` both match (the two call sites spell it differently).
    pub(crate) fn materialize_frame_links(&mut self, var_prefix: &str) {
        let norm_prefix = var_prefix.trim_start_matches(':');
        let mut to_seed: Vec<(usize, Rc<str>, String)> = Vec::new();
        for (fi, f) in self.frames.iter().enumerate() {
            for (name, link) in f.upvars.iter() {
                if let UpvarLink::Global(g) = link {
                    if g.trim_start_matches(':').starts_with(norm_prefix)
                        && !f.locals.contains_key(&**name)
                    {
                        to_seed.push((fi, Rc::clone(name), g.to_string()));
                    }
                }
            }
        }
        for (fi, name, target) in to_seed {
            // The same lookup a read through the link performs — the
            // seeded copy is exactly what the later fallback returns.
            let key = self.redirect_flat(target.clone());
            let v = self
                .globals
                .get(key.as_str())
                .or_else(|| self.globals.get(target.trim_start_matches(':')));
            if let Some(v) = v {
                frame_local_insert(&mut self.frames[fi], name.to_string(), v.clone());
            }
        }
    }

    pub(crate) fn deaden_links_under(&mut self, var_prefix: &str, inside_tree: bool) {
        let aliases: Vec<(String, String)> = self.flat_aliases.clone();
        for (k, t) in aliases {
            if t.starts_with(var_prefix) && !self.ns_variable_links.contains(&t) {
                self.dead_flat.push(k);
            }
        }
        // Aliases living IN the deleted namespace's variable table die
        // with it (an `upvar` inside `namespace eval ns` links ns's own
        // variable).
        self.flat_aliases.retain(|(k, _)| !k.starts_with(var_prefix));
        // Frame links under the dying tree keep their value as a seeded
        // local copy — **only for a delete initiated from inside the
        // tree**: tclsh's teardown is then deferred (activationCount > 0
        // — the var table outlives the delete for the frames inside),
        // so reads through the link still resolve (namespace-46.8 S1 钉
        // 形 `variable ::dn::v seed` + `namespace delete [namespace
        // current]` → read seed)。An outside delete tears the table
        // down immediately: no seed — reads through the alias fail
        // (`can't read "fv": no such variable`、unset 同、exists 0——
        // trv_p11 var-alias-after-del/W 钉形), writes keep the
        // dangling-namespace guard. Writes error either way (the links
        // go dead below).
        if inside_tree {
            self.materialize_frame_links(var_prefix);
        }
        for f in self.frames.iter_mut() {
            let dead: Vec<String> = f
                .upvars
                .iter()
                .filter_map(|(n, l)| match l {
                    UpvarLink::Global(g) => g.starts_with(var_prefix).then(|| n.to_string()),
                    _ => None,
                })
                .collect();
            for link in f.upvars.values_mut() {
                if let UpvarLink::Global(g) = link {
                    if g.starts_with(var_prefix) {
                        *link = UpvarLink::Dead {
                            array: false,
                            was_global: true,
                            target: g.to_string(),
                        };
                    }
                }
            }
            // An outside delete tears the table down immediately: the
            // install-time mirror (cmd_variable's `info vars` placeholder,
            // a copy of the target) is now stale — tclsh's alias reads
            // THROUGH the link and the target is gone.  Drop the mirrors
            // so read/unset/exists report the dead target (trv_p11 ⑤:
            // `can't read/unset "fv": no such variable`、exists 0).
            if !inside_tree {
                for n in dead {
                    f.locals.remove(&n);
                    f.array_locals.remove(&n);
                }
            }
        }
    }

    /// Read a variable, firing read traces the way tclsh does: scalar
    /// reads fire even when the variable is missing; element reads fire
    /// when the element exists OR the array itself exists — a missing
    /// element read on a live array fires the whole-var read trace, whose
    /// callback may create the element (trace-1.7).  No fire when the
    /// array is missing or the base is a scalar (trace-1.8).
    pub fn read_var(&mut self, name: &str) -> Result<Value> {
        // T4 vars：两张 trace 表全空 → fire 一趟是可证空转（fire 也会
        // 为 stamp key 白付两次 resolve_loc 分配），base/index 拆分与
        // is-array 判定一并跳过——元素读走 get_var 的快径守卫。
        if self.var_traces.is_empty() && self.elem_traces.is_empty() {
            return self.get_var(name).cloned();
        }
        // Fast path: scalar read with no variable traces registered — the
        // loop-body `$x` case.  The base-name allocation and the read
        // trace pass are both no-ops without traces, so skip them whole
        // (tclsh: scalar reads take the var-table direct hit).
        if !name.contains('(') && self.var_traces.is_empty() {
            return self.get_var(name).cloned();
        }
        let (base, idx) = match split_array_ref(name) {
            Some((b, i)) => (b.to_string(), Some(i.to_string())),
            None => (name.to_string(), None),
        };
        let fire = match &idx {
            None => true,
            Some(_) => self.is_array_semantic(&base),
        };
        if fire {
            if let Err(e) = self.fire_traces(&base, idx.as_deref(), "read") {
                return Err(Error::runtime(
                    format!("can't read \"{}\": {}", name, e),
                    ErrorCode::Generic,
                ));
            }
        }
        self.get_var(name).cloned()
    }

    /// 顶层标量直读快路径（name-keyed 热循环专用，var_read 咽喉）：
    /// 单次括号扫描 + 单次哈希探测，跳过 eval_var_ref/get_var 的重复
    /// 下标检查与 scope-chain 步进。守卫集 = 全语义路径可能分岔的全
    /// 部条件；任一命中返回 None（调用方走 eval_var_ref 全链）。
    ///
    /// T4：三条臂——(1) `::` 顶层裸名（原样）；(2) 命名空间体裸名
    /// （无 proc 帧：键 = `<ns>::<名字>` 在栈上拼装、单次探测，命中即
    /// 全部答案，miss/数组语义回落全链——T1 的 9.1 规则：体里裸名只
    /// 查当前 ns 自己的表，`canonical_global`+`redirect_flat` 的落点
    /// 由守卫等价替代）；(3) 根限定单叶 `::leaf`（split_global 的落
    /// 点，ts 单元缺席时直读 :: 表）。
    pub(crate) fn read_var_scalar_top(&self, name: &str) -> Option<&Value> {
        crate::interp::dbgtrace::event("read_top", name);
        if name.contains('(')
            || !self.var_traces.is_empty()
            || !self.dead_flat.is_empty()
            || !self.frames.is_empty()
        {
            return None;
        }
        // 根限定单叶（`::leaf`，无内层 `::`）：任何语境直读 :: 表
        // （resolve_loc 的 split_global 臂，无重定向）。ts 单元的
        // ::leaf 读由 ts 分支权威（槽叶与全局叶同名同一变量，读槽
        // 新鲜度）——ts 在席即让路。
        if let Some(leaf) = name.strip_prefix("::") {
            if leaf.contains("::") || self.top_slots.is_some() {
                return None;
            }
            if !self.array_globals.is_empty() && self.array_globals.contains(leaf) {
                return None; // 数组语义（is-array 错误文本归属全链）
            }
            return self.globals.get(leaf);
        }
        if has_ns_sep(name) {
            return None;
        }
        if self.current_namespace.as_ref() == "::" {
            // 原 `::` 顶层裸名臂（守卫原样）。
            if !self.flat_aliases.is_empty() {
                return None;
            }
            if !self.array_globals.is_empty() && self.array_globals.contains(name) {
                return None; // 数组语义（is-array 错误文本归属全链）
            }
            return self.globals.get(name);
        }
        // T4 命名空间体裸名臂：键栈上拼（超长回落），平键重定向缺席
        // 时 `<ns>::<名字>` 与 canonical_global+redirect_flat 的产出逐
        // 字同（平名分支无 `::`/`(`，键构造不歧义）。命中即读；miss
        // 与数组语义回落全链（valueless 条目 errorCode、is-array 文本
        // 归它）。
        if !self.flat_aliases.is_empty() || !self.ns_transient.is_empty() {
            return None;
        }
        let mut buf = [0u8; 64];
        let key = stack_ns_key(self.current_namespace.as_ref(), name, &mut buf)?;
        if !self.array_globals.is_empty() && self.array_globals.contains(key) {
            return None; // 数组语义（is-array 错误文本归属全链）
        }
        self.globals.get(key)
    }

    /// 顶层标量直写实守卫（set_var_scalar_top 前置；分离以避免 value
    /// 在守卫失败时被移动）。守卫集 = set_var 全链可能分岔的条件。
    ///
    /// T4：命名空间体裸名臂（覆写语义）——键已存在才放行（新建落点
    /// 与数组语义归慢链，与读侧同一键构造）；`contains_key` 探测后
    /// set_var_scalar_top 的 `get_mut` 必中（两调用相邻、中间无 interp
    /// 变异）。
    pub(crate) fn scalar_top_write_ok(&self, name: &str) -> bool {
        if name.contains('(')
            || has_ns_sep(name)
            || !self.var_traces.is_empty()
            || !self.dead_flat.is_empty()
            || !self.frames.is_empty()
        {
            return false;
        }
        // const 标记名走 set_var（其顶部守卫持有 can't set 错误形）。
        if !self.const_globals.is_empty() && self.const_globals.iter().any(|c| c == name) {
            return false;
        }
        if self.current_namespace.as_ref() == "::" {
            return self.flat_aliases.is_empty()
                && (self.array_globals.is_empty() || !self.array_globals.contains(name));
        }
        // T4 命名空间体裸名覆写臂。
        if !self.flat_aliases.is_empty() || !self.ns_transient.is_empty() {
            return false;
        }
        let mut buf = [0u8; 64];
        let Some(key) = stack_ns_key(self.current_namespace.as_ref(), name, &mut buf) else {
            return false;
        };
        (self.array_globals.is_empty() || !self.array_globals.contains(key))
            && self.globals.contains_key(key)
    }

    /// 顶层标量直写（守卫由 scalar_top_write_ok 前置；写 trace 已被
    /// var_traces 空守卫排除）。set_var 返回语义 = Ok(新值)。
    ///
    /// T4：ns 体臂按守卫同一构造取 `get_mut` 原地写（键已存在，键串
    /// 不动、不进堆）；防御臂 store_var 即 9.1 的落点语义。
    pub(crate) fn set_var_scalar_top(&mut self, name: &str, value: Value) {
        if self.current_namespace.as_ref() != "::" {
            let mut buf = [0u8; 64];
            if let Some(key) = stack_ns_key(self.current_namespace.as_ref(), name, &mut buf) {
                if let Some(slot) = self.globals.get_mut(key) {
                    *slot = value;
                    return;
                }
            }
            // 守卫已确认键存在——此处不可达；store_var 即同一落点。
            self.store_var(name, value);
            return;
        }
        self.global_insert(name.to_string(), value);
    }

    /// `info exists`, which fires read traces (tclsh probe: yes, even for
    /// missing scalars) but swallows trace errors.
    pub fn exists_firing(&mut self, name: &str) -> bool {
        let (base, idx) = match split_array_ref(name) {
            Some((b, i)) => (b.to_string(), Some(i.to_string())),
            None => (name.to_string(), None),
        };
        let fire = match &idx {
            None => true,
            Some(_) => self.is_array_semantic(&base),
        };
        if fire {
            let _ = self.fire_traces(&base, idx.as_deref(), "read");
        }
        self.var_exists(name)
    }

    /// Force array semantics at `name` (marker + base key) without adding
    /// elements — `array set x {}` on a missing variable still creates it.
    pub(crate) fn mark_array(&mut self, name: &str) -> Result<()> {
        self.check_parent_ns(name)?;
        // Array-ification degrades (element keys are flat map entries).
        self.degrade_frame_local_here(name);
        let loc = self.resolve_loc(name);
        if !self.loc_is_array(&loc) {
            let base_key = loc.base_name().to_string();
            self.loc_mark_array(&loc);
            self.loc_insert(&loc, base_key, Value::empty());
        }
        Ok(())
    }

    /// Normalise a fully-qualified namespace path (leading `::`, no
    /// empty components).
    fn normalise_ns(p: &str) -> String {
        let parts: Vec<&str> = p.split("::").filter(|s| !s.is_empty()).collect();
        format!("::{}", parts.join("::"))
    }

    /// Current mutation stamp of the array at `name` (0 if never mutated).
    pub(crate) fn array_stamp(&self, name: &str) -> u64 {
        self.array_stamps.get(&self.array_stamp_key(name)).copied().unwrap_or(0)
    }

    fn bump_stamp(&mut self, loc: &VarLoc) {
        let k = stamp_key(loc);
        *self.array_stamps.entry(k.clone()).or_insert(0) += 1;
        // tclsh: any element-set change kills every search on the array;
        // an emptied search list also resets the id counter.
        if let Some(list) = self.array_searches.get_mut(&k) {
            list.active.clear();
            list.ctr = 0;
        }
    }

    /// Snapshot of the element names of the array at `name`, in Tcl
    /// hash-table order (the element keys ride the scope's insertion log;
    /// replaying their subsequence reproduces tclsh's `array names`).
    pub(crate) fn array_element_names(&self, name: &str) -> Vec<String> {
        self.array_elements(name, true)
    }

    /// Live element count of the array at `name` (P5), kept per scope by
    /// the element write/remove sites.  `None` when the name's count is
    /// not tracked (no count entry — no array was ever created there, or
    /// an element key slipped in without the marker machinery): callers
    /// fall back to the table scan, so a hole degrades speed, not
    /// correctness.
    pub(crate) fn array_element_count(&self, name: &str) -> Option<usize> {
        let (owner_fi, base) = self.array_owner(name);
        match owner_fi {
            Some(i) => self
                .frames
                .get(i)
                .and_then(|f| f.elem_counts.get(&base))
                .copied(),
            None => self.elem_counts.get(&base).copied(),
        }
    }

    /// The element keys of the array at `name` as the log has them: the
    /// live ones, or with `live_only` off every key ever inserted — once,
    /// at its last occurrence (what the table's size and bucket
    /// statistics were built from; `array statistics`).
    fn array_elements(&self, name: &str, live_only: bool) -> Vec<String> {
        let loc = self.resolve_loc(name);
        let prefix = format!("{}(", loc.base_name());
        let is_element = |k: &str| k.starts_with(&prefix) && k.ends_with(')');
        // An element key `a(k)` without its `a(` and `)`.
        fn stripped<'k>(k: &'k String, from: usize) -> &'k str {
            &k[from..k.len() - 1]
        }
        // The element names are slices of the log's entries all the way
        // through the replay; only the result owns them.
        let collect = |order: &[String],
                       live: &dyn Fn(&str) -> bool,
                       live_count: usize|
         -> Vec<String> {
            // The history starts at the array's (re)creation: after the
            // marker key's last entry (the marker is written when the
            // array is made, never while it lives).
            let order = if live_only {
                order
            } else {
                let base = loc.base_name();
                match order.iter().rposition(|k| k.as_str() == base) {
                    Some(i) => &order[i + 1..],
                    None => order,
                }
            };
            let keys: Vec<&str> = if live_only && order.len() == live_count {
                // Nothing stale in the log: every entry is a live key, once.
                // (A lookup per entry instead was the whole cost of listing
                // a large array: random access into a large table.)
                order
                    .iter()
                    .filter(|k| is_element(k))
                    .map(|k| stripped(k, prefix.len()))
                    .collect()
            } else {
                let mut seen: super::varmap::FxHashedMap<&str, usize> =
                    super::varmap::FxHashedMap::default();
                for (i, k) in order.iter().enumerate() {
                    if is_element(k) && (!live_only || live(k)) {
                        seen.insert(k.as_str(), i);
                    }
                }
                order
                    .iter()
                    .enumerate()
                    .filter(|(i, k)| seen.get(k.as_str()) == Some(i))
                    .map(|(_, k)| stripped(k, prefix.len()))
                    .collect()
            };
            tclhash::tcl_order(keys.iter().copied())
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        match &loc {
            VarLoc::Global(_) => {
                let n = self.globals.len();
                collect(&self.var_order, &|k| self.globals.contains_key(k), n)
            }
            VarLoc::Frame(i, _) => self
                .frames
                .get(*i)
                .map(|f| {
                    let n = f.locals.len();
                    collect(&f.locals_order, &|k| f.locals.contains_key(k), n)
                })
                .unwrap_or_default(),
        }
    }

    pub fn result(&self) -> &Value {
        &self.result
    }

    #[cfg(feature = "std")]
    pub fn set_script_name(&mut self, name: &str) {
        self.script_name = name.to_string();
    }

    #[cfg(feature = "std")]
    pub fn script_name(&self) -> &str {
        &self.script_name
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // -- `::name` global qualification (tclsh 8.6.17) --

    #[test]
    fn test_global_qualification_read() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("set x 5; set ::x").unwrap().as_str(), "5");
    }

    #[test]
    fn test_global_qualification_write() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("set ::y 7; set y").unwrap().as_str(), "7");
    }

    #[test]
    fn test_global_qualification_from_proc() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("set e 1; proc p {} { return $::e }; p").unwrap().as_str(), "1");
    }

    #[test]
    fn test_global_qualification_never_sees_locals() {
        // ::z inside a proc is always the global, even when a local z exists.
        let mut interp = Interp::new();
        let r = interp
            .eval("proc q {} { set ::z 9; set z 8; return [list $z $::z] }; q")
            .unwrap();
        assert_eq!(r.as_str(), "8 9");
        assert_eq!(interp.eval("set z").unwrap().as_str(), "9");
    }

    #[test]
    fn test_global_qualification_unset_and_exists() {
        assert_eq!(interp_eval("set k 1; unset ::k; info exists k"), "0");
        assert_eq!(interp_eval("set k2 1; unset k2; info exists ::k2"), "0");
    }

    fn interp_eval(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    // -- ::errorCode observability (tclsh 8.6.17) --

    #[test]
    fn test_errorcode_from_strict_list_error() {
        // 9.1 重钉（digits91 批）：`09` 已是合法整数，换一个真正非数的
        // 元素来钉 strict 整数比较的 TCL VALUE NUMBER。
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {lsort -integer {09x 8}} m; set ::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "TCL VALUE NUMBER");
    }

    #[test]
    fn test_errorcode_none_after_bare_error() {
        let mut interp = Interp::new();
        let r = interp.eval("catch {error foo} m; set ::errorCode").unwrap();
        assert_eq!(r.as_str(), "NONE");
    }

    #[test]
    fn test_errorcode_from_error_third_arg() {
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {error msg {} {CUSTOM X}} m; set ::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "CUSTOM X");
    }

    #[test]
    fn test_errorcode_persists_after_successful_command() {
        let mut interp = Interp::new();
        let r = interp
            .eval("catch {lsort -index -1 x} m; set unused 1; set ::errorCode")
            .unwrap();
        assert_eq!(r.as_str(), "TCL VALUE INDEX OUTOFRANGE");
    }

    // -- scalar/array distinction (tclsh 8.6.17 oracle probes) --

    fn eval_err(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap_err().to_string()
    }

    #[test]
    fn test_element_set_on_scalar_errors() {
        assert_eq!(
            eval_err("set x {}; set x(0) 44"),
            "can't set \"x(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_element_read_on_scalar_errors() {
        assert_eq!(
            eval_err("set y 5; set y(0)"),
            "can't read \"y(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_scalar_set_on_array_errors() {
        assert_eq!(
            eval_err("set a(0) 1; set a 5"),
            "can't set \"a\": variable is array"
        );
    }

    #[test]
    fn test_scalar_read_of_array_errors() {
        assert_eq!(
            eval_err("set b(0) 1; set b"),
            "can't read \"b\": variable is array"
        );
    }

    #[test]
    fn test_append_element_on_scalar_and_array() {
        assert_eq!(
            eval_err("set z 1; append z(2) a"),
            "can't set \"z(2)\": variable isn't array"
        );
        assert_eq!(
            eval_err("set c(0) 1; append c 5"),
            "can't set \"c\": variable is array"
        );
    }

    #[test]
    fn test_append_element_creates_missing_array() {
        // tclsh: append nm(0) z → nm becomes an array
        assert_eq!(interp_eval("append nm(0) z; set nm(0)"), "z");
    }

    #[test]
    fn test_array_survives_last_element_unset() {
        // tclsh: unset of the last element keeps the array alive
        assert_eq!(interp_eval("set d(0) 1; unset d(0); array exists d"), "1");
        assert_eq!(
            eval_err("set d2(0) 1; unset d2(0); set d2 5"),
            "can't set \"d2\": variable is array"
        );
        assert_eq!(interp_eval("set e(0) 1; unset e(0); info exists e"), "1");
    }

    #[test]
    fn test_unset_whole_array_removes_everything() {
        assert_eq!(
            interp_eval("set g(0) 1; set g(1) 2; unset g; list [info exists g] [array exists g]"),
            "0 0"
        );
    }

    #[test]
    fn test_element_read_error_kinds() {
        assert_eq!(
            eval_err("set a2(0) 1; set a2(5)"),
            "can't read \"a2(5)\": no such element in array"
        );
        assert_eq!(
            eval_err("set zz(5)"),
            "can't read \"zz(5)\": no such variable"
        );
    }

    #[test]
    fn test_element_unset_error_kinds() {
        assert_eq!(
            eval_err("set u9 5; unset u9(3)"),
            "can't unset \"u9(3)\": variable isn't array"
        );
        assert_eq!(
            eval_err("set a3(0) 1; unset a3(5)"),
            "can't unset \"a3(5)\": no such element in array"
        );
        assert_eq!(
            eval_err("unset zz2(5)"),
            "can't unset \"zz2(5)\": no such variable"
        );
    }

    #[test]
    fn test_proc_local_array_shadows_global_scalar() {
        // tclsh: inside a proc, set gs(0) creates a LOCAL array gs even
        // though a global scalar gs exists.
        assert_eq!(
            interp_eval("set gs 5; proc pp {} {set gs(0) 1; return [array exists gs]}; pp"),
            "1"
        );
        assert_eq!(interp_eval("set gs 5; proc pp {} {set gs(0) 1; return [array exists gs]}; pp; set gs"), "5");
    }

    #[test]
    fn test_upvar_element_uses_alias_name_in_errors() {
        // tclsh: upvar to a scalar, element write errors naming the ALIAS.
        assert_eq!(
            eval_err("set us 5; proc up2 {} {upvar 1 us b; set b(0) y}; up2"),
            "can't set \"b(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_upvar_element_read_and_write() {
        // upvar to an array: element ops follow the link.
        assert_eq!(
            interp_eval("set ua(0) x; proc up {} {upvar 1 ua b; set b(0) y; return $b(0)}; up"),
            "y"
        );
        assert_eq!(
            interp_eval("set ua2(0) x; proc up2 {} {upvar 1 ua2 b; return $b(0)}; up2"),
            "x"
        );
    }

    #[test]
    fn test_array_set_on_scalar() {
        // tclsh quirk: empty pairs → "can't array set", non-empty →
        // first element write error.
        assert_eq!(
            eval_err("set s9 5; array set s9 {}"),
            "can't array set \"s9\": variable isn't array"
        );
        assert_eq!(
            eval_err("set arr3 5; array set arr3 {0 1}"),
            "can't set \"arr3(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_incr_element_on_scalar_reads_first() {
        assert_eq!(
            eval_err("set ii 5; incr ii(0)"),
            "can't read \"ii(0)\": variable isn't array"
        );
    }

    #[test]
    fn test_info_exists_element_of_scalar_is_false() {
        assert_eq!(interp_eval("set is2 5; info exists is2(0)"), "0");
    }

    #[test]
    fn test_global_element_access_from_proc() {
        assert_eq!(
            interp_eval("set ga(0) 1; proc gp {} {set ::ga(0) 2; return $::ga(0)}; gp"),
            "2"
        );
    }

    #[test]
    fn test_array_exists_and_size_on_scalar() {
        assert_eq!(interp_eval("set as2 5; array exists as2"), "0");
        assert_eq!(interp_eval("set as2 5; array size as2"), "0");
        assert_eq!(interp_eval("set aw(0) 1; array size aw"), "1");
    }

    #[test]
    fn test_builtin_arrays_exist() {
        assert_eq!(interp_eval("array exists tcl_platform"), "1");
        assert_eq!(interp_eval("array exists env"), "1");
    }

    // -- ARITH errorCode on arithmetic errors (tclsh 8.6.17) --

    #[test]
    fn test_arith_divzero_errorcode() {
        assert_eq!(
            interp_eval("catch {expr {1/0}} m; set ::errorCode"),
            "ARITH DIVZERO {divide by zero}"
        );
        assert_eq!(
            interp_eval("catch {expr {1%0}} m; set ::errorCode"),
            "ARITH DIVZERO {divide by zero}"
        );
    }

    #[test]
    fn test_arith_domain_errorcode() {
        assert_eq!(
            interp_eval("catch {expr {sqrt(-1)}} m; set ::errorCode"),
            "ARITH DOMAIN {domain error: argument not in valid range}"
        );
    }

    #[test]
    fn test_arith_errorcode_in_catch_options() {
        // tclsh: the optionVar is a dict; element syntax is an error.
        assert_eq!(
            interp_eval("catch {expr {1/0}} m o; dict get $o -errorcode"),
            "ARITH DIVZERO {divide by zero}"
        );
    }

    #[test]
    fn test_no_errorcode_write_for_plain_lookup_errors() {
        // tclsh: a missing variable read stamps `TCL LOOKUP VARNAME` (no
        // ARITH code); a missing variable WRITE doesn't exist (set
        // creates), and a wholly fresh interp has no errorCode at all.
        assert_eq!(
            interp_eval("catch {set nosuchvar} m; set ::errorCode"),
            "TCL LOOKUP VARNAME nosuchvar"
        );
        assert_eq!(
            interp_eval("set created 5; catch {set ::errorCode} m; set m"),
            "can't read \"::errorCode\": no such variable"
        );
    }

    // -- trailing-colon / colon-run variable names (tclsh 8.6.17,
    //    trv_p11 ①②钉形：绝对名分段 = `::` 串分隔 + 尾串冒号空名 +
    //    尾单冒号属名字；`variable :` 的名字就是 `:`) --

    #[test]
    fn test_trailing_run_names_empty_variable() {
        // `::t4::` 与 `::t4:::` 同一变量：t4 的**空名**变量
        // （info vars 列表里的 `{}` 条目）。
        assert_eq!(
            interp_eval("namespace eval t4 {}; set ::t4:: 5; list [set ::t4::] [set ::t4:::] [info exists ::t4:::]"),
            "5 5 1"
        );
        assert_eq!(
            interp_eval("namespace eval t5 {}; set ::t5::: 6; list [set ::t5::] [set ::t5:::]"),
            "6 6"
        );
        assert_eq!(
            // 空名变量在本 ns 的 info vars 里以 `{}` 条目现身。
            interp_eval("namespace eval t6 {}; set ::t6:: 1; namespace eval t6 {lappend v [lindex [info vars] 0]; set v}"),
            "{}"
        );
    }

    #[test]
    fn test_colon_name_variable_is_plain_name() {
        // `variable :`：名字是 `:`（var-7.13）——相对 `:` 可读写、which
        // 打印 `::tnv:::`；绝对 `::tnv:::` 是 tnv 的空名变量（≠ `:`），
        // tclsh 的解析/打印不闭环原样保留。
        assert_eq!(
            interp_eval(
                "namespace eval tnv {variable : X1}; \
                 list [namespace eval tnv {set :}] \
                       [namespace eval tnv {info exists :}] \
                       [namespace eval tnv {namespace which -variable :}]"
            ),
            "X1 1 ::tnv:::"
        );
        assert_eq!(
            interp_eval("namespace eval tnv {variable : X1}; info exists ::tnv:::"),
            "0"
        );
        assert_eq!(
            interp_eval("namespace eval tnv {variable : X1}; namespace which -variable ::tnv:::"),
            ""
        );
    }

    #[test]
    fn test_single_trailing_colon_is_name_char() {
        // `::a::b:`：ns ::a 的名字 `b:`（尾**单**冒号属名字）；它与
        // `::a::b::`（::a::b 的空名）不是同一变量。
        assert_eq!(
            interp_eval(
                "namespace eval a {}; set ::a::b: 7; \
                 list [set ::a::b:] [info exists ::a::b:] [info exists ::a::b::]"
            ),
            "7 1 0"
        );
        // 名字中段单冒号同样是名字字符：`a:b`、`::d:e`。
        assert_eq!(
            interp_eval("set a:b 7; namespace which -variable a:b"),
            "::a:b"
        );
        assert_eq!(
            interp_eval("set ::d:e 8; list [set ::d:e] [namespace which -variable ::d:e]"),
            "8 ::d:e"
        );
    }

    #[test]
    fn test_pure_colon_names_are_global_empty_variable() {
        // `:::` 与 `::::` 都是全局 ns 的空名变量（同一键）。
        assert_eq!(
            interp_eval("set ::: 5; set :::: 6; list [set :::] [set ::::] [info exists {}]"),
            "6 6 1"
        );
    }

    #[test]
    fn test_leading_run_skipped_to_global() {
        // `::::z`：前导整串冒号吞掉 → 全局 `z`。
        assert_eq!(
            interp_eval("set ::::z 5; list [set ::z] [set ::::z]"),
            "5 5"
        );
    }

    #[test]
    fn test_write_parent_check_with_trailing_shapes() {
        // 写路径宿主检查：尾串冒号形查 quals 路径（`::nope2::` 报
        // 宿主缺失）；尾单冒号形查更浅一层（`::a::b:` 只要求 ::a）。
        assert_eq!(
            eval_err("set ::nope2:: 5"),
            "can't set \"::nope2::\": parent namespace doesn't exist"
        );
        assert_eq!(
            eval_err("set ::nope3::x 5"),
            "can't set \"::nope3::x\": parent namespace doesn't exist"
        );
        assert_eq!(interp_eval("namespace eval ok {}; set ::ok::leaf: 5; set ::ok::leaf:"), "5");
        assert_eq!(
            eval_err("namespace eval pns {}; namespace eval pns {set sub:: 1}"),
            "can't set \"sub::\": parent namespace doesn't exist"
        );
    }

    #[test]
    fn test_relative_read_with_trailing_run_finds_ns_var() {
        // ns 语境相对名：尾串冒号 = 空、名字逐字（canonical 拼接不吞
        // 尾段）——ns eval 体里的 `set q:: 5` 建的是 w7::q 的空名变量
        // （宿主 ns 必须已存在——tclsh 同）。`set w6::` 在全局对缺失
        // ns 同报宿主缺失。
        assert_eq!(
            eval_err("namespace eval w7 {set q:: 5}"),
            "can't set \"q::\": parent namespace doesn't exist"
        );
        assert_eq!(
            interp_eval(
                "namespace eval w7 {}; namespace eval w7::q {}; \
                 namespace eval w7 {set q:: 5}; \
                 list [set w7::q::] [info exists w7::q]"
            ),
            "5 0"
        );
        assert_eq!(
            eval_err("set w6:: 5"),
            "can't set \"w6::\": parent namespace doesn't exist"
        );
    }

    // -- upvar 数组元素别名生命周期（tclsh 8.6.17，var-1.17/trv_p11
    //    ③④钉形：upvar 0 元素形的链落**帧局部占位**、宿主 unset 后
    //    别名转 dangling）--

    #[test]
    fn test_upvar_element_placeholder_and_dangling() {
        assert_eq!(
            interp_eval(
                "set arr(1) 2; \
                 proc pA {} { upvar 0 arr(1) foo; set r {}; \
                     lappend r [catch {set foo} m] $m; \
                     unset arr; \
                     lappend r [catch {set foo 3} m2] $m2; \
                     lappend r [catch {set foo(3) 3} m3] $m3; \
                     lappend r [info exists foo]; set r }; pA"
            ),
            "1 {can't read \"foo\": no such variable} 1 {can't set \"foo\": upvar refers to element in deleted array} 1 {can't set \"foo(3)\": variable isn't array} 0"
        );
        // 全局 arr 不被帧占位顶替：pA 的 unset 只删帧局部占位。
        assert_eq!(interp_eval("set arr(1) 2; proc pA {} {upvar 0 arr(1) foo; unset arr}; catch pA; set arr(1)"), "2");
    }

    #[test]
    fn test_upvar_element_scalar_base_and_missing_base() {
        // S1：基是帧局部标量 → 建链即报；S2：缺基建帧局部占位数组。
        assert_eq!(
            eval_err("proc pS {} {set x 5; upvar 0 x(1) foo}; pS"),
            "can't access \"x(1)\": variable isn't array"
        );
        assert_eq!(
            interp_eval(
                "proc pM {} {upvar 0 nosuch(1) foo; list [array exists nosuch] [info exists foo]}; pM"
            ),
            "1 0"
        );
    }

    #[test]
    fn test_var_alias_after_outside_ns_delete() {
        // trv_p11 ⑤（W/var-1.15 钉形）：外部删除 → 读/删 no such
        // variable、exists 0、写 dangling-namespace。
        assert_eq!(
            interp_eval(
                "namespace eval dn {variable fv 9}; \
                 proc pV {} {variable ::dn::fv; set r {}; \
                     lappend r [set fv]; namespace delete ::dn; \
                     lappend r [catch {set fv} m1] $m1; \
                     lappend r [catch {unset fv} m2] $m2; \
                     lappend r [catch {set fv 7} m3] $m3; \
                     lappend r [info exists fv]; set r }; pV"
            ),
            "9 1 {can't read \"fv\": no such variable} 1 {can't unset \"fv\": no such variable} 1 {can't set \"fv\": upvar refers to variable in deleted namespace} 0"
        );
    }

    #[test]
    fn test_var_alias_self_delete_still_reads() {
        // namespace-46.8 形（S1）：在自己 ns 里删自己——teardown 延迟
        // 语义（帧语境种子），读仍通。
        assert_eq!(
            interp_eval(
                "namespace eval dn5 {variable fv2 9; proc p {} { \
                     variable ::dn5::fv2 seed; namespace delete ::dn5; \
                     list [set fv2] [info exists fv2] }; set r [p]; set r}"
            ),
            "seed 1"
        );
    }
}
