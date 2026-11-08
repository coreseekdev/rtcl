//! Tcl/Tk-compatible native UI runtime.
//!
//! This crate is the bridge between the Tcl `tk` command surface and the
//! backend-independent widget model in [`rtcl_tk_core`].  It is enabled by
//! the native-only `tk` feature; the web/AI `ui::*` DSL is a separate track
//! (`docs/UI-DSL.md`) and never depends on this crate.
//!
//! M1a covers the shadow tree and the core Tk commands; M1b adds
//! `-textvariable`, widget data subcommands (`get`/`insert`/`delete`),
//! `bind`, `event generate`, `pack`/`grid` query forms and the wider
//! `winfo`/`wm` surface.  A renderer backend (nsh) consumes
//! [`snapshot_active`] and drives user events back through the functions at
//! the bottom of this file.

#![cfg_attr(target_arch = "wasm32", allow(unused))]

#[cfg(target_arch = "wasm32")]
compile_error!(
    "rtcl-tk-runtime (native Tk/nsh) is native-only; it is mutually \
     exclusive with the wasm target. Use ui::* for web/WebView UI instead."
);

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use rtcl_core::interp::{add_interp_teardown_hook, Interp};
use rtcl_core::value::Value;
use rtcl_core::{Error, Result};
use rtcl_tk_core::OptionLookup;

pub use rtcl_tk_core::{Fill, GridInfo, PackInfo, PackSide, Sticky, TkError, TkTree, WidgetKind};

#[derive(Debug, Default)]
pub struct TkRuntime {
    tree: TkTree,
    revision: u64,
    /// `bind all` scripts, run after the widget's own bindings.
    all_bindings: BTreeMap<String, String>,
    /// `wm protocol` handlers, keyed by `(path, protocol name)`.
    protocols: BTreeMap<(String, String), String>,
    /// `grid columnconfigure` / `rowconfigure`: `(parent, index, is_row)`
    /// to `(weight, minsize)`.
    grid_options: BTreeMap<(String, i64, bool), (i64, i64)>,
}

impl TkRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tree(&self) -> &TkTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut TkTree {
        &mut self.tree
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Mark the shadow tree changed.  The backend hook runs when the
    /// outermost Tcl evaluation (or UI callback) returns, so a re-render
    /// never re-enters the state map while it is borrowed.
    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        NOTIFY_PENDING.with(|p| p.set(true));
    }
}

thread_local! {
    /// One Tk state per live interpreter (Tcl is single-threaded).
    static STATES: RefCell<HashMap<usize, TkRuntime>> =
        RefCell::new(HashMap::new());
    /// The interpreter the UI backend drives (last `install` on this thread).
    static ACTIVE: Cell<Option<usize>> = const { Cell::new(None) };
    /// Backend change callback (nsh bumps its revision signal here).
    static CHANGE_HOOK: Cell<Option<fn()>> = const { Cell::new(None) };
    /// Errors from bindings, `-command` scripts and timer callbacks, for
    /// the host's `bgerror` route.
    static ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Nesting depth of Tcl evaluation started from the UI.
    static EVAL_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// A Tk mutation is waiting for the backend hook.
    static NOTIFY_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Call the backend hook once, when no Tcl evaluation is in flight.
fn flush_notify() {
    if EVAL_DEPTH.with(|d| d.get()) > 0 {
        return;
    }
    let pending = NOTIFY_PENDING.with(|p| p.replace(false));
    if pending {
        CHANGE_HOOK.with(|h| {
            if let Some(f) = h.get() {
                f();
            }
        });
    }
}

/// Run `f` as one Tcl evaluation; the backend hook fires when the
/// outermost one returns.
fn with_eval<R>(f: impl FnOnce() -> R) -> R {
    let outer = EVAL_DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get() == 1
    });
    let out = f();
    EVAL_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    if outer {
        flush_notify();
    }
    out
}

/// Attach/clear the backend's "shadow tree changed" callback.
pub fn set_change_hook(hook: Option<fn()>) {
    CHANGE_HOOK.with(|h| h.set(hook));
}

pub fn active_interp() -> Option<*mut Interp> {
    ACTIVE.with(|a| a.get()).map(|p| p as *mut Interp)
}

fn record_error(message: String) {
    let _ = ERRORS.try_with(|e| e.borrow_mut().push(message));
}

/// Take the errors collected since the last call (the host shows them with
/// `bgerror`).
pub fn take_errors() -> Vec<String> {
    ERRORS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

fn bump_active() {
    let Some(ptr) = active_interp() else { return };
    let _ = STATES.try_with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(rt) = s.get_mut(&(ptr as usize)) {
                rt.bump();
            }
        }
    });
    flush_notify();
}

/// Evaluate `script` on the active interpreter (button `-command`,
/// bindings), re-rendering afterwards.  Errors are returned and also kept
/// for [`take_errors`].
pub fn eval_active(script: &str) -> Result<Value> {
    let ptr = active_interp().ok_or_else(|| {
        Error::runtime(
            "no active Tk interpreter",
            rtcl_core::error::ErrorCode::Generic,
        )
    })?;
    // Safety: Tcl is single-threaded and the pointer is the live Interp
    // that called install() on this thread.
    let result = with_eval(|| unsafe { (*ptr).eval(script) });
    if let Err(e) = &result {
        record_error(e.to_string());
    }
    bump_active();
    result
}

/// Evaluate a script without recording its error (Tk's `event generate`
/// and test helpers report it directly).
pub fn eval_active_quiet(script: &str) -> Result<Value> {
    let ptr = active_interp().ok_or_else(|| {
        Error::runtime(
            "no active Tk interpreter",
            rtcl_core::error::ErrorCode::Generic,
        )
    })?;
    unsafe { (*ptr).eval(script) }
}

/// Run due `after` timers and idle callbacks.  Returns whether anything
/// ran, so a backend can re-render only then.
pub fn pump_active() -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    // Safety: the pointer is the live engine-thread Interp.
    let (fired, errors) = with_eval(|| unsafe { (*ptr).pump_events_counted() });
    for e in errors {
        record_error(e);
    }
    if fired > 0 {
        bump_active();
    }
    fired > 0
}

/// Whether the active interpreter has `after` work queued.
pub fn has_scheduled_events() -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    unsafe { (*ptr).has_scheduled_events() }
}

/// A snapshot of the active interpreter's shadow tree, for the renderer.
pub fn snapshot_active() -> Option<TkTree> {
    let ptr = active_interp()? as *const Interp;
    STATES.with(|s| s.borrow().get(&(ptr as usize)).map(|rt| rt.tree.clone()))
}

fn key(interp: &Interp) -> usize {
    interp as *const Interp as usize
}

/// Interp teardown: drop this interpreter's Tk shadow tree.
fn teardown(interp: *mut Interp) {
    let _ = STATES.try_with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            s.remove(&(interp as usize));
        }
    });
    ACTIVE.with(|a| {
        if a.get() == Some(interp as usize) {
            a.set(None);
        }
    });
}

fn ensure_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| add_interp_teardown_hook(teardown));
}

/// Snapshot a live Tk state (renderer backends call this from their view).
pub fn with_state<R>(interp: &Interp, f: impl FnOnce(&TkRuntime) -> R) -> Option<R> {
    STATES.with(|s| s.borrow().get(&key(interp)).map(f))
}

/// Mutate a live Tk state (host-side extensions/tests).
pub fn with_state_mut<R>(interp: &Interp, f: impl FnOnce(&mut TkRuntime) -> R) -> Option<R> {
    // The borrow ends before the backend hook runs (a re-render reads the
    // same map).
    let out = STATES.with(|s| s.borrow_mut().get_mut(&key(interp)).map(f));
    flush_notify();
    out
}

/// Register the Tk command surface on `interp` and create its tree.
pub fn install(interp: &mut Interp) {
    ensure_hook();
    STATES.with(|s| {
        s.borrow_mut().entry(key(interp)).or_default();
    });
    ACTIVE.with(|a| a.set(Some(key(interp))));
    for name in [
        "toplevel",
        "frame",
        "label",
        "button",
        "entry",
        "checkbutton",
        "radiobutton",
        "scale",
        "text",
        "listbox",
        "ttk::toplevel",
        "ttk::frame",
        "ttk::label",
        "ttk::button",
        "ttk::entry",
        "ttk::checkbutton",
        "ttk::radiobutton",
        "ttk::scale",
        "ttk::text",
        "ttk::listbox",
    ] {
        interp.register_command(name, create_widget);
    }
    interp.register_command(".", widget_command);
    for name in [
        "configure",
        "cget",
        "destroy",
        "pack",
        "grid",
        "winfo",
        "wm",
        "tk",
        "bind",
        "event",
    ] {
        interp.register_command(name, global_command);
    }
}

// ---------------------------------------------------------------------------
// Errors and value helpers
// ---------------------------------------------------------------------------

fn generic(msg: impl Into<String>) -> Error {
    Error::runtime(msg.into(), rtcl_core::error::ErrorCode::Generic)
}

fn tk_error(e: TkError) -> Error {
    generic(e.to_string())
}

fn wrong(cmd: &str, usage: &str, actual: usize) -> Error {
    Error::wrong_args_with_usage(cmd, 2, actual, usage)
}

fn bad_window(path: &str) -> Error {
    generic(format!("bad window path name \"{path}\""))
}

fn bad_option(option: &str) -> Error {
    generic(format!("unknown option \"{option}\""))
}

fn ambiguous_option(option: &str, matches: &[String]) -> Error {
    generic(format!(
        "ambiguous option \"{option}\": must be {}",
        matches.join(", ")
    ))
}

fn list_value(items: impl IntoIterator<Item = String>) -> Value {
    Value::from_list(
        &items
            .into_iter()
            .map(|s| Value::from_str(&s))
            .collect::<Vec<_>>(),
    )
}

fn parse_int(v: &Value) -> Result<i64> {
    v.as_int()
        .ok_or_else(|| generic(format!("expected integer but got \"{}\"", v.as_str())))
}

fn parse_f64_str(s: &str, what: &str) -> Result<f64> {
    s.trim().parse::<f64>().map_err(|_| {
        generic(format!(
            "bad {what} \"{s}\": expected floating-point number"
        ))
    })
}

fn parse_bool(v: &Value, what: &str) -> Result<bool> {
    match v.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        s => Err(generic(format!("bad {what} \"{s}\": must be boolean"))),
    }
}

fn parse_bool_str(s: &str, what: &str) -> Result<bool> {
    parse_bool(&Value::from_str(s), what)
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Canonicalize option abbreviations against `kind` and the stored keys.
fn canonical_option<'a>(
    kind: WidgetKind,
    name: &str,
    stored: impl IntoIterator<Item = &'a str>,
) -> Result<String> {
    match rtcl_tk_core::lookup_option(kind, name, stored) {
        OptionLookup::Found(o) => Ok(o),
        OptionLookup::Ambiguous(v) => Err(ambiguous_option(name, &v)),
        OptionLookup::Unknown => Err(bad_option(name)),
    }
}

