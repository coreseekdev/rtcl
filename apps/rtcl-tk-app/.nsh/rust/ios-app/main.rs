//! The iOS release runner with `nsh/base` and the app's Rust components.

fn main() {
    #[cfg(target_os = "ios")]
    nsh_platform_ios::set_ui_sets(vec![nsh_base::registry(), set_app_rtcl_tk_app_native()]);
    #[cfg(target_os = "ios")]
    nsh_platform_ios::set_components(app_components);
    nsh_ios_app::main()
}

/// The UI set of the crate rtcl-tk-app-native: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "ios")]
fn set_app_rtcl_tk_app_native() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_rtcl_tk_app_native::*;
    __nsh_native_module() // the crate rtcl-tk-app-native (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "ios")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(ios)]` registers in its crates.
#[cfg(target_os = "ios")]
fn app_components(__nsh_components: &nsh_platform_ios::Components) {
    {
        #[allow(unused_imports)]
        use app_rtcl_tk_app_native::*;
        __nsh_native_components_ios(__nsh_components); // the crate rtcl-tk-app-native (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "ios")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_ios;
