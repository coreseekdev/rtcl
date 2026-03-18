//! exec command — subprocess execution with output capture.
//!
//! Tcl `exec` runs one or more subprocesses with I/O redirections and
//! returns the collected standard output.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

use std::io::Read;
use std::process::{Command, Stdio};

fn exec_err(msg: std::io::Error) -> Error {
    Error::runtime(msg.to_string(), ErrorCode::Io)
}

fn exec_err_str(msg: impl std::fmt::Display) -> Error {
    Error::runtime(msg.to_string(), ErrorCode::Io)
}

/// `exec ?-ignorestderr? ?-keepnewline? ?--? cmd ?arg ...? ?| cmd ...? ?redirections?`
pub fn cmd_exec(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "exec", 2, args.len(), "exec ?switches? arg ?arg ...?",
        ));
    }

    // Parse switches
    let mut i = 1;
    let mut ignore_stderr = false;
    let mut keep_newline = false;
    while i < args.len() {
        match args[i].as_str() {
            "-ignorestderr" => { ignore_stderr = true; i += 1; }
            "-keepnewline"  => { keep_newline = true;  i += 1; }
            "--"            => { i += 1; break; }
            s if s.starts_with('-') => {
                return Err(exec_err_str(format!("bad switch \"{}\": must be -ignorestderr, -keepnewline, or --", s)));
            }
            _ => break,
        }
    }

    if i >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "exec", 2, args.len(), "exec ?switches? arg ?arg ...?",
        ));
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

    while i < args.len() {
        let token = args[i].as_str();
        match token {
            "|" => {
                if current_stage.is_empty() {
                    return Err(exec_err_str("illegal use of | or |& in command"));
                }
                stages.push(std::mem::take(&mut current_stage));
                i += 1;
            }
            "|&" => {
                if current_stage.is_empty() {
                    return Err(exec_err_str("illegal use of | or |& in command"));
                }
                stages.push(std::mem::take(&mut current_stage));
                merge_stderr_in_pipe = true;
                i += 1;
            }
            "<" => {
                if i + 1 >= args.len() { return Err(exec_err_str("can't specify \"<\" as last word in command")); }
                input_redirect = Some(InputRedirect::File(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "<<" => {
                if i + 1 >= args.len() { return Err(exec_err_str("can't specify \"<<\" as last word in command")); }
                input_redirect = Some(InputRedirect::String(args[i + 1].as_str().to_string()));
                i += 2;
            }
            ">" => {
                if i + 1 >= args.len() { return Err(exec_err_str("can't specify \">\" as last word in command")); }
                output_redirect = Some(OutputRedirect::Truncate(args[i + 1].as_str().to_string()));
                i += 2;
            }
            ">>" => {
                if i + 1 >= args.len() { return Err(exec_err_str("can't specify \">>\" as last word in command")); }
                output_redirect = Some(OutputRedirect::Append(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "2>" => {
                if i + 1 >= args.len() { return Err(exec_err_str("can't specify \"2>\" as last word in command")); }
                stderr_redirect = Some(StderrRedirect::Truncate(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "2>>" => {
                if i + 1 >= args.len() { return Err(exec_err_str("can't specify \"2>>\" as last word in command")); }
                stderr_redirect = Some(StderrRedirect::Append(args[i + 1].as_str().to_string()));
                i += 2;
            }
            "2>@1" => {
                // Redirect stderr to stdout (merge)
                stderr_redirect = Some(StderrRedirect::ToStdout);
                i += 1;
            }
            ">&" | ">>&" => {
                if i + 1 >= args.len() { return Err(exec_err_str(format!("can't specify \"{}\" as last word in command", token))); }
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
                current_stage.push(token);
                i += 1;
            }
        }
    }

    if !current_stage.is_empty() {
        stages.push(current_stage);
    }

    if stages.is_empty() {
        return Err(exec_err_str("no command given"));
    }

    // Build and execute the pipeline.
    let num_stages = stages.len();
    let mut prev_stdout: Option<std::process::ChildStdout> = None;
    let mut children: Vec<std::process::Child> = Vec::new();

    for (stage_idx, stage_args) in stages.iter().enumerate() {
        if stage_args.is_empty() {
            return Err(exec_err_str("no command given"));
        }

        let mut cmd = Command::new(stage_args[0]);
        cmd.args(&stage_args[1..]);

        // stdin
        if stage_idx == 0 {
            match &input_redirect {
                Some(InputRedirect::File(path)) => {
                    let f = std::fs::File::open(path).map_err(|e| {
                        exec_err_str(format!("couldn't read file \"{}\": {}", path, e))
                    })?;
                    cmd.stdin(f);
                }
                Some(InputRedirect::String(s)) => {
                    // We'll need to pipe the string in via a spawned child's stdin
                    // For now, use piped stdin and write to it below
                    cmd.stdin(Stdio::piped());
                    // Save the string — handled below
                    let _ = s; // used after spawn
                }
                None => {
                    cmd.stdin(Stdio::null());
                }
            }
        } else if let Some(stdout) = prev_stdout.take() {
            cmd.stdin(stdout);
        }

        // stdout
        let is_last = stage_idx == num_stages - 1;
        if is_last && output_redirect.is_none() && !background {
            cmd.stdout(Stdio::piped());
        } else if is_last {
            match &output_redirect {
                Some(OutputRedirect::Truncate(path)) => {
                    let f = std::fs::File::create(path).map_err(|e| {
                        exec_err_str(format!("couldn't write file \"{}\": {}", path, e))
                    })?;
                    cmd.stdout(f);
                }
                Some(OutputRedirect::Append(path)) => {
                    let f = std::fs::OpenOptions::new().append(true).create(true).open(path).map_err(|e| {
                        exec_err_str(format!("couldn't write file \"{}\": {}", path, e))
                    })?;
                    cmd.stdout(f);
                }
                None => {
                    // background — discard
                    cmd.stdout(Stdio::null());
                }
            }
        } else {
            cmd.stdout(Stdio::piped());
        }

        // stderr
        if is_last {
            match &stderr_redirect {
                Some(StderrRedirect::Truncate(path)) => {
                    let f = std::fs::File::create(path).map_err(|e| {
                        exec_err_str(format!("couldn't write file \"{}\": {}", path, e))
                    })?;
                    cmd.stderr(f);
                }
                Some(StderrRedirect::Append(path)) => {
                    let f = std::fs::OpenOptions::new().append(true).create(true).open(path).map_err(|e| {
                        exec_err_str(format!("couldn't write file \"{}\": {}", path, e))
                    })?;
                    cmd.stderr(f);
                }
                Some(StderrRedirect::ToStdout) => {
                    // 2>@1 — merge stderr into stdout pipe
                    cmd.stderr(Stdio::piped()); // We'll read it separately and merge
                }
                None if ignore_stderr => {
                    cmd.stderr(Stdio::null());
                }
                None => {
                    cmd.stderr(Stdio::piped());
                }
            }
        } else if merge_stderr_in_pipe {
            // |& merges stderr into the pipe stdout
            cmd.stderr(Stdio::piped());
        } else {
            cmd.stderr(Stdio::inherit());
        }

        let mut child = cmd.spawn().map_err(|e| {
            exec_err_str(format!("couldn't execute \"{}\": {}", stage_args[0], e))
        })?;

        // Handle <<string input for first stage
        if stage_idx == 0 {
            if let Some(InputRedirect::String(ref s)) = input_redirect {
                if let Some(mut stdin) = child.stdin.take() {
                    use std::io::Write;
                    let _ = stdin.write_all(s.as_bytes());
                    // Drop stdin to close the pipe so the child can read EOF
                }
            }
        }

        // Capture stdout for piping to next stage
        if !is_last {
            prev_stdout = child.stdout.take();
        }

        children.push(child);
    }

    if background {
        // Return the PIDs of all spawned processes
        let pids: Vec<String> = children.iter().map(|c| c.id().to_string()).collect();
        // Don't wait — the children run in the background
        // We intentionally leak them (they'll be cleaned up on exit)
        for child in children {
            std::mem::forget(child);
        }
        return Ok(Value::from_str(&pids.join(" ")));
    }

    // Collect output from the last child
    let last = children.last_mut().unwrap();
    let last_pid = last.id();
    let mut stdout_data = String::new();
    if let Some(ref mut stdout) = last.stdout {
        stdout.read_to_string(&mut stdout_data).map_err(exec_err)?;
    }

    let mut stderr_data = String::new();
    if let Some(ref mut stderr) = last.stderr {
        stderr.read_to_string(&mut stderr_data).map_err(exec_err)?;
    }

    // 2>@1: merge stderr into stdout
    if matches!(stderr_redirect, Some(StderrRedirect::ToStdout)) {
        stdout_data.push_str(&stderr_data);
        stderr_data.clear();
    }

    // Wait for all children
    let mut any_error = false;
    let mut exit_code = 0i32;
    for child in children.iter_mut() {
        match child.wait() {
            Ok(status) => {
                if !status.success() {
                    any_error = true;
                    exit_code = status.code().unwrap_or(1);
                }
            }
            Err(e) => {
                return Err(exec_err_str(format!("error waiting for process: {}", e)));
            }
        }
    }

    // Strip trailing newline (unless -keepnewline)
    if !keep_newline && stdout_data.ends_with('\n') {
        stdout_data.pop();
        if stdout_data.ends_with('\r') {
            stdout_data.pop();
        }
    }

    if any_error {
        // Set $::errorCode like jimtcl: {CHILDSTATUS pid exitCode}
        let error_code = format!("CHILDSTATUS {} {}", last_pid, exit_code);
        let _ = interp.set_var("::errorCode", Value::from_str(&error_code));

        let mut msg = stderr_data.clone();
        if msg.is_empty() {
            msg = format!("child process exited abnormally (exit code {})", exit_code);
        }
        if msg.ends_with('\n') { msg.pop(); }
        return Err(Error::runtime(
            format!("{}{}", stdout_data, if msg.is_empty() { String::new() } else { msg }),
            ErrorCode::Generic,
        ));
    }

    // Set $::errorCode to NONE on success
    let _ = interp.set_var("::errorCode", Value::from_str("NONE"));

    if !stderr_data.is_empty() && !ignore_stderr {
        // Tcl convention: non-zero stderr with zero exit = emit to stderr but still succeed
        eprint!("{}", stderr_data);
    }

    Ok(Value::from_str(&stdout_data))
}

// ── Redirect types ─────────────────────────────────────────────────────

enum InputRedirect {
    File(String),
    String(String),
}

enum OutputRedirect {
    Truncate(String),
    Append(String),
}

enum StderrRedirect {
    Truncate(String),
    Append(String),
    ToStdout,
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
        interp.eval("exec true").unwrap();
        let ec = interp.eval("$::errorCode").unwrap();
        assert_eq!(ec.as_str(), "NONE");
    }

    #[cfg(unix)]
    #[test]
    fn test_exec_errorcode_on_failure() {
        let mut interp = Interp::new();
        // false exits with code 1
        let _ = interp.eval("exec false");
        let ec = interp.eval("$::errorCode").unwrap();
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
        let result = interp.eval("exec -badswitch echo hello");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("bad switch"));
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
        let ec = interp.eval("$::errorCode").unwrap();
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