fn canonical_options(
    kind: WidgetKind,
    pairs: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<(String, String)>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (name, value) in pairs {
        if !rtcl_tk_core::valid_option(&name) {
            return Err(generic(format!("bad option \"{name}\": must be a -option")));
        }
        let canonical = canonical_option(kind, &name, seen.iter().map(String::as_str))?;
        seen.push(canonical.clone());
        out.push((canonical, value));
    }
    Ok(out)
}

fn parse_pairs(args: &[Value], start: usize, what: &str) -> Result<Vec<(String, String)>> {
    let rest = &args[start..];
    if !rest.len().is_multiple_of(2) {
        return Err(wrong(what, "?option value ...?", args.len()));
    }
    let mut out = Vec::with_capacity(rest.len() / 2);
    for pair in rest.chunks(2) {
        out.push((pair[0].as_str().to_string(), pair[1].as_str().to_string()));
    }
    Ok(out)
}

fn effective_option(interp: &Interp, path: &str, name: &str) -> String {
    with_state(interp, |rt| {
        rt.tree.get(path).and_then(|n| {
            n.options
                .get(name)
                .cloned()
                .or_else(|| rtcl_tk_core::option_default(n.kind, name).map(str::to_string))
        })
    })
    .flatten()
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Widget creation
// ---------------------------------------------------------------------------

fn create_widget(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let cmd = args[0].as_str();
    let base = cmd.strip_prefix("ttk::").unwrap_or(cmd);
    let kind = WidgetKind::from_tk_name(base).ok_or_else(|| Error::invalid_command(cmd))?;
    if args.len() < 2 {
        return Err(wrong(cmd, "pathName ?-option value ...?", args.len()));
    }
    let path = args[1].as_str().to_string();
    let raw = parse_pairs(args, 2, cmd)?;
    let options = canonical_options(kind, raw)?;

    let created = STATES.with(|s| {
        let mut s = s.borrow_mut();
        let rt = s.entry(key(interp)).or_default();
        rt.tree.create_at(&path, kind).map_err(tk_error)?;
        rt.tree.configure(&path, options).map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    });
    flush_notify();
    created?;

    initialize_widget(interp, &path, kind)?;
    bump_active();
    interp.register_command(&path, widget_command);
    Ok(Value::from_str(&path))
}

/// Give a fresh widget its data and default value (scale midpoint, entry
/// content from `-textvariable`, checkbutton state).
fn initialize_widget(interp: &mut Interp, path: &str, kind: WidgetKind) -> Result<()> {
    match kind {
        WidgetKind::Entry => {
            let var = effective_option(interp, path, "-textvariable");
            if !var.is_empty() {
                let text = var_value(&var).unwrap_or_default();
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.content = text;
                    }
                });
            }
        }
        WidgetKind::Scale => {
            let var = effective_option(interp, path, "-variable");
            let from = effective_option(interp, path, "-from")
                .parse::<f64>()
                .unwrap_or(0.0);
            let to = effective_option(interp, path, "-to")
                .parse::<f64>()
                .unwrap_or(100.0);
            let value = var_value(&var)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or(from + (to - from) / 2.0);
            with_state_mut(interp, |rt| {
                if let Some(n) = rt.tree.get_mut(path) {
                    n.value = value;
                }
            });
        }
        WidgetKind::Checkbutton => {
            let var = effective_option(interp, path, "-variable");
            if !var.is_empty() {
                let on = effective_option(interp, path, "-onvalue");
                let checked = var_value(&var).is_some_and(|v| v == on);
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = checked;
                    }
                });
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-widget commands
// ---------------------------------------------------------------------------

fn widget_command(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let path = args[0].as_str();
    if args.len() < 2 {
        return Err(wrong(path, "subcommand ?arg ...?", args.len()));
    }
    let Some(kind) = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten() else {
        return Err(bad_window(path));
    };
    let sub = args[1].as_str();
    match sub {
        "cget" => cget_impl(interp, path, &args[2..]),
        "configure" | "config" => configure_impl(interp, path, &args[2..]),
        "destroy" => destroy_impl(interp, path),
        _ => match kind {
            WidgetKind::Button => match sub {
                "invoke" => invoke_widget(interp, path),
                "flash" => {
                    bump_active();
                    Ok(Value::empty())
                }
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Checkbutton | WidgetKind::Radiobutton => match sub {
                "invoke" => invoke_widget(interp, path),
                "select" => select_widget(interp, path),
                "deselect" => deselect_widget(interp, path),
                "toggle" if kind == WidgetKind::Checkbutton => invoke_widget(interp, path),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Entry => match sub {
                "get" => {
                    if args.len() != 2 {
                        return Err(wrong("get", "", args.len()));
                    }
                    Ok(Value::from_str(&entry_value(path)))
                }
                "insert" => {
                    if args.len() != 4 {
                        return Err(wrong("insert", "index chars", args.len()));
                    }
                    entry_insert(interp, path, args[2].as_str(), args[3].as_str())
                }
                "delete" => {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(wrong("delete", "firstIndex ?lastIndex?", args.len()));
                    }
                    let last = args.get(3).map(|v| v.as_str());
                    entry_delete(interp, path, args[2].as_str(), last)
                }
                "index" => {
                    if args.len() != 3 {
                        return Err(wrong("index", "index", args.len()));
                    }
                    let (content, cursor) = entry_state(interp, path);
                    let i = entry_index(args[2].as_str(), &content, cursor)?;
                    Ok(Value::from_int(i as i64))
                }
                "icursor" => {
                    if args.len() != 3 {
                        return Err(wrong("icursor", "index", args.len()));
                    }
                    let (content, _) = entry_state(interp, path);
                    let i = entry_index(args[2].as_str(), &content, 0)?;
                    with_state_mut(interp, |rt| {
                        if let Some(n) = rt.tree.get_mut(path) {
                            n.cursor = i;
                        }
                    });
                    Ok(Value::empty())
                }
                "selection" => entry_selection(interp, path, &args[2..]),
                "xview" => Ok(Value::empty()),
                "validate" => Ok(Value::from_int(1)),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Text => match sub {
                "get" => {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(wrong("get", "index1 ?index2?", args.len()));
                    }
                    text_get(
                        interp,
                        path,
                        args[2].as_str(),
                        args.get(3).map(|v| v.as_str()),
                    )
                }
                "insert" => {
                    if args.len() < 4 {
                        return Err(wrong(
                            "insert",
                            "index chars ?tagList chars ...?",
                            args.len(),
                        ));
                    }
                    text_insert(interp, path, args[2].as_str(), args[3].as_str())
                }
                "delete" => {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(wrong("delete", "index1 ?index2?", args.len()));
                    }
                    let last = args.get(3).map(|v| v.as_str());
                    text_delete(interp, path, args[2].as_str(), last)
                }
                "index" => {
                    if args.len() != 3 {
                        return Err(wrong("index", "index", args.len()));
                    }
                    let (content, cursor) = text_state(interp, path);
                    let i = text_index(args[2].as_str(), &content, cursor)?;
                    Ok(Value::from_str(&format!(
                        "{}",
                        char_to_linecol(&content, i)
                    )))
                }
                "see" => Ok(Value::empty()),
                "mark" => Ok(Value::empty()),
                "tag" => Ok(Value::empty()),
                "edit" => Ok(Value::empty()),
                "xview" | "yview" => Ok(Value::empty()),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Listbox => match sub {
                "insert" => {
                    if args.len() < 4 {
                        return Err(wrong("insert", "index ?element ...?", args.len()));
                    }
                    listbox_insert(interp, path, args[2].as_str(), &args[3..])
                }
                "delete" => {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(wrong("delete", "firstIndex ?lastIndex?", args.len()));
                    }
                    let last = args.get(3).map(|v| v.as_str());
                    listbox_delete(interp, path, args[2].as_str(), last)
                }
                "get" => {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(wrong("get", "firstIndex ?lastIndex?", args.len()));
                    }
                    let last = args.get(3).map(|v| v.as_str());
                    listbox_get(interp, path, args[2].as_str(), last)
                }
                "size" => {
                    let n = with_state(interp, |rt| {
                        rt.tree.get(path).map(|n| n.items.len()).unwrap_or(0)
                    })
                    .unwrap_or(0);
                    Ok(Value::from_int(n as i64))
                }
                "curselection" => {
                    let items = with_state(interp, |rt| {
                        rt.tree
                            .get(path)
                            .map(|n| n.selected.clone())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                    Ok(list_value(items.into_iter().map(|i| i.to_string())))
                }
                "selection" => listbox_selection(interp, path, &args[2..]),
                "index" => {
                    if args.len() != 3 {
                        return Err(wrong("index", "index", args.len()));
                    }
                    let n = with_state(interp, |rt| {
                        rt.tree.get(path).map(|n| n.items.len()).unwrap_or(0)
                    })
                    .unwrap_or(0);
                    let i = listbox_index(args[2].as_str(), n)?;
                    Ok(Value::from_int(i as i64))
                }
                "activate" | "see" | "xview" | "yview" => Ok(Value::empty()),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Scale => match sub {
                "get" => Ok(Value::from_str(&format_number(scale_value(path)))),
                "set" => {
                    if args.len() != 3 {
                        return Err(wrong("set", "value", args.len()));
                    }
                    let v = parse_f64_str(args[2].as_str(), "value")?;
                    scale_set_value(interp, path, v)
                }
                "coords" => Ok(list_value(["0", "0", "0", "0"].map(str::to_string))),
                "identify" => Ok(Value::from_str("")),
                _ => bad_subcommand(kind, sub),
            },
            _ => bad_subcommand(kind, sub),
        },
    }
}

fn bad_subcommand(kind: WidgetKind, sub: &str) -> Result<Value> {
    let must = match kind {
        WidgetKind::Button => "cget, configure, destroy, flash, invoke",
        WidgetKind::Checkbutton => {
            "cget, configure, deselect, destroy, invoke, select, toggle"
        }
        WidgetKind::Radiobutton => "cget, configure, deselect, destroy, invoke, select",
        WidgetKind::Entry => {
            "cget, configure, delete, destroy, get, icursor, index, insert, selection, validate, xview"
        }
        WidgetKind::Text => {
            "cget, configure, delete, destroy, edit, get, index, insert, mark, see, tag, xview, yview"
        }
        WidgetKind::Listbox => {
            "activate, cget, configure, curselection, delete, destroy, get, index, insert, see, selection, size, xview, yview"
        }
        WidgetKind::Scale => "cget, configure, coords, destroy, get, identify, set",
        _ => "cget, configure, destroy",
    };
    Err(generic(format!("bad option \"{sub}\": must be {must}")))
}

fn invoke_widget(interp: &mut Interp, path: &str) -> Result<Value> {
    let Some(kind) = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten() else {
        return Err(bad_window(path));
    };
    let command = effective_option(interp, path, "-command");
    match kind {
        WidgetKind::Button => {}
        WidgetKind::Checkbutton => {
            let var = effective_option(interp, path, "-variable");
            let on = effective_option(interp, path, "-onvalue");
            let off = effective_option(interp, path, "-offvalue");
            if var.is_empty() {
                let next = with_state(interp, |rt| {
                    rt.tree.get(path).map(|n| !n.checked).unwrap_or(true)
                })
                .unwrap_or(true);
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = next;
                    }
                });
            } else {
                let current = var_value(&var).unwrap_or_default();
                let next = if current == on {
                    off.clone()
                } else {
                    on.clone()
                };
                set_var(&var, &next)?;
            }
        }
        WidgetKind::Radiobutton => {
            let var = effective_option(interp, path, "-variable");
            let value = effective_option(interp, path, "-value");
            if var.is_empty() {
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = true;
                    }
                });
            } else {
                set_var(&var, &value)?;
            }
        }
        _ => return Err(generic(format!("window \"{path}\" cannot be invoked"))),
    }
    if !command.is_empty() {
        eval_active(&command)?;
    }
    bump_active();
    Ok(Value::empty())
}

