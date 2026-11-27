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
        1 => ("operation not permitted", "EPERM"),
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
        104 => ("connection reset by peer", "ECONNRESET"),
        107 => ("socket is not connected", "ENOTCONN"),
        110 => ("connection timed out", "ETIMEDOUT"),
        111 => ("connection refused", "ECONNREFUSED"),
        98 => ("address already in use", "EADDRINUSE"),
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
    // Rust 的 getaddrinfo 包装文案去掉前缀后是 gai_strerror 原文——
    // 保持原大小写（tclsh 报 "Name or service not known"，首字母大写）。
    if let Some(rest) = s.strip_prefix("failed to lookup address information: ") {
        return rest.to_string();
    }
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
    // lazy-string（§6）：消息值保持未物化引用——wasm stdout 快路径直取
    // 宿主形态；其余路径在使用点 `as_str()` 物化（ choke point 兜底）。
    let msg_val: &Value;

    match args.len() {
        2 => {
            msg_val = &args[1];
        }
        3 => {
            let first = args[1].as_str();
            if first == "-nonewline" {
                nonewline = true;
                msg_val = &args[2];
            } else {
                // first is channelId
                chan_id = first;
                msg_val = &args[2];
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
            msg_val = &args[3];
        }
        _ => return Err(Error::wrong_args_with_usage("puts", 2, args.len(), "?-nonewline? ?channelId? string")),
    }

    // ── 非 io 构建：宿主控制台路径 ─────────────────────────────────
    // stdout/stderr 走 HostConsole 服务（原生 stdio，或 wasm 宿主经
    // set_console 注入的 JS 实现）；stdin 不可写；其余通道视为不存在。
    #[cfg(all(feature = "std", not(feature = "io")))]
    {
        let newline = if nonewline { "" } else { "\n" };
        let out = format!("{}{}", msg_val.as_str(), newline);
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
                let r = if chan_id == "stdout" {
                    // lazy-string 白名单（§6「puts 到 console」）：宿主形
                    // 态值直达 console（零转码，JsConsole 实现消费
                    // HostStr 句柄）；None 回退到物化字符串路径。
                    match interp.console.write_value_stdout(msg_val, !nonewline) {
                        Some(r) => r,
                        None => interp
                            .console
                            .write_stdout(&format!("{}{}", msg_val.as_str(), newline)),
                    }
                } else {
                    interp
                        .console
                        .write_stderr(&format!("{}{}", msg_val.as_str(), newline))
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

        let msg = msg_val.as_str();

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

        if !interp.channels.get(chan_id).map(|ch| ch.is_writable()).unwrap_or(false) {
            return Err(chan_io::not_opened(interp, chan_id, false));
        }
        if chan_io::has_copy_job(interp, chan_id) {
            // tclsh：后台拷贝占用期间 puts → file busy（EC POSIX EBUSY）。
            return Err(chan_io::copy_busy_error(interp, chan_id, false));
        }

        let (ch, cfg) = interp.channels.channel_ctx_mut(chan_id)
            .ok_or_else(|| Error::runtime(
                format!("can not find channel named \"{}\"", chan_id),
                crate::error::ErrorCode::Io,
            ))?;

        let defer = ch.flush_deferred();
        let wr = crate::channel::channel_write_str_cfg(ch, cfg, &out);
        let fl = if wr.is_ok() && !defer { ch.flush().err() } else { None };
        drop(ch);
        drop(cfg);
        // tclsh 形：`error writing "<chan>": <strerror>` /
        // `error flushing "<chan>": <strerror>` + POSIX errorCode。
        wr.map_err(|e| posix_error(interp, format!("error writing \"{}\"", chan_id), &e))?;
        if let Some(e) = fl {
            return Err(posix_error(interp, format!("error flushing \"{}\"", chan_id), &e));
        }

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
    // still parses the whole script by design).  Sourced files run in the
    // file-execution context (TclEvalEx per-command semantics — the
    // file-level if/while/for errorInfo frames).
    interp.arm_file_script();
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
        // C33（9.1 实测钉形）：`file attributes name ?-option value ...?`、
        // `file tildeexpand path`、`file home ?user?`——各自的 arity 与
        // 用法文本独立于通用路径检查。
        "attributes" => return file_attributes(interp, args),
        "tildeexpand" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "file tildeexpand", 3, args.len(), "path",
                ));
            }
            return file_tildeexpand(interp, args[2].as_str());
        }
        "home" => {
            let user = match args.len() {
                2 => None,
                3 => Some(args[2].as_str()),
                _ => {
                    return Err(Error::wrong_args_with_usage(
                        "file home", 2, args.len(), "?user?",
                    ))
                }
            };
            return user_home_dir(interp, user).map(|h| Value::from_str(&h));
        }
        // Nothing named is nothing to delete (tclsh: no error).
        "delete" if args.len() == 2 => return Ok(Value::empty()),
        "rename" | "copy" if args.len() == 2 => {
            return Err(Error::wrong_args_with_usage(
                format!("file {}", args[1].as_str()),
                3,
                args.len(),
                "?-option value ...? source ?source ...? target",
            ));
        }
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
            let cwd = interp.host_fs.pwd();
            file_normalize(&cwd, path)
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
                interp.host_fs.set_times(path, None, Some(secs)).map_err(|e| {
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
            let s = interp.host_fs.stat_full(path, subcmd != "lstat").map_err(|e| {
                posix_error(interp, format!("could not read \"{}\"", path), &e)
            })?;
            let ftype = if s.is_link { "link" }
                else if s.is_dir { "directory" }
                else { "file" };

            interp.set_var(&format!("{}(size)", var_name), Value::from_int(s.size))?;
            interp.set_var(&format!("{}(mtime)", var_name), Value::from_int(s.mtime))?;
            interp.set_var(&format!("{}(atime)", var_name), Value::from_int(s.atime))?;
            interp.set_var(&format!("{}(type)", var_name), Value::from_str(ftype))?;

            // D-33：13 键全形——dev/ino/…/ctime 从 FsStat 取（NativeFs
            // unix 填真值与路由前逐字一致；宿主实现合成自洽值或如实
            // 报错——tclsh 对一切文件系统同样如此）。
            interp.set_var(&format!("{}(dev)", var_name), Value::from_int(s.dev))?;
            interp.set_var(&format!("{}(ino)", var_name), Value::from_int(s.ino))?;
            interp.set_var(&format!("{}(nlink)", var_name), Value::from_int(s.nlink))?;
            interp.set_var(&format!("{}(uid)", var_name), Value::from_int(s.uid))?;
            interp.set_var(&format!("{}(gid)", var_name), Value::from_int(s.gid))?;
            interp.set_var(&format!("{}(mode)", var_name), Value::from_int(s.mode))?;
            interp.set_var(&format!("{}(blksize)", var_name), Value::from_int(s.blksize))?;
            interp.set_var(&format!("{}(blocks)", var_name), Value::from_int(s.blocks))?;
            interp.set_var(&format!("{}(ctime)", var_name), Value::from_int(s.ctime))?;
            Ok(Value::empty())
        }
        "readlink" => {
            // tclsh 原文形状：could not read link "x": <strerror 小写>。
            let target = interp.host_fs.read_link(path).map_err(|e| {
                let text = super::io::io_error_text(&e, "invalid argument");
                if let Some(code) = e.raw_os_error() {
                    super::list::set_error_code(
                        interp,
                        &format!("POSIX ERRNO {{{}}}", code),
                    );
                }
                Error::runtime(
                    format!("could not read link \"{}\": {}", path, text),
                    crate::error::ErrorCode::Io,
                )
            })?;
            Ok(Value::from_str(&target))
        }

        // ── file operations ─────────────────────────────────────────
        "delete" => {
            // file delete ?-force? ?--? path ...
            let mut force = false;
            let mut start = 2;
            let options: Vec<String> = ["-force", "--"].iter().map(|o| o.to_string()).collect();
            while start < args.len() {
                let word = args[start].as_str();
                if !word.starts_with('-') {
                    break;
                }
                match super::misc::index_in_table(word, &options, "option", false) {
                    Ok(0) => {
                        force = true;
                        start += 1;
                    }
                    Ok(_) => {
                        start += 1;
                        break;
                    }
                    Err((e, code)) => {
                        super::list::set_error_code(interp, &code);
                        return Err(e);
                    }
                }
            }
            for j in start..args.len() {
                let p = args[j].as_str();
                // tclsh: a path that does not exist is no error.
                if interp.host_fs.stat_full(p, false).is_err() {
                    continue;
                }
                // Without -force a directory keeps its contents: a
                // non-empty one is an error, nothing in it is touched.
                // (The whole tree used to be removed.)
                let is_link = interp.host_fs.read_link(p).is_ok();
                if !force
                    && !is_link
                    && interp.host_fs.is_dir(p)
                    && !interp.host_fs.list_dir(p).unwrap_or_default().is_empty()
                {
                    super::list::set_error_code(interp, "POSIX EEXIST {file already exists}");
                    return Err(Error::runtime(
                        format!("error deleting \"{}\": directory not empty", p),
                        crate::error::ErrorCode::Generic,
                    ));
                }
                interp.host_fs.delete(p).map_err(|e| {
                    posix_error(interp, format!("error deleting \"{}\"", p), &e)
                })?;
            }
            Ok(Value::empty())
        }
        "mkdir" => {
            for j in 2..args.len() {
                let p = args[j].as_str();
                interp
                    .host_fs
                    .mkdir(p, true)
                    .map_err(|e| posix_error(interp, format!("can't create directory \"{}\"", p), &e))?;
            }
            Ok(Value::empty())
        }
        "rename" => {
            // file rename ?-force? ?--? source target
            let (force, start) = file_force_options(interp, args)?;
            if args.len() - start != 2 {
                return Err(Error::wrong_args_with_usage(
                    "file rename",
                    3,
                    args.len(),
                    "?-option value ...? source ?source ...? target",
                ));
            }
            let src = args[start].as_str();
            // A directory as the target receives the source under its
            // own name (tclsh: `file rename f dir` makes `dir/f`).
            let given = args[start + 1].as_str();
            let dst = if interp.host_fs.is_dir(given) {
                format!(
                    "{}/{}",
                    given.trim_end_matches('/'),
                    src.rsplit('/').next().unwrap_or(src)
                )
            } else {
                given.to_string()
            };
            // tclsh names the source alone when the file is not there.
            if interp.host_fs.stat_full(src, false).is_err() {
                return Err(posix_error(
                    interp,
                    format!("error renaming \"{}\"", src),
                    &std::io::Error::from_raw_os_error(libc_enoent()),
                ));
            }
            if !force && interp.host_fs.exists(&dst) {
                super::list::set_error_code(interp, "POSIX EEXIST {file already exists}");
                return Err(Error::runtime(
                    format!("error renaming \"{}\" to \"{}\": file already exists", src, dst),
                    crate::error::ErrorCode::Generic,
                ));
            }
            interp
                .host_fs
                .rename(src, &dst)
                .map_err(|e| posix_error(interp, format!("error renaming \"{}\" to \"{}\"", src, dst), &e))?;
            Ok(Value::empty())
        }
        "copy" => {
            // file copy ?-force? ?--? source target
            let (force, start) = file_force_options(interp, args)?;
            if args.len() - start != 2 {
                return Err(Error::wrong_args_with_usage(
                    "file copy",
                    3,
                    args.len(),
                    "?-option value ...? source ?source ...? target",
                ));
            }
            let src = args[start].as_str();
            let dst = args[start + 1].as_str();
            if interp.host_fs.stat_full(src, false).is_err() {
                return Err(posix_error(
                    interp,
                    format!("error copying \"{}\"", src),
                    &std::io::Error::from_raw_os_error(libc_enoent()),
                ));
            }
            if !force && interp.host_fs.exists(dst) {
                super::list::set_error_code(interp, "POSIX EEXIST {file already exists}");
                return Err(Error::runtime(
                    // tclsh 消息形状（D-33）：error copying "src" to
                    // "dst": file already exists。
                    format!("error copying \"{}\" to \"{}\": file already exists", src, dst),
                    crate::error::ErrorCode::Generic,
                ));
            }
            interp
                .host_fs
                .copy(src, dst)
                .map_err(|e| posix_error(interp, format!("error copying \"{}\" to \"{}\"", src, dst), &e))?;
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
                "bad option \"{}\": must be atime, attributes, copy, delete, \
                 dirname, executable, exists, extension, home, isdirectory, \
                 isfile, join, link, lstat, mkdir, mtime, normalize, owned, \
                 readable, readlink, rename, rootname, separator, size, \
                 split, stat, tail, tempfile, tildeexpand, type, or writable",
                subcmd
            ),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

/// `file tempfile ?nameVar? ?template?` — a fresh file and a read-write
/// channel on it; `nameVar` gets the path (tclsh 8.6.17).
///
/// The file is created exclusively and readable by its owner only.  The
/// name used to be `<prefix><nanosecond stamp>` created with plain
/// `create`: anyone who could guess the stamp could plant a file there
/// first (a symlink to something else) and have it written, and it was
/// world-readable.
#[cfg(feature = "file")]
fn file_tempfile(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "file tempfile",
            2,
            args.len() - 1,
            "?nameVar? ?template?",
        ));
    }
    let mut path = std::path::PathBuf::new();
    let template = args.get(3).map(|v| v.as_str());
    let mut make_name = || match template {
        None => std::env::temp_dir().join(format!("tcl_{}", random_suffix())),
        Some(t) => tempfile_name(t),
    };
    let mut created = None;
    // The name only has to vary: whether the file is this one is the
    // file system's answer to an exclusive create.
    for _ in 0..1000 {
        let candidate = make_name();
        let mut open = std::fs::OpenOptions::new();
        open.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600);
        }
        match open.open(&candidate) {
            Ok(f) => {
                created = Some(f);
                path = candidate;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(posix_error(
                    interp,
                    "can't create temporary file".to_string(),
                    &e,
                ));
            }
        }
    }
    let _ = created;
    // Opened through the channel table, like any `open`: one place for
    // the naming and the default configuration.
    let id = interp.channels.open_file(&path.to_string_lossy(), "r+").map_err(|e| {
        posix_error(interp, "can't create temporary file".to_string(), &e)
    })?;
    if let Some(var) = args.get(2) {
        let var = var.as_str();
        if !var.is_empty() {
            interp.set_var(var, Value::from_str(&path.to_string_lossy()))?;
        }
    }
    Ok(Value::from_str(&id))
}

