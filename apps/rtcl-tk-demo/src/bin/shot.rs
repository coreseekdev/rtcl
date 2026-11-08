//! Render the Tk demo's GTK4 window to a PNG (visual acceptance, no
//! TypeScript).
//!
//! `cargo run -p rtcl-tk-demo --bin shot -- out.png` opens the app, waits
//! for the first frames, renders the window content offscreen with GSK,
//! writes the PNG and quits.  Interactive states are covered by the
//! headless tests; start a script in the state you want to see instead of
//! mutating after the first frame (an updated GTK widget snapshots empty on
//! this backend).
//!
//! The snapshot runs on GTK's `after-paint` signal (with a 500 ms fallback),
//! as nsh's own test agent does: a widget that is dirty after a Tk re-render
//! renders as an empty texture until the frame is painted.
//!
//! Under a machine without a display: `xvfb-run -a cargo run ...`.

#[cfg(target_os = "linux")]
fn snapshot(window: &gtk4::Window) -> Result<Vec<u8>, String> {
    use gtk4::prelude::*;

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

/// Snapshot once the window's next frame is painted, then quit the app.
#[cfg(target_os = "linux")]
fn capture_once(path: std::path::PathBuf, done: std::sync::Arc<std::sync::Mutex<bool>>) {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use gtk4::prelude::*;

    let Some(app) =
        gtk4::gio::Application::default().and_then(|a| a.downcast::<gtk4::Application>().ok())
    else {
        eprintln!("screenshot: no GTK application");
        return;
    };
    let Some(window) = app.windows().into_iter().next() else {
        eprintln!("screenshot: the app has no window");
        return;
    };

    let once = Rc::new(Cell::new(false));
    let finish = {
        let window = window.clone();
        let path = path.clone();
        let done = done.clone();
        let once = once.clone();
        Rc::new(move || {
            if once.replace(true) {
                return;
            }
            match snapshot(&window) {
                Ok(png) => match std::fs::write(&path, png) {
                    Ok(()) => eprintln!("wrote {}", path.display()),
                    Err(e) => eprintln!("cannot write {}: {e}", path.display()),
                },
                Err(e) => eprintln!("screenshot: {e}"),
            }
            *done.lock().unwrap() = true;
            if let Some(app) = gtk4::gio::Application::default() {
                app.quit();
            }
        })
    };

    // An occluded Wayland window gets no frames; raise it before waiting.
    window.present();
    // Let GTK paint what the last Tk re-render dirtied; a snapshot taken
    // in the same turn as the change renders an empty texture.
    let fallback = finish.clone();
    gtk4::glib::timeout_add_local_once(Duration::from_millis(500), move || fallback());
}

#[cfg(target_os = "linux")]
fn main() {
    use std::time::Duration;

    use nsh_platform_linux as platform;

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let classic = args.iter().any(|a| a == "--classic");
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "tk-demo.png".to_string());
    let path = std::path::PathBuf::from(out);

    // The shot must run on GTK's main thread, after the app has had time to
    // run its `after` work.
    std::thread::spawn(move || {
        let delay = std::env::var("RTCL_SHOT_DELAY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(900);
        std::thread::sleep(Duration::from_millis(delay));
        let done = std::sync::Arc::new(std::sync::Mutex::new(false));
        let done_for_main = done.clone();
        gtk4::glib::MainContext::default().invoke(move || {
            capture_once(path, done_for_main);
        });
        for _ in 0..100 {
            if *done.lock().unwrap() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    platform::set_ui_sets(vec![nsh_base::registry()]);
    let source = if classic {
        platform::AppSource::Rust(platform::Root::of::<rtcl_tk_demo::ClassicApp>())
    } else {
        platform::AppSource::Rust(platform::Root::of::<rtcl_tk_demo::DemoApp>())
    };
    let size = std::env::var("RTCL_SHOT_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(if classic { (760, 1000) } else { (560, 420) });
    platform::run_app(
        source,
        platform::AppOptions {
            title: "rtcl Tk demo".to_owned(),
            app_id: "dev.rtcl.tkdemo".to_owned(),
            size,
            ..Default::default()
        },
    );
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the screenshot demo builds on Linux/GTK4 only");
}