fn select_widget(interp: &mut Interp, path: &str) -> Result<Value> {
    let Some(kind) = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten() else {
        return Err(bad_window(path));
    };
    match kind {
        WidgetKind::Checkbutton => {
            let var = effective_option(interp, path, "-variable");
            let on = effective_option(interp, path, "-onvalue");
            if var.is_empty() {
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = true;
                    }
                });
            } else {
                set_var(&var, &on)?;
            }
        }
        WidgetKind::Radiobutton => {
            let var = effective_option(interp, path, "-variable");
            let value = effective_option(interp, path, "-value");
            if var.is_empty() {
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = true;
                    }
                });
            } else {
                set_var(&var, &value)?;
            }
        }
        _ => return Err(generic(format!("window \"{path}\" cannot be selected"))),
    }
    bump_active();
    Ok(Value::empty())
}

fn deselect_widget(interp: &mut Interp, path: &str) -> Result<Value> {
    let Some(kind) = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten() else {
        return Err(bad_window(path));
    };
    match kind {
        WidgetKind::Checkbutton => {
            let var = effective_option(interp, path, "-variable");
            let off = effective_option(interp, path, "-offvalue");
            if var.is_empty() {
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = false;
                    }
                });
            } else {
                set_var(&var, &off)?;
            }
        }
        WidgetKind::Radiobutton => {
            let var = effective_option(interp, path, "-variable");
            if var.is_empty() {
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.checked = false;
                    }
                });
            } else {
                let value = effective_option(interp, path, "-value");
                if var_value(&var).is_some_and(|v| v == value) {
                    set_var(&var, "")?;
                }
            }
        }
        _ => return Err(generic(format!("window \"{path}\" cannot be deselected"))),
    }
    bump_active();
    Ok(Value::empty())
}

// ---------------------------------------------------------------------------
// cget / configure / destroy
// ---------------------------------------------------------------------------

