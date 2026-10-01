//! Channel I/O commands: open, close, read, gets, seek, tell, eof, flush,
//! fconfigure, fblocked, pid, and the `chan` ensemble.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

fn io_err(msg: std::io::Error) -> Error {
    Error::runtime(msg.to_string(), crate::error::ErrorCode::Io)
}

fn io_err_str(msg: impl std::fmt::Display) -> Error {
    Error::runtime(msg.to_string(), crate::error::ErrorCode::Io)
}

/// Tcl's `can not find channel named "x"` failure of `TclGetChannelFromObj`,
/// which also installs `TCL LOOKUP CHANNEL x` as ::errorCode.
pub(crate) fn chan_not_found(interp: &mut Interp, id: &str) -> Error {
    super::list::set_error_code(
        interp,
        &format!("TCL LOOKUP CHANNEL {}", crate::value::tcl_quote(id)),
    );
    Error::runtime(
        format!("can not find channel named \"{}\"", id),
        crate::error::ErrorCode::Io,
    )
}

/// Tcl's direction mismatch failure: the channel exists but is not open
/// for the requested access. No ::errorCode (tclsh leaves it at NONE).
pub(crate) fn not_opened(id: &str, reading: bool) -> Error {
    Error::runtime(
        format!(
            "channel \"{}\" wasn't opened for {}",
            id,
            if reading { "reading" } else { "writing" }
        ),
        crate::error::ErrorCode::Io,
    )
}

/// Resolve an option/method/subcommand name against `table` using Tcl's
/// unique-prefix matching (exact matches win). `Err(())` = unknown or
/// ambiguous.
fn resolve_prefix(table: &[&'static str], s: &str) -> std::result::Result<&'static str, ()> {
    for t in table {
        if *t == s {
            return Ok(t);
        }
    }
    let mut found: Option<&'static str> = None;
    for t in table {
        if t.starts_with(s) {
            if found.is_some() {
                return Err(());
            }
            found = Some(t);
        }
    }
    found.ok_or(())
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
    // Reflected/transform handlers run their finalize/drain methods first;
    // a missing channel raises with TCL LOOKUP CHANNEL.
    if !interp.channels.contains(id) {
        return Err(chan_not_found(interp, id));
    }
    close_reflected(interp, id)?;
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

    // `read channelId numBytes`
    let count: Option<i64> = if i + 1 < args.len() {
        let v = args[i + 1].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected non-negative integer but got \"{}\"", args[i + 1].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })?;
        if v < 0 {
            return Err(Error::runtime(
                format!("expected non-negative integer but got \"{}\"", args[i + 1].as_str()),
                crate::error::ErrorCode::Generic,
            ));
        }
        Some(v)
    } else {
        None
    };

    // Reflected channels (chan create) read through their handler script.
    if let Some(data) = reflected_read(interp, chan_id, count)? {
        let mut s = data;
        if nonewline && s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') { s.pop(); }
        }
        return Ok(Value::from_str(&s));
    }

    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    // Channel transforms (chan push) filter bytes through their handler.
    let transform = interp.transforms.get(chan_id).cloned();
    if let Some(t) = &transform {
        if t.methods.iter().any(|m| m == "read") {
            let want = count.unwrap_or(4096).max(0) as usize;
            let raw = {
                let ch = interp.channels.get_mut(chan_id).unwrap();
                if want == 0 {
                    String::new()
                } else {
                    crate::channel::channel_read_chars(ch.as_mut(), want).map_err(io_err)?
                }
            };
            let mut data = transform_through_read(interp, t, &raw)?;
            if nonewline && data.ends_with('\n') {
                data.pop();
                if data.ends_with('\r') {
                    data.pop();
                }
            }
            return Ok(Value::from_str(&data));
        }
    }

    let ch = interp.channels.get_mut(chan_id).unwrap();

    if !ch.is_readable() {
        return Err(not_opened(chan_id, true));
    }

    if let Some(count) = count {
        let s = crate::channel::channel_read_chars(ch.as_mut(), count as usize).map_err(io_err)?;
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

    // Reflected channels (chan create) read through their handler script.
    if let Some(data) = reflected_read(interp, chan_id, None)? {
        // A handler read produces whatever the handler returns; a whole
        // "line" read takes everything available.
        if let Some(line) = data.lines().next() {
            let line = line.to_string();
            return finish_gets(interp, args, Some(&line));
        }
        return finish_gets(interp, args, None);
    }

    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    // Channel transforms (chan push) filter bytes through their handler.
    let transform = interp.transforms.get(chan_id).cloned();
    if let Some(t) = &transform {
        if t.methods.iter().any(|m| m == "read") {
            let line = {
                let ch = interp.channels.get_mut(chan_id).unwrap();
                ch.read_line().map_err(io_err)?
            };
            let line = match line {
                Some(s) => Some(transform_through_read(interp, t, &s)?),
                None => None,
            };
            return finish_gets(interp, args, line.as_deref());
        }
    }

    let ch = interp.channels.get_mut(chan_id).unwrap();

    if !ch.is_readable() {
        return Err(not_opened(chan_id, true));
    }

    let line = ch.read_line().map_err(io_err)?;
    finish_gets(interp, args, line.as_deref())
}

