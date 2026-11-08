//! nsh renderer for the native Tk shadow tree.
//!
//! The Tcl command layer lives in `rtcl-tk-runtime`; this crate turns a
//! [`TkTree`] snapshot into nsh elements.  It is deliberately a thin
//! projection: widget semantics (paths/options/pack/grid/bindings) stay in
//! `rtcl-tk-core`/`rtcl-tk-runtime`, and the nsh platform crates own the
//! actual native views.
//!
//! Every Tk mutation bumps [`TkRevision`], which the `TkView` component
//! reads; the whole tree is rebuilt from the snapshot.  Button presses,
//! entry edits, checks, scales and list selections call back into the
//! runtime, which evaluates Tcl and bumps the revision again.
//!
//! A 16 ms interval pumps Tcl `after` timers while any are queued, so
//! `after 100 {.l configure -text Later}` works without the script running
//! its own event loop.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::Duration;

use nsh_base::SliderValue;
use nsh_core::geometry::Axis;
use nsh_core::prelude::*;
use rtcl_tk_core::{Fill, PackSide, TkTree, WidgetKind};

thread_local! {
    /// The renderer's revision signal, created by the first [`TkView`]
    /// render.  Tk mutations bump it from event callbacks, which schedules
    /// a re-render of every component that read it.
    static REVISION: RefCell<Option<Signal<u64>>> = const { RefCell::new(None) };
    /// Set once per engine thread; `TkView` starts the one pump interval.
    static PUMP_STARTED: Cell<bool> = const { Cell::new(false) };
}

fn notify_backend() {
    REVISION.with(|r| {
        if let Some(rev) = *r.borrow() {
            rev.update(|v| *v = v.wrapping_add(1));
        }
    });
}

/// Attach the renderer's invalidation hook to the Tk runtime.
pub fn install_backend() {
    PUMP_STARTED.with(|p| p.set(false));
    REVISION.with(|r| *r.borrow_mut() = None);
    rtcl_tk_runtime::set_change_hook(Some(notify_backend));
}

fn revision_signal() -> Signal<u64> {
    REVISION.with(|r| {
        let mut slot = r.borrow_mut();
        if slot.is_none() {
            *slot = Some(signal(0u64));
        }
        *slot.as_ref().unwrap()
    })
}

/// The whole Tk tree as an nsh element.
///
/// The tree lives in a [`dynamic`] region: Tk mutations bump the revision
/// signal the region reads, so it is rebuilt (including added or removed
/// widgets) without a component re-run — nsh component bodies run once.
#[component]
pub fn TkView() -> impl IntoElement {
    start_event_pump();
    let revision = revision_signal();
    view()
        .flex_col()
        .w_full()
        .p(px(8.0))
        .child(dynamic(move || {
            let _revision = revision.get();
            match rtcl_tk_runtime::snapshot_active() {
                Some(tree) => vec![view()
                    .flex_col()
                    .w_full()
                    .child(render_children(&tree, "."))],
                None => vec![view()
                    .flex_col()
                    .child(text("Tk runtime is not installed").text_size(px(14.0)))],
            }
        }))
}

/// Pump `after` timers on the engine thread.  The interval belongs to the
/// component's owner, so it stops when the view unmounts.
fn start_event_pump() {
    if PUMP_STARTED.with(|p| p.replace(true)) {
        return;
    }
    set_interval(Duration::from_millis(16), || {
        rtcl_tk_runtime::pump_active();
    });
}

fn option<'a>(tree: &'a TkTree, path: &str, name: &str) -> &'a str {
    tree.get(path)
        .and_then(|n| n.options.get(name))
        .map(String::as_str)
        .unwrap_or("")
}

fn effective(tree: &TkTree, path: &str, name: &str) -> String {
    let Some(node) = tree.get(path) else {
        return String::new();
    };
    node.options
        .get(name)
        .cloned()
        .or_else(|| rtcl_tk_core::option_default(node.kind, name).map(str::to_string))
        .unwrap_or_default()
}

fn is_disabled(tree: &TkTree, path: &str) -> bool {
    option(tree, path, "-state") == "disabled"
}

