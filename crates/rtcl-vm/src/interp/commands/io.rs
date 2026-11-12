//! I/O and file commands: puts, source, file, format, glob.

#[cfg(feature = "io")]
use super::chan_io;
use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

// ---------- POSIX 错误形 ----------

/// errno → (Tcl 小写 strerror 文本, 错误码名)。
#[cfg(feature = "std")]
pub(crate) fn errno_text(code: i32) -> Option<(&'static str, &'static str)> {
    Some(match code {
        2 => ("no such file or directory", "ENOENT"),
        9 => ("bad file descriptor", "EBADF"),
        11 => ("resource temporarily unavailable", "EAGAIN"),
        13 => ("permission denied", "EACCES"),
        17 => ("file already exists", "EEXIST"),
        20 => ("not a directory", "ENOTDIR"),
        21 => ("is a directory", "EISDIR"),
        22 => ("invalid argument", "EINVAL"),
        24 => ("too many open files", "EMFILE"),
        28 => ("no space left on device", "ENOSPC"),
        29 => ("invalid seek", "ESPIPE"),
        30 => ("read-only file system", "EROFS"),
        32 => ("broken pipe", "EPIPE"),
        36 => ("file name too long", "ENAMETOOLONG"),
        40 => ("too many levels of symbolic links", "ELOOP"),
        _ => return None,
    })
}

/// io::Error → Tcl/POSIX 文本（去 Rust 的 "(os error N)" 后缀并小写首字母）。
#[cfg(feature = "std")]
pub(crate) fn io_error_text(e: &std::io::Error, fallback: &str) -> String {
    if let Some((text, _)) = e.raw_os_error().and_then(errno_text) {
        return text.to_string();
    }
    let s = e.to_string();
    let s = match s.find(" (os error ") {
        Some(i) => &s[..i],
        None => s.as_str(),
    };
    if s.is_empty() {
        return fallback.to_string();
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => fallback.to_string(),
    }
}

/// 构造 POSIX 错误（消息 = `<prefix>: <text>`，并装 `POSIX <NAME> {text}`）。
#[cfg(feature = "std")]
pub(crate) fn posix_error(interp: &mut Interp, prefix: String, e: &std::io::Error) -> Error {
    let text = io_error_text(e, "i/o error");
    if let Some((_, name)) = e.raw_os_error().and_then(errno_text) {
        super::list::set_error_code(interp, &format!("POSIX {} {{{}}}", name, text));
    }
    Error::runtime(format!("{}: {}", prefix, text), crate::error::ErrorCode::Io)
}

/// seek 失败形：非 seekable（ESPIPE/Unsupported）→ `invalid seek`；
/// 其余按 POSIX 文本。
#[cfg(feature = "std")]
pub(crate) fn seek_error(interp: &mut Interp, chan: &str, e: &std::io::Error) -> Error {
    if e.raw_os_error() == Some(29) || e.kind() == std::io::ErrorKind::Unsupported {
        super::list::set_error_code(interp, "POSIX ESPIPE {invalid seek}");
        return Error::runtime(
            format!("error during seek on \"{}\": invalid seek", chan),
            crate::error::ErrorCode::Io,
        );
    }
    posix_error(interp, format!("error during seek on \"{}\"", chan), e)
}

// ---------- puts ----------

#[cfg(feature = "std")]
pub fn cmd_puts(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // puts ?-nonewline? ?channelId? string
    let mut nonewline = false;
    let mut chan_id = "stdout";
    let msg;

    match args.len() {
        2 => {
            msg = args[1].as_str();
        }
        3 => {
            let first = args[1].as_str();
            if first == "-nonewline" {
                nonewline = true;
                msg = args[2].as_str();
            } else {
                // first is channelId
                chan_id = first;
                msg = args[2].as_str();
            }
        }
        4 => {
            // 4 词形只允许 `puts -nonewline ?channelId? string`；
            // `puts stderr -nonewline hi` 在 tclsh 是 wrong # args。
            if args[1].as_str() != "-nonewline" {
                return Err(Error::wrong_args_with_usage(
                    "puts",
                    2,
                    args.len(),
                    "?-nonewline? ?channelId? string",
                ));
            }
            nonewline = true;
            chan_id = args[2].as_str();
            msg = args[3].as_str();
        }
        _ => return Err(Error::wrong_args_with_usage("puts", 2, args.len(), "?-nonewline? ?channelId? string")),
    }

    // ── 非 io 构建：宿主控制台路径 ─────────────────────────────────
    // stdout/stderr 走 HostConsole 服务（原生 stdio，或 wasm 宿主经
    // set_console 注入的 JS 实现）；stdin 不可写；其余通道视为不存在。
    #[cfg(all(feature = "std", not(feature = "io")))]
    {
        let newline = if nonewline { "" } else { "\n" };
        let out = format!("{}{}", msg, newline);
        match chan_id {
            "stdout" => interp.console.write_stdout(&out).map_err(|e| {
                Error::runtime(
                    format!("error writing \"stdout\": {}", e),
                    crate::error::ErrorCode::Io,
                )
            })?,
            "stderr" => interp.console.write_stderr(&out).map_err(|e| {
                Error::runtime(
                    format!("error writing \"stderr\": {}", e),
                    crate::error::ErrorCode::Io,
                )
            })?,
            "stdin" => return Err(puts_not_opened(interp, chan_id, false)),
            _ => return Err(puts_chan_not_found(interp, chan_id)),
        }
        return Ok(Value::empty());
    }

    // ── io 构建：完整通道栈（反射通道、变换、缓冲与 flush）──────
    #[cfg(feature = "io")]
    {
        // wasm 宿主：内置 stdout/stderr 落到宿主控制台（OsStdout 在
        // wasm32-unknown-unknown 上是对 std::io::stdout 的空转发）。
        // 宿主经 set_console 注入的 JsConsole 才看得到 puts 输出；
        // 其余通道仍走完整通道栈。
        #[cfg(target_family = "wasm")]
        {
            if chan_id == "stdout" || chan_id == "stderr" {
                let newline = if nonewline { "" } else { "\n" };
                let out = format!("{}{}", msg, newline);
                let r = if chan_id == "stdout" {
                    interp.console.write_stdout(&out)
                } else {
                    interp.console.write_stderr(&out)
                };
                r.map_err(|e| {
                    Error::runtime(
                        format!("error writing \"{}\": {}", chan_id, e),
                        crate::error::ErrorCode::Io,
                    )
                })?;
                return Ok(Value::empty());
            }
        }

        if !interp.channels.contains(chan_id) {
            return Err(chan_io::chan_not_found(interp, chan_id));
        }

        // Reflected channels (chan create) write through their handler script.
        if chan_io::reflected_write(interp, chan_id, msg)? {
            if !nonewline {
                chan_io::reflected_write(interp, chan_id, "\n")?;
            }
            return Ok(Value::empty());
        }
        // Channel transforms (chan push) filter bytes through their handler.
        let out = match chan_io::transform_write(interp, chan_id, msg)? {
            Some(data) => data,
            None => msg.to_string(),
        };
        let newline = if nonewline { String::new() } else { "\n".to_string() };
        let out = if newline.is_empty() { out } else { format!("{}{}", out, newline) };

        let (ch, cfg) = interp.channels.channel_ctx_mut(chan_id)
            .ok_or_else(|| Error::runtime(
                format!("can not find channel named \"{}\"", chan_id),
                crate::error::ErrorCode::Io,
            ))?;
        if !ch.is_writable() {
            return Err(chan_io::not_opened(interp, chan_id, false));
        }

        crate::channel::channel_write_str_cfg(ch, cfg, &out).map_err(|e| Error::runtime(
            format!("error writing \"{}\": {}", chan_id, e),
            crate::error::ErrorCode::Io,
        ))?;

        ch.flush().map_err(|e| Error::runtime(
            format!("error flushing \"{}\": {}", chan_id, e),
            crate::error::ErrorCode::Io,
        ))?;

        Ok(Value::empty())
    }
}

/// 非 io 构建的通道缺失错误（与 chan_io::chan_not_found 同一 tclsh
/// 消息与 ::errorCode 形状；那个助手在 io 特性下）。
#[cfg(all(feature = "std", not(feature = "io")))]
fn puts_chan_not_found(interp: &mut Interp, id: &str) -> Error {
    super::list::set_error_code(
        interp,
        &format!("TCL LOOKUP CHANNEL {}", crate::value::tcl_quote(id)),
    );
    Error::runtime(
        format!("can not find channel named \"{}\"", id),
        crate::error::ErrorCode::Io,
    )
}

/// 非 io 构建的通道方向错误（与 chan_io::not_opened 同形）。
#[cfg(all(feature = "std", not(feature = "io")))]
fn puts_not_opened(interp: &mut Interp, id: &str, reading: bool) -> Error {
    super::list::set_error_code(interp, "NONE");
    Error::runtime(
        format!(
            "channel \"{}\" wasn't opened for {}",
            id,
            if reading { "reading" } else { "writing" }
        ),
        crate::error::ErrorCode::Io,
    )
}

#[cfg(not(feature = "std"))]
pub fn cmd_puts(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    // no-std: puts is a no-op
    Ok(Value::empty())
}

// ---------- source ----------