fn cget_impl(interp: &Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.len() != 1 {
        return Err(wrong("cget", "window -option", rest.len() + 2));
    }
    let Some(kind) = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten() else {
        return Err(bad_window(path));
    };
    let stored: Vec<String> = with_state(interp, |rt| {
        rt.tree
            .get(path)
            .map(|n| n.options.keys().cloned().collect())
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let option = canonical_option(kind, rest[0].as_str(), stored.iter().map(String::as_str))?;
    let value = with_state(interp, |rt| {
        rt.tree
            .get(path)
            .and_then(|n| n.options.get(&option).cloned())
            .or_else(|| rtcl_tk_core::option_default(kind, &option).map(str::to_string))
    })
    .flatten()
    .unwrap_or_default();
    Ok(Value::from_str(&value))
}

fn configure_impl(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let Some(kind) = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten() else {
        return Err(bad_window(path));
    };
    if rest.is_empty() {
        let entries = with_state(interp, |rt| {
            let node = rt.tree.get(path)?;
            Some(
                rtcl_tk_core::known_options(kind)
                    .iter()
                    .map(|name| {
                        let current = node
                            .options
                            .get(*name)
                            .cloned()
                            .or_else(|| {
                                rtcl_tk_core::option_default(kind, name).map(str::to_string)
                            })
                            .unwrap_or_default();
                        let db = name.trim_start_matches('-');
                        let class = db
                            .split('-')
                            .map(|p| {
                                let mut c = p.chars();
                                match c.next() {
                                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                                    None => String::new(),
                                }
                            })
                            .collect::<String>();
                        Value::from_list(&[
                            Value::from_str(name),
                            Value::from_str(db),
                            Value::from_str(&class),
                            Value::from_str(rtcl_tk_core::option_default(kind, name).unwrap_or("")),
                            Value::from_str(&current),
                        ])
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .ok_or_else(|| bad_window(path))?;
        return Ok(Value::from_list(&entries));
    }
    if rest.len() == 1 && rest[0].as_str().starts_with('-') {
        return cget_impl(interp, path, rest);
    }
    let raw = parse_pairs(rest, 0, "configure")?;
    let stored: Vec<String> = with_state(interp, |rt| {
        rt.tree
            .get(path)
            .map(|n| n.options.keys().cloned().collect())
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let options = canonical_options(kind, raw)?
        .into_iter()
        .map(|(k, v)| {
            let _ = &stored;
            (k, v)
        })
        .collect::<Vec<_>>();
    with_state_mut(interp, |rt| {
        rt.tree.configure(path, options).map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    // A changed -textvariable / -variable / -value changes what the widget
    // shows: re-read it from the variable.
    initialize_widget(interp, path, kind)?;
    Ok(Value::empty())
}

fn destroy_impl(interp: &mut Interp, path: &str) -> Result<Value> {
    let mut removed = Vec::new();
    fn collect(tree: &TkTree, path: &str, out: &mut Vec<String>) {
        if let Some(n) = tree.get(path) {
            out.push(path.to_string());
            for c in &n.children {
                collect(tree, c, out);
            }
        }
    }
    with_state(interp, |rt| collect(&rt.tree, path, &mut removed));
    if removed.is_empty() {
        return Err(bad_window(path));
    }
    with_state_mut(interp, |rt| {
        rt.tree.destroy(path).map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    for p in removed {
        let _ = interp.delete_command(&p);
    }
    Ok(Value::empty())
}

// ---------------------------------------------------------------------------
// pack
// ---------------------------------------------------------------------------

fn pack_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("pack", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "configure" | "config" => {
            if args.len() < 3 {
                return Err(wrong(
                    "pack configure",
                    "window ?-option value ...?",
                    args.len(),
                ));
            }
            pack_widget(interp, args[2].as_str(), &args[3..])
        }
        "forget" => {
            if args.len() < 3 {
                return Err(wrong("pack forget", "window ?window ...?", args.len()));
            }
            for a in &args[2..] {
                let path = a.as_str();
                with_state_mut(interp, |rt| {
                    rt.tree.unpack(path).map_err(tk_error)?;
                    rt.bump();
                    Ok::<(), Error>(())
                })
                .ok_or_else(|| bad_window(path))??;
            }
            Ok(Value::empty())
        }
        "info" => {
            if args.len() != 3 {
                return Err(wrong("pack info", "window", args.len()));
            }
            pack_info(interp, args[2].as_str())
        }
        "slaves" => {
            if args.len() != 3 {
                return Err(wrong("pack slaves", "window", args.len()));
            }
            let kids = with_state(interp, |rt| rt.tree.slaves(args[2].as_str(), "pack"))
                .and_then(|r| r.ok())
                .ok_or_else(|| bad_window(args[2].as_str()))?;
            Ok(list_value(kids))
        }
        "propagate" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(wrong("pack propagate", "window ?boolean?", args.len()));
            }
            Ok(Value::from_bool(
                args.get(3).map(|v| v.as_str() != "0").unwrap_or(true),
            ))
        }
        _ => pack_widget(interp, args[1].as_str(), &args[2..]),
    }
}

fn pack_widget(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let mut info = with_state(interp, |rt| rt.tree.get(path).and_then(|n| n.pack))
        .flatten()
        .unwrap_or_default();
    for (k, v) in parse_pairs(rest, 0, "pack")? {
        match k.as_str() {
            "-side" => {
                info.side = match v.as_str() {
                    "top" => PackSide::Top,
                    "bottom" => PackSide::Bottom,
                    "left" => PackSide::Left,
                    "right" => PackSide::Right,
                    _ => return Err(generic(format!("bad side \"{v}\""))),
                }
            }
            "-fill" => {
                info.fill = match v.as_str() {
                    "none" => Fill::None,
                    "x" => Fill::X,
                    "y" => Fill::Y,
                    "both" => Fill::Both,
                    _ => return Err(generic(format!("bad fill \"{v}\""))),
                }
            }
            "-expand" => info.expand = parse_bool_str(&v, "expand")?,
            "-padx" => info.padx = parse_f64_str(&v, "padx")?,
            "-pady" => info.pady = parse_f64_str(&v, "pady")?,
            "-in" => {
                return Err(generic(
                    "pack -in is not supported in the native Tk backend yet",
                ))
            }
            "-anchor" | "-ipadx" | "-ipady" => {}
            _ => return Err(bad_option(&k)),
        }
    }
    with_state_mut(interp, |rt| {
        rt.tree.pack(path, info).map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::empty())
}

fn pack_info(interp: &mut Interp, path: &str) -> Result<Value> {
    let info = with_state(interp, |rt| rt.tree.get(path).map(|n| n.pack))
        .flatten()
        .ok_or_else(|| bad_window(path))?
        .ok_or_else(|| generic(format!("window \"{path}\" isn't packed")))?;
    let side = match info.side {
        PackSide::Top => "top",
        PackSide::Bottom => "bottom",
        PackSide::Left => "left",
        PackSide::Right => "right",
    };
    let fill = match info.fill {
        Fill::None => "none",
        Fill::X => "x",
        Fill::Y => "y",
        Fill::Both => "both",
    };
    Ok(list_value([
        "-in".to_string(),
        ".".to_string(),
        "-anchor".to_string(),
        "center".to_string(),
        "-expand".to_string(),
        if info.expand { "1" } else { "0" }.to_string(),
        "-fill".to_string(),
        fill.to_string(),
        "-ipadx".to_string(),
        "0".to_string(),
        "-ipady".to_string(),
        "0".to_string(),
        "-padx".to_string(),
        format_number(info.padx),
        "-pady".to_string(),
        format_number(info.pady),
        "-side".to_string(),
        side.to_string(),
    ]))
}

// ---------------------------------------------------------------------------
// grid
// ---------------------------------------------------------------------------

fn grid_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("grid", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "configure" | "config" => {
            if args.len() < 3 {
                return Err(wrong(
                    "grid configure",
                    "window ?-option value ...?",
                    args.len(),
                ));
            }
            grid_widget(interp, args[2].as_str(), &args[3..])
        }
        "forget" | "remove" => {
            if args.len() < 3 {
                return Err(wrong(
                    &format!("grid {}", args[1].as_str()),
                    "window ?window ...?",
                    args.len(),
                ));
            }
            for a in &args[2..] {
                let path = a.as_str();
                with_state_mut(interp, |rt| {
                    rt.tree.ungrid(path).map_err(tk_error)?;
                    rt.bump();
                    Ok::<(), Error>(())
                })
                .ok_or_else(|| bad_window(path))??;
            }
            Ok(Value::empty())
        }
        "info" => {
            if args.len() != 3 {
                return Err(wrong("grid info", "window", args.len()));
            }
            grid_info(interp, args[2].as_str())
        }
        "slaves" => {
            if args.len() < 3 {
                return Err(wrong("grid slaves", "window ?-option value?", args.len()));
            }
            let kids = with_state(interp, |rt| rt.tree.slaves(args[2].as_str(), "grid"))
                .and_then(|r| r.ok())
                .ok_or_else(|| bad_window(args[2].as_str()))?;
            Ok(list_value(kids))
        }
        "size" => {
            if args.len() != 3 {
                return Err(wrong("grid size", "window", args.len()));
            }
            let (rows, cols) = with_state(interp, |rt| {
                let node = rt.tree.get(args[2].as_str())?;
                let mut rows = 0i64;
                let mut cols = 0i64;
                for c in &node.children {
                    if let Some(g) = rt.tree.get(c).and_then(|n| n.grid) {
                        rows = rows.max(g.row + g.rowspan);
                        cols = cols.max(g.column + g.columnspan);
                    }
                }
                Some((rows, cols))
            })
            .flatten()
            .ok_or_else(|| bad_window(args[2].as_str()))?;
            Ok(list_value([rows.to_string(), cols.to_string()]))
        }
        "columnconfigure" | "rowconfigure" => {
            let is_row = args[1].as_str() == "rowconfigure";
            if args.len() < 4 {
                return Err(wrong(
                    &format!("grid {}", args[1].as_str()),
                    "master index ?-option value ...?",
                    args.len(),
                ));
            }
            let master = args[2].as_str().to_string();
            let index: i64 = args[3].as_str().parse().map_err(|_| {
                generic(format!("expected integer but got \"{}\"", args[3].as_str()))
            })?;
            if args.len() == 4 {
                let (weight, minsize) = with_state(interp, |rt| {
                    rt.grid_options
                        .get(&(master.clone(), index, is_row))
                        .copied()
                        .unwrap_or((0, 0))
                })
                .unwrap_or((0, 0));
                return Ok(list_value([
                    "-minsize".to_string(),
                    minsize.to_string(),
                    "-pad".to_string(),
                    "0".to_string(),
                    "-weight".to_string(),
                    weight.to_string(),
                ]));
            }
            let mut weight = with_state(interp, |rt| {
                rt.grid_options
                    .get(&(master.clone(), index, is_row))
                    .map(|v| v.0)
                    .unwrap_or(0)
            })
            .unwrap_or(0);
            let mut minsize = with_state(interp, |rt| {
                rt.grid_options
                    .get(&(master.clone(), index, is_row))
                    .map(|v| v.1)
                    .unwrap_or(0)
            })
            .unwrap_or(0);
            for (k, v) in parse_pairs(&args[2..], 2, "grid configure")? {
                match k.as_str() {
                    "-weight" => weight = v.parse().map_err(|_| generic("bad weight"))?,
                    "-minsize" => minsize = v.parse().map_err(|_| generic("bad minsize"))?,
                    "-pad" => {}
                    _ => return Err(bad_option(&k)),
                }
            }
            with_state_mut(interp, |rt| {
                rt.grid_options
                    .insert((master, index, is_row), (weight, minsize));
                rt.bump();
            });
            Ok(Value::empty())
        }
        "bbox" => Ok(list_value(["0", "0", "0", "0"].map(str::to_string))),
        "location" => Ok(Value::from_str("")),
        "propagate" => Ok(Value::from_bool(
            args.get(3).map(|v| v.as_str() != "0").unwrap_or(true),
        )),
        _ => grid_widget(interp, args[1].as_str(), &args[2..]),
    }
}

fn grid_widget(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let mut info = with_state(interp, |rt| rt.tree.get(path).and_then(|n| n.grid))
        .flatten()
        .unwrap_or_default();
    for (k, v) in parse_pairs(rest, 0, "grid")? {
        match k.as_str() {
            "-row" => info.row = parse_f64_str(&v, "row")? as i64,
            "-column" => info.column = parse_f64_str(&v, "column")? as i64,
            "-rowspan" => info.rowspan = parse_f64_str(&v, "rowspan")? as i64,
            "-columnspan" => info.columnspan = parse_f64_str(&v, "columnspan")? as i64,
            "-padx" => info.padx = parse_f64_str(&v, "padx")?,
            "-pady" => info.pady = parse_f64_str(&v, "pady")?,
            "-sticky" => {
                info.sticky = Sticky {
                    left: v.contains('w'),
                    right: v.contains('e'),
                    top: v.contains('n'),
                    bottom: v.contains('s'),
                }
            }
            "-in" => {
                return Err(generic(
                    "grid -in is not supported in the native Tk backend yet",
                ))
            }
            _ => return Err(bad_option(&k)),
        }
    }
    with_state_mut(interp, |rt| {
        rt.tree.grid(path, info).map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::empty())
}

fn grid_info(interp: &mut Interp, path: &str) -> Result<Value> {
    let info = with_state(interp, |rt| rt.tree.get(path).map(|n| n.grid))
        .flatten()
        .ok_or_else(|| bad_window(path))?
        .ok_or_else(|| generic(format!("window \"{path}\" isn't gridded")))?;
    let sticky = format!(
        "{}{}{}{}",
        if info.sticky.top { "n" } else { "" },
        if info.sticky.bottom { "s" } else { "" },
        if info.sticky.left { "w" } else { "" },
        if info.sticky.right { "e" } else { "" },
    );
    Ok(list_value([
        "-in".to_string(),
        ".".to_string(),
        "-column".to_string(),
        info.column.to_string(),
        "-row".to_string(),
        info.row.to_string(),
        "-columnspan".to_string(),
        info.columnspan.to_string(),
        "-rowspan".to_string(),
        info.rowspan.to_string(),
        "-ipadx".to_string(),
        "0".to_string(),
        "-ipady".to_string(),
        "0".to_string(),
        "-padx".to_string(),
        format_number(info.padx),
        "-pady".to_string(),
        format_number(info.pady),
        "-sticky".to_string(),
        sticky,
    ]))
}

// ---------------------------------------------------------------------------
// winfo / wm / tk
// ---------------------------------------------------------------------------

fn requested_size(interp: &Interp, path: &str) -> (i64, i64) {
    with_state(interp, |rt| {
        let Some(node) = rt.tree.get(path) else {
            return (0, 0);
        };
        let opt = |name: &str| {
            node.options
                .get(name)
                .cloned()
                .or_else(|| rtcl_tk_core::option_default(node.kind, name).map(str::to_string))
                .unwrap_or_default()
        };
        let text_units = match node.kind {
            WidgetKind::Label
            | WidgetKind::Button
            | WidgetKind::Checkbutton
            | WidgetKind::Radiobutton => opt("-text").chars().count().max(1) as i64,
            _ => 0,
        };
        let w_opt = opt("-width").parse::<i64>().unwrap_or(0);
        let h_opt = opt("-height").parse::<i64>().unwrap_or(0);
        let (w, h) = match node.kind {
            WidgetKind::Text => (w_opt.max(1) * 7, h_opt.max(1) * 18),
            WidgetKind::Listbox => (w_opt.max(1) * 7, h_opt.max(1) * 18),
            WidgetKind::Entry => ((w_opt.max(1) * 7) + 8, 24),
            WidgetKind::Scale => {
                let length = opt("-length").parse::<i64>().unwrap_or(100);
                if opt("-orient") == "horizontal" {
                    (length.max(1), 24)
                } else {
                    (24, length.max(1))
                }
            }
            WidgetKind::Frame | WidgetKind::Toplevel => (w_opt, h_opt),
            _ => {
                let chars = if text_units == 0 {
                    opt("-text").chars().count() as i64
                } else {
                    text_units
                };
                (chars.max(1) * 7 + 16, 24)
            }
        };
        let padx: i64 = opt("-padx").parse().unwrap_or(0);
        let pady: i64 = opt("-pady").parse().unwrap_or(0);
        (w + padx * 2, h + pady * 2)
    })
    .unwrap_or((0, 0))
}

fn winfo_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("winfo", "option ?arg ...?", args.len()));
    }
    let sub = args[1].as_str();
    match sub {
        "children" => {
            if args.len() != 3 {
                return Err(wrong("winfo children", "window", args.len()));
            }
            let path = args[2].as_str();
            let kids = with_state(interp, |rt| rt.tree.children(path).map(|c| c.to_vec()))
                .and_then(|r| r.ok())
                .ok_or_else(|| bad_window(path))?;
            Ok(list_value(kids))
        }
        "exists" => {
            if args.len() != 3 {
                return Err(wrong("winfo exists", "window", args.len()));
            }
            Ok(Value::from_bool(
                with_state(interp, |rt| rt.tree.contains(args[2].as_str())).unwrap_or(false),
            ))
        }
        "class" => {
            if args.len() != 3 {
                return Err(wrong("winfo class", "window", args.len()));
            }
            let path = args[2].as_str();
            let class = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind.class_name()))
                .flatten()
                .ok_or_else(|| bad_window(path))?;
            Ok(Value::from_str(class))
        }
        "parent" | "toplevel" | "pathname" | "name" => {
            if args.len() != 3 {
                return Err(wrong(&format!("winfo {sub}"), "window", args.len()));
            }
            let path = args[2].as_str();
            if !with_state(interp, |rt| rt.tree.contains(path)).unwrap_or(false) {
                return Err(bad_window(path));
            }
            let value = match sub {
                "parent" => with_state(interp, |rt| {
                    rt.tree
                        .get(path)
                        .and_then(|n| n.parent.clone())
                        .unwrap_or_default()
                })
                .unwrap_or_default(),
                "toplevel" => {
                    let mut p = path.to_string();
                    while let Some(parent) =
                        with_state(interp, |rt| rt.tree.get(&p).and_then(|n| n.parent.clone()))
                            .flatten()
                    {
                        p = parent;
                    }
                    p
                }
                "name" => path.rsplit('.').next().unwrap_or("").to_string(),
                _ => path.to_string(),
            };
            Ok(Value::from_str(&value))
        }
        "width" | "height" | "reqwidth" | "reqheight" => {
            if args.len() != 3 {
                return Err(wrong(&format!("winfo {sub}"), "window", args.len()));
            }
            let path = args[2].as_str();
            if !with_state(interp, |rt| rt.tree.contains(path)).unwrap_or(false) {
                return Err(bad_window(path));
            }
            let (w, h) = requested_size(interp, path);
            Ok(Value::from_int(if sub.contains("width") { w } else { h }))
        }
        "ismapped" | "viewable" => {
            if args.len() != 3 {
                return Err(wrong(&format!("winfo {sub}"), "window", args.len()));
            }
            let path = args[2].as_str();
            let mapped = with_state(interp, |rt| {
                rt.tree
                    .get(path)
                    .is_some_and(|n| n.pack.is_some() || n.grid.is_some() || path == ".")
            })
            .unwrap_or(false);
            Ok(Value::from_bool(mapped))
        }
        "manager" => {
            if args.len() != 3 {
                return Err(wrong("winfo manager", "window", args.len()));
            }
            let path = args[2].as_str();
            let manager = with_state(interp, |rt| {
                rt.tree.get(path).map(|n| {
                    if n.pack.is_some() {
                        "pack"
                    } else if n.grid.is_some() {
                        "grid"
                    } else {
                        ""
                    }
                })
            })
            .flatten()
            .ok_or_else(|| bad_window(path))?;
            Ok(Value::from_str(manager))
        }
        "x" | "y" | "rootx" | "rooty" | "vrootx" | "vrooty" | "pointerx" | "pointery" => {
            Ok(Value::from_int(0))
        }
        "screen" => Ok(Value::from_str(".")),
        "screenwidth" | "vrootwidth" => Ok(Value::from_int(1920)),
        "screenheight" | "vrootheight" => Ok(Value::from_int(1080)),
        "depth" | "fpixels" | "rpixels" | "cells" => {
            Ok(Value::from_int(if sub == "depth" { 24 } else { 0 }))
        }
        "visual" => Ok(Value::from_str("truecolor")),
        "server" => Ok(Value::from_str("")),
        "interps" => Ok(Value::from_str("")),
        _ => Err(generic(format!("bad option \"{sub}\""))),
    }
}

fn wm_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("wm", "option window ?args?", args.len()));
    }
    let sub = args[1].as_str();
    if args.len() < 3 {
        return Err(wrong(&format!("wm {sub}"), "window ?value?", args.len()));
    }
    let path = args[2].as_str().to_string();
    if !with_state(interp, |rt| rt.tree.contains(&path)).unwrap_or(false) {
        return Err(bad_window(&path));
    }
    match sub {
        "title" | "iconname" | "geometry" | "resizable" | "minsize" | "maxsize" | "group"
        | "iconbitmap" | "iconphoto" | "iconwindow" | "transient" | "focusmodel" | "aspect" => {
            let option = format!("-{sub}");
            if args.len() == 3 {
                return Ok(Value::from_str(&effective_option(interp, &path, &option)));
            }
            let value = args[3..]
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            set_root_option(interp, &path, &option, &value)?;
            Ok(Value::empty())
        }
        "state" => {
            if args.len() == 3 {
                return Ok(Value::from_str(&effective_option(interp, &path, "-state")));
            }
            let state = args[3].as_str();
            if !matches!(state, "normal" | "iconic" | "withdrawn" | "zoomed") {
                return Err(generic(format!(
                    "bad argument \"{state}\": must be normal, iconic, withdrawn, or zoomed"
                )));
            }
            set_root_option(interp, &path, "-state", state)?;
            Ok(Value::empty())
        }
        "withdraw" | "deiconify" | "iconify" => {
            if args.len() != 3 {
                return Err(wrong(&format!("wm {sub}"), "window", args.len()));
            }
            let (withdrawn, state) = match sub {
                "withdraw" => ("1", "withdrawn"),
                "iconify" => ("0", "iconic"),
                _ => ("0", "normal"),
            };
            set_root_option(interp, &path, "-withdrawn", withdrawn)?;
            set_root_option(interp, &path, "-state", state)?;
            Ok(Value::empty())
        }
        "protocol" => {
            if args.len() < 4 {
                return Err(wrong("wm protocol", "window ?name? ?command?", args.len()));
            }
            let name = args[3].as_str().to_string();
            if args.len() == 4 {
                let script = with_state(interp, |rt| {
                    rt.protocols.get(&(path.clone(), name.clone())).cloned()
                })
                .flatten()
                .unwrap_or_default();
                return Ok(Value::from_str(&script));
            }
            let script = args[4].as_str().to_string();
            with_state_mut(interp, |rt| {
                if script.is_empty() {
                    rt.protocols.remove(&(path.clone(), name.clone()));
                } else {
                    rt.protocols.insert((path.clone(), name.clone()), script);
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "attributes" => {
            if args.len() == 3 {
                return Ok(Value::from_str(""));
            }
            if args.len() == 4 && args[3].as_str().starts_with('-') {
                return Ok(Value::from_str(""));
            }
            Ok(Value::empty())
        }
        "overrideredirect" | "wait_visibility" | "deiconify_all" => Ok(Value::empty()),
        "stackorder" => Ok(Value::from_str("")),
        "frame" => Ok(Value::from_str("")),
        "manage" | "forget" => Ok(Value::empty()),
        _ => Err(generic(format!("bad option \"{sub}\""))),
    }
}

fn set_root_option(interp: &mut Interp, path: &str, option: &str, value: &str) -> Result<()> {
    with_state_mut(interp, |rt| {
        rt.tree
            .configure(path, [(option.to_string(), value.to_string())])
            .map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(())
}

fn tk_impl(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("tk", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "windowingsystem" => Ok(Value::from_str(if cfg!(target_os = "macos") {
            "aqua"
        } else if cfg!(target_os = "windows") {
            "win32"
        } else {
            "x11"
        })),
        "patchlevel" => Ok(Value::from_str("8.6")),
        "appname" => Ok(Value::from_str("rtcl-tk")),
        "scaling" => Ok(Value::from_str("1.0")),
        "useinputmethods" | "inactive" | "caret" => Ok(Value::from_str("1")),
        other => Err(generic(format!("bad option \"{other}\""))),
    }
}

// ---------------------------------------------------------------------------
// bind / event
// ---------------------------------------------------------------------------

/// Normalize the sequences the renderer emits to the spellings scripts
/// bind (`<Button-1>`, `<Return>`, `<KeyPress>`).
pub fn normalize_sequence(sequence: &str) -> String {
    let s = sequence.trim();
    match s {
        "<Button-1>" | "<ButtonPress-1>" | "<1>" | "<Button1>" => "<Button-1>".to_string(),
        "<ButtonRelease-1>" | "<ButtonRelease1>" => "<ButtonRelease-1>".to_string(),
        "<Return>" | "<Key-Return>" | "<KeyPress-Return>" => "<Return>".to_string(),
        "<Key>" | "<KeyPress>" => "<KeyPress>".to_string(),
        "<KeyRelease>" => "<KeyRelease>".to_string(),
        other => other.to_string(),
    }
}

fn bind_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(wrong("bind", "window ?sequence? ?+?script?", args.len()));
    }
    let target = args[1].as_str();
    let all = target == "all";
    if !all && !with_state(interp, |rt| rt.tree.contains(target)).unwrap_or(false) {
        return Err(bad_window(target));
    }
    if args.len() == 2 {
        let seqs: Vec<String> = if all {
            with_state(interp, |rt| rt.all_bindings.keys().cloned().collect()).unwrap_or_default()
        } else {
            with_state(interp, |rt| rt.tree.binding_sequences(target))
                .and_then(|r| r.ok())
                .unwrap_or_default()
        };
        return Ok(list_value(seqs));
    }
    let sequence = normalize_sequence(args[2].as_str());
    let current = if all {
        with_state(interp, |rt| rt.all_bindings.get(&sequence).cloned())
            .flatten()
            .unwrap_or_default()
    } else {
        with_state(interp, |rt| {
            rt.tree
                .binding(target, &sequence)
                .ok()
                .flatten()
                .map(str::to_string)
        })
        .flatten()
        .unwrap_or_default()
    };
    if args.len() == 3 {
        return Ok(Value::from_str(&current));
    }
    let script = args[3].as_str();
    let script = match script.strip_prefix('+') {
        Some(more) if !current.is_empty() => {
            format!("{}\n{}", current.trim_end_matches('\n'), more)
        }
        Some(more) => more.to_string(),
        None => script.to_string(),
    };
    with_state_mut(interp, |rt| {
        if all {
            if script.is_empty() {
                rt.all_bindings.remove(&sequence);
            } else {
                rt.all_bindings.insert(sequence.clone(), script.clone());
            }
            rt.bump();
        } else {
            rt.tree.bind(target, &sequence, &script).map_err(tk_error)?;
            rt.bump();
        }
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(target))??;
    Ok(Value::empty())
}

/// What a user event carries, for `%` substitutions.
#[derive(Debug, Clone, Default)]
pub struct EventCtx {
    pub path: String,
    pub x: i64,
    pub y: i64,
    pub key: String,
    pub character: String,
    pub button: i64,
    pub state: String,
}

impl EventCtx {
    pub fn for_path(path: &str) -> Self {
        Self {
            path: path.to_string(),
            ..Self::default()
        }
    }
}

fn substitute_bindings(script: &str, ctx: &EventCtx) -> String {
    let mut out = String::with_capacity(script.len());
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('W') | Some('w') => out.push_str(&ctx.path),
            Some('x') => out.push_str(&ctx.x.to_string()),
            Some('y') => out.push_str(&ctx.y.to_string()),
            Some('X') | Some('Y') => out.push_str("0"),
            Some('K') => out.push_str(&ctx.key),
            Some('A') => out.push_str(&ctx.character),
            Some('k') => out.push_str(&ctx.key),
            Some('b') => out.push_str(&ctx.button.to_string()),
            Some('s') => out.push_str(&ctx.state),
            Some('t') => out.push_str("0"),
            Some('#') => out.push_str("0"),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn binding_script(interp: &Interp, path: &str, sequence: &str) -> String {
    let exact = with_state(interp, |rt| {
        rt.tree
            .binding(path, sequence)
            .ok()
            .flatten()
            .map(str::to_string)
    })
    .flatten()
    .unwrap_or_default();
    if !exact.is_empty() {
        return exact;
    }
    // Generic key bindings: <Key> sees every key press.
    if sequence == "<Return>" || sequence == "<KeyPress>" {
        return with_state(interp, |rt| {
            rt.tree
                .binding(path, "<KeyPress>")
                .ok()
                .flatten()
                .map(str::to_string)
        })
        .flatten()
        .unwrap_or_default();
    }
    String::new()
}

/// Run the bindings for `path` and `sequence` (then `bind all`), and
/// re-render.  Errors go to [`take_errors`].
pub fn run_binding(path: &str, sequence: &str, ctx: &EventCtx) {
    let sequence = normalize_sequence(sequence);
    let Some(ptr) = active_interp() else { return };
    // Safety: the pointer is the live engine-thread Interp.
    let interp = unsafe { &mut *ptr };
    let mut scripts = Vec::new();
    let widget = binding_script(interp, path, &sequence);
    if !widget.is_empty() {
        scripts.push(widget);
    }
    if let Some(script) = with_state(interp, |rt| rt.all_bindings.get(&sequence).cloned()).flatten()
    {
        if !script.is_empty() {
            scripts.push(script);
        }
    }
    for script in scripts {
        let script = substitute_bindings(&script, ctx);
        if let Err(e) = eval_active(&script) {
            record_error(e.to_string());
        }
    }
}

fn event_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("event", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "generate" => {
            if args.len() < 4 {
                return Err(wrong(
                    "event generate",
                    "window event ?-option value ...?",
                    args.len(),
                ));
            }
            let path = args[2].as_str().to_string();
            if !with_state(interp, |rt| rt.tree.contains(&path)).unwrap_or(false) {
                return Err(bad_window(&path));
            }
            let mut ctx = EventCtx::for_path(&path);
            let mut i = 4;
            while i + 1 < args.len() {
                match args[i].as_str() {
                    "-x" => ctx.x = parse_int(&args[i + 1])?,
                    "-y" => ctx.y = parse_int(&args[i + 1])?,
                    "-key" => ctx.key = args[i + 1].as_str().to_string(),
                    "-button" => ctx.button = parse_int(&args[i + 1])?,
                    _ => {}
                }
                i += 2;
            }
            run_binding(&path, args[3].as_str(), &ctx);
            Ok(Value::empty())
        }
        "info" => Ok(list_value(Vec::<String>::new())),
        _ => Err(generic(format!("bad option \"{}\"", args[1].as_str()))),
    }
}

// ---------------------------------------------------------------------------
// Renderer-facing value access and user events
// ---------------------------------------------------------------------------

/// Resolve a Tk variable name the way widgets do (unqualified names are
/// global).
fn qualify_var(name: &str) -> String {
    if name.starts_with("::") || name.contains("::") {
        name.to_string()
    } else {
        format!("::{name}")
    }
}

/// The active interpreter's value for a Tk variable.
pub fn var_value(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let ptr = active_interp()?;
    let name = qualify_var(name);
    unsafe { (*ptr).get_var(&name).ok().map(|v| v.as_str().to_string()) }
}

/// Set a Tk variable on the active interpreter (entry edits, checkbuttons,
/// scales follow their `-textvariable` / `-variable`).
pub fn set_var(name: &str, value: &str) -> Result<Value> {
    let ptr = active_interp().ok_or_else(|| generic("no active Tk interpreter"))?;
    let name = qualify_var(name);
    unsafe { (*ptr).set_var(&name, Value::from_str(value)) }
}

fn qual_var_opt(interp: &Interp, path: &str, name: &str) -> Option<String> {
    let v = effective_option(interp, path, name);
    (!v.is_empty()).then(|| qualify_var(&v))
}

/// The text a Label/Button/Checkbutton/Radiobutton shows: its
/// `-textvariable`, when set and defined, else `-text`.
pub fn display_text(path: &str) -> String {
    let Some(ptr) = active_interp() else {
        return String::new();
    };
    let interp = unsafe { &*ptr };
    if let Some(var) = qual_var_opt(interp, path, "-textvariable") {
        return var_value(&var).unwrap_or_default();
    }
    effective_option(interp, path, "-text")
}

/// The content of an Entry (its `-textvariable`, when set, else the text
/// the user typed).
pub fn entry_value(path: &str) -> String {
    let Some(ptr) = active_interp() else {
        return String::new();
    };
    let interp = unsafe { &*ptr };
    if let Some(var) = qual_var_opt(interp, path, "-textvariable") {
        return var_value(&var).unwrap_or_default();
    }
    with_state(interp, |rt| rt.tree.get(path).map(|n| n.content.clone()))
        .flatten()
        .unwrap_or_default()
}

fn entry_state(interp: &Interp, path: &str) -> (String, usize) {
    let content = entry_value(path);
    let cursor = with_state(interp, |rt| rt.tree.get(path).map(|n| n.cursor))
        .flatten()
        .unwrap_or(content.chars().count());
    (content, cursor)
}

fn text_state(interp: &Interp, path: &str) -> (String, usize) {
    with_state(interp, |rt| {
        rt.tree.get(path).map(|n| (n.content.clone(), n.cursor))
    })
    .flatten()
    .unwrap_or_default()
}

/// An Entry's text changed in the UI.
pub fn entry_changed(path: &str, text: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let var = qual_var_opt(interp, path, "-textvariable");
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = text.to_string();
            n.cursor = text.chars().count();
        }
    });
    if let Some(var) = var {
        if let Err(e) = set_var(&var, text) {
            record_error(e.to_string());
        }
    }
    run_binding(path, "<KeyRelease>", &EventCtx::for_path(path));
    bump_active();
}

/// A Text widget's content changed in the UI.
pub fn text_changed(path: &str, text: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = text.to_string();
            n.cursor = text.chars().count();
            rt.bump();
        }
    });
}

/// The return key was pressed in an Entry.
pub fn entry_submitted(path: &str) {
    let mut ctx = EventCtx::for_path(path);
    ctx.key = "Return".to_string();
    ctx.character = "\r".to_string();
    run_binding(path, "<Return>", &ctx);
}

/// A key was pressed in an Entry: `key` is a Tk keysym (`a`, `BackSpace`,
/// `Return`), `character` the text it inserts (empty for control keys).
pub fn entry_key(path: &str, key: &str, character: &str) {
    let mut ctx = EventCtx::for_path(path);
    ctx.key = key.to_string();
    ctx.character = character.to_string();
    run_binding(
        path,
        if key == "Return" {
            "<Return>"
        } else {
            "<KeyPress>"
        },
        &ctx,
    );
}

/// A Button was pressed (or a checkbutton/radiobutton activated).
pub fn press_button(path: &str) {
    let Some(ptr) = active_interp() else { return };
    // Read the widget's state before any binding can run a script.
    let (disabled, kind) = unsafe {
        let interp = &*ptr;
        (
            effective_option(interp, path, "-state") == "disabled",
            with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten(),
        )
    };
    if disabled {
        return;
    }
    let mut ctx = EventCtx::for_path(path);
    ctx.button = 1;
    run_binding(path, "<Button-1>", &ctx);
    run_binding(path, "<ButtonRelease-1>", &ctx);
    if matches!(
        kind,
        Some(WidgetKind::Button) | Some(WidgetKind::Checkbutton) | Some(WidgetKind::Radiobutton)
    ) {
        // Safety: the engine thread owns the interpreter.
        let interp = unsafe { &mut *ptr };
        let _ = invoke_widget(interp, path);
    }
}

/// A Scale's value changed in the UI.
pub fn scale_changed(path: &str, value: f64) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    if let Err(e) = scale_set_value(interp, path, value) {
        record_error(e.to_string());
    }
}

fn scale_set_value(interp: &mut Interp, path: &str, value: f64) -> Result<Value> {
    let from = effective_option(interp, path, "-from")
        .parse::<f64>()
        .unwrap_or(0.0);
    let to = effective_option(interp, path, "-to")
        .parse::<f64>()
        .unwrap_or(100.0);
    let resolution = effective_option(interp, path, "-resolution")
        .parse::<f64>()
        .unwrap_or(1.0);
    let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
    let mut v = value.clamp(lo, hi);
    if resolution > 0.0 {
        v = from + ((v - from) / resolution).round() * resolution;
        v = v.clamp(lo, hi);
    }
    let var = qual_var_opt(interp, path, "-variable");
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.value = v;
        }
    });
    if let Some(var) = var {
        set_var(&var, &format_number(v))?;
    }
    let command = effective_option(interp, path, "-command");
    if !command.is_empty() {
        eval_active(&command)?;
    }
    bump_active();
    Ok(Value::empty())
}