/// tclsh's form of a temporary file's name.  A bare name goes into the
/// temporary directory; a template with a directory part is taken as it
/// stands (absolute, or against the current directory).  `_` and six
/// random characters go before the extension — the template's own `X`s
/// are not substituted (`pre` → `/tmp/pre_jojzKs`, `a.bXc.d` →
/// `a.bXc_qdxLoE.d`).
#[cfg(feature = "file")]
fn tempfile_name(template: &str) -> std::path::PathBuf {
    use std::path::Path;
    let full = Path::new(template);
    let full = if full.components().count() == 1 {
        std::env::temp_dir().join(full)
    } else {
        full.to_path_buf()
    };
    let s = full.to_string_lossy();
    let stem_end = s.rfind('/').map(|i| i + 1).unwrap_or(0);
    let split = s[stem_end..]
        .rfind('.')
        .filter(|&i| i > 0)
        .map(|i| stem_end + i)
        .unwrap_or(s.len());
    std::path::PathBuf::from(format!(
        "{}_{}{}",
        &s[..split],
        random_suffix(),
        &s[split..]
    ))
}

/// Six characters of `[A-Za-z0-9]`.  Uniqueness is the file system's to
/// guarantee (the file is created exclusively); this only has to vary.
#[cfg(feature = "file")]
fn random_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut x = STATE.load(Ordering::Relaxed);
    if x == 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        x = nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x |= 1;
    }
    const ALPHABET: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut out = String::with_capacity(6);
    for _ in 0..6 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push(ALPHABET[(x % 62) as usize] as char);
    }
    STATE.store(x, Ordering::Relaxed);
    out
}

