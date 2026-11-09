//! The app's `libnsh.so`: the Android runtime with `nsh/base` and the
//! app's Rust components.

#[cfg(target_os = "android")]
nsh::ui_sets!(vec![nsh_base::registry(), set_app_rtcl_tk_app_native()]);
#[cfg(target_os = "android")]
nsh::root!(app_root());

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "android")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_rtcl_tk_app_native::TclApp>()
        .app_id("dev.rtcl.tkapp")
}

/// The UI set of the crate rtcl-tk-app-native: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "android")]
fn set_app_rtcl_tk_app_native() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_rtcl_tk_app_native::*;
    __nsh_native_module() // the crate rtcl-tk-app-native (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "android")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;