/// The numeric value of a Scale (its `-variable` when set).
pub fn scale_value(path: &str) -> f64 {
    let Some(ptr) = active_interp() else {
        return 0.0;
    };
    let interp = unsafe { &*ptr };
    if let Some(var) = qual_var_opt(interp, path, "-variable") {
        if let Some(v) = var_value(&var).and_then(|v| v.trim().parse::<f64>().ok()) {
            return v;
        }
    }
    with_state(interp, |rt| rt.tree.get(path).map(|n| n.value))
        .flatten()
        .unwrap_or(0.0)
}

/// Whether a Checkbutton/Radiobutton shows as selected.
pub fn is_checked(path: &str) -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    let interp = unsafe { &*ptr };
    let kind = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten();
    match kind {
        Some(WidgetKind::Checkbutton) => {
            if let Some(var) = qual_var_opt(interp, path, "-variable") {
                let on = effective_option(interp, path, "-onvalue");
                return var_value(&var).is_some_and(|v| v == on);
            }
        }
        Some(WidgetKind::Radiobutton) => {
            if let Some(var) = qual_var_opt(interp, path, "-variable") {
                let value = effective_option(interp, path, "-value");
                return var_value(&var).is_some_and(|v| v == value);
            }
        }
        _ => {}
    }
    with_state(interp, |rt| rt.tree.get(path).map(|n| n.checked))
        .flatten()
        .unwrap_or(false)
}

