//! Channel I/O commands: open, close, read, gets, seek, tell, eof, flush, fconfigure, pid.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

fn io_err(msg: std::io::Error) -> Error {
    Error::runtime(msg.to_string(), crate::error::ErrorCode::Io)
}

fn io_err_str(msg: impl std::fmt::Display) -> Error {
    Error::runtime(msg.to_string(), crate::error::ErrorCode::Io)
}

// ---------- open ----------

pub fn cmd_open(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "open", 2, args.len(),
            "open fileName ?access?",
        ));
    }
    let path = args[1].as_str();
    let mode = if args.len() >= 3 { args[2].as_str() } else { "r" };

    // Pipe channel: open |command ?mode?
    if let Some(cmd_str) = path.strip_prefix('|') {
        return open_pipe(interp, cmd_str.trim(), mode);
    }

    let id = interp.channels.open_file(path, mode).map_err(|e| {
        io_err_str(format!("couldn't open \"{}\": {}", path, e))
    })?;
    Ok(Value::from_str(&id))
}

fn open_pipe(interp: &mut Interp, cmd_str: &str, mode: &str) -> Result<Value> {
    let parts: Vec<&str> = cmd_str.split_whitespace().collect();
    if parts.is_empty() {
        return Err(io_err_str("empty pipe command"));
    }

    let mut child_cmd = std::process::Command::new(parts[0]);
    child_cmd.args(&parts[1..]);

    match mode {
        "r" => {
            child_cmd.stdout(std::process::Stdio::piped());
        }
        "w" => {
            child_cmd.stdin(std::process::Stdio::piped());
        }
        "r+" | "w+" => {
            child_cmd.stdin(std::process::Stdio::piped());
            child_cmd.stdout(std::process::Stdio::piped());
        }
        _ => {
            child_cmd.stdout(std::process::Stdio::piped());
        }
    }

    let child = child_cmd.spawn().map_err(|e| {
        io_err_str(format!("couldn't execute \"{}\": {}", cmd_str, e))
    })?;

    let pipe = crate::channel::PipeChannel::new(child);
    let pid = pipe.pid();
    let id = format!("file{}", pid);
    interp.channels.register(id.clone(), Box::new(pipe));
    Ok(Value::from_str(&id))
}

// ---------- close ----------

pub fn cmd_close(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("close", 2, args.len(), "close channelId"));
    }
    let id = args[1].as_str();
    interp.channels.close(id).map_err(io_err)?;
    Ok(Value::empty())
}

// ---------- read ----------

pub fn cmd_read(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("read", 2, args.len(), "read channelId ?numBytes?"));
    }

    let mut i = 1;
    let nonewline = if args[i].as_str() == "-nonewline" {
        i += 1;
        true
    } else {
        false
    };

    if i >= args.len() {
        return Err(Error::wrong_args_with_usage("read", 2, args.len(), "read ?-nonewline? channelId ?numBytes?"));
    }

    let chan_id = args[i].as_str();
    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;

    if i + 1 < args.len() {
        // read channelId numBytes
        let count = args[i + 1].as_int().ok_or_else(|| {
            Error::runtime(format!("expected integer but got \"{}\"", args[i + 1].as_str()), crate::error::ErrorCode::Generic)
        })? as usize;
        let s = crate::channel::channel_read_chars(ch.as_mut(), count).map_err(io_err)?;
        Ok(Value::from_str(&s))
    } else {
        // read channelId  (read all)
        let mut s = ch.read_all().map_err(io_err)?;
        if nonewline && s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') { s.pop(); }
        }
        Ok(Value::from_str(&s))
    }
}

// ---------- gets ----------

pub fn cmd_gets(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage("gets", 2, args.len(), "gets channelId ?varName?"));
    }
    let chan_id = args[1].as_str();

    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;

    let line = ch.read_line().map_err(io_err)?;

    if args.len() == 3 {
        // gets channelId varName → store line in var, return byte count
        let var_name = args[2].as_str();
        match line {
            Some(ref s) => {
                let len = s.len() as i64;
                interp.set_var(var_name, Value::from_str(s))?;
                Ok(Value::from_int(len))
            }
            None => {
                interp.set_var(var_name, Value::from_str(""))?;
                Ok(Value::from_int(-1))
            }
        }
    } else {
        // gets channelId → return the line
        Ok(Value::from_str(line.as_deref().unwrap_or("")))
    }
}