#[cfg(feature = "file")]
pub fn cmd_source(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("source", 2, args.len()));
    }
    let path = args[1].as_str();
    let contents = interp.host_fs.read_file(path).map_err(|e| {
        Error::runtime(
            format!("couldn't read file \"{}\": {}", path, e),
            crate::error::ErrorCode::Io,
        )
    })?;
    // tclsh sources a file incrementally: earlier commands execute even if a
    // later one has a syntax error (the compile-run entry point, `rtcl -f`,
    // still parses the whole script by design).
    let r = interp.eval_incremental(&contents);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            // Sourced files unwind through tclsh's `(file "..." line N)`
            // frame before the enclosing `source` command is named.
            interp.err_exit_frame(&format!("file \"{}\"", path));
        }
    }
    r
}

#[cfg(not(feature = "file"))]
pub fn cmd_source(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "source: not available without 'file' feature",
        crate::error::ErrorCode::InvalidOp,
    ))
}

// ---------- file ----------

#[cfg(feature = "file")]
pub fn cmd_file(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "file", 2, args.len(),
            "subcommand ?arg ...?",
        ));
    }

    let subcmd = args[1].as_str();

    // Subcommands that need no path argument
    match subcmd {
        "tempfile" => return file_tempfile(interp, args),
        "separator" | "sep" => {
            return Ok(Value::from_str(std::path::MAIN_SEPARATOR_STR));
        }
        _ => {}
    }

    // All other subcommands need at least one path argument
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "file", 3, args.len(),
            format!("{} name ?arg ...?", subcmd),
        ));
    }

    let path = args[2].as_str();

    match subcmd {
        // ── existence / type checks ─────────────────────────────────
        "exists" => Ok(Value::from_bool(interp.host_fs.exists(path))),
        "isfile" => Ok(Value::from_bool(interp.host_fs.is_file(path))),
        "isdirectory" => Ok(Value::from_bool(interp.host_fs.is_dir(path))),
        "readable" => Ok(Value::from_bool(interp.host_fs.readable(path))),
        "writable" => Ok(Value::from_bool(interp.host_fs.writable(path))),
        "executable" => Ok(Value::from_bool(interp.host_fs.executable(path))),
        "owned" => {
            // Approximate: check if the file exists and we can read its metadata.
            // A full implementation would compare uid, but that requires libc.
            Ok(Value::from_bool(interp.host_fs.exists(path)))
        }
        "type" => {
            // 数据面：目录/文件判定走 HostFs；link 判定是 unix 深水面
            //（symlink_metadata），留原生直连——HostFs 实现不支持时
            // 统一报 "file"（tclsh 的回退类型）。
            let link = std::fs::symlink_metadata(path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            let t = if link {
                "link"
            } else if interp.host_fs.is_dir(path) {
                "directory"
            } else {
                "file"
            };
            Ok(Value::from_str(t))
        }

        // ── path manipulation ───────────────────────────────────────
        "extension" => {
            // jimtcl returns extension WITH the dot: ".txt"
            let p = std::path::Path::new(path);
            match p.extension().and_then(|s| s.to_str()) {
                Some(ext) => Ok(Value::from_str(&format!(".{}", ext))),
                None => Ok(Value::from_str("")),
            }
        }
        "tail" => {
            let p = std::path::Path::new(path);
            Ok(Value::from_str(
                p.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            ))
        }
        "dirname" | "dir" => {
            // tclsh：`file dirname /` = "/"（根的 parent 是根）。
            if path == "/" {
                return Ok(Value::from_str("/"));
            }
            let p = std::path::Path::new(path);
            Ok(Value::from_str(
                p.parent().and_then(|s| s.to_str()).unwrap_or("."),
            ))
        }
        "rootname" => {
            let p = std::path::Path::new(path);
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Some(parent) = p.parent().and_then(|s| s.to_str()) {
                if parent == "." || parent.is_empty() {
                    Ok(Value::from_str(stem))
                } else {
                    Ok(Value::from_str(&format!("{}/{}", parent, stem)))
                }
            } else {
                Ok(Value::from_str(stem))
            }
        }
        "split" => {
            let p = std::path::Path::new(path);
            let parts: Vec<Value> = p.components()
                .map(|c| Value::from_str(c.as_os_str().to_str().unwrap_or("")))
                .collect();
            Ok(Value::from_list(&parts))
        }
        "join" => {
            let result: Vec<&str> = args[2..].iter().map(|a| a.as_str()).collect();
            let joined: std::path::PathBuf = result.iter().collect();
            Ok(Value::from_str(joined.to_str().unwrap_or("")))
        }
        "normalize" => {
            let p = std::fs::canonicalize(path).unwrap_or_else(|_| {
                std::path::PathBuf::from(path)
            });
            Ok(Value::from_str(&p.to_string_lossy()))
        }

        // ── metadata ────────────────────────────────────────────────
        "size" => {
            let n = interp.host_fs.size(path).map_err(|e| {
                posix_error(interp, format!("could not read \"{}\"", path), &e)
            })?;
            Ok(Value::from_int(n as i64))
        }
        "atime" => {
            let secs = interp.host_fs.atime(path).map_err(|e| {
                posix_error(interp, format!("could not read \"{}\"", path), &e)
            })?;
            Ok(Value::from_int(secs as i64))
        }
        "mtime" => {
            if args.len() > 4 {
                return Err(Error::wrong_args_with_usage(
                    "file mtime",
                    4,
                    args.len(),
                    "name ?time?",
                ));
            }
            if args.len() == 4 {
                // file mtime name time —— 设置 mtime（atime 不变）。
                let secs = match args[3].as_int() {
                    Some(v) => v,
                    None => {
                        super::list::set_error_code(interp, "TCL VALUE NUMBER");
                        return Err(Error::runtime(
                            format!("expected integer but got \"{}\"", args[3].as_str()),
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                };
                let t = std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(secs.max(0) as u64);
                let ft = std::fs::FileTimes::new().set_modified(t);
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .and_then(|f| f.set_times(ft))
                    .map_err(|e| {
                        posix_error(interp, format!("could not set \"{}\"", path), &e)
                    })?;
                return Ok(Value::empty());
            }
            let secs = interp.host_fs.mtime(path).map_err(|e| {
                posix_error(interp, format!("could not read \"{}\"", path), &e)
            })?;
            Ok(Value::from_int(secs as i64))
        }
        "stat" | "lstat" => {
            // file stat name varName / file lstat name varName
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "file", 4, args.len(),
                    format!("{} name varName", subcmd),
                ));
            }
            let var_name = args[3].as_str();
            let meta = if subcmd == "lstat" {
                std::fs::symlink_metadata(path)
            } else {
                std::fs::metadata(path)
            };
            let meta = meta
                .map_err(|e| posix_error(interp, format!("could not read \"{}\"", path), &e))?;
            let size = meta.len() as i64;
            let mtime = meta.modified().ok()
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64).unwrap_or(0);
            let atime = meta.accessed().ok()
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64).unwrap_or(0);
            let ft = meta.file_type();
            let ftype = if ft.is_symlink() { "link" }
                else if ft.is_dir() { "directory" }
                else { "file" };

            interp.set_var(&format!("{}(size)", var_name), Value::from_int(size))?;
            interp.set_var(&format!("{}(mtime)", var_name), Value::from_int(mtime))?;
            interp.set_var(&format!("{}(atime)", var_name), Value::from_int(atime))?;
            interp.set_var(&format!("{}(type)", var_name), Value::from_str(ftype))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                interp.set_var(&format!("{}(dev)", var_name), Value::from_int(meta.dev() as i64))?;
                interp.set_var(&format!("{}(ino)", var_name), Value::from_int(meta.ino() as i64))?;
                interp.set_var(&format!("{}(nlink)", var_name), Value::from_int(meta.nlink() as i64))?;
                interp.set_var(&format!("{}(uid)", var_name), Value::from_int(meta.uid() as i64))?;
                interp.set_var(&format!("{}(gid)", var_name), Value::from_int(meta.gid() as i64))?;
                interp.set_var(&format!("{}(mode)", var_name), Value::from_int(meta.mode() as i64))?;
                // D-33：补 blksize/blocks/ctime（tclsh 13 键全形）。
                interp.set_var(&format!("{}(blksize)", var_name), Value::from_int(meta.blksize() as i64))?;
                interp.set_var(&format!("{}(blocks)", var_name), Value::from_int(meta.blocks() as i64))?;
                interp.set_var(&format!("{}(ctime)", var_name), Value::from_int(meta.ctime() as i64))?;
            }
            Ok(Value::empty())
        }
        "readlink" => {
            let target = std::fs::read_link(path).map_err(|e| {
                Error::runtime(
                    format!("could not readlink \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            Ok(Value::from_str(&target.to_string_lossy()))
        }

        // ── file operations ─────────────────────────────────────────
        "delete" => {
            // file delete ?-force? ?--? path ...
            let mut force = false;
            let mut start = 2;
            while start < args.len() {
                match args[start].as_str() {
                    "-force" | "--force" => { force = true; start += 1; }
                    "--" => { start += 1; break; }
                    _ => break,
                }
            }
            for j in start..args.len() {
                let p = args[j].as_str();
                if interp.host_fs.is_dir(p) {
                    if force {
                        let _ = interp.host_fs.delete(p);
                    } else {
                        // 目录的删除走 HostFs.delete（数据面）；非空
                        // 目录的 remove_dir 语义由实现承载。
                        interp.host_fs.delete(p).map_err(|e| {
                            Error::runtime(
                                format!("error deleting \"{}\": {}", p, e),
                                crate::error::ErrorCode::Io,
                            )
                        })?;
                    }
                } else if interp.host_fs.exists(p) {
                    interp.host_fs.delete(p).map_err(|e| {
                        Error::runtime(
                            format!("error deleting \"{}\": {}", p, e),
                            crate::error::ErrorCode::Io,
                        )
                    })?;
                }
            }
            Ok(Value::empty())
        }
        "mkdir" => {
            for j in 2..args.len() {
                let p = args[j].as_str();
                interp.host_fs.mkdir(p, true).map_err(|e| {
                    Error::runtime(
                        format!("couldn't create directory \"{}\": {}", p, e),
                        crate::error::ErrorCode::Io,
                    )
                })?;
            }
            Ok(Value::empty())
        }
        "rename" => {
            // file rename ?-force? source target
            let mut force = false;
            let mut start = 2;
            while start < args.len() {
                match args[start].as_str() {
                    "-force" | "--force" => { force = true; start += 1; }
                    "--" => { start += 1; break; }
                    _ => break,
                }
            }
            if args.len() - start != 2 {
                return Err(Error::wrong_args_with_usage(
                    "file rename", 4, args.len(), "?-force? source target",
                ));
            }
            let src = args[start].as_str();
            let dst = args[start + 1].as_str();
            if !force && interp.host_fs.exists(dst) {
                return Err(Error::runtime(
                    format!(
                        "error renaming \"{}\" to \"{}\": file already exists",
                        src, dst
                    ),
                    crate::error::ErrorCode::Io,
                ));
            }
            interp.host_fs.rename(src, dst).map_err(|e| {
                Error::runtime(
                    format!("couldn't rename \"{}\": {}", src, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            Ok(Value::empty())
        }
        "copy" => {
            // file copy ?-force? source target
            let mut force = false;
            let mut start = 2;
            while start < args.len() {
                match args[start].as_str() {
                    "-force" | "--force" => { force = true; start += 1; }
                    "--" => { start += 1; break; }
                    _ => break,
                }
            }
            if args.len() - start != 2 {
                return Err(Error::wrong_args_with_usage(
                    "file copy", 4, args.len(), "?-force? source target",
                ));
            }
            let src = args[start].as_str();
            let dst = args[start + 1].as_str();
            if !force && interp.host_fs.exists(dst) {
                return Err(Error::runtime(
                    // tclsh 消息形状（D-33）：error copying "src" to
                    // "dst": file already exists。
                    format!(
                        "error copying \"{}\" to \"{}\": file already exists",
                        src, dst
                    ),
                    crate::error::ErrorCode::Io,
                ));
            }
            interp.host_fs.copy(src, dst).map_err(|e| {
                Error::runtime(
                    format!("couldn't copy \"{}\": {}", src, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            Ok(Value::empty())
        }
        "link" => {
            // file link ?-hard|-symbolic? newname target
            let mut link_type = "hard";
            let mut start = 2;
            if start < args.len() && args[start].as_str().starts_with('-') {
                match args[start].as_str() {
                    "-hard" => { link_type = "hard"; start += 1; }
                    "-symbolic" | "-sym" => { link_type = "symbolic"; start += 1; }
                    _ => {}
                }
            }
            if args.len() - start != 2 {
                return Err(Error::wrong_args_with_usage(
                    "file link", 4, args.len(), "?-hard|-symbolic? newname target",
                ));
            }
            let new_name = args[start].as_str();
            let target = args[start + 1].as_str();
            if link_type == "symbolic" {
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, new_name).map_err(|e| {
                    Error::runtime(
                        format!("couldn't create link \"{}\": {}", new_name, e),
                        crate::error::ErrorCode::Io,
                    )
                })?;
                #[cfg(windows)]
                {
                    if std::path::Path::new(target).is_dir() {
                        std::os::windows::fs::symlink_dir(target, new_name)
                    } else {
                        std::os::windows::fs::symlink_file(target, new_name)
                    }.map_err(|e| {
                        Error::runtime(
                            format!("couldn't create link \"{}\": {}", new_name, e),
                            crate::error::ErrorCode::Io,
                        )
                    })?;
                }
            } else {
                std::fs::hard_link(target, new_name).map_err(|e| {
                    Error::runtime(
                        format!("couldn't create link \"{}\": {}", new_name, e),
                        crate::error::ErrorCode::Io,
                    )
                })?;
            }
            Ok(Value::empty())
        }

        _ => Err(Error::runtime(
            format!(
                "bad option \"{}\": must be atime, copy, delete, dirname, \
                 executable, exists, extension, isdirectory, isfile, join, \
                 link, lstat, mkdir, mtime, normalize, owned, readable, \
                 readlink, rename, rootname, separator, size, split, stat, \
                 tail, tempfile, type, or writable",
                subcmd
            ),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

/// Create a temporary file, return its path.
#[cfg(feature = "file")]
fn file_tempfile(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let prefix = if args.len() >= 3 { args[2].as_str() } else { "tcl" };
    let path = interp.host_fs.tempfile(prefix).map_err(|e| {
        Error::runtime(
            format!("couldn't create temp file: {}", e),
            crate::error::ErrorCode::Io,
        )
    })?;
    Ok(Value::from_str(&path))
}

#[derive(Clone, Copy)]
pub(crate) enum AccessCheck { Read, Write, Exec }

#[cfg(feature = "file")]
pub(crate) fn fs_access(path: &str, check: AccessCheck) -> bool {
    let p = std::path::Path::new(path);
    if !p.exists() { return false; }
    match check {
        AccessCheck::Read => {
            // Try opening for read
            std::fs::File::open(path).is_ok()
        }
        AccessCheck::Write => {
            // Check if metadata says read-only
            std::fs::metadata(path)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
        }
        AccessCheck::Exec => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            }
            #[cfg(not(unix))]
            {
                // On Windows, check common executable extensions
                let ext = std::path::Path::new(path)
                    .extension().and_then(|s| s.to_str())
                    .unwrap_or("").to_lowercase();
                matches!(ext.as_str(), "exe" | "bat" | "cmd" | "com")
            }
        }
    }
}

#[cfg(not(feature = "file"))]
pub fn cmd_file(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "file: not available without 'file' feature",
        crate::error::ErrorCode::InvalidOp,
    ))
}

// ---------- format ----------

/// Tcl_GetDoubleFromObj for format arguments: integers widen, floats
/// pass through, a NaN double is a hard error ("floating point value is
/// Not a Number", TCL VALUE DOUBLE NAN), anything else is
/// "expected floating-point number but got ...".
fn fmt_double_value(interp: &mut Interp, v: &Value) -> Result<f64> {
    if let Some(i) = v.as_int() {
        return Ok(i as f64);
    }
    match v.as_float() {
        Some(f) if f.is_nan() => {
            super::list::set_error_code(interp, "TCL VALUE DOUBLE NAN");
            Err(Error::ControlFlow {
                kind: crate::error::ControlFlow::Error,
                value: Some(Value::from_str("floating point value is Not a Number")),
                level: 1,
                error_info: None,
                error_code: Some("TCL VALUE DOUBLE NAN".to_string()),
            })
        }
        Some(f) => Ok(f),
        None => {
            super::list::set_error_code(interp, "TCL VALUE NUMBER");
            let msg = format!("expected floating-point number but got \"{}\"", v.as_str());
            Err(Error::ControlFlow {
                kind: crate::error::ControlFlow::Error,
                value: Some(Value::from_str(&msg)),
                level: 1,
                error_info: None,
                error_code: Some("TCL VALUE NUMBER".to_string()),
            })
        }
    }
}

/// Render ±inf the way tclsh's format does: case follows the conversion
/// character, sign kept, precision ignored.
fn inf_text(_neg: bool, upper: bool) -> String {
    // Unsigned word; apply_sign adds '-' for negative values.
    if upper {
        "INF".to_string()
    } else {
        "inf".to_string()
    }
}

/// Strict Tcl integer parse for format conversions: after trimming
/// whitespace the whole string must be a valid Tcl integer -- optional
/// sign, then 0x/0b/0o radix forms, legacy leading-0 octal, or decimal.
/// No underscores, no floats. Out-of-range magnitudes keep the low 64
/// bits (tclsh wraps); the sign is applied in u64 space before the
/// i64 reinterpret, so -0xffffffffffffffff formats as +1.
fn parse_tcl_int_strict(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if body.is_empty() {
        return None;
    }
    let (digits, radix): (&str, u32) =
        if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            if r.is_empty() { return None; }
            (r, 16)
        } else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            if r.is_empty() { return None; }
            (r, 2)
        } else if let Some(r) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            if r.is_empty() { return None; }
            (r, 8)
        } else if body.len() > 1 && body.starts_with('0') {
            // Legacy leading-0 octal: "017"; "08" fails the digit check.
            (&body[1..], 8)
        } else {
            (body, 10)
        };
    let mut mag: u64 = 0;
    for c in digits.bytes() {
        let d = (c as char).to_digit(radix)?;
        mag = mag.wrapping_mul(radix as u64).wrapping_add(d as u64);
    }
    Some(if neg {
        (mag.wrapping_neg()) as i64
    } else {
        mag as i64
    })
}

/// Extended Tcl integer parse for the `l`-size conversions: same grammar as
/// `parse_tcl_int_strict`, but the magnitude accumulates in u128 so values
/// beyond 64 bits (which tclsh holds as bignums) survive.  Returns the sign
/// and magnitude; values wider than 128 bits wrap.
fn parse_tcl_int_ext(s: &str) -> Option<(bool, u128)> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if body.is_empty() {
        return None;
    }
    let (digits, radix): (&str, u32) =
        if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            if r.is_empty() { return None; }
            (r, 16)
        } else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            if r.is_empty() { return None; }
            (r, 2)
        } else if let Some(r) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            if r.is_empty() { return None; }
            (r, 8)
        } else if body.len() > 1 && body.starts_with('0') {
            (&body[1..], 8)
        } else {
            (body, 10)
        };
    let mut mag: u128 = 0;
    for c in digits.bytes() {
        let d = (c as char).to_digit(radix)?;
        mag = mag.wrapping_mul(radix as u128).wrapping_add(d as u128);
    }
    Some((neg, mag))
}

/// Format an integer conversion (`l` size) whose magnitude does not fit in
/// i64: tclsh renders the bignum decimal directly.  Sign/flag handling
/// mirrors `format_int`.
fn format_bignum_int(neg: bool, mag: u128, plus: bool, space: bool) -> String {
    let mut s = mag.to_string();
    if neg {
        s.insert(0, '-');
    } else if plus {
        s.insert(0, '+');
    } else if space {
        s.insert(0, ' ');
    }
    s
}

/// Result of the wide (`l`-size) integer parse: an in-range value renders
/// exactly like the 64-bit path; a bignum renders from its magnitude.
enum BigOrLit {
    Lit(i64),
    Big(bool, u128),
}

/// Parse for `l`-size conversions: values that fit i64 render identically
/// to `%d`; larger magnitudes (tclsh bignums) keep full precision.
fn parse_tcl_int_l(s: &str) -> Option<BigOrLit> {
    let (neg, mag) = parse_tcl_int_ext(s)?;
    let fits = if neg {
        mag <= (i64::MAX as u128) + 1
    } else {
        mag <= i64::MAX as u128
    };
    if fits {
        let v = if neg { ((mag as u64).wrapping_neg()) as i64 } else { mag as i64 };
        Some(BigOrLit::Lit(v))
    } else {
        Some(BigOrLit::Big(neg, mag))
    }
}

/// Upper bound for widths/precisions from the format string or '*' args.
/// C saturates at INT_MAX and then really allocates that many bytes; the
/// corpus never exercises widths beyond double digits, so rtcl caps at
/// 1<<20 to keep a runaway "%99999999999d" from exhausting memory.
const FMT_WIDTH_CAP: usize = 1 << 20;

/// Parse a width/precision digit run, saturating at FMT_WIDTH_CAP.
fn parse_capped_digits(s: &str) -> usize {
    match s.parse::<u64>() {
        Ok(v) => v.min(FMT_WIDTH_CAP as u64) as usize,
        Err(_) => FMT_WIDTH_CAP,
    }
}

pub fn cmd_format(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("format", 2, args.len()));
    }
    let fmt = args[1].as_str();
    let mut result = String::new();
    let mut arg_idx = 2usize;
    let mut seen_xpg = false;
    let mut seen_seq = false;
    let bytes = fmt.as_bytes();
    let len = bytes.len();
    let mut pos = 0usize;

    macro_rules! fmt_err {
        ($msg:expr, $code:expr) => {{
            super::list::set_error_code(interp, $code);
            Err(Error::ControlFlow {
                kind: crate::error::ControlFlow::Error,
                value: Some(Value::from_str(&$msg)),
                level: 1,
                error_info: None,
                error_code: Some($code.to_string()),
            })
        }};
    }
    macro_rules! int_arg {
        ($arg:expr) => {
            match parse_tcl_int_strict($arg.as_str()) {
                Some(v) => v,
                None => {
                    let msg = format!("expected integer but got \"{}\"", $arg.as_str());
                    return fmt_err!(msg, "TCL VALUE NUMBER");
                }
            }
        };
    }
    macro_rules! int_l_arg {
        ($arg:expr) => {
            match parse_tcl_int_l($arg.as_str()) {
                Some(v) => v,
                None => {
                    let msg = format!("expected integer but got \"{}\"", $arg.as_str());
                    return fmt_err!(msg, "TCL VALUE NUMBER");
                }
            }
        };
    }
    macro_rules! int_star_arg {
        ($arg:expr) => {
            match parse_tcl_int_strict($arg.as_str()) {
                Some(v) => v,
                None => {
                    let msg = format!("expected integer but got \"{}\"", $arg.as_str());
                    return fmt_err!(msg, "TCL VALUE INTEGER");
                }
            }
        };
    }
    macro_rules! not_enough {
        () => {
            return fmt_err!(
                "not enough arguments for all format specifiers",
                "TCL FORMAT FIELDVARMISMATCH"
            );
        };
    }
    macro_rules! mixing {
        () => {
            return fmt_err!(
                "cannot mix \"%\" and \"%n$\" conversion specifiers",
                "TCL FORMAT MIXEDSPECTYPES"
            );
        };
    }

    while pos < len {
        if bytes[pos] != b'%' {
            // Copy literal text up to the next '%' (UTF-8 safe).
            let next = fmt[pos..].find('%').map_or(len, |off| pos + off);
            result.push_str(&fmt[pos..next]);
            pos = next;
            continue;
        }
        pos += 1;
        if pos >= len {
            // A trailing bare '%' still reserves an argument for the
            // pending conversion before tclsh complains.
            if arg_idx >= args.len() {
                not_enough!();
            }
            return fmt_err!(
                "format string ended in middle of field specifier",
                "TCL FORMAT INCOMPLETE"
            );
        }
        if bytes[pos] == b'%' {
            result.push('%');
            pos += 1;
            continue;
        }

        // Flags (only before the width).
        let mut flag_minus = false;
        let mut flag_plus = false;
        let mut flag_zero = false;
        let mut flag_space = false;
        let mut flag_hash = false;
        macro_rules! parse_flags {
            () => {
                loop {
                    match bytes.get(pos) {
                        Some(b'-') => { flag_minus = true; pos += 1; }
                        Some(b'+') => { flag_plus = true; pos += 1; }
                        // A '0' directly followed by '$' starts the XPG
                        // index ("%0$d"), not the zero flag.
                        Some(b'0') if bytes.get(pos + 1) == Some(&b'$') => break,
                        Some(b'0') => { flag_zero = true; pos += 1; }
                        Some(b' ') => { flag_space = true; pos += 1; }
                        Some(b'#') => { flag_hash = true; pos += 1; }
                        _ => break,
                    }
                }
            };
        }
        parse_flags!();

        // Width: digits (possibly the XPG "%n$" index) or '*'.
        let mut xpg: Option<usize> = None;
        let mut width: Option<usize> = None;
        let mut star_width = false;
        let mut star_prec = false;
        let start = pos;
        while pos < len && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if pos > start {
            if pos < len && bytes[pos] == b'$' {
                pos += 1;
                xpg = Some(
                    core::str::from_utf8(&bytes[start..pos - 1])
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0),
                );
                if seen_seq {
                    mixing!();
                }
                seen_xpg = true;
                // Flags may follow the XPG index ("%1$+d").
                parse_flags!();
            } else {
                width = Some(parse_capped_digits(
                    core::str::from_utf8(&bytes[start..pos]).unwrap_or("0"),
                ));
            }
        }
        if xpg.is_some() && width.is_none() {
            // A width digit run may follow the XPG index ("%2$5d").
            let wstart = pos;
            while pos < len && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos > wstart {
                width = Some(parse_capped_digits(
                    core::str::from_utf8(&bytes[wstart..pos]).unwrap_or("0"),
                ));
            }
        }
        if width.is_none() && pos < len && bytes[pos] == b'*' {
            pos += 1;
            star_width = true;
            if xpg.is_none() {
                if seen_xpg {
                    mixing!();
                }
                seen_seq = true;
            }
            // A digit run after '*' is consumed and discarded; a '$'
            // after it falls through to the conversion-char check.
            while pos < len && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
        }

        // Precision: '.' then digits (empty run -> 0) or '*'.
        let mut precision: Option<usize> = None;
        if pos < len && bytes[pos] == b'.' {
            pos += 1;
            if pos < len && bytes[pos] == b'*' {
                pos += 1;
                star_prec = true;
                if xpg.is_none() {
                    if seen_xpg {
                        mixing!();
                    }
                    seen_seq = true;
                }
                while pos < len && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
            } else {
                let pstart = pos;
                while pos < len && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                precision = Some(if pos > pstart {
                    parse_capped_digits(core::str::from_utf8(&bytes[pstart..pos]).unwrap_or("0"))
                } else {
                    0
                });
            }
        }

        // Size modifiers: one 'h' or a run of 'l's; anything further is
        // treated as the conversion character.
        let mut size_h = false;
        let mut size_l = false;
        if pos < len && bytes[pos] == b'l' {
            while pos < len && bytes[pos] == b'l' {
                pos += 1;
            }
            size_l = true;
        } else if pos < len && bytes[pos] == b'h' {
            size_h = true;
            pos += 1;
        }

        // Conversion character.
        if pos >= len {
            if arg_idx >= args.len() {
                not_enough!();
            }
            return fmt_err!(
                "format string ended in middle of field specifier",
                "TCL FORMAT INCOMPLETE"
            );
        }
        let ch = fmt[pos..].chars().next().unwrap();
        pos += ch.len_utf8();

        // Argument selection, tclsh-style: sequential specs consume
        // arg_idx as they go; an XPG spec re-indexes as argvIndex = n-1
        // and any star width/precision consumes argv[argvIndex++] before
        // the value is taken.
        let arg: &Value;
        match xpg {
            Some(n) => {
                // Availability (index in range) is checked before the
                // conversion character is validated ("%5%" with no args
                // -> not enough; with args -> bad field specifier).
                if n < 1 || n > args.len().saturating_sub(2) {
                    return fmt_err!("\"%n$\" argument index out of range", "TCL FORMAT INDEXRANGE");
                }
                if !matches!(
                    ch,
                    'd' | 'i' | 'u' | 'b' | 'o' | 'x' | 'X' | 'c' | 's' | 'e' | 'E' | 'f'
                        | 'g' | 'G'
                ) {
                    let msg = format!("bad field specifier \"{}\"", ch);
                    return fmt_err!(msg, "TCL FORMAT BADTYPE");
                }
                let mut idx = n - 1;
                if star_width {
                    let w = int_star_arg!(args[idx + 2]);
                    if w < 0 {
                        flag_minus = true;
                        width = Some((w.unsigned_abs() as usize).min(FMT_WIDTH_CAP));
                    } else {
                        width = Some((w as usize).min(FMT_WIDTH_CAP));
                    }
                    idx += 1;
                }
                if star_prec {
                    let pv = int_star_arg!(args[idx + 2]);
                    precision = Some((pv.max(0) as usize).min(FMT_WIDTH_CAP));
                    idx += 1;
                }
                if idx >= args.len() - 2 {
                    return fmt_err!("\"%n$\" argument index out of range", "TCL FORMAT INDEXRANGE");
                }
                arg = &args[idx + 2];
            }
            None => {
                if seen_xpg {
                    mixing!();
                }
                if arg_idx >= args.len() {
                    not_enough!();
                }
                if !matches!(
                    ch,
                    'd' | 'i' | 'u' | 'b' | 'o' | 'x' | 'X' | 'c' | 's' | 'e' | 'E' | 'f'
                        | 'g' | 'G'
                ) {
                    let msg = format!("bad field specifier \"{}\"", ch);
                    return fmt_err!(msg, "TCL FORMAT BADTYPE");
                }
                if star_width {
                    if arg_idx >= args.len() {
                        not_enough!();
                    }
                    let w = int_star_arg!(args[arg_idx]);
                    arg_idx += 1;
                    if w < 0 {
                        flag_minus = true;
                        width = Some((w.unsigned_abs() as usize).min(FMT_WIDTH_CAP));
                    } else {
                        width = Some((w as usize).min(FMT_WIDTH_CAP));
                    }
                }
                if star_prec {
                    if arg_idx >= args.len() {
                        not_enough!();
                    }
                    let pv = int_star_arg!(args[arg_idx]);
                    arg_idx += 1;
                    precision = Some((pv.max(0) as usize).min(FMT_WIDTH_CAP));
                }
                if arg_idx >= args.len() {
                    not_enough!();
                }
                arg = &args[arg_idx];
                arg_idx += 1;
                seen_seq = true;
            }
        }

        let formatted = match ch {
            's' => match precision {
                Some(p) => arg.as_str().chars().take(p).collect::<String>(),
                None => arg.as_str().to_string(),
            },
            'd' | 'i' => {
                if size_l {
                    match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => {
                            format_int(v, 10, false, flag_plus, flag_space, flag_hash)
                        }
                        BigOrLit::Big(neg, mag) => {
                            format_bignum_int(neg, mag, flag_plus, flag_space)
                        }
                    }
                } else {
                    let mut v = int_arg!(arg);
                    if size_h {
                        v = (v as i16) as i64;
                    }
                    format_int(v, 10, false, flag_plus, flag_space, flag_hash)
                }
            }
            'u' => {
                if size_l {
                    match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => (v as u64).to_string(),
                        BigOrLit::Big(false, mag) => mag.to_string(),
                        BigOrLit::Big(true, mag) => format!("-{}", mag),
                    }
                } else {
                    let v = int_arg!(arg);
                    let v = if size_h { (v as u16) as u64 } else { v as u64 };
                    v.to_string()
                }
            }
            'x' => {
                let s = if size_l {
                    match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => format!("{:x}", v),
                        BigOrLit::Big(false, mag) => format!("{:x}", mag),
                        BigOrLit::Big(true, mag) => format!("-{:x}", mag),
                    }
                } else {
                    let v = int_arg!(arg);
                    if size_h { format!("{:x}", v as u16) } else { format!("{:x}", v) }
                };
                if flag_hash { format!("0x{}", s) } else { s }
            }
            'X' => {
                let s = if size_l {
                    match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => format!("{:X}", v),
                        BigOrLit::Big(false, mag) => format!("{:X}", mag),
                        BigOrLit::Big(true, mag) => format!("-{:X}", mag),
                    }
                } else {
                    let v = int_arg!(arg);
                    if size_h { format!("{:X}", v as u16) } else { format!("{:X}", v) }
                };
                if flag_hash { format!("0X{}", s) } else { s }
            }
            'o' => {
                let s = if size_l {
                    match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => format!("{:o}", v),
                        BigOrLit::Big(false, mag) => format!("{:o}", mag),
                        BigOrLit::Big(true, mag) => format!("-{:o}", mag),
                    }
                } else {
                    let v = int_arg!(arg);
                    if size_h { format!("{:o}", v as u16) } else { format!("{:o}", v) }
                };
                if flag_hash && !s.starts_with('0') { format!("0{}", s) } else { s }
            }
            'b' => {
                let s = if size_l {
                    match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => format!("{:b}", v),
                        BigOrLit::Big(false, mag) => format!("{:b}", mag),
                        BigOrLit::Big(true, mag) => format!("-{:b}", mag),
                    }
                } else {
                    let v = int_arg!(arg);
                    if size_h { format!("{:b}", v as u16) } else { format!("{:b}", v) }
                };
                if flag_hash { format!("0b{}", s) } else { s }
            }
            'c' => {
                let v = int_arg!(arg);
                // D-23：%c 是 16 位 Tcl_UniChar——>0xFFFF 直接 U+FFFD
                // （tclsh 实测 65536 → 替换符，无 16 位回绕）。
                let cp = if size_h { v as u16 as u32 } else { v as u32 };
                if cp > 0xFFFF {
                    '\u{FFFD}'.to_string()
                } else {
                    char::from_u32(cp).unwrap_or('\u{FFFD}').to_string()
                }
            }
            'f' => {
                let v = fmt_double_value(interp, arg)?;
                let prec = precision.unwrap_or(6);
                let s = if v.is_infinite() {
                    inf_text(v.is_sign_negative(), false)
                } else {
                    let mut s = format!("{:.*}", prec, v.abs());
                    if flag_hash && prec == 0 {
                        s.push('.');
                    }
                    s
                };
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'e' | 'E' => {
                let v = fmt_double_value(interp, arg)?;
                let upper = ch == 'E';
                let s = if v.is_infinite() {
                    inf_text(v.is_sign_negative(), upper)
                } else {
                    format_exp(v.abs(), precision.unwrap_or(6), upper, flag_hash)
                };
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'g' | 'G' => {
                let v = fmt_double_value(interp, arg)?;
                let upper = ch == 'G';
                let s = if v.is_infinite() {
                    inf_text(v.is_sign_negative(), upper)
                } else {
                    format_g(v.abs(), precision.unwrap_or(6).max(1), upper, flag_hash)
                };
                apply_sign(v, &s, flag_plus, flag_space)
            }
            _ => unreachable!(),
        };

        // Width padding, by characters. Observed tclsh matrix: with '0'
        // and no infinity, integers zero-fill right (sign first, '-'
        // ignored), strings/chars zero-fill left when '-' is present
        // (right otherwise), and floats zero-fill right unless '-' is
        // given (then left-justify with spaces). Without '0' or for
        // infinity results the fill is spaces.
        let w = width.unwrap_or(0);
        let disp = formatted.chars().count();
        if w > disp {
            let pad = w - disp;
            let is_str = matches!(ch, 's' | 'c');
            let is_float = matches!(ch, 'f' | 'e' | 'E' | 'g' | 'G');
            let is_inf = formatted.contains("inf") || formatted.contains("INF");
            let zero_left = is_str && flag_minus;
            if flag_zero && !is_inf && !(is_float && flag_minus) {
                if zero_left {
                    result.push_str(&formatted);
                    for _ in 0..pad {
                        result.push('0');
                    }
                } else if formatted.starts_with('-')
                    || formatted.starts_with('+')
                    || formatted.starts_with(' ')
                {
                    let (sign, rest) = formatted.split_at(1);
                    result.push_str(sign);
                    for _ in 0..pad {
                        result.push('0');
                    }
                    result.push_str(rest);
                } else {
                    for _ in 0..pad {
                        result.push('0');
                    }
                    result.push_str(&formatted);
                }
            } else if flag_minus {
                result.push_str(&formatted);
                for _ in 0..pad {
                    result.push(' ');
                }
            } else {
                for _ in 0..pad {
                    result.push(' ');
                }
                result.push_str(&formatted);
            }
        } else {
            result.push_str(&formatted);
        }
    }
    Ok(Value::from_str(&result))
}