/// tclsh's `file normalize`: lexical.  The path is made absolute against
/// `cwd`, `.` and `..` are resolved and repeated separators collapsed —
/// nothing is asked of the file system, so a path that does not exist
/// normalizes too.  A leading `~` stays the literal component it is
/// (Tcl 9 no longer interprets `~` in pathnames; expansion is
/// `file tildeexpand`'s job).
#[cfg(feature = "file")]
fn file_normalize(cwd: &str, path: &str) -> Result<Value> {
    if path.is_empty() {
        return Ok(Value::empty());
    }
    let mut path = path.to_string();
    // Tcl 9 起路径名里的 `~` 不再展开（9.0 不兼容清单："Ended
    // interpretation of `~` as home directory in pathnames"）——`~` 与
    // `~user` 是普通路径分量；展开只有 `file tildeexpand`（与
    // `file home ?user?`）做，见 `file_tildeexpand`。
    let full = if path.starts_with('/') || cwd.is_empty() {
        path
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), path)
    };
    let absolute = full.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for component in full.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() && *parts.last().unwrap() != ".." {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    Ok(Value::from_str(&if absolute {
        format!("/{}", joined)
    } else {
        joined
    }))
}

// ── file attributes / tildeexpand / home（C33，按 9.1 实测钉形）─────────

/// unix 用户/组查询与 chown 的最小 C 投影：rtcl-vm 不直接依赖 libc
/// crate（file 特性不拉它），extern 声明与 host.rs 的 getuid 同法。
#[cfg(all(unix, feature = "file"))]
mod pw {
    use std::ffi::c_char;

    /// glibc/musl 的 `struct passwd`（只用 pw_uid / pw_dir）。
    #[repr(C)]
    pub struct Passwd {
        pub pw_name: *mut c_char,
        pub pw_passwd: *mut c_char,
        pub pw_uid: u32,
        pub pw_gid: u32,
        pub pw_gecos: *mut c_char,
        pub pw_dir: *mut c_char,
        pub pw_shell: *mut c_char,
    }

    /// glibc/musl 的 `struct group`（只用 gr_name / gr_gid）。
    #[repr(C)]
    pub struct Group {
        pub gr_name: *mut c_char,
        pub gr_passwd: *mut c_char,
        pub gr_gid: u32,
        pub gr_mem: *mut *mut c_char,
    }

    extern "C" {
        pub fn getpwnam(name: *const c_char) -> *mut Passwd;
        pub fn getpwuid(uid: u32) -> *mut Passwd;
        pub fn getgrnam(name: *const c_char) -> *mut Group;
        pub fn getgrgid(gid: u32) -> *mut Group;
        pub fn chown(path: *const c_char, owner: u32, group: u32) -> i32;
    }

    /// NUL 结尾的 Rust 串 → C 串的临时视图（查询函数不保存指针）。
    pub(crate) fn cstr(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap_or_default()
    }

    pub(crate) fn ptr_str(p: *const c_char) -> Option<String> {
        if p.is_null() {
            return None;
        }
        unsafe { std::ffi::CStr::from_ptr(p) }.to_str().ok().map(|s| s.to_string())
    }
}

/// 名字 → uid（getpwnam）；找不到 = None。非 unix 无此概念。
#[cfg(all(unix, feature = "file"))]
fn uid_of(name: &str) -> Option<u32> {
    let c = pw::cstr(name);
    let p = unsafe { pw::getpwnam(c.as_ptr()) };
    if p.is_null() {
        None
    } else {
        Some(unsafe { (*p).pw_uid })
    }
}

/// 非 unix（wasm）存根：没有用户/组数据库——名字查询一律 None（取值
/// 回退数字，设置一律 NO_USER），数值 uid/gid 由 chown_one 的非 unix
/// 分支报 EPERM。
#[cfg(all(not(unix), feature = "file"))]
fn uid_of(_name: &str) -> Option<u32> {
    None
}
#[cfg(all(not(unix), feature = "file"))]
fn gid_of(_name: &str) -> Option<u32> {
    None
}
#[cfg(all(not(unix), feature = "file"))]
fn name_of_uid(_uid: u32) -> Option<String> {
    None
}
#[cfg(all(not(unix), feature = "file"))]
fn name_of_gid(_gid: u32) -> Option<String> {
    None
}

