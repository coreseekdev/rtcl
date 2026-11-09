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

use nsh_base::rich_editor::{
    extensions::starter,
    model::Schema,
    state::{EditorState, EditorStateConfig},
    view::{rich_editor, EditorSession, NodeViews, PartStyles, RichEditorStyles},
};
use nsh_base::SliderValue;
use nsh_core::element::desktop;
use nsh_core::geometry::{Axis, Point, Radians, Size};
use nsh_core::prelude::*;
use rtcl_tk_core::{Fill, PackSide, TkTree, WidgetKind};
use serde_json::json;

thread_local! {
    /// The renderer's revision signal, created by the first [`TkView`]
    /// render.  Tk mutations bump it from event callbacks, which schedules
    /// a re-render of every component that read it.
    static REVISION: RefCell<Option<Signal<u64>>> = const { RefCell::new(None) };
    /// Set once per engine thread; `TkView` starts the one pump interval.
    static PUMP_STARTED: Cell<bool> = const { Cell::new(false) };
    /// Rich-editor sessions for tagged text widgets, keyed by path.
    static WYSIWYG: RefCell<BTreeMap<String, WysiwygEntry>> =
        const { RefCell::new(BTreeMap::new()) };
}

struct WysiwygEntry {
    content: String,
    signature: String,
    session: EditorSession,
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
    if matches!(os(), nsh_core::platform::Os::MacOs) {
        // Re-publish the Tk menu whenever the model changes, under the
        // component's owner so the handlers outlive dynamic re-renders.
        effect(move || {
            let _ = revision.get();
            sync_native_menu();
        });
    }
    view()
        .flex_col()
        .w_full()
        .p(px(8.0))
        .child(dynamic(move || {
            let _revision = revision.get();
            match rtcl_tk_runtime::snapshot_active() {
                Some(tree) => {
                    let mut column = view().flex_col().w_full().gap_2();
                    let native = matches!(os(), nsh_core::platform::Os::MacOs);
                    let bar = rtcl_tk_runtime::menu_bar();
                    if is_desktop() {
                        if let Some(title) = rtcl_tk_runtime::window_title() {
                            desktop::set_title(title);
                        }
                    }
                    if !native {
                        if let Some(bar) = &bar {
                            column = column.child(render_menu_bar(bar));
                        }
                    }
                    column = column.child(render_children(&tree, "."));
                    // The dropdown floats over the content.
                    if !native && bar.is_some() {
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

/// Publish the Tk `menu` tree as the platform's application menu (macOS's
/// system menu, GTK's menu bar).  Headless runs keep the in-app bar.
fn sync_native_menu() {
    let items = rtcl_tk_runtime::menu_bar()
        .map(|bar| native_menu_items(&bar))
        .unwrap_or_default();
    if items.is_empty() {
        desktop::set_application_menu(None);
    } else {
        desktop::set_application_menu(items);
    }
}

fn native_menu_items(menu_path: &str) -> Vec<desktop::MenuItem> {
    let Some(menu) = rtcl_tk_runtime::menu_snapshot(menu_path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, item) in menu.items.iter().enumerate() {
        if item.kind == "separator" {
            out.push(desktop::MenuItem::separator());
            continue;
        }
        let label = item.options.get("-label").cloned().unwrap_or_default();
        let disabled = item.options.get("-state").is_some_and(|s| s == "disabled");
        if item.kind == "cascade" {
            if let Some(sub) = item.options.get("-menu").filter(|s| !s.is_empty()) {
                out.push(
                    desktop::MenuItem::submenu(label, native_menu_items(sub)).disabled(disabled),
                );
                continue;
            }
        }
        let mut entry = desktop::MenuItem::new(label).disabled(disabled).on_select({
            let path = menu_path.to_string();
            move || rtcl_tk_runtime::press_menu_item(&path, i)
        });
        if item.kind == "checkbutton" || item.kind == "radiobutton" {
            let var = item.options.get("-variable").cloned().unwrap_or_default();
            let on = item
                .options
                .get("-onvalue")
                .cloned()
                .or_else(|| item.options.get("-value").cloned())
                .unwrap_or_else(|| "1".to_string());
            let checked =
                !var.is_empty() && rtcl_tk_runtime::var_value(&var).is_some_and(|v| v == on);
            entry = entry.checked(checked);
        }
        out.push(entry);
    }
    out
}

fn parse_color(value: &str) -> Option<AdaptiveColor> {
    let value = value.trim();
    let hex = if let Some(hex) = value.strip_prefix('#') {
        match hex.len() {
            3 => {
                let mut expanded = String::with_capacity(6);
                for c in hex.chars() {
                    expanded.push(c);
                    expanded.push(c);
                }
                u32::from_str_radix(&expanded, 16).ok()?
            }
            6 => u32::from_str_radix(hex, 16).ok()?,
            12 => u32::from_str_radix(&hex[..6], 16).ok()?,
            _ => return None,
        }
    } else {
        match value.to_ascii_lowercase().as_str() {
            "black" => 0x000000,
            "white" => 0xffffff,
            "red" => 0xff0000,
            "green" => 0x00ff00,
            "blue" => 0x0000ff,
            "yellow" => 0xffff00,
            "cyan" => 0x00ffff,
            "magenta" => 0xff00ff,
            "orange" => 0xffa500,
            "purple" => 0x800080,
            "brown" => 0xa52a2a,
            "pink" => 0xffc0cb,
            "gray" | "grey" => 0x808080,
            "lightgray" | "lightgrey" => 0xd3d3d3,
            "darkgray" | "darkgrey" => 0xa9a9a9,
            _ => return None,
        }
    };
    Some(AdaptiveColor::from(rgb(hex)))
}

fn option_color(tree: &TkTree, path: &str, name: &str) -> Option<AdaptiveColor> {
    let value = option(tree, path, name);
    (!value.is_empty()).then(|| parse_color(value)).flatten()
}

fn arrow_head(
    mut painter: Painter,
    from: (f32, f32),
    to: (f32, f32),
    color: Hsla,
    size: f32,
) -> Painter {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / len, dy / len);
    let base = (to.0 - ux * size, to.1 - uy * size);
    let (px, py) = (-uy * size * 0.45, ux * size * 0.45);
    painter = painter
        .fill_color(color)
        .begin_path()
        .move_to(to.0, to.1)
        .line_to(base.0 + px, base.1 + py)
        .line_to(base.0 - px, base.1 - py)
        .close_path()
        .fill();
    painter
}

fn tags_signature(tags: &[rtcl_tk_runtime::TextTagView]) -> String {
    format!("{tags:?}")
}

fn wysiwyg_mark_attrs(
    tags: &[rtcl_tk_runtime::TextTagView],
    start: usize,
    end: usize,
) -> (String, String, String, String, bool) {
    let mut color = String::new();
    let mut background = String::new();
    let mut font_size = String::new();
    let mut font_family = String::new();
    let mut bold = false;
    for tag in tags {
        if !tag.ranges.iter().any(|(a, b)| start < *b && end > *a) {
            continue;
        }
        if let Some(value) = tag.options.get("-foreground") {
            color = value.clone();
        }
        if let Some(value) = tag.options.get("-background") {
            background = value.clone();
        }
        if let Some(value) = tag.options.get("-font") {
            if let Some((family, size, is_bold)) = rtcl_tk_runtime::resolve_font(value) {
                if !family.is_empty() {
                    font_family = family;
                }
                font_size = format!("{size}px");
                bold = is_bold;
            }
        }
    }
    (color, background, font_size, font_family, bold)
}

fn wysiwyg_doc(content: &str, tags: &[rtcl_tk_runtime::TextTagView]) -> serde_json::Value {
    let chars = content.chars().collect::<Vec<_>>();
    let len = chars.len();
    let mut paragraphs = Vec::new();
    let mut line_start = 0usize;
    for line in content.split('\n') {
        let line_len = line.chars().count();
        let line_end = (line_start + line_len).min(len);
        let mut bounds = vec![line_start, line_end];
        for tag in tags {
            for (start, end) in &tag.ranges {
                if *start < line_end && *end > line_start {
                    bounds.push((*start).max(line_start));
                    bounds.push((*end).min(line_end));
                }
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
            let text = chars[start..end].iter().collect::<String>();
            let (color, background, font_size, font_family, bold) =
                wysiwyg_mark_attrs(tags, start, end);
            let mut marks = Vec::new();
            let text_style = json!({
                "type": "textStyle",
                "attrs": {
                    "color": color,
                    "backgroundColor": background,
                    "fontSize": font_size,
                    "fontFamily": font_family,
                }
            });
            if !color.is_empty()
                || !background.is_empty()
                || !font_size.is_empty()
                || !font_family.is_empty()
            {
                marks.push(text_style);
            }
            if bold {
                marks.push(json!({"type": "bold"}));
            }
            nodes.push(if marks.is_empty() {
                json!({"type": "text", "text": text})
            } else {
                json!({"type": "text", "text": text, "marks": marks})
            });
        }
        paragraphs.push(if nodes.is_empty() {
            json!({"type": "paragraph"})
        } else {
            json!({"type": "paragraph", "content": nodes})
        });
        line_start = line_end + 1;
    }
    json!({"type": "doc", "content": paragraphs})
}

fn wysiwyg_schema() -> Schema {
    Schema::new(starter::schema_spec()).expect("the WYSIWYG schema")
}

fn wysiwyg_session(content: &str, tags: &[rtcl_tk_runtime::TextTagView]) -> Option<EditorSession> {
    let schema = wysiwyg_schema();
    let doc = schema.node_from_json(&wysiwyg_doc(content, tags)).ok()?;
    let plugins = starter::plugins(&schema);
    let state =
        EditorState::create(EditorStateConfig::new(schema).doc(doc).plugins(plugins)).ok()?;
    Some(EditorSession::new(state, NodeViews::new()))
}

fn wysiwyg_session_from_doc(json: &str) -> Option<(EditorSession, String)> {
    let schema = wysiwyg_schema();
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let doc = schema.node_from_json(&value).ok()?;
    let plugins = starter::plugins(&schema);
    let state =
        EditorState::create(EditorStateConfig::new(schema).doc(doc).plugins(plugins)).ok()?;
    let session = EditorSession::new(state, NodeViews::new());
    let state = session.state();
    let doc = state.doc();
    let text = doc.text_between(0, doc.content().size(), Some("\n"), None);
    Some((session, text))
}

fn collect_doc_marks(
    value: &serde_json::Value,
    marks: &mut BTreeMap<String, nsh_core::element::Style>,
) {
    if let Some(items) = value.get("content").and_then(|v| v.as_array()) {
        for item in items {
            if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(list) = item.get("marks").and_then(|v| v.as_array()) {
                    for mark in list {
                        let kind = mark.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if kind == "bold" {
                            let style =
                                nsh_core::element::Style::from_decls([nsh_core::StyleDecl::Set(
                                    nsh_core::StyleKey::FontWeight,
                                    nsh_core::PropertyValue::FontWeight(Some(700)),
                                )]);
                            marks.insert("bold".to_string(), style);
                        }
                        if kind == "textStyle" {
                            let attrs = mark.get("attrs").and_then(|v| v.as_object());
                            let Some(attrs) = attrs else { continue };
                            if let Some(color) = attrs.get("color").and_then(|v| v.as_str()) {
                                if !color.is_empty() {
                                    if let Some(parsed) = parse_color(color) {
                                        let style = nsh_core::element::Style::from_decls([
                                            nsh_core::StyleDecl::Set(
                                                nsh_core::StyleKey::Color,
                                                nsh_core::PropertyValue::Color(Some(
                                                    nsh_core::Color::from(parsed.light),
                                                )),
                                            ),
                                        ]);
                                        marks.insert(format!("textStyle[color={color}]"), style);
                                    }
                                }
                            }
                            if let Some(color) =
                                attrs.get("backgroundColor").and_then(|v| v.as_str())
                            {
                                if !color.is_empty() {
                                    if let Some(parsed) = parse_color(color) {
                                        let style = nsh_core::element::Style::from_decls([
                                            nsh_core::StyleDecl::Set(
                                                nsh_core::StyleKey::BackgroundColor,
                                                nsh_core::PropertyValue::Color(Some(
                                                    nsh_core::Color::from(parsed.light),
                                                )),
                                            ),
                                        ]);
                                        marks.insert(
                                            format!("textStyle[backgroundColor={color}]"),
                                            style,
                                        );
                                    }
                                }
                            }
                            if let Some(size) = attrs.get("fontSize").and_then(|v| v.as_str()) {
                                if let Some(points) =
                                    size.strip_suffix("px").and_then(|v| v.parse::<f32>().ok())
                                {
                                    let style = nsh_core::element::Style::from_decls([
                                        nsh_core::StyleDecl::Set(
                                            nsh_core::StyleKey::FontSize,
                                            nsh_core::PropertyValue::OptPoints(Some(Points(
                                                points,
                                            ))),
                                        ),
                                    ]);
                                    marks.insert(format!("textStyle[fontSize={size}]"), style);
                                }
                            }
                        }
                    }
                }
            }
            collect_doc_marks(item, marks);
        }
    }
}

fn wysiwyg_styles_from_doc(json: &str) -> RichEditorStyles {
    let mut marks = BTreeMap::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
        collect_doc_marks(&value, &mut marks);
    }
    let mut styles = RichEditorStyles::default();
    styles.marks = Some(PartStyles(marks));
    styles
}

fn wysiwyg_styles(tags: &[rtcl_tk_runtime::TextTagView]) -> RichEditorStyles {
    let mut marks = BTreeMap::new();
    for tag in tags {
        if let Some(value) = tag.options.get("-foreground") {
            if let Some(color) = parse_color(value) {
                let style = nsh_core::element::Style::from_decls([nsh_core::StyleDecl::Set(
                    nsh_core::StyleKey::Color,
                    nsh_core::PropertyValue::Color(Some(nsh_core::Color::from(color.light))),
                )]);
                marks.insert(format!("textStyle[color={value}]"), style);
            }
        }
        if let Some(value) = tag.options.get("-background") {
            if let Some(color) = parse_color(value) {
                let style = nsh_core::element::Style::from_decls([nsh_core::StyleDecl::Set(
                    nsh_core::StyleKey::BackgroundColor,
                    nsh_core::PropertyValue::Color(Some(nsh_core::Color::from(color.light))),
                )]);
                marks.insert(format!("textStyle[backgroundColor={value}]"), style);
            }
        }
        if let Some(value) = tag.options.get("-font") {
            if let Some((_family, size, bold)) = rtcl_tk_runtime::resolve_font(value) {
                let style = nsh_core::element::Style::from_decls([nsh_core::StyleDecl::Set(
                    nsh_core::StyleKey::FontSize,
                    nsh_core::PropertyValue::OptPoints(Some(Points((size * 1.3333) as f32))),
                )]);
                marks.insert(format!("textStyle[fontSize={size}px]"), style);
                if bold {
                    let style = nsh_core::element::Style::from_decls([nsh_core::StyleDecl::Set(
                        nsh_core::StyleKey::FontWeight,
                        nsh_core::PropertyValue::FontWeight(Some(700)),
                    )]);
                    marks.insert("bold".to_string(), style);
                }
            }
        }
    }
    let mut styles = RichEditorStyles::default();
    styles.marks = Some(PartStyles(marks));
    styles
}

fn render_wysiwyg(tree: &TkTree, path: &str, tags: &[rtcl_tk_runtime::TextTagView]) -> AnyElement {
    let content = rtcl_tk_runtime::text_value(path);
    let signature = tags_signature(tags);
    let cached = rtcl_tk_runtime::rich_doc(path);
    let built = WYSIWYG.with(|map| {
        let mut map = map.borrow_mut();
        if let Some(entry) = map.get(path) {
            if entry.content == content && entry.signature == signature {
                let styles = match &cached {
                    Some(json) => wysiwyg_styles_from_doc(json),
                    None => wysiwyg_styles(tags),
                };
                return Some((entry.session.clone(), styles));
            }
        }
        // The rich document is the data: prefer it, shifting its marks
        // when the text was edited outside the editor.
        if let Some(json) = &cached {
            let merged = match wysiwyg_session_from_doc(json) {
                Some((session, text)) if text == content => Some((json.clone(), session)),
                _ => rtcl_tk_runtime::merge_rich_doc(path, &content).and_then(|merged| {
                    wysiwyg_session_from_doc(&merged).map(|(session, _)| (merged, session))
                }),
            };
            if let Some((json, session)) = merged {
                let styles = wysiwyg_styles_from_doc(&json);
                map.insert(
                    path.to_string(),
                    WysiwygEntry {
                        content,
                        signature,
                        session: session.clone(),
                    },
                );
                return Some((session, styles));
            }
        }
        let session = wysiwyg_session(&content, tags)?;
        let json = serde_json::to_string(&session.state().doc().to_json()).ok()?;
        rtcl_tk_runtime::set_rich_doc(path, json);
        let styles = wysiwyg_styles(tags);
        map.insert(
            path.to_string(),
            WysiwygEntry {
                content,
                signature,
                session: session.clone(),
            },
        );
        Some((session, styles))
    });
    let Some((session, styles)) = built else {
        return render_rich_text(tree, path, tags);
    };
    let change_path = path.to_string();
    let change_session = session.clone();
    let height = option(tree, path, "-height")
        .parse::<f32>()
        .unwrap_or(10.0)
        .clamp(3.0, 40.0);
    view()
        .w_full()
        .h(px(height * 18.0 + 8.0))
        .child(
            rich_editor()
                .editor(session)
                .size_full()
                .styles(styles)
                .on_change(move |_: &_| {
                    let state = change_session.state();
                    let doc = state.doc();
                    let text = doc.text_between(0, doc.content().size(), Some("\n"), None);
                    if let Ok(json) = serde_json::to_string(&doc.to_json()) {
                        rtcl_tk_runtime::set_rich_doc(&change_path, json);
                    }
                    rtcl_tk_runtime::text_changed_editor(&change_path, &text);
                    let tags_now = rtcl_tk_runtime::text_tags(&change_path);
                    let signature = tags_signature(&tags_now);
                    WYSIWYG.with(|map| {
                        if let Some(entry) = map.borrow_mut().get_mut(&change_path) {
                            entry.content = text;
                            entry.signature = signature;
                        }
                    });
                    rtcl_tk_runtime::request_render();
                }),
        )
        .into_element()
}

fn render_rich_text(
    tree: &TkTree,
    path: &str,
    tags: &[rtcl_tk_runtime::TextTagView],
) -> AnyElement {
    let content = rtcl_tk_runtime::text_value(path);
    let chars = content.chars().collect::<Vec<_>>();
    let len = chars.len();
    let mut bounds = vec![0usize, len];
    for tag in tags {
        for (start, end) in &tag.ranges {
            bounds.push((*start).min(len));
            bounds.push((*end).min(len));
        }
    }
    bounds.sort_unstable();
    bounds.dedup();
    let base_size = 14.0f32;
    let default_color =
        option_color(tree, path, "-foreground").unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
    let mut block = text("").text_size(px(base_size));
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end {
            continue;
        }
        let segment = chars[start..end].iter().collect::<String>();
        let mut color = default_color;
        let mut background = None;
        let mut family: Option<SharedString> = None;
        let mut size = base_size;
        let mut bold = false;
        for tag in tags {
            let covers = tag.ranges.iter().any(|(a, b)| start < *b && end > *a);
            if !covers {
                continue;
            }
            if let Some(value) = tag.options.get("-foreground") {
                if let Some(parsed) = parse_color(value) {
                    color = parsed;
                }
            }
            if let Some(value) = tag.options.get("-background") {
                if let Some(parsed) = parse_color(value) {
                    background = Some(parsed);
                }
            }
            if let Some(value) = tag.options.get("-font") {
                if let Some((font_family, point_size, is_bold)) =
                    rtcl_tk_runtime::resolve_font(value)
                {
                    if !font_family.is_empty() {
                        family = Some(SharedString::from(font_family));
                    }
                    size = (point_size * 1.3333) as f32;
                    bold = is_bold;
                }
            }
        }
        let mut span = text(segment).text_size(px(size)).text_color(color);
        if let Some(background) = background {
            span = span.bg(background);
        }
        if let Some(family) = family {
            span = span.font_family(family);
        }
        if bold {
            span = span.font_weight(FontWeight::BOLD);
        }
        block = block.child(span);
    }
    let height = option(tree, path, "-height")
        .parse::<f32>()
        .unwrap_or(24.0)
        .clamp(1.0, 40.0);
    let mut container = view().w_full().h(px(height * 18.0 + 8.0)).p(px(4.0));
    if let Some(bg) = option_color(tree, path, "-background") {
        container = container.bg(bg);
    }
    container
        .child(scroll().w_full().child(block))
        .into_element()
}

fn image_file_of(tree: &TkTree, path: &str) -> Option<String> {
    let name = option(tree, path, "-image");
    if name.is_empty() {
        return None;
    }
    rtcl_tk_runtime::image_path(name)
}

fn sized_image(file: String, name: &str) -> AnyElement {
    let mut element = nsh_core::element::image(file);
    if let Some((width, height)) = rtcl_tk_runtime::image_dimensions(name) {
        if width > 0 {
            element = element.w(px(width as f32));
        }
        if height > 0 {
            element = element.h(px(height as f32));
        }
    }
    element.into_element()
}

fn font_of(tree: &TkTree, path: &str) -> (Option<SharedString>, f32, bool) {
    let spec = option(tree, path, "-font");
    if spec.is_empty() {
        return (None, 14.0, false);
    }
    match rtcl_tk_runtime::resolve_font(spec) {
        Some((family, size, bold)) => (
            (!family.is_empty()).then(|| SharedString::from(family)),
            (size * 1.3333) as f32,
            bold,
        ),
        None => (None, 14.0, false),
    }
}

fn styled_text(
    content: String,
    font: &(Option<SharedString>, f32, bool),
    color: AdaptiveColor,
) -> AnyElement {
    let mut element = text(content).text_size(px(font.1)).text_color(color);
    if let Some(family) = &font.0 {
        element = element.font_family(family.clone());
    }
    if font.2 {
        element = element.font_weight(FontWeight::BOLD);
    }
    element.into_element()
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
                row = row.child(render_packed(tree, c, true));
            }
            if !rights.is_empty() {
                for c in rights {
                    row = row.child(render_packed(tree, c, true));
                }
            }
            container = container.child(row);
        } else {
            container = container.child(render_packed(tree, child, false));
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

fn render_packed(tree: &TkTree, path: &str, in_row: bool) -> AnyElement {
    let pack = tree.get(path).and_then(|n| n.pack.clone());
    let mut cell = view();
    if let Some(p) = &pack {
        if p.expand {
            cell = cell.flex_grow(1.0);
        }
        match p.fill {
            Fill::X | Fill::Both if !in_row => cell = cell.w_full(),
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
    let inner = cell.child(render_node(tree, path));
    if !in_row
        && pack
            .as_ref()
            .is_some_and(|p| !matches!(p.fill, Fill::X | Fill::Both))
    {
        // A parcel that does not fill starts at the leading edge; nsh's
        // column default would align it to the end.
        view()
            .w_full()
            .flex_row()
            .justify_start()
            .child(inner)
            .into_element()
    } else {
        inner.into_element()
    }
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
    for (row_index, mut cells) in rows {
        cells.sort_by_key(|(_, column)| *column);
        let mut row = view().flex_row().items_center().gap_2().w_full();
        let row_weight = rtcl_tk_runtime::grid_weight(path, row_index, true);
        if row_weight > 0 {
            row = row.flex_grow(row_weight as f32);
        }
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
            if let Some(bg) = option_color(tree, path, "-background") {
                frame = frame.bg(bg);
            }
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
            if let Some(bg) = option_color(tree, path, "-background") {
                frame = frame.bg(bg);
            }
            if !label.is_empty() {
                frame = frame.child(
                    text(label)
                        .text_size(px(12.0))
                        .text_color(AdaptiveColor::rgb(0x4b5563, 0xd1d5db)),
                );
            }
            frame.child(render_children(tree, path)).into_element()
        }
        WidgetKind::Label => {
            if let Some(file) = image_file_of(tree, path) {
                return sized_image(file, option(tree, path, "-image"));
            }
            let font = font_of(tree, path);
            let mut container = view().w_full().items_start();
            if let Some(bg) = option_color(tree, path, "-background") {
                container = container.bg(bg);
            }
            let color = option_color(tree, path, "-foreground")
                .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
            container
                .child(styled_text(
                    rtcl_tk_runtime::display_text(path),
                    &font,
                    color,
                ))
                .into_element()
        }
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
                .bg(option_color(tree, path.as_str(), "-background")
                    .unwrap_or(AdaptiveColor::rgb(0xf3f4f6, 0x374151)))
                .disabled(disabled);
            button = if let Some(file) = image_file_of(tree, path.as_str()) {
                button.child(sized_image(file, option(tree, path.as_str(), "-image")))
            } else {
                button.child(styled_text(
                    rtcl_tk_runtime::display_text(&path),
                    &font_of(tree, path.as_str()),
                    option_color(tree, path.as_str(), "-foreground")
                        .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf9fafb)),
                ))
            };
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
                .child(styled_text(
                    label,
                    &font_of(tree, path.as_str()),
                    AdaptiveColor::rgb(0x111827, 0xf3f4f6),
                ));
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
                .child(styled_text(
                    label,
                    &font_of(tree, path.as_str()),
                    AdaptiveColor::rgb(0x111827, 0xf3f4f6),
                ));
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
            let tags = rtcl_tk_runtime::text_tags(path);
            if !tags.is_empty() || rtcl_tk_runtime::rich_doc(path).is_some() {
                return render_wysiwyg(tree, path, &tags);
            }
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
            let rows = option(tree, path, "-height")
                .parse::<usize>()
                .unwrap_or(10)
                .max(1);
            let total = items.len();
            let start = if total > rows {
                ((rtcl_tk_runtime::widget_scroll_fraction(path) * total as f64).round() as usize)
                    .min(total - rows)
            } else {
                0
            };
            let mut list = view()
                .flex_col()
                .w_full()
                .gap_1()
                .p(px(3.0))
                .h(px(rows as f32 * 28.0 + 8.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                .bg(option_color(tree, path, "-background")
                    .unwrap_or(AdaptiveColor::rgb(0xffffff, 0x111827)));
            let text_color = option_color(tree, path, "-foreground")
                .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
            let select_bg = option_color(tree, path, "-selectbackground")
                .unwrap_or(AdaptiveColor::rgb(0xdbeafe, 0x1e3a8a));
            let select_fg = option_color(tree, path, "-selectforeground")
                .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
            for (i, item) in items.iter().enumerate().skip(start).take(rows) {
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
                            select_bg
                        } else {
                            option_color(tree, path, "-background")
                                .unwrap_or(AdaptiveColor::rgb(0xffffff, 0x111827))
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
                                        .text_color(if selected { select_fg } else { text_color }),
                                ),
                        )
                        .on_press(move |_: &()| {
                            rtcl_tk_runtime::listbox_select(&row_path, i);
                        }),
                );
            }
            list.into_element()
        }
        WidgetKind::Message => {
            let font = font_of(tree, path);
            let mut container = view().w_full().items_start();
            if let Some(bg) = option_color(tree, path, "-background") {
                container = container.bg(bg);
            }
            let color = option_color(tree, path, "-foreground")
                .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
            container
                .child(styled_text(
                    rtcl_tk_runtime::display_text(path),
                    &font,
                    color,
                ))
                .into_element()
        }
        WidgetKind::Scrollbar => {
            let horizontal = option(tree, path, "-orient") == "horizontal";
            let (first, last) = rtcl_tk_runtime::scrollbar_fraction(path);
            let first = first as f32;
            let span = (last - first as f64).max(0.08) as f32;
            let thumb_color = AdaptiveColor::rgb(0x9ca3af, 0x6b7280);
            let track_color = AdaptiveColor::rgb(0xe5e7eb, 0x374151);
            let arrow = |label: &str, step: i64, path: String| {
                nsh_base::button()
                    .px(px(2.0))
                    .py(px(1.0))
                    .child(text(label.to_string()).text_size(px(10.0)))
                    .on_press(move |_: &()| {
                        rtcl_tk_runtime::scrollbar_step(&path, step, false);
                    })
                    .into_element()
            };
            if horizontal {
                let width = 160.0f32;
                let left = arrow("←", -1, path.to_string());
                let right = arrow("→", 1, path.to_string());
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(left)
                    .child(
                        view()
                            .w(px(width))
                            .h(px(14.0))
                            .rounded_full()
                            .bg(track_color)
                            .child(
                                view()
                                    .absolute()
                                    .left(px(first * width))
                                    .top(px(2.0))
                                    .w(px(span * width))
                                    .h(px(10.0))
                                    .rounded_full()
                                    .bg(thumb_color),
                            ),
                    )
                    .child(right)
                    .into_element()
            } else {
                let height = 100.0f32;
                let up = arrow("↑", -1, path.to_string());
                let down = arrow("↓", 1, path.to_string());
                view()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(up)
                    .child(
                        view()
                            .w(px(14.0))
                            .h(px(height))
                            .rounded_full()
                            .bg(track_color)
                            .child(
                                view()
                                    .absolute()
                                    .top(px(first * height))
                                    .left(px(2.0))
                                    .h(px(span * height))
                                    .w(px(10.0))
                                    .rounded_full()
                                    .bg(thumb_color),
                            ),
                    )
                    .child(down)
                    .into_element()
            }
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
        WidgetKind::Canvas => {
            let width = option(tree, path, "-width")
                .parse::<f32>()
                .unwrap_or(378.0)
                .max(1.0);
            let height = option(tree, path, "-height")
                .parse::<f32>()
                .unwrap_or(265.0)
                .max(1.0);
            let Some(model) = rtcl_tk_runtime::canvas_snapshot(path) else {
                return view().into_element();
            };
            let region = rtcl_core::value::Value::from_str(option(tree, path, "-scrollregion"))
                .as_list()
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().trim().parse::<f32>().ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let (x_off, y_off) = if region.len() >= 4 {
                (
                    model.xview as f32 * ((region[2] - region[0]) - width).max(0.0),
                    model.yview as f32 * ((region[3] - region[1]) - height).max(0.0),
                )
            } else {
                (0.0, 0.0)
            };
            let mut painter = Painter::new();
            if x_off != 0.0 || y_off != 0.0 {
                painter = painter.translate(-x_off, -y_off);
            }
            for item in &model.items {
                let coords = &item.coords;
                let option =
                    |name: &str| item.options.get(name).and_then(|value| parse_color(value));
                let center = match item.kind.as_str() {
                    "rectangle" | "oval" if coords.len() >= 4 => (
                        (coords[0] + coords[2]) as f32 / 2.0,
                        (coords[1] + coords[3]) as f32 / 2.0,
                    ),
                    "line" | "polygon" if coords.len() >= 2 => {
                        let n = (coords.len() / 2).max(1) as f32;
                        let mut sx = 0.0f32;
                        let mut sy = 0.0f32;
                        for pair in coords.chunks(2) {
                            sx += pair[0] as f32;
                            sy += pair.get(1).copied().unwrap_or(0.0) as f32;
                        }
                        (sx / n, sy / n)
                    }
                    "image" if coords.len() >= 2 => {
                        (coords[0] as f32 + 40.0, coords[1] as f32 + 40.0)
                    }
                    _ if coords.len() >= 2 => (coords[0] as f32, coords[1] as f32),
                    _ => (0.0, 0.0),
                };
                let angle = item
                    .options
                    .get("-angle")
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(0.0)
                    .to_radians();
                let rotated = angle != 0.0;
                if rotated {
                    painter = painter
                        .save()
                        .translate(center.0, center.1)
                        .rotate(angle)
                        .translate(-center.0, -center.1);
                }
                match item.kind.as_str() {
                    "rectangle" if coords.len() >= 4 => {
                        let (x, y, w, h) = (
                            coords[0] as f32,
                            coords[1] as f32,
                            (coords[2] - coords[0]) as f32,
                            (coords[3] - coords[1]) as f32,
                        );
                        if let Some(fill) = option("-fill") {
                            painter = painter.fill_color(fill.light).fill_rect(x, y, w, h);
                        }
                        if let Some(outline) = option("-outline") {
                            painter = painter
                                .stroke_color(outline.light)
                                .line_width(1.0f32)
                                .begin_path()
                                .rect(nsh_core::geometry::Bounds::new(
                                    Point::new(Points(x), Points(y)),
                                    Size::new(Points(w), Points(h)),
                                ))
                                .stroke();
                        }
                    }
                    "line" if coords.len() >= 4 => {
                        let color = option("-fill")
                            .or_else(|| option("-outline"))
                            .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
                        let line_width = item
                            .options
                            .get("-width")
                            .and_then(|v| v.parse::<f32>().ok())
                            .unwrap_or(1.0);
                        let smooth = item.options.get("-smooth").cloned().unwrap_or_default();
                        let points = coords
                            .chunks(2)
                            .filter(|p| p.len() == 2)
                            .map(|p| (p[0] as f32, p[1] as f32))
                            .collect::<Vec<_>>();
                        painter = painter
                            .stroke_color(color.light)
                            .line_width(line_width)
                            .begin_path()
                            .move_to(points[0].0, points[0].1);
                        if smooth == "bezier" && points.len() > 2 {
                            for i in 0..points.len() - 1 {
                                let p1 = points[i];
                                let p2 = points[i + 1];
                                let p0 = if i == 0 { p1 } else { points[i - 1] };
                                let p3 = if i + 2 < points.len() {
                                    points[i + 2]
                                } else {
                                    p2
                                };
                                let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
                                let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
                                painter = painter.bezier_curve_to(
                                    Point::new(Points(c1.0), Points(c1.1)),
                                    Point::new(Points(c2.0), Points(c2.1)),
                                    Point::new(Points(p2.0), Points(p2.1)),
                                );
                            }
                        } else if !matches!(smooth.as_str(), "" | "0" | "false" | "raw")
                            && points.len() > 2
                        {
                            for i in 1..points.len() - 1 {
                                let (cx, cy) = points[i];
                                let (nx, ny) = points[i + 1];
                                painter = painter.quadratic_curve_to(
                                    Point::new(Points(cx), Points(cy)),
                                    Point::new(Points((cx + nx) / 2.0), Points((cy + ny) / 2.0)),
                                );
                            }
                            let last = points[points.len() - 1];
                            painter = painter.line_to(last.0, last.1);
                        } else {
                            for (x, y) in &points[1..] {
                                painter = painter.line_to(*x, *y);
                            }
                        }
                        if let Some(dash) = item.options.get("-dash").filter(|d| !d.is_empty()) {
                            let segments = rtcl_core::value::Value::from_str(dash)
                                .as_list()
                                .map(|list| {
                                    list.iter()
                                        .filter_map(|v| v.as_str().parse::<f32>().ok())
                                        .map(Points)
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            if !segments.is_empty() {
                                painter = painter.line_dash(&segments);
                            }
                        }
                        if let Some(offset) = item
                            .options
                            .get("-dashoffset")
                            .and_then(|v| v.parse::<f32>().ok())
                        {
                            painter = painter.line_dash_offset(offset);
                        }
                        painter = painter.stroke();
                        let arrow = item.options.get("-arrow").cloned().unwrap_or_default();
                        if matches!(arrow.as_str(), "last" | "both") && points.len() >= 2 {
                            let n = points.len();
                            painter = arrow_head(
                                painter,
                                points[n - 2],
                                points[n - 1],
                                color.light,
                                10.0,
                            );
                        }
                        if matches!(arrow.as_str(), "first" | "both") && points.len() >= 2 {
                            painter = arrow_head(painter, points[1], points[0], color.light, 10.0);
                        }
                    }
                    "oval" if coords.len() >= 4 => {
                        let center = Point::new(
                            Points(((coords[0] + coords[2]) / 2.0) as f32),
                            Points(((coords[1] + coords[3]) / 2.0) as f32),
                        );
                        let radii = Size::new(
                            Points(((coords[2] - coords[0]).abs() / 2.0) as f32),
                            Points(((coords[3] - coords[1]).abs() / 2.0) as f32),
                        );
                        painter = painter.begin_path().ellipse(
                            center,
                            radii,
                            Radians(0.0),
                            Radians(0.0),
                            Radians(std::f32::consts::TAU),
                            false,
                        );
                        if let Some(fill) = option("-fill") {
                            painter = painter.fill_color(fill.light).fill();
                        }
                        if let Some(outline) = option("-outline") {
                            painter = painter
                                .stroke_color(outline.light)
                                .line_width(1.0f32)
                                .stroke();
                        }
                    }
                    "text" if coords.len() >= 2 => {
                        let text = item.options.get("-text").cloned().unwrap_or_default();
                        if !text.is_empty() {
                            let color =
                                option("-fill").unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
                            painter = painter.fill_color(color.light).fill_text(
                                &text,
                                coords[0] as f32,
                                coords[1] as f32,
                            );
                        }
                    }
                    "image" if coords.len() >= 2 => {
                        if let Some(name) = item.options.get("-image") {
                            if let Some(file) = rtcl_tk_runtime::image_path(name) {
                                let (iw, ih) =
                                    rtcl_tk_runtime::image_dimensions(name).unwrap_or((80, 80));
                                painter = painter.draw_image(
                                    &file,
                                    nsh_core::geometry::Bounds::new(
                                        Point::new(
                                            Points(coords[0] as f32),
                                            Points(coords[1] as f32),
                                        ),
                                        Size::new(Points(iw as f32), Points(ih as f32)),
                                    ),
                                );
                            }
                        }
                    }
                    _ => {}
                }
                if rotated {
                    painter = painter.restore();
                }
            }
            let mut container = view().w(px(width)).h(px(height)).items_start();
            if let Some(bg) = option_color(tree, path, "-background") {
                container = container.bg(bg);
            }
            let press_path = path.to_string();
            container
                .child(
                    nsh_core::element::canvas(painter)
                        .w(px(width))
                        .h(px(height))
                        .on_press(move |e: &PressEvent| {
                            rtcl_tk_runtime::canvas_press(
                                &press_path,
                                e.position.x.0 as f64,
                                e.position.y.0 as f64,
                            );
                        }),
                )
                .into_element()
        }
        WidgetKind::Chart => {
            let width = option(tree, path, "-width")
                .parse::<f32>()
                .unwrap_or(300.0)
                .max(40.0);
            let height = option(tree, path, "-height")
                .parse::<f32>()
                .unwrap_or(160.0)
                .max(40.0);
            let values = rtcl_core::value::Value::from_str(option(tree, path, "-values"))
                .as_list()
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().trim().parse::<f64>().ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let labels = rtcl_core::value::Value::from_str(option(tree, path, "-labels"))
                .as_list()
                .map(|list| {
                    list.iter()
                        .map(|v| v.as_str().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let max = values.iter().copied().fold(0.0f64, f64::max).max(1.0);
            let count = values.len().max(1) as f32;
            let bar_width = ((width - 8.0 - (count - 1.0) * 6.0) / count).max(4.0);
            let accent = AdaptiveColor::rgb(0x2563eb, 0x3b82f6);
            let mut bars = view()
                .flex_row()
                .items_end()
                .justify_start()
                .w(px(width))
                .h(px(height))
                .gap_1();
            for (i, value) in values.iter().enumerate() {
                let bar_height = ((*value / max) as f32 * (height - 28.0)).max(2.0);
                let label = labels.get(i).cloned().unwrap_or_default();
                bars = bars.child(
                    view()
                        .flex_col()
                        .items_center()
                        .justify_end()
                        .h(px(height))
                        .child(
                            view()
                                .w(px(bar_width))
                                .h(px(bar_height))
                                .rounded_t(px(3.0))
                                .bg(accent),
                        )
                        .child(
                            text(label)
                                .text_size(px(11.0))
                                .text_color(AdaptiveColor::rgb(0x4b5563, 0xd1d5db)),
                        ),
                );
            }
            let mut chart = view().flex_col().items_start().gap_2();
            if let Some(bg) = option_color(tree, path, "-background") {
                chart = chart.bg(bg);
            }
            let title = option(tree, path, "-title");
            if !title.is_empty() {
                chart = chart.child(
                    text(title.to_string())
                        .text_size(px(13.0))
                        .text_color(AdaptiveColor::rgb(0x111827, 0xf3f4f6)),
                );
            }
            chart.child(bars).into_element()
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
            let total = order.len();
            let rows = option(tree, path, "-height")
                .parse::<usize>()
                .unwrap_or(10)
                .max(1);
            let start = if total > rows {
                ((rtcl_tk_runtime::widget_scroll_fraction(path) * total as f64).round() as usize)
                    .min(total - rows)
            } else {
                0
            };
            for (item, depth) in order.iter().skip(start).take(rows) {
                let path = path.to_string();
                let id = item.id.clone();
                let has_children = tv
                    .items
                    .iter()
                    .any(|i| i.parent.as_deref() == Some(id.as_str()));
                let selected = tv.selected.contains(&id);
                let mut row = view().flex_row().w_full().items_center().gap_1();
                if *depth > 0 {
                    row = row.child(view().w(px(16.0 * *depth as f32)));
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
                let select_path = path.clone();
                let select_id = id.clone();
                let mut cells = view().flex_row().w_full().items_center().gap_2();
                cells = cells.child(
                    text(mark.to_string())
                        .text_size(px(12.0))
                        .text_color(AdaptiveColor::rgb(0x2563eb, 0x60a5fa)),
                );
                let text_color = option_color(tree, &path, "-foreground")
                    .unwrap_or(AdaptiveColor::rgb(0x111827, 0xf3f4f6));
                cells = cells.child(
                    text(item.text.clone())
                        .text_size(px(13.0))
                        .text_color(text_color),
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
            let body = body.h(px(rows as f32 * 22.0 + 8.0));
            view()
                .flex_col()
                .w_full()
                .rounded(px(4.0))
                .border_1()
                .border_color(AdaptiveColor::rgb(0xd1d5db, 0x4b5563))
                .child(header)
                .child(body)
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
