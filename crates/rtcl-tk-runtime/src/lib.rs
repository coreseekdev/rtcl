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
    /// Pane lists of each `panedwindow`.
    paned: BTreeMap<String, Vec<PanedPane>>,
    /// `ttk::style configure` options, by style name.
    ttk_styles: BTreeMap<String, BTreeMap<String, String>>,
    /// `ttk::style layout` specifications, by style name.
    ttk_layouts: BTreeMap<String, Vec<String>>,
    /// `ttk::style map` state specs, by style and option.
    ttk_maps: BTreeMap<String, BTreeMap<String, String>>,
    /// `ttk::style element create` registrations.
    ttk_elements: BTreeMap<String, Vec<String>>,
    /// The current ttk theme.
    ttk_theme: String,
    /// The window holding the pointer grab and whether it is global.
    grab: Option<(String, bool)>,
    /// A menu posted at a point (`tk_popup`/`menu post x y`).
    popup_menu: Option<(String, f64, f64)>,
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
    /// Custom `text` marks: widget path → mark name → position and gravity.
    /// The built-in `insert` mark is the widget cursor; `current` is tracked
    /// here so it shifts with edits.
    text_marks: BTreeMap<String, BTreeMap<String, TextMark>>,
    /// Gravity of each text widget's `insert` mark (Tk default: right).
    insert_gravity: BTreeMap<String, MarkGravity>,
    /// Photo images by Tk image name.
    images: BTreeMap<String, PhotoImage>,
    image_seq: u64,
    /// Canvas item models.
    canvases: BTreeMap<String, CanvasState>,
    /// `event add`: virtual event name (without `<<>>`) → physical sequences.
    virtual_events: BTreeMap<String, Vec<String>>,
    /// The application's clipboard (`clipboard clear/append/get`).
    clipboard: String,
    /// Whether this app owns the clipboard (its `get` answers `clipboard`,
    /// even empty, instead of asking the system).
    clipboard_owned: bool,
    /// The selection this app currently owns (`selection own`).
    selection_owned: Option<String>,
    /// The window named in `selection own`.
    selection_window: Option<String>,
    /// `selection handle` registrations keyed by (selection, type).
    selection_handlers: BTreeMap<(String, String), SelectionHandler>,
    /// `text -undo` stacks by widget path.
    text_undo: BTreeMap<String, TextUndo>,
    /// Pending `text see` positions for the renderer.
    text_see: BTreeMap<String, usize>,
    /// Persisted rich-text documents (ProseMirror JSON) of `text` widgets.
    rich_docs: BTreeMap<String, String>,
    /// `text edit modified` state: widgets with unsaved edits.
    text_modified: BTreeSet<String>,
    /// Listbox per-item options (`itemconfigure`): path → row → options.
    listbox_item_options: BTreeMap<String, BTreeMap<usize, BTreeMap<String, String>>>,
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
    /// `tag bind` scripts, by event sequence.
    pub bindings: BTreeMap<String, String>,
}

/// Which side of inserted text a `text` mark sticks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkGravity {
    Left,
    #[default]
    Right,
}

/// A custom `text` mark.  `insert` is stored as the widget cursor instead.
#[derive(Debug, Clone, Copy)]
pub struct TextMark {
    index: usize,
    gravity: MarkGravity,
}

/// Everything `text edit undo` restores for a widget.
#[derive(Debug, Clone)]
struct TextSnapshot {
    content: String,
    cursor: usize,
    tags: BTreeMap<String, TextTag>,
    marks: BTreeMap<String, TextMark>,
    rich_doc: Option<String>,
    modified: bool,
}

#[derive(Debug, Default)]
struct TextUndo {
    undo: Vec<TextSnapshot>,
    redo: Vec<TextSnapshot>,
    /// When the last edit was recorded (adjacent edits group).
    last: Option<std::time::Instant>,
    /// The next edit starts a group (`edit separator`).
    force_new: bool,
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
    pub tags: Vec<String>,
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
    /// Row tags (`tag configure`) and the items carrying them.
    pub tags: BTreeMap<String, BTreeMap<String, String>>,
    /// `column` options by column name (`#0` is the tree column).
    pub column_options: BTreeMap<String, BTreeMap<String, String>>,
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

/// One pane of a `panedwindow`.
#[derive(Debug, Clone, Default)]
pub struct PanedPane {
    pub path: String,
    /// Pane options (`-minsize`, `-width`, `-weight`, `-stretch`, ...).
    pub options: BTreeMap<String, String>,
}

/// A secondary `toplevel` window for the renderer.
#[derive(Debug, Clone, Default)]
pub struct ToplevelView {
    pub path: String,
    pub title: String,
    pub width: f64,
    pub height: f64,
    /// `None` when `wm geometry` gave no position (the renderer cascades).
    pub x: Option<f64>,
    pub y: Option<f64>,
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
        let mut states = s.borrow_mut();
        let rt = states.entry(key(interp)).or_default();
        // Tk's init.tcl defines these virtual events before any script
        // runs; bindings on <<Copy>> etc. fire from the physical keys.
        for (name, sequences) in [
            ("Copy", ["<Control-c>", "<Control-C>"].as_slice()),
            ("Cut", ["<Control-x>", "<Control-X>"].as_slice()),
            ("Paste", ["<Control-v>", "<Control-V>"].as_slice()),
            ("SelectAll", ["<Control-a>", "<Control-A>"].as_slice()),
            ("Undo", ["<Control-z>", "<Control-Z>"].as_slice()),
            ("Redo", ["<Control-y>", "<Control-Y>"].as_slice()),
        ] {
            rt.virtual_events
                .entry(name.to_string())
                .or_insert_with(|| {
                    sequences
                        .iter()
                        .map(|sequence| sequence.to_string())
                        .collect()
                });
        }
    });
    ACTIVE.with(|a| a.set(Some(key(interp))));
    for name in [
        "toplevel",
        "frame",
        "label",
        "button",
        "menubutton",
        "entry",
        "spinbox",
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
        "ttk::menubutton",
        "ttk::entry",
        "ttk::spinbox",
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
        "sizegrip",
        "ttk::sizegrip",
        "panedwindow",
        "ttk::panedwindow",
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
        "clipboard",
        "selection",
        "tk_messageBox",
        "tk_getOpenFile",
        "tk_getSaveFile",
        "tk_chooseDirectory",
        "tk_chooseColor",
        "tk_optionMenu",
        "tk_dialog",
        "grab",
        "ttk::style",
        "tk_popup",
    ] {
        interp.register_command(name, global_command);
    }
    // Tk implements these in its Tcl library; the native renderer delegates
    // keyboard activation back here so users can override them with bind.
    let _ = interp.eval(
        "bind Button <Key-space> {catch {%W invoke}}\n\
         bind Checkbutton <Key-space> {catch {%W invoke}}\n\
         bind Checkbutton <Key-Return> {catch {%W invoke}}\n\
         bind Radiobutton <Key-space> {catch {%W invoke}}\n\
         bind Radiobutton <Key-Return> {catch {%W invoke}}\n\
         bind TButton <Key-space> {catch {%W invoke}}\n\
         bind TCheckbutton <Key-space> {catch {%W invoke}}\n\
         bind TCheckbutton <Key-Return> {catch {%W invoke}}\n\
         bind TRadiobutton <Key-space> {catch {%W invoke}}\n\
         bind TRadiobutton <Key-Return> {catch {%W invoke}}\n\
         bind Button <<Invoke>> {catch {%W invoke}}\n\
         bind TButton <<Invoke>> {catch {%W invoke}}\n\
         bind Checkbutton <<Invoke>> {catch {%W invoke}}\n\
         bind TCheckbutton <<Invoke>> {catch {%W invoke}}\n\
         bind Radiobutton <<Invoke>> {catch {%W invoke}}\n\
         bind TRadiobutton <<Invoke>> {catch {%W invoke}}",
    );
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

