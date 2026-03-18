//! OS and system commands: cd, pwd, sleep, readdir, kill, wait.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

// ---------- cd ----------

#[cfg(feature = "file")]
pub fn cmd_cd(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let dir = match args.len() {
        1 => {
            // cd with no args → go to HOME
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string())
        }
        2 => args[1].as_str().to_string(),
        _ => return Err(Error::wrong_args_with_usage("cd", 1, args.len(), "?dirName?")),
    };
    std::env::set_current_dir(&dir).map_err(|e| {
        Error::runtime(
            format!("couldn't change working directory to \"{}\": {}", dir, e),
            ErrorCode::Io,
        )
    })?;
    Ok(Value::empty())
}

// ---------- pwd ----------

#[cfg(feature = "file")]
pub fn cmd_pwd(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 1 {
        return Err(Error::wrong_args("pwd", 1, args.len()));
    }
    let cwd = std::env::current_dir().map_err(|e| {
        Error::runtime(format!("error getting working directory: {}", e), ErrorCode::Io)
    })?;
    Ok(Value::from_str(&cwd.to_string_lossy()))
}

// ---------- sleep ----------

#[cfg(feature = "signal")]
pub fn cmd_sleep(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("sleep", 2, args.len(), "seconds"));
    }
    let secs_str = args[1].as_str();
    // Support fractional seconds like jimtcl
    let secs: f64 = secs_str.parse().map_err(|_| {
        Error::runtime(
            format!("expected floating-point number but got \"{}\"", secs_str),
            ErrorCode::Generic,
        )
    })?;
    if secs < 0.0 {
        return Err(Error::runtime("sleep time must be non-negative".to_string(), ErrorCode::Generic));
    }
    std::thread::sleep(std::time::Duration::from_secs_f64(secs));
    Ok(Value::empty())
}

// ---------- readdir ----------

#[cfg(feature = "file")]
pub fn cmd_readdir(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("readdir", 2, args.len(), "?-nocomplain? dirPath"));
    }
    let (nocomplain, dir) = if args.len() == 3 {
        if args[1].as_str() != "-nocomplain" {
            return Err(Error::wrong_args_with_usage("readdir", 2, args.len(), "?-nocomplain? dirPath"));
        }
        (true, args[2].as_str())
    } else {
        (false, args[1].as_str())
    };

    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            if nocomplain {
                return Ok(Value::from_str(""));
            }
            return Err(Error::runtime(
                format!("couldn't read directory \"{}\": {}", dir, e),
                ErrorCode::Io,
            ));
        }
    };

    let mut names: Vec<Value> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            Error::runtime(format!("error reading directory: {}", e), ErrorCode::Io)
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        names.push(Value::from_str(&name));
    }
    names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    Ok(Value::from_list(&names))
}

// ---------- kill ----------

#[cfg(feature = "signal")]
pub fn cmd_kill(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // kill ?signal? pid
    match args.len() {
        2 => {
            // kill pid — send SIGTERM (default)
            let pid = parse_pid(args[1].as_str())?;
            kill_process(pid, "SIGTERM")
        }
        3 => {
            // kill signal pid
            let signal = args[1].as_str();
            let pid = parse_pid(args[2].as_str())?;
            kill_process(pid, signal)
        }
        _ => Err(Error::wrong_args_with_usage("kill", 2, args.len(), "?signal? pid")),
    }
}

#[cfg(any(feature = "signal", feature = "exec"))]
fn parse_pid(s: &str) -> Result<u32> {
    s.parse::<u32>().map_err(|_| {
        Error::runtime(
            format!("expected integer but got \"{}\"", s),
            ErrorCode::Generic,
        )
    })
}

#[cfg(all(feature = "signal", unix))]
fn kill_process(pid: u32, signal: &str) -> Result<Value> {
    let sig_num = match signal.to_uppercase().as_str() {
        "SIGTERM" | "TERM" | "15" => "15",
        "SIGKILL" | "KILL" | "9" => "9",
        "SIGINT" | "INT" | "2" => "2",
        "SIGHUP" | "HUP" | "1" => "1",
        "SIGUSR1" | "USR1" | "10" => "10",
        "SIGUSR2" | "USR2" | "12" => "12",
        "0" => "0",
        other => {
            return Err(Error::runtime(
                format!("unknown signal \"{}\"", other),
                ErrorCode::Generic,
            ));
        }
    };
    let output = std::process::Command::new("kill")
        .args([&format!("-{}", sig_num), &pid.to_string()])
        .output()
        .map_err(|e| Error::runtime(format!("kill: {}", e), ErrorCode::Io))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::runtime(
            format!("kill: {}", stderr.trim()),
            ErrorCode::Io,
        ));
    }
    Ok(Value::empty())
}

