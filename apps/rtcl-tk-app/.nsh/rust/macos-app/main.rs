//! The macOS release runner with `nsh/base` and the app's Rust components.

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    nsh_platform_macos::set_ui_sets(vec![nsh_base::registry(), set_app_rtcl_tk_app_native()]);
    #[cfg(target_os = "macos")]
    nsh_platform_macos::set_components(app_components);
    nsh_macos_app::main()
}

/// The UI set of the crate rtcl-tk-app-native: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "macos")]
fn set_app_rtcl_tk_app_native() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_rtcl_tk_app_native::*;
    __nsh_native_module() // the crate rtcl-tk-app-native (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(macos)]` registers in its crates.
#[cfg(target_os = "macos")]
fn app_components(__nsh_components: &nsh_platform_macos::Components) {
    {
        #[allow(unused_imports)]
        use app_rtcl_tk_app_native::*;
        __nsh_native_components_macos(__nsh_components); // the crate rtcl-tk-app-native (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_macos;