/// Shared tail of `gets`: assign the result variable and build the return.
fn finish_gets(interp: &mut Interp, args: &[Value], line: Option<&str>) -> Result<Value> {
    if args.len() == 3 {
        let var_name = args[2].as_str();
        match line {
            Some(s) => {
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
        Ok(Value::from_str(line.unwrap_or("")))
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
    let (origin, negative_start) = if args.len() == 4 {
        match args[3].as_str() {
            "start" => (std::io::SeekFrom::Start(offset.max(0) as u64), offset < 0),
            "current" => (std::io::SeekFrom::Current(offset), false),
            "end" => (std::io::SeekFrom::End(offset), false),
            other => return Err(Error::runtime(
                format!("bad origin \"{}\": must be start, current, or end", other),
                crate::error::ErrorCode::InvalidOp,
            )),
        }
    } else {
        (std::io::SeekFrom::Start(offset.max(0) as u64), offset < 0)
    };

    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let ch = interp.channels.get_mut(chan_id).unwrap();
    if negative_start {
        // TclSeek: a negative offset from "start" is EINVAL.
        return Err(Error::runtime(
            format!("error during seek on \"{}\": invalid argument", chan_id),
            crate::error::ErrorCode::InvalidOp,
        ));
    }
    ch.seek(origin).map_err(|e| Error::runtime(
        format!("error during seek on \"{}\": {}", chan_id, e),
        crate::error::ErrorCode::InvalidOp,
    ))?;
    Ok(Value::empty())
}

// ---------- tell ----------

pub fn cmd_tell(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("tell", 2, args.len(), "tell channelId"));
    }
    let chan_id = args[1].as_str();
    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let ch = interp.channels.get_mut(chan_id).unwrap();
    let pos = ch.tell().map_err(io_err)?;
    Ok(Value::from_int(pos as i64))
}

// ---------- eof ----------

pub fn cmd_eof(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("eof", 2, args.len(), "eof channelId"));
    }
    let chan_id = args[1].as_str();
    // `chan create` channels report EOF through their read handler.
    if let Some(r) = interp.reflected.get(chan_id) {
        return Ok(Value::from_bool(r.at_eof));
    }
    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let ch = interp.channels.get_mut(chan_id).unwrap();
    Ok(Value::from_bool(ch.eof()))
}

// ---------- flush ----------

pub fn cmd_flush(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("flush", 2, args.len(), "flush channelId"));
    }
    let chan_id = args[1].as_str();
    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let ch = interp.channels.get_mut(chan_id).unwrap();
    if !ch.is_writable() {
        return Err(not_opened(chan_id, false));
    }
    ch.flush().map_err(io_err)?;
    Ok(Value::empty())
}

// ---------- fblocked ----------

pub fn cmd_fblocked(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("fblocked", 2, args.len(), "fblocked channelId"));
    }
    let chan_id = args[1].as_str();
    if interp.reflected.contains_key(chan_id) {
        // Handler reads are synchronous — never blocked.
        return Ok(Value::from_bool(false));
    }
    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let ch = interp.channels.get_mut(chan_id).unwrap();
    if !ch.is_readable() {
        return Err(not_opened(chan_id, true));
    }
    // Reads are synchronous/blocking, so a channel is only "blocked" while
    // a non-blocking read left data pending — never the case here.
    Ok(Value::from_bool(false))
}

// ---------- fconfigure ----------

/// The `-option` set fconfigure accepts, in Tcl's report order.
const FCONFIGURE_OPTIONS: &[&str] = &[
    "-blocking",
    "-buffering",
    "-buffersize",
    "-encoding",
    "-eofchar",
    "-translation",
];

/// Render an `-eofchar` query the way Tcl does: single-access channels
/// report a one-element list, read/write channels a two-element list.
fn eofchar_query(ch_read: bool, ch_write: bool, in_c: Option<char>, out_c: Option<char>) -> Value {
    let v = |c: Option<char>| match c {
        Some(c) => Value::from_str(&c.to_string()),
        None => Value::from_str(""),
    };
    if ch_read && ch_write {
        Value::from_list(&[v(in_c), v(out_c)])
    } else if ch_read {
        v(in_c)
    } else {
        v(out_c)
    }
}