// ---------- seek ----------

pub fn cmd_seek(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage("seek", 3, args.len(), "seek channelId offset ?origin?"));
    }
    let chan_id = args[1].as_str();
    let offset = args[2].as_int().ok_or_else(|| {
        Error::runtime(format!("expected integer but got \"{}\"", args[2].as_str()), crate::error::ErrorCode::Generic)
    })?;
    let origin = if args.len() == 4 {
        match args[3].as_str() {
            "start" => std::io::SeekFrom::Start(offset as u64),
            "current" => std::io::SeekFrom::Current(offset),
            "end" => std::io::SeekFrom::End(offset),
            other => return Err(Error::runtime(
                format!("bad origin \"{}\": must be start, current, or end", other),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    } else {
        std::io::SeekFrom::Start(offset as u64)
    };

    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;
    ch.seek(origin).map_err(io_err)?;
    Ok(Value::empty())
}

// ---------- tell ----------

pub fn cmd_tell(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("tell", 2, args.len(), "tell channelId"));
    }
    let chan_id = args[1].as_str();
    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;
    let pos = ch.tell().map_err(io_err)?;
    Ok(Value::from_int(pos as i64))
}

// ---------- eof ----------

pub fn cmd_eof(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("eof", 2, args.len(), "eof channelId"));
    }
    let chan_id = args[1].as_str();
    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;
    Ok(Value::from_bool(ch.eof()))
}

// ---------- flush ----------

pub fn cmd_flush(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("flush", 2, args.len(), "flush channelId"));
    }
    let chan_id = args[1].as_str();
    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;
    ch.flush().map_err(io_err)?;
    Ok(Value::empty())
}

// ---------- fconfigure ----------