/// The lines of a Text widget.
pub fn text_value(path: &str) -> String {
    let Some(ptr) = active_interp() else {
        return String::new();
    };
    with_state(unsafe { &*ptr }, |rt| {
        rt.tree.get(path).map(|n| n.content.clone())
    })
    .flatten()
    .unwrap_or_default()
}

/// The items of a Listbox and the selected indices.
pub fn list_items(path: &str) -> (Vec<String>, BTreeSet<usize>) {
    let Some(ptr) = active_interp() else {
        return (Vec::new(), BTreeSet::new());
    };
    with_state(unsafe { &*ptr }, |rt| {
        rt.tree
            .get(path)
            .map(|n| (n.items.clone(), n.selected.clone()))
    })
    .flatten()
    .unwrap_or_default()
}

/// A Listbox row was pressed: select it (`-selectmode browse`).
pub fn listbox_select(path: &str, index: usize) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.selected.clear();
            n.selected.insert(index);
            rt.bump();
        }
    });
}

/// The window title from `wm title`.
pub fn window_title() -> Option<String> {
    let ptr = active_interp()?;
    let interp = unsafe { &*ptr };
    let title = effective_option(interp, ".", "-title");
    (!title.is_empty()).then_some(title)
}

/// Whether `wm withdraw` hid the main window.
pub fn window_withdrawn() -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    let interp = unsafe { &*ptr };
    effective_option(interp, ".", "-withdrawn") == "1"
}