/// Validate one `-eofchar` element: Tcl allows a single non-NUL ASCII char.
fn eofchar_element(val: &str) -> std::result::Result<Option<char>, Error> {
    let mut chars = val.chars();
    match (chars.next(), chars.next()) {
        (None, _) => Ok(None),
        (Some(c), None) if c != '\0' && (c as u32) <= 0x7f => Ok(Some(c)),
        _ => Err(Error::runtime(
            "bad value for -eofchar: must be non-NUL ASCII character",
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

pub fn cmd_fconfigure(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("fconfigure", 2, args.len(), "fconfigure channelId ?optName? ?value? ..."));
    }
    let chan_id = args[1].as_str();

    // Verify the channel exists
    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let (ch_read, ch_write) = {
        let ch = interp.channels.get(chan_id).unwrap();
        (ch.is_readable(), ch.is_writable())
    };

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
            "-blocking {} -buffering {} -buffersize {} -encoding {} -eofchar {} -translation {}",
            blocking, buffering, cfg.buffer_size, cfg.encoding,
            eofchar_query(ch_read, ch_write, cfg.eofchar_in, cfg.eofchar_out).as_str(),
            translation,
        );
        return Ok(Value::from_str(&result));
    }

    if args.len() == 3 {
        // Query a single option
        let opt = match resolve_prefix(FCONFIGURE_OPTIONS, args[2].as_str()) {
            Ok(o) => o,
            Err(()) => {
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": should be one of -blocking, -buffering, -buffersize, -encoding, -eofchar, or -translation",
                        args[2].as_str()
                    ),
                    crate::error::ErrorCode::InvalidOp,
                ))
            }
        };
        let cfg = interp.channels.config(chan_id).cloned().unwrap_or_default();
        let val = match opt {
            "-blocking" => Value::from_str(if cfg.blocking { "1" } else { "0" }),
            "-buffering" => Value::from_str(match cfg.buffering {
                crate::channel::Buffering::Full => "full",
                crate::channel::Buffering::Line => "line",
                crate::channel::Buffering::None => "none",
            }),
            "-buffersize" => Value::from_int(cfg.buffer_size as i64),
            "-encoding" => Value::from_str(&cfg.encoding),
            "-eofchar" => eofchar_query(ch_read, ch_write, cfg.eofchar_in, cfg.eofchar_out),
            "-translation" => Value::from_str(match cfg.translation {
                crate::channel::TranslationMode::Auto => "auto",
                crate::channel::TranslationMode::Lf => "lf",
                crate::channel::TranslationMode::CrLf => "crlf",
                crate::channel::TranslationMode::Cr => "cr",
                crate::channel::TranslationMode::Binary => "binary",
            }),
            _ => unreachable!(),
        };
        return Ok(val);
    }

    // Set options: pairs of -option value. A trailing option without a
    // value is a wrong-arg-count error (tclsh's fconfigureCmd).
    if (args.len() - 2) % 2 != 0 {
        return Err(Error::wrong_args_with_usage(
            "fconfigure", 2, args.len(),
            "fconfigure channelId ?-option value ...?",
        ));
    }

    let mut i = 2;
    while i + 1 < args.len() {
        let opt = match resolve_prefix(FCONFIGURE_OPTIONS, args[i].as_str()) {
            Ok(o) => o,
            Err(()) => {
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": should be one of -blocking, -buffering, -buffersize, -encoding, -eofchar, or -translation",
                        args[i].as_str()
                    ),
                    crate::error::ErrorCode::InvalidOp,
                ))
            }
        };
        let val = args[i + 1].as_str();
        if !interp.channels.contains(chan_id) {
                return Err(chan_not_found(interp, chan_id));
            }
