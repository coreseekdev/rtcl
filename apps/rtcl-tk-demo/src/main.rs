//! GTK4 entry point for the native Tk demo (M1b).

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;

#[cfg(target_os = "linux")]
fn main() -> impl std::process::Termination {
    platform::set_ui_sets(vec![nsh_base::registry()]);
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<rtcl_tk_demo::DemoApp>()),
        platform::AppOptions {
            title: "rtcl Tk demo".to_owned(),
            app_id: "dev.rtcl.tkdemo".to_owned(),
            size: (520, 260),
            ..Default::default()
        },
    )
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("this demo currently builds on Linux/GTK4 only");
}
