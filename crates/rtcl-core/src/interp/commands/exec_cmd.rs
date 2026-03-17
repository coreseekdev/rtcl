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
pub fn cmd_exec(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
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
    let mut stdout_data = String::new();
    if let Some(ref mut stdout) = last.stdout {
        stdout.read_to_string(&mut stdout_data).map_err(exec_err)?;
    }

    let mut stderr_data = String::new();
    if let Some(ref mut stderr) = last.stderr {
        stderr.read_to_string(&mut stderr_data).map_err(exec_err)?;
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
}