let cfg = interp.channels.config_mut(chan_id).unwrap();
        match opt {
            "-blocking" => {
                cfg.blocking = match Value::from_str(val).as_bool() {
                    Some(b) => b,
                    None => {
                        return Err(Error::runtime(
                            format!("expected boolean value but got \"{}\"", val),
                            crate::error::ErrorCode::Generic,
                        ))
                    }
                };
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
                let size: i64 = val.parse().map_err(|_| Error::runtime(
                    format!("expected integer but got \"{}\"", val),
                    crate::error::ErrorCode::Generic,
                ))?;
                cfg.buffer_size = size.max(0) as usize;
            }
            "-encoding" => {
                cfg.encoding = val.to_string();
            }
            "-eofchar" => {
                // Either a single character (both directions) or a
                // two-element list {readChar writeChar}. An empty value
                // (or list of empties) clears the setting.
                let elems = match Value::from_str(val).as_list_strict() {
                    Ok(e) => e,
                    Err(e) => {
                        super::list::set_error_code(interp, e.code);
                        return Err(Error::runtime(
                            e.message.clone(),
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                };
                let (in_c, out_c) = match elems.len() {
                    0 => (None, None),
                    1 => {
                        let c = eofchar_element(elems[0].as_str())?;
                        (c, c)
                    }
                    2 => (
                        eofchar_element(elems[0].as_str())?,
                        eofchar_element(elems[1].as_str())?,
                    ),
                    _ => {
                        return Err(Error::runtime(
                            "bad value for -eofchar: should be a list of zero, one, or two elements",
                            crate::error::ErrorCode::InvalidOp,
                        ))
                    }
                };
                cfg.eofchar_in = in_c;
                cfg.eofchar_out = out_c;
            }
            "-translation" => {
                cfg.translation = match val {
                    "auto" | "platform" => {
                        if cfg!(windows) {
                            crate::channel::TranslationMode::CrLf
                        } else {
                            crate::channel::TranslationMode::Lf
                        }
                    }
                    "lf" => crate::channel::TranslationMode::Lf,
                    "crlf" => crate::channel::TranslationMode::CrLf,
                    "cr" => crate::channel::TranslationMode::Cr,
                    "binary" => crate::channel::TranslationMode::Binary,
                    _ => return Err(Error::runtime(
                        "bad value for -translation: must be one of auto, binary, cr, lf, crlf, or platform".to_string(),
                        crate::error::ErrorCode::InvalidOp,
                    )),
                };
            }
            _ => unreachable!(),
        }
        i += 2;
    }

    Ok(Value::empty())
}

// ---------- chan ensemble ----------

/// The `chan` subcommand set, in Tcl's report order.
const CHAN_SUBCOMMANDS: &[&str] = &[
    "blocked", "close", "configure", "copy", "create", "eof", "event", "flush",
    "gets", "names", "pending", "pipe", "pop", "postevent", "push", "puts",
    "read", "seek", "tell", "truncate",
];

/// Methods a `chan create` handler may advertise.
const REFLECTED_METHODS: &[&str] = &[
    "blocking", "cget", "cgetall", "configure", "finalize", "initialize",
    "read", "seek", "watch", "write",
];

/// Methods a `chan push` handler may advertise.
const TRANSFORM_METHODS: &[&str] = &[
    "clear", "drain", "finalize", "flush", "initialize", "limit?", "read", "write",
];

/// A reflected channel registered by `chan create`: the handler script is
/// invoked for the channel operations it advertises.
#[derive(Debug, Clone)]
pub struct ReflectedChannel {
    /// Command prefix (as given to `chan create`).
    pub prefix: Vec<String>,
    /// This channel's handle (`rc0`, `rc1`, ...).
    pub handle: String,
    /// Access modes: a subset of `read` / `write`.
    pub modes: Vec<String>,
    /// Methods the handler advertised (canonical names).
    pub methods: Vec<String>,
    /// Set once a handler read returned no data.
    pub at_eof: bool,
}

/// A transform pushed onto a channel with `chan push`.
#[derive(Debug, Clone)]
pub struct ChannelTransform {
    /// Command prefix (as given to `chan push`).
    pub prefix: Vec<String>,
    /// The transform's own handle (`rt0`, `rt1`, ...).
    pub handle: String,
    /// Methods the handler advertised (canonical names).
    pub methods: Vec<String>,
}

/// Build a script string that invokes `prefix` with `args`, quoting each
/// word as a list element would be.
fn handler_script(prefix: &[String], args: &[&str]) -> String {
    let mut words: Vec<String> = prefix.iter().map(|w| crate::value::tcl_quote(w)).collect();
    for a in args {
        words.push(crate::value::tcl_quote(a));
    }
    words.join(" ")
}

/// Invoke a reflected/transform handler method. Errors propagate exactly as
/// the handler raised them (tclsh: handler failures surface verbatim).
fn invoke_handler(interp: &mut Interp, prefix: &[String], args: &[&str]) -> Result<Value> {
    interp.eval(&handler_script(prefix, args))
}

/// Render the `chan handler "..." does not support all required methods`
/// failure raised when `initialize` advertises an incomplete method set.
fn missing_required_methods(prefix: &[String]) -> Error {
    Error::runtime(
        format!(
            "chan handler \"{}\" does not support all required methods",
            prefix.join(" ")
        ),
        crate::error::ErrorCode::Io,
    )
}

/// Validate the list returned by a handler's `initialize`: every element
/// must uniquely prefix-match a method of the channel kind.
fn validate_handler_methods(
    interp: &mut Interp,
    prefix: &[String],
    kind: &str,
    table: &[&'static str],
    returned: &Value,
) -> Result<Vec<String>> {
    let raw = returned.as_str();
    let elems = match Value::from_str(raw).as_list_strict() {
        Ok(e) => e,
        Err(e) => {
            super::list::set_error_code(interp, e.code);
            return Err(Error::runtime(e.message.clone(), crate::error::ErrorCode::Generic));
        }
    };
    let mut methods = Vec::new();
    for el in &elems {
        let name = el.as_str();
        match resolve_prefix(table, name) {
            Ok(m) => methods.push(m.to_string()),
            Err(()) => {
                interp.globals.insert(
                    "errorCode".to_string(),
                    Value::from_str(&format!(
                        "TCL LOOKUP INDEX method {}",
                        crate::value::tcl_quote(name)
                    )),
                );
                interp.err_code_raised = true;
                return Err(Error::runtime(
                    format!(
                        "chan handler \"{} {}\" returned bad method \"{}\": must be {}",
                        prefix.join(" "),
                        kind,
                        name,
                        must_be_list(table)
                    ),
                    crate::error::ErrorCode::Io,
                ));
            }
        }
    }
    Ok(methods)
}

/// Tcl's "must be a, b, or c" rendering of an option table.
fn must_be_list(table: &[&str]) -> String {
    match table.len() {
        0 => String::new(),
        1 => table[0].to_string(),
        2 => format!("{} or {}", table[0], table[1]),
        _ => format!("{}, or {}", table[..table.len() - 1].join(", "), table[table.len() - 1]),
    }
}

pub fn cmd_chan(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan subcommand ?arg ...?\"".to_string(),
        ));
    }
    let sub = args[1].as_str();
    let resolved = match resolve_prefix(CHAN_SUBCOMMANDS, sub) {
        Ok(s) => s,
        Err(()) => {
            interp.globals.insert(
                "errorCode".to_string(),
                Value::from_str(&format!("TCL LOOKUP SUBCOMMAND {}", crate::value::tcl_quote(sub))),
            );
            interp.err_code_raised = true;
            return Err(Error::runtime(
                format!(
                    "unknown or ambiguous subcommand \"{}\": must be {}",
                    sub,
                    must_be_list(CHAN_SUBCOMMANDS)
                ),
                crate::error::ErrorCode::Io,
            ));
        }
    };

    // Subcommands that delegate to the standalone commands: rebuild the
    // argument vector with the standalone command's name at index 0 so
    // arity errors report the same usage text.
    macro_rules! delegate {
        ($func:expr, $name:expr) => {{
            let mut new_args: Vec<Value> = vec![Value::from_str($name)];
            for a in &args[2..] {
                new_args.push(a.clone());
            }
            ($func)(interp, &new_args)
        }};
    }

    match resolved {
        "blocked" => delegate!(cmd_fblocked, "fblocked"),
        "close" => delegate!(cmd_close, "close"),
        "configure" => delegate!(cmd_fconfigure, "fconfigure"),
        "eof" => delegate!(cmd_eof, "eof"),
        "flush" => delegate!(cmd_flush, "flush"),
        "gets" => delegate!(cmd_gets, "gets"),
        "read" => delegate!(cmd_read, "read"),
        "seek" => delegate!(cmd_seek, "seek"),
        "tell" => delegate!(cmd_tell, "tell"),
        "puts" => delegate!(super::io::cmd_puts, "puts"),
        "names" => chan_names(interp, &args[2..]),
        "pending" => chan_pending(interp, &args[2..]),
        "create" => chan_create(interp, &args[2..]),
        "push" => chan_push(interp, &args[2..]),
        "pipe" => chan_pipe(interp, &args[2..]),
        "copy" => chan_copy(interp, &args[2..]),
        "truncate" => chan_truncate(interp, &args[2..]),
        "pop" => chan_pop(interp, &args[2..]),
        // `chan event`/`chan postevent` drive the event loop's channel
        // watchers; with synchronous channels there is nothing to watch.
        "event" => {
            if args.len() < 3 {
                return Err(Error::wrong_args_msg(
                    "wrong # args: should be \"chan event channelId event ?script?\"".to_string(),
                ));
            }
            let _ = interp;
            Ok(Value::empty())
        }
        "postevent" => {
            let chan_id = if args.len() >= 3 { args[2].as_str() } else { "" };
            Err(chan_not_found_reflected(interp, chan_id))
        }
        _ => unreachable!(),
    }
}