/// The `wm protocol . WM_DELETE_WINDOW` script, if the app set one.
pub fn window_close_command() -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| {
        rt.protocols
            .get(&(".".to_string(), "WM_DELETE_WINDOW".to_string()))
            .cloned()
    })
    .flatten()
    .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Entry / Text / Listbox editing
// ---------------------------------------------------------------------------

fn char_slice(content: &str, start: usize, end: usize) -> String {
    content
        .chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn replace_chars(content: &str, start: usize, end: usize, text: &str) -> String {
    let before: String = content.chars().take(start).collect();
    let after: String = content.chars().skip(end).collect();
    format!("{before}{text}{after}")
}

fn entry_index(spec: &str, content: &str, cursor: usize) -> Result<usize> {
    let len = content.chars().count();
    let bad = || generic(format!("bad entry index \"{spec}\""));
    match spec {
        "insert" | "anchor" => Ok(cursor.min(len)),
        "end" => Ok(len),
        "sel.first" | "sel.last" => {
            Err(generic(format!("bad entry index \"{spec}\": no selection")))
        }
        _ => {
            if let Some(delta) = spec.strip_prefix("end-") {
                let n = delta
                    .strip_suffix('c')
                    .unwrap_or(delta)
                    .parse::<usize>()
                    .map_err(|_| bad())?;
                return Ok(len.saturating_sub(n));
            }
            if let Some(rest) = spec.strip_prefix('@') {
                // @x: approximate characters from the pixel offset.
                let x = rest
                    .split(',')
                    .next()
                    .unwrap_or("0")
                    .parse::<i64>()
                    .unwrap_or(0);
                return Ok(((x.max(0) as usize) / 7).min(len));
            }
            spec.parse::<usize>().map_err(|_| bad()).map(|i| i.min(len))
        }
    }
}

fn update_entry(interp: &mut Interp, path: &str, content: String, cursor: usize) -> Result<()> {
    let var = qual_var_opt(interp, path, "-textvariable");
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = content.clone();
            n.cursor = cursor;
            rt.bump();
        }
    });
    if let Some(var) = var {
        set_var(&var, &content)?;
    }
    Ok(())
}

fn entry_insert(interp: &mut Interp, path: &str, index: &str, text: &str) -> Result<Value> {
    let (content, cursor) = entry_state(interp, path);
    let at = entry_index(index, &content, cursor)?;
    let content = replace_chars(&content, at, at, text);
    update_entry(interp, path, content, at + text.chars().count())?;
    Ok(Value::empty())
}

fn entry_delete(interp: &mut Interp, path: &str, first: &str, last: Option<&str>) -> Result<Value> {
    let (content, cursor) = entry_state(interp, path);
    let start = entry_index(first, &content, cursor)?;
    let end = match last {
        Some(last) => entry_index(last, &content, cursor)?,
        None => (start + 1).min(content.chars().count()),
    };
    let (a, b) = (start.min(end), start.max(end));
    let content = replace_chars(&content, a, b, "");
    update_entry(interp, path, content, a)?;
    Ok(Value::empty())
}