fn format_int(v: i64, _base: u32, _upper: bool, plus: bool, space: bool, _hash: bool) -> String {
    if v >= 0 {
        if plus { format!("+{}", v) }
        else if space { format!(" {}", v) }
        else { v.to_string() }
    } else {
        v.to_string()
    }
}

fn apply_sign(v: f64, abs_str: &str, plus: bool, space: bool) -> String {
    if v.is_sign_negative() {
        format!("-{}", abs_str)
    } else if plus {
        format!("+{}", abs_str)
    } else if space {
        format!(" {}", abs_str)
    } else {
        abs_str.to_string()
    }
}

fn format_exp(v: f64, prec: usize, upper: bool, hash: bool) -> String {
    // Rust's {:.*e} is correctly rounded; reassemble with C's exponent
    // layout (sign always present, at least two digits). '#' keeps the
    // decimal point even when the precision is 0 ("1.e+01").
    let s = format!("{:.*e}", prec, v);
    let (mant, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let expn: i32 = exp.parse().unwrap_or(0);
    let e_char = if upper { 'E' } else { 'e' };
    let mut out = String::with_capacity(mant.len() + 6);
    out.push_str(mant);
    if hash && !mant.contains('.') {
        out.push('.');
    }
    out.push(e_char);
    if expn < 0 {
        out.push('-');
    } else {
        out.push('+');
    }
    let a = expn.unsigned_abs();
    if a < 10 {
        out.push('0');
    }
    out.push_str(&a.to_string());
    out
}

/// Drop trailing zeros from the fraction of a %g result (and a trailing
/// bare '.') unless the caller asked for '#' (keep them).
fn trim_g_fraction(s: String) -> String {
    if let Some(pos) = s.find(['e', 'E']) {
        let (mant, exp) = s.split_at(pos);
        if mant.contains('.') {
            let trimmed = mant.trim_end_matches('0');
            let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
            let mut out = String::with_capacity(mant.len() + exp.len());
            out.push_str(trimmed);
            out.push_str(exp);
            out
        } else {
            s
        }
    } else if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
    } else {
        s
    }
}