/// `chan postevent` on a non-reflected channel.
fn chan_not_found_reflected(interp: &mut Interp, id: &str) -> Error {
    interp.globals.insert(
        "errorCode".to_string(),
        Value::from_str(&format!("TCL LOOKUP CHANNEL {}", crate::value::tcl_quote(id))),
    );
    interp.err_code_raised = true;
    Error::runtime(
        format!("can not find reflected channel named \"{}\"", id),
        crate::error::ErrorCode::Io,
    )
}

/// `chan names ?pattern?` — live channels in creation order.
fn chan_names(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 1 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan names ?pattern?\"".to_string(),
        ));
    }
    let pattern = args.first().map(|v| v.as_str());
    let names: Vec<Value> = interp
        .channels
        .channel_names()
        .into_iter()
        .filter(|name| pattern.map(|p| super::super::glob_match(p, name)).unwrap_or(true))
        .map(Value::from_str)
        .collect();
    Ok(Value::from_list(&names))
}

/// `chan pending mode channelId` — bytes buffered for that direction, or
/// -1 when the channel is not open for it.
fn chan_pending(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan pending mode channelId\"".to_string(),
        ));
    }
    let mode = args[0].as_str();
    let reading = match resolve_prefix(&["input", "output"], mode) {
        Ok("input") => true,
        Ok("output") => false,
        Err(()) | Ok(_) => {
            interp.globals.insert(
                "errorCode".to_string(),
                Value::from_str(&format!(
                    "TCL LOOKUP INDEX mode {}",
                    crate::value::tcl_quote(mode)
                )),
            );
            interp.err_code_raised = true;
            return Err(Error::runtime(
                format!("bad mode \"{}\": must be input or output", mode),
                crate::error::ErrorCode::Io,
            ));
        }
    };
    let chan_id = args[1].as_str();
    let supported = {
        if !interp.channels.contains(chan_id) {
                return Err(chan_not_found(interp, chan_id));
            }
let ch = interp.channels.get_mut(chan_id).unwrap();
        if reading { ch.is_readable() } else { ch.is_writable() }
    };
    if !supported {
        return Ok(Value::from_int(-1));
    }
    // Reads and writes go straight through to the OS, so nothing is ever
    // buffered inside the interpreter for a healthy channel.
    Ok(Value::from_int(0))
}