fn render_children(tree: &TkTree, path: &str) -> AnyElement {
    let Some(node) = tree.get(path) else {
        return view().into_element();
    };
    // A parent whose children use grid renders rows of cells; pack lays
    // out along one axis (left/right children become a row).
    if node
        .children
        .iter()
        .any(|c| tree.get(c).is_some_and(|n| n.grid.is_some()))
    {
        return render_grid(tree, path);
    }
    let row = node.children.iter().any(|c| {
        tree.get(c)
            .and_then(|n| n.pack)
            .is_some_and(|p| matches!(p.side, PackSide::Left | PackSide::Right))
    });
    let mut container = view().gap_2();
    container = if row {
        container.flex_row().items_center()
    } else {
        container.flex_col()
    };
    for child in &node.children {
        container = container.child(render_packed(tree, child));
    }
    container.into_element()
}

fn render_packed(tree: &TkTree, path: &str) -> AnyElement {
    let pack = tree.get(path).and_then(|n| n.pack);
    let mut cell = view();
    if let Some(p) = pack {
        if p.expand {
            cell = cell.flex_grow(1.0);
        }
        match p.fill {
            Fill::X | Fill::Both => cell = cell.w_full(),
            _ => {}
        }
        match p.fill {
            Fill::Y | Fill::Both => cell = cell.h_full(),
            _ => {}
        }
        if p.padx > 0.0 || p.pady > 0.0 {
            cell = cell.px(px(p.padx as f32)).py(px(p.pady as f32));
        }
    }
    cell.child(render_node(tree, path)).into_element()
}

fn render_grid(tree: &TkTree, path: &str) -> AnyElement {
    let Some(node) = tree.get(path) else {
        return view().into_element();
    };
    let mut rows: BTreeMap<i64, Vec<(&str, i64)>> = BTreeMap::new();
    for child in &node.children {
        if let Some(g) = tree.get(child).and_then(|n| n.grid) {
            rows.entry(g.row)
                .or_default()
                .push((child.as_str(), g.column));
        }
    }
    let mut container = view().flex_col().gap_2();
    for (_, mut cells) in rows {
        cells.sort_by_key(|(_, column)| *column);
        let mut row = view().flex_row().items_center().gap_2().w_full();
        for (child, _) in cells {
            let mut cell = view().flex_grow(1.0);
            if let Some(g) = tree.get(child).and_then(|n| n.grid) {
                if g.sticky.left && g.sticky.right {
                    cell = cell.w_full();
                }
                if g.padx > 0.0 || g.pady > 0.0 {
                    cell = cell.px(px(g.padx as f32)).py(px(g.pady as f32));
                }
            }
            row = row.child(cell.child(render_node(tree, child)));
        }
        container = container.child(row);
    }
    container.into_element()
}