/// C99 %g: style chosen from the %e exponent X of the value (precision
/// prec-1): %f style with precision prec-1-X when -4 <= X < prec, %e
/// style with precision prec-1 otherwise; trailing fraction zeros dropped
/// unless '#'.
fn format_g(v: f64, prec: usize, upper: bool, hash: bool) -> String {
    if v == 0.0 {
        // Zero never gets the trailing-zero trim (nothing to trim) but the
        // '#' flag keeps a fraction of prec-1 zeros.
        if hash {
            let mut s = String::from("0.");
            for _ in 0..prec.saturating_sub(1) {
                s.push('0');
            }
            s
        } else {
            "0".to_string()
        }
    } else {
        let s_e = format!("{:.*e}", prec.saturating_sub(1), v);
        let exp: i32 = s_e
            .split_once('e')
            .and_then(|(_, e)| e.parse().ok())
            .unwrap_or(0);
        if exp < -4 || exp >= prec as i32 {
            let s = format_exp(v, prec.saturating_sub(1), upper, hash);
            if hash {
                s
            } else {
                trim_g_fraction(s)
            }
        } else {
            let fp = (prec as i32 - 1 - exp).max(0) as usize;
            let s = format!("{:.*}", fp, v);
            if hash {
                if s.contains('.') {
                    s
                } else {
                    format!("{}.", s)
                }
            } else {
                trim_g_fraction(s)
            }
        }
    }
}