/// uid → 名字（getpwuid）；无 passwd 条目 = None（tclsh 回退数字）。
#[cfg(all(unix, feature = "file"))]
fn name_of_uid(uid: u32) -> Option<String> {
    let p = unsafe { pw::getpwuid(uid) };
    if p.is_null() {
        None
    } else {
        pw::ptr_str(unsafe { (*p).pw_name })
    }
}

#[cfg(all(unix, feature = "file"))]
fn gid_of(name: &str) -> Option<u32> {
    let c = pw::cstr(name);
    let p = unsafe { pw::getgrnam(c.as_ptr()) };
    if p.is_null() {
        None
    } else {
        Some(unsafe { (*p).gr_gid })
    }
}

#[cfg(all(unix, feature = "file"))]
fn name_of_gid(gid: u32) -> Option<String> {
    let p = unsafe { pw::getgrgid(gid) };
    if p.is_null() {
        None
    } else {
        pw::ptr_str(unsafe { (*p).gr_name })
    }
}

/// `file attributes name ?-option value ...?`（9.1 unix 属性集：
/// -group / -owner / -permissions，tclUnixFCmd.c 的
/// tclpFileAttrStrings）。取得/设置的消息与 errorCode 按实测钉形：
/// 不存在 `could not read "X": <posix>` + POSIX；坏选项
/// `bad option "X": must be -group, -owner, or -permissions` +
/// TCL LOOKUP INDEX；缺值 `value for "X" missing` +
/// TCL OPERATION FATTR NOVALUE；未知用户/组
/// `could not set owner/group for file "X": user/group "Y" does not
/// exist` + TCL OPERATION SETOWN NO_USER / SETGRP NO_GROUP；坏权限串
/// `unknown permission string format "X"` + TCL VALUE PERMISSION。
#[cfg(feature = "file")]
fn file_attributes(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "file attributes", 3, args.len(),
            "name ?-option value ...?",
        ));
    }
    const ATTRS: [&str; 3] = ["-group", "-owner", "-permissions"];
    let attr_table: Vec<String> = ATTRS.iter().map(|s| s.to_string()).collect();
    let path = args[2].as_str();
    let rest = &args[3..];

    // 一次 stat 支撑取值（tclsh 每个属性各 stat 一次，可观察行为相同：
    // 文件不存在时同样报 could not read + POSIX ENOENT）。
    let s = match interp.host_fs.stat_full(path, true) {
        Ok(s) => s,
        Err(e) => {
            return Err(posix_error(
                interp,
                format!("could not read \"{}\"", path),
                &e,
            ))
        }
    };

    if rest.is_empty() {
        // 全部属性：-group <g> -owner <u> -permissions <mode>（9.1 顺序）。
        let group = name_of_gid(s.gid as u32).unwrap_or_else(|| s.gid.to_string());
        let owner = name_of_uid(s.uid as u32).unwrap_or_else(|| s.uid.to_string());
        let perms = format!("{:#05o}", s.mode & 0x7FFF);
        return Ok(Value::from_list(&[
            Value::from_str("-group"),
            Value::from_str(&group),
            Value::from_str("-owner"),
            Value::from_str(&owner),
            Value::from_str("-permissions"),
            Value::from_str(&perms),
        ]));
    }

    if rest.len() == 1 {
        // 取一个属性；选项允许唯一前缀缩写（tclsh 的 GetIndexFromObj）。
        let which = super::misc::index_in_table(rest[0].as_str(), &attr_table, "option", false)
            .map_err(|(e, code)| {
                super::list::set_error_code(interp, &code);
                e
            })?;
        return Ok(match which {
            0 => Value::from_str(&name_of_gid(s.gid as u32).unwrap_or_else(|| s.gid.to_string())),
            1 => Value::from_str(&name_of_uid(s.uid as u32).unwrap_or_else(|| s.uid.to_string())),
            _ => Value::from_str(&format!("{:#05o}", s.mode & 0x7FFF)),
        });
    }

    // 设置 option/value 对。
    let mut i = 0;
    while i < rest.len() {
        let which = super::misc::index_in_table(rest[i].as_str(), &attr_table, "option", false)
            .map_err(|(e, code)| {
                super::list::set_error_code(interp, &code);
                e
            })?;
        let Some(value) = rest.get(i + 1) else {
            super::list::set_error_code(interp, "TCL OPERATION FATTR NOVALUE");
            return Err(Error::runtime(
                format!("value for \"{}\" missing", rest[i].as_str()),
                crate::error::ErrorCode::Generic,
            ));
        };
        match which {
            0 => set_group_attr(interp, path, value.as_str())?,
            1 => set_owner_attr(interp, path, value.as_str())?,
            _ => set_permissions_attr(interp, path, s.mode as u32, value.as_str())?,
        }
        i += 2;
    }
    Ok(Value::empty())
}

/// `-owner` 的设置：数字即 uid，否则查 passwd（tclUnixFCmd.c 的
/// SetOwnerAttribute）。
#[cfg(feature = "file")]
fn set_owner_attr(interp: &mut Interp, path: &str, value: &str) -> Result<()> {
    let uid = match crate::types::bignum::parse_int(value) {
        Some(crate::types::bignum::IntRep::I64(v)) if v >= 0 && v <= u32::MAX as i64 => v as u32,
        _ => match uid_of(value) {
            Some(u) => u,
            None => {
                super::list::set_error_code(interp, "TCL OPERATION SETOWN NO_USER");
                return Err(Error::runtime(
                    format!(
                        "could not set owner for file \"{}\": user \"{}\" does not exist",
                        path, value
                    ),
                    crate::error::ErrorCode::Generic,
                ));
            }
        },
    };
    chown_one(interp, path, Some(uid), None, "owner")
}

/// `-group` 的设置：数字即 gid，否则查 group。
#[cfg(feature = "file")]
fn set_group_attr(interp: &mut Interp, path: &str, value: &str) -> Result<()> {
    let gid = match crate::types::bignum::parse_int(value) {
        Some(crate::types::bignum::IntRep::I64(v)) if v >= 0 && v <= u32::MAX as i64 => v as u32,
        _ => match gid_of(value) {
            Some(g) => g,
            None => {
                super::list::set_error_code(interp, "TCL OPERATION SETGRP NO_GROUP");
                return Err(Error::runtime(
                    format!(
                        "could not set group for file \"{}\": group \"{}\" does not exist",
                        path, value
                    ),
                    crate::error::ErrorCode::Generic,
                ));
            }
        },
    };
    chown_one(interp, path, None, Some(gid), "group")
}