/// `chan create mode cmdprefix` — build a reflected channel.
fn chan_create(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan create mode cmdprefix\"".to_string(),
        ));
    }
    // The access mode list must parse, and each element must be read/write.
    let mode_elems = match Value::from_str(args[0].as_str()).as_list_strict() {
        Ok(e) => e,
        Err(e) => {
            super::list::set_error_code(interp, e.code);
            return Err(Error::runtime(e.message.clone(), crate::error::ErrorCode::Generic));
        }
    };
    let mut modes: Vec<String> = Vec::new();
    for m in &mode_elems {
        let name = m.as_str();
        match resolve_prefix(&["read", "write"], name) {
            Ok("read") if !modes.iter().any(|x| x == "read") => modes.push("read".to_string()),
            Ok("write") if !modes.iter().any(|x| x == "write") => modes.push("write".to_string()),
            _ => {
                interp.globals.insert(
                    "errorCode".to_string(),
                    Value::from_str(&format!(
                        "TCL LOOKUP INDEX mode {}",
                        crate::value::tcl_quote(name)
                    )),
                );
                interp.err_code_raised = true;
                return Err(Error::runtime(
                    format!("bad mode \"{}\": must be read or write", name),
                    crate::error::ErrorCode::Io,
                ));
            }
        }
    }
    // The command prefix must be a valid list.
    let prefix_elems = match Value::from_str(args[1].as_str()).as_list_strict() {
        Ok(e) => e,
        Err(e) => {
            super::list::set_error_code(interp, e.code);
            return Err(Error::runtime(e.message.clone(), crate::error::ErrorCode::Generic));
        }
    };
    let prefix: Vec<String> = prefix_elems.iter().map(|v| v.as_str().to_string()).collect();

    let handle = interp.channels.alloc_reflected_name();
    let mode_list = Value::from_list(
        &modes.iter().map(|m| Value::from_str(m)).collect::<Vec<_>>(),
    )
    .as_str()
    .to_string();
    let returned = invoke_handler(
        interp,
        &prefix,
        &["initialize", &handle, &mode_list],
    )?;
    let methods =
        validate_handler_methods(interp, &prefix, "initialize", REFLECTED_METHODS, &returned)?;
    // A usable channel needs initialize/finalize/watch plus every
    // direction it was created with.
    let mut required = vec!["initialize", "finalize", "watch"];
    if modes.iter().any(|m| m == "read") {
        required.push("read");
    }
    if modes.iter().any(|m| m == "write") {
        required.push("write");
    }
    for r in required {
        if !methods.iter().any(|m| m == r) {
            return Err(missing_required_methods(&prefix));
        }
    }

    let info = ReflectedChannel {
        prefix,
        handle: handle.clone(),
        modes,
        methods,
        at_eof: false,
    };
    interp.channels.register(
        handle.clone(),
        Box::new(crate::channel::PlaceholderChannel::new(
            info.modes.iter().any(|m| m == "read"),
            info.modes.iter().any(|m| m == "write"),
        )),
    );
    interp.reflected.insert(handle.clone(), info);
    Ok(Value::from_str(&handle))
}