// ---------- glob (file pattern matching) ----------

#[cfg(feature = "file")]
pub fn cmd_glob(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("glob", 2, args.len()));
    }

    let mut i = 1;
    let mut nocomplain = false;
    let mut directory: Option<String> = None;
    let mut types: Option<String> = None;
    let mut tails = false;

    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-nocomplain" => { nocomplain = true; i += 1; }
            "-directory" => {
                if i + 1 < args.len() {
                    directory = Some(args[i + 1].as_str().to_string());
                    i += 2;
                } else {
                    return Err(Error::wrong_args("glob", 2, args.len()));
                }
            }
            "-tails" => { tails = true; i += 1; }
            "-types" | "-type" => {
                if i + 1 < args.len() {
                    types = Some(args[i + 1].as_str().to_ascii_lowercase());
                    i += 2;
                } else {
                    return Err(Error::wrong_args("glob", 2, args.len()));
                }
            }
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    // D-26：Tcl glob 语义（弃 glob crate——其 ** 递归 / {a,b} 交替 /
    // 前导点匹配 / 排序输出全部与 tclsh 相悖）：
    //   * 模式段用 string match 语义（* ? [...]）
    //   * `*`/`?`/`[...]` 不匹配前导点（除非模式段字面以 . 开头）
    //   * 目录遍历按 read_dir 序（tclsh 不排序——同文件系统上与
    //     readdir 输出字节对齐）
    //   * 无 ** 递归、无 {a,b} 交替（字面字符）
    fn seg_match(pattern: &str, name: &str) -> bool {
        if name.starts_with('.') && !pattern.starts_with('.') {
            return false;
        }
        crate::interp::util::glob_match(pattern, name)
    }

    fn join(dir: &str, seg: &str) -> String {
        if dir == "/" {
            format!("/{}", seg)
        } else if dir.is_empty() {
            seg.to_string()
        } else {
            format!("{}/{}", dir, seg)
        }
    }

    fn collect_entry(
        host: &dyn crate::host::HostFs,
        path: &str,
        rest: &[&str],
        types: &Option<String>,
        out: &mut Vec<String>,
    ) {
        if rest.is_empty() {
            let is_dir = host.is_dir(path);
            if let Some(ty) = types {
                let want_dir = ty.contains('d');
                let want_file = ty.contains('f');
                if want_dir && !is_dir {
                    return;
                }
                if want_file && is_dir {
                    return;
                }
            }
            out.push(path.to_string());
            return;
        }
        if host.is_dir(path) {
            walk_dir(host, path, rest, types, out);
        }
    }

    fn walk_dir(
        host: &dyn crate::host::HostFs,
        dir: &str,
        segs: &[&str],
        types: &Option<String>,
        out: &mut Vec<String>,
    ) {
        let (seg, rest) = (segs[0], &segs[1..]);
        let literal = !seg.contains('*') && !seg.contains('?') && !seg.contains('[');
        if literal {
            let path = join(dir, seg);
            if seg_match(seg, seg) && host.exists(&path) {
                collect_entry(host, &path, rest, types, out);
            }
            return;
        }
        let read_dir_path = if dir.is_empty() { ".".to_string() } else { dir.to_string() };
        let rd = match host.list_dir(&read_dir_path) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for name in rd {
            if !seg_match(seg, &name) {
                continue;
            }
            let path = join(dir, &name);
            collect_entry(host, &path, rest, types, out);
        }
    }

    let mut results: Vec<Value> = Vec::new();
    for j in i..args.len() {
        let raw_pattern = args[j].as_str();

        let full_pattern = match &directory {
            Some(dir) => format!("{}/{}", dir, raw_pattern),
            None => raw_pattern.to_string(),
        };
        let segs: Vec<&str> = full_pattern.split('/').filter(|s| !s.is_empty()).collect();
        if segs.is_empty() {
            continue;
        }
        let absolute = full_pattern.starts_with('/');
        let mut matched: Vec<String> = Vec::new();
        if absolute {
            walk_dir(interp.host_fs.as_ref(), "/", &segs, &types, &mut matched);
        } else {
            walk_dir(interp.host_fs.as_ref(), "", &segs, &types, &mut matched);
        }
        for p in matched {
            let s = if tails {
                p.rsplit('/').next().unwrap_or(&p).to_string()
            } else {
                p
            };
            results.push(Value::from_str(&s));
        }
    }

    if results.is_empty() && !nocomplain {
        return Err(Error::runtime(
            "no files matched glob patterns",
            crate::error::ErrorCode::NotFound,
        ));
    }

    Ok(Value::from_list_cached(results))
}