/// chown 包装（非 unix 无操作系统能力：一切设置如实报 permission
/// denied，tclsh 在没有所有权的文件系统上同样如此）。
#[cfg(feature = "file")]
fn chown_one(
    interp: &mut Interp,
    path: &str,
    uid: Option<u32>,
    gid: Option<u32>,
    what: &str,
) -> Result<()> {
    #[cfg(unix)]
    {
        let c = pw::cstr(path);
        // (uid_t)-1 = 不变。
        let u = uid.unwrap_or(u32::MAX);
        let g = gid.unwrap_or(u32::MAX);
        let rc = unsafe { pw::chown(c.as_ptr(), u, g) };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            return Err(posix_error(
                interp,
                format!("could not set {} for file \"{}\"", what, path),
                &e,
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, what);
        super::list::set_error_code(interp, "POSIX EPERM {operation not permitted}");
        Err(Error::runtime(
            format!("could not set {} for file \"{}\": operation not permitted", what, path),
            crate::error::ErrorCode::Io,
        ))
    }
}

/// `-permissions` 的设置（SetPermissionsAttribute）：先吃 `0` 开头的
/// 八进制（`0755`），再试整数字面量（十进制、`0x`/`0o`），都不行按当前
/// 模式解析 `rwxrwxrwx` 与 `ugoa+-=rwxst` 形式的 chmod 串。掩码
/// 0x7FFF，与 tclsh 相同。
#[cfg(feature = "file")]
fn set_permissions_attr(interp: &mut Interp, path: &str, old_mode: u32, value: &str) -> Result<()> {
    let mode: Option<i64> = {
        // SetPermissionsAttribute 的两个分支按原文与 9.1 实测：
        // 分支一，前导 `0` 跟 [0-7] 时跳过这个 `0`、加 `0o` 前缀按
        // 八进制读（`0755` → 493；`0o18` 不是合法八进制 → 分支失败）；
        // 分支二，整数字面量直读——TclParseNumber 的 ZERO 态对数字
        // `goto decimal`：9.x 移除了 legacy 八进制，`018`/`0799`/`09`
        // 都是十进制（expr/scan/is integer 实测同为十进制）。rtcl 的
        // 共享 parse_int 仍按 expr 的 8.6 钉形把 `0NN` 拒成
        // invalid-octal，这里按本命令的 9.1 钉形补十进制直读。
        let scanned_ws = value.trim_start_matches(|c: char| c.is_whitespace());
        let b = scanned_ws.as_bytes();
        let mut parsed = None;
        if b.len() >= 2 && b[0] == b'0' && (b'0'..=b'7').contains(&b[1]) {
            parsed = match crate::types::bignum::parse_int(&format!("0o{}", &scanned_ws[1..])) {
                Some(crate::types::bignum::IntRep::I64(v)) => Some(v),
                _ => None,
            };
        }
        if parsed.is_none() {
            // 分支二按 TclGetWideIntFromObj：整数字面量直读（十六进制
            // `0x1a4`、`0b`、显式 `0o` 都收），parse_int 拒绝的 `0NN`
            // 再按 TclParseNumber 的 ZERO→decimal 补十进制直读
            // （9.x 移除 legacy 八进制，`018`/`0799`/`09` 都是十进制；
            // rtcl 的共享 parse_int 按 expr 的 8.6 钉形仍拒绝它们）。
            parsed = match crate::types::bignum::parse_int(scanned_ws) {
                Some(crate::types::bignum::IntRep::I64(v)) => Some(v),
                _ => {
                    let (neg, digits) = match scanned_ws.strip_prefix('-') {
                        Some(r) => (true, r),
                        None => (false, scanned_ws.strip_prefix('+').unwrap_or(scanned_ws)),
                    };
                    if !digits.is_empty() && digits.bytes().all(|d| d.is_ascii_digit()) {
                        digits.parse::<i64>().ok().map(|v| if neg { -v } else { v })
                    } else {
                        None
                    }
                }
            };
        }
        parsed
    };
    let new_mode = match mode {
        Some(v) => (v & 0x7FFF) as u32,
        None => {
            let mut m = old_mode & 0x7FFF;
            if !get_mode_from_perm_string(value, &mut m) {
                super::list::set_error_code(interp, "TCL VALUE PERMISSION");
                return Err(Error::runtime(
                    format!("unknown permission string format \"{}\"", value),
                    crate::error::ErrorCode::Generic,
                ));
            }
            m
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let set = std::fs::set_permissions(path, std::fs::Permissions::from_mode(new_mode));
        if let Err(e) = set {
            return Err(posix_error(
                interp,
                format!("could not set permissions for file \"{}\"", path),
                &e,
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, new_mode);
        super::list::set_error_code(interp, "POSIX EPERM {operation not permitted}");
        Err(Error::runtime(
            format!("could not set permissions for file \"{}\": operation not permitted", path),
            crate::error::ErrorCode::Io,
        ))
    }
}

/// tclUnixFCmd.c 的 GetModeFromPermString：`rwxrwxrwx`（恰 9 个字符，
/// `-` 占位；s/S/t/T 处理 setuid/setgid/sticky）或 `ugoa+-=rwxst`
/// 的 chmod 风格（逗号分隔多段，基于传入的当前模式）。移植按原文，
/// 位常量不变。
#[cfg(feature = "file")]
fn get_mode_from_perm_string(mode_string: &str, mode: &mut u32) -> bool {
    let cs = mode_string.as_bytes();
    if cs.len() == 9 {
        let mut new_mode = 0u32;
        let mut ok = true;
        for (i, ch) in cs.iter().enumerate() {
            let bit = |p: usize| 1u32 << (8 - p);
            match ch {
                b'r' if i % 3 == 0 => new_mode |= bit(i),
                b'w' if i % 3 == 1 => new_mode |= bit(i),
                b'x' if i % 3 == 2 => new_mode |= bit(i),
                b's' if i % 3 == 2 && i <= 5 => {
                    new_mode |= bit(i);
                    new_mode |= 1 << (11 - i / 3);
                }
                b'S' if i % 3 == 2 && i <= 5 => new_mode |= 1 << (11 - i / 3),
                b't' if i == 8 => {
                    new_mode |= bit(i);
                    new_mode |= 1 << 9;
                }
                b'T' if i == 8 => new_mode |= 1 << 9,
                b'-' => {}
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            *mode = new_mode;
            return true;
        }
    }
    // chmod 风格：ugoa+-=rwxst，逗号分隔。
    let mut n = 0usize;
    while n < cs.len() {
        let old_mode = *mode;
        let (mut who, mut what) = (0u32, 0u32);
        let mut op = 0u8;
        let mut op_found = false;
        let mut who_found = false;
        let mut i = 0usize;
        while n + i < cs.len() {
            let ch = cs[n + i];
            if !who_found {
                match ch {
                    b'u' => {
                        who |= 0x9C0;
                        i += 1;
                        continue;
                    }
                    b'g' => {
                        who |= 0x438;
                        i += 1;
                        continue;
                    }
                    b'o' => {
                        who |= 0x207;
                        i += 1;
                        continue;
                    }
                    b'a' => {
                        who |= 0xFFF;
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            who_found = true;
            if who == 0 {
                who = 0xFFF;
            }
            if !op_found {
                match ch {
                    b'+' => {
                        op = 1;
                        op_found = true;
                        i += 1;
                        continue;
                    }
                    b'-' => {
                        op = 2;
                        op_found = true;
                        i += 1;
                        continue;
                    }
                    b'=' => {
                        op = 3;
                        op_found = true;
                        i += 1;
                        continue;
                    }
                    _ => return false,
                }
            }
            match ch {
                b'r' => what |= 0x124,
                b'w' => what |= 0x92,
                b'x' => what |= 0x49,
                b's' => what |= 0xC00,
                b't' => what |= 0x200,
                b',' => {}
                _ => return false,
            }
            if ch == b',' {
                i += 1;
                break;
            }
            i += 1;
        }
        *mode = match op {
            1 => old_mode | (who & what),
            2 => old_mode & !(who & what),
            3 => (old_mode & !who) | (who & what),
            _ => return false,
        };
        n += i;
    }
    true
}

/// 用户主目录（TclGetUserHome / MakeTildeRelativePath）：None = 当前
/// 用户（$HOME 环境变量；没有报 `couldn't find HOME environment
/// variable to expand path` + TCL VALUE PATH HOMELESS）；Some(u) =
/// passwd 的 pw_dir，未知用户报 `user "u" doesn't exist` +
/// TCL VALUE PATH NOUSER。
#[cfg(feature = "file")]
fn user_home_dir(interp: &mut Interp, user: Option<&str>) -> Result<String> {
    match user {
        None | Some("") => match std::env::var("HOME") {
            Ok(h) => Ok(h),
            Err(_) => {
                super::list::set_error_code(interp, "TCL VALUE PATH HOMELESS");
                Err(Error::runtime(
                    "couldn't find HOME environment variable to expand path",
                    crate::error::ErrorCode::Generic,
                ))
            }
        },
        Some(u) => {
            #[cfg(unix)]
            {
                let c = pw::cstr(u);
                let p = unsafe { pw::getpwnam(c.as_ptr()) };
                let dir = if p.is_null() {
                    None
                } else {
                    pw::ptr_str(unsafe { (*p).pw_dir })
                };
                match dir {
                    Some(d) => Ok(d),
                    None => {
                        super::list::set_error_code(interp, "TCL VALUE PATH NOUSER");
                        Err(Error::runtime(
                            format!("user \"{}\" doesn't exist", u),
                            crate::error::ErrorCode::Generic,
                        ))
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = u;
                super::list::set_error_code(interp, "TCL VALUE PATH NOUSER");
                Err(Error::runtime(
                    format!("user \"{}\" doesn't exist", u),
                    crate::error::ErrorCode::Generic,
                ))
            }
        }
    }
}

/// `file tildeexpand path`（9.0 新命令；Tcl_FSTildeExpand）：只有开头
/// 的 `~`/`~user` 展开——`~`/`~/...` 到当前用户主目录，`~user/...` 到
/// 该用户主目录（未知用户报 NOUSER）；不以 `~` 开头原样返回，中间的
/// `~` 无意义。
#[cfg(feature = "file")]
fn file_tildeexpand(interp: &mut Interp, path: &str) -> Result<Value> {
    if !path.starts_with('~') {
        return Ok(Value::from_str(path));
    }
    // FindSplitPos：第一个分隔符（'~' 自身算 1）。
    let split = path.find('/').unwrap_or(path.len());
    let (home, sub) = if split == 1 {
        // '~' 或 '~/...'：当前用户；sub = path[2..]（'~' 之后非空才拼）。
        (user_home_dir(interp, None)?, path.get(2..))
    } else {
        let user = &path[1..split];
        // '~user/...' 的 sub 在分隔符之后；'~user' 结尾没有。
        let sub = if path.len() > split { Some(&path[split + 1..]) } else { None };
        (user_home_dir(interp, Some(user))?, sub)
    };
    let mut out = home.trim_end_matches('/').to_string();
    if let Some(s) = sub {
        if !s.is_empty() {
            out.push('/');
            out.push_str(s);
        }
    }
    Ok(Value::from_str(&out))
}

/// The `-force` / `--` options of `file copy` and `file rename`, and
/// where the paths start after them.
#[cfg(feature = "file")]
fn file_force_options(interp: &mut Interp, args: &[Value]) -> Result<(bool, usize)> {
    let mut force = false;
    let mut start = 2;
    let options: Vec<String> = ["-force", "--"].iter().map(|o| o.to_string()).collect();
    while start < args.len() {
        let word = args[start].as_str();
        if !word.starts_with('-') || word == "-" {
            break;
        }
        match super::misc::index_in_table(word, &options, "option", false) {
            Ok(0) => {
                force = true;
                start += 1;
            }
            Ok(_) => {
                start += 1;
                break;
            }
            Err((e, code)) => {
                super::list::set_error_code(interp, &code);
                return Err(e);
            }
        }
    }
    Ok((force, start))
}

/// `ENOENT` as an `io::Error`, without naming a libc.
#[cfg(feature = "file")]
fn libc_enoent() -> i32 {
    #[cfg(unix)]
    {
        2
    }
    #[cfg(not(unix))]
    {
        2
    }
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
            let msg = crate::value::value_report("floating-point number", v.as_str());
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
    // 9.1 grammar via the shared scanner (digits91 批): bare leading
    // zeros are DECIMAL (`format %d 017` is 17), `0d` compels decimal,
    // `_` separators are legal between digits. The magnitude still
    // accumulates with wrapping — the format-side truncation rules are a
    // separate batch.
    let scan = crate::types::bignum::scan_number(s.trim(), true)?;
    if !scan.full {
        return None;
    }
    let crate::types::bignum::NumPayload::Int { radix, run } = scan.payload else {
        return None;
    };
    let digits = scan.int_digits(s.trim());
    if digits.is_empty() || digits.bytes().all(|b| b == b'_') {
        return None;
    }
    let mut mag: u64 = 0;
    for &c in digits.as_bytes() {
        if c == b'_' {
            continue;
        }
        mag = mag
            .wrapping_mul(radix as u64)
            .wrapping_add(crate::types::bignum::digit_value(c) as u64);
    }
    Some(if scan.neg {
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
    // Same 9.1 grammar as parse_tcl_int_strict (shared scanner).
    let scan = crate::types::bignum::scan_number(s.trim(), true)?;
    if !scan.full {
        return None;
    }
    let crate::types::bignum::NumPayload::Int { radix, run } = scan.payload else {
        return None;
    };
    let digits = scan.int_digits(s.trim());
    if digits.is_empty() || digits.bytes().all(|b| b == b'_') {
        return None;
    }
    let mut mag: u128 = 0;
    for &c in digits.as_bytes() {
        if c == b'_' {
            continue;
        }
        mag = mag
            .wrapping_mul(radix as u128)
            .wrapping_add(crate::types::bignum::digit_value(c) as u128);
    }
    Some((scan.neg, mag))
}

/// Result of the wide (`l`-size) integer parse: an in-range value renders
/// exactly like the 64-bit path; a bignum renders from its magnitude.
enum BigOrLit {
    Lit(i64),
    Big(bool, u128),
}

/// format 转换的 size 修饰符宽度档（9.1：无修饰符截 32 位，`ll`/`L`
/// 全精度 bignum）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum FmtSize {
    None,
    H,
    L64,
    Wide,
}

/// 组装一个整数转换域：符号 + 可选进制前缀（`#` 旗标；值为零时 tclsh
/// 不加前缀——`%#x` 0 → `0`，`%p` 除外（恒带 0x））+ 按精度补零的数字
/// 串。返回（域, 符号+前缀的字节数），供零填充把 0 插在前缀之后
/// （`%#020d` 6 → `0d000000000000000006` 钉形）。
fn int_field(
    sign: &str,
    prefix: &'static str,
    hash: bool,
    zero_val: bool,
    mag: &str,
    precision: Option<usize>,
) -> (String, usize) {
    let mut m = mag.to_string();
    if let Some(p) = precision {
        if m.len() < p {
            let mut s = String::with_capacity(p);
            for _ in 0..p - m.len() {
                s.push('0');
            }
            s.push_str(&m);
            m = s;
        }
    }
    let pfx = if hash && !zero_val { prefix } else { "" };
    let field = format!("{}{}{}", sign, pfx, m);
    (field, sign.len() + pfx.len())
}

/// 无符号整数的进制串（x/X/o/b；u128 覆盖 i64/u64/u16/u32 与 bignum
/// 幅度四档，Rust 原生支持 u128 的 {:x}/{:X}/{:o}/{:b}）。
fn fmt_base(v: u128, ch: char) -> String {
    match ch {
        'x' => format!("{:x}", v),
        'X' => format!("{:X}", v),
        'o' => format!("{:o}", v),
        _ => format!("{:b}", v),
    }
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
                    let msg = crate::value::value_report("integer", $arg.as_str());
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
                    let msg = crate::value::value_report("integer", $arg.as_str());
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
                    // 9.1：* 宽度/精度的实参错误码同 VALUE NUMBER
                    // （`catch {format %*d x 3}` → TCL VALUE NUMBER，
                    // oracle 实测；format_grammarG 钉形）。
                    let msg = crate::value::value_report("integer", $arg.as_str());
                    return fmt_err!(msg, "TCL VALUE NUMBER");
                }
            }
        };
    }
    // An XPG star width/precision consumes argv[idx+2] before the value
    // does (`%3$*.*d 5 6 7` runs past the arguments) — tclsh reports the
    // index, never a bounds fault.
    macro_rules! star_available {
        ($idx:expr) => {
            if $idx + 2 >= args.len() {
                return fmt_err!("\"%n$\" argument index out of range", "TCL FORMAT INDEXRANGE");
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

        // Size modifiers, Tcl 9 口径：无修饰符 → 一律截 32 位；'h' →
        // 16 位；单个 'l'（或 q/j/z/t = 64 位类型别名）→ 64 位；
        // 'll'/'L' → 全精度 bignum（`%llb` 2**100 全位输出）。
        // 后跟的第二个 'h' 落到转换字符位 → bad field specifier "h"。
        let size = if pos < len {
            match bytes[pos] {
                b'l' => {
                    let mut cnt = 0usize;
                    while pos < len && bytes[pos] == b'l' {
                        pos += 1;
                        cnt += 1;
                    }
                    if cnt == 1 { FmtSize::L64 } else { FmtSize::Wide }
                }
                b'L' => {
                    pos += 1;
                    FmtSize::Wide
                }
                b'q' | b'j' | b'z' | b't' => {
                    pos += 1;
                    FmtSize::L64
                }
                b'h' => {
                    pos += 1;
                    FmtSize::H
                }
                _ => FmtSize::None,
            }
        } else {
            FmtSize::None
        };

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
                    'd' | 'i' | 'u' | 'b' | 'o' | 'x' | 'X' | 'p' | 'c' | 's'
                        | 'e' | 'E' | 'f' | 'g' | 'G'
                ) {
                    let msg = format!("bad field specifier \"{}\"", ch);
                    return fmt_err!(msg, "TCL FORMAT BADTYPE");
                }
                let mut idx = n - 1;
                if star_width {
                    star_available!(idx);
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
                    star_available!(idx);
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
                    'd' | 'i' | 'u' | 'b' | 'o' | 'x' | 'X' | 'p' | 'c' | 's'
                        | 'e' | 'E' | 'f' | 'g' | 'G'
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

        // 整数域的「符号+进制前缀」字节数：零填充必须插在前缀之后
        // （tclsh `%#020d` 6 → `0d000000000000000006`）。非整数转换恒 0。
        let mut int_head = 0usize;
        let formatted = match ch {
            's' => match precision {
                Some(p) => arg.as_str().chars().take(p).collect::<String>(),
                None => arg.as_str().to_string(),
            },
            'd' | 'i' => {
                let (neg, mag) = match size {
                    FmtSize::Wide => match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => (v < 0, v.unsigned_abs().to_string()),
                        BigOrLit::Big(neg, mag) => (neg, mag.to_string()),
                    },
                    _ => {
                        let mut v = int_arg!(arg);
                        match size {
                            FmtSize::H => v = (v as i16) as i64,
                            FmtSize::None => v = (v as i32) as i64,
                            _ => {}
                        }
                        (v < 0, v.unsigned_abs().to_string())
                    }
                };
                let sign = if neg {
                    "-"
                } else if flag_plus {
                    "+"
                } else if flag_space {
                    " "
                } else {
                    ""
                };
                let (f, head) = int_field(
                    sign,
                    "0d",
                    flag_hash,
                    !neg && mag == "0",
                    &mag,
                    precision,
                );
                int_head = head;
                f
            }
            'u' => {
                let mag = match size {
                    FmtSize::Wide => match int_l_arg!(arg) {
                        BigOrLit::Lit(v) if v < 0 => {
                            return fmt_err!(
                                "unsigned bignum format is invalid",
                                "TCL FORMAT BADUNSIGNED"
                            );
                        }
                        BigOrLit::Lit(v) => (v as u64).to_string(),
                        BigOrLit::Big(true, _) => {
                            return fmt_err!(
                                "unsigned bignum format is invalid",
                                "TCL FORMAT BADUNSIGNED"
                            );
                        }
                        BigOrLit::Big(false, mag) => mag.to_string(),
                    },
                    _ => {
                        let v = int_arg!(arg);
                        let uv: u64 = match size {
                            FmtSize::H => (v as u16) as u64,
                            FmtSize::L64 => v as u64,
                            _ => (v as u32) as u64,
                        };
                        uv.to_string()
                    }
                };
                let (f, head) = int_field("", "", false, false, &mag, precision);
                int_head = head;
                f
            }
            'x' | 'X' | 'o' | 'b' => {
                let (neg, mag, zero) = match size {
                    FmtSize::Wide => match int_l_arg!(arg) {
                        BigOrLit::Lit(v) => {
                            (v < 0, fmt_base(v.unsigned_abs() as u128, ch), v == 0)
                        }
                        BigOrLit::Big(neg, m) => (neg, fmt_base(m, ch), !neg && m == 0),
                    },
                    _ => {
                        let v = int_arg!(arg);
                        let uv: u64 = match size {
                            FmtSize::H => (v as u16) as u64,
                            FmtSize::L64 => v as u64,
                            _ => (v as u32) as u64,
                        };
                        (false, fmt_base(uv as u128, ch), uv == 0)
                    }
                };
                let sign = if neg { "-" } else { "" };
                let prefix = match ch {
                    'x' | 'X' => "0x",
                    'o' => "0o",
                    _ => "0b",
                };
                let (f, head) = int_field(sign, prefix, flag_hash, zero, &mag, precision);
                int_head = head;
                f
            }
            'p' => {
                // %p = 64 位无指针宽度的 `%#x`：恒带 0x 前缀（零值也是
                // `0x0`），+/空格/# 旗标与 h 修饰符之外的 size 无效；
                // `h` 修饰的 %p 在 tclsh 9.1 恒渲染 `0x0`（实测钉形）。
                let s = if size == FmtSize::H {
                    "0".to_string()
                } else {
                    let v = int_arg!(arg);
                    fmt_base((v as u64) as u128, 'x')
                };
                let (f, head) = int_field("", "0x", true, false, &s, precision);
                int_head = head;
                f
            }
            'c' => {
                // 9.1 钉形（oracle 实测，utf-20.1）：Tcl_UniChar 已 32 位，
                // %c 接受完整 Unicode——0x10000 → U+10000 单字符；size 修饰
                // 符不窄化（%hc/%lc 0x10000 均 65536）；负数与 >0x10FFFF
                // → U+FFFD；超 i64 的 bignum 报 ARITH IOVERFLOW（非
                // expected integer）。代理区 rtcl 字符串层（Rust String）
                // 不可存独代理 → 同 U+FFFD（oracle 可存但 puts 报 invalid
                // multibyte，语料不钉此面）。
                let v = match parse_tcl_int_strict(arg.as_str()) {
                    Some(v) => v,
                    None => {
                        if crate::types::bignum::parse_int(arg.as_str().trim()).is_some() {
                            return fmt_err!(
                                "integer value too large to represent",
                                "ARITH IOVERFLOW {integer value too large to represent}"
                            );
                        }
                        let msg = format!("expected integer but got \"{}\"", arg.as_str());
                        return fmt_err!(msg, "TCL VALUE NUMBER");
                    }
                };
                // Tcl_GetIntFromObj 的界：[INT_MIN, UINT_MAX]（注意上界是
                // UINT_MAX——2147483648 通过并截断回绕，-2147483649 报错，
                // oracle 实测非对称）；(unsigned)cp > 0x10FFFF → U+FFFD。
                if !(i32::MIN as i64..=(u32::MAX as i64)).contains(&v) {
                    return fmt_err!(
                        "integer value too large to represent",
                        "ARITH IOVERFLOW {integer value too large to represent}"
                    );
                }
                if (0..=0x10FFFF).contains(&v) {
                    char::from_u32(v as u32).unwrap_or('\u{FFFD}').to_string()
                } else {
                    '\u{FFFD}'.to_string()
                }
            }
            'f' => {
                let v = fmt_double_value(interp, arg)?;
                let prec = precision.unwrap_or(6);
                let s = if v.is_infinite() {
                    inf_text(v.is_sign_negative(), false)
                } else {
                    let mut s = fmt_fixed_pad(prec, v.abs());
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
        // infinity results the fill is spaces.  整数转换带精度时 '0'
        // 旗标失效（C99，tclsh `%08.3llx` 5 → `     005` 空格垫）。
        let is_int = matches!(ch, 'd' | 'i' | 'u' | 'x' | 'X' | 'o' | 'b' | 'p');
        let flag_zero = flag_zero && !(is_int && precision.is_some());
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
                } else if int_head > 0 {
                    // 符号+进制前缀在最前，零填其后再接数字
                    // （`%#020d` 6 → `0d000000000000000006`）。
                    let (head, rest) = formatted.split_at(int_head);
                    result.push_str(head);
                    for _ in 0..pad {
                        result.push('0');
                    }
                    result.push_str(rest);
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

/// Digits beyond this are all zeros for any finite double: the widest
/// exact decimal expansion (a subnormal's) has 1074 fractional digits.
const FMT_FLOAT_MAX_PREC: usize = 1100;

/// `format!("{:.*}", prec, v)` for any precision. Rust's float formatter
/// panics ("Formatting argument out of range") past 65535 digits, so
/// format the head and zero-pad the tail — tclsh honours every precision
/// (`format %.65536f 1.5` is a 65538-char result).
fn fmt_fixed_pad(prec: usize, v: f64) -> String {
    let p = prec.min(FMT_FLOAT_MAX_PREC);
    let mut s = format!("{:.*}", p, v);
    if prec > p {
        s.reserve(prec - p);
        for _ in 0..prec - p {
            s.push('0');
        }
    }
    s
}

/// `format!("{:.*e}", prec, v)` for any precision (zeros pad the mantissa).
fn fmt_exp_pad(prec: usize, v: f64) -> String {
    let p = prec.min(FMT_FLOAT_MAX_PREC);
    let s = format!("{:.*e}", p, v);
    if prec > p {
        if let Some(pos) = s.find('e') {
            let mut out = String::with_capacity(s.len() + prec - p);
            out.push_str(&s[..pos]);
            for _ in 0..prec - p {
                out.push('0');
            }
            out.push_str(&s[pos..]);
            return out;
        }
    }
    s
}

fn format_exp(v: f64, prec: usize, upper: bool, hash: bool) -> String {
    // Rust's {:.*e} is correctly rounded; reassemble with C's exponent
    // layout (sign always present, at least two digits). '#' keeps the
    // decimal point even when the precision is 0 ("1.e+01").
    let s = fmt_exp_pad(prec, v);
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
        // Only the exponent is read here; FMT_FLOAT_MAX_PREC digits are
        // already the exact value, so the exponent is exact too.
        let s_e = fmt_exp_pad(prec.saturating_sub(1), v);
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
            let s = fmt_fixed_pad(fp, v);
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
        // A channel on the new file (tclsh); the path comes back in the
        // variable given.
        let channel = interp.eval("file tempfile _tmpname").unwrap();
        assert!(interp.eval(&format!("fconfigure {}", channel.as_str())).is_ok());
        let name = interp.get_var("_tmpname").unwrap().as_str().to_string();
        let path = std::path::Path::new(&name);
        assert!(path.exists());
        interp.eval(&format!("close {}", channel.as_str())).unwrap();
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
