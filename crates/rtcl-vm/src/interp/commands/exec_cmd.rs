//! exec command — subprocess execution with output capture.
//!
//! Tcl `exec` runs one or more subprocesses with I/O redirections and
//! returns the collected standard output.

use crate::error::{Error, ErrorCode, Result};
use crate::host::{
    BytesFd, ExecCommand, ExecIoError, FdMode, HostFd, HostFdFs, HostProcess, JobState, NullFd,
};
use crate::interp::Interp;
use crate::value::Value;

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(all(unix, not(target_arch = "wasm32")))]
mod tty {
    use std::sync::atomic::{AtomicBool, Ordering};

    pub static TSTP: AtomicBool = AtomicBool::new(false);
    pub static INT: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_tstp(_: libc::c_int) {
        TSTP.store(true, Ordering::SeqCst);
    }
    extern "C" fn on_int(_: libc::c_int) {
        INT.store(true, Ordering::SeqCst);
    }

    pub struct Guard {
        old_tstp: libc::sighandler_t,
        old_int: libc::sighandler_t,
    }

    impl Guard {
        pub fn install() -> Self {
            let old_tstp =
                unsafe { libc::signal(libc::SIGTSTP, on_tstp as libc::sighandler_t) };
            let old_int = unsafe { libc::signal(libc::SIGINT, on_int as libc::sighandler_t) };
            Self { old_tstp, old_int }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                libc::signal(libc::SIGTSTP, self.old_tstp);
                libc::signal(libc::SIGINT, self.old_int);
            }
        }
    }
}

fn exec_err(msg: std::io::Error) -> Error {
    Error::runtime(msg.to_string(), ErrorCode::Io)
}

fn exec_err_str(msg: impl std::fmt::Display) -> Error {
    Error::runtime(msg.to_string(), ErrorCode::Io)
}

/// The redirection operator a word of `exec` begins with, and the rest of
/// the word: tclsh's target may be attached (`>/dev/null`, `2>>file`,
/// `<<value`) or come as the next word.  Longest operator first, so `>>`
/// is not `>` writing to a file named `>x`.
fn redirect_op(word: &str) -> Option<(&'static str, &str)> {
    const OPS: [&str; 16] = [
        "2>@1", "2>>@", "2>>", "2>@", "2>", "<<", ">>&@", ">>&", ">>@", ">>", ">&@", ">&", ">@",
        ">", "<@", "<",
    ];
    OPS.iter()
        .find_map(|op| word.strip_prefix(op).map(|rest| (*op, rest)))
}

/// A usage error of `exec`, with the errorCode tclsh raises it with.
fn exec_usage_error(interp: &mut Interp, msg: impl Into<String>, code: &str) -> Error {
    super::list::set_error_code(interp, code);
    Error::runtime(msg.into(), ErrorCode::Generic)
}

/// A pipe as two files: for two streams that must share it (`2>@1`, `|&`)
/// — with two `Stdio::piped()` the output would come out as all of one
/// stream and then all of the other, not as written.
#[cfg(unix)]
fn pipe_files() -> std::io::Result<(std::fs::File, std::fs::File)> {
    use std::os::unix::io::FromRawFd;
    let mut fds = [0; 2];
    // SAFETY: `fds` holds the two ints `pipe` writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both descriptors are fresh and owned from here on.
    unsafe { Ok((std::fs::File::from_raw_fd(fds[0]), std::fs::File::from_raw_fd(fds[1]))) }
}

/// How a child finished, for `exec`'s report.
enum ExitKind {
    Ok,
    Code(i32),
    #[allow(dead_code)]
    Signal(i32),
}

#[cfg(unix)]
fn exit_kind(status: &std::process::ExitStatus) -> ExitKind {
    use std::os::unix::process::ExitStatusExt;
    if let Some(sig) = status.signal() {
        return ExitKind::Signal(sig);
    }
    match status.code() {
        Some(0) | None => ExitKind::Ok,
        Some(c) => ExitKind::Code(c),
    }
}

#[cfg(not(unix))]
fn exit_kind(status: &std::process::ExitStatus) -> ExitKind {
    match status.code() {
        Some(0) | None => ExitKind::Ok,
        Some(c) => ExitKind::Code(c),
    }
}

/// A child's output as text, the way tclsh reads it: the pipe is a channel
/// in the system encoding (UTF-8) with `auto` translation.  Valid UTF-8
/// passes; every other byte is one character with the byte's value
/// (`printf 'A\377B'` is `AÿB`); a CR LF pair and a lone CR are each one
/// newline.
fn tcl_output_text(bytes: Vec<u8>) -> String {
    let bytes = if bytes.contains(&b'\r') {
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' {
                out.push(b'\n');
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        out
    } else {
        bytes
    };
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes.as_slice();
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                text.push_str(s);
                return text;
            }
            Err(e) => {
                let good = e.valid_up_to();
                if let Ok(s) = std::str::from_utf8(&rest[..good]) {
                    text.push_str(s);
                }
                text.push(char::from_u32(rest[good] as u32).unwrap_or('\u{fffd}'));
                rest = &rest[good + 1..];
            }
        }
    }
}

/// tclsh's signal report for a child that died from `sig`: the name and
/// the description (Tcl_SignalId / Tcl_SignalMsg, as 8.6.17 gives them on
/// Linux; the signals it does not know are `unknown signal`).
#[cfg(unix)]
fn signal_text(sig: i32) -> (&'static str, &'static str) {
    match sig {
        1 => ("SIGHUP", "hangup"),
        2 => ("SIGINT", "interrupt"),
        3 => ("SIGQUIT", "quit signal"),
        4 => ("SIGILL", "illegal instruction"),
        5 => ("SIGTRAP", "trace trap"),
        6 => ("SIGABRT", "SIGABRT"),
        7 => ("SIGBUS", "bus error"),
        8 => ("SIGFPE", "floating-point exception"),
        9 => ("SIGKILL", "kill signal"),
        10 => ("SIGUSR1", "user-defined signal 1"),
        11 => ("SIGSEGV", "segmentation violation"),
        12 => ("SIGUSR2", "user-defined signal 2"),
        13 => ("SIGPIPE", "write on pipe with no readers"),
        14 => ("SIGALRM", "alarm clock"),
        15 => ("SIGTERM", "software termination signal"),
        24 => ("SIGXCPU", "exceeded CPU time limit"),
        25 => ("SIGXFSZ", "exceeded file size limit"),
        26 => ("SIGVTALRM", "virtual time alarm"),
        27 => ("SIGPROF", "profiling alarm"),
        29 => ("SIGIO", "input/output possible on file"),
        30 => ("SIGPWR", "power-fail restart"),
        31 => ("SIGSYS", "bad argument to system call"),
        _ => ("unknown signal", "unknown signal"),
    }
}