#[cfg(not(feature = "file"))]
pub fn cmd_glob(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "glob: not available without 'file' feature",
        crate::error::ErrorCode::InvalidOp,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(all(test, feature = "file"))]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use std::io::Write;

    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_name(prefix: &str) -> String {
        let c = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}_{}_{}", prefix, std::process::id(), c)
    }

    fn make_temp_file() -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(unique_name("rtcl_test"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello").unwrap();
        drop(f);
        path
    }

    fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(unique_name("rtcl_test_dir"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_dir_all(p);
    }

    // ── file existence/type tests ─────────────────────────────────────────────

    #[test]
    fn test_file_exists() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file exists $_p").unwrap();
        assert_eq!(result.as_str(), "1");
        cleanup(&path);
    }

    #[test]
    fn test_file_isfile() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file isfile $_p").unwrap();
        assert_eq!(result.as_str(), "1");
        cleanup(&path);
    }

    #[test]
    fn test_file_isdirectory() {
        let mut interp = Interp::new();
        let path = make_temp_dir();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file isdirectory $_p").unwrap();
        assert_eq!(result.as_str(), "1");
        cleanup(&path);
    }

    #[test]
    fn test_file_type_file() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file type $_p").unwrap();
        assert_eq!(result.as_str(), "file");
        cleanup(&path);
    }

    #[test]
    fn test_file_type_directory() {
        let mut interp = Interp::new();
        let path = make_temp_dir();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file type $_p").unwrap();
        assert_eq!(result.as_str(), "directory");
        cleanup(&path);
    }

    // ── file access tests ─────────────────────────────────────────────────────

    #[test]
    fn test_file_readable() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file readable $_p").unwrap();
        assert_eq!(result.as_str(), "1");
        cleanup(&path);
    }

    #[test]
    fn test_file_writable() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file writable $_p").unwrap();
        assert_eq!(result.as_str(), "1");
        cleanup(&path);
    }

    #[test]
    fn test_file_readable_nonexistent() {
        let mut interp = Interp::new();
        let result = interp.eval("file readable /nonexistent/path/xyz").unwrap();
        assert_eq!(result.as_str(), "0");
    }

    // ── file path manipulation tests ──────────────────────────────────────────

    #[test]
    fn test_file_extension() {
        let mut interp = Interp::new();
        let result = interp.eval("file extension /path/to/file.txt").unwrap();
        assert_eq!(result.as_str(), ".txt");
    }

    #[test]
    fn test_file_extension_no_ext() {
        let mut interp = Interp::new();
        let result = interp.eval("file extension /path/to/file").unwrap();
        assert_eq!(result.as_str(), "");
    }

    #[test]
    fn test_file_tail() {
        let mut interp = Interp::new();
        let result = interp.eval("file tail /path/to/file.txt").unwrap();
        assert_eq!(result.as_str(), "file.txt");
    }

    #[test]
    fn test_file_dirname() {
        let mut interp = Interp::new();
        let result = interp.eval("file dirname /path/to/file.txt").unwrap();
        assert_eq!(result.as_str(), "/path/to");
    }

    #[test]
    fn test_file_rootname() {
        let mut interp = Interp::new();
        let result = interp.eval("file rootname /path/to/file.txt").unwrap();
        assert_eq!(result.as_str(), "/path/to/file");
    }

    #[test]
    fn test_file_split() {
        let mut interp = Interp::new();
        let result = interp.eval("file split /a/b/c").unwrap();
        // Should return a list
        assert!(result.as_str().contains("a"));
    }

    #[test]
    fn test_file_join() {
        let mut interp = Interp::new();
        let result = interp.eval("file join a b c").unwrap();
        // Result depends on OS separator
        assert!(!result.as_str().is_empty());
    }

    #[test]
    fn test_file_separator() {
        let mut interp = Interp::new();
        let result = interp.eval("file separator").unwrap();
        // Should be "/" on Unix, "\\" on Windows
        #[cfg(unix)]
        assert_eq!(result.as_str(), "/");
        #[cfg(windows)]
        assert_eq!(result.as_str(), "\\");
    }

    // ── file metadata tests ───────────────────────────────────────────────────

    #[test]
    fn test_file_size() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file size $_p").unwrap();
        assert_eq!(result.as_str(), "5"); // "hello" is 5 bytes
        cleanup(&path);
    }

    #[test]
    fn test_file_atime() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file atime $_p").unwrap();
        // Should be a unix timestamp (positive integer)
        let ts: i64 = result.as_str().parse().unwrap();
        assert!(ts > 0);
        cleanup(&path);
    }

    #[test]
    fn test_file_mtime() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file mtime $_p").unwrap();
        // Should be a unix timestamp (positive integer)
        let ts: i64 = result.as_str().parse().unwrap();
        assert!(ts > 0);
        cleanup(&path);
    }

    #[test]
    fn test_file_stat() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("file stat $_p mystat").unwrap();
        // Check that array elements were set using set var syntax
        let size = interp.eval("set mystat(size)").unwrap();
        assert_eq!(size.as_str(), "5");
        let ftype = interp.eval("set mystat(type)").unwrap();
        assert_eq!(ftype.as_str(), "file");
        cleanup(&path);
    }

    // ── file operations tests ─────────────────────────────────────────────────

    #[test]
    fn test_file_mkdir() {
        let mut interp = Interp::new();
        let dir = std::env::temp_dir().join(unique_name("rtcl_mkdir_test"));
        interp.set_var("_d", Value::from_str(&dir.to_string_lossy())).unwrap();
        interp.eval("file mkdir $_d").unwrap();
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_file_delete() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("file delete $_p").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_file_delete_force() {
        let mut interp = Interp::new();
        let dir = make_temp_dir();
        // Create a file inside the directory
        let file = dir.join("inner.txt");
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(b"inner").unwrap();

        interp.set_var("_d", Value::from_str(&dir.to_string_lossy())).unwrap();
        interp.eval("file delete -force $_d").unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn test_file_rename() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        let new_path = path.with_extension("renamed");
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.set_var("_q", Value::from_str(&new_path.to_string_lossy())).unwrap();
        interp.eval("file rename $_p $_q").unwrap();
        assert!(!path.exists());
        assert!(new_path.exists());
        cleanup(&new_path);
    }

    #[test]
    fn test_file_copy() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        let new_path = path.with_extension("copied");
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.set_var("_q", Value::from_str(&new_path.to_string_lossy())).unwrap();
        interp.eval("file copy $_p $_q").unwrap();
        assert!(path.exists());
        assert!(new_path.exists());
        cleanup(&path);
        cleanup(&new_path);
    }

    #[test]
    fn test_file_tempfile() {
        let mut interp = Interp::new();
        let result = interp.eval("file tempfile").unwrap();
        let path = std::path::Path::new(result.as_str());
        assert!(path.exists());
        cleanup(path);
    }

    #[test]
    fn test_file_owned() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file owned $_p").unwrap();
        // Should be 1 since we created the file
        assert_eq!(result.as_str(), "1");
        cleanup(&path);
    }

    #[test]
    fn test_file_normalize() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        let result = interp.eval("file normalize $_p").unwrap();
        // Normalized path should be absolute - check it exists and is not empty
        let normalized = result.as_str();
        assert!(!normalized.is_empty());
        // On Windows, it should contain a drive letter like "C:"
        // On Unix, it should start with '/'
        #[cfg(unix)]
        assert!(normalized.starts_with('/'));
        #[cfg(windows)]
        assert!(normalized.contains(':'));
        cleanup(&path);
    }

    // ── file lstat tests ──────────────────────────────────────────────────────

    #[test]
    fn test_file_lstat() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("file lstat $_p st").unwrap();
        let size = interp.eval("set st(size)").unwrap();
        assert_eq!(size.as_str(), "5");
        let ftype = interp.eval("set st(type)").unwrap();
        assert_eq!(ftype.as_str(), "file");
        let mtime = interp.eval("set st(mtime)").unwrap();
        let ts: i64 = mtime.as_str().parse().unwrap();
        assert!(ts > 0);
        cleanup(&path);
    }

    // ── file copy -force / rename -force tests ────────────────────────────────

    #[test]
    fn test_file_copy_no_force_existing_target() {
        let mut interp = Interp::new();
        let src = make_temp_file();
        let dst = src.with_extension("copy_target");
        std::fs::write(&dst, b"existing").unwrap();
        interp.set_var("_s", Value::from_str(&src.to_string_lossy())).unwrap();
        interp.set_var("_d", Value::from_str(&dst.to_string_lossy())).unwrap();
        // Without -force, should fail when target exists
        let result = interp.eval("file copy $_s $_d");
        assert!(result.is_err());
        cleanup(&src);
        cleanup(&dst);
    }

    #[test]
    fn test_file_copy_force_overwrites() {
        let mut interp = Interp::new();
        let src = make_temp_file();
        let dst = src.with_extension("copy_force_target");
        std::fs::write(&dst, b"old content").unwrap();
        interp.set_var("_s", Value::from_str(&src.to_string_lossy())).unwrap();
        interp.set_var("_d", Value::from_str(&dst.to_string_lossy())).unwrap();
        interp.eval("file copy -force $_s $_d").unwrap();
        let content = std::fs::read_to_string(&dst).unwrap();
        assert_eq!(content, "hello");
        cleanup(&src);
        cleanup(&dst);
    }

    #[test]
    fn test_file_rename_no_force_existing_target() {
        let mut interp = Interp::new();
        let src = make_temp_file();
        let dst = src.with_extension("rename_target");
        std::fs::write(&dst, b"existing").unwrap();
        interp.set_var("_s", Value::from_str(&src.to_string_lossy())).unwrap();
        interp.set_var("_d", Value::from_str(&dst.to_string_lossy())).unwrap();
        let result = interp.eval("file rename $_s $_d");
        assert!(result.is_err());
        cleanup(&src);
        cleanup(&dst);
    }

    #[test]
    fn test_file_rename_force_overwrites() {
        let mut interp = Interp::new();
        let src = make_temp_file();
        let dst = src.with_extension("rename_force_target");
        std::fs::write(&dst, b"old").unwrap();
        interp.set_var("_s", Value::from_str(&src.to_string_lossy())).unwrap();
        interp.set_var("_d", Value::from_str(&dst.to_string_lossy())).unwrap();
        interp.eval("file rename -force $_s $_d").unwrap();
        assert!(!src.exists());
        let content = std::fs::read_to_string(&dst).unwrap();
        assert_eq!(content, "hello");
        cleanup(&dst);
    }

    // ── file link tests ───────────────────────────────────────────────────────

    #[test]
    fn test_file_link_hard() {
        let mut interp = Interp::new();
        let src = make_temp_file();
        let link_path = src.with_extension("hardlink");
        interp.set_var("_link", Value::from_str(&link_path.to_string_lossy())).unwrap();
        interp.set_var("_src", Value::from_str(&src.to_string_lossy())).unwrap();
        interp.eval("file link -hard $_link $_src").unwrap();
        assert!(link_path.exists());
        let content = std::fs::read_to_string(&link_path).unwrap();
        assert_eq!(content, "hello");
        cleanup(&src);
        cleanup(&link_path);
    }

    // ── file mkdir multiple directories ───────────────────────────────────────

    #[test]
    fn test_file_mkdir_multiple() {
        let mut interp = Interp::new();
        let base = std::env::temp_dir();
        let d1 = base.join(unique_name("rtcl_mkd1"));
        let d2 = base.join(unique_name("rtcl_mkd2"));
        interp.set_var("_d1", Value::from_str(&d1.to_string_lossy())).unwrap();
        interp.set_var("_d2", Value::from_str(&d2.to_string_lossy())).unwrap();
        interp.eval("file mkdir $_d1 $_d2").unwrap();
        assert!(d1.is_dir());
        assert!(d2.is_dir());
        let _ = std::fs::remove_dir(&d1);
        let _ = std::fs::remove_dir(&d2);
    }

    // ── file delete multiple files ────────────────────────────────────────────

    #[test]
    fn test_file_delete_multiple() {
        let mut interp = Interp::new();
        let p1 = make_temp_file();
        let p2 = p1.with_extension("del2");
        std::fs::write(&p2, b"x").unwrap();
        interp.set_var("_a", Value::from_str(&p1.to_string_lossy())).unwrap();
        interp.set_var("_b", Value::from_str(&p2.to_string_lossy())).unwrap();
        interp.eval("file delete $_a $_b").unwrap();
        assert!(!p1.exists());
        assert!(!p2.exists());
    }

    // ── file extension edge cases ─────────────────────────────────────────────

    #[test]
    fn test_file_extension_double_dot() {
        let mut interp = Interp::new();
        let result = interp.eval("file extension /path/file.tar.gz").unwrap();
        assert_eq!(result.as_str(), ".gz");
    }

    #[test]
    fn test_file_extension_hidden_file() {
        let mut interp = Interp::new();
        let result = interp.eval("file extension /path/.hidden").unwrap();
        // On most platforms, .hidden has no extension — the whole thing is the stem
        // Rust treats ".hidden" as no extension
        assert_eq!(result.as_str(), "");
    }

    // ── file rootname edge cases ──────────────────────────────────────────────

    #[test]
    fn test_file_rootname_no_ext() {
        let mut interp = Interp::new();
        let result = interp.eval("file rootname myfile").unwrap();
        assert_eq!(result.as_str(), "myfile");
    }

    // ── file bad subcommand ───────────────────────────────────────────────────

    #[test]
    fn test_file_bad_subcommand() {
        let mut interp = Interp::new();
        let result = interp.eval("file bogus /path");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("bad option"));
    }

    // ── file stat array fields ────────────────────────────────────────────────

    #[test]
    fn test_file_stat_atime_field() {
        let mut interp = Interp::new();
        let path = make_temp_file();
        interp.set_var("_p", Value::from_str(&path.to_string_lossy())).unwrap();
        interp.eval("file stat $_p fs").unwrap();
        let atime = interp.eval("set fs(atime)").unwrap();
        let ts: i64 = atime.as_str().parse().unwrap();
        assert!(ts > 0);
        cleanup(&path);
    }

    // ── glob tests ────────────────────────────────────────────────────────────

    #[test]
    fn test_glob_star() {
        let dir = make_temp_dir();
        std::fs::File::create(dir.join("alpha.txt")).unwrap();
        std::fs::File::create(dir.join("beta.txt")).unwrap();
        std::fs::File::create(dir.join("gamma.rs")).unwrap();
        let mut interp = Interp::new();
        let dstr = dir.to_string_lossy().replace('\\', "/");
        let result = interp.eval(&format!("glob -directory {{{}}} -- *.txt", dstr)).unwrap();
        let items = result.as_list().unwrap();
        assert_eq!(items.len(), 2);
        cleanup(&dir);
    }

    #[test]
    fn test_glob_double_star_recursive() {
        let dir = make_temp_dir();
        let sub = dir.join("sub");
        let sub2 = sub.join("deep");
        std::fs::create_dir_all(&sub2).unwrap();
        std::fs::File::create(dir.join("top.rs")).unwrap();
        std::fs::File::create(sub.join("mid.rs")).unwrap();
        std::fs::File::create(sub2.join("low.rs")).unwrap();
        std::fs::File::create(sub2.join("low.txt")).unwrap();
        let mut interp = Interp::new();
        let dstr = dir.to_string_lossy().replace('\\', "/");
        let result = interp.eval(&format!("glob -nocomplain -directory {{{}}} -- **/*.rs", dstr)).unwrap();
        let items = result.as_list().unwrap();
        // D-26：tclsh 的 ** 只是两个 *（无递归）——**/*.rs 命中
        // sub/mid.rs 一项（tclsh 8.6.17 实测同形）。
        assert_eq!(items.len(), 1, "got: {:?}", items.iter().map(|v| v.as_str()).collect::<Vec<_>>());
        cleanup(&dir);
    }

    #[test]
    fn test_glob_nocomplain_empty() {
        let dir = make_temp_dir();
        let mut interp = Interp::new();
        let dstr = dir.to_string_lossy().replace('\\', "/");
        let result = interp.eval(&format!("glob -nocomplain -directory {{{}}} -- *.nonexistent", dstr)).unwrap();
        assert_eq!(result.as_str(), "");
        cleanup(&dir);
    }

    #[test]
    fn test_glob_error_no_match() {
        let dir = make_temp_dir();
        let mut interp = Interp::new();
        let dstr = dir.to_string_lossy().replace('\\', "/");
        let result = interp.eval(&format!("glob -directory {{{}}} -- *.nonexistent", dstr));
        assert!(result.is_err());
        cleanup(&dir);
    }
}