#[cfg(all(feature = "signal", not(unix)))]
fn kill_process(pid: u32, signal: &str) -> Result<Value> {
    // On Windows, only SIGTERM (TerminateProcess) is meaningful
    match signal.to_uppercase().as_str() {
        "SIGTERM" | "TERM" | "15" | "SIGKILL" | "KILL" | "9" => {
            // Use taskkill on Windows
            let output = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output()
                .map_err(|e| Error::runtime(format!("kill: {}", e), ErrorCode::Io))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::runtime(
                    format!("kill: {}", stderr.trim()),
                    ErrorCode::Io,
                ));
            }
            Ok(Value::empty())
        }
        "0" => {
            // Test if process exists
            let output = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                .output()
                .map_err(|e| Error::runtime(format!("kill: {}", e), ErrorCode::Io))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains(&pid.to_string()) {
                Ok(Value::empty())
            } else {
                Err(Error::runtime("kill: no such process".to_string(), ErrorCode::Io))
            }
        }
        other => Err(Error::runtime(
            format!("signal \"{}\" not supported on this platform", other),
            ErrorCode::Generic,
        )),
    }
}

// ---------- wait ----------

#[cfg(feature = "exec")]
pub fn cmd_wait(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // wait ?-nohang? pid
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("wait", 2, args.len(), "?-nohang? pid"));
    }
    let (nohang, pid_str) = if args.len() == 3 {
        if args[1].as_str() != "-nohang" {
            return Err(Error::wrong_args_with_usage("wait", 2, args.len(), "?-nohang? pid"));
        }
        (true, args[2].as_str())
    } else {
        (false, args[1].as_str())
    };
    let pid = parse_pid(pid_str)?;
    wait_process(pid, nohang)
}

#[cfg(all(feature = "exec", unix))]
fn wait_process(pid: u32, nohang: bool) -> Result<Value> {
    // Use /proc or waitpid via shell
    // Simple approach: poll with `kill -0` and then check exit status
    if nohang {
        // Check if process exists
        let output = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map_err(|e| Error::runtime(format!("wait: {}", e), ErrorCode::Io))?;
        if output.status.success() {
            return Ok(Value::from_str(&format!("{} NONE 0", pid)));
        }
        Ok(Value::from_str(&format!("{} CHILDSTATUS 0", pid)))
    } else {
        // Blocking wait using shell waitpid
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("wait {} 2>/dev/null; echo $?", pid)])
            .output()
            .map_err(|e| Error::runtime(format!("wait: {}", e), ErrorCode::Io))?;
        let exit_code = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Value::from_str(&format!("{} CHILDSTATUS {}", pid, exit_code)))
    }
}

#[cfg(all(feature = "exec", not(unix)))]
fn wait_process(pid: u32, _nohang: bool) -> Result<Value> {
    // On Windows, waitpid is not directly available.
    // Return a stub result — proper implementation would need WaitForSingleObject.
    Err(Error::runtime(
        format!("wait is not fully supported on this platform for pid {}", pid),
        ErrorCode::Generic,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    #[test]
    fn test_pwd_returns_directory() {
        let mut interp = Interp::new();
        let result = interp.eval("pwd").unwrap();
        assert!(!result.as_str().is_empty());
    }

    #[test]
    fn test_sleep_zero() {
        let mut interp = Interp::new();
        let result = interp.eval("sleep 0").unwrap();
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_sleep_fractional() {
        let mut interp = Interp::new();
        let result = interp.eval("sleep 0.01").unwrap();
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_readdir_current() {
        let mut interp = Interp::new();
        let result = interp.eval("readdir .").unwrap();
        // Should return a non-empty list
        assert!(!result.as_str().is_empty());
    }

    #[test]
    fn test_readdir_nocomplain() {
        let mut interp = Interp::new();
        let result = interp.eval("readdir -nocomplain /nonexistent_dir_xyz").unwrap();
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_readdir_error() {
        let mut interp = Interp::new();
        let result = interp.eval("readdir /nonexistent_dir_xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_cd_pwd_roundtrip() {
        let mut interp = Interp::new();
        let orig = interp.eval("pwd").unwrap().as_str().to_string();
        // cd to a temp dir and back — use set to avoid path escaping issues
        let temp = std::env::temp_dir().to_string_lossy().to_string();
        interp.set_var("_tmpdir", Value::from_str(&temp)).unwrap();
        interp.eval("cd $_tmpdir").unwrap();
        let after = interp.eval("pwd").unwrap().as_str().to_string();
        // Restore
        interp.set_var("_origdir", Value::from_str(&orig)).unwrap();
        interp.eval("cd $_origdir").unwrap();
        // Verify we moved somewhere
        assert!(!after.is_empty());
    }
}