pub fn cmd_fconfigure(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("fconfigure", 2, args.len(), "fconfigure channelId ?optName? ?value? ..."));
    }
    let chan_id = args[1].as_str();

    // Verify the channel exists
    if !interp.channels.contains(chan_id) {
        return Err(io_err_str(format!("can not find channel named \"{}\"", chan_id)));
    }

    if args.len() == 2 {
        // Query all options
        let cfg = interp.channels.config(chan_id).cloned().unwrap_or_default();
        let translation = match cfg.translation {
            crate::channel::TranslationMode::Auto => "auto",
            crate::channel::TranslationMode::Lf => "lf",
            crate::channel::TranslationMode::CrLf => "crlf",
            crate::channel::TranslationMode::Cr => "cr",
            crate::channel::TranslationMode::Binary => "binary",
        };
        let buffering = match cfg.buffering {
            crate::channel::Buffering::Full => "full",
            crate::channel::Buffering::Line => "line",
            crate::channel::Buffering::None => "none",
        };
        let blocking = if cfg.blocking { "1" } else { "0" };
        let result = format!(
            "-blocking {} -buffering {} -buffersize {} -encoding {} -translation {}",
            blocking, buffering, cfg.buffer_size, cfg.encoding, translation,
        );
        return Ok(Value::from_str(&result));
    }

    if args.len() == 3 {
        // Query a single option
        let opt = args[2].as_str();
        let cfg = interp.channels.config(chan_id).cloned().unwrap_or_default();
        let val = match opt {
            "-blocking" => if cfg.blocking { "1" } else { "0" }.to_string(),
            "-buffering" => match cfg.buffering {
                crate::channel::Buffering::Full => "full",
                crate::channel::Buffering::Line => "line",
                crate::channel::Buffering::None => "none",
            }.to_string(),
            "-buffersize" => cfg.buffer_size.to_string(),
            "-encoding" => cfg.encoding.clone(),
            "-translation" => match cfg.translation {
                crate::channel::TranslationMode::Auto => "auto",
                crate::channel::TranslationMode::Lf => "lf",
                crate::channel::TranslationMode::CrLf => "crlf",
                crate::channel::TranslationMode::Cr => "cr",
                crate::channel::TranslationMode::Binary => "binary",
            }.to_string(),
            _ => return Err(Error::runtime(
                format!("bad option \"{}\": should be -blocking, -buffering, -buffersize, -encoding, or -translation", opt),
                crate::error::ErrorCode::InvalidOp,
            )),
        };
        return Ok(Value::from_str(&val));
    }

    // Set options: pairs of -option value
    let mut i = 2;
    while i + 1 < args.len() {
        let opt = args[i].as_str();
        let val = args[i + 1].as_str();
        let cfg = interp.channels.config_mut(chan_id)
            .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;
        match opt {
            "-blocking" => {
                cfg.blocking = val == "1" || val == "true" || val == "yes";
            }
            "-buffering" => {
                cfg.buffering = match val {
                    "full" => crate::channel::Buffering::Full,
                    "line" => crate::channel::Buffering::Line,
                    "none" => crate::channel::Buffering::None,
                    _ => return Err(Error::runtime(
                        "bad value for -buffering: must be one of full, line, or none".to_string(),
                        crate::error::ErrorCode::InvalidOp,
                    )),
                };
            }
            "-buffersize" => {
                let size: usize = val.parse().map_err(|_| Error::runtime(
                    format!("expected integer but got \"{}\"", val),
                    crate::error::ErrorCode::Generic,
                ))?;
                cfg.buffer_size = size;
            }
            "-encoding" => {
                cfg.encoding = val.to_string();
            }
            "-translation" => {
                cfg.translation = match val {
                    "auto" => crate::channel::TranslationMode::Auto,
                    "lf" => crate::channel::TranslationMode::Lf,
                    "crlf" => crate::channel::TranslationMode::CrLf,
                    "cr" => crate::channel::TranslationMode::Cr,
                    "binary" => crate::channel::TranslationMode::Binary,
                    _ => return Err(Error::runtime(
                        "bad value for -translation: must be one of auto, lf, crlf, cr, or binary".to_string(),
                        crate::error::ErrorCode::InvalidOp,
                    )),
                };
            }
            _ => return Err(Error::runtime(
                format!("bad option \"{}\": should be -blocking, -buffering, -buffersize, -encoding, or -translation", opt),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
        i += 2;
    }

    Ok(Value::empty())
}

// ---------- pid ----------

pub fn cmd_pid(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() == 1 {
        // pid — return current process ID
        Ok(Value::from_int(std::process::id() as i64))
    } else if args.len() == 2 {
        // pid channelId — return PID(s) of pipe channel as a list
        let chan_id = args[1].as_str();
        let ch = interp.channels.get_mut(chan_id)
            .ok_or_else(|| io_err_str(format!("can not find channel named \"{}\"", chan_id)))?;
        let pids = ch.pids();
        if pids.is_empty() {
            Ok(Value::empty())
        } else {
            let pid_strs: Vec<Value> = pids.iter().map(|p| Value::from_int(*p as i64)).collect();
            Ok(Value::from_list(&pid_strs))
        }
    } else {
        Err(Error::wrong_args_with_usage("pid", 1, args.len(), "pid ?channelId?"))
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(all(test, feature = "io"))]
mod tests {
    use crate::interp::Interp;
    use crate::value::Value;

    // ── pid tests ──────────────────────────────────────────────────────────────

    #[test]
    fn test_pid_no_args_returns_current() {
        let mut interp = Interp::new();
        let result = interp.eval("pid").unwrap();
        let pid: u32 = result.as_str().parse().unwrap();
        assert_eq!(pid, std::process::id());
    }

    #[test]
    fn test_pid_stdout_returns_empty() {
        let mut interp = Interp::new();
        // stdout is not a pipe channel, so pid should return empty
        let result = interp.eval("pid stdout").unwrap();
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_pid_stdin_returns_empty() {
        let mut interp = Interp::new();
        let result = interp.eval("pid stdin").unwrap();
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_pid_nonexistent_channel() {
        let mut interp = Interp::new();
        let result = interp.eval("pid nosuch");
        assert!(result.is_err());
    }

    #[test]
    fn test_pid_too_many_args() {
        let mut interp = Interp::new();
        let result = interp.eval("pid a b");
        assert!(result.is_err());
    }

    // ── fconfigure tests ───────────────────────────────────────────────────────

    #[test]
    fn test_fconfigure_query_all() {
        let mut interp = Interp::new();
        let result = interp.eval("fconfigure stdout").unwrap();
        let s = result.as_str();
        assert!(s.contains("-blocking"));
        assert!(s.contains("-buffering"));
        assert!(s.contains("-buffersize"));
        assert!(s.contains("-encoding"));
        assert!(s.contains("-translation"));
    }

    #[test]
    fn test_fconfigure_query_single() {
        let mut interp = Interp::new();
        let result = interp.eval("fconfigure stdout -buffering").unwrap();
        let val = result.as_str();
        assert!(val == "full" || val == "line" || val == "none");
    }

    #[test]
    fn test_fconfigure_set_buffering() {
        let mut interp = Interp::new();
        interp.eval("fconfigure stdout -buffering none").unwrap();
        let result = interp.eval("fconfigure stdout -buffering").unwrap();
        assert_eq!(result.as_str(), "none");
        // Restore
        interp.eval("fconfigure stdout -buffering line").unwrap();
    }

    #[test]
    fn test_fconfigure_set_translation() {
        let mut interp = Interp::new();
        interp.eval("fconfigure stdout -translation lf").unwrap();
        let result = interp.eval("fconfigure stdout -translation").unwrap();
        assert_eq!(result.as_str(), "lf");
    }

    #[test]
    fn test_fconfigure_set_encoding() {
        let mut interp = Interp::new();
        interp.eval("fconfigure stdout -encoding utf-8").unwrap();
        let result = interp.eval("fconfigure stdout -encoding").unwrap();
        assert_eq!(result.as_str(), "utf-8");
    }

    #[test]
    fn test_fconfigure_set_blocking() {
        let mut interp = Interp::new();
        interp.eval("fconfigure stdout -blocking 0").unwrap();
        let result = interp.eval("fconfigure stdout -blocking").unwrap();
        assert_eq!(result.as_str(), "0");
        // Restore
        interp.eval("fconfigure stdout -blocking 1").unwrap();
    }

    #[test]
    fn test_fconfigure_bad_option() {
        let mut interp = Interp::new();
        let result = interp.eval("fconfigure stdout -nosuchoption");
        assert!(result.is_err());
    }

    #[test]
    fn test_fconfigure_bad_channel() {
        let mut interp = Interp::new();
        let result = interp.eval("fconfigure nosuch");
        assert!(result.is_err());
    }

    #[test]
    fn test_fconfigure_bad_buffering_value() {
        let mut interp = Interp::new();
        let result = interp.eval("fconfigure stdout -buffering invalid");
        assert!(result.is_err());
    }

    // ── open / close / eof tests ───────────────────────────────────────────────

    #[test]
    fn test_open_read_close() {
        let mut interp = Interp::new();
        let path = std::env::temp_dir().join(format!("rtcl_chanio_{}", std::process::id()));
        std::fs::write(&path, b"hello\nworld\n").unwrap();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("set f [open $_p r]").unwrap();
        let line = interp.eval("gets $f").unwrap();
        assert_eq!(line.as_str(), "hello");
        let line2 = interp.eval("gets $f").unwrap();
        assert_eq!(line2.as_str(), "world");
        interp.eval("close $f").unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_open_write_close() {
        let mut interp = Interp::new();
        let path = std::env::temp_dir().join(format!("rtcl_chanio_w_{}", std::process::id()));
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("set f [open $_p w]").unwrap();
        interp.eval("puts $f {written by test}").unwrap();
        interp.eval("close $f").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("written by test"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_eof_on_file() {
        let mut interp = Interp::new();
        let path = std::env::temp_dir().join(format!("rtcl_chanio_eof_{}", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("set f [open $_p r]").unwrap();
        // Read all content
        interp.eval("read $f").unwrap();
        let eof = interp.eval("eof $f").unwrap();
        assert_eq!(eof.as_str(), "1");
        interp.eval("close $f").unwrap();
        let _ = std::fs::remove_file(&path);
    }

    // ── seek / tell tests ──────────────────────────────────────────────────────

    #[test]
    fn test_seek_tell() {
        let mut interp = Interp::new();
        let path = std::env::temp_dir().join(format!("rtcl_chanio_seek_{}", std::process::id()));
        std::fs::write(&path, b"abcdef").unwrap();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("set f [open $_p r]").unwrap();
        interp.eval("seek $f 3").unwrap();
        let pos = interp.eval("tell $f").unwrap();
        assert_eq!(pos.as_str(), "3");
        interp.eval("close $f").unwrap();
        let _ = std::fs::remove_file(&path);
    }

    // ── flush on stdout ────────────────────────────────────────────────────────

    #[test]
    fn test_flush_stdout() {
        let mut interp = Interp::new();
        // flush stdout should not error
        interp.eval("flush stdout").unwrap();
    }
}
