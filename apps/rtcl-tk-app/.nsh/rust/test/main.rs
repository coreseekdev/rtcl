//! The app's headless runner (`nsh test`): its test files and the apps
//! they open, with `nsh/base` and the app's Rust components.

fn main() -> std::process::ExitCode {
    nsh_testing::runner(vec![nsh_base::registry(), set_app_rtcl_tk_app_native()], Some(app_root))
}

/// The UI set of the crate rtcl-tk-app-native: the entry `#[native_module("…")]` makes in it.
fn set_app_rtcl_tk_app_native() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_rtcl_tk_app_native::*;
    __nsh_native_module() // the crate rtcl-tk-app-native (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_rtcl_tk_app_native::TclApp>()
        .app_id("dev.rtcl.tkapp")
}