/// Hand the children of a pipeline that failed after some stage had
/// already been spawned over to the background reaper.  Dropping a `Child`
/// neither kills nor waits, so a child simply left behind is a zombie;
/// tclsh neither kills nor blocks here either — it detaches them
/// (`Tcl_DetachPids`) and the next exec reaps whatever has exited.
fn detach_children(interp: &mut Interp, children: &mut Vec<std::process::Child>) {
    interp.background_children.append(children);
}

/// Stop and reap every child of a pipeline whose output could not be
/// read: they are still running against a pipe nobody will drain.
fn kill_and_reap(children: &mut [std::process::Child]) {
    for child in children {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Hand a `<<` string to the child from a helper thread. Written from the
/// parent it filled the pipe (64KiB) and the parent — not yet reading the
/// child's output — waited for it forever: `exec tr x y << [string
/// repeat x 200000]` never returned.
fn feed_stdin(mut stdin: std::process::ChildStdin, bytes: Vec<u8>) {
    std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&bytes);
        // Dropping stdin closes the pipe, so the child reads EOF.
    });
}

/// Read a child's stdout and stderr to EOF, both at once.
///
/// Draining stdout first and stderr after deadlocks as soon as the child
/// writes more than a pipe's worth to stderr while its stdout is still
/// open (the child blocks on stderr, the parent waits for stdout's EOF):
/// `exec sh -c {echo OUT; head -c 66000 /dev/zero | tr '\0' E >&2}` hung.
/// `poll(2)` on both descriptors, a non-blocking amount at a time.
#[cfg(unix)]
fn drain_both(
    mut stdout: std::process::ChildStdout,
    mut stderr: std::process::ChildStderr,
) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    use std::os::unix::io::AsRawFd;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut fds = [
        libc::pollfd { fd: stdout.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        libc::pollfd { fd: stderr.as_raw_fd(), events: libc::POLLIN, revents: 0 },
    ];
    let mut buf = [0u8; 65536];
    let mut open = 2usize;
    while open > 0 {
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        for (i, sink) in [(0usize, &mut out), (1usize, &mut err)] {
            if fds[i].fd < 0 || fds[i].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
                continue;
            }
            let n = if i == 0 { stdout.read(&mut buf) } else { stderr.read(&mut buf) };
            match n {
                Ok(0) => {
                    fds[i].fd = -1;
                    open -= 1;
                }
                Ok(k) => sink.extend_from_slice(&buf[..k]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok((out, err))
}

/// Where `poll(2)` is not there: stdout, then stderr (the deadlock above
/// stays possible on such a platform).
#[cfg(not(unix))]
fn drain_both(
    mut stdout: std::process::ChildStdout,
    mut stderr: std::process::ChildStderr,
) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.read_to_end(&mut out)?;
    stderr.read_to_end(&mut err)?;
    Ok((out, err))
}

/// Read a pipeline's captured output until the last child closes it; a
/// failure reaps the children first.  `merged` is the read end of a pipe
/// both streams share (`2>@1`), standing in for the child's stdout.
fn collect_output(
    interp: &mut Interp,
    children: &mut [std::process::Child],
    merged: Option<std::fs::File>,
) -> std::result::Result<(String, String), Error> {
    fn read_all<R: Read>(mut r: R) -> std::io::Result<Vec<u8>> {
        let mut v = Vec::new();
        r.read_to_end(&mut v)?;
        Ok(v)
    }
    fn read_failed(
        interp: &mut Interp,
        children: &mut [std::process::Child],
        e: std::io::Error,
    ) -> Error {
        kill_and_reap(children);
        super::io::posix_error(interp, "couldn't read the command's output".to_string(), &e)
    }
    if let Some(f) = merged {
        return read_all(f)
            .map(|v| (tcl_output_text(v), String::new()))
            .map_err(|e| read_failed(interp, children, e));
    }
    let last = children.last_mut().unwrap();
    let bytes = match (last.stdout.take(), last.stderr.take()) {
        (Some(o), Some(e)) => drain_both(o, e),
        (Some(o), None) => read_all(o).map(|v| (v, Vec::new())),
        (None, Some(e)) => read_all(e).map(|v| (Vec::new(), v)),
        (None, None) => Ok((Vec::new(), Vec::new())),
    };
    bytes
        .map(|(o, e)| (tcl_output_text(o), tcl_output_text(e)))
        .map_err(|e| read_failed(interp, children, e))
}

/// `exec ?-ignorestderr? ?-keepnewline? ?--? cmd ?arg ...? ?| cmd ...? ?redirections?`
impl Interp {
    /// Reap finished background `exec &` children without blocking.
    pub(crate) fn reap_background_children(&mut self) {
        let mut i = 0;
        while i < self.background_children.len() {
            match self.background_children[i].try_wait() {
                Ok(Some(status)) => {
                    let pid = self.background_children[i].id();
                    self.note_child_status(pid, status);
                    self.background_children.swap_remove(i);
                }
                Err(_) => {
                    self.background_children.swap_remove(i);
                }
                Ok(None) => i += 1,
            }
        }
    }

    /// Remember a background child's exit status for a later `wait` —
    /// the handle is dropped on reap, the status need not be.
    pub(crate) fn note_child_status(
        &mut self,
        pid: u32,
        status: std::process::ExitStatus,
    ) {
        if self.child_status.len() >= 4096 {
            self.child_status.clear();
        }
        self.child_status.insert(pid, raw_status(status));
    }
}

/// A process's exit status for `wait`: the exit code, or `128 + signal`
/// when a signal ended it (the raw wait status would be the code in bits
/// 8–15 — 768 for an exit of 3).
#[cfg(unix)]
pub(crate) fn raw_status(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.code().or_else(|| status.signal().map(|s| 128 + s)).unwrap_or(0)
}

#[cfg(not(unix))]
pub(crate) fn raw_status(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(0)
}

pub fn cmd_exec(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    interp.reap_background_children();
    if args.len() < 2 {
        // Bare: the registry's usage text completes it (tclsh: `exec
        // ?-option ...? arg ?arg ...?`).
        return Err(Error::wrong_args(args[0].as_str(), 2, args.len()));
    }

    // Host-backed exec（rogo）：外部命令是 wasm，所有 I/O 走 fd 表。
    // 没有注入 HostExec 的 rtcl-cli 仍走下面的原生 std 路径。
    if interp.host_exec().is_some() {
        return cmd_exec_host(interp, args);
    }

    // Parse switches: exactly, and anything else is refused by tclsh's
    // index-lookup error.
    const SWITCHES: [&str; 3] = ["-ignorestderr", "-keepnewline", "--"];
    let switch_table: Vec<String> = SWITCHES.iter().map(|o| o.to_string()).collect();
    let mut i = 1;
    let mut ignore_stderr = false;
    let mut keep_newline = false;
    while i < args.len() {
        let word = args[i].as_str();
        if !word.starts_with('-') {
            break;
        }
        let which = super::misc::index_in_table(word, &switch_table, "option", true)
            .map_err(|(e, code)| {
                super::list::set_error_code(interp, &code);
                e
            })?;
        match SWITCHES[which] {
            "-ignorestderr" => ignore_stderr = true,
            "-keepnewline" => keep_newline = true,
            _ => {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    if i >= args.len() {
        return Err(Error::wrong_args(args[0].as_str(), 2, args.len()));
    }

    // Split the remaining arguments at pipe tokens ("|" and "|&") into stages.
    // Also collect redirections that apply to the last (or only) stage.
    let mut stages: Vec<Vec<&str>> = Vec::new();
    let mut current_stage: Vec<&str> = Vec::new();
    let mut merge_stderr_in_pipe = false; // For "|&"
    let mut input_redirect: Option<InputRedirect> = None;
    let mut output_redirect: Option<OutputRedirect> = None;
    let mut stderr_redirect: Option<StderrRedirect> = None;
    let mut background = false;

    // The redirections in command-line order: tclsh opens them before it
    // starts any process.
    let mut redirects: Vec<Redirect> = Vec::new();
    let syntax_error = |interp: &mut Interp, token: &str| {
        exec_usage_error(
            interp,
            format!("can't specify \"{}\" as last word in command", token),
            "TCL OPERATION EXEC SYNTAX",
        )
    };

    while i < args.len() {
        let token = args[i].as_str();
        match token {
            "|" | "|&" => {
                if current_stage.is_empty() {
                    return Err(exec_usage_error(
                        interp,
                        "illegal use of | or |& in command",
                        "TCL OPERATION EXEC PIPESYNTAX",
                    ));
                }
                stages.push(std::mem::take(&mut current_stage));
                if token == "|&" {
                    merge_stderr_in_pipe = true;
                }
                i += 1;
                continue;
            }
            "&" if i + 1 == args.len() => {
                background = true;
                i += 1;
                continue;
            }
            _ => {}
        }
        let Some((op, attached)) = redirect_op(token) else {
            current_stage.push(token);
            i += 1;
            continue;
        };
        if op.ends_with('@') {
            // The child gets the channel's own descriptor.
            let name: String = if !attached.is_empty() {
                attached.to_string()
            } else {
                match args.get(i + 1) {
                    Some(t) => t.as_str().to_string(),
                    None => {
                        return Err(exec_usage_error(
                            interp,
                            format!("can't specify \"{}\" as last word in command", op),
                            "TCL OPERATION EXEC SYNTAX",
                        ));
                    }
                }
            };
            let append = op.starts_with(">>") || op == "2>>@" || op == ">>&@";
            let mut to = |stream: Stream| Redirect {
                stream,
                append,
                target: Target::Channel(name.clone()),
            };
            match op {
                "<@" => {
                    redirects.push(to(Stream::In));
                    input_redirect = Some(InputRedirect::Channel);
                }
                ">@" | ">>@" => {
                    redirects.push(to(Stream::Out));
                    output_redirect = Some(OutputRedirect::Channel);
                }
                ">&@" | ">>&@" => {
                    redirects.push(to(Stream::Out));
                    redirects.push(to(Stream::Err));
                    output_redirect = Some(OutputRedirect::Channel);
                    stderr_redirect = Some(StderrRedirect::Channel);
                }
                _ => {
                    redirects.push(to(Stream::Err));
                    stderr_redirect = Some(StderrRedirect::Channel);
                }
            }
            i += if attached.is_empty() { 2 } else { 1 };
            continue;
        }
        if op == "2>@1" {
            // It has no target, and redirects the last stage's output:
            // tclsh lets it only close the command.
            if i + 1 != args.len() {
                return Err(exec_usage_error(
                    interp,
                    "must specify \"2>@1\" as last word in command",
                    "TCL OPERATION EXEC PIPESYNTAX",
                ));
            }
            stderr_redirect = Some(StderrRedirect::ToStdout);
            i += 1;
            continue;
        }
        let target: String = if !attached.is_empty() {
            attached.to_string()
        } else {
            match args.get(i + 1) {
                Some(t) => t.as_str().to_string(),
                None => {
                    return Err(exec_usage_error(
                        interp,
                        format!("can't specify \"{}\" as last word in command", op),
                        "TCL OPERATION EXEC SYNTAX",
                    ));
                }
            }
        };
        let to = |stream: Stream, append: bool, path: &str| Redirect {
            stream,
            append,
            target: Target::File(path.to_string()),
        };
        match op {
            "<" => {
                redirects.push(to(Stream::In, false, &target));
                input_redirect = Some(InputRedirect::File(target));
            }
            "<<" => input_redirect = Some(InputRedirect::String(target)),
            ">" => {
                redirects.push(to(Stream::Out, false, &target));
                output_redirect = Some(OutputRedirect::Truncate(target));
            }
            ">>" => {
                redirects.push(to(Stream::Out, true, &target));
                output_redirect = Some(OutputRedirect::Append(target));
            }
            // `>& file` / `>>& file`: stderr goes to the same file.
            ">&" => {
                redirects.push(to(Stream::Out, false, &target));
                redirects.push(to(Stream::Err, false, &target));
                output_redirect = Some(OutputRedirect::Truncate(target.clone()));
                stderr_redirect = Some(StderrRedirect::Truncate(target));
            }
            ">>&" => {
                redirects.push(to(Stream::Out, true, &target));
                redirects.push(to(Stream::Err, true, &target));
                output_redirect = Some(OutputRedirect::Append(target.clone()));
                stderr_redirect = Some(StderrRedirect::Append(target));
            }
            "2>" => {
                redirects.push(to(Stream::Err, false, &target));
                stderr_redirect = Some(StderrRedirect::Truncate(target));
            }
            "2>>" => {
                redirects.push(to(Stream::Err, true, &target));
                stderr_redirect = Some(StderrRedirect::Append(target));
            }
            _ => unreachable!("every operator is handled or refused above"),
        }
        i += if attached.is_empty() { 2 } else { 1 };
    }

    if !current_stage.is_empty() {
        stages.push(current_stage);
    } else if !stages.is_empty() {
        // A pipeline that ends in `|`.
        return Err(exec_usage_error(
            interp,
            "illegal use of | or |& in command",
            "TCL OPERATION EXEC PIPESYNTAX",
        ));
    }

    if stages.is_empty() {
        // Redirections, but no command anywhere (tclsh: the same error
        // as a misplaced `|`).
        return Err(exec_usage_error(
            interp,
            "illegal use of | or |& in command",
            "TCL OPERATION EXEC PIPESYNTAX",
        ));
    }

    // Every redirection is opened before any process starts, in
    // command-line order: the first that fails is the error, and nothing
    // has run yet.
    let (mut input_file, mut output_file, mut stderr_file) = (None, None, None);
    // A redirection to a channel gives the child a duplicate of its
    // descriptor (`exec … >@ stdout`, `<@ $chan`).
    let (mut input_fd, mut output_fd, mut stderr_fd): (
        Option<std::os::fd::OwnedFd>,
        Option<std::os::fd::OwnedFd>,
        Option<std::os::fd::OwnedFd>,
    ) = (None, None, None);
    for r in &redirects {
        let writing = r.stream != Stream::In;
        match &r.target {
            Target::File(path) => {
                let opened = if !writing {
                    std::fs::File::open(path)
                } else if r.append {
                    std::fs::OpenOptions::new().append(true).create(true).open(path)
                } else {
                    std::fs::File::create(path)
                };
                let file = match opened {
                    Ok(f) => f,
                    Err(e) => {
                        return Err(super::io::posix_error(
                            interp,
                            format!(
                                "couldn't {} file \"{}\"",
                                if writing { "write" } else { "read" },
                                path
                            ),
                            &e,
                        ));
                    }
                };
                match r.stream {
                    Stream::In => input_file = Some(file),
                    Stream::Out => output_file = Some(file),
                    Stream::Err => stderr_file = Some(file),
                }
            }
            Target::Channel(name) => {
                let fd = interp.channels.fd_for_child(name, writing);
                let fd = match fd {
                    Ok(fd) => fd,
                    Err(e) => {
                        return Err(match e.kind() {
                            std::io::ErrorKind::NotFound => exec_usage_error(
                                interp,
                                e.to_string(),
                                &format!("TCL LOOKUP CHANNEL {}", name),
                            ),
                            std::io::ErrorKind::PermissionDenied => {
                                exec_usage_error(interp, e.to_string(), "TCL OPERATION EXEC BADCHAN")
                            }
                            _ => super::io::posix_error(
                                interp,
                                format!("couldn't duplicate the descriptor of channel \"{}\"", name),
                                &e,
                            ),
                        });
                    }
                };
                match r.stream {
                    Stream::In => input_fd = Some(fd),
                    Stream::Out => output_fd = Some(fd),
                    Stream::Err => stderr_fd = Some(fd),
                }
            }
        }
    }
    fn clone_file(interp: &mut Interp, f: &std::fs::File) -> Result<std::fs::File> {
        f.try_clone().map_err(|e| {
            super::io::posix_error(
                interp,
                "couldn't duplicate the file descriptor".to_string(),
                &e,
            )
        })
    }
    fn pipe(interp: &mut Interp) -> Result<(std::fs::File, std::fs::File)> {
        pipe_files()
            .map_err(|e| super::io::posix_error(interp, "couldn't create pipe".to_string(), &e))
    }

    // Build and execute the pipeline.
    let num_stages = stages.len();
    // The read end of the pipe the next stage reads from.
    let mut prev_stdout: Option<std::fs::File> = None;
    let mut children: Vec<std::process::Child> = Vec::new();
    // `2>@1` without a file: one pipe carries both streams of the last
    // stage, and this is its read end.
    let mut merged_stdout: Option<std::fs::File> = None;

    for (stage_idx, stage_args) in stages.iter().enumerate() {
        let is_last = stage_idx == num_stages - 1;
        let mut cmd = Command::new(stage_args[0]);
        cmd.args(&stage_args[1..]);

        // stdin: the caller's, unless redirected or fed by the previous
        // stage (tclsh leaves a first stage on the caller's stdin).
        if stage_idx == 0 {
            if let Some(f) = input_file.take() {
                cmd.stdin(f);
            } else if let Some(fd) = input_fd.take() {
                cmd.stdin(Stdio::from(fd));
            } else if matches!(input_redirect, Some(InputRedirect::String(_))) {
                // Fed by a helper thread once the child exists.
                cmd.stdin(Stdio::piped());
            }
        } else if let Some(read_end) = prev_stdout.take() {
            cmd.stdin(read_end);
        }

        // stdout: captured, redirected, passed on — or, for a background
        // job, the caller's own.
        if !is_last {
            let (read_end, write_end) = pipe(interp)?;
            let for_stderr = if merge_stderr_in_pipe {
                Some(clone_file(interp, &write_end)?)
            } else {
                None
            };
            cmd.stdout(write_end);
            if let Some(e) = for_stderr {
                cmd.stderr(e);
            }
            prev_stdout = Some(read_end);
        } else if let Some(f) = output_file.take() {
            let for_stderr = if matches!(stderr_redirect, Some(StderrRedirect::ToStdout)) {
                Some(clone_file(interp, &f)?)
            } else {
                None
            };
            cmd.stdout(f);
            if let Some(e) = for_stderr {
                cmd.stderr(e);
            }
        } else if let Some(fd) = output_fd.take() {
            cmd.stdout(Stdio::from(fd));
        } else if matches!(stderr_redirect, Some(StderrRedirect::ToStdout)) && !background {
            let (read_end, write_end) = pipe(interp)?;
            let for_stderr = clone_file(interp, &write_end)?;
            cmd.stdout(write_end);
            cmd.stderr(for_stderr);
            merged_stdout = Some(read_end);
        } else if !background {
            cmd.stdout(Stdio::piped());
        }

        // stderr: a file; the caller's for `-ignorestderr`, a background
        // job or a stage whose stderr is not joined; captured otherwise.
        if is_last && !matches!(stderr_redirect, Some(StderrRedirect::ToStdout)) {
            if let Some(f) = stderr_file.take() {
                cmd.stderr(f);
            } else if let Some(fd) = stderr_fd.take() {
                cmd.stderr(Stdio::from(fd));
            } else if !ignore_stderr && !background {
                cmd.stderr(Stdio::piped());
            }
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                // The stages already running are left to finish on their
                // own (tclsh detaches them); the next exec reaps them.
                detach_children(interp, &mut children);
                return Err(super::io::posix_error(
                    interp,
                    format!("couldn't execute \"{}\"", stage_args[0]),
                    &e,
                ));
            }
        };

        // `<<` input for the first stage.
        if stage_idx == 0 {
            if let Some(InputRedirect::String(s)) = &input_redirect {
                if let Some(stdin) = child.stdin.take() {
                    feed_stdin(stdin, s.as_bytes().to_vec());
                }
            }
        }

        children.push(child);
    }

    if background {
        // Return the PIDs; keep the handles so a later exec/update/Drop can
        // try_wait them (Tcl's detached-process reaping), instead of
        // leaking Child handles with mem::forget.
        let pids: Vec<String> = children.iter().map(|c| c.id().to_string()).collect();
        interp.background_children.extend(children);
        return Ok(Value::from_str(&pids.join(" ")));
    }

    // Both streams at once, until the last child closes them.
    let (data, err) = collect_output(interp, &mut children, merged_stdout)?;

    // Wait for the children and classify each completion, in tclsh's
    // order (TclCleanupChildren): a failing child's errorCode replaces
    // the one before it; a child killed by a signal leaves the message, a
    // non-zero exit only the fact that it happened.
    let mut failed = false;
    let mut abnormal_exit = false;
    let mut error_code: Option<String> = None;
    let mut reason = String::new();
    for child in children.iter_mut() {
        let pid = child.id();
        let status = child.wait().map_err(|e| {
            super::io::posix_error(
                interp,
                "error waiting for process to exit".to_string(),
                &e,
            )
        })?;
        match exit_kind(&status) {
            ExitKind::Ok => {}
            ExitKind::Code(code) => {
                failed = true;
                abnormal_exit = true;
                error_code = Some(format!("CHILDSTATUS {} {}", pid, code));
            }
            ExitKind::Signal(sig) => {
                failed = true;
                let (name, msg) = signal_text(sig);
                error_code = Some(
                    Value::from_list(&[
                        Value::from_str("CHILDKILLED"),
                        Value::from_str(&pid.to_string()),
                        Value::from_str(name),
                        Value::from_str(msg),
                    ])
                    .as_str()
                    .to_string(),
                );
                reason = format!("child killed: {}\n", msg);
            }
        }
    }

    // The result is the output with one trailing newline gone (unless
    // -keepnewline).  An error's message is the same text with the reason
    // after it — or the captured stderr, which is the report of its own:
    // a child that wrote to stderr failed the exec even when it exited 0.
    let mut text = data;
    if !err.is_empty() {
        failed = true;
        text.push_str(&err);
    } else if abnormal_exit {
        text.push_str("child process exited abnormally");
    } else {
        text.push_str(&reason);
    }
    if !keep_newline && text.ends_with('\n') {
        text.pop();
    }

    if failed {
        match error_code {
            Some(code) => super::list::set_error_code(interp, &code),
            None => super::list::set_error_code(interp, "NONE"),
        }
        return Err(Error::runtime(text, ErrorCode::Generic));
    }
    Ok(Value::from_str(&text))
}

/// Which of a child's streams a redirection names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    In,
    Out,
    Err,
}

// ── Redirect types ─────────────────────────────────────────────────────

enum InputRedirect {
    File(String),
    String(String),
    /// `<@ chan`: the channel's descriptor.
    Channel,
}

enum OutputRedirect {
    Truncate(String),
    Append(String),
    /// `>@ chan` / `>>@ chan`: the channel's descriptor.  The child
    /// writes where the channel would (`>>@` after what it holds — a
    /// duplicate shares the file's position; tclsh 8.6.17 loses the
    /// child's output on the `>>@` forms instead).
    Channel,
}

enum StderrRedirect {
    Truncate(String),
    Append(String),
    ToStdout,
    /// `2>@ chan`: the channel's descriptor.
    Channel,
}

/// What a redirection names: a file, or a channel the child gets the
/// descriptor of (`>@ chan`, `<@ chan`).
enum Target {
    File(String),
    Channel(String),
}

struct Redirect {
    stream: Stream,
    append: bool,
    target: Target,
}

/// The host-backed `exec` runs on the host's own descriptor table: an
/// interpreter channel (`exec … >@ chan`) has nothing there to give.
fn host_channel_redirect() -> Error {
    Error::runtime(
        "redirection to a channel is not supported by the host's exec",
        ErrorCode::InvalidOp,
    )
}

/// Host-backed `exec`：每个 stage 构建完整 fd 表；阶段间用 pipe 的子进程侧
/// 端点直连，最后一段的 stdout/stderr 由父进程侧端点捕获。
///
/// 0/1/2 只是 fd 号；重定向、管道、heredoc 全部走同一套 `HostFd`。
fn cmd_exec_host(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut i = 1;
    let mut ignore_stderr = false;
    let mut keep_newline = false;
    while i < args.len() {
        match args[i].as_str() {
            "-ignorestderr" => {
                ignore_stderr = true;
                i += 1;
            }
            "-keepnewline" => {
                keep_newline = true;
                i += 1;
            }
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') => {
                return Err(exec_err_str(format!(
                    "bad switch \"{s}\": must be -ignorestderr, -keepnewline, or --"
                )));
            }
            _ => break,
        }
    }

    let mut stages: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut merge_stderr_in_pipe = false;
    let mut input_redirect: Option<InputRedirect> = None;
    let mut output_redirect: Option<OutputRedirect> = None;
    let mut stderr_redirect: Option<StderrRedirect> = None;
    let mut background = false;

    while i < args.len() {
        let token = args[i].as_str();
        match token {
            "|" => {
                if current.is_empty() {
                    return Err(exec_err_str("illegal use of | or |& in command"));
                }
                stages.push(std::mem::take(&mut current));
                i += 1;
            }
            "|&" => {
                if current.is_empty() {
                    return Err(exec_err_str("illegal use of | or |& in command"));
                }
                stages.push(std::mem::take(&mut current));
                merge_stderr_in_pipe = true;
                i += 1;
            }
            "<" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str("can't specify \"<\" as last word in command"));
                }
                input_redirect = Some(InputRedirect::File(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "<<" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str("can't specify \"<<\" as last word in command"));
                }
                input_redirect = Some(InputRedirect::String(args[i + 1].as_str().to_string()));
                i += 2;
            }
            ">" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str("can't specify \">\" as last word in command"));
                }
                output_redirect = Some(OutputRedirect::Truncate(args[i + 1].as_str().to_string()));
                i += 2;
            }
            ">>" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str("can't specify \">>\" as last word in command"));
                }
                output_redirect = Some(OutputRedirect::Append(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "2>" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str("can't specify \"2>\" as last word in command"));
                }
                stderr_redirect = Some(StderrRedirect::Truncate(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "2>>" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str("can't specify \"2>>\" as last word in command"));
                }
                stderr_redirect = Some(StderrRedirect::Append(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "2>@1" => {
                stderr_redirect = Some(StderrRedirect::ToStdout);
                i += 1;
            }
            ">&" | ">>&" => {
                if i + 1 >= args.len() {
                    return Err(exec_err_str(format!(
                        "can't specify \"{token}\" as last word in command"
                    )));
                }
                let path = args[i + 1].as_str().to_string();
                if token == ">&" {
                    output_redirect = Some(OutputRedirect::Truncate(path.clone()));
                    stderr_redirect = Some(StderrRedirect::Truncate(path));
                } else {
                    output_redirect = Some(OutputRedirect::Append(path.clone()));
                    stderr_redirect = Some(StderrRedirect::Append(path));
                }
                i += 2;
            }
            "&" if i + 1 == args.len() => {
                background = true;
                i += 1;
            }
            _ => {
                current.push(token.to_string());
                i += 1;
            }
        }
    }
    if !current.is_empty() {
        stages.push(current);
    }
    if stages.is_empty() {
        return Err(exec_err_str("no command given"));
    }
    if background && interp.host_jobs().is_none() {
        return Err(exec_err_str("exec: host job table unavailable"));
    }

    let command_text = stages
        .iter()
        .map(|s| s.join(" "))
        .collect::<Vec<_>>()
        .join(" | ");
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    let interactive = {
        use std::io::IsTerminal;
        let i = std::io::stdin().is_terminal();
        let o = std::io::stdout().is_terminal();
        if std::env::var_os("ROGO_JOB_DEBUG").is_some() {
            eprintln!("exec-host: stdin_tty={i} stdout_tty={o}");
        }
        i && o
    };
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    let interactive = false;
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    let _sig_guard = if interactive {
        Some(tty::Guard::install())
    } else {
        None
    };

    let host = interp
        .host_exec()
        .ok_or_else(|| exec_err_str("exec: host exec unavailable"))?;
    let fs = interp.fd_fs();
    let n = stages.len();
    let mut stage_in: Option<Box<dyn HostFd>> = None;
    let mut handles: Vec<Box<dyn HostProcess>> = Vec::with_capacity(n);
    let mut out_r: Option<Box<dyn HostFd>> = None;
    let mut err_r: Option<Box<dyn HostFd>> = None;

    for (idx, stage) in stages.iter().enumerate() {
        let last = idx + 1 == n;
        let mut fds: Vec<Option<Box<dyn HostFd>>> = vec![None, None, None];

        // fd 0
        if idx == 0 {
            fds[0] = Some(match &input_redirect {
                Some(InputRedirect::File(p)) => host_open(fs, p, FdMode::Read)?,
                Some(InputRedirect::String(s)) => {
                    Box::new(BytesFd::new(s.as_bytes().to_vec()))
                }
                // The host's descriptors are its own: an interpreter
                // channel has no counterpart to hand over.
                Some(InputRedirect::Channel) => {
                    return Err(host_channel_redirect())
                }
                None => Box::new(NullFd),
            });
        } else {
            fds[0] = stage_in.take();
        }

        // fd 1 + link to next stage
        let stdout_child: Option<Box<dyn HostFd>> = if last {
            if let Some(r) = &output_redirect {
                let mode = match r {
                    OutputRedirect::Truncate(_) => FdMode::Write,
                    OutputRedirect::Append(_) => FdMode::Append,
                    OutputRedirect::Channel => return Err(host_channel_redirect()),
                };
                let path = match r {
                    OutputRedirect::Truncate(p) | OutputRedirect::Append(p) => p,
                    OutputRedirect::Channel => unreachable!("refused above"),
                };
                Some(host_open(fs, path, mode)?)
            } else {
                let (r, w) = host.pipe().map_err(exec_err_str)?;
                out_r = Some(r);
                Some(w)
            }
        } else {
            let (r, w) = host.pipe().map_err(exec_err_str)?;
            stage_in = Some(r);
            Some(w)
        };

        // `2>@1` 必须在 fd1 被移入表之前复制。
        let stdout_dup = if last && matches!(stderr_redirect, Some(StderrRedirect::ToStdout)) {
            stdout_child
                .as_ref()
                .map(|f| f.dup())
                .transpose()
                .map_err(exec_err_str)?
        } else {
            None
        };
        fds[1] = stdout_child;

        // fd 2
        if last {
            fds[2] = match &stderr_redirect {
                Some(StderrRedirect::Truncate(p)) => Some(host_open(fs, p, FdMode::Write)?),
                Some(StderrRedirect::Append(p)) => Some(host_open(fs, p, FdMode::Append)?),
                Some(StderrRedirect::ToStdout) => stdout_dup,
                Some(StderrRedirect::Channel) => return Err(host_channel_redirect()),
                None if ignore_stderr => Some(Box::new(NullFd)),
                None => {
                    let (r, w) = host.pipe().map_err(exec_err_str)?;
                    err_r = Some(r);
                    Some(w)
                }
            };
        } else if merge_stderr_in_pipe {
            fds[2] = fds[1]
                .as_ref()
                .map(|f| f.dup())
                .transpose()
                .map_err(exec_err_str)?;
        } else {
            fds[2] = Some(Box::new(NullFd));
        }

        let cmd = ExecCommand {
            prog: stage[0].clone(),
            argv: stage.clone(),
        };
        let p = host.spawn(&cmd, fds, &[]).map_err(exec_err_str)?;
        handles.push(p);
    }

    if background {
        // 后台 job：输出不阻塞调用方，交给 drain 线程；句柄进宿主作业表。
        if let Some(f) = out_r.take() {
            spawn_drain(f, false);
        }
        if let Some(f) = err_r.take() {
            spawn_drain(f, true);
        }
        let jobs = interp
            .host_jobs()
            .ok_or_else(|| exec_err_str("exec: host job table unavailable"))?;
        let id = jobs.add(handles, command_text.clone());
        return Ok(Value::from_str(&id.to_string()));
    }

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut out_eof = out_r.is_none();
    let mut err_eof = err_r.is_none();
    let mut done = vec![false; handles.len()];
    let mut any_error = false;
    let mut exit_code = 0i32;

    loop {
        #[cfg(all(unix, not(target_arch = "wasm32")))]
        if interactive {
            if tty::INT.swap(false, std::sync::atomic::Ordering::SeqCst) {
                for h in &mut handles {
                    let _ = h.kill();
                }
            }
            if tty::TSTP.swap(false, std::sync::atomic::Ordering::SeqCst) {
                if std::env::var_os("ROGO_JOB_DEBUG").is_some() {
                    eprintln!("exec-host: tstp requested");
                }
                for h in &mut handles {
                    let _ = h.stop();
                }
                // 最多等 500ms 的 checkpoint ack；纯计算保持 stopping。
                let start = Instant::now();
                loop {
                    let all_stopped = handles.iter_mut().all(|h| {
                        matches!(
                            h.state(),
                            Ok(JobState::Stopped) | Ok(JobState::Exited(_)) | Ok(JobState::Killed)
                        )
                    });
                    if all_stopped || start.elapsed() >= Duration::from_millis(500) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                match job_prompt(&mut handles, &command_text) {
                    PromptAction::Fg => {}
                    PromptAction::Bg => {
                        if let Some(f) = out_r.take() {
                            spawn_drain(f, false);
                        }
                        if let Some(f) = err_r.take() {
                            spawn_drain(f, true);
                        }
                        let jobs = interp
                            .host_jobs()
                            .ok_or_else(|| exec_err_str("exec: host job table unavailable"))?;
                        let id = jobs.add(handles, command_text.clone());
                        return Ok(Value::from_str(&id.to_string()));
                    }
                    PromptAction::Kill | PromptAction::Eof => {
                        for h in &mut handles {
                            let _ = h.kill();
                        }
                    }
                }
            }
        }
        if let Some(f) = out_r.as_mut() {
            if !out_eof {
                out_eof = read_host_fd(f.as_mut(), &mut out)?;
            }
        }
        if let Some(f) = err_r.as_mut() {
            if !err_eof {
                err_eof = read_host_fd(f.as_mut(), &mut err)?;
            }
        }
        let mut all = true;
        for (idx, p) in handles.iter_mut().enumerate() {
            if done[idx] {
                continue;
            }
            if let Some(code) = p.wait().map_err(exec_err_str)? {
                done[idx] = true;
                if code != 0 {
                    any_error = true;
                    exit_code = code;
                }
            } else {
                all = false;
            }
        }
        if all {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    // 收尾排空管道剩余字节。
    if let Some(f) = out_r.as_mut() {
        while !out_eof {
            out_eof = read_host_fd(f.as_mut(), &mut out)?;
        }
    }
    if let Some(f) = err_r.as_mut() {
        while !err_eof {
            err_eof = read_host_fd(f.as_mut(), &mut err)?;
        }
    }

    let mut stdout = String::from_utf8_lossy(&out).to_string();
    if !keep_newline && stdout.ends_with('\n') {
        stdout.pop();
        if stdout.ends_with('\r') {
            stdout.pop();
        }
    }
    let stderr = String::from_utf8_lossy(&err).to_string();

    if any_error {
        let last_pid = handles.last().map(|p| p.id()).unwrap_or(0);
        let error_code = format!("CHILDSTATUS {last_pid} {exit_code}");
        let _ = interp.set_var("::errorCode", Value::from_str(&error_code));
        interp.err_code_raised = true;
        let mut msg = stderr.clone();
        if msg.is_empty() {
            msg = format!("child process exited abnormally (exit code {exit_code})");
        }
        if msg.ends_with('\n') {
            msg.pop();
        }
        return Err(Error::runtime(
            format!("{stdout}{msg}"),
            ErrorCode::Generic,
        ));
    }

    let _ = interp.set_var("::errorCode", Value::from_str("NONE"));
    if !stderr.is_empty() && !ignore_stderr {
        eprint!("{stderr}");
    }
    Ok(Value::from_str(&stdout))
}

fn host_open(fs: Option<&dyn HostFdFs>, path: &str, mode: FdMode) -> Result<Box<dyn HostFd>> {
    let fs = fs.ok_or_else(|| exec_err_str("exec: host filesystem unavailable"))?;
    fs.open(path, mode).map_err(exec_err_str)
}

fn spawn_drain(mut f: Box<dyn HostFd>, stderr: bool) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 64 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stderr {
                        use std::io::Write;
                        let _ = std::io::stderr().write_all(&buf[..n]);
                    } else {
                        use std::io::Write;
                        let _ = std::io::stdout().write_all(&buf[..n]);
                    }
                }
                Err(ExecIoError::WouldBlock) => {
                    let _ = f.poll(true, false, 10);
                }
                Err(_) => break,
            }
        }
    });
}