// ── encoding 命令（D-29 宽表）──────────────────────────────────────

/// 字节串（每字符一字节）→ Vec<u8>。
fn byte_str_to_bytes(data: &str) -> Vec<u8> {
    data.chars().map(|c| (c as u32 & 0xFF) as u8).collect()
}

/// Vec<u8> → 字节串（每字节一个 0..255 的字符）。
fn bytes_to_byte_str(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

pub fn cmd_encoding(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "encoding", 2, args.len(),
            "subcommand ?arg ...?",
        ));
    }
    let resolve = |w: &str| -> Result<&'static str> {
        const SUBS: &[&str] = &["convertfrom", "convertto", "dirs", "names", "system"];
        let hits: Vec<&str> = SUBS.iter().filter(|s| s.starts_with(w)).copied().collect();
        match hits.len() {
            1 => Ok(hits[0]),
            _ => {
                let joined = match SUBS.len() {
                    1 => SUBS[0].to_string(),
                    2 => format!("{} or {}", SUBS[0], SUBS[1]),
                    _ => format!(
                        "{}, or {}",
                        SUBS[..SUBS.len() - 1].join(", "),
                        SUBS[SUBS.len() - 1]
                    ),
                };
                Err(Error::Msg(format!(
                    "unknown or ambiguous subcommand \"{}\": must be {}",
                    w,
                    joined
                )))
            }
        }
    };
    match resolve(args[1].as_str())? {
        "names" => Ok(Value::from_list(
            &crate::encoding::NAMES
                .iter()
                .map(|e| Value::from_str(e))
                .collect::<Vec<_>>(),
        )),
        "system" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "encoding system", 2, args.len(), "?encodingName?",
                ));
            }
            if args.len() == 3 {
                let name = args[2].as_str();
                if !crate::encoding::known(name) {
                    super::list::set_error_code(
                        interp,
                        &format!("TCL LOOKUP ENCODING {}", name),
                    );
                    return Err(Error::Msg(format!("unknown encoding \"{}\"", name)));
                }
                interp.channels.system_encoding = name.to_string();
                return Ok(Value::empty());
            }
            Ok(Value::from_str(&interp.channels.system_encoding.clone()))
        }
        "dirs" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "encoding dirs", 2, args.len(), "?dirList?",
                ));
            }
            if args.len() == 3 {
                interp.channels.encoding_dirs = args[2].as_str().to_string();
                // tclsh 的 encoding dirs 是进程级搜索路径：同步 .enc 表
                // 后端并清空已加载缓存。
                let list = Value::from_str(args[2].as_str())
                    .as_list()
                    .unwrap_or_else(|| vec![args[2].clone()]);
                let dirs: Vec<String> =
                    list.iter().map(|v| v.as_str().to_string()).collect();
                crate::encoding::set_table_dirs(&dirs);
                return Ok(Value::empty());
            }
            Ok(Value::from_list(&[Value::from_str(
                &interp.channels.encoding_dirs.clone(),
            )]))
        }
        "convertfrom" | "convertto" => {
            let from = args[1].as_str().ends_with("from");
            if args.len() < 4 || args.len() > 4 {
                return Err(Error::wrong_args_with_usage(
                    if from { "encoding convertfrom" } else { "encoding convertto" },
                    4, args.len(),
                    "encodingName data",
                ));
            }
            let name = args[2].as_str();
            if !crate::encoding::known(name) {
                super::list::set_error_code(
                    interp,
                    &format!("TCL LOOKUP ENCODING {}", name),
                );
                return Err(Error::Msg(format!("unknown encoding \"{}\"", name)));
            }
            let data = args[3].as_str();
            if from {
                let bytes = byte_str_to_bytes(data);
                Ok(Value::from_str(&crate::encoding::decode(&bytes, name)))
            } else {
                let bytes = crate::encoding::encode(data, name);
                Ok(Value::from_str(&bytes_to_byte_str(&bytes)))
            }
        }
        _ => unreachable!(),
    }
}