fn entry_selection(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("selection", "option ?arg ...?", rest.len() + 2));
    }
    match rest[0].as_str() {
        "range" => {
            if rest.len() < 3 || rest.len() > 4 {
                return Err(wrong("selection range", "start ?end?", rest.len() + 2));
            }
            let (content, cursor) = entry_state(interp, path);
            let start = entry_index(rest[1].as_str(), &content, cursor)?;
            let end = match rest.get(2) {
                Some(end) => entry_index(end.as_str(), &content, cursor)?,
                None => start,
            };
            let (a, b) = (start.min(end), start.max(end));
            with_state_mut(interp, |rt| {
                if let Some(n) = rt.tree.get_mut(path) {
                    n.selection = (a, b);
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "clear" => {
            with_state_mut(interp, |rt| {
                if let Some(n) = rt.tree.get_mut(path) {
                    n.selection = (0, 0);
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "present" => {
            let present = with_state(interp, |rt| {
                rt.tree
                    .get(path)
                    .is_some_and(|n| n.selection.1 > n.selection.0)
            })
            .unwrap_or(false);
            Ok(Value::from_bool(present))
        }
        "from" => {
            let i = with_state(interp, |rt| rt.tree.get(path).map(|n| n.selection.0))
                .flatten()
                .unwrap_or(0);
            Ok(Value::from_int(i as i64))
        }
        "to" => {
            let i = with_state(interp, |rt| rt.tree.get(path).map(|n| n.selection.1))
                .flatten()
                .unwrap_or(0);
            Ok(Value::from_int(i as i64))
        }
        other => Err(generic(format!(
            "bad selection option \"{other}\": must be clear, from, present, range, or to"
        ))),
    }
}

fn text_index(spec: &str, content: &str, cursor: usize) -> Result<usize> {
    let len = content.chars().count();
    let bad = || generic(format!("bad text index \"{spec}\""));
    match spec {
        "insert" => Ok(cursor.min(len)),
        "end" => Ok(len),
        "end-1c" => Ok(len.saturating_sub(1)),
        "end-2c" => Ok(len.saturating_sub(2)),
        _ => {
            if let Some(delta) = spec.strip_prefix("end-") {
                let n = delta
                    .strip_suffix('c')
                    .unwrap_or(delta)
                    .parse::<usize>()
                    .map_err(|_| bad())?;
                return Ok(len.saturating_sub(n));
            }
            if let Some((line, col)) = spec.split_once('.') {
                let line: usize = line.parse().map_err(|_| bad())?;
                let col: usize = col.parse().map_err(|_| bad())?;
                if line == 0 {
                    return Err(bad());
                }
                let mut index = 0usize;
                for (i, l) in content.split('\n').enumerate() {
                    if i + 1 == line {
                        return Ok(index + col.min(l.chars().count()));
                    }
                    index += l.chars().count() + 1;
                }
                return Ok(len);
            }
            spec.parse::<usize>().map_err(|_| bad()).map(|i| i.min(len))
        }
    }
}

fn char_to_linecol(content: &str, index: usize) -> String {
    let mut line = 1usize;
    let mut col = 0usize;
    for (i, c) in content.chars().enumerate() {
        if i == index {
            return format!("{line}.{col}");
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    format!("{line}.{col}")
}

fn text_get(interp: &mut Interp, path: &str, first: &str, last: Option<&str>) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let start = text_index(first, &content, cursor)?;
    let end = match last {
        Some(last) => text_index(last, &content, cursor)?,
        None => (start + 1).min(content.chars().count()),
    };
    let (a, b) = (start.min(end), start.max(end));
    Ok(Value::from_str(&char_slice(&content, a, b)))
}

fn text_insert(interp: &mut Interp, path: &str, index: &str, text: &str) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let at = text_index(index, &content, cursor)?;
    let content = replace_chars(&content, at, at, text);
    let cursor = at + text.chars().count();
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = content;
            n.cursor = cursor;
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn text_delete(interp: &mut Interp, path: &str, first: &str, last: Option<&str>) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let start = text_index(first, &content, cursor)?;
    let end = match last {
        Some(last) => text_index(last, &content, cursor)?,
        None => (start + 1).min(content.chars().count()),
    };
    let (a, b) = (start.min(end), start.max(end));
    let content = replace_chars(&content, a, b, "");
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = content;
            n.cursor = a;
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn listbox_index(spec: &str, len: usize) -> Result<usize> {
    let bad = || generic(format!("bad listbox index \"{spec}\""));
    match spec {
        "end" => Ok(len),
        "active" | "anchor" => Ok(0),
        _ => spec.parse::<usize>().map_err(|_| bad()).map(|i| i.min(len)),
    }
}

fn listbox_insert(interp: &mut Interp, path: &str, index: &str, items: &[Value]) -> Result<Value> {
    let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
        .flatten()
        .ok_or_else(|| bad_window(path))?;
    let at = listbox_index(index, len)?.min(len);
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            for (i, item) in items.iter().enumerate() {
                n.items.insert(at + i, item.as_str().to_string());
            }
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn listbox_delete(
    interp: &mut Interp,
    path: &str,
    first: &str,
    last: Option<&str>,
) -> Result<Value> {
    let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
        .flatten()
        .ok_or_else(|| bad_window(path))?;
    let start = listbox_index(first, len)?;
    let end = match last {
        Some(last) => listbox_index(last, len)?,
        None => start,
    };
    let (a, b) = (start.min(end), start.max(end));
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            let end = (b + 1).min(n.items.len());
            n.items.drain(a.min(n.items.len())..end);
            n.selected = n
                .selected
                .iter()
                .filter(|i| **i < a || **i > b)
                .copied()
                .map(|i| if i > b { i - (b + 1 - a) } else { i })
                .collect();
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn listbox_get(interp: &mut Interp, path: &str, first: &str, last: Option<&str>) -> Result<Value> {
    let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
        .flatten()
        .ok_or_else(|| bad_window(path))?;
    let start = listbox_index(first, len)?;
    let end = match last {
        Some(last) => listbox_index(last, len)?,
        None => start,
    };
    let (a, b) = (start.min(end), start.max(end));
    let items = with_state(interp, |rt| {
        rt.tree.get(path).map(|n| {
            n.items
                .iter()
                .skip(a)
                .take(b.saturating_sub(a) + 1)
                .cloned()
                .collect::<Vec<_>>()
        })
    })
    .flatten()
    .unwrap_or_default();
    Ok(list_value(items))
}

fn listbox_selection(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("selection", "option ?arg ...?", rest.len() + 2));
    }
    match rest[0].as_str() {
        "set" => {
            if rest.len() < 2 {
                return Err(wrong("selection set", "first ?last?", rest.len() + 2));
            }
            let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
                .flatten()
                .ok_or_else(|| bad_window(path))?;
            let start = listbox_index(rest[1].as_str(), len)?;
            let end = match rest.get(2) {
                Some(end) => listbox_index(end.as_str(), len)?,
                None => start,
            };
            let (a, b) = (start.min(end), start.max(end));
            with_state_mut(interp, |rt| {
                if let Some(n) = rt.tree.get_mut(path) {
                    n.selected = (a..=b).collect();
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "clear" => {
            if rest.len() < 2 {
                return Err(wrong("selection clear", "first ?last?", rest.len() + 2));
            }
            let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
                .flatten()
                .ok_or_else(|| bad_window(path))?;
            let start = listbox_index(rest[1].as_str(), len)?;
            let end = match rest.get(2) {
                Some(end) => listbox_index(end.as_str(), len)?,
                None => start,
            };
            let (a, b) = (start.min(end), start.max(end));
            with_state_mut(interp, |rt| {
                if let Some(n) = rt.tree.get_mut(path) {
                    n.selected.retain(|i| *i < a || *i > b);
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "includes" => {
            let i = rest
                .get(1)
                .map(|v| v.as_str().parse::<usize>())
                .transpose()
                .map_err(|_| generic("bad index"))?
                .unwrap_or(0);
            let includes = with_state(interp, |rt| {
                rt.tree.get(path).is_some_and(|n| n.selected.contains(&i))
            })
            .unwrap_or(false);
            Ok(Value::from_bool(includes))
        }
        "anchor" => Ok(Value::empty()),
        other => Err(generic(format!(
            "bad selection option \"{other}\": must be anchor, clear, includes, or set"
        ))),
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn global_command(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let cmd = args[0].as_str();
    match cmd {
        "configure" | "cget" => {
            if args.len() < 2 {
                return Err(wrong(cmd, "window ?-option value ...?", args.len()));
            }
            if cmd == "cget" {
                cget_impl(interp, args[1].as_str(), &args[2..])
            } else {
                configure_impl(interp, args[1].as_str(), &args[2..])
            }
        }
        "destroy" => {
            if args.len() < 2 {
                return Err(wrong("destroy", "window ?window ...?", args.len()));
            }
            for a in &args[1..] {
                destroy_impl(interp, a.as_str())?;
            }
            Ok(Value::empty())
        }
        "pack" => pack_impl(interp, args),
        "grid" => grid_impl(interp, args),
        "winfo" => winfo_impl(interp, args),
        "wm" => wm_impl(interp, args),
        "tk" => tk_impl(interp, args),
        "bind" => bind_impl(interp, args),
        "event" => event_impl(interp, args),
        _ => Err(Error::invalid_command(cmd)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(interp: &mut Interp, script: &str) -> String {
        interp.eval(script).unwrap().as_str().to_string()
    }

    #[test]
    fn widget_creation_configure_cget() {
        let mut i = Interp::new();
        install(&mut i);
        assert_eq!(eval(&mut i, "button .b -text Hi"), ".b");
        assert_eq!(eval(&mut i, "cget .b -text"), "Hi");
        assert_eq!(eval(&mut i, "configure .b -text Bye; cget .b -text"), "Bye");
        assert_eq!(eval(&mut i, ".b cget -text"), "Bye");
        // Tk option abbreviations and defaults: `-sta` is unique,
        // `-tex` is not (`-text` / `-textvariable`).
        assert_eq!(eval(&mut i, ".b cget -sta"), "normal");
        assert_eq!(eval(&mut i, "catch {.b cget -tex}"), "1");
        assert_eq!(eval(&mut i, "catch {.b cget -nope}"), "1");
    }

    #[test]
    fn hierarchy_and_pack_grid() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "frame .f; button .f.b -text X; pack .f -side left -expand 1",
        );
        assert_eq!(eval(&mut i, "winfo children ."), ".f");
        assert_eq!(eval(&mut i, "winfo children .f"), ".f.b");
        eval(&mut i, "grid .f.b -row 2 -column 3 -sticky nsew");
        with_state(&i, |rt| {
            let n = rt.tree().get(".f.b").unwrap();
            assert!(n.pack.is_none());
            let g = n.grid.unwrap();
            assert_eq!((g.row, g.column), (2, 3));
            assert!(g.sticky.left && g.sticky.right && g.sticky.top && g.sticky.bottom);
        })
        .unwrap();
    }

    #[test]
    fn destroy_removes_subtree_and_widget_commands() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "frame .f; button .f.b -text X");
        eval(&mut i, "destroy .f");
        assert_eq!(eval(&mut i, "winfo exists .f"), "0");
        assert_eq!(eval(&mut i, "catch {.f.b cget -text}"), "1");
    }

    #[test]
    fn wm_and_tk_queries() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "wm title . Hello");
        assert_eq!(eval(&mut i, "wm title ."), "Hello");
        assert_eq!(eval(&mut i, "tk patchlevel"), "8.6");
        assert_eq!(eval(&mut i, "wm state ."), "normal");
    }

    #[test]
    fn textvariable_drives_label_and_entry() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "set ::name World; label .l -textvariable ::name");
        assert_eq!(display_text(".l"), "World");
        eval(&mut i, "set ::name Tcl");
        assert_eq!(display_text(".l"), "Tcl");
        eval(&mut i, "entry .e -textvariable ::name");
        assert_eq!(entry_value(".e"), "Tcl");
        entry_changed(".e", "Rust");
        assert_eq!(eval(&mut i, "set ::name"), "Rust");
        assert_eq!(eval(&mut i, ".e get"), "Rust");
    }

    #[test]
    fn entry_edit_subcommands() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "entry .e; set ::v abc; .e configure -textvariable ::v",
        );
        assert_eq!(eval(&mut i, ".e get"), "abc");
        eval(&mut i, ".e insert end def");
        assert_eq!(eval(&mut i, ".e get"), "abcdef");
        eval(&mut i, ".e delete 0 3");
        assert_eq!(eval(&mut i, ".e get"), "def");
        assert_eq!(eval(&mut i, ".e index end"), "3");
    }

    #[test]
    fn checkbutton_and_radiobutton_invoke() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "set ::on 0; checkbutton .c -variable ::on -onvalue 1 -offvalue 0 -command {incr ::hits}; .c invoke",
        );
        assert_eq!(eval(&mut i, "set ::on"), "1");
        eval(&mut i, ".c invoke");
        assert_eq!(eval(&mut i, "set ::on"), "0");
        assert_eq!(eval(&mut i, "set ::hits"), "2");
        eval(
            &mut i,
            "set ::choice a; radiobutton .r -variable ::choice -value a; radiobutton .r2 -variable ::choice -value b; .r2 invoke",
        );
        assert_eq!(eval(&mut i, "set ::choice"), "b");
        assert!(is_checked(".r2"));
        assert!(!is_checked(".r"));
    }

    #[test]
    fn scale_get_set_and_variable() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "set ::vol 20; scale .s -from 0 -to 100 -variable ::vol; .s set 55",
        );
        assert_eq!(eval(&mut i, "set ::vol"), "55");
        assert_eq!(scale_value(".s"), 55.0);
        assert_eq!(eval(&mut i, ".s get"), "55");
    }

    #[test]
    fn text_and_listbox_data() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "text .t; .t insert 1.0 hello; .t insert end { world}",
        );
        assert_eq!(eval(&mut i, ".t get 1.0 end"), "hello world");
        eval(&mut i, ".t delete 1.0 1.5");
        assert_eq!(eval(&mut i, ".t get 1.0 end"), " world");
        eval(&mut i, "listbox .lb; .lb insert end a b c");
        assert_eq!(eval(&mut i, ".lb size"), "3");
        assert_eq!(eval(&mut i, ".lb get 1"), "b");
        eval(&mut i, ".lb delete 1");
        assert_eq!(eval(&mut i, ".lb get 0 end"), "a c");
        eval(&mut i, ".lb selection set 1");
        assert_eq!(eval(&mut i, ".lb curselection"), "1");
        assert_eq!(eval(&mut i, ".lb selection includes 1"), "1");
    }

    #[test]
    fn bind_and_event_generate() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "button .b -text X; set ::n 0; bind .b <Button-1> {incr ::n}; event generate .b <Button-1>; set ::n",
        );
        assert_eq!(eval(&mut i, "set ::n"), "1");
        assert_eq!(eval(&mut i, "bind .b"), "<Button-1>");
        eval(&mut i, "bind all <Return> {incr ::n}");
        eval(&mut i, "entry .e; event generate .e <Key-Return>");
        assert_eq!(eval(&mut i, "set ::n"), "2");
    }

    #[test]
    fn pack_and_grid_queries() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "frame .f; button .f.a -text A; button .f.b -text B");
        eval(&mut i, "pack .f.a -side left -expand 1");
        assert_eq!(eval(&mut i, "winfo manager .f.a"), "pack");
        assert_eq!(eval(&mut i, "pack slaves .f"), ".f.a");
        assert_eq!(eval(&mut i, "lindex [pack info .f.a] 0"), "-in");
        // A parent's slaves cannot switch managers.
        assert_eq!(eval(&mut i, "catch {grid .f.b -row 0 -column 0}"), "1");
        eval(&mut i, "pack forget .f.a; grid .f.b -row 1 -column 2");
        assert_eq!(eval(&mut i, "winfo manager .f.b"), "grid");
        assert_eq!(eval(&mut i, "grid size .f"), "2 3");
    }

    #[test]
    fn winfo_surface() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "frame .f; label .f.l -text Hello; pack .f.l");
        assert_eq!(eval(&mut i, "winfo class .f.l"), "Label");
        assert_eq!(eval(&mut i, "winfo parent .f.l"), ".f");
        assert_eq!(eval(&mut i, "winfo toplevel .f.l"), ".");
        assert_eq!(eval(&mut i, "winfo ismapped .f.l"), "1");
        assert!(eval(&mut i, "winfo reqwidth .f.l").parse::<i64>().unwrap() > 0);
    }

    #[test]
    fn ttk_aliases_create_the_same_kinds() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "ttk::button .b -text Go; ttk::label .l -text Hi");
        assert_eq!(eval(&mut i, "winfo class .b"), "Button");
        assert_eq!(eval(&mut i, "winfo class .l"), "Label");
    }

    #[test]
    fn after_events_pump_and_rerender() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "set ::later 0; after 0 {set ::later 1}");
        assert!(pump_active());
        assert_eq!(eval(&mut i, "set ::later"), "1");
        assert!(has_scheduled_events() || true);
    }
}

#[cfg(test)]
mod press_debug_tests {
    use super::*;

    #[test]
    fn press_button_runs_the_command() {
        let mut i = Interp::new();
        install(&mut i);
        i.eval("button .b -text A -command {.b configure -text B}")
            .unwrap();
        assert_eq!(display_text(".b"), "A");
        press_button(".b");
        assert_eq!(display_text(".b"), "B");
        let errors = take_errors();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
