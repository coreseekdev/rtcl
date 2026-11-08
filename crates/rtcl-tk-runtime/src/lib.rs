//! Tcl/Tk-compatible native UI runtime.
//!
//! This crate is the bridge between the Tcl `tk` command surface and the
//! backend-independent widget model in [`rtcl_tk_core`].  It is enabled by
//! the native-only `tk` feature; the web/AI `ui::*` DSL is a separate track
//! (`docs/UI-DSL.md`) and never depends on this crate.
//!
//! M1 covers the shadow tree and the core Tk commands: widget creation,
//! `configure`/`cget`, `pack`/`grid`, `destroy`, `winfo`, `wm`, and the
//! per-widget command registered under its path.  A renderer backend (nsh)
//! consumes [`TkRuntime::with_state`] snapshots and pushes user events back
//! through the Tcl event queue.

#![cfg_attr(target_arch = "wasm32", allow(unused))]

#[cfg(target_arch = "wasm32")]
compile_error!(
    "rtcl-tk-runtime (native Tk/nsh) is native-only; it is mutually \
     exclusive with the wasm target. Use ui::* for web/WebView UI instead."
);

use std::cell::RefCell;
use std::collections::HashMap;

use rtcl_core::interp::{add_interp_teardown_hook, Interp};
use rtcl_core::value::Value;
use rtcl_core::{Error, Result};

pub use rtcl_tk_core::{
    Fill, GridInfo, PackInfo, PackSide, Sticky, TkError, TkTree, WidgetKind,
};

#[derive(Debug, Default)]
pub struct TkRuntime {
    tree: TkTree,
}

impl TkRuntime {
    pub fn new() -> Self {
        Self { tree: TkTree::new() }
    }

    pub fn tree(&self) -> &TkTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut TkTree {
        &mut self.tree
    }
}

thread_local! {
    /// One Tk state per live interpreter (Tcl is single-threaded).
    static STATES: RefCell<HashMap<usize, TkRuntime>> =
        RefCell::new(HashMap::new());
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
    STATES.with(|s| s.borrow_mut().get_mut(&key(interp)).map(f))
}

/// Register the M1 Tk command surface on `interp` and create its tree.
pub fn install(interp: &mut Interp) {
    ensure_hook();
    STATES.with(|s| {
        s.borrow_mut().entry(key(interp)).or_default();
    });
    for name in [
        "toplevel", "frame", "label", "button", "entry", "checkbutton",
        "radiobutton", "scale", "text", "listbox",
    ] {
        interp.register_command(name, create_widget);
    }
    interp.register_command(".", widget_command);
    for name in ["configure", "cget", "destroy", "pack", "grid", "winfo", "wm", "tk"] {
        interp.register_command(name, global_command);
    }
}

fn tk_error(e: TkError) -> Error {
    Error::runtime(e.to_string(), rtcl_core::error::ErrorCode::Generic)
}

fn wrong(cmd: &str, usage: &str, actual: usize) -> Error {
    Error::wrong_args_with_usage(cmd, 2, actual, usage)
}

fn parse_pairs(args: &[Value], start: usize, what: &str) -> Result<Vec<(String, String)>> {
    let rest = &args[start..];
    if !rest.len().is_multiple_of(2) {
        return Err(wrong(what, "?option value ...?", args.len()));
    }
    let mut out = Vec::with_capacity(rest.len() / 2);
    for pair in rest.chunks(2) {
        let name = pair[0].as_str();
        if !rtcl_tk_core::valid_option(name) {
            return Err(Error::runtime(
                format!("bad option \"{name}\": must be a -option"),
                rtcl_core::error::ErrorCode::Generic,
            ));
        }
        out.push((name.to_string(), pair[1].as_str().to_string()));
    }
    Ok(out)
}

fn parse_int(v: &Value) -> Result<i64> {
    v.as_int().ok_or_else(|| {
        Error::runtime(format!("expected integer but got \"{}\"", v.as_str()), rtcl_core::error::ErrorCode::Generic)
    })
}

fn parse_float(v: &Value, what: &str) -> Result<f64> {
    if let Some(i) = v.as_int() {
        return Ok(i as f64);
    }
    v.as_float().ok_or_else(|| {
        Error::runtime(format!("bad {what} \"{}\": expected number", v.as_str()), rtcl_core::error::ErrorCode::Generic)
    })
}

fn parse_bool(v: &Value, what: &str) -> Result<bool> {
    match v.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        s => Err(Error::runtime(
            format!("bad {what} \"{s}\": must be boolean"),
            rtcl_core::error::ErrorCode::Generic,
        )),
    }
}

