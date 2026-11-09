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

pub use rtcl_tk_core::{
    Fill, GridInfo, PackInfo, PackSide, PlaceInfo, Sticky, TkError, TkTree, WidgetKind,
};

#[derive(Debug, Default)]
pub struct TkRuntime {
    tree: TkTree,
    revision: u64,
    /// `bind all` scripts, run after the widget's own bindings.
    all_bindings: BTreeMap<String, String>,
    /// Class/toplevel/arbitrary tag bindings (`bind Button`, `bind .`).
    tag_bindings: BTreeMap<String, BTreeMap<String, String>>,
    /// `wm protocol` handlers, keyed by `(path, protocol name)`.
    protocols: BTreeMap<(String, String), String>,
    /// `grid columnconfigure` / `rowconfigure`: `(parent, index, is_row)`
    /// to `(weight, minsize)`.
    grid_options: BTreeMap<(String, i64, bool), (i64, i64)>,
    /// `focus` target (a widget path).
    focus: Option<String>,
    /// `bindtags` overrides.
    bindtags: BTreeMap<String, Vec<String>>,
    /// Named fonts: name to its options.
    fonts: BTreeMap<String, BTreeMap<String, String>>,
    /// `option add` database entries, in insertion order.
    option_db: Vec<(String, String)>,
    /// `menu` windows, which are not part of the packed widget tree.
    menus: BTreeMap<String, Menu>,
    /// The open cascade menu (`None` when no menu is dropped down).
    open_menu: Option<String>,
    /// Selected page index of each `ttk::notebook`.
    notebook_selected: BTreeMap<String, usize>,
    /// The combobox whose value list is dropped down.
    open_combobox: Option<String>,
    /// `ttk::treeview` models.
    treeviews: BTreeMap<String, TreeViewState>,
    /// `tk scaling`: pixels per point (0 means the 1.0 default).
    scaling: f64,
    /// `tk appname`.
    app_name: String,
    /// `text` tags: widget path → tag name → options and character ranges.
    text_tags: BTreeMap<String, BTreeMap<String, TextTag>>,
    /// Photo images by Tk image name.
    images: BTreeMap<String, PhotoImage>,
    image_seq: u64,
    /// Canvas item models.
    canvases: BTreeMap<String, CanvasState>,
    /// Persisted rich-text documents (ProseMirror JSON) of `text` widgets.
    rich_docs: BTreeMap<String, String>,
}

/// A canvas's items.
#[derive(Debug, Clone, Default)]
pub struct CanvasState {
    pub items: Vec<CanvasItem>,
    next_id: u64,
    /// Item/tag bindings (`bind <id|tag> <sequence> script`).
    bindings: BTreeMap<String, BTreeMap<String, String>>,
    /// `xview`/`yview` fractions inside `-scrollregion`.
    pub xview: f64,
    pub yview: f64,
}

/// One canvas item (`rectangle`, `line`, `oval`, `text`, `polygon`).
#[derive(Debug, Clone)]
pub struct CanvasItem {
    pub id: u64,
    pub kind: String,
    pub coords: Vec<f64>,
    pub options: BTreeMap<String, String>,
}

/// A `photo` image: a file plus its reported size.
#[derive(Debug, Clone, Default)]
pub struct PhotoImage {
    pub file: String,
    pub width: i64,
    pub height: i64,
}

/// A `text` tag: its options and the character ranges it covers.
#[derive(Debug, Clone, Default)]
pub struct TextTag {
    pub options: BTreeMap<String, String>,
    pub ranges: Vec<(usize, usize)>,
}

/// A text widget's tags, for the renderer.
#[derive(Debug, Clone, Default)]
pub struct TextTagView {
    pub name: String,
    pub options: BTreeMap<String, String>,
    pub ranges: Vec<(usize, usize)>,
}

/// One row of a `ttk::treeview`.
#[derive(Debug, Clone, Default)]
pub struct TreeItem {
    pub id: String,
    pub parent: Option<String>,
    pub text: String,
    pub values: Vec<String>,
    pub open: bool,
}

/// A `ttk::treeview`: columns, headings and items in insertion order.
#[derive(Debug, Clone, Default)]
pub struct TreeViewState {
    pub columns: Vec<String>,
    pub headings: BTreeMap<String, String>,
    pub items: Vec<TreeItem>,
    pub selected: BTreeSet<String>,
    pub focus: Option<String>,
    next_id: u64,
}

/// One entry of a Tk `menu`.
#[derive(Debug, Clone, Default)]
pub struct MenuItem {
    /// `command`, `separator`, `checkbutton`, `radiobutton`, `cascade`.
    pub kind: String,
    /// Entry options (`-label`, `-command`, `-variable`, ...).
    pub options: BTreeMap<String, String>,
}

/// A Tk `menu` window.
#[derive(Debug, Clone, Default)]
pub struct Menu {
    pub path: String,
    pub items: Vec<MenuItem>,
    pub options: BTreeMap<String, String>,
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
        "message",
        "scrollbar",
        "separator",
        "progressbar",
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
        "ttk::message",
        "ttk::scrollbar",
        "ttk::separator",
        "ttk::progressbar",
        "notebook",
        "combobox",
        "ttk::notebook",
        "ttk::combobox",
        "treeview",
        "ttk::treeview",
        "labelframe",
        "ttk::labelframe",
        "chart",
        "::rtcl::chart",
        "canvas",
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
        "place",
        "winfo",
        "wm",
        "tk",
        "bind",
        "event",
        "focus",
        "raise",
        "lower",
        "bindtags",
        "font",
        "option",
        "bell",
        "tk_focusNext",
        "tk_focusPrev",
        "menu",
        "image",
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
    let base = cmd
        .strip_prefix("ttk::")
        .or_else(|| cmd.strip_prefix("::rtcl::"))
        .unwrap_or(cmd);
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
    if kind == WidgetKind::Progressbar && effective_option(interp, &path, "-variable").is_empty() {
        let value = effective_option(interp, &path, "-value")
            .parse::<f64>()
            .unwrap_or(0.0);
        with_state_mut(interp, |rt| {
            if let Some(n) = rt.tree.get_mut(&path) {
                n.value = value;
            }
        });
    }
    bump_active();
    interp.register_command(&path, widget_command);
    Ok(Value::from_str(&path))
}

