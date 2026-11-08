//! nsh renderer for the native Tk shadow tree.
//!
//! The Tcl command layer lives in `rtcl-tk-runtime`; this crate turns a
//! [`TkTree`] snapshot into nsh elements.  It is deliberately a thin
//! projection: widget semantics (paths/options/pack/grid) stay in
//! `rtcl-tk-core`, and the nsh platform crates own the actual native views.
//!
//! M1b surface: toplevel/frame/label/button/entry.  Button presses evaluate
//! the Tk `-command` script on the active interpreter and bump a global
//! revision signal; the component reads that signal, so nsh re-renders the
//! tree after a Tcl mutation.

use nsh_core::prelude::*;
use rtcl_tk_core::{TkTree, WidgetKind};

/// Global revision bumped by Tk mutations; read by [`TkView`].
#[derive(Default, Clone, Copy)]
struct TkRevision(u64);

fn notify_backend() {
    let rev = global::<TkRevision>();
    rev.set(TkRevision(rev.get().0.wrapping_add(1)));
}

/// Attach the renderer's invalidation hook to the Tk runtime.
pub fn install_backend() {
    rtcl_tk_runtime::set_change_hook(Some(notify_backend));
}

/// The whole Tk tree as an nsh element.
#[component]
pub fn TkView() -> impl IntoElement {
    let _revision = global::<TkRevision>().get().0;
    match rtcl_tk_runtime::snapshot_active() {
        Some(tree) => render_children(&tree, "."),
        None => view()
            .flex_col()
            .child(text("Tk runtime is not installed").text_size(px(14.0)))
            .into_element(),
    }
}

fn option<'a>(tree: &'a TkTree, path: &str, name: &str) -> &'a str {
    tree.get(path)
        .and_then(|n| n.options.get(name))
        .map(String::as_str)
        .unwrap_or("")
}

fn render_children(tree: &TkTree, path: &str) -> AnyElement {
    let mut root = view().flex_col().gap_2();
    if let Some(n) = tree.get(path) {
        for child in &n.children {
            root = root.child(render_node(tree, child));
        }
    }
    root.into_element()
}

fn render_node(tree: &TkTree, path: &str) -> AnyElement {
    let Some(n) = tree.get(path) else {
        return text("").into_element();
    };
    match n.kind {
        WidgetKind::Toplevel | WidgetKind::Frame => render_children(tree, path),
        WidgetKind::Label => text(option(tree, path, "-text").to_string())
            .text_size(px(14.0))
            .into_element(),
        WidgetKind::Button => {
            let cmd = option(tree, path, "-command").to_string();
            let label = signal(SharedString::from(option(tree, path, "-text").to_string()));
            let label_for_cb = label;
            let path_for_cb = path.to_string();
            nsh_base::button()
                .px(px(10.0))
                .py(px(6.0))
                .child(text(label).text_size(px(14.0)))
                .on_press(move |_: &()| {
                    if !cmd.is_empty() {
                        // Tcl errors are left to the host's bgerror route in
                        // the next milestone; the button label still follows
                        // the shadow tree.
                        let _ = rtcl_tk_runtime::eval_active(&cmd);
                        let new_label = rtcl_tk_runtime::snapshot_active().and_then(|tree| {
                            tree.get(&path_for_cb)
                                .and_then(|n| n.options.get("-text").cloned())
                        });
                        if let Some(text) = new_label {
                            label_for_cb.set(SharedString::from(text));
                        }
                    }
                })
                .into_element()
        }
        WidgetKind::Entry => nsh_base::input()
            .default_value(option(tree, path, "-text").to_string())
            .into_element(),
        _ => text(format!("[{}]", option(tree, path, "-text"))).into_element(),
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