fn create_widget(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let cmd = args[0].as_str();
    let kind = WidgetKind::from_tk_name(cmd)
        .ok_or_else(|| Error::invalid_command(cmd))?;
    if args.len() < 2 {
        return Err(wrong(cmd, "pathName ?-option value ...?", args.len()));
    }
    let path = args[1].as_str();
    let options = parse_pairs(args, 2, cmd)?;
    STATES.with(|s| {
        let mut s = s.borrow_mut();
        let rt = s.entry(key(interp)).or_default();
        rt.tree
            .create_at(path, kind)
            .map_err(tk_error)?;
        rt.tree.configure(path, options).map_err(tk_error)?;
        Ok(())
    })?;
    interp.register_command(path, widget_command);
    Ok(Value::from_str(path))
}

fn widget_command(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let path = args[0].as_str();
    if args.len() < 2 {
        return Err(wrong(path, "subcommand ?arg ...?", args.len()));
    }
    let sub = args[1].as_str();
    match sub {
        "cget" => cget_impl(interp, path, &args[2..]),
        "configure" | "config" => configure_impl(interp, path, &args[2..]),
        "destroy" => destroy_impl(interp, path),
        _ => Err(Error::runtime(
            format!("bad option \"{sub}\": must be cget, configure, or destroy"),
            rtcl_core::error::ErrorCode::Generic,
        )),
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
        _ => Err(Error::invalid_command(cmd)),
    }
}

fn cget_impl(interp: &Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.len() != 1 {
        return Err(wrong("cget", "window -option", rest.len() + 2));
    }
    let option = rest[0].as_str();
    let value = STATES.with(|s| {
        s.borrow()
            .get(&key(interp))
            .and_then(|rt| rt.tree.cget(path, option).ok())
            .flatten()
            .map(str::to_string)
    });
    match value {
        Some(v) => Ok(Value::from_str(&v)),
        None => {
            if with_state(interp, |rt| rt.tree.contains(path)).unwrap_or(false) {
                Err(Error::runtime(
                    format!("unknown option \"{option}\""),
                    rtcl_core::error::ErrorCode::Generic,
                ))
            } else {
                Err(Error::runtime(
                    format!("bad window path name \"{path}\""),
                    rtcl_core::error::ErrorCode::Generic,
                ))
            }
        }
    }
}

fn configure_impl(interp: &mut Interp, path: &str, rest: &[Value]) -> Result<Value> {
    if rest.is_empty() {
        let pairs = STATES.with(|s| {
            s.borrow()
                .get(&key(interp))
                .and_then(|rt| rt.tree.get(path))
                .map(|n| {
                    n.options
                        .iter()
                        .flat_map(|(k, v)| [Value::from_str(k), Value::from_str(v)])
                        .collect::<Vec<_>>()
                })
        });
        return Ok(match pairs {
            Some(p) => Value::from_list(&p),
            None => {
                return Err(Error::runtime(
                    format!("bad window path name \"{path}\""),
                    rtcl_core::error::ErrorCode::Generic,
                ))
            }
        });
    }
    if rest.len() == 1 && rest[0].as_str().starts_with('-') {
        return cget_impl(interp, path, rest);
    }
    let options = parse_pairs(rest, 0, "configure")?;
    STATES.with(|s| {
        s.borrow_mut()
            .get_mut(&key(interp))
            .ok_or_else(|| {
                Error::runtime(
                    format!("bad window path name \"{path}\""),
                    rtcl_core::error::ErrorCode::Generic,
                )
            })?
            .tree
            .configure(path, options)
            .map_err(tk_error)
    })?;
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
        return Err(Error::runtime(
            format!("bad window path name \"{path}\""),
            rtcl_core::error::ErrorCode::Generic,
        ));
    }
    STATES.with(|s| {
        s.borrow_mut()
            .get_mut(&key(interp))
            .map(|rt| rt.tree.destroy(path))
    })
    .unwrap_or(Ok(()))
    .map_err(tk_error)?;
    for p in removed {
        let _ = interp.delete_command(&p);
    }
    Ok(Value::empty())
}

fn pack_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("pack", "window ?-option value ...?", args.len()));
    }
    let path = args[1].as_str();
    let options = parse_pairs(args, 2, "pack")?;
    let mut info = PackInfo::default();
    for (k, v) in options {
        match k.as_str() {
            "-side" => {
                info.side = match v.as_str() {
                    "top" => PackSide::Top,
                    "bottom" => PackSide::Bottom,
                    "left" => PackSide::Left,
                    "right" => PackSide::Right,
                    _ => return Err(Error::runtime(format!("bad side \"{v}\""), rtcl_core::error::ErrorCode::Generic)),
                }
            }
            "-fill" => {
                info.fill = match v.as_str() {
                    "none" => Fill::None,
                    "x" => Fill::X,
                    "y" => Fill::Y,
                    "both" => Fill::Both,
                    _ => return Err(Error::runtime(format!("bad fill \"{v}\""), rtcl_core::error::ErrorCode::Generic)),
                }
            }
            "-expand" => info.expand = parse_bool(&Value::from_str(&v), "expand")?,
            "-padx" => info.padx = parse_float(&Value::from_str(&v), "padx")?,
            "-pady" => info.pady = parse_float(&Value::from_str(&v), "pady")?,
            _ => return Err(Error::runtime(format!("unknown option \"{k}\""), rtcl_core::error::ErrorCode::Generic)),
        }
    }
    STATES.with(|s| {
        s.borrow_mut()
            .get_mut(&key(interp))
            .ok_or_else(|| Error::runtime(format!("bad window path name \"{path}\""), rtcl_core::error::ErrorCode::Generic))?
            .tree
            .pack(path, info)
            .map_err(tk_error)
    })?;
    Ok(Value::empty())
}