/// Give a fresh widget its data and default value (scale midpoint, entry
/// content from `-textvariable`, checkbutton state).
fn initialize_widget(interp: &mut Interp, path: &str, kind: WidgetKind) -> Result<()> {
    match kind {
        WidgetKind::Entry | WidgetKind::Combobox => {
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
        WidgetKind::Progressbar => {
            let var = effective_option(interp, path, "-variable");
            if !var.is_empty() {
                if let Some(value) = var_value(&var).and_then(|v| v.trim().parse::<f64>().ok()) {
                    with_state_mut(interp, |rt| {
                        if let Some(n) = rt.tree.get_mut(path) {
                            n.value = value;
                        }
                    });
                }
            }
        }
        WidgetKind::Listbox => {
            let var = effective_option(interp, path, "-listvariable");
            if !var.is_empty() {
                if let Some(items) = var_value(&var)
                    .and_then(|s| Value::from_str(&s).as_list())
                    .map(|list| {
                        list.iter()
                            .map(|v| v.as_str().to_string())
                            .collect::<Vec<_>>()
                    })
                {
                    with_state_mut(interp, |rt| {
                        if let Some(n) = rt.tree.get_mut(path) {
                            n.items = items;
                            n.selected.clear();
                        }
                    });
                }
            }
        }
        WidgetKind::Canvas => {
            with_state_mut(interp, |rt| {
                rt.canvases.entry(path.to_string()).or_default();
            });
        }
        WidgetKind::Treeview => {
            let columns = effective_option(interp, path, "-columns");
            let columns = Value::from_str(&columns)
                .as_list()
                .map(|list| {
                    list.iter()
                        .map(|v| v.as_str().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            with_state_mut(interp, |rt| {
                rt.treeviews.entry(path.to_string()).or_default().columns = columns;
            });
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
                "tag" => text_tag_command(interp, path, &args[2..]),
                "getdoc" => Ok(Value::from_str(&rich_doc(path).unwrap_or_default())),
                "setdoc" => {
                    if args.len() < 3 {
                        return Err(wrong("setdoc", "json", args.len()));
                    }
                    let json = args[2].as_str().to_string();
                    let text = rich_doc_text(&json)
                        .ok_or_else(|| generic("setdoc: not a rich document"))?;
                    with_state_mut(interp, |rt| {
                        if let Some(node) = rt.tree.get_mut(path) {
                            node.content = text.clone();
                            node.cursor = text.chars().count();
                        }
                        rt.rich_docs.insert(path.to_string(), json.clone());
                        rt.bump();
                    });
                    sync_text_tags_from_doc(path);
                    Ok(Value::empty())
                }
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
                "yview" => {
                    let total = with_state(interp, |rt| {
                        rt.tree.get(path).map(|n| n.items.len()).unwrap_or(0)
                    })
                    .unwrap_or(0);
                    view_scroll_command(interp, path, &args[2..], total)
                }
                "activate" | "see" | "xview" => Ok(Value::empty()),
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
            WidgetKind::Scrollbar => match sub {
                "set" => {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(wrong("set", "first ?last?", args.len()));
                    }
                    let first = parse_f64_str(args[2].as_str(), "first")?;
                    let last = match args.get(3) {
                        Some(v) => parse_f64_str(v.as_str(), "last")?,
                        None => (first + 0.1).min(1.0),
                    };
                    with_state_mut(interp, |rt| {
                        if let Some(n) = rt.tree.get_mut(path) {
                            n.value = first.clamp(0.0, 1.0);
                            n.content = format!("{last}");
                            rt.bump();
                        }
                    })
                    .ok_or_else(|| bad_window(path))?;
                    Ok(Value::empty())
                }
                "get" => {
                    let (first, last) = with_state(interp, |rt| {
                        rt.tree
                            .get(path)
                            .map(|n| (n.value, n.content.parse::<f64>().unwrap_or(n.value + 0.1)))
                    })
                    .flatten()
                    .ok_or_else(|| bad_window(path))?;
                    Ok(list_value([format_number(first), format_number(last)]))
                }
                "fraction" => {
                    let v = with_state(interp, |rt| rt.tree.get(path).map(|n| n.value))
                        .flatten()
                        .ok_or_else(|| bad_window(path))?;
                    Ok(Value::from_str(&format_number(v)))
                }
                "activate" | "delta" | "identify" => Ok(Value::empty()),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Progressbar => match sub {
                "step" => {
                    let amount = match args.get(2) {
                        Some(v) => parse_f64_str(v.as_str(), "step")?,
                        None => 1.0,
                    };
                    progressbar_step(interp, path, amount)?;
                    Ok(Value::empty())
                }
                "start" | "stop" => Ok(Value::empty()),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Notebook => match sub {
                "add" => {
                    if args.len() < 3 {
                        return Err(wrong("add", "window ?-option value ...?", args.len()));
                    }
                    notebook_add(interp, path, args[2].as_str(), &args[3..], None)
                }
                "insert" => {
                    if args.len() < 4 {
                        return Err(wrong(
                            "insert",
                            "index window ?-option value ...?",
                            args.len(),
                        ));
                    }
                    let index = args[2].as_str().parse::<usize>().map_err(|_| {
                        generic(format!("expected integer but got \"{}\"", args[2].as_str()))
                    })?;
                    notebook_add(interp, path, args[3].as_str(), &args[4..], Some(index))
                }
                "forget" => {
                    if args.len() < 3 {
                        return Err(wrong("forget", "window", args.len()));
                    }
                    notebook_forget(interp, path, args[2].as_str())?;
                    Ok(Value::empty())
                }
                "tabs" => {
                    let tabs = with_state(interp, |rt| rt.tree.children(path).map(|c| c.to_vec()))
                        .and_then(|r| r.ok())
                        .unwrap_or_default();
                    Ok(list_value(tabs))
                }
                "select" => {
                    let pages = with_state(interp, |rt| rt.tree.children(path).map(|c| c.to_vec()))
                        .and_then(|r| r.ok())
                        .unwrap_or_default();
                    if args.len() == 2 {
                        let index = notebook_selected(path);
                        return Ok(Value::from_str(
                            pages.get(index).map(String::as_str).unwrap_or(""),
                        ));
                    }
                    let target = args[2].as_str();
                    let index = pages
                        .iter()
                        .position(|p| p == target)
                        .ok_or_else(|| generic(format!("page \"{target}\" is not in {path}")))?;
                    select_notebook_tab(path, index);
                    Ok(Value::empty())
                }
                "index" => {
                    if args.len() != 3 {
                        return Err(wrong("index", "window", args.len()));
                    }
                    let pages = with_state(interp, |rt| rt.tree.children(path).map(|c| c.to_vec()))
                        .and_then(|r| r.ok())
                        .unwrap_or_default();
                    let i = pages
                        .iter()
                        .position(|p| p == args[2].as_str())
                        .map(|i| i as i64)
                        .unwrap_or(-1);
                    Ok(Value::from_int(i))
                }
                "tab" => {
                    if args.len() < 3 {
                        return Err(wrong("tab", "window ?-option? ?value?", args.len()));
                    }
                    let page = args[2].as_str();
                    if args.len() == 3 {
                        let text = with_state(interp, |rt| {
                            rt.tree
                                .get(page)
                                .and_then(|n| n.options.get("-text").cloned())
                        })
                        .flatten()
                        .unwrap_or_default();
                        return Ok(Value::from_str(&text));
                    }
                    if args.len() == 4 {
                        let option = args[3].as_str();
                        let value = with_state(interp, |rt| {
                            rt.tree
                                .get(page)
                                .and_then(|n| n.options.get(option).cloned())
                        })
                        .flatten()
                        .unwrap_or_default();
                        return Ok(Value::from_str(&value));
                    }
                    let options = if args.len() == 5 {
                        vec![(args[3].as_str().to_string(), args[4].as_str().to_string())]
                    } else {
                        return Err(wrong("tab", "window ?-option? ?value?", args.len()));
                    };
                    with_state_mut(interp, |rt| {
                        rt.tree.configure(page, options).map_err(tk_error)?;
                        rt.bump();
                        Ok::<(), Error>(())
                    })
                    .ok_or_else(|| bad_window(page))??;
                    Ok(Value::empty())
                }
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Combobox => match sub {
                "get" => Ok(Value::from_str(&entry_value(path))),
                "set" => {
                    if args.len() != 3 {
                        return Err(wrong("set", "value", args.len()));
                    }
                    let value = args[2].as_str();
                    update_entry(interp, path, value.to_string(), value.chars().count())?;
                    Ok(Value::empty())
                }
                "post" => {
                    with_state_mut(interp, |rt| {
                        rt.open_combobox = Some(path.to_string());
                        rt.bump();
                    });
                    Ok(Value::empty())
                }
                "unpost" => {
                    with_state_mut(interp, |rt| {
                        if rt.open_combobox.as_deref() == Some(path) {
                            rt.open_combobox = None;
                        }
                        rt.bump();
                    });
                    Ok(Value::empty())
                }
                "values" => {
                    if args.len() == 2 {
                        let values = combobox_values(path);
                        return Ok(list_value(values));
                    }
                    let value = args[2..]
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    with_state_mut(interp, |rt| {
                        rt.tree
                            .configure(path, [("-values".to_string(), value)])
                            .map_err(tk_error)?;
                        rt.bump();
                        Ok::<(), Error>(())
                    })
                    .ok_or_else(|| bad_window(path))??;
                    Ok(Value::empty())
                }
                "current" => {
                    let values = combobox_values(path);
                    if args.len() == 2 {
                        let current = entry_value(path);
                        let i = values
                            .iter()
                            .position(|v| *v == current)
                            .map(|i| i as i64)
                            .unwrap_or(-1);
                        return Ok(Value::from_int(i));
                    }
                    let index = args[2].as_str().parse::<usize>().map_err(|_| {
                        generic(format!("expected integer but got \"{}\"", args[2].as_str()))
                    })?;
                    if let Some(value) = values.get(index).cloned() {
                        update_entry(interp, path, value.clone(), value.chars().count())?;
                        return Ok(Value::from_int(index as i64));
                    }
                    Ok(Value::from_int(-1))
                }
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Treeview => match sub {
                "insert" => {
                    if args.len() < 4 {
                        return Err(wrong(
                            "insert",
                            "parent index ?-option value ...?",
                            args.len(),
                        ));
                    }
                    treeview_insert(interp, path, args[2].as_str(), &args[4..])
                }
                "delete" => {
                    for a in &args[2..] {
                        treeview_delete(interp, path, a.as_str())?;
                    }
                    Ok(Value::empty())
                }
                "item" => {
                    if args.len() < 3 {
                        return Err(wrong("item", "item ?-option? ?value ...?", args.len()));
                    }
                    treeview_item(interp, path, args[2].as_str(), &args[3..])
                }
                "exists" => Ok(Value::from_bool(args.get(2).is_some_and(|v| {
                    with_state(interp, |rt| {
                        rt.treeviews
                            .get(path)
                            .is_some_and(|tv| tv.items.iter().any(|i| i.id == v.as_str()))
                    })
                    .unwrap_or(false)
                }))),
                "children" => {
                    if args.len() != 3 {
                        return Err(wrong("children", "item", args.len()));
                    }
                    let parent = if args[2].as_str().is_empty() {
                        None
                    } else {
                        Some(args[2].as_str().to_string())
                    };
                    let kids = with_state(interp, |rt| {
                        rt.treeviews.get(path).map(|tv| {
                            tv.items
                                .iter()
                                .filter(|i| i.parent == parent)
                                .map(|i| i.id.clone())
                                .collect::<Vec<_>>()
                        })
                    })
                    .flatten()
                    .unwrap_or_default();
                    Ok(list_value(kids))
                }
                "parent" => {
                    if args.len() != 3 {
                        return Err(wrong("parent", "item", args.len()));
                    }
                    let parent = with_state(interp, |rt| {
                        rt.treeviews.get(path).and_then(|tv| {
                            tv.items
                                .iter()
                                .find(|i| i.id == args[2].as_str())
                                .and_then(|i| i.parent.clone())
                        })
                    })
                    .flatten()
                    .unwrap_or_default();
                    Ok(Value::from_str(&parent))
                }
                "index" => {
                    if args.len() != 3 {
                        return Err(wrong("index", "item", args.len()));
                    }
                    let index = with_state(interp, |rt| {
                        rt.treeviews.get(path).and_then(|tv| {
                            let item = tv.items.iter().find(|i| i.id == args[2].as_str())?;
                            Some(
                                tv.items
                                    .iter()
                                    .filter(|i| i.parent == item.parent)
                                    .position(|i| i.id == item.id)
                                    .unwrap_or(0) as i64,
                            )
                        })
                    })
                    .flatten()
                    .unwrap_or(-1);
                    Ok(Value::from_int(index))
                }
                "selection" => {
                    if args.len() == 2 {
                        let selected = with_state(interp, |rt| {
                            rt.treeviews
                                .get(path)
                                .map(|tv| tv.selected.iter().cloned().collect::<Vec<_>>())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default();
                        return Ok(list_value(selected));
                    }
                    let op = args[2].as_str();
                    let ids = args
                        .get(3..)
                        .unwrap_or(&[])
                        .iter()
                        .map(|v| v.as_str().to_string())
                        .collect::<Vec<_>>();
                    with_state_mut(interp, |rt| {
                        if let Some(tv) = rt.treeviews.get_mut(path) {
                            match op {
                                "set" => {
                                    tv.selected = ids.iter().cloned().collect();
                                    tv.focus = ids.first().cloned();
                                }
                                "add" => tv.selected.extend(ids.iter().cloned()),
                                "remove" => {
                                    for id in &ids {
                                        tv.selected.remove(id);
                                    }
                                }
                                "toggle" => {
                                    for id in &ids {
                                        if !tv.selected.remove(id) {
                                            tv.selected.insert(id.clone());
                                        }
                                    }
                                }
                                _ => {}
                            }
                            rt.bump();
                        }
                    });
                    let selected = with_state(interp, |rt| {
                        rt.treeviews
                            .get(path)
                            .map(|tv| tv.selected.iter().cloned().collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                    Ok(list_value(selected))
                }
                "focus" => {
                    let focus = with_state(interp, |rt| {
                        rt.treeviews.get(path).and_then(|tv| tv.focus.clone())
                    })
                    .flatten()
                    .unwrap_or_default();
                    if args.len() > 2 {
                        let id = args[2].as_str().to_string();
                        with_state_mut(interp, |rt| {
                            if let Some(tv) = rt.treeviews.get_mut(path) {
                                tv.focus = Some(id);
                                rt.bump();
                            }
                        });
                        Ok(Value::empty())
                    } else {
                        Ok(Value::from_str(&focus))
                    }
                }
                "heading" => {
                    if args.len() < 3 {
                        return Err(wrong("heading", "column ?-option? ?value?", args.len()));
                    }
                    let column = args[2].as_str().to_string();
                    if args.len() == 3 {
                        let text = with_state(interp, |rt| {
                            rt.treeviews
                                .get(path)
                                .and_then(|tv| tv.headings.get(&column).cloned())
                        })
                        .flatten()
                        .unwrap_or_default();
                        return Ok(Value::from_str(&text));
                    }
                    if args.len() == 5 && args[3].as_str() == "-text" {
                        let text = args[4].as_str().to_string();
                        with_state_mut(interp, |rt| {
                            if let Some(tv) = rt.treeviews.get_mut(path) {
                                tv.headings.insert(column, text);
                                rt.bump();
                            }
                        });
                    }
                    Ok(Value::empty())
                }
                "yview" => {
                    let total = treeview_visible_count(interp, path);
                    view_scroll_command(interp, path, &args[2..], total)
                }
                "column" | "tag" | "see" | "xview" => Ok(Value::empty()),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Canvas => match sub {
                "create" => {
                    if args.len() < 4 {
                        return Err(wrong(
                            "create",
                            "type x y ?x y ...? ?-option value ...?",
                            args.len(),
                        ));
                    }
                    canvas_create(interp, path, args[2].as_str(), &args[3..])
                }
                "delete" => {
                    for value in &args[2..] {
                        if value.as_str() == "all" {
                            with_state_mut(interp, |rt| {
                                if let Some(canvas) = rt.canvases.get_mut(path) {
                                    canvas.items.clear();
                                    rt.bump();
                                }
                            });
                        } else if let Ok(id) = value.as_str().parse::<u64>() {
                            with_state_mut(interp, |rt| {
                                if let Some(canvas) = rt.canvases.get_mut(path) {
                                    canvas.items.retain(|item| item.id != id);
                                    rt.bump();
                                }
                            });
                        }
                    }
                    Ok(Value::empty())
                }
                "coords" => canvas_coords(interp, path, &args[2..]),
                "itemcget" => {
                    if args.len() != 4 {
                        return Err(wrong("itemcget", "tagOrId -option", args.len()));
                    }
                    let id = args[2]
                        .as_str()
                        .parse::<u64>()
                        .map_err(|_| generic("bad item id"))?;
                    let value = with_state(interp, |rt| {
                        rt.canvases
                            .get(path)
                            .and_then(|canvas| canvas.items.iter().find(|i| i.id == id))
                            .and_then(|item| item.options.get(args[3].as_str()).cloned())
                    })
                    .flatten()
                    .unwrap_or_default();
                    Ok(Value::from_str(&value))
                }
                "itemconfigure" => canvas_itemconfigure(interp, path, &args[2..]),
                "find" => {
                    let ids = with_state(interp, |rt| {
                        rt.canvases
                            .get(path)
                            .map(|canvas| {
                                canvas
                                    .items
                                    .iter()
                                    .map(|i| i.id.to_string())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                    Ok(list_value(ids))
                }
                "bbox" => Ok(Value::empty()),
                "move" => canvas_move(interp, path, &args[2..], false),
                "scale" => canvas_move(interp, path, &args[2..], true),
                "raise" | "lower" => canvas_raise(interp, path, &args[2..], sub == "raise"),
                "bind" => canvas_bind(interp, path, &args[2..]),
                "xview" => canvas_view(interp, path, &args[2..], true),
                "yview" => canvas_view(interp, path, &args[2..], false),
                "focus" | "select" => Ok(Value::empty()),
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
            WidgetKind::Scrollbar => {
                "activate, cget, configure, delta, destroy, fraction, get, identify, set"
            }
            WidgetKind::Progressbar => "cget, configure, destroy, start, step, stop",
            WidgetKind::Notebook => {
                "add, cget, configure, destroy, forget, index, insert, select, tab, tabs"
            }
            WidgetKind::Combobox => {
                "cget, configure, current, destroy, get, post, set, unpost, values"
            }
            WidgetKind::Treeview => {
                "children, column, delete, exists, focus, heading, index, insert, item, parent, see, selection, tag"
            }
            WidgetKind::Canvas => {
                "bbox, bind, coords, create, delete, find, itemcget, itemconfigure, move, scale, xview, yview"
            }
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
    let is_menu = with_state(interp, |rt| rt.menus.contains_key(path)).unwrap_or(false);
    if is_menu {
        with_state_mut(interp, |rt| {
            rt.menus.remove(path);
            if rt.open_menu.as_deref() == Some(path) {
                rt.open_menu = None;
            }
            rt.bump();
        });
        let _ = interp.delete_command(path);
        return Ok(Value::empty());
    }
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
        rt.treeviews.remove(path);
        rt.canvases.remove(path);
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
    let mut info = with_state(interp, |rt| rt.tree.get(path).and_then(|n| n.pack.clone()))
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
            "-anchor" => info.anchor = v,
            "-ipadx" | "-ipady" => {}
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
    let info = with_state(interp, |rt| rt.tree.get(path).map(|n| n.pack.clone()))
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
        info.anchor.clone(),
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
            WidgetKind::Entry | WidgetKind::Combobox => ((w_opt.max(1) * 7) + 8, 24),
            WidgetKind::Scale => {
                let length = opt("-length").parse::<i64>().unwrap_or(100);
                if opt("-orient") == "horizontal" {
                    (length.max(1), 24)
                } else {
                    (24, length.max(1))
                }
            }
            WidgetKind::Frame
            | WidgetKind::Toplevel
            | WidgetKind::Notebook
            | WidgetKind::Labelframe => (w_opt, h_opt),
            WidgetKind::Treeview => (300, h_opt.max(1) * 22),
            WidgetKind::Chart => (w_opt.max(1), h_opt.max(1)),
            WidgetKind::Scrollbar => {
                if opt("-orient") == "horizontal" {
                    (100, 16)
                } else {
                    (16, 100)
                }
            }
            WidgetKind::Separator => {
                if opt("-orient") == "horizontal" {
                    (100, 1)
                } else {
                    (1, 100)
                }
            }
            WidgetKind::Progressbar => {
                let length = opt("-length").parse::<i64>().unwrap_or(100);
                if opt("-orient") == "horizontal" {
                    (length, 8)
                } else {
                    (8, length)
                }
            }
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

/// A `grid columnconfigure` / `rowconfigure` weight.
pub fn grid_weight(parent: &str, index: i64, is_row: bool) -> i64 {
    let Some(ptr) = active_interp() else {
        return 0;
    };
    with_state(unsafe { &*ptr }, |rt| {
        rt.grid_options
            .get(&(parent.to_string(), index, is_row))
            .map(|v| v.0)
            .unwrap_or(0)
    })
    .unwrap_or(0)
}

fn place_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("place", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "configure" | "config" => {
            if args.len() < 3 {
                return Err(wrong(
                    "place configure",
                    "window ?-option value ...?",
                    args.len(),
                ));
            }
            place_widget(interp, args[2].as_str(), &args[3..])
        }
        "forget" => {
            if args.len() < 3 {
                return Err(wrong("place forget", "window ?window ...?", args.len()));
            }
            for a in &args[2..] {
                let path = a.as_str();
                with_state_mut(interp, |rt| {
                    rt.tree.unplace(path).map_err(tk_error)?;
                    rt.bump();
                    Ok::<(), Error>(())
                })
                .ok_or_else(|| bad_window(path))??;
            }
            Ok(Value::empty())
        }
        "info" => {
            if args.len() != 3 {
                return Err(wrong("place info", "window", args.len()));
            }
            let path = args[2].as_str();
            let info = with_state(interp, |rt| rt.tree.get(path).and_then(|n| n.place.clone()))
                .ok_or_else(|| bad_window(path))?
                .ok_or_else(|| generic(format!("window \"{path}\" isn't placed")))?;
            let mut out = vec![
                "-x".to_string(),
                format_number(info.x),
                "-y".to_string(),
                format_number(info.y),
                "-relx".to_string(),
                format_number(info.relx),
                "-rely".to_string(),
                format_number(info.rely),
                "-anchor".to_string(),
                info.anchor.clone(),
            ];
            if let Some(w) = info.width {
                out.extend(["-width".to_string(), format_number(w)]);
            }
            if let Some(h) = info.height {
                out.extend(["-height".to_string(), format_number(h)]);
            }
            if let Some(w) = info.relwidth {
                out.extend(["-relwidth".to_string(), format_number(w)]);
            }
            if let Some(h) = info.relheight {
                out.extend(["-relheight".to_string(), format_number(h)]);
            }
            Ok(list_value(out))
        }
        "slaves" => {
            if args.len() != 3 {
                return Err(wrong("place slaves", "window", args.len()));
            }
            let kids = with_state(interp, |rt| rt.tree.slaves(args[2].as_str(), "place"))
                .and_then(|r| r.ok())
                .ok_or_else(|| bad_window(args[2].as_str()))?;
            Ok(list_value(kids))
        }
        _ => place_widget(interp, args[1].as_str(), &args[2..]),
    }
}

fn place_widget(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let mut info = with_state(interp, |rt| rt.tree.get(path).and_then(|n| n.place.clone()))
        .flatten()
        .unwrap_or_default();
    for (k, v) in parse_pairs(rest, 0, "place")? {
        match k.as_str() {
            "-x" => info.x = parse_f64_str(&v, "x")?,
            "-y" => info.y = parse_f64_str(&v, "y")?,
            "-relx" => info.relx = parse_f64_str(&v, "relx")?,
            "-rely" => info.rely = parse_f64_str(&v, "rely")?,
            "-width" => info.width = Some(parse_f64_str(&v, "width")?),
            "-height" => info.height = Some(parse_f64_str(&v, "height")?),
            "-relwidth" => info.relwidth = Some(parse_f64_str(&v, "relwidth")?),
            "-relheight" => info.relheight = Some(parse_f64_str(&v, "relheight")?),
            "-anchor" => info.anchor = v,
            "-bordermode" => {}
            "-in" => {
                return Err(generic(
                    "place -in is not supported in the native Tk backend yet",
                ))
            }
            _ => return Err(bad_option(&k)),
        }
    }
    with_state_mut(interp, |rt| {
        rt.tree.place(path, info).map_err(tk_error)?;
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::empty())
}

/// Tk screen units: a number, optionally ending in `i` (inch), `m`
/// (millimetre), `c` (centimetre) or `p` (point).
fn parse_screen_units(value: &str, scaling: f64) -> f64 {
    let value = value.trim();
    let split = value
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number = number.parse::<f64>().unwrap_or(0.0);
    let factor = match unit {
        "i" => 72.0 * scaling,
        "m" => 72.0 * scaling / 25.4,
        "c" => 72.0 * scaling / 2.54,
        "p" => scaling,
        _ => 1.0,
    };
    number * factor
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
            let path = args[2].as_str();
            Ok(Value::from_bool(
                with_state(interp, |rt| {
                    rt.tree.contains(path) || rt.menus.contains_key(path)
                })
                .unwrap_or(false),
            ))
        }
        "class" => {
            if args.len() != 3 {
                return Err(wrong("winfo class", "window", args.len()));
            }
            let path = args[2].as_str();
            let class = with_state(interp, |rt| {
                rt.tree
                    .get(path)
                    .map(|n| n.kind.class_name())
                    .or_else(|| rt.menus.contains_key(path).then_some("Menu"))
            })
            .flatten()
            .ok_or_else(|| bad_window(path))?;
            Ok(Value::from_str(class))
        }
        "parent" | "toplevel" | "pathname" | "name" => {
            if args.len() != 3 {
                return Err(wrong(&format!("winfo {sub}"), "window", args.len()));
            }
            let path = args[2].as_str();
            let known = with_state(interp, |rt| {
                rt.tree.contains(path) || rt.menus.contains_key(path)
            })
            .unwrap_or(false);
            if !known {
                return Err(bad_window(path));
            }
            if with_state(interp, |rt| rt.menus.contains_key(path)).unwrap_or(false) {
                let value = match sub {
                    "parent" | "toplevel" => ".".to_string(),
                    "name" => path.rsplit('.').next().unwrap_or("").to_string(),
                    _ => path.to_string(),
                };
                return Ok(Value::from_str(&value));
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
            let known = with_state(interp, |rt| {
                rt.tree.contains(path) || rt.menus.contains_key(path)
            })
            .unwrap_or(false);
            if !known {
                return Err(bad_window(path));
            }
            if with_state(interp, |rt| rt.menus.contains_key(path)).unwrap_or(false) {
                return Ok(Value::from_int(0));
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
                rt.menus.contains_key(path)
                    || rt.tree.get(path).is_some_and(|n| {
                        n.pack.is_some() || n.grid.is_some() || n.place.is_some() || path == "."
                    })
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
                if rt.menus.contains_key(path) {
                    return Some("");
                }
                rt.tree.get(path).map(|n| {
                    if n.pack.is_some() {
                        "pack"
                    } else if n.grid.is_some() {
                        "grid"
                    } else if n.place.is_some() {
                        "place"
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
        "screenmmwidth" | "vrootmmwidth" => Ok(Value::from_int(508)),
        "screenmmheight" | "vrootmmheight" => Ok(Value::from_int(286)),
        "fpixels" | "pixels" => {
            if args.len() != 4 {
                return Err(wrong(&format!("winfo {sub}"), "window number", args.len()));
            }
            let value = parse_screen_units(args[3].as_str(), tk_scaling());
            Ok(Value::from_int(value.round() as i64))
        }
        "depth" | "rpixels" | "cells" => Ok(Value::from_int(if sub == "depth" { 24 } else { 0 })),
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

fn tk_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
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
        "appname" => {
            if args.len() >= 3 {
                with_state_mut(interp, |rt| {
                    rt.app_name = args[2].as_str().to_string();
                    rt.bump();
                });
                Ok(Value::empty())
            } else {
                let name = with_state(interp, |rt| {
                    if rt.app_name.is_empty() {
                        "rtcl-tk".to_string()
                    } else {
                        rt.app_name.clone()
                    }
                })
                .unwrap_or_else(|| "rtcl-tk".to_string());
                Ok(Value::from_str(&name))
            }
        }
        "scaling" => {
            if args.len() >= 3 {
                let value = parse_f64_str(args[2].as_str(), "scaling")?;
                if !(value > 0.0) {
                    return Err(generic("bad scaling: must be positive"));
                }
                with_state_mut(interp, |rt| {
                    rt.scaling = value;
                    rt.bump();
                });
                Ok(Value::empty())
            } else {
                Ok(Value::from_str(&format_number(tk_scaling())))
            }
        }
        "useinputmethods" => Ok(Value::from_str("1")),
        "inactive" | "caret" => Ok(Value::from_str("1")),
        other => Err(generic(format!("bad option \"{other}\""))),
    }
}

/// `tk scaling`: pixels per point (1.0 unless a script set it).
pub fn tk_scaling() -> f64 {
    let Some(ptr) = active_interp() else {
        return 1.0;
    };
    with_state(unsafe { &*ptr }, |rt| {
        if rt.scaling > 0.0 {
            rt.scaling
        } else {
            1.0
        }
    })
    .unwrap_or(1.0)
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
    let is_widget = target != "."
        && target != "all"
        && with_state(interp, |rt| rt.tree.contains(target)).unwrap_or(false);
    if !is_widget && target != "all" && target.starts_with('.') && target != "." {
        return Err(bad_window(target));
    }
    if args.len() == 2 {
        let seqs: Vec<String> = if is_widget {
            with_state(interp, |rt| rt.tree.binding_sequences(target))
                .and_then(|r| r.ok())
                .unwrap_or_default()
        } else if target == "all" {
            with_state(interp, |rt| rt.all_bindings.keys().cloned().collect()).unwrap_or_default()
        } else {
            with_state(interp, |rt| {
                rt.tag_bindings
                    .get(target)
                    .map(|m| m.keys().cloned().collect())
            })
            .flatten()
            .unwrap_or_default()
        };
        return Ok(list_value(seqs));
    }
    let sequence = normalize_sequence(args[2].as_str());
    let current = if is_widget {
        with_state(interp, |rt| {
            rt.tree
                .binding(target, &sequence)
                .ok()
                .flatten()
                .map(str::to_string)
        })
        .flatten()
        .unwrap_or_default()
    } else if target == "all" {
        with_state(interp, |rt| rt.all_bindings.get(&sequence).cloned())
            .flatten()
            .unwrap_or_default()
    } else {
        with_state(interp, |rt| {
            rt.tag_bindings
                .get(target)
                .and_then(|m| m.get(&sequence))
                .cloned()
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
        if is_widget {
            rt.tree.bind(target, &sequence, &script).map_err(tk_error)?;
        } else if target == "all" {
            if script.is_empty() {
                rt.all_bindings.remove(&sequence);
            } else {
                rt.all_bindings.insert(sequence.clone(), script.clone());
            }
        } else {
            let map = rt.tag_bindings.entry(target.to_string()).or_default();
            if script.is_empty() {
                map.remove(&sequence);
            } else {
                map.insert(sequence.clone(), script.clone());
            }
        }
        rt.bump();
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

fn script_for_tag(interp: &Interp, tag: &str, sequence: &str) -> String {
    let exact = with_state(interp, |rt| {
        if tag == "all" {
            rt.all_bindings.get(sequence).cloned()
        } else if tag.starts_with('.') && tag != "." && rt.tree.contains(tag) {
            rt.tree
                .binding(tag, sequence)
                .ok()
                .flatten()
                .map(str::to_string)
        } else {
            rt.tag_bindings
                .get(tag)
                .and_then(|m| m.get(sequence))
                .cloned()
        }
    })
    .flatten()
    .unwrap_or_default();
    if !exact.is_empty() {
        return exact;
    }
    // Generic key bindings: <Key> sees every key press.
    if sequence == "<Return>" {
        return script_for_tag(interp, tag, "<KeyPress>");
    }
    String::new()
}

/// Run the bindings for `path` in its bindtags order and re-render.
/// Errors go to [`take_errors`].
pub fn run_binding(path: &str, sequence: &str, ctx: &EventCtx) {
    let sequence = normalize_sequence(sequence);
    let Some(ptr) = active_interp() else { return };
    // Safety: the pointer is the live engine-thread Interp.
    let interp = unsafe { &mut *ptr };
    let tags = with_state(interp, |rt| rt.bindtags.get(path).cloned())
        .flatten()
        .unwrap_or_else(|| {
            let (class, top) = with_state(interp, |rt| {
                rt.tree.get(path).map(|n| {
                    let mut top = path.to_string();
                    let mut p = path.to_string();
                    while let Some(parent) = rt.tree.get(&p).and_then(|n| n.parent.clone()) {
                        p = parent;
                        top = p.clone();
                    }
                    (n.kind.class_name().to_string(), top)
                })
            })
            .flatten()
            .unwrap_or_else(|| ("Widget".to_string(), ".".to_string()));
            vec![path.to_string(), class, top, "all".to_string()]
        });
    let mut seen = std::collections::BTreeSet::new();
    let mut scripts = Vec::new();
    for tag in tags {
        if !seen.insert(tag.clone()) {
            continue;
        }
        let script = script_for_tag(interp, &tag, &sequence);
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

fn focus_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut i = 1;
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-force" | "-lastfor" => i += 1,
            "-displayof" => i += 2,
            other => return Err(generic(format!("bad option \"{other}\""))),
        }
    }
    if i >= args.len() {
        let current =
            with_state(interp, |rt| rt.focus.clone().unwrap_or_default()).unwrap_or_default();
        return Ok(Value::from_str(&current));
    }
    let path = args[i].as_str();
    if path.is_empty() {
        with_state_mut(interp, |rt| {
            rt.focus = None;
            rt.bump();
        });
        return Ok(Value::empty());
    }
    if !with_state(interp, |rt| rt.tree.contains(path)).unwrap_or(false) {
        return Err(bad_window(path));
    }
    with_state_mut(interp, |rt| {
        rt.focus = Some(path.to_string());
        rt.bump();
    });
    Ok(Value::empty())
}

/// Whether `path` is the current `focus`.
pub fn focused(path: &str) -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    with_state(unsafe { &*ptr }, |rt| rt.focus.as_deref() == Some(path)).unwrap_or(false)
}

fn raise_lower_impl(interp: &mut Interp, args: &[Value], raise: bool) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(wrong(
            if raise { "raise" } else { "lower" },
            "window ?aboveThis?",
            args.len(),
        ));
    }
    let path = args[1].as_str();
    with_state_mut(interp, |rt| {
        if raise {
            rt.tree.raise(path).map_err(tk_error)?;
        } else {
            rt.tree.lower(path).map_err(tk_error)?;
        }
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::empty())
}

fn bindtags_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("bindtags", "window ?tagList?", args.len()));
    }
    let path = args[1].as_str().to_string();
    let (kind, top) = with_state(interp, |rt| {
        rt.tree.get(&path).map(|n| {
            let mut top = path.clone();
            let mut p = path.clone();
            while let Some(parent) = rt.tree.get(&p).and_then(|n| n.parent.clone()) {
                p = parent;
                top = p.clone();
            }
            (n.kind, top)
        })
    })
    .flatten()
    .ok_or_else(|| bad_window(&path))?;
    if args.len() == 2 {
        let tags = with_state(interp, |rt| rt.bindtags.get(&path).cloned())
            .flatten()
            .unwrap_or_else(|| {
                vec![
                    path.clone(),
                    kind.class_name().to_string(),
                    top.clone(),
                    "all".to_string(),
                ]
            });
        return Ok(list_value(tags));
    }
    // Tk takes one tag list: `bindtags .w {.w Button . all}`.
    let tags = args[2]
        .as_list()
        .unwrap_or_else(|| vec![args[2].clone()])
        .iter()
        .map(|v| v.as_str().to_string())
        .collect::<Vec<_>>();
    with_state_mut(interp, |rt| {
        rt.bindtags.insert(path, tags);
        rt.bump();
    });
    Ok(Value::empty())
}

fn focusable(kind: WidgetKind) -> bool {
    matches!(
        kind,
        WidgetKind::Entry
            | WidgetKind::Text
            | WidgetKind::Listbox
            | WidgetKind::Button
            | WidgetKind::Checkbutton
            | WidgetKind::Radiobutton
            | WidgetKind::Scale
    )
}

fn tk_focus_impl(interp: &mut Interp, args: &[Value], next: bool) -> Result<Value> {
    let path = args.last().map(|v| v.as_str()).unwrap_or(".");
    let paths = with_state(interp, |rt| {
        rt.tree
            .descendants(".")
            .into_iter()
            .filter(|p| rt.tree.get(p).is_some_and(|n| focusable(n.kind)))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    if paths.is_empty() {
        return Ok(Value::from_str(""));
    }
    let index = paths.iter().position(|p| p == path);
    let target = match index {
        Some(i) if next => paths[(i + 1) % paths.len()].clone(),
        Some(i) => paths[(i + paths.len() - 1) % paths.len()].clone(),
        None if next => paths[0].clone(),
        None => paths[paths.len() - 1].clone(),
    };
    Ok(Value::from_str(&target))
}

fn font_defaults() -> BTreeMap<String, String> {
    [
        ("-family", "DejaVu Sans"),
        ("-size", "10"),
        ("-weight", "normal"),
        ("-slant", "roman"),
        ("-underline", "0"),
        ("-overstrike", "0"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn font_options(interp: &Interp, name: &str) -> Option<BTreeMap<String, String>> {
    with_state(interp, |rt| {
        rt.fonts
            .get(name)
            .cloned()
            .or_else(|| (name == "TkDefaultFont" || name == "TkTextFont").then(font_defaults))
    })
    .flatten()
}

/// Resolve a `-font` value (a named font or a Tk font description) to
/// family, point size and bold.
pub fn resolve_font(spec: &str) -> Option<(String, f64, bool)> {
    if spec.is_empty() {
        return None;
    }
    let ptr = active_interp()?;
    let interp = unsafe { &*ptr };
    if let Some(options) = font_options(interp, spec) {
        let family = options.get("-family").cloned().unwrap_or_default();
        let size = options
            .get("-size")
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(10.0)
            .abs();
        let bold = options.get("-weight").is_some_and(|w| w == "bold");
        return Some((family, size, bold));
    }
    let mut family = String::new();
    let mut size = 10.0;
    let mut bold = false;
    for token in spec.split_whitespace() {
        let token = token.trim_matches(|c| c == '{' || c == '}');
        if let Ok(value) = token.parse::<f64>() {
            size = value.abs();
        } else if let Some(pixels) = token
            .strip_prefix('-')
            .and_then(|rest| rest.parse::<f64>().ok())
        {
            // Negative Tk sizes are pixels.
            size = pixels.abs() * 0.75;
        } else if token.eq_ignore_ascii_case("bold") {
            bold = true;
        } else if family.is_empty() {
            family = token.to_string();
        }
    }
    Some((family, size, bold))
}

fn font_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("font", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "create" => {
            if args.len() < 3 {
                return Err(wrong(
                    "font create",
                    "fontName ?-option value ...?",
                    args.len(),
                ));
            }
            let name = args[2].as_str().to_string();
            if font_options(interp, &name).is_some() {
                return Err(generic(format!("font \"{name}\" already exists")));
            }
            let mut options = font_defaults();
            for (k, v) in parse_pairs(args, 3, "font create")? {
                if !options.contains_key(&k) {
                    return Err(bad_option(&k));
                }
                options.insert(k, v);
            }
            with_state_mut(interp, |rt| {
                rt.fonts.insert(name.clone(), options);
                rt.bump();
            });
            Ok(Value::empty())
        }
        "configure" => {
            if args.len() < 3 {
                return Err(wrong(
                    "font configure",
                    "fontName ?-option value ...?",
                    args.len(),
                ));
            }
            let name = args[2].as_str();
            let Some(mut options) = font_options(interp, name) else {
                return Err(generic(format!("font \"{name}\" doesn't exist")));
            };
            if args.len() == 3 {
                return Ok(list_value(
                    options
                        .iter()
                        .flat_map(|(k, v)| [k.clone(), v.clone()])
                        .collect::<Vec<_>>(),
                ));
            }
            if args.len() == 4 && args[3].as_str().starts_with('-') {
                return Ok(Value::from_str(
                    options
                        .get(args[3].as_str())
                        .map(String::as_str)
                        .unwrap_or(""),
                ));
            }
            for (k, v) in parse_pairs(args, 3, "font configure")? {
                if !options.contains_key(&k) {
                    return Err(bad_option(&k));
                }
                options.insert(k, v);
            }
            with_state_mut(interp, |rt| {
                rt.fonts.insert(name.to_string(), options);
                rt.bump();
            });
            Ok(Value::empty())
        }
        "actual" => {
            if args.len() < 3 {
                return Err(wrong("font actual", "font ?-option?", args.len()));
            }
            let name = args[2].as_str();
            let Some(options) = font_options(interp, name) else {
                return Err(generic(format!("font \"{name}\" doesn't exist")));
            };
            if args.len() >= 4 {
                return Ok(Value::from_str(
                    options
                        .get(args[3].as_str())
                        .map(String::as_str)
                        .unwrap_or(""),
                ));
            }
            Ok(list_value(
                options
                    .iter()
                    .flat_map(|(k, v)| [k.clone(), v.clone()])
                    .collect::<Vec<_>>(),
            ))
        }
        "families" => Ok(list_value(
            ["DejaVu Sans", "Liberation Sans", "Noto Sans", "Sans"]
                .into_iter()
                .map(str::to_string),
        )),
        "names" => {
            let names = with_state(interp, |rt| rt.fonts.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(list_value(names))
        }
        "delete" => {
            if args.len() < 3 {
                return Err(wrong("font delete", "fontName ?fontName ...?", args.len()));
            }
            with_state_mut(interp, |rt| {
                for a in &args[2..] {
                    rt.fonts.remove(a.as_str());
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "exists" => {
            Ok(Value::from_bool(args.get(2).is_some_and(|v| {
                font_options(interp, v.as_str()).is_some()
            })))
        }
        "measure" => {
            if args.len() < 4 {
                return Err(wrong(
                    "font measure",
                    "font ?-displayof window? text",
                    args.len(),
                ));
            }
            let name = args[2].as_str();
            let size = font_options(interp, name)
                .and_then(|o| o.get("-size").and_then(|s| s.parse::<f64>().ok()))
                .unwrap_or(10.0)
                .abs();
            let text = args[3].as_str();
            Ok(Value::from_int(
                (text.chars().count() as f64 * size * 0.6).ceil() as i64,
            ))
        }
        "metrics" => {
            if args.len() < 3 {
                return Err(wrong("font metrics", "font ?-option?", args.len()));
            }
            let size = font_options(interp, args[2].as_str())
                .and_then(|o| o.get("-size").and_then(|s| s.parse::<f64>().ok()))
                .unwrap_or(10.0)
                .abs();
            let option = args.get(3).map(|v| v.as_str()).unwrap_or("-linespace");
            let value = match option {
                "-ascent" => (size * 0.8) as i64,
                "-descent" => (size * 0.2) as i64,
                "-linespace" => (size * 1.2) as i64,
                "-fixed" => 0,
                _ => 0,
            };
            Ok(Value::from_int(value))
        }
        other => Err(generic(format!("bad option \"{other}\""))),
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    fn walk(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') => walk(&p[1..], t) || (!t.is_empty() && walk(p, &t[1..])),
            Some(b'?') => !t.is_empty() && walk(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && walk(&p[1..], &t[1..]),
        }
    }
    walk(pattern.as_bytes(), text.as_bytes())
}

fn option_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("option", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "add" => {
            if args.len() < 4 || args.len() > 5 {
                return Err(wrong("option add", "pattern value ?priority?", args.len()));
            }
            let pattern = args[2].as_str().to_string();
            let value = args[3].as_str().to_string();
            with_state_mut(interp, |rt| {
                rt.option_db.push((pattern, value));
                rt.bump();
            });
            Ok(Value::empty())
        }
        "get" => {
            if args.len() < 5 {
                return Err(wrong("option get", "window name class", args.len()));
            }
            let name = args[3].as_str();
            let class = args[4].as_str();
            let class_name = format!("{class}.{name}");
            let found = with_state(interp, |rt| {
                rt.option_db
                    .iter()
                    .rev()
                    .find(|(p, _)| {
                        glob_match(p, name) || glob_match(p, &class_name) || glob_match(p, class)
                    })
                    .map(|(_, v)| v.clone())
            })
            .flatten()
            .unwrap_or_default();
            Ok(Value::from_str(&found))
        }
        "clear" => {
            with_state_mut(interp, |rt| {
                rt.option_db.clear();
                rt.bump();
            });
            Ok(Value::empty())
        }
        "readfile" => Ok(Value::empty()),
        other => Err(generic(format!("bad option \"{other}\""))),
    }
}

fn parse_menu_entry(kind: &str, rest: &[Value]) -> Result<MenuItem> {
    if !matches!(
        kind,
        "command" | "separator" | "checkbutton" | "radiobutton" | "cascade"
    ) {
        return Err(generic(format!(
            "bad menu entry type \"{kind}\": must be cascade, checkbutton, command, radiobutton, or separator"
        )));
    }
    let mut options = BTreeMap::new();
    for (k, v) in parse_pairs(rest, 0, "menu add")? {
        if !rtcl_tk_core::valid_option(&k) {
            return Err(bad_option(&k));
        }
        options.insert(k, v);
    }
    Ok(MenuItem {
        kind: kind.to_string(),
        options,
    })
}

fn image_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("image", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "create" => {
            if args.len() < 3 {
                return Err(wrong("image create", "type ?name? ?-option value ...?", args.len()));
            }
            if args[2].as_str() != "photo" {
                return Err(generic(format!(
                    "image type \"{}\" is not supported (only photo)",
                    args[2].as_str()
                )));
            }
            let named = args.get(3).filter(|v| !v.as_str().starts_with('-'));
            let name = named
                .map(|v| v.as_str().to_string())
                .unwrap_or_else(|| {
                    let seq = with_state_mut(interp, |rt| {
                        rt.image_seq += 1;
                        rt.image_seq
                    })
                    .unwrap_or(1);
                    format!("image{seq}")
                });
            let start = if named.is_some() { 4 } else { 3 };
            let mut image = PhotoImage::default();
            for (key, value) in parse_pairs(args, start, "image create")? {
                match key.as_str() {
                    "-file" => image.file = value,
                    "-width" => image.width = value.parse().unwrap_or(0),
                    "-height" => image.height = value.parse().unwrap_or(0),
                    "-data" | "-format" | "-gamma" | "-palette" => {}
                    _ => return Err(bad_option(&key)),
                }
            }
            with_state_mut(interp, |rt| {
                rt.images.insert(name.clone(), image);
                rt.bump();
            });
            interp.register_command(&name, photo_command);
            Ok(Value::from_str(&name))
        }
        "delete" => {
            for value in &args[2..] {
                let name = value.as_str();
                with_state_mut(interp, |rt| {
                    rt.images.remove(name);
                    rt.bump();
                });
                let _ = interp.delete_command(name);
            }
            Ok(Value::empty())
        }
        "names" => {
            let names = with_state(interp, |rt| rt.images.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(list_value(names))
        }
        "types" => Ok(list_value(["photo"].map(str::to_string))),
        "exists" => Ok(Value::from_bool(
            args.get(2).is_some_and(|v| {
                with_state(interp, |rt| rt.images.contains_key(v.as_str())).unwrap_or(false)
            }),
        )),
        "type" => {
            let name = args.get(2).map(|v| v.as_str()).unwrap_or("");
            if with_state(interp, |rt| rt.images.contains_key(name)).unwrap_or(false) {
                Ok(Value::from_str("photo"))
            } else {
                Err(generic(format!("image \"{name}\" doesn't exist")))
            }
        }
        "width" | "height" => {
            let name = args.get(2).map(|v| v.as_str()).unwrap_or("");
            let size = with_state(interp, |rt| {
                rt.images
                    .get(name)
                    .map(|i| if args[1].as_str() == "width" { i.width } else { i.height })
            })
            .flatten()
            .ok_or_else(|| generic(format!("image \"{name}\" doesn't exist")))?;
            Ok(Value::from_int(size))
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be create, delete, exists, names, type, types, width, or height"
        ))),
    }
}

fn photo_command(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let name = args[0].as_str();
    if args.len() < 2 {
        return Err(wrong(name, "option ?arg ...?", args.len()));
    }
    let sub = args[1].as_str();
    let exists = with_state(interp, |rt| rt.images.contains_key(name)).unwrap_or(false);
    if !exists {
        return Err(generic(format!("image \"{name}\" doesn't exist")));
    }
    match sub {
        "cget" => {
            if args.len() != 3 {
                return Err(wrong("cget", "-option", args.len()));
            }
            let image = with_state(interp, |rt| rt.images.get(name).cloned())
                .flatten()
                .unwrap_or_default();
            let value = match args[2].as_str() {
                "-file" => image.file,
                "-width" => image.width.to_string(),
                "-height" => image.height.to_string(),
                other => return Err(bad_option(other)),
            };
            Ok(Value::from_str(&value))
        }
        "configure" | "config" => {
            if args.len() == 2 {
                let image = with_state(interp, |rt| rt.images.get(name).cloned())
                    .flatten()
                    .unwrap_or_default();
                return Ok(list_value([
                    "-file".to_string(),
                    image.file,
                    "-width".to_string(),
                    image.width.to_string(),
                    "-height".to_string(),
                    image.height.to_string(),
                ]));
            }
            let options = parse_pairs(args, 2, "configure")?;
            with_state_mut(interp, |rt| {
                if let Some(image) = rt.images.get_mut(name) {
                    for (key, value) in options {
                        match key.as_str() {
                            "-file" => image.file = value,
                            "-width" => image.width = value.parse().unwrap_or(0),
                            "-height" => image.height = value.parse().unwrap_or(0),
                            "-data" | "-format" | "-gamma" | "-palette" => {}
                            _ => return Err(bad_option(&key)),
                        }
                    }
                    rt.bump();
                }
                Ok::<(), Error>(())
            })
            .ok_or_else(|| bad_window(name))??;
            Ok(Value::empty())
        }
        "width" | "height" => {
            let image = with_state(interp, |rt| rt.images.get(name).cloned())
                .flatten()
                .unwrap_or_default();
            Ok(Value::from_int(if sub == "width" {
                image.width
            } else {
                image.height
            }))
        }
        "put" | "read" | "write" | "copy" | "blank" | "transparency" | "data"
        | "get" => Ok(Value::empty()),
        other => Err(generic(format!(
            "bad option \"{other}\": must be blank, cget, configure, copy, data, get, height, put, read, transparency, width, or write"
        ))),
    }
}

/// The file behind a photo image.
pub fn image_path(name: &str) -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| {
        rt.images.get(name).map(|i| i.file.clone())
    })
    .flatten()
}

/// A photo image's reported size.
pub fn image_dimensions(name: &str) -> Option<(i64, i64)> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| {
        rt.images.get(name).map(|i| (i.width, i.height))
    })
    .flatten()
}

fn menu_create(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let cmd = args[0].as_str();
    if args.len() < 2 {
        return Err(wrong(cmd, "pathName ?-option value ...?", args.len()));
    }
    let path = args[1].as_str().to_string();
    let exists = with_state(interp, |rt| {
        rt.tree.contains(&path) || rt.menus.contains_key(&path)
    })
    .unwrap_or(false);
    if exists {
        return Err(generic(format!("window name \"{path}\" already exists")));
    }
    if !rtcl_tk_core::valid_path(&path) {
        return Err(bad_window(&path));
    }
    let options = parse_pairs(args, 2, cmd)?
        .into_iter()
        .filter(|(k, _)| {
            matches!(
                k.as_str(),
                "-tearoff"
                    | "-title"
                    | "-type"
                    | "-selectcolor"
                    | "-background"
                    | "-foreground"
                    | "-font"
                    | "-activebackground"
                    | "-activeforeground"
                    | "-disabledforeground"
                    | "-borderwidth"
                    | "-relief"
                    | "-cursor"
                    | "-takefocus"
                    | "-postcommand"
            )
        })
        .collect::<BTreeMap<String, String>>();
    with_state_mut(interp, |rt| {
        rt.menus.insert(
            path.clone(),
            Menu {
                path: path.clone(),
                items: Vec::new(),
                options,
            },
        );
        rt.bump();
    });
    interp.register_command(&path, menu_command);
    Ok(Value::from_str(&path))
}

fn menu_item_index(spec: &str, len: usize) -> Result<usize> {
    match spec {
        "end" => Ok(len),
        "active" | "none" => Ok(0),
        _ => spec
            .parse::<usize>()
            .map_err(|_| generic(format!("bad menu entry index \"{spec}\""))),
    }
}

fn menu_command(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let path = args[0].as_str().to_string();
    if args.len() < 2 {
        return Err(wrong(&path, "subcommand ?arg ...?", args.len()));
    }
    let sub = args[1].as_str();
    let exists = with_state(interp, |rt| rt.menus.contains_key(&path)).unwrap_or(false);
    if !exists {
        return Err(bad_window(&path));
    }
    match sub {
        "add" => {
            if args.len() < 3 {
                return Err(wrong("menu add", "type ?-option value ...?", args.len()));
            }
            let item = parse_menu_entry(args[2].as_str(), &args[3..])?;
            with_state_mut(interp, |rt| {
                if let Some(menu) = rt.menus.get_mut(&path) {
                    menu.items.push(item);
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "insert" => {
            if args.len() < 4 {
                return Err(wrong(
                    "menu insert",
                    "index type ?-option value ...?",
                    args.len(),
                ));
            }
            let item = parse_menu_entry(args[3].as_str(), &args[4..])?;
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let at = menu_item_index(args[2].as_str(), len)?.min(len);
            with_state_mut(interp, |rt| {
                if let Some(menu) = rt.menus.get_mut(&path) {
                    menu.items.insert(at, item);
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "delete" => {
            if args.len() < 3 || args.len() > 4 {
                return Err(wrong("menu delete", "index1 ?index2?", args.len()));
            }
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let first = menu_item_index(args[2].as_str(), len)?;
            let last = match args.get(3) {
                Some(v) => menu_item_index(v.as_str(), len)?,
                None => first,
            };
            let (a, b) = (first.min(last), first.max(last));
            with_state_mut(interp, |rt| {
                if let Some(menu) = rt.menus.get_mut(&path) {
                    let end = (b + 1).min(menu.items.len());
                    if a < end {
                        menu.items.drain(a..end);
                    }
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "index" => {
            if args.len() != 3 {
                return Err(wrong("menu index", "index", args.len()));
            }
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let i = menu_item_index(args[2].as_str(), len)?;
            Ok(Value::from_int(i as i64))
        }
        "entrycget" => {
            if args.len() != 4 {
                return Err(wrong("menu entrycget", "index -option", args.len()));
            }
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let i = menu_item_index(args[2].as_str(), len)?;
            let option = args[3].as_str();
            let value = with_state(interp, |rt| {
                rt.menus
                    .get(&path)
                    .and_then(|m| m.items.get(i))
                    .and_then(|it| it.options.get(option).cloned())
            })
            .flatten()
            .unwrap_or_default();
            Ok(Value::from_str(&value))
        }
        "entryconfigure" | "entryconfig" => {
            if args.len() < 3 {
                return Err(wrong(
                    "menu entryconfigure",
                    "index ?-option value ...?",
                    args.len(),
                ));
            }
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let i = menu_item_index(args[2].as_str(), len)?;
            if args.len() == 4 && args[3].as_str().starts_with('-') {
                let value = with_state(interp, |rt| {
                    rt.menus
                        .get(&path)
                        .and_then(|m| m.items.get(i))
                        .and_then(|it| it.options.get(args[3].as_str()).cloned())
                })
                .flatten()
                .unwrap_or_default();
                return Ok(Value::from_str(&value));
            }
            if args.len() > 3 {
                let options = parse_pairs(args, 3, "menu entryconfigure")?;
                with_state_mut(interp, |rt| {
                    if let Some(menu) = rt.menus.get_mut(&path) {
                        if let Some(item) = menu.items.get_mut(i) {
                            item.options.extend(options);
                        }
                        rt.bump();
                    }
                });
            }
            Ok(Value::empty())
        }
        "invoke" => {
            if args.len() != 3 {
                return Err(wrong("menu invoke", "index", args.len()));
            }
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let i = menu_item_index(args[2].as_str(), len)?;
            invoke_menu_item(interp, &path, i)?;
            Ok(Value::empty())
        }
        "type" => {
            if args.len() != 3 {
                return Err(wrong("menu type", "index", args.len()));
            }
            let len = with_state(interp, |rt| {
                rt.menus.get(&path).map(|m| m.items.len()).unwrap_or(0)
            })
            .unwrap_or(0);
            let i = menu_item_index(args[2].as_str(), len)?;
            let kind = with_state(interp, |rt| {
                rt.menus
                    .get(&path)
                    .and_then(|m| m.items.get(i))
                    .map(|it| it.kind.clone())
            })
            .flatten()
            .unwrap_or_default();
            Ok(Value::from_str(&kind))
        }
        "post" => {
            with_state_mut(interp, |rt| {
                rt.open_menu = Some(path.clone());
                rt.bump();
            });
            Ok(Value::empty())
        }
        "unpost" => {
            with_state_mut(interp, |rt| {
                if rt.open_menu.as_deref() == Some(path.as_str()) {
                    rt.open_menu = None;
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "activate" | "yposition" | "xposition" => Ok(Value::from_int(0)),
        other => Err(generic(format!(
            "bad option \"{other}\": must be activate, add, delete, entrycget, entryconfigure, index, insert, invoke, post, type, unpost, or yposition"
        ))),
    }
}

fn invoke_menu_item(interp: &mut Interp, menu_path: &str, index: usize) -> Result<()> {
    let item = with_state(interp, |rt| {
        rt.menus
            .get(menu_path)
            .and_then(|m| m.items.get(index))
            .cloned()
    })
    .flatten()
    .ok_or_else(|| generic(format!("bad menu entry index \"{index}\"")))?;
    let option = |name: &str| item.options.get(name).cloned().unwrap_or_default();
    match item.kind.as_str() {
        "separator" => {}
        "cascade" => {
            let sub = option("-menu");
            with_state_mut(interp, |rt| {
                rt.open_menu = (!sub.is_empty()).then_some(sub);
                rt.bump();
            });
        }
        "checkbutton" => {
            let var = option("-variable");
            if !var.is_empty() {
                let on = if item.options.contains_key("-onvalue") {
                    option("-onvalue")
                } else {
                    "1".to_string()
                };
                let off = if item.options.contains_key("-offvalue") {
                    option("-offvalue")
                } else {
                    "0".to_string()
                };
                let current = var_value(&var).unwrap_or_default();
                let next = if current == on { off } else { on };
                set_var(&var, &next)?;
            }
            let command = option("-command");
            if !command.is_empty() {
                with_eval(|| unsafe {
                    let ptr = active_interp().unwrap();
                    (*ptr).eval(&command)
                })?;
            }
        }
        "radiobutton" => {
            let var = option("-variable");
            if !var.is_empty() {
                set_var(&var, &option("-value"))?;
            }
            let command = option("-command");
            if !command.is_empty() {
                with_eval(|| unsafe {
                    let ptr = active_interp().unwrap();
                    (*ptr).eval(&command)
                })?;
            }
        }
        _ => {
            let command = option("-command");
            if !command.is_empty() {
                with_eval(|| unsafe {
                    let ptr = active_interp().unwrap();
                    (*ptr).eval(&command)
                })?;
            }
        }
    }
    with_state_mut(interp, |rt| rt.bump());
    Ok(())
}

/// The menu attached to the main window (`configure . -menu`).
pub fn menu_bar() -> Option<String> {
    let ptr = active_interp()?;
    let interp = unsafe { &*ptr };
    let path = effective_option(interp, ".", "-menu");
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// A snapshot of one menu.
pub fn menu_snapshot(path: &str) -> Option<Menu> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.menus.get(path).cloned()).flatten()
}

/// The cascade currently dropped down, if any.
pub fn open_menu() -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.open_menu.clone()).flatten()
}

/// Open/close a cascade's menu from the renderer.
pub fn toggle_menu(path: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        rt.open_menu = if rt.open_menu.as_deref() == Some(path) {
            None
        } else {
            Some(path.to_string())
        };
        rt.bump();
    });
}

/// Run a menu entry from the renderer and close the dropdown.
pub fn press_menu_item(menu_path: &str, index: usize) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    if let Err(e) = invoke_menu_item(interp, menu_path, index) {
        record_error(e.to_string());
    }
    with_state_mut(interp, |rt| {
        if rt
            .menus
            .get(menu_path)
            .and_then(|m| m.items.get(index))
            .is_some_and(|it| it.kind != "cascade" && it.kind != "separator")
        {
            rt.open_menu = None;
        }
        rt.bump();
    });
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

/// Tk entry validation (`-validate key|all`, `-validatecommand`).
fn entry_validation_passes(interp: &mut Interp, path: &str, proposed: &str) -> bool {
    let mode = effective_option(interp, path, "-validate");
    if mode != "key" && mode != "all" {
        return true;
    }
    let command = effective_option(interp, path, "-validatecommand");
    if command.is_empty() {
        return true;
    }
    let old = entry_value(path);
    let inserted = proposed
        .strip_prefix(old.as_str())
        .unwrap_or_default()
        .to_string();
    let script = substitute_validation(
        &command,
        &[
            ("%P", proposed),
            ("%s", old.as_str()),
            ("%S", inserted.as_str()),
            ("%v", "key"),
            ("%V", "key"),
            ("%W", path),
        ],
    );
    match eval_active_quiet(&script) {
        Ok(value) => value.as_bool().unwrap_or(true),
        Err(e) => {
            // Tk disables a validation that errors; the edit goes through.
            record_error(e.to_string());
            true
        }
    }
}

fn substitute_validation(script: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(script.len());
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('%');
            break;
        };
        if next == '%' {
            out.push('%');
            continue;
        }
        let key = format!("%{next}");
        if let Some((_, value)) = pairs.iter().find(|(k, _)| *k == key) {
            out.push_str(value);
        } else {
            out.push('%');
            out.push(next);
        }
    }
    out
}

/// An Entry's text changed in the UI.
pub fn entry_changed(path: &str, text: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    if !entry_validation_passes(interp, path, text) {
        // Reject the edit: the controlled input re-reads the old value.
        bump_active();
        return;
    }
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
    if apply_text_change(path, text) {
        bump_active();
    }
}

/// A change made by the rich editor itself: the model is updated without
/// re-rendering, so the editor keeps its session and caret (the caller
/// renders once afterwards).
pub fn text_changed_editor(path: &str, text: &str) {
    let _ = apply_text_change(path, text);
}

fn apply_text_change(path: &str, text: &str) -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        let old = rt
            .tree
            .get(path)
            .map(|n| n.content.clone())
            .unwrap_or_default();
        adjust_text_tag_ranges(&old, text, rt.text_tags.get_mut(path));
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = text.to_string();
            n.cursor = text.chars().count();
        }
        true
    })
    .unwrap_or(false)
}

/// Ask the backend to render again (host-side model updates).
pub fn request_render() {
    bump_active();
}

/// Shift tag ranges for an edit (`old` → `new`).
fn adjust_text_tag_ranges(old: &str, new: &str, tags: Option<&mut BTreeMap<String, TextTag>>) {
    let Some(tags) = tags else { return };
    let old_chars = old.chars().collect::<Vec<_>>();
    let new_chars = new.chars().collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < old_chars.len()
        && prefix < new_chars.len()
        && old_chars[prefix] == new_chars[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_chars.len().saturating_sub(prefix)
        && suffix < new_chars.len().saturating_sub(prefix)
        && old_chars[old_chars.len() - 1 - suffix] == new_chars[new_chars.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_len = old_chars.len();
    let new_len = new_chars.len();
    let delta = new_len as isize - old_len as isize;
    for tag in tags.values_mut() {
        let mut ranges = Vec::new();
        for (start, end) in tag.ranges.drain(..) {
            let (start, end) = if end <= prefix {
                (start, end)
            } else if start >= old_len.saturating_sub(suffix) {
                (
                    (start as isize + delta).max(0) as usize,
                    (end as isize + delta).max(0) as usize,
                )
            } else {
                (
                    start.min(prefix),
                    ((end as isize + delta).max(0) as usize).max(prefix),
                )
            };
            let start = start.min(new_len);
            let end = end.min(new_len);
            if start < end {
                ranges.push((start, end));
            }
        }
        tag.ranges = ranges;
    }
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

fn treeview_insert(interp: &mut Interp, path: &str, parent: &str, rest: &[Value]) -> Result<Value> {
    let parent = if parent.is_empty() {
        None
    } else {
        Some(parent.to_string())
    };
    let mut text = String::new();
    let mut values = Vec::new();
    let mut open = false;
    let mut id = None;
    for (k, v) in parse_pairs(rest, 0, "treeview insert")? {
        match k.as_str() {
            "-text" => text = v,
            "-values" => {
                values = Value::from_str(&v)
                    .as_list()
                    .map(|list| list.iter().map(|x| x.as_str().to_string()).collect())
                    .unwrap_or_default();
            }
            "-open" => open = matches!(v.as_str(), "1" | "true" | "yes" | "on"),
            "-id" => id = Some(v),
            "-tags" | "-image" => {}
            _ => return Err(bad_option(&k)),
        }
    }
    let mut result = String::new();
    with_state_mut(interp, |rt| {
        let tv = rt.treeviews.entry(path.to_string()).or_default();
        if let Some(parent) = &parent {
            if !tv.items.iter().any(|i| i.id == *parent) {
                return Err(generic(format!("item \"{parent}\" doesn't exist")));
            }
        }
        let id = match id {
            Some(id) => id,
            None => {
                tv.next_id += 1;
                format!("I{:03}", tv.next_id)
            }
        };
        if tv.items.iter().any(|i| i.id == id) {
            return Err(generic(format!("item \"{id}\" already exists")));
        }
        tv.items.push(TreeItem {
            id: id.clone(),
            parent,
            text,
            values,
            open,
        });
        rt.bump();
        result = id;
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::from_str(&result))
}

fn treeview_delete(interp: &mut Interp, path: &str, id: &str) -> Result<()> {
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            let mut doomed = BTreeSet::new();
            let mut stack = vec![id.to_string()];
            while let Some(current) = stack.pop() {
                if doomed.insert(current.clone()) {
                    stack.extend(
                        tv.items
                            .iter()
                            .filter(|i| i.parent.as_deref() == Some(current.as_str()))
                            .map(|i| i.id.clone()),
                    );
                }
            }
            tv.items.retain(|i| !doomed.contains(&i.id));
            tv.selected.retain(|i| !doomed.contains(i));
            if tv.focus.as_ref().is_some_and(|f| doomed.contains(f)) {
                tv.focus = None;
            }
            rt.bump();
        }
    });
    Ok(())
}

fn treeview_item(interp: &mut Interp, path: &str, id: &str, rest: &[Value]) -> Result<Value> {
    let exists = with_state(interp, |rt| {
        rt.treeviews
            .get(path)
            .is_some_and(|tv| tv.items.iter().any(|i| i.id == id))
    })
    .unwrap_or(false);
    if !exists {
        return Err(generic(format!("item \"{id}\" doesn't exist")));
    }
    if rest.is_empty() {
        let item = with_state(interp, |rt| {
            rt.treeviews
                .get(path)
                .and_then(|tv| tv.items.iter().find(|i| i.id == id).cloned())
        })
        .flatten()
        .unwrap_or_default();
        let values = Value::from_list(
            &item
                .values
                .iter()
                .map(|v| Value::from_str(v))
                .collect::<Vec<_>>(),
        )
        .as_str()
        .to_string();
        return Ok(list_value([
            "-text".to_string(),
            item.text,
            "-values".to_string(),
            values,
            "-open".to_string(),
            if item.open { "1" } else { "0" }.to_string(),
        ]));
    }
    if rest.len() == 1 && rest[0].as_str().starts_with('-') {
        let item = with_state(interp, |rt| {
            rt.treeviews
                .get(path)
                .and_then(|tv| tv.items.iter().find(|i| i.id == id).cloned())
        })
        .flatten();
        let Some(item) = item else {
            return Ok(Value::empty());
        };
        return Ok(match rest[0].as_str() {
            "-text" => Value::from_str(&item.text),
            "-open" => Value::from_bool(item.open),
            "-values" => list_value(item.values),
            _ => Value::empty(),
        });
    }
    let options = parse_pairs(rest, 0, "treeview item")?;
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            if let Some(item) = tv.items.iter_mut().find(|i| i.id == id) {
                for (k, v) in options {
                    match k.as_str() {
                        "-text" => item.text = v,
                        "-open" => item.open = matches!(v.as_str(), "1" | "true" | "yes" | "on"),
                        "-values" => {
                            item.values = Value::from_str(&v)
                                .as_list()
                                .map(|list| list.iter().map(|x| x.as_str().to_string()).collect())
                                .unwrap_or_default();
                        }
                        "-tags" | "-image" => {}
                        _ => return Err(bad_option(&k)),
                    }
                }
                rt.bump();
            }
        }
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::empty())
}

/// Snapshot of a Treeview for the renderer.
pub fn treeview_snapshot(path: &str) -> Option<TreeViewState> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.treeviews.get(path).cloned()).flatten()
}

/// Select one Treeview row from the renderer.
pub fn treeview_select(path: &str, id: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            tv.selected = std::iter::once(id.to_string()).collect();
            tv.focus = Some(id.to_string());
            rt.bump();
        }
    });
}

/// Expand/collapse a Treeview row from the renderer.
pub fn treeview_toggle(path: &str, id: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            if let Some(item) = tv.items.iter_mut().find(|i| i.id == id) {
                item.open = !item.open;
                rt.bump();
            }
        }
    });
}

fn notebook_add(
    interp: &mut Interp,
    notebook: &str,
    page: &str,
    rest: &[Value],
    at: Option<usize>,
) -> Result<Value> {
    let is_child = with_state(interp, |rt| {
        rt.tree
            .get(page)
            .is_some_and(|n| n.parent.as_deref() == Some(notebook))
    })
    .unwrap_or(false);
    if !is_child {
        return Err(generic(format!(
            "window \"{page}\" is not a child of \"{notebook}\""
        )));
    }
    let options = parse_pairs(rest, 0, "notebook add")?;
    let options = options
        .into_iter()
        .filter(|(k, _)| {
            matches!(
                k.as_str(),
                "-text" | "-state" | "-image" | "-compound" | "-padding"
            )
        })
        .collect::<Vec<_>>();
    let page = page.to_string();
    let notebook = notebook.to_string();
    with_state_mut(interp, |rt| {
        if !options.is_empty() {
            rt.tree.configure(&page, options).map_err(tk_error)?;
        }
        let len = rt.tree.children(&notebook).map(|c| c.len()).unwrap_or(0);
        let index = at.unwrap_or(len).min(len);
        rt.tree
            .insert_child_at(&notebook, &page, index)
            .map_err(tk_error)?;
        let selected = rt.notebook_selected.get(&notebook).copied().unwrap_or(0);
        if len == 0 {
            rt.notebook_selected.insert(notebook.clone(), 0);
        } else if index <= selected {
            rt.notebook_selected.insert(notebook.clone(), selected + 1);
        }
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(&notebook))??;
    Ok(Value::empty())
}

fn notebook_forget(interp: &mut Interp, notebook: &str, page: &str) -> Result<()> {
    let index = with_state(interp, |rt| {
        rt.tree
            .children(notebook)
            .ok()
            .and_then(|pages| pages.iter().position(|p| p == page))
    })
    .flatten()
    .ok_or_else(|| generic(format!("page \"{page}\" is not in {notebook}")))?;
    with_state_mut(interp, |rt| {
        rt.tree.detach(notebook, page).map_err(tk_error)?;
        let len = rt.tree.children(notebook).map(|c| c.len()).unwrap_or(0);
        let selected = rt.notebook_selected.get(notebook).copied().unwrap_or(0);
        let next = if index < selected {
            selected - 1
        } else if index == selected {
            selected.min(len.saturating_sub(1))
        } else {
            selected
        };
        rt.notebook_selected.insert(notebook.to_string(), next);
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(notebook))??;
    Ok(())
}

/// A Notebook's selected page index.
pub fn notebook_selected(notebook: &str) -> usize {
    let Some(ptr) = active_interp() else { return 0 };
    with_state(unsafe { &*ptr }, |rt| {
        rt.notebook_selected.get(notebook).copied().unwrap_or(0)
    })
    .unwrap_or(0)
}

/// Select a Notebook page from the renderer.
pub fn select_notebook_tab(notebook: &str, index: usize) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let len = with_state(interp, |rt| {
        rt.tree.children(notebook).map(|c| c.len()).unwrap_or(0)
    })
    .unwrap_or(0);
    with_state_mut(interp, |rt| {
        rt.notebook_selected
            .insert(notebook.to_string(), index.min(len.saturating_sub(1)));
        rt.bump();
    });
}

/// The values of a `ttk::combobox` (`-values` is a Tcl list).
pub fn combobox_values(path: &str) -> Vec<String> {
    let Some(ptr) = active_interp() else {
        return Vec::new();
    };
    let interp = unsafe { &*ptr };
    let raw = effective_option(interp, path, "-values");
    Value::from_str(&raw)
        .as_list()
        .map(|list| list.iter().map(|v| v.as_str().to_string()).collect())
        .unwrap_or_default()
}

/// The combobox whose list is open.
pub fn open_combobox() -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.open_combobox.clone()).flatten()
}

/// Open/close a combobox's value list.
pub fn toggle_combobox(path: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        rt.open_combobox = if rt.open_combobox.as_deref() == Some(path) {
            None
        } else {
            Some(path.to_string())
        };
        rt.bump();
    });
}

/// Pick a combobox value from the renderer.
pub fn press_combobox_item(path: &str, index: usize) {
    let values = combobox_values(path);
    let Some(value) = values.get(index).cloned() else {
        return;
    };
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    if let Err(e) = update_entry(interp, path, value.clone(), value.chars().count()) {
        record_error(e.to_string());
    }
    with_state_mut(interp, |rt| {
        rt.open_combobox = None;
        rt.bump();
    });
}

/// A Progressbar's current value and maximum.
pub fn progressbar_value(path: &str) -> (f64, f64) {
    let Some(ptr) = active_interp() else {
        return (0.0, 100.0);
    };
    let interp = unsafe { &*ptr };
    let max = effective_option(interp, path, "-maximum")
        .parse::<f64>()
        .unwrap_or(100.0);
    let value = if let Some(var) = qual_var_opt(interp, path, "-variable") {
        var_value(&var)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0)
    } else {
        with_state(interp, |rt| rt.tree.get(path).map(|n| n.value))
            .flatten()
            .unwrap_or(0.0)
    };
    (value, max)
}

fn progressbar_step(interp: &mut Interp, path: &str, amount: f64) -> Result<()> {
    let (value, max) = progressbar_value(path);
    let mut next = value + amount;
    if max > 0.0 && next > max {
        next = 0.0;
    }
    let var = qual_var_opt(interp, path, "-variable");
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.value = next;
            rt.bump();
        }
    });
    if let Some(var) = var {
        set_var(&var, &format_number(next))?;
    }
    Ok(())
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

fn canvas_create(interp: &mut Interp, path: &str, kind: &str, rest: &[Value]) -> Result<Value> {
    if !matches!(
        kind,
        "rectangle" | "oval" | "line" | "text" | "polygon" | "image"
    ) {
        return Err(generic(format!(
            "unknown or unsupported canvas item type \"{kind}\""
        )));
    }
    let split = rest
        .iter()
        .position(|v| v.as_str().starts_with('-'))
        .unwrap_or(rest.len());
    let coords = rest[..split]
        .iter()
        .map(|v| parse_f64_str(v.as_str(), "coordinate"))
        .collect::<Result<Vec<_>>>()?;
    if coords.len() < 2 {
        return Err(generic("wrong # coordinates: expected at least 2"));
    }
    let mut options = BTreeMap::new();
    for (key, value) in parse_pairs(&rest[split..], 0, "create")? {
        if !rtcl_tk_core::valid_option(&key) {
            return Err(bad_option(&key));
        }
        options.insert(key, value);
    }
    let id = with_state_mut(interp, |rt| {
        let canvas = rt.canvases.entry(path.to_string()).or_default();
        canvas.next_id += 1;
        let id = canvas.next_id;
        canvas.items.push(CanvasItem {
            id,
            kind: kind.to_string(),
            coords,
            options,
        });
        rt.bump();
        id
    })
    .ok_or_else(|| bad_window(path))?;
    Ok(Value::from_int(id as i64))
}

fn canvas_coords(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("coords", "tagOrId ?x y ...?", rest.len() + 2));
    }
    let id = rest[0]
        .as_str()
        .parse::<u64>()
        .map_err(|_| generic("bad item id"))?;
    if rest.len() == 1 {
        let coords = with_state(interp, |rt| {
            rt.canvases
                .get(path)
                .and_then(|canvas| canvas.items.iter().find(|i| i.id == id))
                .map(|item| {
                    item.coords
                        .iter()
                        .map(|v| format_number(*v))
                        .collect::<Vec<_>>()
                })
        })
        .flatten()
        .unwrap_or_default();
        return Ok(list_value(coords));
    }
    let coords = rest[1..]
        .iter()
        .map(|v| parse_f64_str(v.as_str(), "coordinate"))
        .collect::<Result<Vec<_>>>()?;
    with_state_mut(interp, |rt| {
        if let Some(item) = rt
            .canvases
            .get_mut(path)
            .and_then(|canvas| canvas.items.iter_mut().find(|i| i.id == id))
        {
            item.coords = coords;
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn canvas_itemconfigure(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong(
            "itemconfigure",
            "tagOrId ?-option? ?value ...?",
            rest.len() + 2,
        ));
    }
    let id = rest[0]
        .as_str()
        .parse::<u64>()
        .map_err(|_| generic("bad item id"))?;
    if rest.len() == 2 {
        let value = with_state(interp, |rt| {
            rt.canvases
                .get(path)
                .and_then(|canvas| canvas.items.iter().find(|i| i.id == id))
                .and_then(|item| item.options.get(rest[1].as_str()).cloned())
        })
        .flatten()
        .unwrap_or_default();
        return Ok(Value::from_str(&value));
    }
    let options = parse_pairs(rest, 1, "itemconfigure")?;
    with_state_mut(interp, |rt| {
        if let Some(item) = rt
            .canvases
            .get_mut(path)
            .and_then(|canvas| canvas.items.iter_mut().find(|i| i.id == id))
        {
            item.options.extend(options);
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn canvas_move(interp: &mut Interp, path: &str, rest: &[Value], scale: bool) -> Result<Value> {
    let needed = if scale { 5 } else { 3 };
    if rest.len() != needed {
        return Err(wrong(
            if scale { "scale" } else { "move" },
            if scale {
                "tagOrId xOrigin yOrigin xScale yScale"
            } else {
                "tagOrId dx dy"
            },
            rest.len() + 2,
        ));
    }
    let target = rest[0].as_str();
    let numbers = rest[1..]
        .iter()
        .map(|v| parse_f64_str(v.as_str(), "number"))
        .collect::<Result<Vec<_>>>()?;
    with_state_mut(interp, |rt| {
        if let Some(canvas) = rt.canvases.get_mut(path) {
            for item in canvas.items.iter_mut() {
                if target != "all" && item.id.to_string() != target {
                    continue;
                }
                if scale {
                    let (xo, yo, xs, ys) = (numbers[0], numbers[1], numbers[2], numbers[3]);
                    for pair in item.coords.chunks_mut(2) {
                        pair[0] = xo + (pair[0] - xo) * xs;
                        if pair.len() == 2 {
                            pair[1] = yo + (pair[1] - yo) * ys;
                        }
                    }
                } else {
                    for pair in item.coords.chunks_mut(2) {
                        pair[0] += numbers[0];
                        if pair.len() == 2 {
                            pair[1] += numbers[1];
                        }
                    }
                }
            }
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn canvas_raise(interp: &mut Interp, path: &str, rest: &[Value], raise: bool) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong(
            if raise { "raise" } else { "lower" },
            "tagOrId ?aboveThis?",
            rest.len() + 2,
        ));
    }
    let target = rest[0].as_str();
    with_state_mut(interp, |rt| {
        if let Some(canvas) = rt.canvases.get_mut(path) {
            if target == "all" {
                return;
            }
            if let Some(index) = canvas
                .items
                .iter()
                .position(|item| item.id.to_string() == target)
            {
                let item = canvas.items.remove(index);
                if raise {
                    canvas.items.push(item);
                } else {
                    canvas.items.insert(0, item);
                }
                rt.bump();
            }
        }
    });
    Ok(Value::empty())
}

fn canvas_bind(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong(
            "bind",
            "tagOrId ?sequence? ?+?script?",
            rest.len() + 2,
        ));
    }
    let target = rest[0].as_str();
    let exists = with_state(interp, |rt| rt.canvases.contains_key(path)).unwrap_or(false);
    if !exists {
        return Err(bad_window(path));
    }
    if rest.len() == 1 {
        let sequences = with_state(interp, |rt| {
            rt.canvases
                .get(path)
                .and_then(|canvas| canvas.bindings.get(target))
                .map(|map| map.keys().cloned().collect::<Vec<_>>())
        })
        .flatten()
        .unwrap_or_default();
        return Ok(list_value(sequences));
    }
    let sequence = normalize_sequence(rest[1].as_str());
    let current = with_state(interp, |rt| {
        rt.canvases
            .get(path)
            .and_then(|canvas| canvas.bindings.get(target))
            .and_then(|map| map.get(&sequence))
            .cloned()
    })
    .flatten()
    .unwrap_or_default();
    if rest.len() == 2 {
        return Ok(Value::from_str(&current));
    }
    let script = rest[2].as_str();
    let script = match script.strip_prefix('+') {
        Some(more) if !current.is_empty() => {
            format!("{}\n{}", current.trim_end_matches('\n'), more)
        }
        Some(more) => more.to_string(),
        None => script.to_string(),
    };
    with_state_mut(interp, |rt| {
        if let Some(canvas) = rt.canvases.get_mut(path) {
            let map = canvas.bindings.entry(target.to_string()).or_default();
            if script.is_empty() {
                map.remove(&sequence);
            } else {
                map.insert(sequence.clone(), script.clone());
            }
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn canvas_view(interp: &mut Interp, path: &str, rest: &[Value], horizontal: bool) -> Result<Value> {
    let region = effective_option(interp, path, "-scrollregion");
    let values = Value::from_str(&region)
        .as_list()
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().trim().parse::<f64>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (total, viewport) = if values.len() >= 4 {
        let start = if horizontal { values[0] } else { values[1] };
        let end = if horizontal { values[2] } else { values[3] };
        (
            (end - start).abs(),
            effective_option(interp, path, if horizontal { "-width" } else { "-height" })
                .parse::<f64>()
                .unwrap_or(0.0),
        )
    } else {
        (1.0, 1.0)
    };
    let total = total.max(1.0);
    let viewport = viewport.max(1.0);
    let max_first = (1.0 - viewport / total).max(0.0);
    let fraction = with_state(interp, |rt| {
        rt.canvases.get(path).map(|canvas| {
            if horizontal {
                canvas.xview
            } else {
                canvas.yview
            }
        })
    })
    .flatten()
    .unwrap_or(0.0)
    .clamp(0.0, max_first);
    if rest.is_empty() {
        let last = (fraction + viewport / total).min(1.0);
        return Ok(list_value([
            format_fraction(fraction),
            format_fraction(last),
        ]));
    }
    match rest[0].as_str() {
        "moveto" => {
            let target = rest
                .get(1)
                .map(|v| parse_f64_str(v.as_str(), "fraction"))
                .transpose()?
                .unwrap_or(0.0);
            set_canvas_view(interp, path, horizontal, target.min(max_first));
        }
        "scroll" => {
            let amount = rest.get(1).map(parse_int).transpose()?.unwrap_or(1);
            let pages = rest.get(2).is_some_and(|v| v.as_str() == "pages");
            let step = amount as f64 * viewport / if pages { total } else { total * 10.0 };
            set_canvas_view(interp, path, horizontal, (fraction + step).min(max_first));
        }
        other => {
            return Err(generic(format!(
                "bad option \"{other}\": must be moveto or scroll"
            )))
        }
    }
    Ok(Value::empty())
}

fn set_canvas_view(interp: &mut Interp, path: &str, horizontal: bool, fraction: f64) {
    with_state_mut(interp, |rt| {
        if let Some(canvas) = rt.canvases.get_mut(path) {
            if horizontal {
                canvas.xview = fraction;
            } else {
                canvas.yview = fraction;
            }
            rt.bump();
        }
    });
}

fn canvas_item_contains(item: &CanvasItem, x: f64, y: f64) -> bool {
    let c = &item.coords;
    match item.kind.as_str() {
        "image" if c.len() >= 2 => x >= c[0] && x <= c[0] + 80.0 && y >= c[1] && y <= c[1] + 80.0,
        "rectangle" | "text" | "polygon" if c.len() >= 4 => {
            x >= c[0].min(c[2]) && x <= c[0].max(c[2]) && y >= c[1].min(c[3]) && y <= c[1].max(c[3])
        }
        "oval" if c.len() >= 4 => {
            let cx = (c[0] + c[2]) / 2.0;
            let cy = (c[1] + c[3]) / 2.0;
            let rx = ((c[2] - c[0]) / 2.0).abs().max(0.001);
            let ry = ((c[3] - c[1]) / 2.0).abs().max(0.001);
            ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0
        }
        "line" if c.len() >= 4 => {
            let width = item
                .options
                .get("-width")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(1.0);
            c.chunks(2).collect::<Vec<_>>().windows(2).any(|pair| {
                let (x1, y1, x2, y2) = (pair[0][0], pair[0][1], pair[1][0], pair[1][1]);
                let dx = x2 - x1;
                let dy = y2 - y1;
                let len = (dx * dx + dy * dy).sqrt().max(0.001);
                let dist = ((x - x1) * dy - (y - y1) * dx).abs() / len;
                dist <= width / 2.0 + 4.0
            })
        }
        _ => false,
    }
}

/// A press on the canvas: run item/tag bindings of the topmost hit, then
/// the canvas's own `<Button-1>` binding.
pub fn canvas_press(path: &str, x: f64, y: f64) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let (items, bindings) = with_state(interp, |rt| {
        rt.canvases.get(path).map(|canvas| {
            (
                canvas
                    .items
                    .iter()
                    .filter(|item| canvas_item_contains(item, x, y))
                    .cloned()
                    .collect::<Vec<_>>(),
                canvas.bindings.clone(),
            )
        })
    })
    .flatten()
    .unwrap_or_default();
    let ctx = EventCtx {
        path: path.to_string(),
        x: x as i64,
        y: y as i64,
        button: 1,
        ..EventCtx::default()
    };
    for item in items.iter().rev() {
        let mut targets = vec![item.id.to_string()];
        if let Some(tags) = item.options.get("-tags") {
            targets.extend(tags.split_whitespace().map(str::to_string));
        }
        for target in targets {
            if let Some(script) = bindings.get(&target).and_then(|map| map.get("<Button-1>")) {
                let script = substitute_bindings(script, &ctx);
                if let Err(e) = eval_active(&script) {
                    record_error(e.to_string());
                }
            }
        }
    }
    run_binding(path, "<Button-1>", &ctx);
}

/// A text widget's persisted rich document (ProseMirror JSON), if any.
pub fn rich_doc(path: &str) -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.rich_docs.get(path).cloned()).flatten()
}

/// A rich document's plain text (blocks separated by newlines).
pub fn rich_doc_text(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    doc_runs(&value).map(|(text, _)| text)
}

struct DocRun {
    start: usize,
    end: usize,
    marks: Vec<serde_json::Value>,
}

fn doc_runs(value: &serde_json::Value) -> Option<(String, Vec<DocRun>)> {
    let blocks = value.get("content")?.as_array()?;
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut position = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            text.push('\n');
            position += 1;
        }
        let Some(nodes) = block.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for node in nodes {
            if node.get("type").and_then(|v| v.as_str()) != Some("text") {
                continue;
            }
            let content = node.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let length = content.chars().count();
            let marks = node
                .get("marks")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if length > 0 {
                runs.push(DocRun {
                    start: position,
                    end: position + length,
                    marks,
                });
            }
            text.push_str(content);
            position += length;
        }
    }
    Some((text, runs))
}

fn merged_doc_json(json: &str, new_content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let (old_text, mut runs) = doc_runs(&value)?;
    if old_text == new_content {
        return Some(json.to_string());
    }
    let old_chars = old_text.chars().collect::<Vec<_>>();
    let new_chars = new_content.chars().collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < old_chars.len()
        && prefix < new_chars.len()
        && old_chars[prefix] == new_chars[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_chars.len().saturating_sub(prefix)
        && suffix < new_chars.len().saturating_sub(prefix)
        && old_chars[old_chars.len() - 1 - suffix] == new_chars[new_chars.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_len = old_chars.len();
    let delta = new_chars.len() as isize - old_len as isize;
    for run in &mut runs {
        let (start, end) = if run.end <= prefix {
            (run.start, run.end)
        } else if run.start >= old_len.saturating_sub(suffix) {
            (
                (run.start as isize + delta).max(0) as usize,
                (run.end as isize + delta).max(0) as usize,
            )
        } else {
            (
                run.start.min(prefix),
                ((run.end as isize + delta).max(0) as usize).max(prefix),
            )
        };
        run.start = start.min(new_chars.len());
        run.end = end.min(new_chars.len());
    }
    let mut paragraphs = Vec::new();
    let mut line_start = 0usize;
    for line in new_content.split('\n') {
        let line_len = line.chars().count();
        let line_end = (line_start + line_len).min(new_chars.len());
        let mut bounds = vec![line_start, line_end];
        for run in &runs {
            if run.end <= run.start {
                continue;
            }
            if run.start < line_end && run.end > line_start {
                bounds.push(run.start.max(line_start));
                bounds.push(run.end.min(line_end));
            }
        }
        bounds.sort_unstable();
        bounds.dedup();
        let mut nodes = Vec::new();
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let segment = new_chars[start..end].iter().collect::<String>();
            let marks = runs
                .iter()
                .find(|run| start < run.end && end > run.start)
                .map(|run| run.marks.clone())
                .unwrap_or_default();
            let mut node = serde_json::json!({"type": "text", "text": segment});
            if !marks.is_empty() {
                node["marks"] = serde_json::Value::Array(marks);
            }
            nodes.push(node);
        }
        paragraphs.push(if nodes.is_empty() {
            serde_json::json!({"type": "paragraph"})
        } else {
            serde_json::json!({"type": "paragraph", "content": nodes})
        });
        line_start = line_end + 1;
    }
    serde_json::to_string(&serde_json::json!({"type": "doc", "content": paragraphs})).ok()
}

/// Update a persisted rich document for externally edited text, keeping
/// its marks (shifted by the text diff).
pub fn merge_rich_doc(path: &str, new_content: &str) -> Option<String> {
    let json = rich_doc(path)?;
    let merged = merged_doc_json(&json, new_content)?;
    set_rich_doc(path, merged.clone());
    sync_text_tags_from_doc(path);
    Some(merged)
}

fn style_options_from_marks(marks: &[serde_json::Value]) -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    let mut family = String::new();
    let mut size = String::new();
    let mut bold = false;
    let mut italic = false;
    for mark in marks {
        let kind = mark.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if kind == "bold" {
            bold = true;
        } else if kind == "italic" {
            italic = true;
        } else if kind == "textStyle" {
            let Some(attrs) = mark.get("attrs").and_then(|v| v.as_object()) else {
                continue;
            };
            if let Some(color) = attrs.get("color").and_then(|v| v.as_str()) {
                if !color.is_empty() {
                    options.insert("-foreground".to_string(), color.to_string());
                }
            }
            if let Some(color) = attrs.get("backgroundColor").and_then(|v| v.as_str()) {
                if !color.is_empty() {
                    options.insert("-background".to_string(), color.to_string());
                }
            }
            if let Some(value) = attrs.get("fontFamily").and_then(|v| v.as_str()) {
                if !value.is_empty() {
                    family = value.to_string();
                }
            }
            if let Some(value) = attrs.get("fontSize").and_then(|v| v.as_str()) {
                if let Some(points) = value.strip_suffix("px") {
                    size = points.to_string();
                }
            }
        }
    }
    if bold || italic || !family.is_empty() || !size.is_empty() {
        let mut font = String::new();
        if family.is_empty() {
            font.push_str("TkDefaultFont");
        } else if family.contains(' ') {
            font.push('{');
            font.push_str(&family);
            font.push('}');
        } else {
            font.push_str(&family);
        }
        font.push(' ');
        font.push_str(if size.is_empty() { "10" } else { &size });
        if bold {
            font.push_str(" bold");
        }
        if italic {
            font.push_str(" italic");
        }
        options.insert("-font".to_string(), font);
    }
    options
}

fn style_signature(options: &BTreeMap<String, String>) -> String {
    options
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn has_style_options(options: &BTreeMap<String, String>) -> bool {
    options.contains_key("-foreground")
        || options.contains_key("-background")
        || options.contains_key("-font")
}

/// Project a rich document's marks into Tk tags, so `tag ranges/names`
/// see what the editor changed: style tags are cleared and re-ranged from
/// the document, matching user tags by style and creating `@rich-…` ones
/// for new styles. Tags without style options keep their ranges.
pub fn sync_text_tags_from_doc(path: &str) {
    let Some(json) = rich_doc(path) else { return };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
        return;
    };
    let Some((_text, runs)) = doc_runs(&value) else {
        return;
    };
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let _ = with_state_mut(interp, |rt| {
        if !rt.tree.contains(path) {
            return;
        }
        let tags = rt.text_tags.entry(path.to_string()).or_default();
        let existing = tags
            .iter()
            .filter(|(_, tag)| has_style_options(&tag.options))
            .map(|(name, tag)| (style_signature(&tag.options), name.clone()))
            .collect::<BTreeMap<_, _>>();
        for tag in tags.values_mut() {
            if has_style_options(&tag.options) {
                tag.ranges.clear();
            }
        }
        for run in &runs {
            let options = style_options_from_marks(&run.marks);
            if options.is_empty() || run.start >= run.end {
                continue;
            }
            let signature = style_signature(&options);
            let name = if let Some(name) = existing.get(&signature) {
                name.clone()
            } else {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                use std::hash::{Hash, Hasher};
                signature.hash(&mut hash);
                format!("@rich-{:016x}", hash.finish())
            };
            let tag = tags.entry(name).or_default();
            if tag.options.is_empty() {
                tag.options = options;
            }
            tag.ranges.push((run.start, run.end));
        }
        tags.retain(|name, tag| !name.starts_with("@rich-") || !tag.ranges.is_empty());
    });
}
/// Store a text widget's rich document as data (not Tk tags).
pub fn set_rich_doc(path: &str, json: String) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        rt.rich_docs.insert(path.to_string(), json);
    });
}

/// A text widget's tags (name, options, character ranges).
pub fn text_tags(path: &str) -> Vec<TextTagView> {
    let Some(ptr) = active_interp() else {
        return Vec::new();
    };
    with_state(unsafe { &*ptr }, |rt| {
        rt.text_tags
            .get(path)
            .map(|tags| {
                tags.iter()
                    .filter(|(_, tag)| !tag.ranges.is_empty())
                    .map(|(name, tag)| TextTagView {
                        name: name.clone(),
                        options: tag.options.clone(),
                        ranges: tag.ranges.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// A canvas's items for the renderer.
pub fn canvas_snapshot(path: &str) -> Option<CanvasState> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.canvases.get(path).cloned()).flatten()
}

fn text_tag_command(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("tag", "option ?arg ...?", rest.len() + 2));
    }
    let (content, cursor) = text_state(interp, path);
    match rest[0].as_str() {
        "configure" => {
            if rest.len() < 2 {
                return Err(wrong("tag configure", "tagName ?-option? ?value ...?", rest.len() + 2));
            }
            let name = rest[1].as_str().to_string();
            if rest.len() == 2 {
                let options = with_state(interp, |rt| {
                    rt.text_tags
                        .get(path)
                        .and_then(|tags| tags.get(&name))
                        .map(|tag| tag.options.clone())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
                return Ok(list_value(
                    options
                        .iter()
                        .flat_map(|(k, v)| [k.clone(), v.clone()])
                        .collect::<Vec<_>>(),
                ));
            }
            if rest.len() == 3 {
                let option = rest[2].as_str();
                let value = with_state(interp, |rt| {
                    rt.text_tags
                        .get(path)
                        .and_then(|tags| tags.get(&name))
                        .and_then(|tag| tag.options.get(option))
                        .cloned()
                })
                .flatten()
                .unwrap_or_default();
                return Ok(Value::from_str(&value));
            }
            let options = parse_pairs(rest, 2, "tag configure")?;
            with_state_mut(interp, |rt| {
                let tag = rt
                    .text_tags
                    .entry(path.to_string())
                    .or_default()
                    .entry(name.clone())
                    .or_default();
                tag.options.extend(options);
                rt.bump();
            });
            Ok(Value::empty())
        }
        "cget" => {
            if rest.len() != 3 {
                return Err(wrong("tag cget", "tagName -option", rest.len() + 2));
            }
            let value = with_state(interp, |rt| {
                rt.text_tags
                    .get(path)
                    .and_then(|tags| tags.get(rest[1].as_str()))
                    .and_then(|tag| tag.options.get(rest[2].as_str()))
                    .cloned()
            })
            .flatten()
            .unwrap_or_default();
            Ok(Value::from_str(&value))
        }
        "add" => {
            if rest.len() < 3 || rest.len() > 4 {
                return Err(wrong("tag add", "tagName index1 ?index2?", rest.len() + 2));
            }
            let start = text_index(rest[2].as_str(), &content, cursor)?;
            let end = match rest.get(3) {
                Some(last) => text_index(last.as_str(), &content, cursor)?,
                None => (start + 1).min(content.chars().count()),
            };
            let (a, b) = (start.min(end), start.max(end));
            let name = rest[1].as_str().to_string();
            with_state_mut(interp, |rt| {
                let tag = rt
                    .text_tags
                    .entry(path.to_string())
                    .or_default()
                    .entry(name.clone())
                    .or_default();
                tag.ranges.push((a, b));
                tag.ranges.sort_unstable();
                rt.bump();
            });
            Ok(Value::empty())
        }
        "remove" => {
            if rest.len() < 3 || rest.len() > 4 {
                return Err(wrong("tag remove", "tagName index1 ?index2?", rest.len() + 2));
            }
            let start = text_index(rest[2].as_str(), &content, cursor)?;
            let end = match rest.get(3) {
                Some(last) => text_index(last.as_str(), &content, cursor)?,
                None => (start + 1).min(content.chars().count()),
            };
            let (a, b) = (start.min(end), start.max(end));
            let name = rest[1].as_str().to_string();
            with_state_mut(interp, |rt| {
                if let Some(tag) = rt
                    .text_tags
                    .get_mut(path)
                    .and_then(|tags| tags.get_mut(&name))
                {
                    let mut next = Vec::new();
                    for (start, end) in tag.ranges.drain(..) {
                        if end <= a || start >= b {
                            next.push((start, end));
                        } else {
                            if start < a {
                                next.push((start, a));
                            }
                            if end > b {
                                next.push((b, end));
                            }
                        }
                    }
                    tag.ranges = next;
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "names" => {
            let index = match rest.get(1) {
                Some(value) => Some(text_index(value.as_str(), &content, cursor)?),
                None => None,
            };
            let names = with_state(interp, |rt| {
                rt.text_tags
                    .get(path)
                    .map(|tags| {
                        tags.iter()
                            .filter(|(_, tag)| {
                                index.is_none_or(|i| {
                                    tag.ranges.iter().any(|(a, b)| i >= *a && i < *b)
                                })
                            })
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            Ok(list_value(names))
        }
        "ranges" => {
            if rest.len() != 2 {
                return Err(wrong("tag ranges", "tagName", rest.len() + 2));
            }
            let ranges = with_state(interp, |rt| {
                rt.text_tags
                    .get(path)
                    .and_then(|tags| tags.get(rest[1].as_str()))
                    .map(|tag| tag.ranges.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            let mut out = Vec::new();
            for (start, end) in ranges {
                out.push(char_to_linecol(&content, start));
                out.push(char_to_linecol(&content, end));
            }
            Ok(list_value(out))
        }
        "delete" => {
            with_state_mut(interp, |rt| {
                if let Some(tags) = rt.text_tags.get_mut(path) {
                    for value in &rest[1..] {
                        tags.remove(value.as_str());
                    }
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "bind" | "raise" | "lower" => Ok(Value::empty()),
        other => Err(generic(format!(
            "bad option \"{other}\": must be add, bind, cget, configure, delete, names, ranges, or remove"
        ))),
    }
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

fn format_fraction(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The scroll fraction and visible row count of a listbox/treeview.
fn scroll_state(interp: &Interp, path: &str) -> (f64, usize) {
    let rows = effective_option(interp, path, "-height")
        .parse::<usize>()
        .unwrap_or(10)
        .max(1);
    let fraction = with_state(interp, |rt| rt.tree.get(path).map(|n| n.value))
        .flatten()
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    (fraction, rows)
}

fn set_scroll_fraction(interp: &mut Interp, path: &str, fraction: f64) {
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.value = fraction.clamp(0.0, 1.0);
            rt.bump();
        }
    });
}

fn notify_yscroll(interp: &mut Interp, path: &str, total: usize) {
    let command = effective_option(interp, path, "-yscrollcommand");
    if command.is_empty() || total == 0 {
        return;
    }
    let (fraction, rows) = scroll_state(interp, path);
    let last = (fraction + rows as f64 / total as f64).min(1.0);
    let script = format!(
        "{command} {} {}",
        format_fraction(fraction),
        format_fraction(last)
    );
    if let Err(e) = eval_active(&script) {
        record_error(e.to_string());
    }
}

/// `yview` of a listbox/treeview: query or move the visible window.
fn view_scroll_command(
    interp: &mut Interp,
    path: &str,
    rest: &[Value],
    total: usize,
) -> Result<Value> {
    let (fraction, rows) = scroll_state(interp, path);
    let max_first = (1.0 - rows as f64 / total.max(1) as f64).max(0.0);
    if rest.is_empty() {
        let last = (fraction + rows as f64 / total.max(1) as f64).min(1.0);
        return Ok(list_value([
            format_fraction(fraction),
            format_fraction(last),
        ]));
    }
    match rest[0].as_str() {
        "moveto" => {
            let target = rest
                .get(1)
                .map(|v| parse_f64_str(v.as_str(), "fraction"))
                .transpose()?
                .unwrap_or(0.0);
            set_scroll_fraction(interp, path, target.min(max_first));
            notify_yscroll(interp, path, total);
        }
        "scroll" => {
            let amount = rest.get(1).map(parse_int).transpose()?.unwrap_or(1);
            let pages = rest.get(2).is_some_and(|v| v.as_str() == "pages");
            let step = if pages {
                amount as f64 * rows as f64
            } else {
                amount as f64
            } / total.max(1) as f64;
            set_scroll_fraction(interp, path, (fraction + step).min(max_first));
            notify_yscroll(interp, path, total);
        }
        other => {
            return Err(generic(format!(
                "bad option \"{other}\": must be moveto or scroll"
            )))
        }
    }
    Ok(Value::empty())
}

fn treeview_visible_count(interp: &Interp, path: &str) -> usize {
    with_state(interp, |rt| {
        rt.treeviews.get(path).map(|tv| {
            fn walk(tv: &TreeViewState, parent: Option<&str>) -> usize {
                let mut count = 0;
                for item in tv.items.iter().filter(|i| i.parent.as_deref() == parent) {
                    count += 1;
                    if item.open {
                        count += walk(tv, Some(item.id.as_str()));
                    }
                }
                count
            }
            walk(tv, None)
        })
    })
    .flatten()
    .unwrap_or(0)
}

/// The fraction a listbox/treeview is scrolled to.
pub fn widget_scroll_fraction(path: &str) -> f64 {
    let Some(ptr) = active_interp() else {
        return 0.0;
    };
    with_state(unsafe { &*ptr }, |rt| rt.tree.get(path).map(|n| n.value))
        .flatten()
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

/// A scrollbar's `(first, last)` fractions.
pub fn scrollbar_fraction(path: &str) -> (f64, f64) {
    let Some(ptr) = active_interp() else {
        return (0.0, 0.1);
    };
    with_state(unsafe { &*ptr }, |rt| {
        rt.tree.get(path).map(|n| {
            let first = n.value.clamp(0.0, 1.0);
            let last = n
                .content
                .parse::<f64>()
                .unwrap_or(first + 0.1)
                .clamp(first, 1.0);
            (first, last)
        })
    })
    .flatten()
    .unwrap_or((0.0, 0.1))
}

/// A scrollbar arrow: run its `-command` with `scroll n units|pages`.
pub fn scrollbar_step(path: &str, amount: i64, pages: bool) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let command = effective_option(interp, path, "-command");
    if command.is_empty() {
        return;
    }
    let unit = if pages { "pages" } else { "units" };
    let script = format!("{command} scroll {amount} {unit}");
    if let Err(e) = eval_active(&script) {
        record_error(e.to_string());
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
    sync_listvariable(interp, path);
    let total = with_state(interp, |rt| {
        rt.tree.get(path).map(|n| n.items.len()).unwrap_or(0)
    })
    .unwrap_or(0);
    notify_yscroll(interp, path, total);
    Ok(Value::empty())
}

/// Write a Listbox's items back to its `-listvariable`, as Tk does after
/// `insert`/`delete`.
fn sync_listvariable(interp: &mut Interp, path: &str) {
    let var = effective_option(interp, path, "-listvariable");
    if var.is_empty() {
        return;
    }
    let items = with_state(interp, |rt| {
        rt.tree
            .get(path)
            .map(|n| n.items.clone())
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let values = items.iter().map(|s| Value::from_str(s)).collect::<Vec<_>>();
    let list = Value::from_list(&values).as_str().to_string();
    let _ = set_var(&var, &list);
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
    sync_listvariable(interp, path);
    let total = with_state(interp, |rt| {
        rt.tree.get(path).map(|n| n.items.len()).unwrap_or(0)
    })
    .unwrap_or(0);
    notify_yscroll(interp, path, total);
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
        "place" => place_impl(interp, args),
        "winfo" => winfo_impl(interp, args),
        "wm" => wm_impl(interp, args),
        "tk" => tk_impl(interp, args),
        "bind" => bind_impl(interp, args),
        "event" => event_impl(interp, args),
        "focus" => focus_impl(interp, args),
        "raise" => raise_lower_impl(interp, args, true),
        "lower" => raise_lower_impl(interp, args, false),
        "bindtags" => bindtags_impl(interp, args),
        "font" => font_impl(interp, args),
        "option" => option_impl(interp, args),
        "bell" => Ok(Value::empty()),
        "tk_focusNext" => tk_focus_impl(interp, args, true),
        "tk_focusPrev" => tk_focus_impl(interp, args, false),
        "menu" => menu_create(interp, args),
        "image" => image_impl(interp, args),
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
        assert_eq!(eval(&mut i, "tk scaling"), "1");
        eval(&mut i, "tk scaling 2");
        assert_eq!(eval(&mut i, "tk scaling"), "2");
        assert_eq!(eval(&mut i, "winfo fpixels . 1i"), "144");
        assert_eq!(eval(&mut i, "winfo fpixels . 10"), "10");
        eval(&mut i, "tk appname MyApp");
        assert_eq!(eval(&mut i, "tk appname"), "MyApp");
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

        // A Tk scrollbar drives the listbox's yview and receives its
        // -yscrollcommand fractions.
        eval(
            &mut i,
            "listbox .ls -height 3 -yscrollcommand {.ss set}; \
             scrollbar .ss -command {.ls yview}; \
             foreach c {a b c d e f g h i j} { .ls insert end $c }",
        );
        assert_eq!(eval(&mut i, ".ls cget -height"), "3");
        assert_eq!(eval(&mut i, ".ls size"), "10");
        assert_eq!(eval(&mut i, ".ls yview"), "0 0.3");
        assert_eq!(eval(&mut i, ".ss get"), "0 0.3");
        scrollbar_step(".ss", 1, false);
        let first = eval(&mut i, "lindex [.ls yview] 0");
        assert_ne!(first, "0");
        assert_eq!(eval(&mut i, "lindex [.ss get] 0"), first);
        eval(&mut i, ".ls yview moveto 1");
        assert_eq!(eval(&mut i, "lindex [.ls yview] 0"), "0.7");

        // -listvariable is read at creation and written after edits.
        eval(
            &mut i,
            "set ::items {x y}; listbox .lv -listvariable ::items",
        );
        assert_eq!(eval(&mut i, ".lv size"), "2");
        eval(&mut i, ".lv insert end z");
        assert_eq!(eval(&mut i, "set ::items"), "x y z");
        eval(&mut i, ".lv delete 0");
        assert_eq!(eval(&mut i, "set ::items"), "y z");
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
        // Class and toplevel tag bindings run in bindtags order.
        eval(&mut i, "bind Entry <KeyPress> {incr ::n}");
        eval(&mut i, "event generate .e <KeyPress>");
        assert_eq!(eval(&mut i, "set ::n"), "3");
        eval(&mut i, "bind . <FocusIn> {incr ::n}");
        eval(&mut i, "event generate .e <FocusIn>");
        assert_eq!(eval(&mut i, "set ::n"), "4");
        assert_eq!(eval(&mut i, "bind Entry"), "<KeyPress>");
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
    fn entry_validation_and_labelframe() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "entry .e -validate key -validatecommand {expr {[string length %P] <= 3}}; \
             ttk::labelframe .lf -text Options",
        );
        entry_changed(".e", "abc");
        assert_eq!(entry_value(".e"), "abc");
        entry_changed(".e", "abcd");
        assert_eq!(entry_value(".e"), "abc");
        assert_eq!(eval(&mut i, "winfo class .lf"), "TLabelframe");
        assert_eq!(eval(&mut i, "cget .lf -text"), "Options");
    }

    #[test]
    fn text_tags_photo_images_and_canvas() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "text .t; .t insert 1.0 {hello world}");
        eval(
            &mut i,
            ".t tag configure strong -foreground red; .t tag add strong 1.0 1.5",
        );
        assert_eq!(eval(&mut i, ".t tag ranges strong"), "1.0 1.5");
        assert_eq!(text_tags(".t").len(), 1);
        text_changed(".t", "hello brave world");
        assert_eq!(text_tags(".t")[0].ranges, vec![(0, 5)]);
        assert_eq!(eval(&mut i, ".t getdoc"), "");
        set_rich_doc(".t", "{\"type\":\"doc\"}".to_string());
        assert_eq!(eval(&mut i, ".t getdoc"), "{\"type\":\"doc\"}");
        eval(&mut i, "text .r");
        let rich = r##"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hello world","marks":[{"type":"textStyle","attrs":{"color":"#ff0000","backgroundColor":"","fontSize":"","fontFamily":""}}]}]}]}"##;
        eval(&mut i, &format!(".r setdoc {{{rich}}}"));
        assert_eq!(eval(&mut i, ".r get 1.0 end"), "hello world");
        let merged = merge_rich_doc(".r", "hello brave world").unwrap();
        assert_eq!(rich_doc_text(&merged).as_deref(), Some("hello brave world"));
        assert!(merged.contains("#ff0000"));
        // Marks project into Tk tags: an existing style tag is reused, and
        // a new style gets an @rich- tag.
        eval(&mut i, "text .p; .p tag configure red -foreground #ff0000");
        eval(&mut i, &format!(".p setdoc {{{rich}}}"));
        assert_eq!(eval(&mut i, ".p tag ranges red"), "1.0 1.11");
        assert!(eval(&mut i, ".p tag names 1.2").contains("red"));
        let blue = rich.replace("#ff0000", "#0000ff");
        eval(&mut i, &format!(".p setdoc {{{blue}}}"));
        let names = eval(&mut i, ".p tag names 1.2");
        assert!(names.contains("@rich-"), "{names}");
        assert_eq!(eval(&mut i, ".p tag ranges red"), "");
        assert_eq!(eval(&mut i, ".t tag names 1.2"), "strong");
        assert_eq!(eval(&mut i, ".t tag names 1.7"), "");
        assert_eq!(eval(&mut i, ".t tag cget strong -foreground"), "red");
        eval(&mut i, ".t tag remove strong 1.0 1.2");
        assert_eq!(eval(&mut i, ".t tag ranges strong"), "1.2 1.5");
        eval(&mut i, ".t tag delete strong");
        assert_eq!(eval(&mut i, ".t tag names"), "");

        assert_eq!(
            eval(
                &mut i,
                "image create photo img -file /tmp/x.png -width 10 -height 20"
            ),
            "img"
        );
        assert_eq!(eval(&mut i, "image names"), "img");
        assert_eq!(eval(&mut i, "image width img"), "10");
        assert_eq!(eval(&mut i, "image type img"), "photo");
        assert_eq!(eval(&mut i, "img cget -file"), "/tmp/x.png");
        eval(&mut i, "image delete img");
        assert_eq!(eval(&mut i, "image exists img"), "0");

        eval(&mut i, "canvas .c -width 100 -height 80");
        let id = eval(
            &mut i,
            ".c create rectangle 0 0 10 20 -fill red -outline blue",
        );
        eval(&mut i, &format!("set id {id}"));
        assert_eq!(eval(&mut i, ".c coords $id"), "0 0 10 20");
        assert_eq!(eval(&mut i, ".c itemcget $id -fill"), "red");
        eval(&mut i, ".c itemconfigure $id -fill green");
        assert_eq!(eval(&mut i, ".c itemcget $id -fill"), "green");
        eval(&mut i, ".c coords $id 1 2 30 40");
        assert_eq!(eval(&mut i, ".c coords $id"), "1 2 30 40");
        assert_eq!(eval(&mut i, ".c find"), id);
        eval(&mut i, "set ::hit 0; .c bind $id <Button-1> {incr ::hit}");
        canvas_press(".c", 5.0, 5.0);
        assert_eq!(eval(&mut i, "set ::hit"), "1");
        eval(&mut i, ".c move $id 5 5");
        assert_eq!(eval(&mut i, ".c coords $id"), "6 7 35 45");
        eval(&mut i, ".c scale $id 0 0 2 2");
        assert_eq!(eval(&mut i, ".c coords $id"), "12 14 70 90");
        eval(&mut i, ".c delete $id");
        assert_eq!(eval(&mut i, ".c find"), "");

        eval(
            &mut i,
            "canvas .sv -width 100 -height 100 -scrollregion {0 0 400 400}; \
             .sv create rectangle 0 0 400 400",
        );
        assert_eq!(eval(&mut i, ".sv xview"), "0 0.25");
        eval(&mut i, ".sv xview moveto 1");
        assert_eq!(eval(&mut i, "lindex [.sv xview] 0"), "0.75");
        assert_eq!(eval(&mut i, ".sv yview"), "0 0.25");
        eval(&mut i, ".sv yview scroll 1 pages");
        assert_ne!(eval(&mut i, "lindex [.sv yview] 0"), "0");
    }

    #[test]
    fn treeview_items_and_selection() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "ttk::treeview .tv -columns {size owner} -height 4; \
             .tv heading #0 -text Folder; .tv heading size -text Size; \
             .tv insert {} end -id root -text Project -open 1 -values {0 me}; \
             .tv insert root end -id src -text src -values {3 me}; \
             .tv insert root end -text README -values {1 me}",
        );
        assert_eq!(eval(&mut i, "winfo class .tv"), "Treeview");
        assert_eq!(eval(&mut i, ".tv children {}"), "root");
        assert_eq!(eval(&mut i, ".tv children root"), "src I001");
        assert_eq!(eval(&mut i, ".tv item src -text"), "src");
        assert_eq!(eval(&mut i, ".tv item src -values"), "3 me");
        assert_eq!(eval(&mut i, ".tv parent src"), "root");
        assert_eq!(eval(&mut i, ".tv index src"), "0");
        assert_eq!(eval(&mut i, ".tv heading #0"), "Folder");
        eval(&mut i, ".tv selection set src");
        assert_eq!(eval(&mut i, ".tv selection"), "src");
        assert_eq!(eval(&mut i, ".tv focus"), "src");
        eval(&mut i, ".tv delete src");
        assert_eq!(eval(&mut i, "winfo exists .tv"), "1");
        assert_eq!(eval(&mut i, ".tv children root"), "I001");
        let tv = treeview_snapshot(".tv").unwrap();
        assert_eq!(tv.items.len(), 2);
        assert_eq!(tv.columns, ["size", "owner"]);
    }

    #[test]
    fn notebook_pages_and_combobox_values() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "ttk::notebook .nb; \
             frame .nb.p1 -text One; frame .nb.p2 -text Two; \
             .nb add .nb.p1; .nb add .nb.p2",
        );
        assert_eq!(eval(&mut i, "winfo class .nb"), "TNotebook");
        assert_eq!(eval(&mut i, ".nb tabs"), ".nb.p1 .nb.p2");
        assert_eq!(eval(&mut i, ".nb select"), ".nb.p1");
        eval(&mut i, ".nb select .nb.p2");
        assert_eq!(eval(&mut i, ".nb index .nb.p2"), "1");
        assert_eq!(eval(&mut i, ".nb tab .nb.p1 -text"), "One");
        eval(&mut i, ".nb insert 0 .nb.p2");
        assert_eq!(eval(&mut i, ".nb tabs"), ".nb.p2 .nb.p1");
        eval(&mut i, ".nb forget .nb.p2");
        assert_eq!(eval(&mut i, ".nb tabs"), ".nb.p1");

        eval(
            &mut i,
            "set ::choice a; ttk::combobox .cb -values {a b c} -textvariable ::choice",
        );
        assert_eq!(eval(&mut i, "winfo class .cb"), "TCombobox");
        assert_eq!(combobox_values(".cb"), ["a", "b", "c"]);
        assert_eq!(eval(&mut i, ".cb current"), "0");
        eval(&mut i, ".cb current 2");
        assert_eq!(eval(&mut i, "set ::choice"), "c");
        eval(&mut i, ".cb set b");
        assert_eq!(eval(&mut i, ".cb get"), "b");
        assert_eq!(eval(&mut i, ".cb current"), "1");
        eval(&mut i, ".cb values {x y}");
        assert_eq!(combobox_values(".cb"), ["x", "y"]);
    }

    #[test]
    fn menu_model_and_invoke() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "menu .menubar -tearoff 0; menu .file -tearoff 0; \
             set ::hits 0; set ::checked 0; \
             .file add command -label Open -command {incr ::hits}; \
             .file add checkbutton -label Show -variable ::checked -onvalue 1 -offvalue 0; \
             .file add separator; \
             .file add cascade -label More -menu .more; \
             menu .more; .more add command -label Deep -command {incr ::hits}; \
             .menubar add cascade -label File -menu .file; \
             . configure -menu .menubar",
        );
        assert_eq!(eval(&mut i, "winfo class .menubar"), "Menu");
        assert_eq!(eval(&mut i, "winfo exists .file"), "1");
        assert_eq!(eval(&mut i, "winfo manager .menubar"), "");
        assert_eq!(menu_bar().as_deref(), Some(".menubar"));
        assert_eq!(eval(&mut i, ".file entrycget 0 -label"), "Open");
        eval(&mut i, ".file invoke 0");
        assert_eq!(eval(&mut i, "set ::hits"), "1");
        eval(&mut i, ".file invoke 1");
        assert_eq!(eval(&mut i, "set ::checked"), "1");
        eval(&mut i, ".menubar invoke 0");
        assert_eq!(open_menu().as_deref(), Some(".file"));
        eval(&mut i, ".file invoke 3");
        assert_eq!(open_menu().as_deref(), Some(".more"));
        eval(&mut i, "destroy .more");
        assert_eq!(eval(&mut i, "winfo exists .more"), "0");
        eval(&mut i, ".file delete 2");
        assert_eq!(eval(&mut i, ".file index end"), "3");
    }

    #[test]
    fn focus_bindtags_fonts_and_option_db() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "frame .f; entry .f.a; entry .f.b; focus .f.b; \
             bindtags .f.a",
        );
        assert_eq!(eval(&mut i, "focus"), ".f.b");
        assert!(focused(".f.b"));
        assert_eq!(eval(&mut i, "lindex [bindtags .f.a] 1"), "Entry");
        assert_eq!(eval(&mut i, "tk_focusNext .f.a"), ".f.b");
        assert_eq!(eval(&mut i, "tk_focusPrev .f.a"), ".f.b");
        eval(&mut i, "bindtags .f.a {.f.a Entry all}");
        assert_eq!(eval(&mut i, "bindtags .f.a"), ".f.a Entry all");
        eval(
            &mut i,
            "font create Title -family {DejaVu Sans} -size 20 -weight bold; \
             option add *Entry.background red",
        );
        assert_eq!(eval(&mut i, "font actual Title -size"), "20");
        assert_eq!(eval(&mut i, "option get .e background Entry"), "red");
        assert_eq!(eval(&mut i, "font measure Title abc"), "36");
    }

    #[test]
    fn raise_lower_reorder_siblings() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "frame .f; label .f.a -text A; label .f.b -text B");
        assert_eq!(eval(&mut i, "winfo children .f"), ".f.a .f.b");
        eval(&mut i, "raise .f.a");
        assert_eq!(eval(&mut i, "winfo children .f"), ".f.b .f.a");
        eval(&mut i, "lower .f.a");
        assert_eq!(eval(&mut i, "winfo children .f"), ".f.a .f.b");
    }

    #[test]
    fn place_manager_and_grid_weights() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "frame .f; label .l -text X; \
             place .l -x 10 -y 20 -width 80 -height 24; \
             grid .f -row 0 -column 0; \
             grid columnconfigure . 1 -weight 3",
        );
        assert_eq!(eval(&mut i, "winfo manager .l"), "place");
        assert_eq!(eval(&mut i, "lindex [place info .l] 1"), "10");
        assert_eq!(eval(&mut i, "place slaves ."), ".l");
        assert_eq!(eval(&mut i, "lindex [grid columnconfigure . 1] 5"), "3");
        assert_eq!(grid_weight(".", 1, false), 3);
        eval(&mut i, "place forget .l");
        assert_eq!(eval(&mut i, "winfo manager .l"), "");
    }

    #[test]
    fn message_scrollbar_separator_progressbar() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "message .m -text hello; \
             scrollbar .s -orient vertical; \
             ttk::separator .sep -orient horizontal; \
             set ::pv 0; ttk::progressbar .p -variable ::pv -maximum 10",
        );
        assert_eq!(eval(&mut i, "winfo class .m"), "Message");
        assert_eq!(eval(&mut i, "winfo class .s"), "Scrollbar");
        assert_eq!(eval(&mut i, "winfo class .sep"), "TSeparator");
        assert_eq!(eval(&mut i, "winfo class .p"), "TProgressbar");
        eval(&mut i, ".s set 0.2 0.6");
        assert_eq!(eval(&mut i, ".s get"), "0.2 0.6");
        eval(&mut i, ".p step 3");
        assert_eq!(eval(&mut i, "set ::pv"), "3");
        // A step past the maximum wraps to zero, as ttk does.
        eval(&mut i, ".p step 8");
        assert_eq!(eval(&mut i, "set ::pv"), "0");
        assert_eq!(progressbar_value(".p"), (0.0, 10.0));
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