/// `chan push channel cmdprefix` — wrap a channel in a transform.
fn chan_push(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan push channel cmdprefix\"".to_string(),
        ));
    }
    let chan_id = args[0].as_str();
    let (ch_read, ch_write) = {
        if !interp.channels.contains(chan_id) {
                return Err(chan_not_found(interp, chan_id));
            }
let ch = interp.channels.get_mut(chan_id).unwrap();
        (ch.is_readable(), ch.is_writable())
    };
    let prefix_elems = match Value::from_str(args[1].as_str()).as_list_strict() {
        Ok(e) => e,
        Err(e) => {
            super::list::set_error_code(interp, e.code);
            return Err(Error::runtime(e.message.clone(), crate::error::ErrorCode::Generic));
        }
    };
    let prefix: Vec<String> = prefix_elems.iter().map(|v| v.as_str().to_string()).collect();

    let handle = interp.channels.alloc_transform_name();
    let mut modes: Vec<&str> = Vec::new();
    if ch_read {
        modes.push("read");
    }
    if ch_write {
        modes.push("write");
    }
    let mode_list = modes.join(" ");
    let returned = invoke_handler(
        interp,
        &prefix,
        &["initialize", &handle, &mode_list],
    )?;
    let methods =
        validate_handler_methods(interp, &prefix, "initialize", TRANSFORM_METHODS, &returned)?;
    interp.transforms.insert(
        chan_id.to_string(),
        ChannelTransform {
            prefix,
            handle,
            methods,
        },
    );
    Ok(Value::from_str(chan_id))
}

/// `chan pipe` — a connected pair of channels (read end, write end).
fn chan_pipe(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if !args.is_empty() {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan pipe\"".to_string(),
        ));
    }
    let (read_end, write_end) = crate::channel::pipe_pair();
    let rid = interp.channels.register_new(Box::new(read_end));
    let wid = interp.channels.register_new(Box::new(write_end));
    Ok(Value::from_str(&format!("{} {}", rid, wid)))
}

/// `chan copy ?-size n? from to` — synchronous copy of readable → writable.
fn chan_copy(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut size: Option<i64> = None;
    let mut idx = 0;
    if args.first().map(|v| v.as_str() == "-size").unwrap_or(false) {
        if args.len() < 2 {
            return Err(Error::wrong_args_msg(
                "wrong # args: should be \"chan copy ?-size size? fromChan toChan\"".to_string(),
            ));
        }
        size = args[1].as_int();
        idx = 2;
    }
    if args.len() - idx != 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan copy ?-size size? fromChan toChan\"".to_string(),
        ));
    }
    let from = args[idx].as_str().to_string();
    let to = args[idx + 1].as_str().to_string();

    // Source must be readable, target must be writable.
    if !interp.channels.get(&from).map(|ch| ch.is_readable()).unwrap_or(false) {
        return Err(match interp.channels.get(&from) {
            Some(_) => not_opened(&from, true),
            None => chan_not_found(interp, &from),
        });
    }
    if !interp.channels.get(&to).map(|ch| ch.is_writable()).unwrap_or(false) {
        return Err(match interp.channels.get(&to) {
            Some(_) => not_opened(&to, false),
            None => chan_not_found(interp, &to),
        });
    }

    let mut copied: i64 = 0;
    loop {
        let want = size.map(|s| (s - copied).min(4096).max(0) as usize).unwrap_or(4096);
        if want == 0 {
            break;
        }
        let chunk = {
            let ch = interp.channels.get_mut(&from).unwrap();
            crate::channel::channel_read_chars(ch.as_mut(), want)
        };
        let chunk = match chunk {
            Ok(c) if c.is_empty() => break,
            Ok(c) => c,
            Err(e) => {
                return Err(Error::runtime(
                    format!("error reading \"{}\": {}", from, e),
                    crate::error::ErrorCode::Io,
                ))
            }
        };
        let n = chunk.len() as i64;
        {
            let ch = interp.channels.get_mut(&to).unwrap();
            if let Err(e) = crate::channel::channel_write_str(ch.as_mut(), &chunk) {
                return Err(Error::runtime(
                    format!("error writing \"{}\": {}", to, e),
                    crate::error::ErrorCode::Io,
                ));
            }
            let _ = ch.flush();
        }
        copied += n;
    }
    Ok(Value::from_int(copied))
}