fn render_node(tree: &TkTree, path: &str) -> AnyElement {
    let Some(node) = tree.get(path) else {
        return text("").into_element();
    };
    match node.kind {
        WidgetKind::Toplevel | WidgetKind::Frame => {
            let mut frame = view().flex_col().gap_2();
            if node.kind == WidgetKind::Frame {
                frame = frame.p(px(4.0));
            }
            frame.child(render_children(tree, path)).into_element()
        }
        WidgetKind::Label => text(rtcl_tk_runtime::display_text(path))
            .text_size(px(14.0))
            .into_element(),
        WidgetKind::Button => {
            let path = path.to_string();
            let disabled = is_disabled(tree, path.as_str());
            let mut button = nsh_base::button()
                .px(px(12.0))
                .py(px(6.0))
                .disabled(disabled)
                .child(text(rtcl_tk_runtime::display_text(&path)).text_size(px(14.0)));
            if !disabled {
                let press_path = path.clone();
                button = button.on_press(move |_: &()| {
                    rtcl_tk_runtime::press_button(&press_path);
                });
            }
            button.into_element()
        }
        WidgetKind::Entry => {
            let path = path.to_string();
            let show = option(tree, path.as_str(), "-show").to_string();
            let disabled = is_disabled(tree, path.as_str());
            let readonly = option(tree, path.as_str(), "-state") == "readonly";
            let change_path = path.clone();
            let submit_path = path.clone();
            let key_path = path.clone();
            nsh_base::input()
                .value(rtcl_tk_runtime::entry_value(&path))
                .masked(!show.is_empty())
                .disabled(disabled)
                .readonly(readonly)
                .on_change(move |text: &str| {
                    rtcl_tk_runtime::entry_changed(&change_path, text);
                })
                .on_submit(move |_: &str| {
                    rtcl_tk_runtime::entry_submitted(&submit_path);
                })
                .on_key_press(move |key: &str| {
                    let tcl_key = if key == "Enter" { "Return" } else { key };
                    let character = if key.chars().count() == 1 { key } else { "" };
                    rtcl_tk_runtime::entry_key(&key_path, tcl_key, character);
                })
                .into_element()
        }
        WidgetKind::Checkbutton => {
            let path = path.to_string();
            let checked = rtcl_tk_runtime::is_checked(&path);
            let mark = if checked { "☑" } else { "☐" };
            let label = rtcl_tk_runtime::display_text(&path);
            let disabled = is_disabled(tree, path.as_str());
            let press_path = path.clone();
            let row = view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(text(mark.to_string()).text_size(px(16.0)))
                .child(text(label).text_size(px(14.0)));
            let mut control = nsh_base::checkbox()
                .checked(checked)
                .disabled(disabled)
                .child(row);
            if !disabled {
                control = control.on_change(move |_: &bool| {
                    rtcl_tk_runtime::press_button(&press_path);
                });
            }
            control.into_element()
        }
        WidgetKind::Radiobutton => {
            let path = path.to_string();
            let checked = rtcl_tk_runtime::is_checked(&path);
            let mark = if checked { "◉" } else { "○" };
            let label = rtcl_tk_runtime::display_text(&path);
            let disabled = is_disabled(tree, path.as_str());
            let press_path = path.clone();
            let row = view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(text(mark.to_string()).text_size(px(16.0)))
                .child(text(label).text_size(px(14.0)));
            let mut control = nsh_base::radio()
                .checked(checked)
                .disabled(disabled)
                .child(row);
            if !disabled {
                control = control.on_change(move |_: &bool| {
                    rtcl_tk_runtime::press_button(&press_path);
                });
            }
            control.into_element()
        }
        WidgetKind::Scale => {
            let path = path.to_string();
            let value = rtcl_tk_runtime::scale_value(&path);
            let from = effective(tree, path.as_str(), "-from")
                .parse::<f64>()
                .unwrap_or(0.0);
            let to = effective(tree, path.as_str(), "-to")
                .parse::<f64>()
                .unwrap_or(100.0);
            let step = effective(tree, path.as_str(), "-resolution")
                .parse::<f64>()
                .unwrap_or(1.0);
            let disabled = is_disabled(tree, path.as_str());
            let change_path = path.clone();
            let mut row = view().flex_row().items_center().gap_2();
            if !option(tree, path.as_str(), "-label").is_empty() {
                row = row.child(text(option(tree, path.as_str(), "-label").to_string()));
            }
            row = row.child(
                nsh_base::slider()
                    .value(move || SliderValue::Single(value))
                    .min(from)
                    .max(to)
                    .step(step)
                    .axis(Axis::Horizontal)
                    .disabled(disabled)
                    .track_style(|s| s.w(px(180.0)).h(px(24.0)).justify_center())
                    .indicator_style(|s| s.h(px(4.0)))
                    .range_style(|s| s.h(px(4.0)))
                    .thumb_style(|s| s.size(px(14.0)))
                    .on_change(move |v: &SliderValue| {
                        rtcl_tk_runtime::scale_changed(&change_path, v.start());
                    }),
            );
            row = row.child(text(format_number(value)).text_size(px(12.0)));
            row.into_element()
        }
        WidgetKind::Text => {
            let path = path.to_string();
            let disabled = is_disabled(tree, path.as_str());
            let change_path = path.clone();
            nsh_base::input()
                .multiline(true)
                .value(rtcl_tk_runtime::text_value(&path))
                .disabled(disabled)
                .on_change(move |text: &str| {
                    rtcl_tk_runtime::text_changed(&change_path, text);
                })
                .into_element()
        }
        WidgetKind::Listbox => {
            let (items, selected) = rtcl_tk_runtime::list_items(path);
            let mut list = view().flex_col().gap_1();
            for (i, item) in items.iter().enumerate() {
                let selected = selected.contains(&i);
                let prefix = if selected { "▶ " } else { "   " };
                let row_path = path.to_string();
                list = list.child(
                    nsh_base::button()
                        .px(px(4.0))
                        .py(px(2.0))
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .child(text(prefix).text_size(px(14.0)))
                                .child(text(item.clone()).text_size(px(14.0))),
                        )
                        .on_press(move |_: &()| {
                            rtcl_tk_runtime::listbox_select(&row_path, i);
                        }),
                );
            }
            list.into_element()
        }
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_renders_without_active_interp() {
        // The component is exercised by an nsh host; here we only pin the
        // hook installation and the empty-snapshot contract.
        install_backend();
        assert!(rtcl_tk_runtime::snapshot_active().is_none());
    }
}
