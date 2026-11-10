//! Evaluates a Tcl script against the GTK-backed Tk runtime and prints what
//! it `puts`: rtcl's side of a comparison with real Tk (`wish`).
//!
//! The model-only runtime answers font questions with size-based
//! approximations; the GTK backend measures with pango, and only registers
//! that measurer once GTK is up.  This binary brings GTK up, installs the
//! backend and runs the script without opening a window.
//!
//! `cargo run -p rtcl-tk-demo --bin tkprobe -- script.tcl`
//! (see `specs/tk/run.sh` for the comparison against `wish`).

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: tkprobe script.tcl");
        return ExitCode::from(2);
    };
    let script = match std::fs::read_to_string(&path) {
        Ok(script) => script,
        Err(e) => {
            eprintln!("tkprobe: couldn't read \"{path}\": {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = gtk4::init() {
        eprintln!("tkprobe: GTK did not start: {e}");
        return ExitCode::from(1);
    }
    // The runtime keeps the interpreter's address: it stays in this frame.
    let mut interp = rtcl_core::Interp::new();
    rtcl_tk_runtime::install(&mut interp);
    rtcl_tk_nsh::install_backend();
    match interp.eval(&script) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            // `exit` unwinds as an error carrying the status, as the
            // other hosts read it.
            let message = e.to_string();
            match message
                .strip_prefix("exit with value: ")
                .and_then(|code| code.trim().parse::<u8>().ok())
            {
                Some(code) => ExitCode::from(code),
                None => {
                    eprintln!("tkprobe: {message}");
                    ExitCode::from(1)
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("tkprobe measures through the GTK backend: Linux only");
}