/// `chan truncate channelId ?length?` — truncate a seekable channel.
fn chan_truncate(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan truncate channelId ?length?\"".to_string(),
        ));
    }
    let chan_id = args[0].as_str();
    let length = if args.len() == 2 {
        args[1].as_int().unwrap_or(0) as u64
    } else {
        if !interp.channels.contains(chan_id) {
                return Err(chan_not_found(interp, chan_id));
            }
let ch = interp.channels.get_mut(chan_id).unwrap();
        ch.tell().unwrap_or(0)
    };
    if !interp.channels.contains(chan_id) {
        return Err(chan_not_found(interp, chan_id));
    }
    let ch = interp.channels.get_mut(chan_id).unwrap();
    if !ch.is_writable() {
        return Err(not_opened(chan_id, false));
    }
    ch.set_length(length).map_err(io_err)?;
    Ok(Value::empty())
}

/// `chan pop channel ?direction?` — remove the topmost transform.
fn chan_pop(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.is_empty() || args.len() > 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"chan pop channel ?direction?\"".to_string(),
        ));
    }
    let chan_id = args[0].as_str();
    interp.transforms.remove(chan_id);
    Ok(Value::empty())
}

/// Read from a reflected channel through its handler. `Ok(None)` means the
/// channel is not reflected (plain channel read should proceed).
fn reflected_read(interp: &mut Interp, chan_id: &str, count: Option<i64>) -> Result<Option<String>> {
    let info = match interp.reflected.get(chan_id) {
        Some(i) => i.clone(),
        None => return Ok(None),
    };
    if !info.modes.iter().any(|m| m == "read") || !info.methods.iter().any(|m| m == "read") {
        return Err(not_opened(chan_id, true));
    }
    let want = count.unwrap_or(4096).max(0).to_string();
    let data = invoke_handler(interp, &info.prefix, &["read", &info.handle, &want])?;
    let data = data.as_str().to_string();
    if let Some(r) = interp.reflected.get_mut(chan_id) {
        r.at_eof = data.is_empty();
    }
    Ok(Some(data))
}

/// Pass raw bytes through a transform's `read` handler.
fn transform_through_read(
    interp: &mut Interp,
    t: &ChannelTransform,
    raw: &str,
) -> Result<String> {
    let data = invoke_handler(interp, &t.prefix, &["read", &t.handle, raw])?;
    Ok(data.as_str().to_string())
}

/// Write to a reflected channel through its handler. `Ok(false)` means the
/// channel is not reflected (plain write should proceed).
pub(crate) fn reflected_write(interp: &mut Interp, chan_id: &str, data: &str) -> Result<bool> {
    let Some(info) = interp.reflected.get(chan_id).cloned() else {
        return Ok(false);
    };
    if !info.modes.iter().any(|m| m == "write") || !info.methods.iter().any(|m| m == "write") {
        return Err(not_opened(chan_id, false));
    }
    invoke_handler(interp, &info.prefix, &["write", &info.handle, data])?;
    Ok(true)
}

/// Write through a channel transform's `write` handler, if it has one.
pub(crate) fn transform_write(
    interp: &mut Interp,
    chan_id: &str,
    data: &str,
) -> Result<Option<String>> {
    let Some(t) = interp.transforms.get(chan_id).cloned() else {
        return Ok(None);
    };
    if !t.methods.iter().any(|m| m == "write") {
        return Ok(None);
    }
    let out = invoke_handler(interp, &t.prefix, &["write", &t.handle, data])?;
    Ok(Some(out.as_str().to_string()))
}

/// Close out reflected/transform state for `close channelId`; returns true
/// when the channel was a reflected one (its handler has been finalized).
pub(crate) fn close_reflected(interp: &mut Interp, chan_id: &str) -> Result<bool> {
    if let Some(info) = interp.reflected.remove(chan_id) {
        if info.methods.iter().any(|m| m == "finalize") {
            invoke_handler(interp, &info.prefix, &["finalize", &info.handle])?;
        }
        interp.transforms.remove(chan_id);
        return Ok(true);
    }
    if let Some(t) = interp.transforms.remove(chan_id) {
        if t.methods.iter().any(|m| m == "drain") {
            let _ = invoke_handler(interp, &t.prefix, &["drain", &t.handle]);
        }
        if t.methods.iter().any(|m| m == "flush") {
            let _ = invoke_handler(interp, &t.prefix, &["flush", &t.handle]);
        }
        if t.methods.iter().any(|m| m == "finalize") {
            let _ = invoke_handler(interp, &t.prefix, &["finalize", &t.handle]);
        }
    }
    Ok(false)
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
