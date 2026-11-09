//! Describes the app's UI sets for `nsh`: one JSON document on standard
//! output (nsh-core `ui::description`).

fn main() {
    let sets = vec![set_app_rtcl_tk_app_native()];
    println!("{}", nsh_core::element::description::Description::of(&sets).to_json());
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
