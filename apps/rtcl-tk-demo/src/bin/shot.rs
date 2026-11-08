//! Render the Tk demo's GTK4 window to a PNG (visual acceptance, no
//! TypeScript).
//!
//! `cargo run -p rtcl-tk-demo --bin shot -- out.png` opens the app, waits
//! for the first frames plus the `after` timer, renders the window content
//! offscreen with GSK, writes the PNG and quits.  Set `RTCL_TK_AUTO=1` to
//! invoke the demo button first (the `shot` script schedules that from
//! Tcl).
//!
//! Under a machine without a display: `xvfb-run -a cargo run ...`.

#[cfg(target_os = "linux")]
fn capture() -> Result<Vec<u8>, String> {
    use gtk4::prelude::*;

    let app = gtk4::gio::Application::default()
        .and_then(|a| a.downcast::<gtk4::Application>().ok())
        .ok_or("no GTK application")?;
    let window = app
        .windows()
        .into_iter()
        .next()
        .ok_or("the app has no window")?;
    let child = window.child().ok_or("the window shows nothing")?;
    let (w, h) = (child.width(), child.height());
    let scale = f64::from(window.scale_factor().max(1));
    let paintable = gtk4::WidgetPaintable::new(Some(&child));
    let snapshot = gtk4::Snapshot::new();
    snapshot.scale(scale as f32, scale as f32);
    paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
    let (pw, ph) = ((f64::from(w) * scale) as f32, (f64::from(h) * scale) as f32);
    let node = snapshot.to_node().unwrap_or_else(|| {
        gtk4::gsk::ColorNode::new(
            &gtk4::gdk::RGBA::TRANSPARENT,
            &gtk4::graphene::Rect::new(0.0, 0.0, pw, ph),
        )
        .upcast()
    });
    let renderer = window.renderer().ok_or("the window has no renderer")?;
    let bounds = gtk4::graphene::Rect::new(0.0, 0.0, pw, ph);
    let texture = renderer.render_texture(&node, Some(&bounds));
    Ok(texture.save_to_png_bytes().to_vec())
}

#[cfg(target_os = "linux")]
fn main() {
    use std::time::Duration;

    use gtk4::prelude::*;
    use nsh_platform_linux as platform;

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tk-demo.png".to_string());
    let path = std::path::PathBuf::from(out);

    // The shot must run on GTK's main thread, after the app has painted.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(900));
        let done = std::sync::Arc::new(std::sync::Mutex::new(false));
        let done_for_main = done.clone();
        gtk4::glib::MainContext::default().invoke(move || {
            match capture() {
                Ok(png) => match std::fs::write(&path, png) {
                    Ok(()) => eprintln!("wrote {}", path.display()),
                    Err(e) => eprintln!("cannot write {}: {e}", path.display()),
                },
                Err(e) => eprintln!("screenshot: {e}"),
            }
            *done_for_main.lock().unwrap() = true;
            if let Some(app) = gtk4::gio::Application::default() {
                app.quit();
            }
        });
        for _ in 0..100 {
            if *done.lock().unwrap() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    platform::set_ui_sets(vec![nsh_base::registry()]);
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<rtcl_tk_demo::DemoApp>()),
        platform::AppOptions {
            title: "rtcl Tk demo".to_owned(),
            app_id: "dev.rtcl.tkdemo".to_owned(),
            size: (560, 420),
            ..Default::default()
        },
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the screenshot demo builds on Linux/GTK4 only");
}