fn read_host_fd(f: &mut dyn HostFd, out: &mut Vec<u8>) -> Result<bool> {
    let mut buf = [0u8; 64 * 1024];
    loop {
        match f.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(ExecIoError::WouldBlock) => return Ok(false),
            Err(ExecIoError::BrokenPipe) => return Ok(true),
            Err(ExecIoError::Other(e)) => return Err(exec_err_str(e)),
        }
    }
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
enum PromptAction {
    Fg,
    Bg,
    Kill,
    Eof,
}

/// 前台 Ctrl-Z 的受限 job prompt：只接受 fg/bg/jobs/kill/help，不跑 Tcl。
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn job_prompt(handles: &mut [Box<dyn HostProcess>], command: &str) -> PromptAction {
    use std::io::Write;
    loop {
        if tty::INT.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return PromptAction::Kill;
        }
        // Ctrl-Z 在 job prompt 内无意义。
        let _ = tty::TSTP.swap(false, std::sync::atomic::Ordering::SeqCst);

        let id = handles.last().map(|h| h.id()).unwrap_or(0);
        let state = handles
            .iter_mut()
            .filter_map(|h| h.state().ok())
            .next_back()
            .unwrap_or(JobState::Running);
        println!("[{id}]+ {} {command}", state.label());
        print!("job% ");
        let _ = std::io::stdout().flush();

        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => return PromptAction::Eof,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                if tty::INT.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    return PromptAction::Kill;
                }
                continue;
            }
            Err(_) => return PromptAction::Eof,
        }
        match line.trim() {
            "" | "fg" => {
                for h in handles.iter_mut() {
                    let _ = h.resume();
                }
                return PromptAction::Fg;
            }
            "bg" => {
                for h in handles.iter_mut() {
                    let _ = h.resume();
                }
                return PromptAction::Bg;
            }
            "jobs" | "list" => {
                for h in handles.iter_mut() {
                    let st = h.state().unwrap_or(JobState::Running);
                    println!("{} {} {command}", h.id(), st.label());
                }
            }
            "kill" => return PromptAction::Kill,
            "help" => println!("job prompt: fg | bg | jobs | kill | help"),
            other => println!("job prompt: unknown command \"{other}\" (fg|bg|jobs|kill|help)"),
        }
    }
}

