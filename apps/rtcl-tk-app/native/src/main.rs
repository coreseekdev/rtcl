//! Runs the app on the machine's native backend (desktop debugging).

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    rtcl_tk_nsh::install_backend();
    // A windowed app shows the Tk menu as the platform's menu bar (the
    // Windows host builds a Win32 bar; macOS/Linux detect themselves).
    rtcl_tk_nsh::set_native_menus(true);
    platform::set_ui_sets(vec![nsh_base::registry()]);
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<rtcl_tk_app_native::TclApp>()),
        platform::AppOptions {
            title: "rtcl Tk".to_owned(),
            app_id: "dev.rtcl.tkapp".to_owned(),
            size: (520, 260),
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("run this app via `nsh run` on iOS, Android and the web");
}
