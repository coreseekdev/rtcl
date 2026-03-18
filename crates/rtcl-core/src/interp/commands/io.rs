//! I/O and file commands: puts, source, file, format, glob.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;

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
            if args[1].as_str() == "-nonewline" {
                nonewline = true;
                chan_id = args[2].as_str();
            } else {
                chan_id = args[1].as_str();
            }
            msg = args[3].as_str();
        }
        _ => return Err(Error::wrong_args_with_usage("puts", 2, args.len(), "?-nonewline? ?channelId? string")),
    }

    let ch = interp.channels.get_mut(chan_id)
        .ok_or_else(|| Error::runtime(
            format!("can not find channel named \"{}\"", chan_id),
            crate::error::ErrorCode::Io,
        ))?;

    crate::channel::channel_write_str(ch.as_mut(), msg).map_err(|e| Error::runtime(
        format!("error writing \"{}\": {}", chan_id, e),
        crate::error::ErrorCode::Io,
    ))?;

    if !nonewline {
        crate::channel::channel_write_str(ch.as_mut(), "\n").map_err(|e| Error::runtime(
            format!("error writing \"{}\": {}", chan_id, e),
            crate::error::ErrorCode::Io,
        ))?;
    }

    ch.flush().map_err(|e| Error::runtime(
        format!("error flushing \"{}\": {}", chan_id, e),
        crate::error::ErrorCode::Io,
    ))?;

    Ok(Value::empty())
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
    let contents = std::fs::read_to_string(path).map_err(|e| {
        Error::runtime(
            format!("couldn't read file \"{}\": {}", path, e),
            crate::error::ErrorCode::Io,
        )
    })?;
    interp.eval(&contents)
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
        "tempfile" => return file_tempfile(args),
        "separator" | "sep" => {
            return Ok(Value::from_str(std::path::MAIN_SEPARATOR_STR));
        }
        _ => {}
    }

    // All other subcommands need at least one path argument
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "file", 3, args.len(),
            &format!("{} name ?arg ...?", subcmd),
        ));
    }

    let path = args[2].as_str();

    match subcmd {
        // ── existence / type checks ─────────────────────────────────
        "exists" => Ok(Value::from_bool(std::path::Path::new(path).exists())),
        "isfile" => Ok(Value::from_bool(std::path::Path::new(path).is_file())),
        "isdirectory" => Ok(Value::from_bool(std::path::Path::new(path).is_dir())),
        "readable" => Ok(Value::from_bool(file_access(path, AccessCheck::Read))),
        "writable" => Ok(Value::from_bool(file_access(path, AccessCheck::Write))),
        "executable" => Ok(Value::from_bool(file_access(path, AccessCheck::Exec))),
        "owned" => {
            // Approximate: check if the file exists and we can read its metadata.
            // A full implementation would compare uid, but that requires libc.
            Ok(Value::from_bool(std::fs::metadata(path).is_ok()))
        }
        "type" => {
            let meta = std::fs::symlink_metadata(path).map_err(|e| {
                Error::runtime(
                    format!("could not read \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            let ft = meta.file_type();
            let t = if ft.is_symlink() {
                "link"
            } else if ft.is_dir() {
                "directory"
            } else if ft.is_file() {
                "file"
            } else {
                "file" // character/block special etc. — fallback
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
            let meta = std::fs::metadata(path).map_err(|e| {
                Error::runtime(
                    format!("could not stat \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            Ok(Value::from_int(meta.len() as i64))
        }
        "atime" => {
            let meta = std::fs::metadata(path).map_err(|e| {
                Error::runtime(
                    format!("could not stat \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            let t = meta.accessed().map_err(|e| {
                Error::runtime(format!("could not get atime: {}", e), crate::error::ErrorCode::Io)
            })?;
            let secs = t.duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default().as_secs();
            Ok(Value::from_int(secs as i64))
        }
        "mtime" => {
            let meta = std::fs::metadata(path).map_err(|e| {
                Error::runtime(
                    format!("could not stat \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            let t = meta.modified().map_err(|e| {
                Error::runtime(format!("could not get mtime: {}", e), crate::error::ErrorCode::Io)
            })?;
            let secs = t.duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default().as_secs();
            Ok(Value::from_int(secs as i64))
        }
        "stat" | "lstat" => {
            // file stat name varName / file lstat name varName
            if args.len() < 4 {
                return Err(Error::wrong_args_with_usage(
                    "file", 4, args.len(),
                    &format!("{} name varName", subcmd),
                ));
            }
            let var_name = args[3].as_str();
            let meta = if subcmd == "lstat" {
                std::fs::symlink_metadata(path)
            } else {
                std::fs::metadata(path)
            };
            let meta = meta.map_err(|e| {
                Error::runtime(
                    format!("could not stat \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
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
                let pp = std::path::Path::new(p);
                if pp.is_dir() {
                    if force {
                        std::fs::remove_dir_all(p).ok();
                    } else {
                        std::fs::remove_dir(p).map_err(|e| {
                            Error::runtime(
                                format!("error deleting \"{}\": {}", p, e),
                                crate::error::ErrorCode::Io,
                            )
                        })?;
                    }
                } else if pp.exists() {
                    std::fs::remove_file(p).map_err(|e| {
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
                std::fs::create_dir_all(p).map_err(|e| {
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
            if !force && std::path::Path::new(dst).exists() {
                return Err(Error::runtime(
                    format!("error renaming \"{}\": target exists", src),
                    crate::error::ErrorCode::Io,
                ));
            }
            std::fs::rename(src, dst).map_err(|e| {
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
            if !force && std::path::Path::new(dst).exists() {
                return Err(Error::runtime(
                    format!("error copying \"{}\": target exists", src),
                    crate::error::ErrorCode::Io,
                ));
            }
            std::fs::copy(src, dst).map_err(|e| {
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
fn file_tempfile(args: &[Value]) -> Result<Value> {
    let prefix = if args.len() >= 3 { args[2].as_str() } else { "tcl" };
    let dir = std::env::temp_dir();
    // Simple approach: use timestamp-based name
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{}{}", prefix, stamp);
    let path = dir.join(name);
    std::fs::File::create(&path).map_err(|e| {
        Error::runtime(
            format!("couldn't create temp file: {}", e),
            crate::error::ErrorCode::Io,
        )
    })?;
    Ok(Value::from_str(&path.to_string_lossy()))
}

enum AccessCheck { Read, Write, Exec }

#[cfg(feature = "file")]
fn file_access(path: &str, check: AccessCheck) -> bool {
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

pub fn cmd_format(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("format", 2, args.len()));
    }
    let fmt = args[1].as_str();
    let mut result = String::new();
    let mut arg_idx = 2;
    let bytes = fmt.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        if bytes[pos] != b'%' {
            result.push(bytes[pos] as char);
            pos += 1;
            continue;
        }
        pos += 1; // skip '%'
        if pos >= len { break; }

        // %%
        if bytes[pos] == b'%' {
            result.push('%');
            pos += 1;
            continue;
        }

        // Parse flags: - + 0 space #
        let mut flag_minus = false;
        let mut flag_plus = false;
        let mut flag_zero = false;
        let mut flag_space = false;
        let mut flag_hash = false;
        loop {
            if pos >= len { break; }
            match bytes[pos] {
                b'-' => { flag_minus = true; pos += 1; }
                b'+' => { flag_plus = true; pos += 1; }
                b'0' => { flag_zero = true; pos += 1; }
                b' ' => { flag_space = true; pos += 1; }
                b'#' => { flag_hash = true; pos += 1; }
                _ => break,
            }
        }

        // Parse width (number or *)
        let width: Option<usize> = if pos < len && bytes[pos] == b'*' {
            pos += 1;
            if arg_idx < args.len() {
                let w = args[arg_idx].as_int().unwrap_or(0);
                arg_idx += 1;
                Some(w.unsigned_abs() as usize)
            } else {
                None
            }
        } else {
            let start = pos;
            while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
            if pos > start {
                Some(core::str::from_utf8(&bytes[start..pos]).unwrap_or("0").parse().unwrap_or(0))
            } else {
                None
            }
        };

        // Parse precision: .number or .*
        let precision: Option<usize> = if pos < len && bytes[pos] == b'.' {
            pos += 1;
            if pos < len && bytes[pos] == b'*' {
                pos += 1;
                if arg_idx < args.len() {
                    let p = args[arg_idx].as_int().unwrap_or(0);
                    arg_idx += 1;
                    Some(p.max(0) as usize)
                } else {
                    Some(0)
                }
            } else {
                let start = pos;
                while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                if pos > start {
                    Some(core::str::from_utf8(&bytes[start..pos]).unwrap_or("0").parse().unwrap_or(0))
                } else {
                    Some(0)
                }
            }
        } else {
            None
        };

        // Skip length modifiers (l, h, ll, etc.)
        while pos < len && matches!(bytes[pos], b'l' | b'h' | b'L') { pos += 1; }

        if pos >= len { break; }
        let spec = bytes[pos] as char;
        pos += 1;

        if arg_idx >= args.len() && spec != '%' {
            return Err(Error::runtime(
                "not enough arguments for all format specifiers",
                crate::error::ErrorCode::Generic,
            ));
        }

        let formatted = match spec {
            's' => {
                let s = args[arg_idx].as_str().to_string();
                arg_idx += 1;
                if let Some(prec) = precision {
                    if prec < s.len() { s[..prec].to_string() } else { s }
                } else { s }
            }
            'd' | 'i' => {
                let v = args[arg_idx].as_int().unwrap_or(0);
                arg_idx += 1;
                format_int(v, 10, false, flag_plus, flag_space, flag_hash)
            }
            'u' => {
                let v = args[arg_idx].as_int().unwrap_or(0) as u64;
                arg_idx += 1;
                v.to_string()
            }
            'x' => {
                let v = args[arg_idx].as_int().unwrap_or(0);
                arg_idx += 1;
                let s = format!("{:x}", v);
                if flag_hash { format!("0x{}", s) } else { s }
            }
            'X' => {
                let v = args[arg_idx].as_int().unwrap_or(0);
                arg_idx += 1;
                let s = format!("{:X}", v);
                if flag_hash { format!("0X{}", s) } else { s }
            }
            'o' => {
                let v = args[arg_idx].as_int().unwrap_or(0);
                arg_idx += 1;
                let s = format!("{:o}", v);
                if flag_hash && !s.starts_with('0') { format!("0{}", s) } else { s }
            }
            'b' => {
                let v = args[arg_idx].as_int().unwrap_or(0);
                arg_idx += 1;
                format!("{:b}", v)
            }
            'f' => {
                let v = args[arg_idx].as_float().unwrap_or(0.0);
                arg_idx += 1;
                let prec = precision.unwrap_or(6);
                let s = format!("{:.*}", prec, v.abs());
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'e' => {
                let v = args[arg_idx].as_float().unwrap_or(0.0);
                arg_idx += 1;
                let prec = precision.unwrap_or(6);
                let s = format_exp(v.abs(), prec, false);
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'E' => {
                let v = args[arg_idx].as_float().unwrap_or(0.0);
                arg_idx += 1;
                let prec = precision.unwrap_or(6);
                let s = format_exp(v.abs(), prec, true);
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'g' => {
                let v = args[arg_idx].as_float().unwrap_or(0.0);
                arg_idx += 1;
                let prec = precision.unwrap_or(6).max(1);
                let s = format_g(v.abs(), prec, false);
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'G' => {
                let v = args[arg_idx].as_float().unwrap_or(0.0);
                arg_idx += 1;
                let prec = precision.unwrap_or(6).max(1);
                let s = format_g(v.abs(), prec, true);
                apply_sign(v, &s, flag_plus, flag_space)
            }
            'c' => {
                let v = args[arg_idx].as_int().unwrap_or(0) as u32;
                arg_idx += 1;
                char::from_u32(v).map_or(String::new(), |c| c.to_string())
            }
            _ => {
                format!("%{}", spec)
            }
        };

        // Apply width and alignment
        let w = width.unwrap_or(0);
        if w > formatted.len() {
            let pad = w - formatted.len();
            let fill = if flag_zero && !flag_minus && !matches!(spec, 's' | 'c') { '0' } else { ' ' };
            if flag_minus {
                result.push_str(&formatted);
                for _ in 0..pad { result.push(' '); }
            } else if fill == '0' && (formatted.starts_with('-') || formatted.starts_with('+') || formatted.starts_with(' ')) {
                // Pad zeros after sign
                let (sign, rest) = formatted.split_at(1);
                result.push_str(sign);
                for _ in 0..pad { result.push('0'); }
                result.push_str(rest);
            } else {
                for _ in 0..pad { result.push(fill); }
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
    if v.is_sign_negative() && v != 0.0 {
        format!("-{}", abs_str)
    } else if plus {
        format!("+{}", abs_str)
    } else if space {
        format!(" {}", abs_str)
    } else {
        abs_str.to_string()
    }
}

fn format_exp(v: f64, prec: usize, upper: bool) -> String {
    let e_char = if upper { 'E' } else { 'e' };
    if v == 0.0 {
        return format!("{:.*}{}{}", prec, 0.0_f64, e_char, "+00");
    }
    let exp = v.abs().log10().floor() as i32;
    let mantissa = v / 10.0_f64.powi(exp);
    format!("{:.*}{}{:+03}", prec, mantissa, e_char, exp)
}

fn format_g(v: f64, prec: usize, upper: bool) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let exp = v.abs().log10().floor() as i32;
    if exp < -4 || exp >= prec as i32 {
        format_exp(v, prec.saturating_sub(1), upper)
    } else {
        // Trim trailing zeros
        let s = format!("{:.*}", prec.saturating_sub(1).max(0), v);
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    }
}

// ---------- glob (file pattern matching) ----------

#[cfg(feature = "file")]
pub fn cmd_glob(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("glob", 2, args.len()));
    }

    let mut i = 1;
    let mut nocomplain = false;
    let mut directory = None;

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
            "--" => { i += 1; break; }
            _ => break,
        }
    }

    let mut results: Vec<Value> = Vec::new();
    for j in i..args.len() {
        let pattern = args[j].as_str();
        let search_dir = directory.as_deref().unwrap_or(".");
        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if glob_match(pattern, &name) {
                    if let Some(ref dir) = directory {
                        results.push(Value::from_str(&format!("{}/{}", dir, name)));
                    } else {
                        results.push(Value::from_str(&name));
                    }
                }
            }
        }
    }

    if results.is_empty() && !nocomplain {
        return Err(Error::runtime(
            "no files matched glob patterns",
            crate::error::ErrorCode::NotFound,
        ));
    }

    Ok(Value::from_list(&results))
}

#[cfg(not(feature = "file"))]
pub fn cmd_glob(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "glob: not available without 'file' feature",
        crate::error::ErrorCode::InvalidOp,
    ))
}