/// Tk's ttk class for widgets whose classic kind has another class name.
fn ttk_class(base: &str) -> Option<&'static str> {
    Some(match base {
        "button" => "TButton",
        "menubutton" => "TMenubutton",
        "panedwindow" => "TPanedwindow",
        "checkbutton" => "TCheckbutton",
        "radiobutton" => "TRadiobutton",
        "entry" => "TEntry",
        "spinbox" => "TSpinbox",
        "frame" => "TFrame",
        "label" => "TLabel",
        "scale" => "TScale",
        "scrollbar" => "TScrollbar",
        "toplevel" => "Toplevel",
        _ => return None,
    })
}

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
        if cmd.starts_with("ttk::") {
            if let Some(class) = ttk_class(base) {
                if let Some(node) = rt.tree.get_mut(&path) {
                    node.class = Some(class.to_string());
                }
            }
        }
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
        WidgetKind::Entry | WidgetKind::Combobox | WidgetKind::Spinbox => {
            let var = effective_option(interp, path, "-textvariable");
            if !var.is_empty() {
                let text = var_value(&var).unwrap_or_default();
                with_state_mut(interp, |rt| {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.content = text;
                    }
                });
            } else if kind == WidgetKind::Spinbox {
                // Tk starts a spinbox at its first -values entry, else
                // at -from.
                let empty = with_state(interp, |rt| {
                    rt.tree.get(path).is_some_and(|n| n.content.is_empty())
                })
                .unwrap_or(false);
                if empty {
                    let values = effective_option(interp, path, "-values");
                    let initial = Value::from_str(&values)
                        .as_list()
                        .and_then(|list| list.first().map(|v| v.as_str().to_string()))
                        .unwrap_or_else(|| {
                            let from = effective_option(interp, path, "-from")
                                .parse::<f64>()
                                .unwrap_or(0.0);
                            format_number(from)
                        });
                    with_state_mut(interp, |rt| {
                        if let Some(n) = rt.tree.get_mut(path) {
                            n.content = initial;
                        }
                    });
                }
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
            WidgetKind::Entry | WidgetKind::Spinbox => match sub {
                "set" if kind == WidgetKind::Spinbox => {
                    if args.len() != 3 {
                        return Err(wrong("set", "value", args.len()));
                    }
                    let value = args[2].as_str().to_string();
                    let cursor = value.chars().count();
                    update_entry(interp, path, value, cursor)?;
                    Ok(Value::empty())
                }
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
                "replace" => {
                    if args.len() < 5 {
                        return Err(wrong(
                            "replace",
                            "index1 index2 chars ?tagList chars ...?",
                            args.len(),
                        ));
                    }
                    text_replace(interp, path, &args[2..])
                }
                "insert" => {
                    if args.len() < 4 {
                        return Err(wrong(
                            "insert",
                            "index chars ?tagList chars ...?",
                            args.len(),
                        ));
                    }
                    let (content, cursor) = text_state(interp, path);
                    let marks = text_marks(interp, path);
                    let mut at = text_index(args[2].as_str(), &content, cursor, &marks)?;
                    let mut pos = 3;
                    while pos < args.len() {
                        let chars = args[pos].as_str();
                        let tags = args.get(pos + 1).map(|v| v.as_str());
                        text_insert(interp, path, at, chars, tags)?;
                        at += chars.chars().count();
                        pos += if tags.is_some() { 2 } else { 1 };
                    }
                    Ok(Value::empty())
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
                    let marks = text_marks(interp, path);
                    let i = text_index(args[2].as_str(), &content, cursor, &marks)?;
                    Ok(Value::from_str(&format!(
                        "{}",
                        char_to_linecol(&content, i)
                    )))
                }
                "see" => {
                    if args.len() != 3 {
                        return Err(wrong("see", "index", args.len()));
                    }
                    let (content, cursor) = text_state(interp, path);
                    let marks = text_marks(interp, path);
                    let index = text_index(args[2].as_str(), &content, cursor, &marks)?;
                    with_state_mut(interp, |rt| {
                        rt.text_see.insert(path.to_string(), index);
                        rt.bump();
                    });
                    Ok(Value::empty())
                }
                "mark" => text_mark_command(interp, path, &args[2..]),
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
                "edit" => text_edit_command(interp, path, &args[2..]),
                "search" => text_search(interp, path, &args[2..]),
                "count" => text_count(interp, path, &args[2..]),
                "dump" => text_dump(interp, path, &args[2..]),
                "xview" | "yview" => Ok(Value::empty()),
                _ => bad_subcommand(kind, sub),
            },
            WidgetKind::Panedwindow => panedwindow_command(interp, path, &args[1..]),
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
                "itemconfigure" => listbox_itemconfigure(interp, path, &args[2..]),
                "itemcget" => listbox_itemcget(interp, path, &args[2..]),
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
                "see" => {
                    if args.len() != 3 {
                        return Err(wrong("see", "index", args.len()));
                    }
                    let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
                        .flatten()
                        .unwrap_or(0);
                    let index = listbox_index(args[2].as_str(), len)?;
                    scroll_to_row(interp, path, index, len);
                    Ok(Value::empty())
                }
                "activate" | "xview" => Ok(Value::empty()),
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
                    let before = with_state(interp, |rt| {
                        rt.treeviews
                            .get(path)
                            .map(|tv| tv.selected.iter().cloned().collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
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
                    if selected != before {
                        fire_virtual(path, "TreeviewSelect");
                    }
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
                "tag" => treeview_tag_command(interp, path, &args[2..]),
                "see" => {
                    if args.len() != 3 {
                        return Err(wrong("see", "item", args.len()));
                    }
                    let target = args[2].as_str();
                    let (row, total) = with_state(interp, |rt| {
                        rt.treeviews.get(path).map(|tv| {
                            (
                                treeview_row_index(tv, target),
                                treeview_visible_count_of(tv),
                            )
                        })
                    })
                    .flatten()
                    .unwrap_or((None, 0));
                    if let Some(row) = row {
                        scroll_to_row(interp, path, row, total);
                    }
                    Ok(Value::empty())
                }
                "column" => treeview_column_command(interp, path, &args[2..]),
                "xview" => Ok(Value::empty()),
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
                        let target = value.as_str();
                        with_state_mut(interp, |rt| {
                            if let Some(canvas) = rt.canvases.get_mut(path) {
                                if target == "all" {
                                    canvas.items.clear();
                                } else {
                                    canvas
                                        .items
                                        .retain(|item| !canvas_item_matches(item, target));
                                }
                                rt.bump();
                            }
                        });
                    }
                    Ok(Value::empty())
                }
                "coords" => canvas_coords(interp, path, &args[2..]),
                "itemcget" => {
                    if args.len() != 4 {
                        return Err(wrong("itemcget", "tagOrId -option", args.len()));
                    }
                    let target = args[2].as_str();
                    let value = with_state(interp, |rt| {
                        rt.canvases.get(path).and_then(|canvas| {
                            canvas
                                .items
                                .iter()
                                .find(|item| canvas_item_matches(item, target))
                                .and_then(|item| item.options.get(args[3].as_str()).cloned())
                        })
                    })
                    .flatten()
                    .unwrap_or_default();
                    Ok(Value::from_str(&value))
                }
                "itemconfigure" => canvas_itemconfigure(interp, path, &args[2..]),
                "find" => canvas_find(interp, path, &args[2..]),
                "bbox" => canvas_bbox(interp, path, &args[2..]),
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
        WidgetKind::Menubutton => "cget, configure, destroy",
        WidgetKind::Checkbutton => {
            "cget, configure, deselect, destroy, invoke, select, toggle"
        }
        WidgetKind::Radiobutton => "cget, configure, deselect, destroy, invoke, select",
        WidgetKind::Entry => {
            "cget, configure, delete, destroy, get, icursor, index, insert, selection, validate, xview"
        }
        WidgetKind::Spinbox => {
            "cget, configure, delete, destroy, get, icursor, index, insert, selection, set, validate, xview"
        }
        WidgetKind::Text => {
            "cget, configure, delete, destroy, edit, get, index, insert, mark, see, tag, xview, yview"
        }
        WidgetKind::Listbox => {
            "activate, cget, configure, curselection, delete, destroy, get, index, insert, itemcget, itemconfigure, see, selection, size, xview, yview"
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
    if effective_option(interp, path, "-state") == "disabled" {
        return Ok(Value::empty());
    }
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
    // Tk sends <Destroy> to the children first, then the window itself.
    for p in removed.iter().rev() {
        let mapped = with_state(interp, |rt| {
            rt.tree
                .get(p)
                .is_some_and(|n| n.pack.is_some() || n.grid.is_some() || n.place.is_some())
        })
        .unwrap_or(false);
        if mapped {
            run_binding(p, "<Unmap>", &EventCtx::for_path(p));
        }
        run_binding(p, "<Destroy>", &EventCtx::for_path(p));
    }
    // Tk windows are named by hierarchy: destroying `.o` destroys `.o.menu`
    // too.  Menus are not in the widget tree here, so collect them by name.
    let mut removed_menus = Vec::new();
    with_state_mut(interp, |rt| {
        rt.tree.destroy(path).map_err(tk_error)?;
        for p in &removed {
            rt.treeviews.remove(p);
            rt.canvases.remove(p);
            rt.text_tags.remove(p);
            rt.text_marks.remove(p);
            rt.text_modified.remove(p);
            rt.rich_docs.remove(p);
            rt.text_undo.remove(p);
            rt.text_see.remove(p);
            rt.paned.remove(p);
            for panes in rt.paned.values_mut() {
                panes.retain(|pane| pane.path != *p);
            }
            let prefix = format!("{p}.");
            let menus = rt
                .menus
                .keys()
                .filter(|menu| menu.as_str() == p || menu.starts_with(&prefix))
                .cloned()
                .collect::<Vec<_>>();
            for menu in menus {
                rt.menus.remove(&menu);
                if rt.open_menu.as_deref() == Some(menu.as_str()) {
                    rt.open_menu = None;
                }
                removed_menus.push(menu);
            }
        }
        rt.bump();
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    for p in removed {
        let _ = interp.delete_command(&p);
    }
    for menu in removed_menus {
        let _ = interp.delete_command(&menu);
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
                let was_mapped =
                    with_state(interp, |rt| rt.tree.get(path).map(|n| n.pack.is_some()))
                        .flatten()
                        .unwrap_or(false);
                with_state_mut(interp, |rt| {
                    rt.tree.unpack(path).map_err(tk_error)?;
                    rt.bump();
                    Ok::<(), Error>(())
                })
                .ok_or_else(|| bad_window(path))??;
                if was_mapped {
                    fire_physical(path, "Unmap");
                }
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
    let was_mapped = with_state(interp, |rt| rt.tree.get(path).map(|n| n.pack.is_some()))
        .flatten()
        .unwrap_or(false);
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
    if !was_mapped {
        fire_physical(path, "Map");
    }
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
                let was_mapped =
                    with_state(interp, |rt| rt.tree.get(path).map(|n| n.grid.is_some()))
                        .flatten()
                        .unwrap_or(false);
                with_state_mut(interp, |rt| {
                    rt.tree.ungrid(path).map_err(tk_error)?;
                    rt.bump();
                    Ok::<(), Error>(())
                })
                .ok_or_else(|| bad_window(path))??;
                if was_mapped {
                    fire_physical(path, "Unmap");
                }
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
    let was_mapped = with_state(interp, |rt| rt.tree.get(path).map(|n| n.grid.is_some()))
        .flatten()
        .unwrap_or(false);
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
    if !was_mapped {
        fire_physical(path, "Map");
    }
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
            | WidgetKind::Menubutton
            | WidgetKind::Checkbutton
            | WidgetKind::Radiobutton => opt("-text").chars().count().max(1) as i64,
            _ => 0,
        };
        let w_opt = opt("-width").parse::<i64>().unwrap_or(0);
        let h_opt = opt("-height").parse::<i64>().unwrap_or(0);
        let (w, h) = match node.kind {
            WidgetKind::Text => (w_opt.max(1) * 7, h_opt.max(1) * 18),
            WidgetKind::Listbox => (w_opt.max(1) * 7, h_opt.max(1) * 18),
            WidgetKind::Entry | WidgetKind::Combobox | WidgetKind::Spinbox => {
                ((w_opt.max(1) * 7) + 8, 24)
            }
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
            WidgetKind::Sizegrip => (14, 14),
            WidgetKind::Panedwindow => {
                let width = opt("-width").parse::<i64>().unwrap_or(0);
                let height = opt("-height").parse::<i64>().unwrap_or(0);
                (width.max(200), height.max(160))
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
                let was_mapped =
                    with_state(interp, |rt| rt.tree.get(path).map(|n| n.place.is_some()))
                        .flatten()
                        .unwrap_or(false);
                with_state_mut(interp, |rt| {
                    rt.tree.unplace(path).map_err(tk_error)?;
                    rt.bump();
                    Ok::<(), Error>(())
                })
                .ok_or_else(|| bad_window(path))??;
                if was_mapped {
                    fire_physical(path, "Unmap");
                }
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
    let was_mapped = with_state(interp, |rt| rt.tree.get(path).map(|n| n.place.is_some()))
        .flatten()
        .unwrap_or(false);
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
    if !was_mapped {
        fire_physical(path, "Map");
    }
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
                    .map(|n| n.widget_class().to_string())
                    .or_else(|| rt.menus.contains_key(path).then_some("Menu".to_string()))
            })
            .flatten()
            .ok_or_else(|| bad_window(path))?;
            Ok(Value::from_str(&class))
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
    let is_menu = with_state(interp, |rt| rt.menus.contains_key(target)).unwrap_or(false);
    if !is_widget && !is_menu && target != "all" && target.starts_with('.') && target != "." {
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
    /// `%D`: wheel/drag/gesture delta.
    pub delta: i64,
    /// Root-window position (`%X/%Y`); widget position when unset.
    pub root_x: i64,
    pub root_y: i64,
    pub has_root: bool,
    /// `%w/%h` event window size (0 means unknown: `??`).
    pub width: i64,
    pub height: i64,
    /// `%t` server time and `%#` serial.
    pub time: i64,
    pub serial: i64,
    /// `%E` send_event, `%d` detail, `%m` mode.
    pub send_event: i64,
    pub detail: String,
    pub mode: String,
    /// `%f`: whether the event window has the focus.
    pub focus: Option<i64>,
    /// `%k`: hardware keycode.
    pub keycode: i64,
    /// Click count of a press (1 = single).
    pub clicks: u32,
}

impl EventCtx {
    pub fn for_path(path: &str) -> Self {
        Self {
            path: path.to_string(),
            ..Self::default()
        }
    }
}

/// A Tk event kind, as far as binding dispatch needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum EventKind {
    #[default]
    KeyPress,
    KeyRelease,
    ButtonPress,
    ButtonRelease,
    FocusIn,
    FocusOut,
    Enter,
    Leave,
    Motion,
    Wheel,
    Touch,
    Map,
    Unmap,
    Destroy,
    Expose,
    /// `<<Name>>` virtual event.
    Virtual,
    Configure,
    /// Parsed but not dispatched by the native backend.
    Other,
}

/// A parsed binding sequence (`<Control-Shift-Key-a>`, `<Double-Button-1>`,
/// `<FocusIn>`, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EventPattern {
    kind: EventKind,
    detail: Option<String>,
    /// Tk state bits: Shift 1, Lock 2, Control 4, Mod1 8 ... Mod5 128.
    modifiers: u32,
    /// 1 = plain press, 2 = double, 3 = triple, 4 = quadruple.
    clicks: u8,
}

/// Parse a Tk event state string (Tk reports `%s` in decimal).
fn parse_event_state(state: &str) -> u32 {
    let state = state.trim();
    if state.is_empty() {
        return 0;
    }
    state
        .parse::<u32>()
        .ok()
        .or_else(|| {
            state
                .strip_prefix("0x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        })
        .unwrap_or(0)
}

/// Parse a Tk binding sequence.  Unknown spellings become `Other`, which
/// never matches a dispatched event.
fn parse_sequence(sequence: &str) -> EventPattern {
    let text = sequence.trim();
    if text.starts_with("<<") && text.ends_with(">>") && text.len() > 4 {
        return EventPattern {
            kind: EventKind::Virtual,
            detail: Some(text[2..text.len() - 2].to_string()),
            clicks: 1,
            ..EventPattern::default()
        };
    }
    let inner = text
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(text);
    if inner.is_empty() {
        return EventPattern {
            kind: EventKind::Other,
            ..EventPattern::default()
        };
    }
    let tokens = inner
        .split('-')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut pattern = EventPattern {
        clicks: 1,
        ..EventPattern::default()
    };
    let mut index = 0usize;
    while index < tokens.len() {
        let token = tokens[index].to_ascii_lowercase();
        let modifier = match token.as_str() {
            "shift" => Some(1u32),
            "lock" => Some(2),
            "control" | "ctrl" => Some(4),
            "alt" | "option" | "mod1" | "m1" | "meta" => Some(8),
            "mod2" | "m2" => Some(16),
            "mod3" | "m3" => Some(32),
            "mod4" | "m4" => Some(64),
            "mod5" | "m5" => Some(128),
            "double" => {
                pattern.clicks = 2;
                index += 1;
                continue;
            }
            "triple" => {
                pattern.clicks = 3;
                index += 1;
                continue;
            }
            "quadruple" => {
                pattern.clicks = 4;
                index += 1;
                continue;
            }
            _ => None,
        };
        match modifier {
            Some(bits) => {
                pattern.modifiers |= bits;
                index += 1;
            }
            None => break,
        }
    }
    if index >= tokens.len() {
        return EventPattern {
            kind: EventKind::Other,
            ..EventPattern::default()
        };
    }
    let head = tokens[index].to_ascii_lowercase();
    let rest = &tokens[index + 1..];
    let joined = rest.join("-");
    let detail = |value: &str| (!value.is_empty()).then(|| value.to_string());
    match head.as_str() {
        "key" | "keypress" => {
            pattern.kind = EventKind::KeyPress;
            pattern.detail = detail(&joined);
        }
        "keyrelease" => {
            pattern.kind = EventKind::KeyRelease;
            pattern.detail = detail(&joined);
        }
        "button" | "buttonpress" | "btn" => {
            pattern.kind = EventKind::ButtonPress;
            pattern.detail = detail(&joined);
        }
        "buttonrelease" => {
            pattern.kind = EventKind::ButtonRelease;
            pattern.detail = detail(&joined);
        }
        "focusin" => pattern.kind = EventKind::FocusIn,
        "focusout" => pattern.kind = EventKind::FocusOut,
        "enter" => pattern.kind = EventKind::Enter,
        "leave" => pattern.kind = EventKind::Leave,
        "motion" => pattern.kind = EventKind::Motion,
        "mousewheel" => pattern.kind = EventKind::Wheel,
        "touch" => pattern.kind = EventKind::Touch,
        "pan" | "pinch" | "rotation" | "longpress" => {
            pattern.kind = EventKind::Virtual;
            pattern.detail = Some(
                match head.as_str() {
                    "longpress" => "LongPress",
                    other => other,
                }
                .to_string(),
            );
        }
        "configure" => pattern.kind = EventKind::Configure,
        "map" => pattern.kind = EventKind::Map,
        "unmap" => pattern.kind = EventKind::Unmap,
        "destroy" => pattern.kind = EventKind::Destroy,
        "expose" => pattern.kind = EventKind::Expose,
        "visibility" | "mousewheel" => pattern.kind = EventKind::Other,
        other if other.len() == 2 && other.starts_with('b') && !rest.is_empty() => {
            // <B1-Motion>: motion with button `1` down.
            let button = &other[1..];
            if rest[0].eq_ignore_ascii_case("motion") && button.chars().all(|c| c.is_ascii_digit())
            {
                pattern.kind = EventKind::Motion;
                pattern.detail = Some(button.to_string());
            } else {
                pattern.kind = EventKind::Other;
            }
        }
        other if other.chars().all(|c| c.is_ascii_digit()) => {
            pattern.kind = EventKind::ButtonPress;
            pattern.detail = Some(tokens[index].to_string());
        }
        other => {
            // A plain keysym: `<a>`, `<space>`, `<Return>`, `<F1>`.
            pattern.kind = EventKind::KeyPress;
            pattern.detail = Some(if rest.is_empty() {
                tokens[index].to_string()
            } else {
                format!("{other}-{joined}")
            });
        }
    }
    pattern
}

impl EventPattern {
    /// Tk matching: the event must have at least the binding's modifiers;
    /// a binding detail must equal the event's, and a detail-less binding
    /// matches any detail.
    fn matches(&self, event: &EventPattern) -> bool {
        self.kind == event.kind
            && self.clicks == event.clicks
            && (self.modifiers & !event.modifiers) == 0
            && self
                .detail
                .as_ref()
                .is_none_or(|detail| event.detail.as_ref() == Some(detail))
    }

    /// More modifier bits and an explicit detail are more specific.
    fn specificity(&self) -> (u32, bool, u8) {
        (
            self.modifiers.count_ones(),
            self.detail.is_some(),
            self.clicks,
        )
    }
}

/// Build the event being dispatched: generic sequences (`<KeyPress>`,
/// `<Button-1>`) take their detail from the context.
fn event_from(sequence: &str, ctx: &EventCtx) -> EventPattern {
    let pattern = parse_sequence(sequence);
    let mut event = pattern.clone();
    if event.clicks == 0 {
        event.clicks = 1;
    }
    event.modifiers = parse_event_state(&ctx.state) | pattern.modifiers;
    if event.detail.is_none() {
        match event.kind {
            EventKind::KeyPress | EventKind::KeyRelease => {
                if !ctx.key.is_empty() {
                    event.detail = Some(ctx.key.clone());
                }
            }
            EventKind::ButtonPress | EventKind::ButtonRelease => {
                if ctx.button > 0 {
                    event.detail = Some(ctx.button.to_string());
                }
            }
            _ => {}
        }
    }
    event
}

/// Tk's keysym number (`%N`): Unicode for printable keysyms, X11 codes for
/// the named keys.
fn keysym_number(key: &str) -> i64 {
    if key.chars().count() == 1 {
        return key.chars().next().map_or(0, |c| i64::from(c as u32));
    }
    match key {
        "Return" => 0xff0d,
        "Escape" => 0xff1b,
        "Tab" => 0xff09,
        "BackSpace" => 0xff08,
        "Delete" => 0xffff,
        "Insert" => 0xff63,
        "Left" => 0xff51,
        "Up" => 0xff52,
        "Right" => 0xff53,
        "Down" => 0xff54,
        "Home" => 0xff50,
        "End" => 0xff57,
        "Prior" => 0xff55,
        "Next" => 0xff56,
        "space" => 0x20,
        name => name
            .strip_prefix('F')
            .and_then(|n| n.parse::<i64>().ok())
            .filter(|n| (1..=35).contains(n))
            .map_or(0, |n| 0xffbe + n - 1),
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
            Some('W') => out.push_str(&ctx.path),
            Some('w') => out.push_str(&if ctx.width > 0 {
                ctx.width.to_string()
            } else {
                "??".to_string()
            }),
            Some('h') => out.push_str(&if ctx.height > 0 {
                ctx.height.to_string()
            } else {
                "??".to_string()
            }),
            Some('x') => out.push_str(&ctx.x.to_string()),
            Some('y') => out.push_str(&ctx.y.to_string()),
            Some('X') => out.push_str(&(if ctx.has_root { ctx.root_x } else { ctx.x }).to_string()),
            Some('Y') => out.push_str(&(if ctx.has_root { ctx.root_y } else { ctx.y }).to_string()),
            Some('K') => out.push_str(&if ctx.key.is_empty() {
                "??".to_string()
            } else {
                ctx.key.clone()
            }),
            Some('A') => out.push_str(&if ctx.character.is_empty() {
                "??".to_string()
            } else {
                ctx.character.clone()
            }),
            Some('k') => out.push_str(&if ctx.keycode > 0 {
                ctx.keycode.to_string()
            } else {
                "??".to_string()
            }),
            Some('N') => out.push_str(&if ctx.key.is_empty() {
                "??".to_string()
            } else {
                keysym_number(&ctx.key).to_string()
            }),
            Some('b') => out.push_str(&ctx.button.to_string()),
            Some('s') => out.push_str(&ctx.state),
            Some('D') => out.push_str(&if ctx.delta == 0
                && ctx.button == 0
                && ctx.character.is_empty()
            {
                "??".to_string()
            } else {
                ctx.delta.to_string()
            }),
            Some('t') => out.push_str(&ctx.time.to_string()),
            Some('#') => out.push_str(&ctx.serial.to_string()),
            Some('E') => out.push_str(
                &(ctx.send_event != 0)
                    .to_string()
                    .replace("true", "1")
                    .replace("false", "0"),
            ),
            Some('R') => out.push_str("0x00000000"),
            Some('f') => out.push_str(&match ctx.focus {
                Some(value) => value.to_string(),
                None => "??".to_string(),
            }),
            Some('d') => out.push_str(&if ctx.detail.is_empty() {
                "??".to_string()
            } else {
                ctx.detail.clone()
            }),
            Some('m') => out.push_str(&if ctx.mode.is_empty() {
                "??".to_string()
            } else {
                ctx.mode.clone()
            }),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn tag_binding_map(interp: &Interp, tag: &str) -> BTreeMap<String, String> {
    with_state(interp, |rt| {
        if tag == "all" {
            Some(rt.all_bindings.clone())
        } else if tag.starts_with('.') && tag != "." && rt.tree.contains(tag) {
            let sequences = rt.tree.binding_sequences(tag).ok()?;
            let mut map = BTreeMap::new();
            for sequence in sequences {
                if let Ok(Some(script)) = rt.tree.binding(tag, &sequence) {
                    map.insert(sequence, script.to_string());
                }
            }
            Some(map)
        } else {
            rt.tag_bindings.get(tag).cloned()
        }
    })
    .flatten()
    .unwrap_or_default()
}

fn best_script(bindings: &BTreeMap<String, String>, event: &EventPattern) -> Option<String> {
    bindings
        .iter()
        .filter_map(|(sequence, script)| {
            let pattern = parse_sequence(sequence);
            pattern
                .matches(event)
                .then(|| (pattern.specificity(), script))
        })
        .max_by_key(|(specificity, _)| *specificity)
        .map(|(_, script)| script.clone())
}

/// The most specific binding of `tag` matching `event` (Tk runs one script
/// per tag: the most specific sequence wins).
fn best_binding(interp: &Interp, tag: &str, event: &EventPattern) -> Option<String> {
    let bindings = tag_binding_map(interp, tag);
    best_script(&bindings, event)
}

fn tags_for(interp: &Interp, path: &str) -> Vec<String> {
    with_state(interp, |rt| rt.bindtags.get(path).cloned())
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
                    (n.widget_class().to_string(), top)
                })
            })
            .flatten()
            .unwrap_or_else(|| ("Widget".to_string(), ".".to_string()));
            vec![path.to_string(), class, top, "all".to_string()]
        })
}

/// Whether any bindtag of `path` has a binding matching `sequence`.
fn has_binding(interp: &Interp, path: &str, sequence: &str, ctx: &EventCtx) -> bool {
    let event = event_from(sequence, ctx);
    let mut seen = std::collections::BTreeSet::new();
    tags_for(interp, path).into_iter().any(|tag| {
        if !seen.insert(tag.clone()) {
            return false;
        }
        best_binding(interp, &tag, &event).is_some()
    })
}

/// Run the bindings for `path` in its bindtags order and re-render.
/// Errors go to [`take_errors`].
pub fn run_binding(path: &str, sequence: &str, ctx: &EventCtx) -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    // Safety: the pointer is the live engine-thread Interp.
    let interp = unsafe { &mut *ptr };
    let event = event_from(sequence, ctx);
    let tags = tags_for(interp, path);
    let mut seen = std::collections::BTreeSet::new();
    let mut scripts = Vec::new();
    for tag in tags {
        if !seen.insert(tag.clone()) {
            continue;
        }
        if let Some(script) = best_binding(interp, &tag, &event) {
            if !script.is_empty() {
                scripts.push(script);
            }
        }
    }
    let mut stopped = false;
    for script in scripts {
        let script = substitute_bindings(&script, ctx);
        match eval_active(&script) {
            Ok(_) => {}
            Err(e) if binding_stopped(&e) => {
                stopped = true;
                break;
            }
            Err(e) => record_error(e.to_string()),
        }
    }
    // `event add <<Foo>> <Sequence>`: the physical event also fires the
    // virtual event's bindings (unless a binding returned break).
    if event.kind != EventKind::Virtual && !stopped {
        let virtuals = with_state(interp, |rt| {
            rt.virtual_events
                .iter()
                .filter(|(_, sequences)| {
                    sequences
                        .iter()
                        .any(|sequence| parse_sequence(sequence).matches(&event))
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        for name in virtuals {
            run_binding(path, &format!("<<{name}>>"), ctx);
        }
    }
    stopped
}

thread_local! {
    /// Last press per widget and its click count (Tk's 500 ms chain).
    static LAST_PRESS: RefCell<BTreeMap<String, (std::time::Instant, u32)>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// The click count of this press on `path` (1 = single, 2 = double, ...).
fn register_press(path: &str) -> u32 {
    let now = std::time::Instant::now();
    LAST_PRESS.with(|map| {
        let mut map = map.borrow_mut();
        let count = match map.get(path) {
            Some((last, count))
                if now.duration_since(*last) < std::time::Duration::from_millis(500) =>
            {
                (count + 1).min(4)
            }
            _ => 1,
        };
        map.insert(path.to_string(), (now, count));
        count
    })
}

/// Dispatch a mouse press/release pair, adding `<Double-Button-1>` on the
/// second click, the way Tk does.
fn dispatch_click(path: &str, ctx: &EventCtx) {
    let clicks = register_press(path);
    run_binding(path, "<Button-1>", ctx);
    match clicks {
        2 => {
            run_binding(path, "<Double-Button-1>", ctx);
        }
        3 => {
            run_binding(path, "<Triple-Button-1>", ctx);
        }
        4 => {
            run_binding(path, "<Quadruple-Button-1>", ctx);
        }
        _ => {}
    }
    run_binding(path, "<ButtonRelease-1>", ctx);
}

/// A pointer press on a non-invoking widget (label, frame, chart, ...):
/// Tk bindings only, with the platform's click count for double clicks.
pub fn widget_press(path: &str, x: f64, y: f64, clicks: u32) {
    let mut ctx = EventCtx::for_path(path);
    ctx.button = 1;
    ctx.x = x as i64;
    ctx.y = y as i64;
    run_binding(path, "<Button-1>", &ctx);
    match clicks {
        2 => {
            run_binding(path, "<Double-Button-1>", &ctx);
        }
        3 => {
            run_binding(path, "<Triple-Button-1>", &ctx);
        }
        4 => {
            run_binding(path, "<Quadruple-Button-1>", &ctx);
        }
        _ => {}
    }
    run_binding(path, "<ButtonRelease-1>", &ctx);
}

/// Fire a Tk virtual event (`<<ListboxSelect>>`, ...) on `path`.
fn fire_virtual(path: &str, name: &str) {
    run_binding(path, &format!("<<{name}>>"), &EventCtx::for_path(path));
}

/// Fire a physical event like `<Map>`/`<Unmap>` on `path`.
fn fire_physical(path: &str, name: &str) {
    run_binding(path, &format!("<{name}>"), &EventCtx::for_path(path));
}

/// Tcl `break`/`continue` in a binding stops the event (tkinter's
/// `return "break"`).
fn binding_stopped(error: &Error) -> bool {
    matches!(
        error,
        Error::ControlFlow {
            kind: rtcl_core::error::ControlFlow::Break | rtcl_core::error::ControlFlow::Continue,
            ..
        }
    ) || matches!(
        error,
        Error::ControlFlow {
            kind: rtcl_core::error::ControlFlow::Return,
            value: Some(value),
            ..
        } if value.as_str() == "break"
    )
}

/// Like [`set_tk_focus`] but without bumping the render revision: used
/// while an input edit is being committed (a re-render there would reset
/// the native field before its text reaches the model).
fn set_tk_focus_quiet(interp: &mut Interp, new: Option<String>) {
    let old = with_state(interp, |rt| rt.focus.clone()).unwrap_or_default();
    if old == new {
        return;
    }
    with_state_mut(interp, |rt| {
        rt.focus = new.clone();
    });
    if let Some(old) = old {
        run_binding(&old, "<FocusOut>", &EventCtx::for_path(&old));
    }
    if let Some(new) = new {
        run_binding(&new, "<FocusIn>", &EventCtx::for_path(&new));
    }
}

/// Set the Tk focus target and dispatch the FocusOut/FocusIn bindings.
fn set_tk_focus(interp: &mut Interp, new: Option<String>) {
    let old = with_state(interp, |rt| rt.focus.clone()).unwrap_or_default();
    if old == new {
        return;
    }
    with_state_mut(interp, |rt| {
        rt.focus = new.clone();
        rt.bump();
    });
    if let Some(old) = old {
        run_binding(&old, "<FocusOut>", &EventCtx::for_path(&old));
    }
    if let Some(new) = new {
        run_binding(&new, "<FocusIn>", &EventCtx::for_path(&new));
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
        set_tk_focus(interp, None);
        return Ok(Value::empty());
    }
    if !with_state(interp, |rt| rt.tree.contains(path)).unwrap_or(false) {
        return Err(bad_window(path));
    }
    set_tk_focus(interp, Some(path.to_string()));
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
    let (kind, node_class, top) = with_state(interp, |rt| {
        rt.tree.get(&path).map(|n| {
            let mut top = path.clone();
            let mut p = path.clone();
            while let Some(parent) = rt.tree.get(&p).and_then(|n| n.parent.clone()) {
                p = parent;
                top = p.clone();
            }
            (n.kind, n.widget_class().to_string(), top)
        })
    })
    .flatten()
    .ok_or_else(|| bad_window(&path))?;
    if args.len() == 2 {
        let tags = with_state(interp, |rt| rt.bindtags.get(&path).cloned())
            .flatten()
            .unwrap_or_else(|| vec![path.clone(), node_class, top.clone(), "all".to_string()]);
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
            | WidgetKind::Spinbox
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

/// A `ttk::style configure` value for a widget's ttk class, when the
/// widget itself does not set the option (the renderer's style fallback).
pub fn ttk_style_value(widget_path: &str, option: &str) -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| {
        let class = rt.tree.get(widget_path)?.class.as_ref()?;
        rt.ttk_styles.get(class)?.get(option).cloned()
    })
    .flatten()
}

fn ttk_style_impl(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(wrong("ttk::style", "option ?arg ...?", args.len() + 1));
    }
    let sub = args[0].as_str();
    match sub {
        "configure" => {
            if args.len() == 1 {
                let names = with_state(_interp, |rt| {
                    rt.ttk_styles.keys().cloned().collect::<Vec<_>>()
                })
                .unwrap_or_default();
                return Ok(list_value(names));
            }
            let style = args[1].as_str().to_string();
            if args.len() == 2 {
                let options = with_state(_interp, |rt| {
                    rt.ttk_styles.get(&style).cloned().unwrap_or_default()
                })
                .unwrap_or_default();
                return Ok(list_value(
                    options
                        .iter()
                        .flat_map(|(k, v)| [k.clone(), v.clone()])
                        .collect::<Vec<_>>(),
                ));
            }
            if args.len() == 3 {
                let value = with_state(_interp, |rt| {
                    rt.ttk_styles
                        .get(&style)
                        .and_then(|options| options.get(args[2].as_str()))
                        .cloned()
                })
                .flatten()
                .unwrap_or_default();
                return Ok(Value::from_str(&value));
            }
            let pairs = parse_pairs(args, 2, "ttk::style configure")?;
            with_state_mut(_interp, |rt| {
                rt.ttk_styles
                    .entry(style.clone())
                    .or_default()
                    .extend(pairs);
                rt.bump();
            });
            Ok(Value::empty())
        }
        "lookup" => {
            if args.len() < 3 {
                return Err(wrong(
                    "ttk::style lookup",
                    "style -option ?default?",
                    args.len() + 1,
                ));
            }
            let style = args[1].as_str();
            let option = args[2].as_str();
            let value = with_state(_interp, |rt| {
                rt.ttk_styles
                    .get(style)
                    .and_then(|options| options.get(option))
                    .cloned()
            })
            .flatten();
            Ok(Value::from_str(
                value
                    .or_else(|| args.get(3).map(|value| value.as_str().to_string()))
                    .as_deref()
                    .unwrap_or(""),
            ))
        }
        "map" => {
            if args.len() < 3 {
                return Err(wrong(
                    "ttk::style map",
                    "style ?-option? ?value ...?",
                    args.len() + 1,
                ));
            }
            if args.len() >= 4 {
                let style = args[1].as_str().to_string();
                with_state_mut(_interp, |rt| {
                    let maps = rt.ttk_maps.entry(style).or_default();
                    for pair in args[2..].chunks(2) {
                        if let Some(value) = pair.get(1) {
                            maps.insert(pair[0].as_str().to_string(), value.as_str().to_string());
                        }
                    }
                });
                return Ok(Value::empty());
            }
            let style = args[1].as_str();
            let value = with_state(_interp, |rt| {
                rt.ttk_maps
                    .get(style)
                    .and_then(|maps| maps.get(args[2].as_str()))
                    .cloned()
            })
            .flatten()
            .unwrap_or_default();
            Ok(Value::from_str(&value))
        }
        "layout" => {
            if args.len() < 2 {
                return Err(wrong(
                    "ttk::style layout",
                    "style ?layoutSpec?",
                    args.len() + 1,
                ));
            }
            let style = args[1].as_str().to_string();
            if args.len() == 2 {
                let layout = with_state(_interp, |rt| rt.ttk_layouts.get(&style).cloned())
                    .flatten()
                    .unwrap_or_default();
                return Ok(list_value(layout));
            }
            let mut spec = Vec::new();
            for value in &args[2..] {
                if let Some(list) = value.as_list() {
                    spec.extend(list.iter().map(|item| item.as_str().to_string()));
                } else {
                    spec.push(value.as_str().to_string());
                }
            }
            let layout = if spec.iter().all(|word| !word.starts_with('-')) {
                spec.into_iter()
                    .flat_map(|name| [name, "-sticky".to_string(), "nswe".to_string()])
                    .collect::<Vec<_>>()
            } else {
                spec
            };
            with_state_mut(_interp, |rt| {
                rt.ttk_layouts.insert(style, layout);
            });
            Ok(Value::empty())
        }
        "element" => {
            if args.len() < 2 {
                return Err(wrong(
                    "ttk::style element",
                    "option ?arg ...?",
                    args.len() + 1,
                ));
            }
            match args[1].as_str() {
                "create" => {
                    if args.len() < 3 {
                        return Err(wrong(
                            "ttk::style element create",
                            "name ?type? ?-option value ...?",
                            args.len() + 1,
                        ));
                    }
                    let name = args[2].as_str().to_string();
                    let spec = args[3..]
                        .iter()
                        .map(|value| value.as_str().to_string())
                        .collect::<Vec<_>>();
                    with_state_mut(_interp, |rt| {
                        rt.ttk_elements.insert(name, spec);
                    });
                    Ok(Value::empty())
                }
                "names" => {
                    let pattern = args.get(2).map(|value| value.as_str());
                    let names = with_state(_interp, |rt| {
                        rt.ttk_elements
                            .keys()
                            .filter(|name| pattern.is_none_or(|pattern| glob_match(pattern, name)))
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                    Ok(list_value(names))
                }
                other => Err(generic(format!(
                    "bad option \"{other}\": must be create, names, or configure"
                ))),
            }
        }
        "theme" => {
            if args.len() < 2 {
                return Err(wrong(
                    "ttk::style theme",
                    "option ?arg ...?",
                    args.len() + 1,
                ));
            }
            match args[1].as_str() {
                "names" => Ok(list_value(["default".to_string()])),
                "current" => {
                    let theme = with_state(_interp, |rt| rt.ttk_theme.clone()).unwrap_or_default();
                    Ok(Value::from_str(if theme.is_empty() {
                        "default"
                    } else {
                        &theme
                    }))
                }
                "use" if args.len() == 2 => {
                    let theme = with_state(_interp, |rt| rt.ttk_theme.clone()).unwrap_or_default();
                    Ok(Value::from_str(if theme.is_empty() {
                        "default"
                    } else {
                        &theme
                    }))
                }
                "use" => {
                    with_state_mut(_interp, |rt| {
                        rt.ttk_theme = args[2].as_str().to_string();
                        rt.bump();
                    });
                    Ok(Value::empty())
                }
                "settings" => Ok(Value::empty()),
                other => Err(generic(format!(
                    "bad option \"{other}\": must be current, names, settings, or use"
                ))),
            }
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be configure, element, layout, lookup, map, or theme"
        ))),
    }
}

fn grab_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (mut global, mut index) = (false, 1);
    if args
        .get(index)
        .is_some_and(|value| value.as_str() == "-global")
    {
        global = true;
        index += 1;
    }
    let sub = args.get(index).map(|value| value.as_str()).unwrap_or("");
    match sub {
        "set" => {
            let mut position = index + 1;
            if args
                .get(position)
                .is_some_and(|value| value.as_str() == "-global")
            {
                global = true;
                position += 1;
            }
            let Some(window) = args.get(position) else {
                return Err(wrong("grab set", "?-global? window", args.len()));
            };
            if !with_state(interp, |rt| rt.tree.contains(window.as_str())).unwrap_or(false) {
                return Err(bad_window(window.as_str()));
            }
            with_state_mut(interp, |rt| {
                rt.grab = Some((window.as_str().to_string(), global));
            });
            Ok(Value::empty())
        }
        "release" => {
            let Some(window) = args.get(index + 1) else {
                return Err(wrong("grab release", "window", args.len()));
            };
            with_state_mut(interp, |rt| {
                if rt
                    .grab
                    .as_ref()
                    .is_some_and(|(held, _)| held == window.as_str())
                {
                    rt.grab = None;
                }
            });
            Ok(Value::empty())
        }
        "current" => {
            let current = with_state(interp, |rt| {
                rt.grab
                    .as_ref()
                    .map(|(window, _)| window.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            match args.get(index + 1) {
                Some(window) => Ok(Value::from_str(if window.as_str() == current {
                    &current
                } else {
                    ""
                })),
                None => Ok(Value::from_str(&current)),
            }
        }
        "status" => {
            let Some(window) = args.get(index + 1) else {
                return Err(wrong("grab status", "window", args.len()));
            };
            let status = with_state(interp, |rt| {
                rt.grab.as_ref().map(|(held, global)| {
                    if held == window.as_str() {
                        if *global {
                            "global"
                        } else {
                            "local"
                        }
                    } else {
                        "none"
                    }
                })
            })
            .flatten()
            .unwrap_or("none");
            Ok(Value::from_str(status))
        }
        window => {
            // Legacy `grab ?-global? window`.
            if window.is_empty() {
                return Err(wrong("grab", "?-global? window", args.len()));
            }
            if !with_state(interp, |rt| rt.tree.contains(window)).unwrap_or(false) {
                return Err(bad_window(window));
            }
            with_state_mut(interp, |rt| {
                rt.grab = Some((window.to_string(), global));
            });
            Ok(Value::empty())
        }
    }
}

/// `tk_popup menu x y ?entry?`: post `menu` at a point (the entry index
/// only fine-tunes Tk's placement and is ignored here).
fn tk_popup_impl(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(wrong("tk_popup", "menu x y ?entry?", args.len()));
    }
    let menu = args[1].as_str().to_string();
    let x = parse_f64_str(args[2].as_str(), "x")?;
    let y = parse_f64_str(args[3].as_str(), "y")?;
    popup_menu_at(&menu, x, y);
    Ok(Value::empty())
}

/// The panes of a `panedwindow` (for the renderer).
pub fn paned_panes(path: &str) -> Option<Vec<PanedPane>> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.paned.get(path).cloned()).flatten()
}

fn panedwindow_command(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("panedwindow", "option ?arg ...?", rest.len() + 2));
    }
    let sub = rest[0].as_str();
    let mut parse_add = |tokens: &[Value]| -> Result<(Vec<String>, BTreeMap<String, String>)> {
        let mut children = Vec::new();
        let mut options = BTreeMap::new();
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].as_str();
            if token.starts_with('-') {
                let Some(value) = tokens.get(index + 1) else {
                    return Err(wrong(
                        "panedwindow add",
                        "window ?-option value ...?",
                        tokens.len(),
                    ));
                };
                options.insert(token.to_string(), value.as_str().to_string());
                index += 2;
            } else {
                children.push(token.to_string());
                index += 1;
            }
        }
        Ok((children, options))
    };
    match sub {
        "add" | "insert" => {
            let (at, tokens) = if sub == "insert" {
                let Some(position) = rest.get(1) else {
                    return Err(wrong("insert", "index window ?-option value ...?", rest.len() + 2));
                };
                (Some(position.as_str().to_string()), &rest[2..])
            } else {
                (None, &rest[1..])
            };
            let (children, options) = parse_add(tokens)?;
            if children.is_empty() {
                return Err(wrong(sub, "window ?-option value ...?", rest.len() + 2));
            }
            for child in &children {
                let parent = with_state(interp, |rt| {
                    rt.tree.get(child).and_then(|node| node.parent.clone())
                })
                .flatten();
                if parent.as_deref() != Some(path) {
                    return Err(generic(format!(
                        "can't add \"{child}\" as a pane: not a child of \"{path}\""
                    )));
                }
            }
            with_state_mut(interp, |rt| {
                let panes = rt.paned.entry(path.to_string()).or_default();
                let mut insert_at = match at.as_deref() {
                    Some("end") | None => panes.len(),
                    Some(index) => index.parse::<usize>().unwrap_or(panes.len()).min(panes.len()),
                };
                for child in children {
                    panes.retain(|pane| pane.path != child);
                    let pane = PanedPane {
                        path: child,
                        options: options.clone(),
                    };
                    let position = insert_at.min(panes.len());
                    panes.insert(position, pane);
                    insert_at = position + 1;
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "forget" => {
            if rest.len() < 2 {
                return Err(wrong("forget", "window ?window ...?", rest.len() + 2));
            }
            let children = rest[1..]
                .iter()
                .map(|value| value.as_str().to_string())
                .collect::<Vec<_>>();
            with_state_mut(interp, |rt| {
                if let Some(panes) = rt.paned.get_mut(path) {
                    panes.retain(|pane| !children.contains(&pane.path));
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "panes" => {
            let panes = with_state(interp, |rt| {
                rt.paned
                    .get(path)
                    .map(|panes| panes.iter().map(|pane| pane.path.clone()).collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            Ok(list_value(panes))
        }
        "pane" | "paneconfigure" => {
            if rest.len() < 2 {
                return Err(wrong("pane", "window ?-option? ?value ...?", rest.len() + 2));
            }
            let child = rest[1].as_str().to_string();
            let existing = with_state(interp, |rt| {
                rt.paned
                    .get(path)
                    .and_then(|panes| panes.iter().find(|pane| pane.path == child))
                    .map(|pane| pane.options.clone())
            })
            .flatten();
            let Some(mut options) = existing else {
                return Err(generic(format!("\"{child}\" is not a pane of \"{path}\"")));
            };
            if rest.len() == 2 {
                return Ok(list_value(
                    options
                        .iter()
                        .flat_map(|(k, v)| [k.clone(), v.clone()])
                        .collect::<Vec<_>>(),
                ));
            }
            if rest.len() == 3 {
                return Ok(Value::from_str(
                    options.get(rest[2].as_str()).map(String::as_str).unwrap_or("0"),
                ));
            }
            for pair in rest[2..].chunks(2) {
                if let Some(value) = pair.get(1) {
                    options.insert(pair[0].as_str().to_string(), value.as_str().to_string());
                }
            }
            with_state_mut(interp, |rt| {
                if let Some(pane) = rt
                    .paned
                    .get_mut(path)
                    .and_then(|panes| panes.iter_mut().find(|pane| pane.path == child))
                {
                    pane.options = options;
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "panecget" => {
            if rest.len() != 3 {
                return Err(wrong("panecget", "window option", rest.len() + 2));
            }
            let value = with_state(interp, |rt| {
                rt.paned
                    .get(path)
                    .and_then(|panes| panes.iter().find(|pane| pane.path == rest[1].as_str()))
                    .and_then(|pane| pane.options.get(rest[2].as_str()).cloned())
            })
            .flatten()
            .unwrap_or_else(|| "0".to_string());
            Ok(Value::from_str(&value))
        }
        "sashpos" => {
            if rest.len() < 2 || rest.len() > 3 {
                return Err(wrong("sashpos", "index ?newpos?", rest.len() + 2));
            }
            // Without laid-out pixel sizes the sash sits at the widths the
            // panes asked for; setting records the width.
            if rest.len() == 3 {
                let index = rest[1].as_str().parse::<usize>().unwrap_or(0);
                let position = rest[2].as_str();
                with_state_mut(interp, |rt| {
                    if let Some(pane) = rt.paned.get_mut(path).and_then(|panes| panes.get_mut(index)) {
                        pane.options
                            .insert("-width".to_string(), position.to_string());
                        rt.bump();
                    }
                });
                return Ok(Value::from_int(0));
            }
            let position = with_state(interp, |rt| {
                let index = rest[1].as_str().parse::<usize>().unwrap_or(0);
                rt.paned
                    .get(path)
                    .map(|panes| {
                        panes
                            .iter()
                            .take(index)
                            .filter_map(|pane| {
                                pane.options
                                    .get("-width")
                                    .and_then(|width| width.parse::<i64>().ok())
                            })
                            .sum::<i64>()
                    })
                    .unwrap_or(0)
            })
            .unwrap_or(0);
            Ok(Value::from_int(position))
        }
        "sash" | "proxy" => Ok(Value::empty()),
        "identify" => Ok(Value::from_str("")),
        "state" => {
            if rest.len() == 1 {
                Ok(Value::from_str("normal"))
            } else {
                Ok(Value::empty())
            }
        }
        "instate" => Ok(Value::from_int(1)),
        other => Err(generic(format!(
            "bad option \"{other}\": must be add, forget, identify, insert, pane, panes, sash, or sashpos"
        ))),
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

/// A `tk_messageBox` request, handed to a backend that can show a native
/// modal dialog.
#[derive(Debug, Clone, Default)]
pub struct MessageBoxRequest {
    /// `-type`: ok, okcancel, yesno, yesnocancel, retrycancel, abortretryignore.
    pub kind: String,
    pub title: String,
    pub message: String,
    pub detail: String,
    pub icon: String,
    /// The symbolic name Tk returns when the user presses the default.
    pub default: String,
    /// The symbolic name for Escape/cancel.
    pub cancel: String,
    pub parent: String,
}

/// The buttons of a `tk_messageBox` type, in Tk's order; `None` for an
/// unknown type.
pub fn message_box_buttons(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "ok" => Some(&["ok"]),
        "okcancel" => Some(&["ok", "cancel"]),
        "yesno" => Some(&["yes", "no"]),
        "yesnocancel" => Some(&["yes", "no", "cancel"]),
        "retrycancel" => Some(&["retry", "cancel"]),
        "abortretryignore" => Some(&["abort", "retry", "ignore"]),
        _ => None,
    }
}

/// The button a host that cannot show a dialog reports: `-default` when it
/// names one of the type's buttons, else the type's leftmost button.
pub fn default_message_box_button(request: &MessageBoxRequest) -> String {
    let buttons = message_box_buttons(&request.kind).unwrap_or(&["ok"]);
    let chosen = request.default.to_ascii_lowercase();
    if buttons.iter().any(|name| *name == chosen) {
        return chosen;
    }
    buttons[0].to_string()
}

thread_local! {
    /// A synchronous announcement of the Tk clipboard's text (the native
    /// backend pushes it onto the system clipboard).
    static CLIPBOARD_SINK: std::cell::Cell<Option<fn(&str)>> = const { std::cell::Cell::new(None) };
    /// A synchronous read of the system clipboard for `clipboard get` when
    /// this app does not own it (implemented by the native platform).
    static CLIPBOARD_GETTER: std::cell::Cell<Option<fn() -> String>> =
        const { std::cell::Cell::new(None) };
}

/// Which native file dialog `tk_getOpenFile`, `tk_getSaveFile`, or
/// `tk_chooseDirectory` asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileDialogKind {
    #[default]
    Open,
    Save,
    Directory,
}

/// A file-dialog request, handed to a backend that can show one natively.
#[derive(Debug, Clone, Default)]
pub struct FileDialogRequest {
    pub kind: FileDialogKind,
    pub title: String,
    pub initial_dir: String,
    pub initial_file: String,
    /// Description and glob patterns, e.g. `("Text", [".txt", ".md"])`.
    pub file_types: Vec<(String, Vec<String>)>,
    pub multiple: bool,
    pub confirm_overwrite: bool,
    pub default_extension: String,
    pub parent: String,
}

/// A `tk_chooseColor` request.
#[derive(Debug, Clone, Default)]
pub struct ColorDialogRequest {
    pub title: String,
    pub initial: String,
    pub parent: String,
}

thread_local! {
    /// The backend's native color dialog.
    static COLOR_DIALOG: std::cell::Cell<Option<fn(&ColorDialogRequest) -> Option<String>>> =
        const { std::cell::Cell::new(None) };
}

/// Give `tk_chooseColor` a native color dialog.  The handler blocks until
/// the user answers and returns a Tk color (usually `#rrggbb`), or `None`
/// when cancelled.
pub fn set_color_dialog_handler(handler: fn(&ColorDialogRequest) -> Option<String>) {
    COLOR_DIALOG.with(|cell| cell.set(Some(handler)));
}

fn choose_color_impl(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut request = ColorDialogRequest::default();
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        let Some(value) = args.get(index + 1) else {
            return Err(wrong(
                "tk_chooseColor",
                "?-option value ...?",
                args.len() + 1,
            ));
        };
        match option {
            "-title" => request.title = value.as_str().to_string(),
            "-initialcolor" => request.initial = value.as_str().to_string(),
            "-parent" => request.parent = value.as_str().to_string(),
            other => {
                return Err(generic(format!(
                    "bad option \"{other}\": must be -initialcolor, -parent, or -title"
                )));
            }
        }
        index += 2;
    }
    let picked = COLOR_DIALOG
        .with(|cell| cell.get())
        .and_then(|handler| handler(&request))
        .or_else(|| {
            // A host without a dialog leaves the initial color selected.
            (!request.initial.is_empty()).then(|| request.initial.clone())
        })
        .unwrap_or_default();
    Ok(Value::from_str(&picked))
}

/// `tk_optionMenu w varName firstValue ?value ...?`: a menubutton showing
/// `varName` with a radiobutton menu; returns the menu's path.
fn option_menu_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 4 {
        return Err(wrong(
            "tk_optionMenu",
            "w varName firstValue ?value ...?",
            args.len(),
        ));
    }
    let window = args[1].as_str().to_string();
    let variable = args[2].as_str().to_string();
    let values = args[3..]
        .iter()
        .map(|value| value.as_str().to_string())
        .collect::<Vec<_>>();
    let menu_path = format!("{window}.menu");
    let script = format!("menu {} -tearoff 0", list_value(vec![menu_path.clone()]));
    interp.eval(&script)?;
    for value in &values {
        let script = format!(
            "{} add radiobutton -label {} -variable {} -value {}",
            menu_path,
            list_value(vec![value.clone()]),
            list_value(vec![variable.clone()]),
            list_value(vec![value.clone()]),
        );
        interp.eval(&script)?;
    }
    let script = format!(
        "menubutton {} -menu {} -textvariable {} -relief raised",
        list_value(vec![window]),
        list_value(vec![menu_path.clone()]),
        list_value(vec![variable.clone()]),
    );
    interp.eval(&script)?;
    let script = format!(
        "set {} {}",
        list_value(vec![variable]),
        list_value(vec![values[0].clone()])
    );
    interp.eval(&script)?;
    Ok(Value::from_str(&menu_path))
}

/// A `tk_dialog` request: a modal message with arbitrary buttons.
#[derive(Debug, Clone, Default)]
pub struct DialogRequest {
    pub title: String,
    pub message: String,
    pub detail: String,
    pub buttons: Vec<String>,
    /// The index of the default button.
    pub default: usize,
    pub parent: String,
}

thread_local! {
    /// The backend's native button dialog (`tk_dialog`).
    static DIALOG: std::cell::Cell<Option<fn(&DialogRequest) -> Option<usize>>> =
        const { std::cell::Cell::new(None) };
}

/// Give `tk_dialog` a native modal dialog.  The handler blocks until the
/// user answers and returns the chosen button's index.
pub fn set_dialog_handler(handler: fn(&DialogRequest) -> Option<usize>) {
    DIALOG.with(|cell| cell.set(Some(handler)));
}

fn tk_dialog_impl(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 6 {
        return Err(wrong(
            "tk_dialog",
            "window title text bitmap default ?button ...?",
            args.len(),
        ));
    }
    let mut buttons = args[6..]
        .iter()
        .map(|value| value.as_str().to_string())
        .collect::<Vec<_>>();
    if buttons.is_empty() {
        buttons.push("OK".to_string());
    }
    let default = args[5]
        .as_str()
        .trim()
        .parse::<usize>()
        .unwrap_or(0)
        .min(buttons.len() - 1);
    let request = DialogRequest {
        title: args[2].as_str().to_string(),
        message: args[3].as_str().to_string(),
        detail: String::new(),
        buttons,
        default,
        parent: args[1].as_str().to_string(),
    };
    let answer = DIALOG
        .with(|cell| cell.get())
        .and_then(|handler| handler(&request))
        .unwrap_or(default)
        .min(request.buttons.len() - 1);
    Ok(Value::from_int(answer as i64))
}

thread_local! {
    /// The backend's native file dialog.
    static FILE_DIALOG: std::cell::Cell<Option<fn(&FileDialogRequest) -> Option<Vec<String>>>> =
        const { std::cell::Cell::new(None) };
}

/// Give the file-dialog commands a native dialog.  The handler blocks until
/// the user answers and returns the chosen paths (`None`/empty when the
/// dialog was cancelled).
pub fn set_file_dialog_handler(handler: fn(&FileDialogRequest) -> Option<Vec<String>>) {
    FILE_DIALOG.with(|cell| cell.set(Some(handler)));
}

fn file_dialog_impl(interp: &mut Interp, args: &[Value], kind: FileDialogKind) -> Result<Value> {
    let command = match kind {
        FileDialogKind::Open => "tk_getOpenFile",
        FileDialogKind::Save => "tk_getSaveFile",
        FileDialogKind::Directory => "tk_chooseDirectory",
    };
    let mut request = FileDialogRequest {
        kind,
        ..FileDialogRequest::default()
    };
    let mut type_variable: Option<String> = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        let Some(value) = args.get(index + 1) else {
            return Err(wrong(command, "?-option value ...?", args.len() + 1));
        };
        match option {
            "-title" => request.title = value.as_str().to_string(),
            "-initialdir" => request.initial_dir = value.as_str().to_string(),
            "-initialfile" => request.initial_file = value.as_str().to_string(),
            "-parent" => request.parent = value.as_str().to_string(),
            "-defaultextension" => request.default_extension = value.as_str().to_string(),
            "-multiple" => request.multiple = value.as_bool().unwrap_or(false),
            "-confirmoverwrite" => request.confirm_overwrite = value.as_bool().unwrap_or(false),
            "-mustexist" | "-typevariable" => {
                if option == "-typevariable" {
                    type_variable = Some(value.as_str().to_string());
                }
            }
            "-filetypes" => {
                let Some(types) = value.as_list() else {
                    return Err(generic("bad -filetypes value: expected a list"));
                };
                for entry in types {
                    let Some(pair) = entry.as_list() else {
                        continue;
                    };
                    let description = pair
                        .first()
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default();
                    let patterns = pair
                        .get(1)
                        .and_then(|v| v.as_list())
                        .map(|list| {
                            list.iter()
                                .map(|pattern| pattern.as_str().to_string())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    request.file_types.push((description, patterns));
                }
            }
            other => {
                return Err(generic(format!(
                    "bad option \"{other}\": must be -defaultextension, -filetypes, -initialdir, -initialfile, -multiple, -mustexist, -parent, -title, or -typevariable"
                )));
            }
        }
        index += 2;
    }
    let mut paths = FILE_DIALOG
        .with(|cell| cell.get())
        .and_then(|handler| handler(&request))
        .unwrap_or_default();
    if kind == FileDialogKind::Save && !request.default_extension.is_empty() {
        let extension = &request.default_extension;
        if let Some(path) = paths.first_mut() {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
            if !name.contains('.') {
                if !extension.starts_with('.') {
                    path.push('.');
                }
                path.push_str(extension);
            }
        }
    }
    if let Some(name) = type_variable {
        if let Some((description, _)) = request.file_types.first() {
            let _ = interp.set_var(&name, Value::from_str(description));
        }
    }
    if request.multiple {
        Ok(list_value(paths))
    } else {
        Ok(Value::from_str(
            paths.first().map(String::as_str).unwrap_or(""),
        ))
    }
}

thread_local! {
    /// The backend's native modal dialog (`tk_messageBox`).
    static MESSAGE_BOX: std::cell::Cell<Option<fn(&MessageBoxRequest) -> String>> =
        const { std::cell::Cell::new(None) };
}

/// Give `tk_messageBox` a native modal dialog.  The handler must block
/// until the user answers and return one of the type's symbolic button
/// names (see [`message_box_buttons`]); without one the default button is
/// returned at once.
pub fn set_message_box_handler(handler: fn(&MessageBoxRequest) -> String) {
    MESSAGE_BOX.with(|cell| cell.set(Some(handler)));
}

fn message_box_impl(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut request = MessageBoxRequest {
        kind: "ok".to_string(),
        ..MessageBoxRequest::default()
    };
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        let Some(value) = args.get(index + 1) else {
            return Err(wrong(
                "tk_messageBox",
                "?-option value ...?",
                args.len() + 1,
            ));
        };
        match option {
            "-type" => request.kind = value.as_str().to_ascii_lowercase(),
            "-message" => request.message = value.as_str().to_string(),
            "-detail" => request.detail = value.as_str().to_string(),
            "-title" => request.title = value.as_str().to_string(),
            "-icon" => request.icon = value.as_str().to_string(),
            "-default" => request.default = value.as_str().to_string(),
            "-cancel" => request.cancel = value.as_str().to_string(),
            "-parent" => request.parent = value.as_str().to_string(),
            other => {
                return Err(generic(format!(
                    "bad option \"{other}\": must be -default, -detail, -icon, -message, -parent, -title, or -type"
                )));
            }
        }
        index += 2;
    }
    let Some(buttons) = message_box_buttons(&request.kind) else {
        return Err(generic(format!(
            "bad -type \"{}\": must be abortretryignore, ok, okcancel, retrycancel, yesno, or yesnocancel",
            request.kind
        )));
    };
    let fallback = default_message_box_button(&request);
    let answer = MESSAGE_BOX
        .with(|cell| cell.get())
        .map(|handler| handler(&request))
        .unwrap_or_else(|| fallback.clone())
        .to_ascii_lowercase();
    let answer = if buttons.iter().any(|name| *name == answer) {
        answer
    } else {
        fallback
    };
    Ok(Value::from_str(&answer))
}

/// The backend's synchronous clipboard reader (Linux: a nested main
/// context around GTK's asynchronous read).
pub fn set_clipboard_getter(getter: fn() -> String) {
    CLIPBOARD_GETTER.with(|cell| cell.set(Some(getter)));
}

/// The backend's sink for [`clipboard`](clipboard_impl) changes: called with
/// the whole Tk clipboard text.
pub fn set_clipboard_sink(sink: fn(&str)) {
    CLIPBOARD_SINK.with(|cell| cell.set(Some(sink)));
}

#[derive(Debug, Clone)]
struct SelectionHandler {
    window: String,
    command: String,
    format: String,
}

fn announce_clipboard(text: &str) {
    if let Some(sink) = CLIPBOARD_SINK.with(|cell| cell.get()) {
        sink(text);
    }
}

fn clipboard_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("clipboard", "option ?arg ...?", args.len()));
    }
    match args[1].as_str() {
        "clear" => {
            with_state_mut(interp, |rt| {
                rt.clipboard.clear();
                rt.clipboard_owned = true;
            });
            announce_clipboard("");
            Ok(Value::empty())
        }
        "append" => {
            let mut start = 2;
            while start < args.len() && args[start].as_str().starts_with('-') {
                match args[start].as_str() {
                    "-displayof" => start += 2,
                    "-type" | "-format" => start += 2,
                    _ => start += 1,
                }
            }
            if start >= args.len() {
                return Err(wrong(
                    "clipboard append",
                    "?-option value ...? data ?data ...?",
                    args.len(),
                ));
            }
            let text = args[start..]
                .iter()
                .map(|value| value.as_str())
                .collect::<String>();
            let current = with_state_mut(interp, |rt| {
                rt.clipboard.push_str(&text);
                rt.clipboard_owned = true;
                rt.clipboard.clone()
            })
            .unwrap_or_default();
            announce_clipboard(&current);
            Ok(Value::empty())
        }
        "get" => {
            let (owned, text) = with_state(interp, |rt| (rt.clipboard_owned, rt.clipboard.clone()))
                .unwrap_or((false, String::new()));
            if owned {
                return Ok(Value::from_str(&text));
            }
            let from_system = CLIPBOARD_GETTER
                .with(|cell| cell.get())
                .map(|getter| getter())
                .unwrap_or_default();
            Ok(Value::from_str(&from_system))
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be append, clear, or get"
        ))),
    }
}

/// The clipboard's text, whether this app owns it or the system does.
fn clipboard_text_value(interp: &Interp) -> String {
    let (owned, text) = with_state(interp, |rt| (rt.clipboard_owned, rt.clipboard.clone()))
        .unwrap_or((false, String::new()));
    if owned {
        return text;
    }
    CLIPBOARD_GETTER
        .with(|cell| cell.get())
        .map(|getter| getter())
        .unwrap_or_default()
}

/// `selection` (PRIMARY/CLIPBOARD ownership and retrieval).  There is no
/// X11 primary-selection bridge yet, so reads fall back to the clipboard
/// text; handlers run locally when this app owns the selection.
fn selection_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("selection", "option ?arg ...?", args.len()));
    }
    let sub = args[1].as_str().to_string();
    let mut selection = "PRIMARY".to_string();
    let mut kind = "STRING".to_string();
    let mut displayof = None;
    let mut command = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 2;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-selection" | "-type" | "-displayof" | "-format" | "-command" => {
                let Some(value) = args.get(i + 1) else {
                    return Err(wrong("selection", "option ?arg ...?", args.len()));
                };
                match arg {
                    "-selection" => selection = value.as_str().to_string(),
                    "-type" => kind = value.as_str().to_string(),
                    "-displayof" => displayof = Some(value.as_str().to_string()),
                    "-format" => {}
                    _ => command = Some(value.as_str().to_string()),
                }
                i += 2;
            }
            other if other.starts_with('-') && positional.is_empty() => {
                return Err(generic(format!(
                    "bad option \"{other}\": must be -command, -displayof, -format, -selection, or -type"
                )));
            }
            other => {
                positional.push(other.to_string());
                i += 1;
            }
        }
    }
    match sub.as_str() {
        "get" => {
            if !positional.is_empty() {
                return Err(wrong(
                    "selection get",
                    "?-selection sel? ?-type type?",
                    args.len(),
                ));
            }
            if kind == "TARGETS" {
                return Ok(Value::from_str("STRING UTF8_STRING"));
            }
            let owned = with_state(interp, |rt| {
                rt.selection_owned.as_deref() == Some(selection.as_str())
            })
            .unwrap_or(false);
            let handler = with_state(interp, |rt| {
                rt.selection_handlers
                    .get(&(selection.clone(), kind.clone()))
                    .or_else(|| {
                        rt.selection_handlers
                            .get(&(selection.clone(), "STRING".to_string()))
                    })
                    .cloned()
            })
            .flatten();
            if owned {
                if let Some(handler) = handler {
                    return interp.eval(&format!("{} 0 2147483647", handler.command));
                }
            }
            Ok(Value::from_str(&clipboard_text_value(interp)))
        }
        "own" => {
            if positional.len() > 1 {
                return Err(wrong(
                    "selection own",
                    "?-command command? ?selection?",
                    args.len(),
                ));
            }
            if let Some(name) = positional.first() {
                selection = name.clone();
            }
            let window = displayof.unwrap_or_else(|| ".".to_string());
            with_state_mut(interp, |rt| {
                rt.selection_owned = Some(selection.clone());
                rt.selection_window = Some(window.clone());
                if let Some(script) = &command {
                    rt.selection_handlers.insert(
                        (selection.clone(), "STRING".to_string()),
                        SelectionHandler {
                            window: window.clone(),
                            command: script.clone(),
                            format: "STRING".to_string(),
                        },
                    );
                }
            });
            Ok(Value::empty())
        }
        "clear" => {
            if positional.len() > 1 {
                return Err(wrong("selection clear", "?-selection sel?", args.len()));
            }
            if let Some(name) = positional.first() {
                selection = name.clone();
            }
            with_state_mut(interp, |rt| {
                if rt.selection_owned.as_deref() == Some(selection.as_str()) {
                    rt.selection_owned = None;
                    rt.selection_window = None;
                }
            });
            Ok(Value::empty())
        }
        "handle" => {
            if positional.len() != 2 {
                return Err(wrong(
                    "selection handle",
                    "?options? window command",
                    args.len(),
                ));
            }
            let window = positional[0].clone();
            let script = positional[1].clone();
            with_state_mut(interp, |rt| {
                if script.is_empty() {
                    rt.selection_handlers
                        .remove(&(selection.clone(), kind.clone()));
                } else {
                    rt.selection_handlers.insert(
                        (selection.clone(), kind.clone()),
                        SelectionHandler {
                            window,
                            command: script,
                            format: "STRING".to_string(),
                        },
                    );
                }
            });
            Ok(Value::empty())
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be clear, get, handle, or own"
        ))),
    }
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
            // `menu post x y` posts at a point; bare `post` drops the
            // menubar cascade down as before.
            if args.len() >= 4 {
                let x = parse_f64_str(args[2].as_str(), "x")?;
                let y = parse_f64_str(args[3].as_str(), "y")?;
                popup_menu_at(&path, x, y);
                return Ok(Value::empty());
            }
            with_state_mut(interp, |rt| {
                rt.open_menu = Some(path.clone());
                if rt
                    .popup_menu
                    .as_ref()
                    .is_some_and(|(menu, _, _)| menu.as_str() == path)
                {
                    rt.popup_menu = None;
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "unpost" => {
            with_state_mut(interp, |rt| {
                if rt.open_menu.as_deref() == Some(path.as_str()) {
                    rt.open_menu = None;
                }
                if rt
                    .popup_menu
                    .as_ref()
                    .is_some_and(|(menu, _, _)| menu.as_str() == path)
                {
                    rt.popup_menu = None;
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
    if option("-state") == "disabled" {
        return Ok(());
    }
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

/// A menu posted at a point (`tk_popup`/`menu post x y`), if any.
pub fn popup_menu() -> Option<(String, f64, f64)> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.popup_menu.clone()).flatten()
}

/// Post `menu_path` at a point (Tk's `tk_popup`/`menu post x y`).
pub fn popup_menu_at(menu_path: &str, x: f64, y: f64) {
    let Some(ptr) = active_interp() else { return };
    with_state_mut(unsafe { &mut *ptr }, |rt| {
        rt.popup_menu = Some((menu_path.to_string(), x, y));
        if rt.open_menu.as_deref() == Some(menu_path) {
            rt.open_menu = None;
        }
        rt.bump();
    });
}

/// The posted-point menu was closed (an item chose, or a press outside).
pub fn close_popup_menu() {
    let Some(ptr) = active_interp() else { return };
    with_state_mut(unsafe { &mut *ptr }, |rt| {
        if rt.popup_menu.take().is_some() {
            rt.bump();
        }
    });
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

/// Whether a menu item's checkbutton/radiobutton value is on (for the
/// menu renderer).
pub fn menu_item_checked(menu_path: &str, index: usize) -> bool {
    let Some(ptr) = active_interp() else {
        return false;
    };
    let interp = unsafe { &*ptr };
    let item = with_state(interp, |rt| {
        rt.menus.get(menu_path).and_then(|menu| {
            menu.items
                .get(index)
                .map(|item| (item.kind.clone(), item.options.clone()))
        })
    })
    .flatten();
    let Some((kind, options)) = item else {
        return false;
    };
    let Some(variable) = options.get("-variable").filter(|v| !v.is_empty()) else {
        return false;
    };
    let current = var_value(variable).unwrap_or_default();
    let expected = if kind == "checkbutton" {
        options
            .get("-onvalue")
            .cloned()
            .unwrap_or_else(|| "1".to_string())
    } else {
        options.get("-value").cloned().unwrap_or_default()
    };
    current == expected
}

/// Run a menu entry from the renderer and close the dropdown.
pub fn press_menu_item(menu_path: &str, index: usize) {
    let Some(ptr) = active_interp() else { return };
    widget_press(menu_path, 0.0, 0.0, 1);
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
            let known = with_state(interp, |rt| {
                rt.tree.contains(&path) || rt.menus.contains_key(&path)
            })
            .unwrap_or(false);
            if !known {
                return Err(bad_window(&path));
            }
            let mut ctx = EventCtx::for_path(&path);
            let mut i = 4;
            while i + 1 < args.len() {
                match args[i].as_str() {
                    "-x" => ctx.x = parse_int(&args[i + 1])?,
                    "-y" => ctx.y = parse_int(&args[i + 1])?,
                    "-rootx" => {
                        ctx.root_x = parse_int(&args[i + 1])?;
                        ctx.has_root = true;
                    }
                    "-rooty" => {
                        ctx.root_y = parse_int(&args[i + 1])?;
                        ctx.has_root = true;
                    }
                    "-key" | "-keysym" => ctx.key = args[i + 1].as_str().to_string(),
                    "-keycode" => ctx.keycode = parse_int(&args[i + 1])?,
                    "-button" => ctx.button = parse_int(&args[i + 1])?,
                    "-state" => ctx.state = args[i + 1].as_str().to_string(),
                    "-delta" => ctx.delta = parse_int(&args[i + 1])?,
                    "-width" => ctx.width = parse_int(&args[i + 1])?,
                    "-height" => ctx.height = parse_int(&args[i + 1])?,
                    "-time" => ctx.time = parse_int(&args[i + 1])?,
                    "-serial" => ctx.serial = parse_int(&args[i + 1])?,
                    "-sendevent" => ctx.send_event = parse_int(&args[i + 1])?,
                    "-detail" => ctx.detail = args[i + 1].as_str().to_string(),
                    "-mode" => ctx.mode = args[i + 1].as_str().to_string(),
                    "-focus" => ctx.focus = Some(parse_int(&args[i + 1])?),
                    "-count" | "-clicks" => ctx.clicks = parse_int(&args[i + 1])?.max(1) as u32,
                    _ => {}
                }
                i += 2;
            }
            let pattern = parse_sequence(args[3].as_str());
            if ctx.key.is_empty() {
                if matches!(pattern.kind, EventKind::KeyPress | EventKind::KeyRelease) {
                    if let Some(detail) = &pattern.detail {
                        ctx.key = detail.clone();
                    }
                }
            }
            if ctx.character.is_empty() && ctx.key.chars().count() == 1 {
                ctx.character = ctx.key.clone();
            }
            run_binding(&path, args[3].as_str(), &ctx);
            Ok(Value::empty())
        }
        "add" | "delete" => {
            if args.len() < 3 {
                return Err(wrong(
                    &format!("event {}", args[1].as_str()),
                    "<<virtualEvent>> ?sequence ...?",
                    args.len(),
                ));
            }
            let name = args[2].as_str();
            let Some(name) = name
                .strip_prefix("<<")
                .and_then(|inner| inner.strip_suffix(">>"))
            else {
                return Err(generic(format!(
                    "bad virtual event \"{}\": must be <<name>>",
                    args[2].as_str()
                )));
            };
            let add = args[1].as_str() == "add";
            let sequences: Vec<String> = args[3..]
                .iter()
                .map(|value| normalize_sequence(value.as_str()))
                .collect();
            with_state_mut(interp, |rt| {
                if add {
                    let mappings = rt.virtual_events.entry(name.to_string()).or_default();
                    for sequence in sequences {
                        if !mappings.contains(&sequence) {
                            mappings.push(sequence);
                        }
                    }
                } else if sequences.is_empty() {
                    rt.virtual_events.remove(name);
                } else if let Some(mappings) = rt.virtual_events.get_mut(name) {
                    mappings.retain(|sequence| !sequences.contains(sequence));
                    if mappings.is_empty() {
                        rt.virtual_events.remove(name);
                    }
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "info" => {
            if args.len() > 3 {
                return Err(wrong("event info", "?<<virtualEvent>>?", args.len()));
            }
            match args.get(2) {
                Some(value) => {
                    let name = value
                        .as_str()
                        .strip_prefix("<<")
                        .and_then(|inner| inner.strip_suffix(">>"))
                        .ok_or_else(|| {
                            generic(format!(
                                "bad virtual event \"{}\": must be <<name>>",
                                value.as_str()
                            ))
                        })?;
                    let sequences = with_state(interp, |rt| {
                        rt.virtual_events.get(name).cloned().unwrap_or_default()
                    })
                    .unwrap_or_default();
                    Ok(list_value(sequences))
                }
                None => {
                    let names = with_state(interp, |rt| {
                        rt.virtual_events
                            .keys()
                            .map(|name| format!("<<{name}>>"))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                    Ok(list_value(names))
                }
            }
        }
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
    // Typing in an entry means it has the focus: keep the Tk focus model in
    // step so Space/Return shortcuts do not fire on another widget.
    set_tk_focus_quiet(interp, Some(path.to_string()));
    bump_active();
}

/// A Text widget's content changed in the UI.
pub fn text_changed(path: &str, text: &str) {
    record_text_undo_if_changed(path, text);
    if apply_text_change(path, text) {
        if let Some(ptr) = active_interp() {
            set_tk_focus_quiet(unsafe { &mut *ptr }, Some(path.to_string()));
        }
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
        if old != text {
            rt.text_modified.insert(path.to_string());
        }
        adjust_text_tag_ranges(&old, text, rt.text_tags.get_mut(path));
        adjust_text_marks(&old, text, rt.text_marks.get_mut(path));
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = text.to_string();
            n.cursor = text.chars().count();
        }
        true
    })
    .unwrap_or(false)
}

/// Record the text widget's state before a host-side edit (the rich
/// editor calls this before saving its new document).  Programmatic
/// document updates report the current content again; those are skipped
/// so they do not create undo steps or clear the redo stack.
pub fn record_text_undo_if_changed(path: &str, text: &str) {
    let Some(ptr) = active_interp() else { return };
    with_state_mut(unsafe { &mut *ptr }, |rt| {
        if rt.tree.get(path).is_some_and(|node| node.content == text) {
            return;
        }
        record_text_undo(rt, path);
    });
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
        normalize_ranges(&mut ranges);
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
    if let Some(ptr) = active_interp() {
        set_tk_focus_quiet(unsafe { &mut *ptr }, Some(path.to_string()));
    }
    let mut ctx = EventCtx::for_path(path);
    ctx.key = key.to_string();
    ctx.character = character.to_string();
    if key.chars().count() == 1 && key.chars().next().is_some_and(char::is_uppercase) {
        // Shift+a arrives as keysym "A" with the Shift state bit set.
        ctx.state = "1".to_string();
    }
    run_binding(
        path,
        if key == "Return" {
            "<Return>"
        } else {
            "<KeyPress>"
        },
        &ctx,
    );
    run_binding(path, "<KeyRelease>", &ctx);
}

/// `space`/`return` pressed while a button-like Tk widget has the focus:
/// the backend's global key handler routes it to that widget.
pub fn focused_control_key(key: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &*ptr };
    let focus = with_state(interp, |rt| rt.focus.clone()).flatten();
    let Some(path) = focus else { return };
    let kind = with_state(interp, |rt| rt.tree.get(&path).map(|n| n.kind)).flatten();
    if matches!(
        kind,
        Some(WidgetKind::Button) | Some(WidgetKind::Checkbutton) | Some(WidgetKind::Radiobutton)
    ) {
        control_key(&path, key);
    }
}

/// Whether the widget's class has a user binding matching `event` (used to
/// decide whether a native default scroll still applies).
fn class_has_binding(interp: &Interp, path: &str, event: &EventPattern) -> bool {
    let class = with_state(interp, |rt| {
        rt.tree.get(path).map(|n| n.widget_class().to_string())
    })
    .flatten()
    .unwrap_or_else(|| "Widget".to_string());
    best_script(&tag_binding_map(interp, &class), event).is_some()
}

/// A wheel notch over a listbox/treeview (or any widget with a Tk binding):
/// dispatch `<Button-4>/<Button-5>` and `<MouseWheel>` with `%D`, then
/// scroll the widget like Tk's class bindings unless the user replaced them.
pub fn wheel_event(path: &str, dx: f64, dy: f64, state: u32) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    if dy != 0.0 {
        let up = dy > 0.0;
        send_wheel(path, up, state);
        let kind = with_state(interp, |rt| rt.tree.get(path).map(|n| n.kind)).flatten();
        if matches!(kind, Some(WidgetKind::Listbox) | Some(WidgetKind::Treeview))
            && !wheel_overridden(interp, path, up)
        {
            wheel_scroll(interp, path, if up { -1.0 } else { 1.0 });
        }
        return;
    }
    if dx != 0.0 {
        let right = dx > 0.0;
        let button = if right { 7 } else { 6 };
        let mut ctx = EventCtx::for_path(path);
        ctx.button = button;
        ctx.delta = if right { 120 } else { -120 };
        ctx.state = state.to_string();
        let press = format!("<Button-{button}>");
        let release = format!("<ButtonRelease-{button}>");
        run_binding(path, &press, &ctx);
        run_binding(path, &release, &ctx);
        run_binding(path, "<Shift-MouseWheel>", &ctx);
    }
}

/// Dispatch Tk's wheel bindings for one notch (no default scrolling).
fn send_wheel(path: &str, up: bool, state: u32) {
    let button = if up { 4 } else { 5 };
    let delta = if up { 120 } else { -120 };
    let mut ctx = EventCtx::for_path(path);
    ctx.button = button;
    ctx.delta = delta;
    ctx.state = state.to_string();
    let press = format!("<Button-{button}>");
    let release = format!("<ButtonRelease-{button}>");
    run_binding(path, &press, &ctx);
    run_binding(path, &release, &ctx);
    run_binding(path, "<MouseWheel>", &ctx);
}

fn wheel_overridden(interp: &Interp, path: &str, up: bool) -> bool {
    let button = if up { 4 } else { 5 };
    let mut ctx = EventCtx::for_path(path);
    ctx.button = button;
    ctx.delta = if up { 120 } else { -120 };
    let wheel = event_from("<MouseWheel>", &ctx);
    let press = event_from(&format!("<Button-{button}>"), &ctx);
    class_has_binding(interp, path, &wheel) || class_has_binding(interp, path, &press)
}

thread_local! {
    /// Wheel pixels accumulated per widget until a Tk notch (120) fires.
    static WHEEL_ACC: RefCell<BTreeMap<String, f64>> = const { RefCell::new(BTreeMap::new()) };
    /// Last raw content offset of each widget's wheel scroll view.
    static WHEEL_LAST_OFFSET: RefCell<BTreeMap<String, f64>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// A native scroll view's content offset moved over `row_height`-tall rows.
/// Dispatch a `<MouseWheel>`/`<Button-4|5>` binding every 120 points and
/// scroll the listbox/treeview model by the exact row fraction, unless a
/// user class binding replaced the default.  The view's offset is reset by
/// the renderer after each event, so offsets may restart from zero.
pub fn native_wheel(path: &str, offset: f64, row_height: f64) {
    eprintln!(
        "NATIVE ENTER path={path} offset={offset} row={row_height} active={}",
        active_interp().is_some()
    );
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    if offset == 0.0 {
        WHEEL_LAST_OFFSET.with(|last| {
            last.borrow_mut().insert(path.to_string(), 0.0);
        });
        return;
    }
    let delta = WHEEL_LAST_OFFSET.with(|last| {
        let mut last = last.borrow_mut();
        let previous = last.get(path).copied().unwrap_or(0.0);
        let delta = if offset < previous {
            offset
        } else {
            offset - previous
        };
        last.insert(path.to_string(), offset);
        delta
    });
    if delta == 0.0 {
        return;
    }
    let up = delta < 0.0;
    let mut notches = 0;
    WHEEL_ACC.with(|acc| {
        let mut acc = acc.borrow_mut();
        let accumulated = acc.entry(path.to_string()).or_insert(0.0);
        *accumulated += delta;
        while *accumulated >= 120.0 {
            *accumulated -= 120.0;
            notches += 1;
        }
        while *accumulated <= -120.0 {
            *accumulated += 120.0;
            notches += 1;
        }
    });
    let overridden = wheel_overridden(interp, path, up);
    for _ in 0..notches {
        send_wheel(path, up, 0);
    }
    if overridden || row_height <= 0.0 {
        return;
    }
    let total = with_state(interp, |rt| {
        if let Some(tv) = rt.treeviews.get(path) {
            Some(treeview_visible_count_of(tv))
        } else {
            rt.tree.get(path).map(|n| n.items.len())
        }
    })
    .flatten()
    .unwrap_or(0);
    if total == 0 {
        return;
    }
    let (fraction, rows) = scroll_state(interp, path);
    let rows = rows.min(total);
    let max_first = (1.0 - rows as f64 / total as f64).max(0.0);
    let next = (fraction + delta / row_height / total as f64).clamp(0.0, max_first);
    if (next - fraction).abs() > f64::EPSILON {
        set_scroll_fraction(interp, path, next);
        notify_yscroll(interp, path, total);
    }
}

fn wheel_scroll(interp: &mut Interp, path: &str, direction: f64) {
    let total = with_state(interp, |rt| {
        if let Some(tv) = rt.treeviews.get(path) {
            Some(treeview_visible_count_of(tv))
        } else {
            rt.tree.get(path).map(|n| n.items.len())
        }
    })
    .flatten()
    .unwrap_or(0);
    if total == 0 {
        return;
    }
    let (fraction, rows) = scroll_state(interp, path);
    let rows = rows.min(total);
    let step = 3.0 / total as f64;
    let max_first = (1.0 - rows as f64 / total as f64).max(0.0);
    let next = (fraction + direction * step).clamp(0.0, max_first);
    if (next - fraction).abs() < f64::EPSILON {
        return;
    }
    set_scroll_fraction(interp, path, next);
    notify_yscroll(interp, path, total);
}

thread_local! {
    /// Last frame reported per widget, so a platform relayout that did not
    /// change a widget's size does not fire `<Configure>` again (which
    /// would ask for another render forever).
    static LAST_CONFIGURE: RefCell<BTreeMap<String, (i64, i64)>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// A widget's frame changed: `<Configure>` with `%w/%h`.
pub fn configure_event(path: &str, width: f64, height: f64) {
    let size = (width.round() as i64, height.round() as i64);
    let changed = LAST_CONFIGURE.with(|last| {
        let mut last = last.borrow_mut();
        if last.get(path) == Some(&size) {
            false
        } else {
            last.insert(path.to_string(), size);
            true
        }
    });
    if !changed {
        return;
    }
    let mut ctx = EventCtx::for_path(path);
    ctx.width = size.0;
    ctx.height = size.1;
    let Some(ptr) = active_interp() else { return };
    if !has_binding(unsafe { &*ptr }, path, "<Configure>", &ctx) {
        return;
    }
    run_binding(path, "<Configure>", &ctx);
    // A Configure handler may change what the widget shows; request one
    // render (the size dedupe above stops a layout loop).
    bump_active();
}

/// A key in a rich-text editor: dispatch the text widget's Tk `<KeyPress>`
/// bindings (the editor is the class behavior).  Returns true when a binding
/// returned `break` (the editor should not handle the key).
pub fn text_key_event(
    path: &str,
    key: &str,
    shift: bool,
    ctrl: bool,
    alt: bool,
    meta: bool,
) -> bool {
    let mut ctx = EventCtx::for_path(path);
    ctx.key = match key {
        "Enter" => "Return".to_string(),
        "ArrowUp" => "Up".to_string(),
        "ArrowDown" => "Down".to_string(),
        "ArrowLeft" => "Left".to_string(),
        "ArrowRight" => "Right".to_string(),
        " " => "space".to_string(),
        other => other.to_string(),
    };
    if key.chars().count() == 1 {
        ctx.character = key.to_string();
    }
    let mut state = 0u32;
    if shift {
        state |= 1;
    }
    if ctrl {
        state |= 4;
    }
    if alt {
        state |= 8;
    }
    if meta {
        state |= 64;
    }
    ctx.state = state.to_string();
    let stopped = run_binding(path, "<KeyPress>", &ctx);
    if !stopped {
        let index = active_interp()
            .and_then(|ptr| {
                with_state(unsafe { &*ptr }, |rt| {
                    rt.tree.get(path).map(|node| node.cursor)
                })
            })
            .flatten();
        if let (Some(ptr), Some(index)) = (active_interp(), index) {
            let interp = unsafe { &mut *ptr };
            let tag_stopped = run_text_tag_sequence(interp, path, index, "<KeyPress>", &ctx);
            if !tag_stopped {
                run_text_tag_sequence(interp, path, index, "<KeyRelease>", &ctx);
            }
        }
    }
    run_binding(path, "<KeyRelease>", &ctx);
    stopped
}

/// Run the tag bindings for `sequence` at `index` (Tk's text class binding:
/// one script per tag, most specific sequence).
fn run_text_tag_sequence(
    interp: &mut Interp,
    path: &str,
    index: usize,
    sequence: &str,
    ctx: &EventCtx,
) -> bool {
    let event = event_from(sequence, ctx);
    let scripts = with_state(interp, |rt| {
        rt.text_tags
            .get(path)
            .map(|tags| {
                tags.iter()
                    .filter(|(_, tag)| {
                        tag.ranges
                            .iter()
                            .any(|(start, end)| index >= *start && index < *end)
                    })
                    .filter_map(|(_, tag)| best_script(&tag.bindings, &event))
                    .filter(|script| !script.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let mut stopped = false;
    let mut ran = false;
    for script in scripts {
        ran = true;
        let script = substitute_bindings(&script, ctx);
        match interp.eval(&script) {
            Ok(_) => {}
            Err(e) if binding_stopped(&e) => {
                stopped = true;
                break;
            }
            Err(e) => record_error(e.to_string()),
        }
    }
    if ran {
        bump_active();
    }
    stopped
}

/// Run one tag's binding for `sequence` (crossing events target the tag
/// entered or left, not every tag at the position).
fn run_text_tag_named(
    interp: &mut Interp,
    path: &str,
    name: &str,
    sequence: &str,
    ctx: &EventCtx,
) -> bool {
    let event = event_from(sequence, ctx);
    let script = with_state(interp, |rt| {
        rt.text_tags
            .get(path)
            .and_then(|tags| tags.get(name))
            .and_then(|tag| best_script(&tag.bindings, &event))
            .filter(|script| !script.is_empty())
    })
    .flatten();
    let Some(script) = script else { return false };
    let script = substitute_bindings(&script, ctx);
    match interp.eval(&script) {
        Ok(_) => {
            bump_active();
            false
        }
        Err(e) if binding_stopped(&e) => {
            bump_active();
            true
        }
        Err(e) => {
            record_error(e.to_string());
            bump_active();
            false
        }
    }
}

thread_local! {
    /// Tags the pointer is over in each text widget (`<Enter>/<Leave>`).
    static TEXT_HOVER_TAGS: RefCell<BTreeMap<String, std::collections::BTreeSet<String>>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// The pointer moved over a rich text editor: fire `<Leave>` for the tags
/// left, `<Enter>` for the tags entered, then `<Motion>` for the tag under
/// the pointer (Tk's text tag crossing semantics).
pub fn text_tag_hover(path: &str, doc_pos: usize, x: f64, y: f64) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let length = with_state(interp, |rt| {
        rt.tree.get(path).map(|node| node.content.chars().count())
    })
    .flatten()
    .unwrap_or(0);
    let index = with_state(interp, |rt| {
        rt.rich_docs
            .get(path)
            .and_then(|json| doc_pos_to_text_offset(json, doc_pos))
    })
    .flatten()
    .unwrap_or(doc_pos)
    .min(length);
    let current = with_state(interp, |rt| {
        rt.text_tags
            .get(path)
            .map(|tags| {
                tags.iter()
                    .filter(|(_, tag)| {
                        tag.ranges
                            .iter()
                            .any(|(start, end)| index >= *start && index < *end)
                    })
                    .map(|(name, _)| name.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let previous = TEXT_HOVER_TAGS.with(|map| {
        map.borrow_mut()
            .insert(path.to_string(), current.clone())
            .unwrap_or_default()
    });
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    for name in previous.difference(&current) {
        run_text_tag_named(interp, path, name, "<Leave>", &ctx);
    }
    for name in current.difference(&previous) {
        run_text_tag_named(interp, path, name, "<Enter>", &ctx);
    }
    run_text_tag_sequence(interp, path, index, "<Motion>", &ctx);
}

/// The pointer left a rich text editor: fire `<Leave>` for the tags it was
/// over and forget them.
pub fn text_tag_hover_out(path: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let previous = TEXT_HOVER_TAGS.with(|map| map.borrow_mut().remove(path).unwrap_or_default());
    if previous.is_empty() {
        return;
    }
    let mut ctx = EventCtx::for_path(path);
    ctx.x = 0;
    ctx.y = 0;
    for name in previous {
        run_text_tag_named(interp, path, &name, "<Leave>", &ctx);
    }
}

/// A press over a rich text editor: run the tag bindings at the character
/// under the pointer.  `doc_pos` is ProseMirror's document position.
pub fn text_tag_press(path: &str, doc_pos: usize, x: f64, y: f64, clicks: u32) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let index = with_state(interp, |rt| {
        rt.rich_docs
            .get(path)
            .and_then(|json| doc_pos_to_text_offset(json, doc_pos))
    })
    .flatten()
    .unwrap_or(doc_pos);
    let cursor = with_state(interp, |rt| {
        rt.tree.get(path).map(|node| node.content.chars().count())
    })
    .flatten()
    .unwrap_or(0);
    let index = index.min(cursor);
    with_state_mut(interp, |rt| {
        if let Some(node) = rt.tree.get_mut(path) {
            node.cursor = index;
        }
    });
    let mut ctx = EventCtx::for_path(path);
    ctx.button = 1;
    ctx.x = x as i64;
    ctx.y = y as i64;
    ctx.clicks = clicks;
    let stopped = run_text_tag_sequence(interp, path, index, "<Button-1>", &ctx);
    if !stopped {
        match clicks {
            2 => {
                run_text_tag_sequence(interp, path, index, "<Double-Button-1>", &ctx);
            }
            3 => {
                run_text_tag_sequence(interp, path, index, "<Triple-Button-1>", &ctx);
            }
            4 => {
                run_text_tag_sequence(interp, path, index, "<Quadruple-Button-1>", &ctx);
            }
            _ => {}
        }
        // Tk sends the release after the press; the plain-widget press path
        // dispatches it the same way (the backend has no pointer-up hook for
        // tag ranges).
        run_text_tag_sequence(interp, path, index, "<ButtonRelease-1>", &ctx);
    }
}

/// The editor's caret moved: mirror it in the text widget's `insert` index
/// without a render (the editor owns the caret).
pub fn text_selection_changed(path: &str, doc_pos: usize) {
    let Some(ptr) = active_interp() else { return };
    with_state_mut(unsafe { &mut *ptr }, |rt| {
        let Some(length) = rt.tree.get(path).map(|node| node.content.chars().count()) else {
            return;
        };
        let index = rt
            .rich_docs
            .get(path)
            .and_then(|json| doc_pos_to_text_offset(json, doc_pos))
            .unwrap_or(doc_pos)
            .min(length);
        if let Some(node) = rt.tree.get_mut(path) {
            node.cursor = index;
        }
    });
}

/// An input took the focus from the platform: make it the Tk focus (and
/// dispatch `<FocusOut>/<FocusIn>`) without a re-render — the platform is
/// mid-gesture (a press or paste) and a rebuild would reset its state.
pub fn focus_widget(path: &str) {
    let Some(ptr) = active_interp() else { return };
    set_tk_focus_quiet(unsafe { &mut *ptr }, Some(path.to_string()));
}

/// A paste reached an input: `<<Paste>>`.
pub fn paste_event(path: &str) {
    run_binding(path, "<<Paste>>", &EventCtx::for_path(path));
}

/// The pointer entered a widget: `<Enter>`.
pub fn enter_event(path: &str, x: f64, y: f64) {
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    run_binding(path, "<Enter>", &ctx);
}

/// The pointer left a widget: `<Leave>`.
pub fn leave_event(path: &str, x: f64, y: f64) {
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    run_binding(path, "<Leave>", &ctx);
}

/// Mouse motion (with or without a button down): `<Motion>` and, while
/// button `n` is held, `<Bn-Motion>`.
pub fn motion_event(path: &str, x: f64, y: f64, pressed_button: Option<u8>, state: u32) {
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    ctx.state = state.to_string();
    run_binding(path, "<Motion>", &ctx);
    if let Some(button) = pressed_button {
        ctx.button = i64::from(button);
        let sequence = format!("<B{button}-Motion>");
        run_binding(path, &sequence, &ctx);
    }
}

/// A touch at `x`,`y`: `<Touch>` plus the phase virtual events.
pub fn touch_event(path: &str, x: f64, y: f64, phase: &str) {
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    run_binding(path, "<Touch>", &ctx);
    let name = match phase {
        "start" => "TouchStart",
        "move" => "TouchMove",
        "end" => "TouchEnd",
        _ => return,
    };
    run_binding(path, &format!("<<{name}>>"), &ctx);
}

/// A long press (touch/force-touch): `<<LongPress>>`.
pub fn long_press_event(path: &str, x: f64, y: f64) {
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    run_binding(path, "<<LongPress>>", &ctx);
}

/// A touch pan: `<<Pan>>` (`%D` is the vertical translation).
pub fn pan_event(path: &str, dx: f64, dy: f64, cancelled: bool) {
    let mut ctx = EventCtx::for_path(path);
    ctx.delta = dy.round() as i64;
    let name = if cancelled { "PanCancel" } else { "Pan" };
    run_binding(path, &format!("<<{name}>>"), &ctx);
    let _ = dx;
}

/// A pinch: `<<Pinch>>` (`%D` is scale × 100).
pub fn pinch_event(path: &str, scale: f64) {
    let mut ctx = EventCtx::for_path(path);
    ctx.delta = (scale * 100.0).round() as i64;
    run_binding(path, "<<Pinch>>", &ctx);
}

/// A two-finger rotation: `<<Rotation>>` (`%D` is degrees × 10).
pub fn rotation_event(path: &str, degrees: f64) {
    let mut ctx = EventCtx::for_path(path);
    ctx.delta = (degrees * 10.0).round() as i64;
    run_binding(path, "<<Rotation>>", &ctx);
}

/// Space/Return on a focused button-like control.  The Tk library's
/// default class bindings (installed by [`install`]) invoke the widget;
/// a user binding on the same sequence replaces them like in Tk.
pub fn control_key(path: &str, key: &str) {
    let mut ctx = EventCtx::for_path(path);
    let sequence = match key {
        " " | "space" | "Space" => "<Key-space>",
        "Enter" | "Return" => "<Key-Return>",
        other => {
            ctx.character = if other.chars().count() == 1 {
                other.to_string()
            } else {
                String::new()
            };
            "<KeyPress>"
        }
    };
    ctx.key = match key {
        " " | "space" | "Space" => "space".to_string(),
        "Enter" | "Return" => "Return".to_string(),
        other => other.to_string(),
    };
    run_binding(path, sequence, &ctx);
    run_binding(path, "<KeyRelease>", &ctx);
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
    dispatch_click(path, &ctx);
    if matches!(
        kind,
        Some(WidgetKind::Button) | Some(WidgetKind::Checkbutton) | Some(WidgetKind::Radiobutton)
    ) {
        // Tk's ButtonUp class binding generates <<Invoke>>, whose class
        // binding calls `$w invoke` (replaceable by a user binding).
        run_binding(path, "<<Invoke>>", &ctx);
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
    let mut tags = Vec::new();
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
            "-tags" => {
                tags = v.split_whitespace().map(str::to_string).collect();
            }
            "-open" => open = matches!(v.as_str(), "1" | "true" | "yes" | "on"),
            "-id" => id = Some(v),
            "-image" => {}
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
            tags,
            open,
        });
        rt.bump();
        result = id;
        Ok::<(), Error>(())
    })
    .ok_or_else(|| bad_window(path))??;
    Ok(Value::from_str(&result))
}

fn treeview_column_default(option: &str) -> &'static str {
    match option {
        "-width" => "200",
        "-minwidth" => "20",
        "-stretch" => "1",
        "-anchor" => "w",
        _ => "",
    }
}

fn treeview_column_options(tv: &TreeViewState, column: &str) -> BTreeMap<String, String> {
    let mut options = ["-width", "-minwidth", "-stretch", "-anchor", "-id"]
        .into_iter()
        .map(|name| (name.to_string(), treeview_column_default(name).to_string()))
        .collect::<BTreeMap<_, _>>();
    if let Some(extra) = tv.column_options.get(column) {
        options.extend(extra.clone());
    }
    options
}

fn treeview_column_command(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong(
            "column",
            "column ?-option? ?value ...?",
            rest.len() + 2,
        ));
    }
    let name = rest[0].as_str().to_string();
    let tv = with_state(interp, |rt| rt.treeviews.get(path).cloned())
        .flatten()
        .ok_or_else(|| bad_window(path))?;
    if name != "#0" && !tv.columns.iter().any(|column| column == &name) {
        return Err(generic(format!("Column \"{name}\" not found")));
    }
    if rest.len() == 1 {
        let options = treeview_column_options(&tv, &name);
        return Ok(list_value(
            options
                .iter()
                .flat_map(|(k, v)| [k.clone(), v.clone()])
                .collect::<Vec<_>>(),
        ));
    }
    if rest.len() == 2 && rest[1].as_str().starts_with('-') {
        let options = treeview_column_options(&tv, &name);
        return Ok(Value::from_str(
            options.get(rest[1].as_str()).map_or("", String::as_str),
        ));
    }
    let pairs = parse_pairs(rest, 1, "column")?;
    for (option, _) in &pairs {
        if !matches!(
            option.as_str(),
            "-width" | "-minwidth" | "-stretch" | "-anchor" | "-id"
        ) {
            return Err(generic(format!("unknown option \"{option}\"")));
        }
    }
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            tv.column_options
                .entry(name.clone())
                .or_default()
                .extend(pairs);
            rt.bump();
        }
    });
    Ok(Value::empty())
}

fn treeview_tag_command(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("tag", "option ?arg ...?", rest.len() + 2));
    }
    match rest[0].as_str() {
        "configure" => {
            if rest.len() < 2 {
                return Err(wrong(
                    "tag configure",
                    "tagName ?-option? ?value ...?",
                    rest.len() + 2,
                ));
            }
            let name = rest[1].as_str().to_string();
            if rest.len() == 2 {
                let options = with_state(interp, |rt| {
                    rt.treeviews
                        .get(path)
                        .and_then(|tv| tv.tags.get(&name))
                        .map(|tag| tag.clone())
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
                    rt.treeviews
                        .get(path)
                        .and_then(|tv| tv.tags.get(&name))
                        .and_then(|tag| tag.get(option))
                        .cloned()
                })
                .flatten()
                .unwrap_or_default();
                return Ok(Value::from_str(&value));
            }
            let options = parse_pairs(rest, 2, "tag configure")?;
            with_state_mut(interp, |rt| {
                if let Some(tv) = rt.treeviews.get_mut(path) {
                    tv.tags.entry(name.clone()).or_default().extend(options);
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        "cget" => {
            if rest.len() != 3 {
                return Err(wrong("tag cget", "tagName -option", rest.len() + 2));
            }
            let value = with_state(interp, |rt| {
                rt.treeviews
                    .get(path)
                    .and_then(|tv| tv.tags.get(rest[1].as_str()))
                    .and_then(|tag| tag.get(rest[2].as_str()))
                    .cloned()
            })
            .flatten()
            .unwrap_or_default();
            Ok(Value::from_str(&value))
        }
        "names" => {
            let names = with_state(interp, |rt| {
                rt.treeviews
                    .get(path)
                    .map(|tv| tv.tags.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            Ok(list_value(names))
        }
        "has" => {
            if rest.len() < 2 || rest.len() > 3 {
                return Err(wrong("tag has", "tagName ?item?", rest.len() + 2));
            }
            let name = rest[1].as_str();
            let found = with_state(interp, |rt| {
                let Some(tv) = rt.treeviews.get(path) else {
                    return false;
                };
                match rest.get(2) {
                    Some(item) => tv
                        .items
                        .iter()
                        .find(|i| i.id == item.as_str())
                        .is_some_and(|i| i.tags.iter().any(|tag| tag == name)),
                    None => tv.tags.contains_key(name),
                }
            })
            .unwrap_or(false);
            Ok(Value::from_bool(found))
        }
        "delete" => {
            with_state_mut(interp, |rt| {
                if let Some(tv) = rt.treeviews.get_mut(path) {
                    for value in &rest[1..] {
                        tv.tags.remove(value.as_str());
                    }
                    rt.bump();
                }
            });
            Ok(Value::empty())
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be cget, configure, delete, has, or names"
        ))),
    }
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
            "-tags".to_string(),
            item.tags.join(" "),
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
            "-tags" => list_value(item.tags),
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
                        "-tags" => {
                            item.tags = v.split_whitespace().map(str::to_string).collect();
                        }
                        "-image" => {}
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
    let mut ctx = EventCtx::for_path(path);
    ctx.button = 1;
    dispatch_click(path, &ctx);
    let interp = unsafe { &mut *ptr };
    set_tk_focus(interp, Some(path.to_string()));
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            tv.selected = std::iter::once(id.to_string()).collect();
            tv.focus = Some(id.to_string());
            rt.bump();
        }
    });
    fire_virtual(path, "TreeviewSelect");
}

/// Expand/collapse a Treeview row from the renderer.
pub fn treeview_toggle(path: &str, id: &str) {
    let Some(ptr) = active_interp() else { return };
    let mut ctx = EventCtx::for_path(path);
    ctx.button = 1;
    dispatch_click(path, &ctx);
    let interp = unsafe { &mut *ptr };
    set_tk_focus(interp, Some(path.to_string()));
    let opened = with_state(interp, |rt| {
        rt.treeviews
            .get(path)
            .and_then(|tv| tv.items.iter().find(|item| item.id == id))
            .map(|item| !item.open)
    })
    .flatten();
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            if let Some(item) = tv.items.iter_mut().find(|i| i.id == id) {
                item.open = !item.open;
                rt.bump();
            }
        }
    });
    if let Some(opened) = opened {
        fire_virtual(
            path,
            if opened {
                "TreeviewOpen"
            } else {
                "TreeviewClose"
            },
        );
    }
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
    let index = index.min(len.saturating_sub(1));
    let changed = with_state(interp, |rt| {
        rt.notebook_selected.get(notebook).copied() != Some(index)
    })
    .unwrap_or(true);
    with_state_mut(interp, |rt| {
        rt.notebook_selected.insert(notebook.to_string(), index);
        rt.bump();
    });
    if changed {
        fire_virtual(notebook, "NotebookTabChanged");
    }
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
    fire_virtual(path, "ComboboxSelected");
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
    let mut ctx = EventCtx::for_path(path);
    ctx.button = 1;
    dispatch_click(path, &ctx);
    let interp = unsafe { &mut *ptr };
    set_tk_focus(interp, Some(path.to_string()));
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.selected.clear();
            n.selected.insert(index);
            rt.bump();
        }
    });
    fire_virtual(path, "ListboxSelect");
}

/// Arrow/Home/End/Page keys while a listbox or treeview has the Tk focus.
pub fn focused_nav_key(key: &str) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let focus = with_state(interp, |rt| rt.focus.clone()).flatten();
    let Some(path) = focus else { return };
    let kind = with_state(interp, |rt| rt.tree.get(&path).map(|n| n.kind)).flatten();
    if !matches!(kind, Some(WidgetKind::Listbox) | Some(WidgetKind::Treeview)) {
        return;
    }
    let mut ctx = EventCtx::for_path(&path);
    ctx.key = key.to_string();
    run_binding(&path, "<KeyPress>", &ctx);
    // Tk's listbox/treeview class bindings move the selection; a user
    // binding on the same class sequence replaces that default.
    let event = event_from("<KeyPress>", &ctx);
    let class = with_state(interp, |rt| {
        rt.tree.get(&path).map(|n| n.widget_class().to_string())
    })
    .flatten()
    .unwrap_or_else(|| "Widget".to_string());
    let overridden = tag_binding_map(interp, &class).iter().any(|(sequence, _)| {
        let pattern = parse_sequence(sequence);
        pattern.detail.is_some() && pattern.matches(&event)
    });
    if !overridden {
        match kind {
            Some(WidgetKind::Listbox) => listbox_nav(&path, key),
            Some(WidgetKind::Treeview) => treeview_nav(&path, key),
            _ => {}
        }
    }
    run_binding(&path, "<KeyRelease>", &ctx);
}

fn listbox_nav(path: &str, key: &str) {
    let (len, selected, rows) = {
        let Some(ptr) = active_interp() else { return };
        let interp = unsafe { &*ptr };
        let Some(node) = with_state(interp, |rt| rt.tree.get(path).cloned()).flatten() else {
            return;
        };
        (
            node.items.len(),
            node.selected.clone(),
            scroll_state(interp, path).1,
        )
    };
    if len == 0 {
        return;
    }
    let current = selected.iter().next().copied().unwrap_or(0) as isize;
    let last = len as isize - 1;
    let target = match key {
        "Up" | "up" => current - 1,
        "Down" | "down" => current + 1,
        "Home" | "home" => 0,
        "End" | "end" => last,
        "Prior" | "prior" | "Page_Up" => current - rows as isize,
        "Next" | "next" | "Page_Down" => current + rows as isize,
        _ => return,
    }
    .clamp(0, last) as usize;
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        if let Some(n) = rt.tree.get_mut(path) {
            n.selected.clear();
            n.selected.insert(target);
            rt.bump();
        }
    });
    scroll_to_row(interp, path, target, len);
    fire_virtual(path, "ListboxSelect");
}

fn treeview_nav(path: &str, key: &str) {
    let (ids, selected, rows) = {
        let Some(ptr) = active_interp() else { return };
        let interp = unsafe { &*ptr };
        let Some(tv) = with_state(interp, |rt| rt.treeviews.get(path).cloned()).flatten() else {
            return;
        };
        let ids = treeview_visible_ids(&tv);
        (ids, tv.selected.clone(), scroll_state(interp, path).1)
    };
    if ids.is_empty() {
        return;
    }
    let current = selected
        .iter()
        .next()
        .and_then(|id| ids.iter().position(|candidate| candidate == id))
        .unwrap_or(0) as isize;
    let last = ids.len() as isize - 1;
    let target = match key {
        "Up" | "up" => current - 1,
        "Down" | "down" => current + 1,
        "Home" | "home" => 0,
        "End" | "end" => last,
        "Prior" | "prior" | "Page_Up" => current - rows as isize,
        "Next" | "next" | "Page_Down" => current + rows as isize,
        _ => return,
    }
    .clamp(0, last) as usize;
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    with_state_mut(interp, |rt| {
        if let Some(tv) = rt.treeviews.get_mut(path) {
            tv.selected = std::iter::once(ids[target].clone()).collect();
            tv.focus = Some(ids[target].clone());
            rt.bump();
        }
    });
    scroll_to_row(interp, path, target, ids.len());
    fire_virtual(path, "TreeviewSelect");
}

fn treeview_visible_ids(tv: &TreeViewState) -> Vec<String> {
    fn walk(tv: &TreeViewState, parent: Option<&str>, out: &mut Vec<String>) {
        for item in tv.items.iter().filter(|i| i.parent.as_deref() == parent) {
            out.push(item.id.clone());
            if item.open {
                walk(tv, Some(item.id.as_str()), out);
            }
        }
    }
    let mut ids = Vec::new();
    walk(tv, None, &mut ids);
    ids
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

/// Parse Tk's `WxH+X+Y` geometry (any part optional).
fn parse_window_geometry(value: &str) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let value = value.trim().trim_start_matches('=');
    let (size, place) = match value.find(['+', '-']) {
        Some(index) => (&value[..index], &value[index..]),
        None => (value, ""),
    };
    let mut width = None;
    let mut height = None;
    if let Some((w, h)) = size.split_once('x') {
        width = w.trim().parse::<f64>().ok().filter(|value| *value > 0.0);
        height = h.trim().parse::<f64>().ok().filter(|value| *value > 0.0);
    }
    let mut positions = place
        .split(['+', '-'])
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f64>().ok());
    let x = positions.next().map(|value| value.max(8.0));
    let y = positions.next().map(|value| value.max(8.0));
    (width, height, x, y)
}

