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
                Some(tree) => {
                    let mut column = view().flex_col().w_full().gap_2();
                    let bar = rtcl_tk_runtime::menu_bar();
                    if let Some(bar) = &bar {
                        column = column.child(render_menu_bar(bar));
                    }
                    column = column.child(render_children(&tree, "."));
                    // The dropdown floats over the content.
                    if bar.is_some() {
                        if let Some(open) = rtcl_tk_runtime::open_menu() {
                            column = column.child(render_menu_dropdown(&open));
                        }
                    }
                    vec![column]
                }
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
        // Idle apps wake for nothing: `after` work is rare.
        if rtcl_tk_runtime::has_scheduled_events() {
            rtcl_tk_runtime::pump_active();
        }
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

fn render_menu_bar(menu_path: &str) -> AnyElement {
    let Some(menu) = rtcl_tk_runtime::menu_snapshot(menu_path) else {
        return view().into_element();
    };
    let mut bar = view()
        .flex_row()
        .w_full()
        .items_center()
        .gap_1()
        .pb(px(2.0))
        .border_b(px(1.0))
        .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563));
    for (i, item) in menu.items.iter().enumerate() {
        if item.kind == "separator" {
            continue;
        }
        let label = item
            .options
            .get("-label")
            .cloned()
            .unwrap_or_else(|| "?".to_string());
        let path = menu_path.to_string();
        let submenu = item.options.get("-menu").cloned();
        bar = bar.child(
            nsh_base::button()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .child(
                    text(label)
                        .text_size(px(13.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                )
                .on_press(move |_: &()| match &submenu {
                    Some(sub) if !sub.is_empty() => rtcl_tk_runtime::toggle_menu(sub),
                    _ => rtcl_tk_runtime::press_menu_item(&path, i),
                }),
        );
    }
    bar.into_element()
}

fn render_menu_dropdown(menu_path: &str) -> AnyElement {
    let Some(menu) = rtcl_tk_runtime::menu_snapshot(menu_path) else {
        return view().into_element();
    };
    let mut panel = view()
        .absolute()
        .top(px(30.0))
        .left(px(8.0))
        .flex_col()
        .w(px(220.0))
        .p(px(4.0))
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
        .bg(AdaptiveColor::rgb(0xffffff, 0x1f2937));
    for (i, item) in menu.items.iter().enumerate() {
        if item.kind == "separator" {
            panel = panel.child(
                view()
                    .w_full()
                    .h(px(1.0))
                    .bg(AdaptiveColor::rgb(0xe5e7eb, 0x374151)),
            );
            continue;
        }
        let label = item.options.get("-label").cloned().unwrap_or_default();
        let var = item.options.get("-variable").cloned().unwrap_or_default();
        let on = item
            .options
            .get("-onvalue")
            .cloned()
            .unwrap_or_else(|| "1".to_string());
        let value = item.options.get("-value").cloned().unwrap_or_default();
        let mark = match item.kind.as_str() {
            "checkbutton" => {
                if rtcl_tk_runtime::var_value(&var).is_some_and(|v| v == on) {
                    "☑ "
                } else {
                    "☐ "
                }
            }
            "radiobutton" => {
                if rtcl_tk_runtime::var_value(&var).is_some_and(|v| v == value) {
                    "◉ "
                } else {
                    "○ "
                }
            }
            "cascade" => "",
            _ => "",
        };
        let suffix = if item.kind == "cascade" { " ▸" } else { "" };
        let disabled = item.options.get("-state").is_some_and(|s| s == "disabled");
        let path = menu_path.to_string();
        if disabled {
            panel = panel.child(
                text(format!("{mark}{label}{suffix}"))
                    .text_size(px(13.0))
                    .text_color(AdaptiveColor::rgb(0x9ca3af, 0x6b7280)),
            );
            continue;
        }
        panel = panel.child(
            nsh_base::button()
                .w_full()
                .justify_start()
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .child(
                    text(format!("{mark}{label}{suffix}"))
                        .text_size(px(13.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                )
                .on_press(move |_: &()| {
                    rtcl_tk_runtime::press_menu_item(&path, i);
                }),
        );
    }
    panel.into_element()
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
    // Pack children flow in order; consecutive left/right children share
    // a row, with the right-hand ones pushed to its end (a listbox and its
    // scrollbar, for instance).
    let mut container = view().flex_col().gap_2();
    let mut placed = Vec::new();
    let mut i = 0;
    while i < node.children.len() {
        let child = &node.children[i];
        if tree.get(child).is_some_and(|n| n.place.is_some()) {
            placed.push(child);
            i += 1;
            continue;
        }
        let side = tree
            .get(child)
            .and_then(|n| n.pack.clone())
            .map(|p| p.side)
            .unwrap_or(PackSide::Top);
        if matches!(side, PackSide::Left | PackSide::Right) {
            let mut lefts = Vec::new();
            let mut rights = Vec::new();
            while i < node.children.len() {
                let c = &node.children[i];
                let s = tree
                    .get(c)
                    .and_then(|n| n.pack.clone())
                    .map(|p| p.side)
                    .unwrap_or(PackSide::Top);
                match s {
                    PackSide::Left => lefts.push(c),
                    PackSide::Right => rights.push(c),
                    _ => break,
                }
                i += 1;
            }
            let mut row = view().flex_row().items_center().gap_2().w_full();
            for c in lefts {
                row = row.child(render_packed(tree, c));
            }
            if !rights.is_empty() {
                row = row.child(view().flex_grow(1.0));
                for c in rights {
                    row = row.child(render_packed(tree, c));
                }
            }
            container = container.child(row);
        } else {
            container = container.child(render_packed(tree, child));
            i += 1;
        }
    }
    for child in placed {
        container = container.child(render_placed(tree, child));
    }
    container.into_element()
}

fn render_placed(tree: &TkTree, path: &str) -> AnyElement {
    let Some(info) = tree.get(path).and_then(|n| n.place.clone()) else {
        return render_node(tree, path);
    };
    let mut cell = view().absolute();
    cell = if info.relx != 0.0 {
        cell.left(relative(info.relx as f32))
    } else {
        cell.left(px(info.x as f32))
    };
    cell = if info.rely != 0.0 {
        cell.top(relative(info.rely as f32))
    } else {
        cell.top(px(info.y as f32))
    };
    if let Some(w) = info.relwidth {
        cell = cell.w(relative(w as f32));
    } else if let Some(w) = info.width {
        cell = cell.w(px(w as f32));
    }
    if let Some(h) = info.relheight {
        cell = cell.h(relative(h as f32));
    } else if let Some(h) = info.height {
        cell = cell.h(px(h as f32));
    }
    // `-anchor` says which point of the widget the coordinates name.
    let anchor = info.anchor.as_str();
    let fx = if anchor.contains('e') && !anchor.contains('w') {
        1.0f32
    } else if anchor == "center" {
        0.5
    } else {
        0.0
    };
    let fy = if anchor.contains('s') && !anchor.contains('n') {
        1.0f32
    } else if anchor == "center" {
        0.5
    } else {
        0.0
    };
    if fx != 0.0 {
        cell = cell.translate_x(relative(-fx));
    }
    if fy != 0.0 {
        cell = cell.translate_y(relative(-fy));
    }
    cell.child(render_node(tree, path)).into_element()
}

fn render_packed(tree: &TkTree, path: &str) -> AnyElement {
    let pack = tree.get(path).and_then(|n| n.pack.clone());
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
        if p.anchor.contains('w') && !p.anchor.contains('e') {
            cell = cell.self_start();
        } else if p.anchor.contains('e') && !p.anchor.contains('w') {
            cell = cell.self_end();
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
        for (child, column) in cells {
            let weight = rtcl_tk_runtime::grid_weight(path, column, false).max(1) as f32;
            let mut cell = view().flex_grow(weight);
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
        WidgetKind::Labelframe => {
            let label = option(tree, path, "-text").to_string();
            let mut frame = view()
                .flex_col()
                .items_start()
                .w_full()
                .gap_2()
                .p(px(6.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563));
            if !label.is_empty() {
                frame = frame.child(
                    text(label)
                        .text_size(px(12.0))
                        .text_color(AdaptiveColor::rgb(0x4b5563, 0xd1d5db)),
                );
            }
            frame.child(render_children(tree, path)).into_element()
        }
        WidgetKind::Label => view()
            .w_full()
            .items_start()
            .child(
                text(rtcl_tk_runtime::display_text(path))
                    .text_size(px(14.0))
                    .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
            )
            .into_element(),
        WidgetKind::Button => {
            let path = path.to_string();
            let disabled = is_disabled(tree, path.as_str());
            let mut button = nsh_base::button()
                .self_start()
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                .bg(AdaptiveColor::rgb(0xf3f4f6, 0x374151))
                .disabled(disabled)
                .child(
                    text(rtcl_tk_runtime::display_text(&path))
                        .text_size(px(14.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf9fafb)),
                );
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
                .auto_focus(rtcl_tk_runtime::focused(&path))
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
            let label = rtcl_tk_runtime::display_text(&path);
            let disabled = is_disabled(tree, path.as_str());
            let press_path = path.clone();
            let mut control = nsh_base::checkbox()
                .checked(checked)
                .disabled(disabled)
                .flex_row()
                .items_center()
                .gap_2()
                .indicator_style(move |s| {
                    let box_style = s
                        .size(px(16.0))
                        .rounded(px(3.0))
                        .border_1()
                        .border_color(AdaptiveColor::rgb(0x6b7280, 0x9ca3af));
                    if checked {
                        box_style.bg(AdaptiveColor::rgb(0x2563eb, 0x3b82f6))
                    } else {
                        box_style.bg(AdaptiveColor::rgb(0xffffff, 0x1f2937))
                    }
                })
                .child(
                    text(label)
                        .text_size(px(14.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                );
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
                .child(
                    text(mark.to_string())
                        .text_size(px(16.0))
                        .text_color(AdaptiveColor::rgb(0x2563eb, 0x60a5fa)),
                )
                .child(
                    text(label)
                        .text_size(px(14.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                );
            let mut control = nsh_base::radio()
                .checked(checked)
                .disabled(disabled)
                .flex_row()
                .items_center()
                .gap_2()
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
                    .indicator_style(|s| s.h(px(4.0)).bg(AdaptiveColor::rgb(0xd1d5db, 0x4b5563)))
                    .range_style(|s| s.h(px(4.0)).bg(AdaptiveColor::rgb(0x2563eb, 0x3b82f6)))
                    .thumb_style(|s| {
                        s.size(px(14.0))
                            .rounded_full()
                            .bg(AdaptiveColor::rgb(0x2563eb, 0x3b82f6))
                    })
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
                .auto_focus(rtcl_tk_runtime::focused(&path))
                .value(rtcl_tk_runtime::text_value(&path))
                .disabled(disabled)
                .on_change(move |text: &str| {
                    rtcl_tk_runtime::text_changed(&change_path, text);
                })
                .into_element()
        }
        WidgetKind::Listbox => {
            let (items, selected) = rtcl_tk_runtime::list_items(path);
            let mut list = view()
                .flex_col()
                .gap_1()
                .p(px(3.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                .bg(AdaptiveColor::rgb(0xffffff, 0x111827));
            for (i, item) in items.iter().enumerate() {
                let selected = selected.contains(&i);
                let prefix = if selected { "▶ " } else { "   " };
                let row_path = path.to_string();
                list = list.child(
                    nsh_base::button()
                        .w_full()
                        .px(px(6.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(if selected {
                            AdaptiveColor::rgb(0xdbeafe, 0x1e3a8a)
                        } else {
                            AdaptiveColor::rgb(0xffffff, 0x111827)
                        })
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .child(
                                    text(prefix)
                                        .text_size(px(14.0))
                                        .text_color(AdaptiveColor::rgb(0x2563eb, 0x60a5fa)),
                                )
                                .child(
                                    text(item.clone())
                                        .text_size(px(14.0))
                                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                                ),
                        )
                        .on_press(move |_: &()| {
                            rtcl_tk_runtime::listbox_select(&row_path, i);
                        }),
                );
            }
            let rows = option(tree, path, "-height")
                .parse::<f32>()
                .unwrap_or(10.0)
                .max(1.0);
            scroll()
                .w_full()
                .h(px(rows * 28.0 + 8.0))
                .scrollbar(true)
                .child(list)
                .into_element()
        }
        WidgetKind::Message => view()
            .w_full()
            .items_start()
            .child(
                text(rtcl_tk_runtime::display_text(path))
                    .text_size(px(14.0))
                    .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
            )
            .into_element(),
        WidgetKind::Scrollbar => {
            let horizontal = option(tree, path, "-orient") == "horizontal";
            let track = if horizontal {
                view()
                    .w_full()
                    .h(px(12.0))
                    .rounded_full()
                    .bg(AdaptiveColor::rgb(0xe5e7eb, 0x374151))
            } else {
                view()
                    .w(px(12.0))
                    .h(px(120.0))
                    .rounded_full()
                    .bg(AdaptiveColor::rgb(0xe5e7eb, 0x374151))
            };
            track.into_element()
        }
        WidgetKind::Separator => {
            if option(tree, path, "-orient") == "horizontal" {
                view()
                    .w_full()
                    .h(px(1.0))
                    .bg(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                    .into_element()
            } else {
                view()
                    .w(px(1.0))
                    .h(px(24.0))
                    .bg(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                    .into_element()
            }
        }
        WidgetKind::Progressbar => {
            let (value, max) = rtcl_tk_runtime::progressbar_value(path);
            let fraction = if max > 0.0 {
                (value / max).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let horizontal = option(tree, path, "-orient") == "horizontal";
            let mut track = if horizontal {
                view()
                    .w_full()
                    .h(px(8.0))
                    .rounded_full()
                    .bg(AdaptiveColor::rgb(0xe5e7eb, 0x374151))
            } else {
                view()
                    .w(px(8.0))
                    .h(px(100.0))
                    .rounded_full()
                    .bg(AdaptiveColor::rgb(0xe5e7eb, 0x374151))
            };
            track = if horizontal {
                track.child(
                    view()
                        .h_full()
                        .w(px(100.0 * fraction))
                        .rounded_full()
                        .bg(AdaptiveColor::rgb(0x2563eb, 0x3b82f6)),
                )
            } else {
                track.child(
                    view()
                        .w_full()
                        .h(px(100.0 * fraction))
                        .rounded_full()
                        .bg(AdaptiveColor::rgb(0x2563eb, 0x3b82f6)),
                )
            };
            track.into_element()
        }
        WidgetKind::Notebook => {
            let pages = node.children.clone();
            let selected = rtcl_tk_runtime::notebook_selected(path);
            let mut tabs = view().flex_row().items_center().gap_1().w_full();
            for (i, page) in pages.iter().enumerate() {
                let label = tree
                    .get(page)
                    .and_then(|n| n.options.get("-text").cloned())
                    .unwrap_or_else(|| page.rsplit('.').next().unwrap_or(page).to_string());
                let notebook = path.to_string();
                tabs = tabs.child(
                    nsh_base::button()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(5.0))
                        .bg(if i == selected {
                            AdaptiveColor::rgb(0x2563eb, 0x3b82f6)
                        } else {
                            AdaptiveColor::rgb(0xe5e7eb, 0x374151)
                        })
                        .child(
                            text(label)
                                .text_size(px(13.0))
                                .text_color(if i == selected {
                                    AdaptiveColor::rgb(0xffffff, 0xffffff)
                                } else {
                                    AdaptiveColor::rgb(0x111827, 0xf3f4f6)
                                }),
                        )
                        .on_press(move |_: &()| {
                            rtcl_tk_runtime::select_notebook_tab(&notebook, i);
                        }),
                );
            }
            let mut column = view().flex_col().gap_2().w_full().child(tabs);
            // Keep every page mounted and hide the inactive ones: re-mounting
            // the selected page on a switch loses its native widgets on GTK.
            for (i, page) in pages.iter().enumerate() {
                let body = view()
                    .w_full()
                    .items_start()
                    .p(px(6.0))
                    .child(render_node(tree, page));
                column = column.child(if i == selected { body } else { body.hidden() });
            }
            column.into_element()
        }
        WidgetKind::Combobox => {
            let path = path.to_string();
            let values = rtcl_tk_runtime::combobox_values(&path);
            let open = rtcl_tk_runtime::open_combobox().as_deref() == Some(path.as_str());
            let disabled = is_disabled(tree, path.as_str());
            let readonly = option(tree, path.as_str(), "-state") == "readonly";
            let change_path = path.clone();
            let toggle_path = path.clone();
            let row = view()
                .flex_row()
                .justify_start()
                .items_center()
                .gap_1()
                .child(
                    view().w(px(200.0)).child(
                        nsh_base::input()
                            .value(rtcl_tk_runtime::entry_value(&path))
                            .disabled(disabled)
                            .readonly(readonly)
                            .on_change(move |text: &str| {
                                rtcl_tk_runtime::entry_changed(&change_path, text);
                            }),
                    ),
                )
                .child(
                    nsh_base::button()
                        .px(px(8.0))
                        .py(px(5.0))
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                        .bg(AdaptiveColor::rgb(0xf3f4f6, 0x374151))
                        .disabled(disabled)
                        .child(
                            text("▼")
                                .text_size(px(13.0))
                                .text_color(AdaptiveColor::rgb(0x111827, 0xf9fafb)),
                        )
                        .on_press(move |_: &()| {
                            rtcl_tk_runtime::toggle_combobox(&toggle_path);
                        }),
                );
            let mut column = view().flex_col().items_start().gap_1().child(row);
            if open {
                let mut list = view()
                    .flex_col()
                    .w(px(220.0))
                    .p(px(3.0))
                    .gap_1()
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                    .bg(AdaptiveColor::rgb(0xffffff, 0x1f2937));
                for (i, value) in values.iter().enumerate() {
                    let value_path = path.clone();
                    list = list.child(
                        nsh_base::button()
                            .w_full()
                            .justify_start()
                            .px(px(6.0))
                            .py(px(3.0))
                            .rounded(px(4.0))
                            .child(
                                text(value.clone())
                                    .text_size(px(13.0))
                                    .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                            )
                            .on_press(move |_: &()| {
                                rtcl_tk_runtime::press_combobox_item(&value_path, i);
                            }),
                    );
                }
                column = column.child(list);
            }
            column.into_element()
        }
        WidgetKind::Treeview => {
            let Some(tv) = rtcl_tk_runtime::treeview_snapshot(path) else {
                return text("").into_element();
            };
            let heading = |column: &str, fallback: &str| {
                tv.headings
                    .get(column)
                    .cloned()
                    .unwrap_or_else(|| fallback.to_string())
            };
            let mut header = view()
                .flex_row()
                .w_full()
                .items_center()
                .gap_2()
                .px(px(6.0))
                .py(px(3.0))
                .bg(AdaptiveColor::rgb(0xe5e7eb, 0x374151));
            header = header.child(
                text(heading("#0", "Item"))
                    .text_size(px(13.0))
                    .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
            );
            for column in &tv.columns {
                header = header.child(
                    text(heading(column, column))
                        .text_size(px(13.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                );
            }
            let mut body = view().flex_col().w_full().gap_1().p(px(4.0));
            let mut order = Vec::new();
            collect_treeview_rows(&tv, None, 0, &mut order);
            for (item, depth) in order {
                let path = path.to_string();
                let id = item.id.clone();
                let has_children = tv
                    .items
                    .iter()
                    .any(|i| i.parent.as_deref() == Some(id.as_str()));
                let selected = tv.selected.contains(&id);
                let mut row = view().flex_row().w_full().items_center().gap_1();
                if depth > 0 {
                    row = row.child(view().w(px(16.0 * depth as f32)));
                }
                let mark = if has_children {
                    if item.open {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                let mark = if has_children {
                    if item.open {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                let select_path = path.clone();
                let select_id = id.clone();
                let mut cells = view().flex_row().w_full().items_center().gap_2();
                cells = cells.child(
                    text(mark.to_string())
                        .text_size(px(12.0))
                        .text_color(AdaptiveColor::rgb(0x2563eb, 0x60a5fa)),
                );
                cells = cells.child(
                    text(item.text.clone())
                        .text_size(px(13.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                );
                for value in &item.values {
                    cells = cells.child(
                        text(value.clone())
                            .text_size(px(13.0))
                            .text_color(AdaptiveColor::rgb(0x4b5563, 0xd1d5db)),
                    );
                }
                let button = nsh_base::button()
                    .w_full()
                    .justify_start()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(if selected {
                        AdaptiveColor::rgb(0xdbeafe, 0x1e3a8a)
                    } else {
                        AdaptiveColor::rgb(0xffffff, 0x111827)
                    })
                    .child(cells);
                row = row.child(button.on_press(move |_: &()| {
                    if has_children {
                        rtcl_tk_runtime::treeview_toggle(&select_path, &select_id);
                    } else {
                        rtcl_tk_runtime::treeview_select(&select_path, &select_id);
                    }
                }));
                body = body.child(row);
            }
            let rows = option(tree, path, "-height")
                .parse::<f32>()
                .unwrap_or(10.0)
                .max(1.0);
            view()
                .flex_col()
                .w_full()
                .rounded(px(4.0))
                .border_1()
                .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                .child(header)
                .child(
                    scroll()
                        .w_full()
                        .h(px(rows * 22.0 + 8.0))
                        .scrollbar(true)
                        .child(body),
                )
                .into_element()
        }
    }
}

fn collect_treeview_rows<'a>(
    tv: &'a rtcl_tk_runtime::TreeViewState,
    parent: Option<&str>,
    depth: usize,
    out: &mut Vec<(&'a rtcl_tk_runtime::TreeItem, usize)>,
) {
    for item in tv.items.iter().filter(|i| i.parent.as_deref() == parent) {
        out.push((item, depth));
        if item.open {
            collect_treeview_rows(tv, Some(item.id.as_str()), depth + 1, out);
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