/// `job list|stop|resume|fg|bg|kill|wait ?job-id?`
pub fn cmd_job(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let sub = args
        .get(1)
        .map(|v| v.as_str().to_string())
        .ok_or_else(|| exec_err_str("usage: job list|stop|resume|fg|bg|kill|wait ?job-id?"))?;
    let jobs = interp
        .host_jobs()
        .ok_or_else(|| exec_err_str("job: host job table unavailable"))?;
    match sub.as_str() {
        "list" => {
            let list = jobs.list();
            let text = list
                .iter()
                .map(|j| format!("{} {} {}", j.id, j.state.label(), j.command))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Value::from_str(&text))
        }
        "stop" => {
            jobs.stop(job_id(args)?).map_err(exec_err_str)?;
            Ok(Value::empty())
        }
        "resume" => {
            jobs.resume(job_id(args)?).map_err(exec_err_str)?;
            Ok(Value::empty())
        }
        "kill" => {
            jobs.kill(job_id(args)?).map_err(exec_err_str)?;
            Ok(Value::empty())
        }
        "bg" => {
            jobs.resume(job_id(args)?).map_err(exec_err_str)?;
            Ok(Value::empty())
        }
        "fg" | "wait" => {
            let id = job_id(args)?;
            if sub == "fg" {
                jobs.resume(id).map_err(exec_err_str)?;
            }
            loop {
                if let Some(code) = jobs.wait(id).map_err(exec_err_str)? {
                    return Ok(Value::from_str(&code.to_string()));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        other => Err(exec_err_str(format!(
            "job: unknown subcommand \"{other}\" (list|stop|resume|fg|bg|kill|wait)"
        ))),
    }
}

fn job_id(args: &[Value]) -> Result<u64> {
    let raw = args
        .get(2)
        .map(|v| v.as_str())
        .ok_or_else(|| exec_err_str("job: missing job-id"))?;
    raw.trim_start_matches('%')
        .parse::<u64>()
        .map_err(|_| exec_err_str(format!("job: bad job-id \"{raw}\"")))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(all(test, feature = "exec"))]
mod tests {
    use crate::interp::Interp;

    // ── basic exec tests ───────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_echo() {
        let mut interp = Interp::new();
        let result = interp.eval("exec echo hello").unwrap();
        assert_eq!(result.as_str().trim(), "hello");
    }

    #[cfg(windows)]
    #[test]
    fn test_exec_echo() {
        let mut interp = Interp::new();
        // On Windows, use cmd /c echo
        let result = interp.eval("exec cmd /c echo hello").unwrap();
        assert_eq!(result.as_str().trim(), "hello");
    }

    // ── exec 2>@1 redirect tests ───────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_stderr_to_stdout() {
        let mut interp = Interp::new();
        // Use 2>@1 to merge stderr into stdout
        // sh -c "echo out; echo err >&2" writes "out" to stdout and "err" to stderr
        let result = interp.eval("exec sh -c {echo out; echo err >&2} 2>@1").unwrap();
        let output = result.as_str();
        assert!(output.contains("out"));
        assert!(output.contains("err"));
    }

    // ── exec errorCode tests ───────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_errorcode_on_success() {
        let mut interp = Interp::new();
        // A command that succeeds leaves ::errorCode as it found it.
        interp.set_var("::errorCode", crate::value::Value::from_str("SENTINEL")).unwrap();
        interp.eval("exec true").unwrap();
        let ec = interp.get_var("::errorCode").unwrap();
        assert_eq!(ec.as_str(), "SENTINEL");
    }

    #[cfg(unix)]
    #[test]
    fn test_exec_errorcode_on_failure() {
        let mut interp = Interp::new();
        // false exits with code 1
        let _ = interp.eval("exec false");
        let ec = interp.get_var("::errorCode").unwrap();
        // Should be something like "CHILDSTATUS <pid> 1"
        assert!(ec.as_str().starts_with("CHILDSTATUS"));
        assert!(ec.as_str().ends_with("1"));
    }

    // ── exec -keepnewline tests ────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_keepnewline() {
        let mut interp = Interp::new();
        let result = interp.eval("exec -keepnewline echo hello").unwrap();
        assert!(result.as_str().ends_with('\n'));
    }

    #[cfg(unix)]
    #[test]
    fn test_exec_no_keepnewline() {
        let mut interp = Interp::new();
        let result = interp.eval("exec echo hello").unwrap();
        assert!(!result.as_str().ends_with('\n'));
    }

    // ── exec -ignorestderr tests ───────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_ignorestderr() {
        let mut interp = Interp::new();
        // With -ignorestderr, stderr goes to the terminal, not captured
        let result = interp.eval("exec -ignorestderr sh -c {echo out; echo err >&2}");
        // Should succeed (no error for stderr output)
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_str().trim(), "out");
    }

    // ── exec -- end-of-switches tests ──────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_double_dash() {
        let mut interp = Interp::new();
        // -- signals end of switches, allowing command names starting with -
        let result = interp.eval("exec -- echo hello").unwrap();
        assert_eq!(result.as_str().trim(), "hello");
    }

    // ── exec bad switch test ───────────────────────────────────────────────────

    #[test]
    fn test_exec_bad_switch() {
        let mut interp = Interp::new();
        // tclsh refuses an unknown switch with its index-lookup error.
        let r = interp
            .eval("catch {exec -badswitch echo hello} m; list $m $::errorCode")
            .unwrap();
        assert_eq!(
            r.as_str(),
            "{bad option \"-badswitch\": must be -ignorestderr, -keepnewline, or --} {TCL LOOKUP INDEX option -badswitch}"
        );
    }

    // ── exec pipe tests ────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_pipe() {
        let mut interp = Interp::new();
        let result = interp.eval("exec echo {hello world} | tr a-z A-Z").unwrap();
        assert_eq!(result.as_str().trim(), "HELLO WORLD");
    }

    // ── exec input redirect << tests ───────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_input_string() {
        let mut interp = Interp::new();
        let result = interp.eval("exec cat << {hello from string}").unwrap();
        assert_eq!(result.as_str(), "hello from string");
    }

    // ── exec output redirect > tests ───────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_output_redirect_file() {
        let mut interp = Interp::new();
        let path = std::env::temp_dir().join(format!("rtcl_exec_redir_{}", std::process::id()));
        interp.set_var("_f", crate::value::Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("exec echo {redirected output} > $_f").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("redirected output"));
        let _ = std::fs::remove_file(&path);
    }

    // ── exec 2> stderr redirect tests ──────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_stderr_redirect_file() {
        let mut interp = Interp::new();
        let path = std::env::temp_dir().join(format!("rtcl_exec_stderr_{}", std::process::id()));
        interp.set_var("_f", crate::value::Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("exec sh -c {echo err >&2} 2> $_f").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("err"));
        let _ = std::fs::remove_file(&path);
    }

    // ── exec errorCode pid extraction ──────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_exec_errorcode_has_pid() {
        let mut interp = Interp::new();
        let _ = interp.eval("exec sh -c {exit 42}");
        let ec = interp.get_var("::errorCode").unwrap();
        let parts: Vec<&str> = ec.as_str().split_whitespace().collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "CHILDSTATUS");
        // parts[1] should be a numeric pid
        let pid: u32 = parts[1].parse().unwrap();
        assert!(pid > 0);
        assert_eq!(parts[2], "42");
    }

    // ── exec no command test ───────────────────────────────────────────────────

    #[test]
    fn test_exec_no_args() {
        let mut interp = Interp::new();
        let result = interp.eval("exec");
        assert!(result.is_err());
    }

    // ── exec nonexistent command ───────────────────────────────────────────────

    #[test]
    fn test_exec_nonexistent_command() {
        let mut interp = Interp::new();
        let result = interp.eval("exec this_command_does_not_exist_xyz123");
        assert!(result.is_err());
    }
}