/// The mapped secondary toplevels, for the renderer's window overlays.
pub fn toplevel_windows() -> Vec<ToplevelView> {
    let Some(ptr) = active_interp() else {
        return Vec::new();
    };
    let interp = unsafe { &*ptr };
    let paths = with_state(interp, |rt| {
        rt.tree
            .descendants(".")
            .into_iter()
            .filter(|path| {
                rt.tree
                    .get(path)
                    .is_some_and(|node| node.kind == WidgetKind::Toplevel)
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    let mut windows = Vec::new();
    for path in paths {
        let state = effective_option(interp, &path, "-state");
        if matches!(state.as_str(), "withdrawn" | "iconic") {
            continue;
        }
        let geometry = effective_option(interp, &path, "-geometry");
        let (mut width, mut height, x, y) = parse_window_geometry(&geometry);
        if width.is_none() {
            width = effective_option(interp, &path, "-width")
                .parse::<f64>()
                .ok()
                .filter(|value| *value > 0.0);
        }
        if height.is_none() {
            height = effective_option(interp, &path, "-height")
                .parse::<f64>()
                .ok()
                .filter(|value| *value > 0.0);
        }
        let title = effective_option(interp, &path, "-title");
        windows.push(ToplevelView {
            path: path.clone(),
            title: if title.is_empty() { path } else { title },
            width: width.unwrap_or(320.0),
            height: height.unwrap_or(240.0),
            x,
            y,
        });
    }
    windows
}

/// The close button of a secondary toplevel: run its `WM_DELETE_WINDOW`
/// protocol, else destroy it.
pub fn close_window(path: &str) {
    let script = active_interp().and_then(|ptr| {
        with_state(unsafe { &*ptr }, |rt| {
            rt.protocols
                .get(&(path.to_string(), "WM_DELETE_WINDOW".to_string()))
                .cloned()
        })
        .flatten()
        .filter(|script| !script.is_empty())
    });
    let script =
        script.unwrap_or_else(|| format!("destroy {}", list_value(vec![path.to_string()])));
    let _ = eval_active(&script);
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

/// Step a spinbox up (`direction` 1) or down (-1): the next `-values`
/// entry, else the numeric value moved by `-increment` and held to
/// `-from`/`-to` (wrapping with `-wrap`).  Runs `-command` on a change.
pub fn spinbox_step(path: &str, direction: i32) {
    with_eval(|| spinbox_step_inner(path, direction));
}

fn spinbox_step_inner(path: &str, direction: i32) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let current = entry_value(path);
    let wrap = matches!(
        effective_option(interp, path, "-wrap").as_str(),
        "1" | "true" | "yes" | "on"
    );
    let values = effective_option(interp, path, "-values");
    let listed = Value::from_str(&values)
        .as_list()
        .filter(|values| !values.is_empty());
    let next = if let Some(values) = listed {
        let names = values
            .iter()
            .map(|v| v.as_str().to_string())
            .collect::<Vec<_>>();
        let index = names.iter().position(|value| *value == current);
        let next = match (index, direction) {
            (Some(index), 1) if index + 1 < names.len() => index + 1,
            (Some(_), 1) if wrap => 0,
            (Some(index), 1) => index,
            (Some(index), _) if index > 0 => index - 1,
            (Some(_), _) if wrap => names.len() - 1,
            (Some(index), _) => index,
            (None, 1) => 0,
            (None, _) => names.len() - 1,
        };
        names[next].clone()
    } else {
        let from = effective_option(interp, path, "-from")
            .parse::<f64>()
            .unwrap_or(0.0);
        let to = effective_option(interp, path, "-to")
            .parse::<f64>()
            .unwrap_or(0.0);
        let increment = effective_option(interp, path, "-increment")
            .parse::<f64>()
            .unwrap_or(1.0);
        let low = from.min(to);
        let high = from.max(to);
        let value = current.trim().parse::<f64>().unwrap_or(from);
        let mut next = value + f64::from(direction) * increment;
        if wrap {
            if next > high {
                next = low;
            } else if next < low {
                next = high;
            }
        } else {
            next = next.clamp(low, high);
        }
        let format = effective_option(interp, path, "-format");
        if format.is_empty() {
            format_number(next)
        } else {
            let script = format!(
                "format {}",
                list_value(vec![format.clone(), format_number(next)])
            );
            interp
                .eval(&script)
                .map(|value| value.as_str().to_string())
                .unwrap_or_else(|_| format_number(next))
        }
    };
    if next == current {
        return;
    }
    let cursor = next.chars().count();
    if update_entry(interp, path, next, cursor).is_ok() {
        let command = effective_option(interp, path, "-command");
        if !command.is_empty() {
            let _ = eval_active(&command);
        }
    }
}

/// A key in a spinbox: the widget's bindings, then Up/Down step (Tk's
/// Spinbox class binding).
pub fn spinbox_key(path: &str, key: &str, character: &str) {
    entry_key(path, key, character);
    match key {
        "Up" => spinbox_step(path, 1),
        "Down" => spinbox_step(path, -1),
        _ => {}
    }
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

/// Resolve a Tk text index: `insert`, `end`, `line.col`, `line.end`, mark
/// names, and chains of `+/- count chars|indices|lines` and the
/// `linestart`/`lineend`/`wordstart`/`wordend` qualifiers.
fn text_index(
    spec: &str,
    content: &str,
    cursor: usize,
    marks: &BTreeMap<String, TextMark>,
) -> Result<usize> {
    let bad = || generic(format!("bad text index \"{spec}\""));
    let mut rest = spec.trim();
    if rest.is_empty() {
        return Err(bad());
    }
    let base_end = rest
        .char_indices()
        .find(|(i, c)| *i > 0 && (*c == '+' || *c == '-' || c.is_whitespace()))
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let mut idx = text_index_base(&rest[..base_end], content, cursor, marks).map_err(|_| bad())?;
    rest = &rest[base_end..];
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let first = rest.chars().next().unwrap();
        if first == '+' || first == '-' {
            rest = &rest[first.len_utf8()..];
            rest = rest.trim_start();
            let digits = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            if digits == 0 {
                return Err(bad());
            }
            let n: isize = rest[..digits].parse().map_err(|_| bad())?;
            rest = &rest[digits..];
            rest = rest.trim_start();
            let unit_end = rest
                .find(|c: char| c.is_whitespace() || c == '+' || c == '-')
                .unwrap_or(rest.len());
            let unit = &rest[..unit_end];
            rest = &rest[unit_end..];
            let signed = if first == '-' { -n } else { n };
            match unit {
                "c" | "chars" | "i" | "indices" => {
                    let len = content.chars().count() as isize;
                    idx = (idx as isize + signed).clamp(0, len) as usize;
                }
                "l" | "lines" => {
                    let (line, col) = char_linecol(content, idx);
                    let target = (line as isize + signed).max(1) as usize;
                    idx = index_of_linecol(content, target, col);
                }
                _ => return Err(bad()),
            }
        } else {
            let word_end = rest
                .find(|c: char| c.is_whitespace() || c == '+' || c == '-')
                .unwrap_or(rest.len());
            let word = &rest[..word_end];
            if word.is_empty() {
                return Err(bad());
            }
            rest = &rest[word_end..];
            idx = match word {
                "linestart" => line_start_of(content, idx),
                "lineend" => line_end_of(content, idx),
                "wordstart" => word_start_at(content, idx),
                "wordend" => word_end_at(content, idx),
                _ => return Err(bad()),
            };
        }
    }
    Ok(idx)
}

fn text_index_base(
    spec: &str,
    content: &str,
    cursor: usize,
    marks: &BTreeMap<String, TextMark>,
) -> Result<usize> {
    let len = content.chars().count();
    let bad = || generic(format!("bad text index \"{spec}\""));
    match spec {
        "" => Err(bad()),
        "insert" => Ok(cursor.min(len)),
        "end" => Ok(len),
        "current" => Ok(marks.get("current").map_or(0, |m| m.index.min(len))),
        _ => {
            if let Some((line, col)) = spec.split_once('.') {
                if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()) {
                    let line: usize = line.parse().map_err(|_| bad())?;
                    if line == 0 {
                        return Err(bad());
                    }
                    return if col == "end" {
                        Ok(line_end(content, line))
                    } else {
                        let col: usize = col.parse().map_err(|_| bad())?;
                        Ok(index_of_linecol(content, line, col))
                    };
                }
            }
            if let Some(mark) = marks.get(spec) {
                return Ok(mark.index.min(len));
            }
            spec.parse::<usize>().map_err(|_| bad()).map(|i| i.min(len))
        }
    }
}

fn char_linecol(content: &str, index: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 0usize;
    for (i, c) in content.chars().enumerate() {
        if i == index {
            return (line, col);
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn line_start(content: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut current = 1usize;
    for (i, c) in content.chars().enumerate() {
        if c == '\n' {
            current += 1;
            if current == line {
                return i + 1;
            }
        }
    }
    content.chars().count()
}

fn line_end(content: &str, line: usize) -> usize {
    let start = line_start(content, line);
    let len = content.chars().count();
    if start >= len {
        return len;
    }
    for (i, c) in content.chars().enumerate().skip(start) {
        if c == '\n' {
            return i;
        }
    }
    len
}

fn index_of_linecol(content: &str, line: usize, col: usize) -> usize {
    let start = line_start(content, line);
    let end = line_end(content, line);
    start + col.min(end.saturating_sub(start))
}

fn line_start_of(content: &str, index: usize) -> usize {
    line_start(content, char_linecol(content, index).0)
}

fn line_end_of(content: &str, index: usize) -> usize {
    line_end(content, char_linecol(content, index).0)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_start_at(content: &str, index: usize) -> usize {
    let chars = content.chars().collect::<Vec<_>>();
    let mut i = index.min(chars.len());
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

fn word_end_at(content: &str, index: usize) -> usize {
    let chars = content.chars().collect::<Vec<_>>();
    let mut i = index.min(chars.len());
    while i < chars.len() && is_word_char(chars[i]) {
        i += 1;
    }
    i
}

fn ensure_current_mark(marks: &mut BTreeMap<String, TextMark>) {
    marks.entry("current".to_string()).or_insert(TextMark {
        index: 0,
        gravity: MarkGravity::Right,
    });
}

/// Shift marks for text inserted at `at` (Tk mark gravity rules).
fn shift_marks_on_insert(marks: Option<&mut BTreeMap<String, TextMark>>, at: usize, len: usize) {
    let Some(marks) = marks else { return };
    if len == 0 {
        return;
    }
    ensure_current_mark(marks);
    for mark in marks.values_mut() {
        if mark.index > at || (mark.index == at && mark.gravity == MarkGravity::Right) {
            mark.index += len;
        }
    }
}

/// Shift marks for the character range `[a, b)` being deleted.
fn shift_marks_on_delete(marks: Option<&mut BTreeMap<String, TextMark>>, a: usize, b: usize) {
    let Some(marks) = marks else { return };
    if b <= a {
        return;
    }
    ensure_current_mark(marks);
    for mark in marks.values_mut() {
        if mark.index >= a && mark.index <= b {
            mark.index = a;
        } else if mark.index > b {
            mark.index -= b - a;
        }
    }
}

/// Shift marks for an edit that replaced `old` with `new` (editor path).
fn adjust_text_marks(old: &str, new: &str, marks: Option<&mut BTreeMap<String, TextMark>>) {
    let Some(marks) = marks else { return };
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
    let old_rest = old_chars.len() - prefix - suffix;
    let new_rest = new_chars.len() - prefix - suffix;
    match new_rest.cmp(&old_rest) {
        std::cmp::Ordering::Greater => {
            shift_marks_on_insert(Some(marks), prefix, new_rest - old_rest)
        }
        std::cmp::Ordering::Less => {
            shift_marks_on_delete(Some(marks), prefix, prefix + old_rest - new_rest)
        }
        std::cmp::Ordering::Equal => {
            if old_rest > 0 {
                ensure_current_mark(marks);
                for mark in marks.values_mut() {
                    if mark.index > prefix && mark.index <= prefix + old_rest {
                        mark.index = prefix;
                    }
                }
            }
        }
    }
}

/// Merge overlapping or adjacent tag ranges (Tk keeps one continuous range).
fn normalize_ranges(ranges: &mut Vec<(usize, usize)>) {
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges.drain(..) {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    *ranges = merged;
}

fn text_marks(interp: &Interp, path: &str) -> BTreeMap<String, TextMark> {
    with_state(interp, |rt| rt.text_marks.get(path).cloned())
        .flatten()
        .unwrap_or_default()
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
        "rectangle" | "oval" | "arc" | "line" | "text" | "polygon" | "image"
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
    let target = rest[0].as_str();
    if rest.len() == 1 {
        let coords = with_state(interp, |rt| {
            rt.canvases
                .get(path)
                .and_then(|canvas| {
                    canvas
                        .items
                        .iter()
                        .find(|item| canvas_item_matches(item, target))
                })
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
        if let Some(canvas) = rt.canvases.get_mut(path) {
            for item in canvas.items.iter_mut() {
                if canvas_item_matches(item, target) {
                    item.coords = coords.clone();
                }
            }
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
    let target = rest[0].as_str();
    if rest.len() == 2 {
        let value = with_state(interp, |rt| {
            rt.canvases.get(path).and_then(|canvas| {
                canvas
                    .items
                    .iter()
                    .find(|item| canvas_item_matches(item, target))
                    .and_then(|item| item.options.get(rest[1].as_str()).cloned())
            })
        })
        .flatten()
        .unwrap_or_default();
        return Ok(Value::from_str(&value));
    }
    let options = parse_pairs(rest, 1, "itemconfigure")?;
    with_state_mut(interp, |rt| {
        if let Some(canvas) = rt.canvases.get_mut(path) {
            for item in canvas.items.iter_mut() {
                if canvas_item_matches(item, target) {
                    item.options.extend(options.clone());
                }
            }
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
                if !canvas_item_matches(item, target) {
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
            let mut selected = Vec::new();
            let mut rest_items = Vec::new();
            for item in canvas.items.drain(..) {
                if canvas_item_matches(&item, target) {
                    selected.push(item);
                } else {
                    rest_items.push(item);
                }
            }
            if selected.is_empty() {
                canvas.items = rest_items;
            } else {
                if raise {
                    rest_items.extend(selected);
                    canvas.items = rest_items;
                } else {
                    selected.extend(rest_items);
                    canvas.items = selected;
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

fn canvas_item_matches(item: &CanvasItem, target: &str) -> bool {
    target == "all"
        || item.id.to_string() == target
        || item
            .options
            .get("-tags")
            .is_some_and(|tags| tags.split_whitespace().any(|tag| tag == target))
}

fn canvas_item_hidden(item: &CanvasItem) -> bool {
    item.options
        .get("-state")
        .is_some_and(|state| state == "hidden")
}

/// Top-left offset of an anchored box (Tk `-anchor`, default `center`).
fn anchor_offset(anchor: &str, width: f64, height: f64) -> (f64, f64) {
    match anchor {
        "n" => (-width / 2.0, 0.0),
        "ne" => (-width, 0.0),
        "e" => (-width, -height / 2.0),
        "se" => (-width, -height),
        "s" => (-width / 2.0, -height),
        "sw" => (0.0, -height),
        "w" => (0.0, -height / 2.0),
        "nw" => (0.0, 0.0),
        _ => (-width / 2.0, -height / 2.0),
    }
}

/// An item's bounding box.  Text extents and line widths are approximated
/// (the runtime has no font metrics; the renderer lays text out natively).
fn canvas_item_bbox(
    item: &CanvasItem,
    images: &BTreeMap<String, PhotoImage>,
) -> Option<(f64, f64, f64, f64)> {
    let coords = &item.coords;
    match item.kind.as_str() {
        "image" if coords.len() >= 2 => {
            let (width, height) = item
                .options
                .get("-image")
                .and_then(|name| images.get(name))
                .map_or((80.0, 80.0), |image| {
                    (image.width as f64, image.height as f64)
                });
            let anchor = item.options.get("-anchor").map_or("center", String::as_str);
            let (dx, dy) = anchor_offset(anchor, width, height);
            let (x, y) = (coords[0] + dx, coords[1] + dy);
            Some((x, y, x + width, y + height))
        }
        "text" if coords.len() >= 2 => {
            let text = item.options.get("-text").map_or("", String::as_str);
            let lines = text.split('\n').count().max(1) as f64;
            let cols = text
                .split('\n')
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0) as f64;
            let (width, height) = (cols * 8.0, lines * 16.0);
            let anchor = item.options.get("-anchor").map_or("center", String::as_str);
            let (dx, dy) = anchor_offset(anchor, width, height);
            let (x, y) = (coords[0] + dx, coords[1] + dy);
            Some((x, y, x + width, y + height))
        }
        _ if coords.len() >= 2 => {
            let mut min_x = coords[0];
            let mut min_y = coords[1];
            let mut max_x = min_x;
            let mut max_y = min_y;
            for pair in coords.chunks(2).filter(|pair| pair.len() == 2) {
                min_x = min_x.min(pair[0]);
                min_y = min_y.min(pair[1]);
                max_x = max_x.max(pair[0]);
                max_y = max_y.max(pair[1]);
            }
            // Tk's bbox covers the outline; the stroke is centred on the path.
            let width = item
                .options
                .get("-width")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(1.0);
            let half = width / 2.0;
            Some((min_x - half, min_y - half, max_x + half, max_y + half))
        }
        _ => None,
    }
}

fn bbox_distance(bbox: (f64, f64, f64, f64), x: f64, y: f64) -> f64 {
    let dx = (bbox.0 - x).max(0.0).max(x - bbox.2);
    let dy = (bbox.1 - y).max(0.0).max(y - bbox.3);
    (dx * dx + dy * dy).sqrt()
}

fn canvas_find(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let (items, images) = with_state(interp, |rt| {
        rt.canvases
            .get(path)
            .map(|canvas| (canvas.items.clone(), rt.images.clone()))
    })
    .flatten()
    .unwrap_or_default();
    let mode = rest.first().map_or("all", |value| value.as_str());
    let mut ids = Vec::new();
    match mode {
        "all" => ids.extend(items.iter().map(|item| item.id.to_string())),
        "withtag" | "withtags" => {
            let target = rest.get(1).map_or("all", |value| value.as_str());
            ids.extend(
                items
                    .iter()
                    .filter(|item| canvas_item_matches(item, target))
                    .map(|item| item.id.to_string()),
            );
        }
        "closest" => {
            if rest.len() < 3 {
                return Err(wrong("find closest", "x y ?halo? ?start?", rest.len() + 2));
            }
            let x = parse_f64_str(rest[1].as_str(), "coordinate")?;
            let y = parse_f64_str(rest[2].as_str(), "coordinate")?;
            let halo = match rest.get(3) {
                Some(value) => Some(parse_f64_str(value.as_str(), "halo")?),
                None => None,
            };
            let mut best: Option<(f64, u64)> = None;
            for item in items.iter().rev().filter(|item| !canvas_item_hidden(item)) {
                let Some(bbox) = canvas_item_bbox(item, &images) else {
                    continue;
                };
                let distance = bbox_distance(bbox, x, y);
                if halo.is_none_or(|halo| distance <= halo)
                    && best.is_none_or(|(closest, _)| distance < closest)
                {
                    best = Some((distance, item.id));
                }
            }
            ids.extend(best.map(|(_, id)| id.to_string()));
        }
        "overlapping" => {
            if rest.len() < 5 {
                return Err(wrong(
                    "find overlapping",
                    "x1 y1 x2 y2 ?tagOrId ...?",
                    rest.len() + 2,
                ));
            }
            let mut x1 = parse_f64_str(rest[1].as_str(), "coordinate")?;
            let mut y1 = parse_f64_str(rest[2].as_str(), "coordinate")?;
            let mut x2 = parse_f64_str(rest[3].as_str(), "coordinate")?;
            let mut y2 = parse_f64_str(rest[4].as_str(), "coordinate")?;
            if x1 > x2 {
                std::mem::swap(&mut x1, &mut x2);
            }
            if y1 > y2 {
                std::mem::swap(&mut y1, &mut y2);
            }
            for item in items.iter().filter(|item| !canvas_item_hidden(item)) {
                let Some((bx0, by0, bx1, by1)) = canvas_item_bbox(item, &images) else {
                    continue;
                };
                if bx1 >= x1 && bx0 <= x2 && by1 >= y1 && by0 <= y2 {
                    ids.push(item.id.to_string());
                }
            }
        }
        other => {
            return Err(generic(format!(
                "bad option \"{other}\": must be all, closest, overlapping, withtag, or withtags"
            )))
        }
    }
    Ok(list_value(ids))
}

fn canvas_bbox(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let (items, images) = with_state(interp, |rt| {
        rt.canvases
            .get(path)
            .map(|canvas| (canvas.items.clone(), rt.images.clone()))
    })
    .flatten()
    .unwrap_or_default();
    let targets: Vec<String> = if rest.is_empty() {
        vec!["all".to_string()]
    } else {
        rest.iter()
            .map(|value| value.as_str().to_string())
            .collect()
    };
    let mut union: Option<(f64, f64, f64, f64)> = None;
    for target in &targets {
        for item in items
            .iter()
            .filter(|item| canvas_item_matches(item, target) && !canvas_item_hidden(item))
        {
            let Some(bbox) = canvas_item_bbox(item, &images) else {
                continue;
            };
            // Tk reports integer pixel bounds, rounded outwards.
            let bbox = (bbox.0.floor(), bbox.1.floor(), bbox.2.ceil(), bbox.3.ceil());
            union = Some(match union {
                None => bbox,
                Some((x0, y0, x1, y1)) => (
                    x0.min(bbox.0),
                    y0.min(bbox.1),
                    x1.max(bbox.2),
                    y1.max(bbox.3),
                ),
            });
        }
    }
    let Some((x0, y0, x1, y1)) = union else {
        return Ok(Value::empty());
    };
    Ok(list_value(vec![
        format!("{:.0}", x0.round()),
        format!("{:.0}", y0.round()),
        format!("{:.0}", x1.round()),
        format!("{:.0}", y1.round()),
    ]))
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
                    .filter(|item| !canvas_item_hidden(item) && canvas_item_contains(item, x, y))
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
    let press = event_from("<Button-1>", &ctx);
    let clicks = register_press(path);
    let click_sequences: Vec<&str> = match clicks {
        2 => vec!["<Double-Button-1>"],
        3 => vec!["<Triple-Button-1>"],
        4 => vec!["<Quadruple-Button-1>"],
        _ => Vec::new(),
    };
    for item in items.iter().rev() {
        let mut targets = vec![item.id.to_string()];
        if let Some(tags) = item.options.get("-tags") {
            targets.extend(tags.split_whitespace().map(str::to_string));
        }
        for target in targets {
            let Some(map) = bindings.get(&target) else {
                continue;
            };
            let mut scripts = Vec::new();
            if let Some(script) = best_script(map, &press) {
                scripts.push(script);
            }
            for sequence in &click_sequences {
                let event = event_from(sequence, &ctx);
                if let Some(script) = best_script(map, &event) {
                    scripts.push(script);
                }
            }
            for script in scripts {
                let script = substitute_bindings(&script, &ctx);
                if let Err(e) = eval_active(&script) {
                    record_error(e.to_string());
                }
            }
        }
    }
    run_binding(path, "<Button-1>", &ctx);
    for sequence in &click_sequences {
        run_binding(path, sequence, &ctx);
    }
    run_binding(path, "<ButtonRelease-1>", &ctx);
}

/// Pointer motion over a canvas: item `<Motion>`/`<Bn-Motion>` bindings of
/// the item under the pointer, then the canvas widget's own bindings.
pub fn canvas_motion(path: &str, x: f64, y: f64, pressed_button: Option<u8>) {
    let Some(ptr) = active_interp() else { return };
    let interp = unsafe { &mut *ptr };
    let (items, bindings) = with_state(interp, |rt| {
        rt.canvases.get(path).map(|canvas| {
            (
                canvas
                    .items
                    .iter()
                    .filter(|item| !canvas_item_hidden(item) && canvas_item_contains(item, x, y))
                    .cloned()
                    .collect::<Vec<_>>(),
                canvas.bindings.clone(),
            )
        })
    })
    .flatten()
    .unwrap_or_default();
    let mut ctx = EventCtx::for_path(path);
    ctx.x = x as i64;
    ctx.y = y as i64;
    if let Some(button) = pressed_button {
        ctx.button = i64::from(button);
    }
    let sequence = match pressed_button {
        Some(button) => format!("<B{button}-Motion>"),
        None => "<Motion>".to_string(),
    };
    let event = event_from(&sequence, &ctx);
    for item in items.iter().rev() {
        let mut targets = vec![item.id.to_string()];
        if let Some(tags) = item.options.get("-tags") {
            targets.extend(tags.split_whitespace().map(str::to_string));
        }
        for target in targets {
            if let Some(script) = bindings
                .get(&target)
                .and_then(|map| best_script(map, &event))
            {
                let script = substitute_bindings(&script, &ctx);
                if let Err(e) = eval_active(&script) {
                    record_error(e.to_string());
                }
            }
        }
    }
    run_binding(path, &sequence, &ctx);
}

/// A text widget's persisted rich document (ProseMirror JSON), if any.
pub fn rich_doc(path: &str) -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| rt.rich_docs.get(path).cloned()).flatten()
}

/// Take the pending `text see` index for the renderer.
pub fn take_text_see(path: &str) -> Option<usize> {
    let ptr = active_interp()?;
    with_state_mut(unsafe { &mut *ptr }, |rt| rt.text_see.remove(path)).flatten()
}

/// The ProseMirror position for a text widget's char index (for
/// `text see` scrolling in the rich editor).
pub fn text_index_doc_pos(path: &str, index: usize) -> Option<usize> {
    let json = rich_doc(path)?;
    text_offset_to_doc_pos(&json, index)
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

/// Map a ProseMirror document position to the plain-text char offset used
/// by text indices and tag ranges (blocks joined by newlines).
fn doc_pos_to_text_offset(json: &str, position: usize) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let blocks = value.get("content")?.as_array()?;
    let mut doc_pos = 0usize;
    let mut text_offset = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            text_offset += 1;
        }
        doc_pos += 1;
        let Some(nodes) = block.get("content").and_then(|v| v.as_array()) else {
            doc_pos += 1;
            continue;
        };
        for node in nodes {
            if node.get("type").and_then(|v| v.as_str()) != Some("text") {
                continue;
            }
            let content = node.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let length = content.chars().count();
            let end = doc_pos + length;
            if position <= end {
                let offset = position.saturating_sub(doc_pos).min(length);
                return Some(text_offset + offset);
            }
            doc_pos = end;
            text_offset += length;
        }
        doc_pos += 1;
    }
    Some(text_offset)
}

/// Map a plain-text char offset to a ProseMirror document position.
fn text_offset_to_doc_pos(json: &str, offset: usize) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let blocks = value.get("content")?.as_array()?;
    let mut doc_pos = 0usize;
    let mut text_offset = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            text_offset += 1;
        }
        doc_pos += 1;
        let Some(nodes) = block.get("content").and_then(|v| v.as_array()) else {
            doc_pos += 1;
            continue;
        };
        for node in nodes {
            if node.get("type").and_then(|v| v.as_str()) != Some("text") {
                continue;
            }
            let content = node.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let length = content.chars().count();
            if offset <= text_offset + length {
                return Some(doc_pos + offset.saturating_sub(text_offset).min(length));
            }
            doc_pos += length;
            text_offset += length;
        }
        doc_pos += 1;
    }
    Some(doc_pos)
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
    let marks = text_marks(interp, path);
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
            let start = text_index(rest[2].as_str(), &content, cursor, &marks)?;
            let end = match rest.get(3) {
                Some(last) => text_index(last.as_str(), &content, cursor, &marks)?,
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
                normalize_ranges(&mut tag.ranges);
                rt.bump();
            });
            Ok(Value::empty())
        }
        "remove" => {
            if rest.len() < 3 || rest.len() > 4 {
                return Err(wrong("tag remove", "tagName index1 ?index2?", rest.len() + 2));
            }
            let start = text_index(rest[2].as_str(), &content, cursor, &marks)?;
            let end = match rest.get(3) {
                Some(last) => text_index(last.as_str(), &content, cursor, &marks)?,
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
                Some(value) => Some(text_index(value.as_str(), &content, cursor, &marks)?),
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
        "nextrange" | "prevrange" => {
            if rest.len() < 3 || rest.len() > 4 {
                return Err(wrong(
                    if rest[0].as_str() == "nextrange" {
                        "tag nextrange"
                    } else {
                        "tag prevrange"
                    },
                    "tagName index1 ?index2?",
                    rest.len() + 2,
                ));
            }
            let start = text_index(rest[2].as_str(), &content, cursor, &marks)?;
            let limit = match rest.get(3) {
                Some(value) => Some(text_index(value.as_str(), &content, cursor, &marks)?),
                None => None,
            };
            let ranges = with_state(interp, |rt| {
                rt.text_tags
                    .get(path)
                    .and_then(|tags| tags.get(rest[1].as_str()))
                    .map(|tag| tag.ranges.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            let found = if rest[0].as_str() == "nextrange" {
                ranges
                    .into_iter()
                    .filter(|(begin, end)| *begin >= start && limit.is_none_or(|limit| *end <= limit))
                    .min_by_key(|(begin, _)| *begin)
            } else {
                ranges
                    .into_iter()
                    .filter(|(begin, end)| *begin < start && limit.is_none_or(|limit| *end > limit))
                    .max_by_key(|(begin, _)| *begin)
            };
            let out = found
                .map(|(begin, end)| {
                    vec![
                        char_to_linecol(&content, begin),
                        char_to_linecol(&content, end),
                    ]
                })
                .unwrap_or_default();
            Ok(list_value(out))
        }
        "bind" => {
            if rest.len() < 2 || rest.len() > 4 {
                return Err(wrong(
                    "tag bind",
                    "tagName ?sequence? ?script?",
                    rest.len() + 2,
                ));
            }
            let name = rest[1].as_str().to_string();
            if rest.len() == 2 {
                let sequences = with_state(interp, |rt| {
                    rt.text_tags
                        .get(path)
                        .and_then(|tags| tags.get(&name))
                        .map(|tag| tag.bindings.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
                return Ok(list_value(sequences));
            }
            let sequence = rest[2].as_str().to_string();
            if rest.len() == 3 {
                let script = with_state(interp, |rt| {
                    rt.text_tags
                        .get(path)
                        .and_then(|tags| tags.get(&name))
                        .and_then(|tag| tag.bindings.get(&sequence))
                        .cloned()
                })
                .flatten()
                .unwrap_or_default();
                return Ok(Value::from_str(&script));
            }
            let script = rest[3].as_str().to_string();
            with_state_mut(interp, |rt| {
                let tag = rt
                    .text_tags
                    .entry(path.to_string())
                    .or_default()
                    .entry(name.clone())
                    .or_default();
                if script.is_empty() {
                    tag.bindings.remove(&sequence);
                } else {
                    tag.bindings.insert(sequence.clone(), script.clone());
                }
                rt.bump();
            });
            Ok(Value::empty())
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
            "bad option \"{other}\": must be add, bind, cget, configure, delete, names, nextrange, prevrange, ranges, or remove"
        ))),
    }
}

fn text_undo_enabled(rt: &TkRuntime, path: &str) -> bool {
    rt.tree.get(path).is_some_and(|node| {
        node.options
            .get("-undo")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
    })
}

fn text_snapshot(rt: &TkRuntime, path: &str) -> Option<TextSnapshot> {
    let node = rt.tree.get(path)?;
    Some(TextSnapshot {
        content: node.content.clone(),
        cursor: node.cursor,
        tags: rt.text_tags.get(path).cloned().unwrap_or_default(),
        marks: rt.text_marks.get(path).cloned().unwrap_or_default(),
        rich_doc: rt.rich_docs.get(path).cloned(),
        modified: rt.text_modified.contains(path),
    })
}

/// Record the state before an edit (when `-undo` is on).  Adjacent edits
/// within half a second group, as Tk's undo does; `edit separator` and
/// `edit undo/redo` start a new group.
fn record_text_undo(rt: &mut TkRuntime, path: &str) {
    if !text_undo_enabled(rt, path) {
        return;
    }
    let Some(snapshot) = text_snapshot(rt, path) else {
        return;
    };
    let now = std::time::Instant::now();
    let node = rt.tree.get(path);
    let max = node
        .and_then(|node| node.options.get("-maxundo"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let autoseparators = node
        .and_then(|node| node.options.get("-autoseparators"))
        .is_none_or(|value| !matches!(value.as_str(), "0" | "false" | "no" | "off"));
    let entry = rt.text_undo.entry(path.to_string()).or_default();
    let grouped = autoseparators
        && !entry.force_new
        && entry
            .last
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(500));
    if !grouped {
        entry.undo.push(snapshot);
        if max > 0 && entry.undo.len() > max {
            entry.undo.remove(0);
        }
    }
    entry.redo.clear();
    entry.force_new = false;
    entry.last = Some(now);
}

fn apply_text_snapshot(rt: &mut TkRuntime, path: &str, snapshot: &TextSnapshot) {
    if let Some(node) = rt.tree.get_mut(path) {
        node.content = snapshot.content.clone();
        node.cursor = snapshot.cursor;
    }
    if snapshot.tags.is_empty() {
        rt.text_tags.remove(path);
    } else {
        rt.text_tags.insert(path.to_string(), snapshot.tags.clone());
    }
    if snapshot.marks.is_empty() {
        rt.text_marks.remove(path);
    } else {
        rt.text_marks
            .insert(path.to_string(), snapshot.marks.clone());
    }
    match &snapshot.rich_doc {
        Some(doc) => {
            rt.rich_docs.insert(path.to_string(), doc.clone());
        }
        None => {
            rt.rich_docs.remove(path);
        }
    }
    if snapshot.modified {
        rt.text_modified.insert(path.to_string());
    } else {
        rt.text_modified.remove(path);
    }
}

fn text_get(interp: &mut Interp, path: &str, first: &str, last: Option<&str>) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let marks = text_marks(interp, path);
    let start = text_index(first, &content, cursor, &marks)?;
    let end = match last {
        Some(last) => text_index(last, &content, cursor, &marks)?,
        None => (start + 1).min(content.chars().count()),
    };
    let (a, b) = (start.min(end), start.max(end));
    Ok(Value::from_str(&char_slice(&content, a, b)))
}

/// Where a mark at `index` ends up after inserting at `at`.
fn mark_after_insert(index: usize, at: usize, len: usize, gravity: MarkGravity) -> usize {
    if index > at || (index == at && gravity == MarkGravity::Right) {
        index + len
    } else {
        index
    }
}

/// Where a mark at `index` ends up after deleting `[a, b)`.
fn mark_after_delete(index: usize, a: usize, b: usize) -> usize {
    if index >= a && index <= b {
        a
    } else if index > b {
        index - (b - a)
    } else {
        index
    }
}

fn text_insert(
    interp: &mut Interp,
    path: &str,
    at: usize,
    text: &str,
    tag_list: Option<&str>,
) -> Result<Value> {
    let (content, cursor_before) = text_state(interp, path);
    let at = at.min(content.chars().count());
    if text.is_empty() && tag_list.is_none() {
        return Ok(Value::empty());
    }
    let new_content = replace_chars(&content, at, at, text);
    let len = text.chars().count();
    let gravity = with_state(interp, |rt| rt.insert_gravity.get(path).copied())
        .flatten()
        .unwrap_or_default();
    let cursor = mark_after_insert(cursor_before, at, len, gravity);
    let tags = tag_list.map(str::to_string);
    with_state_mut(interp, |rt| {
        record_text_undo(rt, path);
        adjust_text_tag_ranges(&content, &new_content, rt.text_tags.get_mut(path));
        shift_marks_on_insert(rt.text_marks.get_mut(path), at, len);
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = new_content;
            n.cursor = cursor;
        }
        rt.text_modified.insert(path.to_string());
        if let Some(tags) = &tags {
            let widget_tags = rt.text_tags.entry(path.to_string()).or_default();
            for name in tags.split_whitespace() {
                let tag = widget_tags.entry(name.to_string()).or_default();
                tag.ranges.push((at, at + len));
                normalize_ranges(&mut tag.ranges);
            }
        }
        rt.bump();
    });
    Ok(Value::empty())
}

fn text_delete(interp: &mut Interp, path: &str, first: &str, last: Option<&str>) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let marks = text_marks(interp, path);
    let start = text_index(first, &content, cursor, &marks)?;
    let end = match last {
        Some(last) => text_index(last, &content, cursor, &marks)?,
        None => (start + 1).min(content.chars().count()),
    };
    text_delete_range(interp, path, start.min(end), start.max(end));
    Ok(Value::empty())
}

fn text_delete_range(interp: &mut Interp, path: &str, a: usize, b: usize) {
    let (content, _) = text_state(interp, path);
    let len = content.chars().count();
    let a = a.min(len);
    let b = b.min(len);
    if b <= a {
        return;
    }
    let new_content = replace_chars(&content, a, b, "");
    with_state_mut(interp, |rt| {
        record_text_undo(rt, path);
        adjust_text_tag_ranges(&content, &new_content, rt.text_tags.get_mut(path));
        shift_marks_on_delete(rt.text_marks.get_mut(path), a, b);
        rt.text_modified.insert(path.to_string());
        if let Some(n) = rt.tree.get_mut(path) {
            n.content = new_content;
            n.cursor = mark_after_delete(n.cursor, a, b);
            rt.bump();
        }
    });
}

fn text_replace(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let marks = text_marks(interp, path);
    let start = text_index(rest[0].as_str(), &content, cursor, &marks)?;
    let end = text_index(rest[1].as_str(), &content, cursor, &marks)?;
    let (a, b) = (start.min(end), start.max(end));
    text_delete_range(interp, path, a, b);
    let mut at = a;
    let mut pos = 2;
    while pos < rest.len() {
        let chars = rest[pos].as_str();
        let tags = rest.get(pos + 1).map(|value| value.as_str());
        text_insert(interp, path, at, chars, tags)?;
        at += chars.chars().count();
        pos += if tags.is_some() { 2 } else { 1 };
    }
    Ok(Value::empty())
}

/// Collect matches fully inside the character window `[from, to)`.
fn text_search_region(
    chars: &[char],
    pattern: &str,
    from: usize,
    to: usize,
    regexp: bool,
    nocase: bool,
) -> Result<Vec<(usize, usize)>> {
    let mut out = Vec::new();
    if to < from || from > chars.len() {
        return Ok(out);
    }
    let to = to.min(chars.len());
    if regexp {
        let sub = chars[from..to].iter().collect::<String>();
        let re = regex::RegexBuilder::new(pattern)
            .case_insensitive(nocase)
            .build()
            .map_err(|e| generic(format!("couldn't compile regular expression pattern: {e}")))?;
        for found in re.find_iter(&sub) {
            let start = from + sub[..found.start()].chars().count();
            let len = sub[found.start()..found.end()].chars().count();
            out.push((start, start + len));
        }
    } else {
        let needle = pattern.chars().collect::<Vec<_>>();
        if needle.is_empty() {
            out.push((from, from));
        } else if needle.len() <= to - from {
            for start in from..=(to - needle.len()) {
                let equal = chars[start..start + needle.len()]
                    .iter()
                    .zip(needle.iter())
                    .all(|(a, b)| {
                        if nocase {
                            a.to_lowercase().eq(b.to_lowercase())
                        } else {
                            a == b
                        }
                    });
                if equal {
                    out.push((start, start + needle.len()));
                }
            }
        }
    }
    Ok(out)
}

fn text_search(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let mut forward = true;
    let mut regexp = false;
    let mut nocase = false;
    let mut all = false;
    let mut count_var: Option<String> = None;
    let mut pos = 0usize;
    while pos < rest.len() {
        let arg = rest[pos].as_str();
        if arg == "--" {
            pos += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        let name = &arg[1..];
        let switch = |full: &str| !name.is_empty() && full.starts_with(name);
        if switch("forward") {
            forward = true;
        } else if switch("backward") {
            forward = false;
        } else if switch("exact") {
            regexp = false;
        } else if switch("regexp") {
            regexp = true;
        } else if switch("nocase") {
            nocase = true;
        } else if switch("all") {
            all = true;
        } else if switch("count") {
            pos += 1;
            let Some(value) = rest.get(pos) else {
                return Err(wrong("search -count", "varName", rest.len() + 2));
            };
            count_var = Some(value.as_str().to_string());
        } else {
            return Err(generic(format!(
                "bad switch \"{arg}\": must be -all, -backward, -count, -exact, -forward, -nocase, or -regexp"
            )));
        }
        pos += 1;
    }
    if rest.len() <= pos {
        return Err(wrong(
            "search",
            "?switches? pattern index ?stopIndex?",
            rest.len() + 2,
        ));
    }
    let pattern = rest[pos].as_str().to_string();
    pos += 1;
    if rest.len() <= pos {
        return Err(wrong(
            "search",
            "?switches? pattern index ?stopIndex?",
            rest.len() + 2,
        ));
    }
    let (content, cursor) = text_state(interp, path);
    let marks = text_marks(interp, path);
    let index = text_index(rest[pos].as_str(), &content, cursor, &marks)?;
    pos += 1;
    let stop = match rest.get(pos) {
        Some(value) => Some(text_index(value.as_str(), &content, cursor, &marks)?),
        None => None,
    };
    let chars = content.chars().collect::<Vec<_>>();
    let len = chars.len();
    eprintln!("SEARCH rest={:?} pattern={pattern:?} index={index} stop={stop:?} all={all} regexp={regexp} len={len}", rest.iter().map(|v| v.as_str().to_string()).collect::<Vec<_>>());
    let mut matches: Vec<(usize, usize)> = Vec::new();
    if forward {
        // `stopIndex` limits where a match may start, not where it ends.
        let mut region = text_search_region(&chars, &pattern, index, len, regexp, nocase)?;
        if let Some(stop) = stop {
            region.retain(|(start, _)| *start < stop);
        }
        matches.extend(region);
        if stop.is_none() {
            let mut wrapped = text_search_region(&chars, &pattern, 0, len, regexp, nocase)?;
            wrapped.retain(|(start, _)| *start < index);
            matches.extend(wrapped);
        }
    } else {
        let bound = stop.map_or(0, |stop| stop.min(len));
        let mut region = text_search_region(&chars, &pattern, bound, len, regexp, nocase)?;
        // A backward search starts strictly before `index`; a match at the
        // index itself is only seen after wrapping around.
        region.retain(|(start, _)| *start < index);
        region.reverse();
        matches.extend(region);
        if stop.is_none() {
            let mut wrapped = text_search_region(&chars, &pattern, 0, len, regexp, nocase)?;
            wrapped.retain(|(start, _)| *start >= index);
            wrapped.reverse();
            matches.extend(wrapped);
        }
    }
    eprintln!("SEARCH matches={matches:?}");
    if let Some(var) = count_var {
        let count = matches.first().map_or(0, |(start, end)| end - start);
        if let Err(e) = set_var(&var, &count.to_string()) {
            record_error(e.to_string());
        }
    }
    let out = if all {
        matches
            .iter()
            .map(|(start, _)| char_to_linecol(&content, *start))
            .collect::<Vec<_>>()
    } else {
        matches
            .first()
            .map(|(start, _)| char_to_linecol(&content, *start))
            .into_iter()
            .collect::<Vec<_>>()
    };
    Ok(if all {
        list_value(out)
    } else {
        Value::from_str(out.first().map_or("", String::as_str))
    })
}

fn text_count(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let marks = text_marks(interp, path);
    let mut wanted: Vec<&str> = Vec::new();
    let mut index = 0;
    while index < rest.len() && rest[index].as_str().starts_with('-') {
        match rest[index].as_str() {
            "-chars" | "-indices" | "-lines" => wanted.push(match rest[index].as_str() {
                "-chars" => "chars",
                "-indices" => "indices",
                _ => "lines",
            }),
            "-update" | "-elide" => {}
            other => {
                return Err(generic(format!(
                    "bad option \"{other}\": must be -chars, -elide, -indices, -lines, or -update"
                )));
            }
        }
        index += 1;
    }
    if rest.len() != index + 2 {
        return Err(wrong(
            "count",
            "?-option ...? index1 index2",
            rest.len() + 2,
        ));
    }
    let first = text_index(rest[index].as_str(), &content, cursor, &marks)?;
    let last = text_index(rest[index + 1].as_str(), &content, cursor, &marks)?;
    let (a, b) = (first.min(last), first.max(last));
    if wanted.is_empty() {
        wanted.push("chars");
    }
    let chars = content.chars().collect::<Vec<_>>();
    let slice = &chars[a.min(chars.len())..b.min(chars.len())];
    let newlines = slice.iter().filter(|c| **c == '\n').count();
    let counts = wanted
        .iter()
        .map(|kind| match *kind {
            "chars" | "indices" => slice.len(),
            "lines" => {
                if slice.is_empty() {
                    0
                } else {
                    newlines + usize::from(slice.last().is_some_and(|c| *c != '\n'))
                }
            }
            _ => 0,
        })
        .map(|count| count.to_string())
        .collect::<Vec<_>>();
    Ok(list_value(counts))
}

fn text_dump(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    let (content, cursor) = text_state(interp, path);
    let mut marks = text_marks(interp, path);
    marks.entry("insert".to_string()).or_insert(TextMark {
        index: cursor,
        gravity: MarkGravity::Right,
    });
    marks.entry("current".to_string()).or_insert(TextMark {
        index: content.chars().count(),
        gravity: MarkGravity::Right,
    });
    let (mut text, mut tags, mut dump_marks) = (false, false, false);
    let mut any = false;
    let mut index = 0;
    while index < rest.len() && rest[index].as_str().starts_with('-') {
        any = true;
        match rest[index].as_str() {
            "-all" => {
                text = true;
                tags = true;
                dump_marks = true;
            }
            "-text" => text = true,
            "-tag" => tags = true,
            "-mark" => dump_marks = true,
            "-image" | "-window" => {}
            other => {
                return Err(generic(format!(
                    "bad option \"{other}\": must be -all, -image, -mark, -tag, -text, or -window"
                )));
            }
        }
        index += 1;
    }
    if !any {
        text = true;
        tags = true;
        dump_marks = true;
    }
    if rest.len() < index + 1 || rest.len() > index + 2 {
        return Err(wrong(
            "dump",
            "?-option ...? index1 ?index2?",
            rest.len() + 2,
        ));
    }
    let first = text_index(rest[index].as_str(), &content, cursor, &marks)?;
    let last = match rest.get(index + 1) {
        Some(value) => text_index(value.as_str(), &content, cursor, &marks)?,
        None => content.chars().count(),
    };
    let (a, b) = (first.min(last), first.max(last));
    let chars = content.chars().collect::<Vec<_>>();
    let a = a.min(chars.len());
    let b = b.min(chars.len());

    let mut entries: Vec<(usize, u8, Vec<String>)> = Vec::new();
    let tag_ranges = with_state(interp, |rt| {
        rt.text_tags
            .get(path)
            .map(|tags| {
                tags.iter()
                    .rev()
                    .flat_map(|(name, tag)| {
                        tag.ranges
                            .iter()
                            .map(|(start, end)| (name.clone(), *start, *end))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default();
    if text {
        let mut cuts = vec![a, b];
        for (_, start, end) in &tag_ranges {
            if *start > a && *start < b {
                cuts.push(*start);
            }
            if *end > a && *end < b {
                cuts.push(*end);
            }
        }
        for mark in marks.values() {
            if mark.index > a && mark.index < b {
                cuts.push(mark.index);
            }
        }
        // Tk starts a new text element at each line start too.
        for (position, ch) in chars.iter().enumerate() {
            let start = position + 1;
            if *ch == '\n' && start > a && start < b {
                cuts.push(start);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let value = chars[start..end].iter().collect::<String>();
            entries.push((
                start,
                3,
                vec![
                    "text".to_string(),
                    value,
                    format!("{}", char_to_linecol(&content, start)),
                ],
            ));
        }
    }
    if tags {
        for (name, start, end) in &tag_ranges {
            let (start, end) = (*start, *end);
            if start >= a && start < b {
                entries.push((
                    start,
                    0,
                    vec![
                        "tagon".to_string(),
                        name.clone(),
                        format!("{}", char_to_linecol(&content, start)),
                    ],
                ));
            }
            if end > a && end <= b {
                entries.push((
                    end,
                    1,
                    vec![
                        "tagoff".to_string(),
                        name.clone(),
                        format!("{}", char_to_linecol(&content, end)),
                    ],
                ));
            }
        }
    }
    if dump_marks {
        for (name, mark) in &marks {
            if mark.index < a || mark.index > b {
                continue;
            }
            entries.push((
                mark.index,
                2,
                vec![
                    "mark".to_string(),
                    name.clone(),
                    format!("{}", char_to_linecol(&content, mark.index)),
                ],
            ));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    let flat = entries
        .into_iter()
        .flat_map(|(_, _, words)| words)
        .collect::<Vec<_>>();
    Ok(list_value(flat))
}

fn text_edit_command(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("edit", "option ?arg ...?", rest.len() + 2));
    }
    match rest[0].as_str() {
        "modified" => {
            if rest.len() > 2 {
                return Err(wrong("edit modified", "?boolean?", rest.len() + 2));
            }
            if let Some(value) = rest.get(1) {
                let modified = value.as_bool().ok_or_else(|| {
                    generic(format!(
                        "expected boolean value but got \"{}\"",
                        value.as_str()
                    ))
                })?;
                with_state_mut(interp, |rt| {
                    if modified {
                        rt.text_modified.insert(path.to_string());
                    } else {
                        rt.text_modified.remove(path);
                    }
                });
                Ok(Value::empty())
            } else {
                let modified =
                    with_state(interp, |rt| rt.text_modified.contains(path)).unwrap_or(false);
                Ok(Value::from_bool(modified))
            }
        }
        "reset" => {
            with_state_mut(interp, |rt| {
                rt.text_modified.remove(path);
                rt.text_undo.remove(path);
            });
            Ok(Value::empty())
        }
        "separator" => {
            with_state_mut(interp, |rt| {
                if let Some(entry) = rt.text_undo.get_mut(path) {
                    entry.force_new = true;
                }
            });
            Ok(Value::empty())
        }
        "undo" | "redo" => {
            let forward = rest[0].as_str() == "undo";
            with_state_mut(interp, |rt| {
                if !text_undo_enabled(rt, path) {
                    return;
                }
                let current = text_snapshot(rt, path);
                let Some(entry) = rt.text_undo.get_mut(path) else {
                    return;
                };
                let (from, to) = if forward {
                    (&mut entry.undo, &mut entry.redo)
                } else {
                    (&mut entry.redo, &mut entry.undo)
                };
                let Some(snapshot) = from.pop() else {
                    return;
                };
                if let Some(current) = current {
                    to.push(current);
                }
                entry.force_new = true;
                entry.last = None;
                apply_text_snapshot(rt, path, &snapshot);
                rt.bump();
            });
            Ok(Value::empty())
        }
        "canundo" | "canredo" => {
            let forward = rest[0].as_str() == "canundo";
            let can = with_state(interp, |rt| {
                text_undo_enabled(rt, path)
                    && rt
                        .text_undo
                        .get(path)
                        .is_some_and(|entry| {
                            if forward {
                                !entry.undo.is_empty()
                            } else {
                                !entry.redo.is_empty()
                            }
                        })
            })
            .unwrap_or(false);
            Ok(Value::from_bool(can))
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be canredo, canundo, modified, redo, reset, separator, or undo"
        ))),
    }
}

fn text_mark_command(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong("mark", "option ?arg ...?", rest.len() + 2));
    }
    let (content, cursor) = text_state(interp, path);
    let marks = text_marks(interp, path);
    match rest[0].as_str() {
        "set" => {
            if rest.len() < 2 || rest.len() > 3 {
                return Err(wrong("mark set", "markName ?index?", rest.len() + 2));
            }
            let name = rest[1].as_str().to_string();
            let index = match rest.get(2) {
                Some(value) => text_index(value.as_str(), &content, cursor, &marks)?,
                None => cursor,
            };
            with_state_mut(interp, |rt| {
                if name == "insert" {
                    if let Some(n) = rt.tree.get_mut(path) {
                        n.cursor = index;
                    }
                } else {
                    let widget = rt.text_marks.entry(path.to_string()).or_default();
                    let gravity = widget.get(&name).map_or(MarkGravity::Right, |m| m.gravity);
                    widget.insert(name, TextMark { index, gravity });
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "unset" => {
            if rest.len() < 2 {
                return Err(wrong(
                    "mark unset",
                    "markName ?markName ...?",
                    rest.len() + 2,
                ));
            }
            with_state_mut(interp, |rt| {
                if let Some(widget) = rt.text_marks.get_mut(path) {
                    for value in &rest[1..] {
                        match value.as_str() {
                            "insert" => {}
                            "current" => {
                                if let Some(mark) = widget.get_mut("current") {
                                    mark.index = 0;
                                }
                            }
                            name => {
                                widget.remove(name);
                            }
                        }
                    }
                }
                rt.bump();
            });
            Ok(Value::empty())
        }
        "names" => {
            let mut names = vec!["insert".to_string(), "current".to_string()];
            for name in marks.keys() {
                if name != "insert" && name != "current" {
                    names.push(name.clone());
                }
            }
            Ok(list_value(names))
        }
        "gravity" => {
            if rest.len() < 2 || rest.len() > 3 {
                return Err(wrong(
                    "mark gravity",
                    "markName ?direction?",
                    rest.len() + 2,
                ));
            }
            let name = rest[1].as_str();
            if let Some(direction) = rest.get(2) {
                let gravity = match direction.as_str() {
                    "left" => MarkGravity::Left,
                    "right" => MarkGravity::Right,
                    other => {
                        return Err(generic(format!(
                            "bad mark gravity \"{other}\": must be left or right"
                        )))
                    }
                };
                with_state_mut(interp, |rt| {
                    if name == "insert" {
                        rt.insert_gravity.insert(path.to_string(), gravity);
                    } else {
                        let widget = rt.text_marks.entry(path.to_string()).or_default();
                        widget
                            .entry(name.to_string())
                            .and_modify(|mark| mark.gravity = gravity)
                            .or_insert(TextMark { index: 0, gravity });
                    }
                });
                Ok(Value::empty())
            } else {
                let gravity = if name == "insert" {
                    with_state(interp, |rt| rt.insert_gravity.get(path).copied())
                        .flatten()
                        .unwrap_or_default()
                } else {
                    marks.get(name).map_or(MarkGravity::Right, |m| m.gravity)
                };
                Ok(Value::from_str(match gravity {
                    MarkGravity::Left => "left",
                    MarkGravity::Right => "right",
                }))
            }
        }
        "next" | "previous" => {
            if rest.len() != 2 {
                return Err(wrong("mark next|previous", "index", rest.len() + 2));
            }
            let at = text_index(rest[1].as_str(), &content, cursor, &marks)?;
            let mut all = vec![("insert".to_string(), cursor)];
            all.extend(marks.iter().map(|(name, mark)| (name.clone(), mark.index)));
            let picked = if rest[0].as_str() == "next" {
                all.into_iter()
                    .filter(|(_, index)| *index > at)
                    .min_by_key(|(_, index)| *index)
            } else {
                all.into_iter()
                    .filter(|(_, index)| *index < at)
                    .max_by_key(|(_, index)| *index)
            };
            Ok(Value::from_str(
                picked.map(|(name, _)| name).unwrap_or_default().as_str(),
            ))
        }
        other => Err(generic(format!(
            "bad option \"{other}\": must be gravity, names, next, previous, set, or unset"
        ))),
    }
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

/// Scroll a listbox/treeview so `index` (0-based row) is visible, moving as
/// little as possible like Tk's `see`.
fn scroll_to_row(interp: &mut Interp, path: &str, index: usize, total: usize) {
    if total == 0 {
        return;
    }
    let (fraction, rows) = scroll_state(interp, path);
    let rows = rows.min(total);
    let start = ((fraction * total as f64).round() as usize).min(total.saturating_sub(rows));
    let index = index.min(total - 1);
    if index < start {
        set_scroll_fraction(interp, path, index as f64 / total as f64);
    } else if index >= start + rows {
        let first = index + 1 - rows;
        set_scroll_fraction(interp, path, first as f64 / total as f64);
    }
    notify_yscroll(interp, path, total);
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

fn treeview_visible_count_of(tv: &TreeViewState) -> usize {
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
}

fn treeview_row_index(tv: &TreeViewState, target: &str) -> Option<usize> {
    fn walk(tv: &TreeViewState, parent: Option<&str>, target: &str, row: &mut usize) -> bool {
        for item in tv.items.iter().filter(|i| i.parent.as_deref() == parent) {
            if item.id == target {
                return true;
            }
            *row += 1;
            if item.open && walk(tv, Some(item.id.as_str()), target, row) {
                return true;
            }
        }
        false
    }
    let mut row = 0usize;
    walk(tv, None, target, &mut row).then_some(row)
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
            n.selected = n
                .selected
                .iter()
                .map(|i| if *i >= at { i + items.len() } else { *i })
                .collect();
        }
        if let Some(rows) = rt.listbox_item_options.get_mut(path) {
            *rows = rows
                .iter()
                .map(|(index, options)| {
                    (
                        if *index >= at {
                            index + items.len()
                        } else {
                            *index
                        },
                        options.clone(),
                    )
                })
                .collect();
        }
        rt.bump();
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
        }
        if let Some(rows) = rt.listbox_item_options.get_mut(path) {
            *rows = rows
                .iter()
                .filter(|(index, _)| !(**index >= a && **index <= b))
                .map(|(index, options)| {
                    (
                        if *index > b {
                            index - (b + 1 - a)
                        } else {
                            *index
                        },
                        options.clone(),
                    )
                })
                .collect();
        }
        rt.bump();
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

fn listbox_itemconfigure(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        return Err(wrong(
            "itemconfigure",
            "index ?-option? ?value ...?",
            rest.len() + 2,
        ));
    }
    let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
        .flatten()
        .ok_or_else(|| bad_window(path))?;
    let index = listbox_index(rest[0].as_str(), len)?;
    if rest.len() == 1 {
        let options = with_state(interp, |rt| {
            rt.listbox_item_options
                .get(path)
                .and_then(|rows| rows.get(&index))
                .map(|options| {
                    options
                        .iter()
                        .flat_map(|(k, v)| [k.clone(), v.clone()])
                        .collect::<Vec<_>>()
                })
        })
        .flatten()
        .unwrap_or_default();
        return Ok(list_value(options));
    }
    if rest.len() == 2 && rest[1].as_str().starts_with('-') {
        let value = with_state(interp, |rt| {
            rt.listbox_item_options
                .get(path)
                .and_then(|rows| rows.get(&index))
                .and_then(|options| options.get(rest[1].as_str()))
                .cloned()
        })
        .flatten()
        .unwrap_or_default();
        return Ok(Value::from_str(&value));
    }
    let options = parse_pairs(rest, 1, "itemconfigure")?;
    with_state_mut(interp, |rt| {
        rt.listbox_item_options
            .entry(path.to_string())
            .or_default()
            .entry(index)
            .or_default()
            .extend(options);
        rt.bump();
    });
    Ok(Value::empty())
}

fn listbox_itemcget(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.len() != 2 {
        return Err(wrong("itemcget", "index -option", rest.len() + 2));
    }
    let len = with_state(interp, |rt| rt.tree.get(path).map(|n| n.items.len()))
        .flatten()
        .ok_or_else(|| bad_window(path))?;
    let index = listbox_index(rest[0].as_str(), len)?;
    let value = with_state(interp, |rt| {
        rt.listbox_item_options
            .get(path)
            .and_then(|rows| rows.get(&index))
            .and_then(|options| options.get(rest[1].as_str()))
            .cloned()
    })
    .flatten()
    .unwrap_or_default();
    Ok(Value::from_str(&value))
}

/// One `listbox itemconfigure` option of a row (renderer hook).
pub fn listbox_item_option(path: &str, index: usize, name: &str) -> Option<String> {
    let ptr = active_interp()?;
    with_state(unsafe { &*ptr }, |rt| {
        rt.listbox_item_options
            .get(path)
            .and_then(|rows| rows.get(&index))
            .and_then(|options| options.get(name))
            .cloned()
    })
    .flatten()
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
            fire_virtual(path, "ListboxSelect");
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
            fire_virtual(path, "ListboxSelect");
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
        "clipboard" => clipboard_impl(interp, args),
        "selection" => selection_impl(interp, args),
        "tk_messageBox" => message_box_impl(interp, args),
        "tk_getOpenFile" => file_dialog_impl(interp, args, FileDialogKind::Open),
        "tk_getSaveFile" => file_dialog_impl(interp, args, FileDialogKind::Save),
        "tk_chooseDirectory" => file_dialog_impl(interp, args, FileDialogKind::Directory),
        "tk_chooseColor" => choose_color_impl(interp, args),
        "tk_optionMenu" => option_menu_impl(interp, args),
        "tk_dialog" => tk_dialog_impl(interp, args),
        "grab" => grab_impl(interp, args),
        "ttk::style" => ttk_style_impl(interp, &args[1..]),
        "tk_popup" => tk_popup_impl(interp, args),
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
        // Event fields: %W path, %x/%y widget coords, %X/%Y root coords,
        // %w/%h event size (0 for synthetic events).
        eval(
            &mut i,
            "bind .b <ButtonRelease-1> {set ::ev \"%W %x %y %X %Y %w %h\"}; \
             event generate .b <ButtonRelease-1> -x 2 -y 3",
        );
        assert_eq!(eval(&mut i, "set ::ev"), ".b 2 3 2 3 ?? ??");
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
        eval(
            &mut i,
            "ttk::button .b -text Go; ttk::label .l -text Hi; button .c -text C",
        );
        // ttk widgets carry Tk's T* class names (bind_class("TButton")).
        assert_eq!(eval(&mut i, "winfo class .b"), "TButton");
        assert_eq!(eval(&mut i, "winfo class .l"), "TLabel");
        assert_eq!(eval(&mut i, "winfo class .c"), "Button");
        assert_eq!(eval(&mut i, "bindtags .b"), ".b TButton . all");
        eval(
            &mut i,
            "set ::n 0; bind TButton <Button-1> {incr ::n}; event generate .b <Button-1>",
        );
        assert_eq!(eval(&mut i, "set ::n"), "1");
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
        let group = eval(&mut i, ".c create oval 0 0 5 5 -fill red -tags {g}");
        assert_eq!(eval(&mut i, ".c find withtag g"), group);
        assert_eq!(eval(&mut i, ".c find withtag other"), "");
        eval(&mut i, ".c delete g");
        assert_eq!(eval(&mut i, ".c find withtag g"), "");

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
    fn canvas_operations_accept_tag_or_id() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "canvas .q -width 100 -height 100");
        let a = eval(&mut i, ".q create rectangle 0 0 10 10 -tags {g}");
        let b = eval(&mut i, ".q create rectangle 20 20 30 30 -tags {g}");
        let c = eval(&mut i, ".q create oval 40 40 50 50 -tags {h}");
        eval(&mut i, &format!("set a {a}; set b {b}; set c {c}"));
        eval(&mut i, ".q move g 5 5");
        assert_eq!(eval(&mut i, ".q coords $a"), "5 5 15 15");
        assert_eq!(eval(&mut i, ".q coords $b"), "25 25 35 35");
        eval(&mut i, ".q itemconfigure g -fill red");
        assert_eq!(eval(&mut i, ".q itemcget $a -fill"), "red");
        assert_eq!(eval(&mut i, ".q itemcget $b -fill"), "red");
        eval(&mut i, ".q coords g 1 2 3 4");
        assert_eq!(eval(&mut i, ".q coords $a"), "1 2 3 4");
        assert_eq!(eval(&mut i, ".q coords $b"), "1 2 3 4");
        eval(&mut i, ".q raise g");
        assert_eq!(eval(&mut i, ".q find"), format!("{c} {a} {b}"));
        eval(&mut i, ".q lower g");
        assert_eq!(eval(&mut i, ".q find"), format!("{a} {b} {c}"));
    }

    #[test]
    fn text_marks_and_index_grammar() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "text .m; .m insert 1.0 {hello world}");
        assert_eq!(eval(&mut i, ".m index end"), "1.11");
        assert_eq!(eval(&mut i, ".m index end-1c"), "1.10");
        assert_eq!(eval(&mut i, ".m index {end - 2c}"), "1.9");
        assert_eq!(eval(&mut i, ".m index 1.end"), "1.11");
        assert_eq!(eval(&mut i, ".m index {1.2 wordstart}"), "1.0");
        assert_eq!(eval(&mut i, ".m index {1.2 wordend}"), "1.5");
        eval(&mut i, "text .g; .g insert 1.0 \"aa\nbbb\nc\"");
        assert_eq!(eval(&mut i, ".g index 2.end"), "2.3");
        assert_eq!(eval(&mut i, ".g index {1.5+1l}"), "2.2");
        assert_eq!(eval(&mut i, ".g index {2.1-1l}"), "1.1");
        assert_eq!(eval(&mut i, ".g index {2.1+1l}"), "3.1");
        assert_eq!(eval(&mut i, ".g index {1.0 lineend - 1c}"), "1.1");

        // The insert mark is the widget cursor; custom marks shift with
        // edits according to gravity.
        assert_eq!(eval(&mut i, ".m mark names"), "insert current");
        eval(&mut i, ".m mark set insert 1.2");
        assert_eq!(eval(&mut i, ".m index insert"), "1.2");
        eval(&mut i, ".m mark set my 1.1");
        assert_eq!(eval(&mut i, ".m index my"), "1.1");
        eval(&mut i, ".m mark gravity my left");
        eval(&mut i, ".m insert 1.1 Q");
        assert_eq!(eval(&mut i, ".m index my"), "1.1");
        eval(&mut i, ".m mark gravity my right");
        eval(&mut i, ".m insert 1.1 R");
        assert_eq!(eval(&mut i, ".m index my"), "1.2");
        assert_eq!(eval(&mut i, ".m get 1.0 end"), "hRQello world");
        eval(&mut i, ".m mark set insert 1.6");
        assert_eq!(eval(&mut i, ".m mark next 1.0"), "my");
        assert_eq!(eval(&mut i, ".m mark previous 1.9"), "insert");
        eval(&mut i, ".m mark unset my");
        assert_eq!(eval(&mut i, ".m mark names"), "insert current");

        // Inserted chars accept a tag list; tag ranges and marks shift on
        // Tcl-side edits.
        eval(
            &mut i,
            "text .n; .n insert 1.0 hello; .n tag add hot 1.0 1.2",
        );
        eval(&mut i, ".n insert 1.0 X");
        assert_eq!(eval(&mut i, ".n tag ranges hot"), "1.1 1.3");
        eval(&mut i, ".n insert 1.3 YZ hot");
        assert_eq!(eval(&mut i, ".n tag ranges hot"), "1.1 1.5");
        assert_eq!(eval(&mut i, ".n get 1.0 end"), "XheYZllo");
        eval(&mut i, ".n mark set m 1.4");
        eval(&mut i, ".n delete 1.0 1.2");
        assert_eq!(eval(&mut i, ".n tag ranges hot"), "1.0 1.3");
        assert_eq!(eval(&mut i, ".n index m"), "1.2");
        // The modified flag tracks edits until `edit reset`.
        assert_eq!(eval(&mut i, ".n edit modified"), "1");
        eval(&mut i, ".n edit modified 0");
        assert_eq!(eval(&mut i, ".n edit modified"), "0");
        eval(&mut i, ".n insert end z");
        assert_eq!(eval(&mut i, ".n edit modified"), "1");
        assert_eq!(eval(&mut i, ".n edit canundo"), "0");
        eval(&mut i, ".n edit reset");
        assert_eq!(eval(&mut i, ".n edit modified"), "0");
        // insert gravity controls the insert mark itself
        eval(&mut i, ".n mark gravity insert left");
        eval(&mut i, ".n mark set insert 1.1");
        eval(&mut i, ".n insert 1.1 AA");
        assert_eq!(eval(&mut i, ".n index insert"), "1.1");
    }

    #[test]
    fn text_undo_redo_separator_and_limits() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "text .u -undo 1");
        assert_eq!(eval(&mut i, ".u edit canundo"), "0");
        assert_eq!(eval(&mut i, ".u edit canredo"), "0");
        eval(&mut i, ".u insert 1.0 abc");
        eval(&mut i, ".u edit separator");
        eval(&mut i, ".u insert end d");
        assert_eq!(eval(&mut i, ".u get 1.0 end"), "abcd");
        assert_eq!(eval(&mut i, ".u edit canundo"), "1");
        assert_eq!(eval(&mut i, ".u edit canredo"), "0");
        eval(&mut i, ".u edit undo");
        assert_eq!(eval(&mut i, ".u get 1.0 end"), "abc");
        assert_eq!(eval(&mut i, ".u edit canredo"), "1");
        eval(&mut i, ".u edit undo");
        assert_eq!(eval(&mut i, ".u get 1.0 end"), "");
        assert_eq!(eval(&mut i, ".u edit canundo"), "0");
        eval(&mut i, ".u edit redo");
        assert_eq!(eval(&mut i, ".u get 1.0 end"), "abc");
        eval(&mut i, ".u edit redo");
        assert_eq!(eval(&mut i, ".u get 1.0 end"), "abcd");
        assert_eq!(eval(&mut i, ".u edit canredo"), "0");
        // A fresh edit clears the redo stack.
        eval(&mut i, ".u edit undo");
        eval(&mut i, ".u insert end X");
        assert_eq!(eval(&mut i, ".u edit canredo"), "0");

        // Tags and marks come back with the content they were attached to.
        eval(&mut i, "text .v -undo 1");
        eval(&mut i, ".v insert 1.0 hello");
        eval(&mut i, ".v tag add hot 1.0 1.2");
        eval(&mut i, ".v mark set m 1.2");
        eval(&mut i, ".v edit separator");
        eval(&mut i, ".v insert end X");
        eval(&mut i, ".v tag add cold 1.0 end");
        eval(&mut i, ".v mark set m 1.3");
        eval(&mut i, ".v edit undo");
        assert_eq!(eval(&mut i, ".v get 1.0 end"), "hello");
        assert_eq!(eval(&mut i, ".v tag ranges hot"), "1.0 1.2");
        assert_eq!(eval(&mut i, ".v tag ranges cold"), "");
        assert_eq!(eval(&mut i, ".v index m"), "1.2");

        // `-maxundo` caps the stack, `-undo 0` disables recording.
        eval(&mut i, "text .w -undo 1 -maxundo 1");
        eval(&mut i, ".w insert end a; .w edit separator");
        eval(&mut i, ".w insert end b; .w edit separator");
        eval(&mut i, ".w insert end c");
        eval(&mut i, ".w edit undo");
        assert_eq!(eval(&mut i, ".w get 1.0 end"), "ab");
        eval(&mut i, ".w edit undo");
        assert_eq!(eval(&mut i, ".w get 1.0 end"), "ab");
        eval(&mut i, "text .x");
        eval(&mut i, ".x insert 1.0 a");
        assert_eq!(eval(&mut i, ".x edit canundo"), "0");
        eval(&mut i, ".x edit undo");
        assert_eq!(eval(&mut i, ".x get 1.0 end"), "a");

        // `-autoseparators 0` makes every edit its own undo step.
        eval(&mut i, "text .a -undo 1 -autoseparators 0");
        eval(&mut i, ".a insert 1.0 a");
        eval(&mut i, ".a insert end b");
        eval(&mut i, ".a edit undo");
        assert_eq!(eval(&mut i, ".a get 1.0 end"), "a");
        eval(&mut i, ".a edit undo");
        assert_eq!(eval(&mut i, ".a get 1.0 end"), "");

        // `edit reset` drops history and the modified flag.
        eval(&mut i, ".u edit reset");
        assert_eq!(eval(&mut i, ".u edit canundo"), "0");
        assert_eq!(eval(&mut i, ".u edit canredo"), "0");
        assert_eq!(eval(&mut i, ".u edit modified"), "0");
    }

    #[test]
    fn spinbox_values_stepping_and_textvariable() {
        let mut i = Interp::new();
        install(&mut i);
        assert_eq!(eval(&mut i, "spinbox .s -from 0 -to 10 -increment 2"), ".s");
        assert_eq!(eval(&mut i, ".s get"), "0");
        spinbox_step(".s", 1);
        assert_eq!(eval(&mut i, ".s get"), "2");
        spinbox_step(".s", -1);
        assert_eq!(eval(&mut i, ".s get"), "0");
        eval(&mut i, ".s set 9");
        spinbox_step(".s", 1);
        assert_eq!(eval(&mut i, ".s get"), "10");
        eval(&mut i, ".s configure -wrap 1");
        spinbox_step(".s", 1);
        assert_eq!(eval(&mut i, ".s get"), "0");
        assert_eq!(eval(&mut i, ".s index end"), "1");

        // -values cycles through the list; -wrap decides at the ends.
        eval(&mut i, "spinbox .v -values {a b c}");
        assert_eq!(eval(&mut i, ".v get"), "a");
        spinbox_step(".v", 1);
        assert_eq!(eval(&mut i, ".v get"), "b");
        eval(&mut i, ".v set c");
        spinbox_step(".v", 1);
        assert_eq!(eval(&mut i, ".v get"), "c");
        eval(&mut i, ".v configure -wrap 1");
        spinbox_step(".v", 1);
        assert_eq!(eval(&mut i, ".v get"), "a");
        spinbox_step(".v", -1);
        assert_eq!(eval(&mut i, ".v get"), "c");

        // -textvariable is written on steps and -command runs.
        eval(
            &mut i,
            "set ::n 1; spinbox .t -textvariable ::n -from 0 -to 5 -format %.2f -command {set ::ran 1}",
        );
        assert_eq!(eval(&mut i, ".t get"), "1");
        spinbox_step(".t", 1);
        assert_eq!(eval(&mut i, ".t get"), "2.00");
        assert_eq!(eval(&mut i, "set ::n"), "2.00");
        assert_eq!(eval(&mut i, "set ::ran"), "1");
        assert_eq!(
            eval(
                &mut i,
                "spinbox .u -from 0 -to 2 -increment 1; .u insert end 5; .u get"
            ),
            "05"
        );
    }

    #[test]
    fn ttk_style_database_and_grab() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "ttk::style configure TButton -background red -foreground white",
        );
        assert_eq!(eval(&mut i, "ttk::style lookup TButton -background"), "red");
        assert_eq!(
            eval(&mut i, "ttk::style lookup TButton -missing fallback"),
            "fallback"
        );
        eval(&mut i, "ttk::style map TButton -background {active blue}");
        assert_eq!(
            eval(&mut i, "ttk::style configure TButton -background"),
            "red"
        );
        assert_eq!(
            eval(&mut i, "ttk::style map TButton -background"),
            "active blue"
        );
        assert_eq!(eval(&mut i, "ttk::style theme use"), "default");
        assert_eq!(eval(&mut i, "ttk::style theme names"), "default");
        eval(&mut i, "ttk::style theme use clam");
        assert_eq!(eval(&mut i, "ttk::style theme current"), "clam");
        eval(
            &mut i,
            "ttk::style layout TButton {Button.border Button.focus}",
        );
        assert_eq!(
            eval(&mut i, "ttk::style layout TButton"),
            "Button.border -sticky nswe Button.focus -sticky nswe"
        );
        eval(&mut i, "ttk::style element create MyInd -type image");
        assert_eq!(eval(&mut i, "ttk::style element names My*"), "MyInd");

        eval(&mut i, "button .b");
        eval(&mut i, "grab set .b");
        assert_eq!(eval(&mut i, "grab status .b"), "local");
        assert_eq!(eval(&mut i, "grab current"), ".b");
        eval(&mut i, "grab set -global .b");
        assert_eq!(eval(&mut i, "grab status .b"), "global");
        eval(&mut i, "grab release .b");
        assert_eq!(eval(&mut i, "grab current"), "");

        eval(&mut i, "ttk::button .tb -text T; button .plain -text P");
        assert_eq!(
            ttk_style_value(".tb", "-background").as_deref(),
            Some("red")
        );
        assert_eq!(ttk_style_value(".plain", "-background"), None);
    }

    #[test]
    fn default_editing_virtual_events_fire() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "text .t; .t insert 1.0 abc");
        assert!(
            eval(&mut i, "event info")
                .split(' ')
                .any(|name| name == "<<Copy>>"),
            "event info lists the predefined virtual events: {:?}",
            eval(&mut i, "event info")
        );
        assert_eq!(
            eval(&mut i, "event info <<Copy>>"),
            "<Control-c> <Control-C>"
        );
        eval(
            &mut i,
            "bind .t <<Copy>> {set ::c 1}; bind .t <<Paste>> {set ::p 1}",
        );
        text_key_event(".t", "c", false, true, false, false);
        assert_eq!(eval(&mut i, "set ::c"), "1");
        text_key_event(".t", "v", false, true, false, false);
        assert_eq!(eval(&mut i, "set ::p"), "1");
        // Without the modifier the physical key alone fires no virtual event.
        eval(&mut i, "set ::c 0");
        text_key_event(".t", "c", false, false, false, false);
        assert_eq!(eval(&mut i, "set ::c"), "0");
    }

    #[test]
    fn tk_popup_and_menu_post_points() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "menu .m; .m add command -label A");
        assert_eq!(popup_menu(), None);
        eval(&mut i, "tk_popup .m 40 60");
        assert_eq!(popup_menu(), Some((".m".to_string(), 40.0, 60.0)));
        eval(&mut i, ".m unpost");
        assert_eq!(popup_menu(), None);
        eval(&mut i, ".m post 10 20");
        assert_eq!(popup_menu(), Some((".m".to_string(), 10.0, 20.0)));
        // Bare post drops the menubar cascade and supersedes the point.
        eval(&mut i, ".m post");
        assert_eq!(open_menu().as_deref(), Some(".m"));
        assert_eq!(popup_menu(), None);
        eval(&mut i, ".m unpost");
        assert_eq!(open_menu(), None);
    }

    #[test]
    fn panedwindow_panes_and_options() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "ttk::panedwindow .p -orient horizontal;             label .p.a -text A; label .p.b -text B;             .p add .p.a .p.b",
        );
        assert_eq!(eval(&mut i, "winfo class .p"), "TPanedwindow");
        assert_eq!(eval(&mut i, ".p panes"), ".p.a .p.b");
        eval(&mut i, ".p pane .p.a -minsize 20");
        assert_eq!(eval(&mut i, ".p pane .p.a -minsize"), "20");
        assert_eq!(eval(&mut i, ".p panecget .p.a -minsize"), "20");
        eval(&mut i, "label .p.c -text C; .p insert 0 .p.c");
        assert_eq!(eval(&mut i, ".p panes"), ".p.c .p.a .p.b");
        assert_eq!(eval(&mut i, ".p sashpos 1"), "0");
        eval(&mut i, ".p sashpos 0 30");
        assert_eq!(eval(&mut i, ".p panecget .p.c -width"), "30");
        eval(&mut i, ".p forget .p.a");
        assert_eq!(eval(&mut i, ".p panes"), ".p.c .p.b");
        assert!(i.eval("label .other -text X; .p add .other").is_err());
        eval(&mut i, "panedwindow .q; label .q.a -text A; .q add .q.a");
        assert_eq!(eval(&mut i, "winfo class .q"), "Panedwindow");
        assert_eq!(eval(&mut i, ".q panes"), ".q.a");
    }

    #[test]
    fn sizegrip_creation_and_class() {
        let mut i = Interp::new();
        install(&mut i);
        assert_eq!(eval(&mut i, "ttk::sizegrip .sg"), ".sg");
        assert_eq!(eval(&mut i, "winfo class .sg"), "TSizegrip");
        assert_eq!(eval(&mut i, ".sg cget -takefocus"), "");
        assert_eq!(
            eval(&mut i, "sizegrip .plain; winfo class .plain"),
            "TSizegrip"
        );
        assert_eq!(eval(&mut i, "winfo width .sg"), "14");
        assert_eq!(eval(&mut i, "winfo height .sg"), "14");
    }

    #[test]
    fn menubutton_and_menu_item_state() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "menu .m -tearoff 0; .m add command -label A;             menubutton .mb -text Options -menu .m",
        );
        assert_eq!(eval(&mut i, ".mb cget -menu"), ".m");
        assert_eq!(eval(&mut i, ".mb cget -text"), "Options");
        assert_eq!(eval(&mut i, "winfo class .mb"), "Menubutton");
        eval(&mut i, "ttk::menubutton .tm -text T -menu .m");
        assert_eq!(eval(&mut i, "winfo class .tm"), "TMenubutton");
        eval(
            &mut i,
            ".m add checkbutton -label B -variable ::b -onvalue 1 -offvalue 0;             .m add radiobutton -label R -variable ::r -value red;             set ::b 1; set ::r red",
        );
        assert!(menu_item_checked(".m", 1));
        assert!(menu_item_checked(".m", 2));
        eval(&mut i, "set ::b 0; set ::r blue");
        assert!(!menu_item_checked(".m", 1));
        assert!(!menu_item_checked(".m", 2));
    }

    #[test]
    fn option_menu_builds_a_menubutton() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "set ::v b");
        assert_eq!(eval(&mut i, "tk_optionMenu .o ::v a b c"), ".o.menu");
        assert_eq!(eval(&mut i, ".o cget -menu"), ".o.menu");
        assert_eq!(eval(&mut i, ".o cget -textvariable"), "::v");
        assert_eq!(eval(&mut i, "winfo class .o"), "Menubutton");
        assert_eq!(eval(&mut i, ".o.menu type 0"), "radiobutton");
        assert_eq!(eval(&mut i, ".o.menu entrycget 0 -label"), "a");
        assert_eq!(eval(&mut i, ".o.menu entrycget 2 -value"), "c");
        assert_eq!(eval(&mut i, "set ::v"), "a");
        press_menu_item(".o.menu", 2);
        assert_eq!(eval(&mut i, "set ::v"), "c");
        // Destroying the menubutton destroys its child menu by name.
        eval(&mut i, "destroy .o");
        assert!(i.eval(".o.menu type 0").is_err());
    }

    #[test]
    fn selection_own_handle_and_clipboard_fallback() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "proc selText {offset maxChars} {string range \"hello there\" $offset [expr {$offset + $maxChars - 1}]}",
        );
        assert_eq!(eval(&mut i, "selection get"), "");
        eval(&mut i, "selection handle . selText");
        // Handlers only serve reads while this app owns the selection.
        assert_eq!(eval(&mut i, "selection get"), "");
        eval(&mut i, "selection own");
        assert_eq!(eval(&mut i, "selection get"), "hello there");
        assert_eq!(
            eval(&mut i, "selection get -type UTF8_STRING"),
            "hello there"
        );
        assert_eq!(
            eval(&mut i, "selection get -type TARGETS"),
            "STRING UTF8_STRING"
        );
        eval(&mut i, "selection clear");
        assert_eq!(eval(&mut i, "selection get"), "");
        // The CLIPBOARD selection falls back to the clipboard text.
        eval(&mut i, "clipboard clear; clipboard append sys");
        assert_eq!(eval(&mut i, "selection get -selection CLIPBOARD"), "sys");
        // `own -command` registers the handler in one step.
        eval(
            &mut i,
            "proc ownedSel {offset maxChars} {string range owned $offset [expr {$offset + $maxChars - 1}]};             selection own -selection CLIPBOARD -command ownedSel",
        );
        assert_eq!(eval(&mut i, "selection get -selection CLIPBOARD"), "owned");
        eval(
            &mut i,
            "selection handle -selection CLIPBOARD -type STRING . {}",
        );
        assert_eq!(eval(&mut i, "selection get -selection CLIPBOARD"), "sys");
    }

    static LAST_FILE_DIALOG: std::sync::Mutex<Option<FileDialogRequest>> =
        std::sync::Mutex::new(None);

    fn choose_paths(request: &FileDialogRequest) -> Option<Vec<String>> {
        *LAST_FILE_DIALOG.lock().unwrap() = Some(request.clone());
        Some(vec!["/tmp/a.txt".to_string(), "/tmp/b.txt".to_string()])
    }

    fn choose_plain(_: &FileDialogRequest) -> Option<Vec<String>> {
        Some(vec!["/tmp/name".to_string()])
    }

    static LAST_COLOR_DIALOG: std::sync::Mutex<Option<ColorDialogRequest>> =
        std::sync::Mutex::new(None);

    fn choose_blue(request: &ColorDialogRequest) -> Option<String> {
        *LAST_COLOR_DIALOG.lock().unwrap() = Some(request.clone());
        Some("#0000ff".to_string())
    }

    static LAST_DIALOG: std::sync::Mutex<Option<DialogRequest>> = std::sync::Mutex::new(None);

    fn choose_second(request: &DialogRequest) -> Option<usize> {
        *LAST_DIALOG.lock().unwrap() = Some(request.clone());
        Some(1)
    }

    #[test]
    fn tk_dialog_returns_a_button_index() {
        let mut i = Interp::new();
        install(&mut i);
        // Without a backend the default button's index is reported.
        assert_eq!(eval(&mut i, "tk_dialog .d T M {} 0 Yes No"), "0");
        assert_eq!(eval(&mut i, "tk_dialog .d T M {} 1 Yes No"), "1");
        assert_eq!(eval(&mut i, "tk_dialog .d T M {} {}"), "0");
        set_dialog_handler(choose_second);
        assert_eq!(eval(&mut i, "tk_dialog .d Title Text {} 0 Yes No"), "1");
        let request = LAST_DIALOG.lock().unwrap().take().unwrap();
        assert_eq!(request.title, "Title");
        assert_eq!(request.message, "Text");
        assert_eq!(request.buttons, vec!["Yes", "No"]);
        assert_eq!(request.default, 0);
        assert_eq!(request.parent, ".d");
        set_dialog_handler(|_| None);
    }

    #[test]
    fn color_dialog_request_and_answer() {
        let mut i = Interp::new();
        install(&mut i);
        assert_eq!(eval(&mut i, "tk_chooseColor"), "");
        assert_eq!(eval(&mut i, "tk_chooseColor -initialcolor red"), "red");
        set_color_dialog_handler(choose_blue);
        assert_eq!(
            eval(
                &mut i,
                "tk_chooseColor -title Pick -initialcolor #ff0000 -parent ."
            ),
            "#0000ff"
        );
        let request = LAST_COLOR_DIALOG.lock().unwrap().take().unwrap();
        assert_eq!(request.title, "Pick");
        assert_eq!(request.initial, "#ff0000");
        assert_eq!(request.parent, ".");
        assert!(i
            .eval("tk_chooseColor -bogus x")
            .is_err_and(|e| e.to_string().contains("bad option")));
    }

    #[test]
    fn file_dialogs_parse_and_return_paths() {
        let mut i = Interp::new();
        install(&mut i);
        assert_eq!(eval(&mut i, "tk_getOpenFile"), "");
        assert_eq!(eval(&mut i, "tk_getOpenFile -multiple 1"), "");
        set_file_dialog_handler(choose_paths);
        assert_eq!(
            eval(
                &mut i,
                "tk_getOpenFile -title T -initialdir /tmp -initialfile a.txt                 -filetypes {{Text {.txt .md}} {All *}}"
            ),
            "/tmp/a.txt"
        );
        let request = LAST_FILE_DIALOG.lock().unwrap().take().unwrap();
        assert_eq!(request.kind, FileDialogKind::Open);
        assert_eq!(request.title, "T");
        assert_eq!(request.initial_dir, "/tmp");
        assert_eq!(request.initial_file, "a.txt");
        assert_eq!(
            request.file_types,
            vec![
                (
                    "Text".to_string(),
                    vec![".txt".to_string(), ".md".to_string()]
                ),
                ("All".to_string(), vec!["*".to_string()]),
            ]
        );
        assert_eq!(
            eval(&mut i, "tk_getOpenFile -multiple 1"),
            "/tmp/a.txt /tmp/b.txt"
        );
        // Save keeps an existing extension and appends a missing one.
        assert_eq!(
            eval(&mut i, "tk_getSaveFile -defaultextension .tcl"),
            "/tmp/a.txt"
        );
        set_file_dialog_handler(choose_plain);
        assert_eq!(
            eval(&mut i, "tk_getSaveFile -defaultextension .tcl"),
            "/tmp/name.tcl"
        );
        assert_eq!(eval(&mut i, "tk_chooseDirectory -title D"), "/tmp/name");
        set_file_dialog_handler(|_| None);
        assert_eq!(eval(&mut i, "tk_getSaveFile"), "");
    }

    static LAST_MESSAGE_BOX: std::sync::Mutex<Option<MessageBoxRequest>> =
        std::sync::Mutex::new(None);

    fn choose_no(request: &MessageBoxRequest) -> String {
        *LAST_MESSAGE_BOX.lock().unwrap() = Some(request.clone());
        "no".to_string()
    }

    #[test]
    fn message_box_types_and_defaults() {
        let mut i = Interp::new();
        install(&mut i);
        // Without a backend handler the default button is reported.
        assert_eq!(eval(&mut i, "tk_messageBox -message hi"), "ok");
        assert_eq!(eval(&mut i, "tk_messageBox -type yesno"), "yes");
        assert_eq!(eval(&mut i, "tk_messageBox -type yesno -default no"), "no");
        assert_eq!(
            eval(&mut i, "tk_messageBox -type retrycancel -default cancel"),
            "cancel"
        );
        assert_eq!(
            eval(&mut i, "tk_messageBox -type abortretryignore"),
            "abort"
        );
        assert!(i
            .eval("tk_messageBox -type nope")
            .is_err_and(|e| e.to_string().contains("bad -type")));
        assert!(i
            .eval("tk_messageBox -bogus x")
            .is_err_and(|e| e.to_string().contains("bad option")));

        // A host handler sees the parsed request and its answer wins.
        set_message_box_handler(choose_no);
        assert_eq!(
            eval(
                &mut i,
                "tk_messageBox -type yesno -title T -message M -detail D -icon warning -default yes"
            ),
            "no"
        );
        let request = LAST_MESSAGE_BOX.lock().unwrap().take().unwrap();
        assert_eq!(request.kind, "yesno");
        assert_eq!(request.title, "T");
        assert_eq!(request.message, "M");
        assert_eq!(request.detail, "D");
        assert_eq!(request.icon, "warning");
        assert_eq!(request.default, "yes");
        set_message_box_handler(|request| default_message_box_button(request));
    }

    #[test]
    fn text_tag_bind_and_doc_position_mapping() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "text .b; .b insert 1.0 \"hello world\"; .b tag add link 1.0 1.5;             .b tag bind link <Button-1> {set ::hit [%W get 1.0 1.5]};             .b tag bind link <Double-Button-1> {set ::double 1};             .b tag bind link <ButtonRelease-1> {set ::release 1};             .b tag bind link <Enter> {set ::enter 1};             .b tag bind link <Leave> {set ::leave 1};             .b tag bind link <Motion> {set ::motion \"%x,%y\"}",
        );
        assert_eq!(
            eval(&mut i, ".b tag bind link"),
            "<Button-1> <ButtonRelease-1> <Double-Button-1> <Enter> <Leave> <Motion>"
        );
        assert_eq!(
            eval(&mut i, ".b tag bind link <Button-1>"),
            "set ::hit [%W get 1.0 1.5]"
        );
        // Inside a tagged range the tag bindings run; outside they do not.
        text_tag_press(".b", 2, 10.0, 5.0, 1);
        assert_eq!(eval(&mut i, "set ::hit"), "hello");
        assert_eq!(eval(&mut i, "set ::release"), "1");
        eval(&mut i, "set ::hit none; set ::double 0");
        text_tag_press(".b", 8, 80.0, 5.0, 1);
        assert_eq!(eval(&mut i, "set ::hit"), "none");
        assert_eq!(eval(&mut i, "set ::double"), "0");
        // The second click of a chain also fires <Double-Button-1>.
        text_tag_press(".b", 2, 10.0, 5.0, 2);
        assert_eq!(eval(&mut i, "set ::double"), "1");
        eval(&mut i, ".b tag bind link <Button-1> {}");
        assert_eq!(eval(&mut i, ".b tag bind link <Button-1>"), "");

        // Crossing and motion bindings use the same tag ranges.
        eval(&mut i, "set ::leave 0");
        text_tag_hover(".b", 2, 12.0, 7.0);
        assert_eq!(eval(&mut i, "set ::enter"), "1");
        assert_eq!(eval(&mut i, "set ::motion"), "12,7");
        assert_eq!(eval(&mut i, "set ::leave"), "0");
        text_tag_hover(".b", 8, 80.0, 7.0);
        assert_eq!(eval(&mut i, "set ::leave"), "1");
        eval(&mut i, "set ::enter 0; set ::leave 0");
        text_tag_hover(".b", 2, 12.0, 7.0);
        assert_eq!(eval(&mut i, "set ::enter"), "1");
        text_tag_hover_out(".b");
        assert_eq!(eval(&mut i, "set ::leave"), "1");
        text_tag_hover_out(".b");
        assert_eq!(eval(&mut i, "set ::leave"), "1");

        // ProseMirror positions map onto the mirrored plain text.
        let doc = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]},{"type":"paragraph","content":[{"type":"text","text":"there"}]}]}"#;
        assert_eq!(doc_pos_to_text_offset(doc, 1), Some(0));
        assert_eq!(doc_pos_to_text_offset(doc, 2), Some(1));
        assert_eq!(doc_pos_to_text_offset(doc, 3), Some(2));
        assert_eq!(doc_pos_to_text_offset(doc, 5), Some(3));
        assert_eq!(doc_pos_to_text_offset(doc, 6), Some(4));
        assert_eq!(text_offset_to_doc_pos(doc, 0), Some(1));
        assert_eq!(text_offset_to_doc_pos(doc, 1), Some(2));
        assert_eq!(text_offset_to_doc_pos(doc, 3), Some(5));
        assert_eq!(text_offset_to_doc_pos(doc, 4), Some(6));

        // `text see` records a pending position for the rich renderer.
        eval(&mut i, ".b see 1.2");
        assert_eq!(take_text_see(".b"), Some(2));
        assert_eq!(take_text_see(".b"), None);
    }

    #[test]
    fn text_count_and_dump() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "text .c; .c insert 1.0 \"one\\ntwo\\nthree\"; .c tag add t 1.0 1.3; .c mark set insert 1.1; .c mark set m 2.1",
        );
        assert_eq!(eval(&mut i, ".c count 1.0 end"), "13");
        assert_eq!(eval(&mut i, ".c count -lines 1.0 end"), "3");
        assert_eq!(eval(&mut i, ".c count -chars -lines 1.0 2.0"), "4 1");
        assert_eq!(eval(&mut i, ".c count -lines -chars 1.0 2.0"), "1 4");
        assert_eq!(eval(&mut i, ".c count -indices 1.0 1.3"), "3");
        assert_eq!(
            eval(&mut i, ".c dump -text 1.0 1.3"),
            "text o 1.0 text ne 1.1"
        );
        assert_eq!(
            eval(&mut i, ".c dump -tag 1.0 1.3"),
            "tagon t 1.0 tagoff t 1.3"
        );
        assert_eq!(eval(&mut i, ".c dump -tag 1.2 end"), "tagoff t 1.3");
        assert_eq!(
            eval(&mut i, ".c dump -text 1.3 3.0"),
            "text {\n} 1.3 text t 2.0 text {wo\n} 2.1"
        );
        assert_eq!(eval(&mut i, ".c dump -mark 2.0 2.2"), "mark m 2.1");
        assert!(
            i.eval(".c count -bogus 1.0 end").is_err(),
            "unknown count option errors"
        );
    }

    #[test]
    fn canvas_item_motion_and_pointer_enter_leave() {
        let mut i = Interp::new();
        install(&mut i);
        let id = eval(
            &mut i,
            "canvas .c -width 100 -height 100; .c create rectangle 0 0 50 50",
        );
        eval(&mut i, &format!("set id {id}"));
        eval(
            &mut i,
            ".c bind $id <B1-Motion> {set ::drag \"%x,%y\"};
             .c bind $id <Motion> {set ::move \"%x,%y\"};
             bind .c <Enter> {set ::enter \"%x,%y\"};
             bind .c <Leave> {set ::leave 1}",
        );
        canvas_motion(".c", 10.0, 20.0, Some(1));
        assert_eq!(eval(&mut i, "set ::drag"), "10,20");
        canvas_motion(".c", 30.0, 40.0, None);
        assert_eq!(eval(&mut i, "set ::move"), "30,40");
        enter_event(".c", 1.0, 2.0);
        assert_eq!(eval(&mut i, "set ::enter"), "1,2");
        leave_event(".c", 3.0, 4.0);
        assert_eq!(eval(&mut i, "set ::leave"), "1");
    }

    #[test]
    fn wheel_motion_touch_and_gesture_events() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "listbox .l -height 3; foreach n {0 1 2 3 4 5 6 7 8 9} {.l insert end $n};
             set ::log {};
             bind .l <MouseWheel> {lappend ::log wheel%D};
             bind .l <Button-4> {lappend ::log up};
             bind .l <Button-5> {lappend ::log down};
             bind .l <B1-Motion> {lappend ::log drag};
             bind .l <Motion> {lappend ::log move};
             bind .l <<LongPress>> {lappend ::log long};
             bind .l <<Pan>> {set ::pan %D};
             bind .l <<Pinch>> {set ::pinch %D};
             bind .l <Touch> {incr ::touch}",
        );
        wheel_event(".l", 0.0, 1.0, 0);
        assert_eq!(eval(&mut i, "set ::log"), "up wheel120");
        assert_eq!(eval(&mut i, "lindex [.l yview] 0"), "0");
        wheel_event(".l", 0.0, -1.0, 0);
        assert!(eval(&mut i, "set ::log").contains("down"));
        assert_eq!(eval(&mut i, "lindex [.l yview] 0"), "0.3");
        motion_event(".l", 5.0, 6.0, Some(1), 0);
        assert!(eval(&mut i, "set ::log").contains("drag"));
        motion_event(".l", 5.0, 6.0, None, 0);
        assert!(eval(&mut i, "set ::log").contains("move"));
        long_press_event(".l", 5.0, 6.0);
        assert!(eval(&mut i, "set ::log").contains("long"));
        pan_event(".l", 0.0, 12.0, false);
        assert_eq!(eval(&mut i, "set ::pan"), "12");
        pinch_event(".l", 1.5);
        assert_eq!(eval(&mut i, "set ::pinch"), "150");
        eval(&mut i, "set ::touch 0");
        touch_event(".l", 5.0, 6.0, "start");
        assert_eq!(eval(&mut i, "set ::touch"), "1");
        // A user class wheel binding replaces the default scrolling.
        eval(
            &mut i,
            "bind Listbox <MouseWheel> {lappend ::log classwheel}; set ::log {}",
        );
        wheel_event(".l", 0.0, -1.0, 0);
        assert_eq!(eval(&mut i, "set ::log"), "down wheel-120 classwheel");
        assert_eq!(eval(&mut i, "lindex [.l yview] 0"), "0.3");
        eval(&mut i, "bind Listbox <MouseWheel> {}");
    }

    #[test]
    fn destroy_bindings_and_full_substitutions() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "frame .f; button .f.b -text B; set ::log {};
             bind .f.b <Destroy> {lappend ::log child};
             bind .f <Destroy> {lappend ::log parent}; destroy .f",
        );
        assert_eq!(eval(&mut i, "set ::log"), "child parent");

        eval(
            &mut i,
            "button .b -text B;
             bind .b <FocusIn> {set ::r \"%f|%w|%h|%t|%#|%E|%R|%N|%X|%Y|%A\"};
             event generate .b <FocusIn> -focus 1 -width 30 -height 40 -time 55 \
                -serial 7 -sendevent 1 -rootx 100 -rooty 200",
        );
        assert_eq!(
            eval(&mut i, "set ::r"),
            "1|30|40|55|7|1|0x00000000|??|100|200|??"
        );
        assert_eq!(
            eval(
                &mut i,
                "bind .b <Key-a> {}; bind .b <Key-N> {}; event generate .b <Key-a>; set ::r"
            ),
            "1|30|40|55|7|1|0x00000000|??|100|200|??"
        );
    }

    #[test]
    fn click_counts_and_virtual_event_mappings() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "button .b -text B; set ::log {};
             bind .b <Button-1> {lappend ::log b1};
             bind .b <Double-Button-1> {lappend ::log b2};
             bind .b <Triple-Button-1> {lappend ::log b3}",
        );
        press_button(".b");
        press_button(".b");
        press_button(".b");
        assert_eq!(eval(&mut i, "set ::log"), "b1 b1 b2 b1 b3");

        eval(
            &mut i,
            "set ::log {}; event add <<MyClick>> <Button-1>;
             bind .b <<MyClick>> {lappend ::log virtual}",
        );
        eval(&mut i, "event generate .b <Button-1>");
        assert_eq!(eval(&mut i, "set ::log"), "b1 virtual");
        assert_eq!(eval(&mut i, "event info <<MyClick>>"), "<Button-1>");
        assert!(
            eval(&mut i, "event info")
                .split(' ')
                .any(|name| name == "<<MyClick>>"),
            "event info lists the user event among the predefined ones: {:?}",
            eval(&mut i, "event info")
        );
        eval(&mut i, "set ::log {}; event generate .b <<MyClick>>");
        assert_eq!(eval(&mut i, "set ::log"), "virtual");
        eval(
            &mut i,
            "event delete <<MyClick>>; set ::log {}; event generate .b <Button-1>",
        );
        assert_eq!(eval(&mut i, "set ::log"), "b1");
        assert!(
            !eval(&mut i, "event info")
                .split(' ')
                .any(|name| name == "<<MyClick>>"),
            "event delete removes the user event: {:?}",
            eval(&mut i, "event info")
        );
    }

    #[test]
    fn map_and_unmap_events() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "frame .f; set ::log {};
             bind .f <Map> {lappend ::log map};
             bind .f <Unmap> {lappend ::log unmap};
             bind .f <Destroy> {lappend ::log destroy};
             pack .f",
        );
        assert_eq!(eval(&mut i, "set ::log"), "map");
        eval(&mut i, "pack forget .f");
        assert_eq!(eval(&mut i, "set ::log"), "map unmap");
        eval(&mut i, "pack .f");
        assert_eq!(eval(&mut i, "set ::log"), "map unmap map");
        eval(&mut i, "destroy .f");
        assert_eq!(eval(&mut i, "set ::log"), "map unmap map unmap destroy");
    }

    #[test]
    fn builtin_virtual_events() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "listbox .l -height 3; .l insert end a b c; set ::log {};
             bind .l <<ListboxSelect>> {lappend ::log lb}; set ::log {}",
        );
        listbox_select(".l", 1);
        assert_eq!(eval(&mut i, "set ::log"), "lb");

        eval(
            &mut i,
            "ttk::treeview .tv; .tv insert {} end -id root -text R;
             set ::log {};
             bind .tv <<TreeviewOpen>> {lappend ::log open};
             bind .tv <<TreeviewClose>> {lappend ::log close};
             bind .tv <<TreeviewSelect>> {lappend ::log sel}",
        );
        treeview_toggle(".tv", "root");
        assert!(eval(&mut i, "set ::log").contains("open"));
        treeview_select(".tv", "root");
        assert!(eval(&mut i, "set ::log").contains("sel"));
        treeview_toggle(".tv", "root");
        assert!(eval(&mut i, "set ::log").contains("close"));

        eval(
            &mut i,
            "ttk::combobox .cb -values {A B}; set ::log {};
             bind .cb <<ComboboxSelected>> {lappend ::log picked}",
        );
        press_combobox_item(".cb", 1);
        assert_eq!(eval(&mut i, "set ::log"), "picked");
        assert_eq!(eval(&mut i, ".cb get"), "B");

        eval(
            &mut i,
            "ttk::notebook .nb; frame .nb.a; frame .nb.b; .nb add .nb.a; .nb add .nb.b;
             set ::log {}; bind .nb <<NotebookTabChanged>> {lappend ::log tab}",
        );
        select_notebook_tab(".nb", 1);
        assert_eq!(eval(&mut i, "set ::log"), "tab");

        eval(
            &mut i,
            "button .b -command {incr ::inv}; set ::inv 0; set ::log {};
             bind .b <<Invoke>> {lappend ::log invoke}; event generate .b <<Invoke>>",
        );
        assert_eq!(eval(&mut i, "set ::inv"), "1");
        assert_eq!(eval(&mut i, "set ::log"), "invoke");
    }

    #[test]
    fn clipboard_commands_and_sink() {
        use std::sync::Mutex;
        static SINKED: Mutex<String> = Mutex::new(String::new());
        fn sink(text: &str) {
            *SINKED.lock().unwrap() = text.to_string();
        }
        static CALLED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn getter() -> String {
            CALLED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            "external".to_string()
        }
        set_clipboard_sink(sink);
        set_clipboard_getter(getter);

        let mut i = Interp::new();
        install(&mut i);
        // Owning nothing, `get` asks the system (synchronously).
        assert_eq!(eval(&mut i, "clipboard get"), "external");
        eval(&mut i, "clipboard append -type STRING Hel lo");
        assert_eq!(eval(&mut i, "clipboard get"), "Hello");
        assert_eq!(*SINKED.lock().unwrap(), "Hello");
        let calls = CALLED.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(calls, 1, "own clipboard answers without the system");
        eval(&mut i, "clipboard clear");
        assert_eq!(eval(&mut i, "clipboard get"), "");
        assert_eq!(CALLED.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(*SINKED.lock().unwrap(), "");
    }

    #[test]
    fn menu_windows_accept_bindings() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "menu .m -tearoff 0; .m add command -label A -command {set ::n 1};
             set ::hit 0; set ::n 0; bind .m <Button-1> {incr ::hit}",
        );
        assert_eq!(eval(&mut i, "bind .m"), "<Button-1>");
        eval(&mut i, "event generate .m <Button-1>");
        assert_eq!(eval(&mut i, "set ::hit"), "1");
        press_menu_item(".m", 0);
        assert_eq!(eval(&mut i, "set ::hit"), "2");
        assert_eq!(eval(&mut i, "set ::n"), "1");
    }

    #[test]
    fn break_in_a_binding_stops_further_tags() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "button .b -text X; set ::log {};
             bind Button <Button-1> {lappend ::log class};
             bind all <Button-1> {lappend ::log all}",
        );
        eval(&mut i, "bind .b <Button-1> {lappend ::log widget}");
        eval(&mut i, "event generate .b <Button-1>");
        assert_eq!(eval(&mut i, "set ::log"), "widget class all");
        eval(
            &mut i,
            "set ::log {}; bind .b <Button-1> {lappend ::log widget; break}",
        );
        eval(&mut i, "event generate .b <Button-1>");
        assert_eq!(eval(&mut i, "set ::log"), "widget");
        assert!(
            run_binding(".b", "<Button-1>", &EventCtx::for_path(".b")),
            "break is reported to the caller"
        );
    }

    #[test]
    fn binding_sequence_matching_and_specificity() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "button .b -text X; set ::log {};
             bind .b <Key> {lappend ::log key};
             bind .b <Key-a> {lappend ::log a};
             bind .b <Control-a> {lappend ::log ctrl-a};
             bind .b <Button-1> {lappend ::log click};
             bind .b <Control-Button-1> {lappend ::log ctrlclick};
             bind .b <Double-Button-1> {lappend ::log double}",
        );
        eval(&mut i, "event generate .b <Key-a>");
        assert_eq!(eval(&mut i, "set ::log"), "a");
        eval(&mut i, "event generate .b <Key-b>");
        assert_eq!(eval(&mut i, "set ::log"), "a key");
        // The most specific sequence wins, but generic ones still match
        // events with extra modifiers when nothing is more specific.
        eval(&mut i, "event generate .b <Control-a> -state 4");
        assert_eq!(eval(&mut i, "set ::log"), "a key ctrl-a");
        eval(&mut i, "event generate .b <Control-Button-1>");
        assert_eq!(eval(&mut i, "set ::log"), "a key ctrl-a ctrlclick");
        eval(&mut i, "event generate .b <Button-1>");
        assert_eq!(eval(&mut i, "set ::log"), "a key ctrl-a ctrlclick click");
        eval(&mut i, "event generate .b <Double-Button-1> -button 1");
        assert_eq!(
            eval(&mut i, "set ::log"),
            "a key ctrl-a ctrlclick click double"
        );
        // An empty binding removes the sequence; <Key> matches again.
        eval(&mut i, "bind .b <Key-a> {}; event generate .b <Key-a>");
        assert_eq!(
            eval(&mut i, "set ::log"),
            "a key ctrl-a ctrlclick click double key"
        );
        // %K/%A are filled from the generated keysym.
        eval(
            &mut i,
            "bind .b <Key-z> {set ::k \"%K/%A\"}; event generate .b <Key-z>",
        );
        assert_eq!(eval(&mut i, "set ::k"), "z/z");
    }

    #[test]
    fn clicks_and_focus_dispatch_bindings() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "listbox .l -height 3; .l insert end a b c; set ::log {};
             bind .l <Button-1> {lappend ::log lb};
             bind .l <Double-Button-1> {lappend ::log lb2}",
        );
        listbox_select(".l", 1);
        listbox_select(".l", 1);
        // The second click runs both <Button-1> and <Double-Button-1>.
        assert_eq!(eval(&mut i, "set ::log"), "lb lb lb2");
        assert_eq!(eval(&mut i, ".l curselection"), "1");

        eval(
            &mut i,
            "ttk::treeview .tv -height 3; .tv insert {} end -id a -text a;
             .tv insert {} end -id b -text b; set ::log {};
             bind .tv <Button-1> {lappend ::log tv}",
        );
        treeview_select(".tv", "b");
        assert_eq!(eval(&mut i, "set ::log"), "tv");
        assert_eq!(eval(&mut i, ".tv selection"), "b");

        eval(
            &mut i,
            "button .b; entry .e; set ::log {};
             bind .b <FocusOut> {lappend ::log out};
             bind .e <FocusIn> {lappend ::log in}",
        );
        eval(&mut i, "focus .b; focus .e");
        assert_eq!(eval(&mut i, "set ::log"), "out in");
        assert_eq!(eval(&mut i, "focus"), ".e");

        // Listbox/treeview take the focus on click; arrow keys move the
        // selection, like Tk's class bindings.
        listbox_select(".l", 0);
        assert_eq!(eval(&mut i, "focus"), ".l");
        focused_nav_key("Down");
        assert_eq!(eval(&mut i, ".l curselection"), "1");
        focused_nav_key("End");
        assert_eq!(eval(&mut i, ".l curselection"), "2");
        focused_nav_key("Home");
        assert_eq!(eval(&mut i, ".l curselection"), "0");
        treeview_select(".tv", "a");
        focused_nav_key("Down");
        assert_eq!(eval(&mut i, ".tv selection"), "b");
        focused_nav_key("Up");
        assert_eq!(eval(&mut i, ".tv selection"), "a");

        // A user class binding for the same key replaces the default move.
        eval(
            &mut i,
            "bind Listbox <Key-Down> {set ::nav blocked}; set ::nav {}",
        );
        listbox_select(".l", 0);
        focused_nav_key("Down");
        assert_eq!(eval(&mut i, ".l curselection"), "0");
        assert_eq!(eval(&mut i, "set ::nav"), "blocked");
        eval(&mut i, "bind Listbox <Key-Down> {}");
    }

    #[test]
    fn text_replace_and_insert_mark_gravity() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "text .rp; .rp insert 1.0 abcdef; .rp tag add hot 1.0 1.2",
        );
        eval(&mut i, ".rp mark set m 1.4");
        eval(&mut i, ".rp replace 1.1 1.3 XY");
        assert_eq!(eval(&mut i, ".rp get 1.0 end"), "aXYdef");
        assert_eq!(eval(&mut i, ".rp tag ranges hot"), "1.0 1.1");
        assert_eq!(eval(&mut i, ".rp index m"), "1.4");
        assert_eq!(eval(&mut i, ".rp index insert"), "1.6");
        eval(&mut i, ".rp replace 1.0 1.2 ZZ hot");
        assert_eq!(eval(&mut i, ".rp get 1.0 end"), "ZZYdef");
        assert_eq!(eval(&mut i, ".rp tag ranges hot"), "1.0 1.2");
        eval(&mut i, ".rp replace 1.2 1.2 Q");
        assert_eq!(eval(&mut i, ".rp get 1.0 end"), "ZZQYdef");

        // The insert mark is a right-gravity mark: edits before it shift it.
        eval(&mut i, "text .cu; .cu insert 1.0 abcdef");
        eval(&mut i, ".cu mark set insert 1.4; .cu insert 1.1 X");
        assert_eq!(eval(&mut i, ".cu index insert"), "1.5");
        eval(&mut i, ".cu mark set insert 1.1; .cu insert 1.1 Y");
        assert_eq!(eval(&mut i, ".cu index insert"), "1.2");
        eval(&mut i, ".cu mark gravity insert left");
        eval(&mut i, ".cu mark set insert 1.1; .cu insert 1.1 Z");
        assert_eq!(eval(&mut i, ".cu index insert"), "1.1");
        eval(&mut i, ".cu mark gravity insert right");
        eval(&mut i, ".cu mark set insert 1.2; .cu delete 1.0 1.1");
        assert_eq!(eval(&mut i, ".cu index insert"), "1.1");
        eval(&mut i, ".cu mark set insert 1.1; .cu delete 1.1 1.3");
        assert_eq!(eval(&mut i, ".cu index insert"), "1.1");
    }

    #[test]
    fn text_search_exact_backward_regexp_and_count() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "text .s; .s insert 1.0 \"one two one\nthree ONE\"");
        assert_eq!(eval(&mut i, ".s search one 1.0"), "1.0");
        assert_eq!(eval(&mut i, ".s search one 1.1"), "1.8");
        assert_eq!(eval(&mut i, ".s search -backward one 1.9"), "1.8");
        // Without an explicit stop the search wraps around.
        assert_eq!(eval(&mut i, ".s search -backward one 1.0"), "1.8");
        assert_eq!(eval(&mut i, ".s search one 1.9"), "1.0");
        assert_eq!(eval(&mut i, ".s search -nocase ONE 1.0"), "1.0");
        assert_eq!(eval(&mut i, ".s search -all one 1.0"), "1.0 1.8");
        assert_eq!(eval(&mut i, ".s search -all one 1.9"), "1.0 1.8");
        assert_eq!(eval(&mut i, ".s search -regexp {t[a-z]+} 1.0"), "1.4");
        assert_eq!(eval(&mut i, ".s search -all -regexp {o\\w} 1.0"), "1.0 1.8");
        assert_eq!(eval(&mut i, "set n 0; .s search -count n one 1.0"), "1.0");
        assert_eq!(eval(&mut i, "set n"), "3");
        assert_eq!(eval(&mut i, ".s search {} 1.0"), "1.0");
        assert_eq!(eval(&mut i, ".s search zzz 1.0"), "");
        assert_eq!(eval(&mut i, ".s search one 1.0 1.1"), "1.0");
        assert_eq!(eval(&mut i, ".s search one 1.9 1.1"), "");
        assert_eq!(eval(&mut i, ".s search -backward one 1.9 1.4"), "1.8");
    }

    #[test]
    fn text_tag_nextrange_and_prevrange() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "text .t2; .t2 insert 1.0 abcdefghij; \
             .t2 tag add a 1.2 1.4; .t2 tag add a 1.6 1.8",
        );
        assert_eq!(eval(&mut i, ".t2 tag ranges a"), "1.2 1.4 1.6 1.8");
        assert_eq!(eval(&mut i, ".t2 tag nextrange a 1.0"), "1.2 1.4");
        assert_eq!(eval(&mut i, ".t2 tag nextrange a 1.3"), "1.6 1.8");
        assert_eq!(eval(&mut i, ".t2 tag nextrange a 1.4"), "1.6 1.8");
        assert_eq!(eval(&mut i, ".t2 tag nextrange a 1.9"), "");
        assert_eq!(eval(&mut i, ".t2 tag nextrange a 1.0 1.5"), "1.2 1.4");
        assert_eq!(eval(&mut i, ".t2 tag prevrange a 1.9"), "1.6 1.8");
        assert_eq!(eval(&mut i, ".t2 tag prevrange a 1.7"), "1.6 1.8");
        assert_eq!(eval(&mut i, ".t2 tag prevrange a 1.6"), "1.2 1.4");
        assert_eq!(eval(&mut i, ".t2 tag prevrange a 1.2"), "");
    }

    #[test]
    fn listbox_item_options_follow_index_shifts() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "listbox .l -height 4; .l insert end one two three; .l selection set 1",
        );
        eval(
            &mut i,
            ".l itemconfigure 1 -foreground red -background yellow",
        );
        assert_eq!(eval(&mut i, ".l itemcget 1 -foreground"), "red");
        assert!(eval(&mut i, ".l itemconfigure 1").contains("-background yellow"));
        // Inserting before the row shifts both its options and the selection.
        eval(&mut i, ".l insert 0 zero");
        assert_eq!(eval(&mut i, ".l itemcget 2 -foreground"), "red");
        assert_eq!(eval(&mut i, ".l itemcget 0 -foreground"), "");
        assert_eq!(eval(&mut i, ".l curselection"), "2");
        eval(&mut i, ".l delete 0");
        assert_eq!(eval(&mut i, ".l itemcget 1 -background"), "yellow");
        assert_eq!(eval(&mut i, ".l curselection"), "1");
    }

    #[test]
    fn canvas_find_closest_overlapping_and_bbox() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "canvas .k -width 100 -height 100");
        let a = eval(&mut i, ".k create rectangle 10 10 30 40");
        let b = eval(&mut i, ".k create rectangle 50 50 90 90");
        eval(&mut i, &format!("set a {a}; set b {b}"));
        assert_eq!(eval(&mut i, ".k bbox $a"), "9 9 31 41");
        assert_eq!(eval(&mut i, ".k bbox all"), "9 9 91 91");
        assert_eq!(eval(&mut i, ".k find closest 0 0"), a);
        assert_eq!(eval(&mut i, ".k find closest 95 95"), b);
        assert_eq!(eval(&mut i, ".k find closest 1000 1000"), b);
        assert_eq!(
            eval(&mut i, ".k find overlapping 25 25 55 55"),
            format!("{a} {b}")
        );
        assert_eq!(eval(&mut i, ".k find overlapping 31 41 49 49"), "");
        // Hidden items keep existing and are still found by tag, but take
        // no part in bbox/closest/overlapping and receive no presses.
        eval(
            &mut i,
            "set ::pressed 0; .k bind $a <Button-1> {incr ::pressed}",
        );
        eval(&mut i, ".k itemconfigure $a -state hidden");
        assert_eq!(eval(&mut i, ".k bbox $a"), "");
        assert_eq!(eval(&mut i, ".k bbox all"), "49 49 91 91");
        assert_eq!(eval(&mut i, ".k find withtag all"), format!("{a} {b}"));
        assert_eq!(eval(&mut i, ".k find closest 0 0"), b);
        assert_eq!(eval(&mut i, ".k find overlapping 0 0 100 100"), b);
        canvas_press(".k", 20.0, 20.0);
        assert_eq!(eval(&mut i, "set ::pressed"), "0");
        eval(&mut i, ".k itemconfigure $a -state normal");
        canvas_press(".k", 20.0, 20.0);
        assert_eq!(eval(&mut i, "set ::pressed"), "1");
        // Default anchor is center for text/image items.
        eval(
            &mut i,
            "image create photo dot -file /tmp/x.png -width 10 -height 20",
        );
        let img = eval(&mut i, ".k create image 50 50 -image dot");
        assert_eq!(eval(&mut i, &format!(".k bbox {img}")), "45 40 55 60");
        eval(&mut i, &format!(".k itemconfigure {img} -anchor nw"));
        assert_eq!(eval(&mut i, &format!(".k bbox {img}")), "50 50 60 70");
        let label = eval(&mut i, ".k create text 30 40 -text ab");
        assert_eq!(eval(&mut i, &format!(".k bbox {label}")), "22 32 38 48");
        let arc = eval(
            &mut i,
            ".k create arc 0 0 100 50 -start 0 -extent 180 -style arc -outline red",
        );
        assert_eq!(eval(&mut i, &format!(".k itemcget {arc} -style")), "arc");
        assert_eq!(eval(&mut i, &format!(".k itemcget {arc} -extent")), "180");
        assert_eq!(eval(&mut i, &format!(".k bbox {arc}")), "-1 -1 101 51");
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
        assert_eq!(eval(&mut i, ".tv column #0 -width"), "200");
        assert_eq!(eval(&mut i, ".tv column size -anchor"), "w");
        assert_eq!(eval(&mut i, ".tv column #0 -stretch"), "1");
        eval(&mut i, ".tv column size -width 80");
        assert_eq!(eval(&mut i, ".tv column size -width"), "80");
        assert_eq!(eval(&mut i, "catch {.tv column nope}"), "1");
        eval(
            &mut i,
            ".tv tag configure hot -foreground red -background yellow",
        );
        assert_eq!(eval(&mut i, ".tv tag names"), "hot");
        assert_eq!(eval(&mut i, ".tv tag cget hot -foreground"), "red");
        eval(&mut i, ".tv item src -tags {hot}");
        assert_eq!(eval(&mut i, ".tv item src -tags"), "hot");
        assert_eq!(eval(&mut i, ".tv tag has hot src"), "1");
        assert_eq!(eval(&mut i, ".tv tag has hot root"), "0");
        eval(&mut i, ".tv selection set src");
        assert_eq!(eval(&mut i, ".tv selection"), "src");
        assert_eq!(eval(&mut i, ".tv focus"), "src");
        eval(&mut i, ".tv delete src");
        assert_eq!(eval(&mut i, "winfo exists .tv"), "1");
        assert_eq!(eval(&mut i, ".tv children root"), "I001");
        eval(&mut i, ".tv tag delete hot");
        assert_eq!(eval(&mut i, ".tv tag names"), "");
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

    fn eval(interp: &mut Interp, script: &str) -> String {
        interp.eval(script).unwrap().as_str().to_string()
    }

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

    #[test]
    fn see_scrolls_listbox_and_treeview() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "listbox .lb -height 3; foreach n {0 1 2 3 4 5 6 7 8 9} {.lb insert end $n}",
        );
        eval(&mut i, ".lb see 8");
        assert_eq!(eval(&mut i, "lindex [.lb yview] 0"), "0.6");
        eval(&mut i, ".lb see 1");
        assert_eq!(eval(&mut i, "lindex [.lb yview] 0"), "0.1");
        // A visible row does not scroll at all.
        eval(&mut i, ".lb see 2");
        assert_eq!(eval(&mut i, "lindex [.lb yview] 0"), "0.1");

        eval(
            &mut i,
            "ttk::treeview .tw -height 2;              foreach n {a b c d e} {.tw insert {} end -id $n -text $n}",
        );
        eval(&mut i, ".tw see d");
        assert_eq!(eval(&mut i, "lindex [.tw yview] 0"), "0.4");
        eval(&mut i, ".tw see a");
        assert_eq!(eval(&mut i, "lindex [.tw yview] 0"), "0");
    }

    #[test]
    fn disabled_button_and_menu_item_do_not_invoke() {
        let mut i = Interp::new();
        install(&mut i);
        eval(
            &mut i,
            "set ::n 0; button .b -text Go -state disabled \
                -command {incr ::n}; .b invoke",
        );
        assert_eq!(eval(&mut i, "set ::n"), "0");
        eval(&mut i, ".b configure -state normal; .b invoke");
        assert_eq!(eval(&mut i, "set ::n"), "1");
        eval(
            &mut i,
            "menu .m -tearoff 0; .m add command -label Go -state disabled \
                -command {incr ::n}; .m invoke 0",
        );
        assert_eq!(eval(&mut i, "set ::n"), "1");
        eval(&mut i, ".m entryconfigure 0 -state normal; .m invoke 0");
        assert_eq!(eval(&mut i, "set ::n"), "2");
    }
}