fn grid_impl(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(wrong("grid", "window ?-option value ...?", args.len()));
    }
    let path = args[1].as_str();
    let options = parse_pairs(args, 2, "grid")?;
    let mut info = GridInfo::default();
    for (k, v) in options {
        match k.as_str() {
            "-row" => info.row = parse_int(&Value::from_str(&v))?,
            "-column" => info.column = parse_int(&Value::from_str(&v))?,
            "-rowspan" => info.rowspan = parse_int(&Value::from_str(&v))?,
            "-columnspan" => info.columnspan = parse_int(&Value::from_str(&v))?,
            "-padx" => info.padx = parse_float(&Value::from_str(&v), "padx")?,
            "-pady" => info.pady = parse_float(&Value::from_str(&v), "pady")?,
            "-sticky" => {
                info.sticky = Sticky {
                    left: v.contains('w'),
                    right: v.contains('e'),
                    top: v.contains('n'),
                    bottom: v.contains('s'),
                }
            }
            _ => return Err(Error::runtime(format!("unknown option \"{k}\""), rtcl_core::error::ErrorCode::Generic)),
        }
    }
    STATES.with(|s| {
        s.borrow_mut()
            .get_mut(&key(interp))
            .ok_or_else(|| Error::runtime(format!("bad window path name \"{path}\""), rtcl_core::error::ErrorCode::Generic))?
            .tree
            .grid(path, info)
            .map_err(tk_error)
    })?;
    Ok(Value::empty())
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
                .ok_or_else(|| Error::runtime(format!("bad window path name \"{path}\""), rtcl_core::error::ErrorCode::Generic))?;
            Ok(Value::from_list(&kids.iter().map(|s| Value::from_str(s)).collect::<Vec<_>>()))
        }
        "exists" => {
            if args.len() != 3 {
                return Err(wrong("winfo exists", "window", args.len()));
            }
            let path = args[2].as_str();
            Ok(Value::from_bool(with_state(interp, |rt| rt.tree.contains(path)).unwrap_or(false)))
        }
        "class" => {
            if args.len() != 3 {
                return Err(wrong("winfo class", "window", args.len()));
            }
            let path = args[2].as_str();
            let kind = with_state(interp, |rt| rt.tree.get(path).map(|n| format!("{:?}", n.kind)))
                .flatten()
                .ok_or_else(|| Error::runtime(format!("bad window path name \"{path}\""), rtcl_core::error::ErrorCode::Generic))?;
            Ok(Value::from_str(&kind))
        }
        "width" | "height" => {
            if args.len() != 3 {
                return Err(wrong(&format!("winfo {sub}"), "window", args.len()));
            }
            // Geometry is backend-owned; M1 reports 0 until a frame is laid out.
            if with_state(interp, |rt| rt.tree.contains(args[2].as_str())).unwrap_or(false) {
                Ok(Value::from_int(0))
            } else {
                Err(Error::runtime(format!("bad window path name \"{}\"", args[2].as_str()), rtcl_core::error::ErrorCode::Generic))
            }
        }
        _ => Err(Error::runtime(format!("bad option \"{sub}\": must be children, class, exists, height, or width"), rtcl_core::error::ErrorCode::Generic)),
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
    let path = args[2].as_str();
    let option = format!("-{sub}");
    match sub {
        "title" | "geometry" | "resizable" => {
            if args.len() == 3 {
                return cget_impl(interp, path, &[Value::from_str(&option)]);
            }
            let value: String = args[3..]
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            configure_impl(interp, path, &[Value::from_str(&option), Value::from_str(&value)])?;
            Ok(Value::empty())
        }
        "withdraw" | "deiconify" | "iconify" => {
            if args.len() != 3 {
                return Err(wrong(&format!("wm {sub}"), "window", args.len()));
            }
            configure_impl(interp, path, &[Value::from_str(&option), Value::from_str("1")])?;
            Ok(Value::empty())
        }
        _ => Err(Error::runtime(format!("bad option \"{sub}\""), rtcl_core::error::ErrorCode::Generic)),
    }
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
        other => Err(Error::runtime(
            format!("bad option \"{other}\": must be patchlevel or windowingsystem"),
            rtcl_core::error::ErrorCode::Generic,
        )),
    }
}

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
    }

    #[test]
    fn hierarchy_and_pack_grid() {
        let mut i = Interp::new();
        install(&mut i);
        eval(&mut i, "frame .f; button .f.b -text X; pack .f -side left -expand 1");
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
    }
}
