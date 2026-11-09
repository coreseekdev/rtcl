//! The Linux development host with `nsh/base` and the app's Rust components.

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    nsh_platform_linux::set_ui_sets(vec![nsh_base::registry(), set_app_rtcl_tk_app_native()]);
    #[cfg(target_os = "linux")]
    nsh_platform_linux::set_components(app_components);
    #[cfg(target_os = "linux")]
    nsh_platform_linux::set_root(app_root);
    nsh_linux_host::main()
}

/// The UI set of the crate rtcl-tk-app-native: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "linux")]
fn set_app_rtcl_tk_app_native() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_rtcl_tk_app_native::*;
    __nsh_native_module() // the crate rtcl-tk-app-native (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(linux)]` registers in its crates.
#[cfg(target_os = "linux")]
fn app_components(__nsh_components: &nsh_platform_linux::Components) {
    {
        #[allow(unused_imports)]
        use app_rtcl_tk_app_native::*;
        __nsh_native_components_linux(__nsh_components); // the crate rtcl-tk-app-native (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_linux;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "linux")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_rtcl_tk_app_native::TclApp>()
        .app_id("dev.rtcl.tkapp")
}
