//! I/O and file commands: puts, source, file, format, glob.

use crate::error::{Error, Result};
use crate::interp::{glob_match, Interp};
use crate::value::Value;

// ---------- puts ----------

#[cfg(feature = "std")]
pub fn cmd_puts(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    match args.len() {
        2 => {
            println!("{}", args[1].as_str());
            Ok(Value::empty())
        }
        3 => {
            let flag = args[1].as_str();
            let msg = args[2].as_str();
            if flag == "-nonewline" || flag == "stdout" || flag == "stderr" {
                if flag == "-nonewline" {
                    print!("{}", msg);
                } else if flag == "stderr" {
                    eprintln!("{}", msg);
                } else {
                    println!("{}", msg);
                }
                Ok(Value::empty())
            } else {
                // treat as channel + data
                println!("{}", msg);
                Ok(Value::empty())
            }
        }
        4 => {
            let flag = args[1].as_str();
            let _chan = args[2].as_str();
            let msg = args[3].as_str();
            if flag == "-nonewline" {
                print!("{}", msg);
            } else {
                println!("{}", msg);
            }
            Ok(Value::empty())
        }
        _ => Err(Error::wrong_args_with_usage("puts", 2, args.len(), "?-nonewline? ?channelId? string")),
    }
}

#[cfg(not(feature = "std"))]
pub fn cmd_puts(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    // no-std: puts is a no-op
    Ok(Value::empty())
}

// ---------- source ----------

#[cfg(feature = "std")]
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

#[cfg(not(feature = "std"))]
pub fn cmd_source(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "source: not available in no-std mode",
        crate::error::ErrorCode::InvalidOp,
    ))
}

// ---------- file ----------

#[cfg(feature = "std")]
pub fn cmd_file(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args("file", 3, args.len()));
    }

    let subcmd = args[1].as_str();
    let path = args[2].as_str();

    match subcmd {
        "exists" => Ok(Value::from_bool(std::path::Path::new(path).exists())),
        "isfile" => Ok(Value::from_bool(std::path::Path::new(path).is_file())),
        "isdirectory" => Ok(Value::from_bool(std::path::Path::new(path).is_dir())),
        "size" => {
            let meta = std::fs::metadata(path).map_err(|e| {
                Error::runtime(
                    format!("could not stat \"{}\": {}", path, e),
                    crate::error::ErrorCode::Io,
                )
            })?;
            Ok(Value::from_int(meta.len() as i64))
        }
        "extension" => {
            let p = std::path::Path::new(path);
            Ok(Value::from_str(
                p.extension().and_then(|s| s.to_str()).unwrap_or(""),
            ))
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
        "join" => {
            let result: Vec<&str> = args[2..].iter().map(|a| a.as_str()).collect();
            let joined: std::path::PathBuf = result.iter().collect();
            Ok(Value::from_str(joined.to_str().unwrap_or("")))
        }
        "delete" => {
            if std::path::Path::new(path).is_dir() {
                std::fs::remove_dir_all(path).ok();
            } else {
                std::fs::remove_file(path).ok();
            }
            Ok(Value::empty())
        }
        "mkdir" => {
            std::fs::create_dir_all(path).map_err(|e| {
                Error::runtime(format!("couldn't create directory \"{}\": {}", path, e), crate::error::ErrorCode::Io)
            })?;
            Ok(Value::empty())
        }
        "rename" => {
            if args.len() != 4 {
                return Err(Error::wrong_args("file rename", 4, args.len()));
            }
            let new_path = args[3].as_str();
            std::fs::rename(path, new_path).map_err(|e| {
                Error::runtime(format!("couldn't rename \"{}\": {}", path, e), crate::error::ErrorCode::Io)
            })?;
            Ok(Value::empty())
        }
        _ => Err(Error::runtime(
            format!("unknown file subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

#[cfg(not(feature = "std"))]
pub fn cmd_file(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "file: not available in no-std mode",
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
    let mut chars = fmt.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            if let Some(spec) = chars.next() {
                match spec {
                    '%' => result.push('%'),
                    's' => {
                        if arg_idx < args.len() {
                            result.push_str(args[arg_idx].as_str());
                            arg_idx += 1;
                        }
                    }
                    'd' | 'i' => {
                        if arg_idx < args.len() {
                            let v = args[arg_idx].as_int().unwrap_or(0);
                            result.push_str(&v.to_string());
                            arg_idx += 1;
                        }
                    }
                    'f' => {
                        if arg_idx < args.len() {
                            let v = args[arg_idx].as_float().unwrap_or(0.0);
                            result.push_str(&format!("{:.6}", v));
                            arg_idx += 1;
                        }
                    }
                    'x' => {
                        if arg_idx < args.len() {
                            let v = args[arg_idx].as_int().unwrap_or(0);
                            result.push_str(&format!("{:x}", v));
                            arg_idx += 1;
                        }
                    }
                    'X' => {
                        if arg_idx < args.len() {
                            let v = args[arg_idx].as_int().unwrap_or(0);
                            result.push_str(&format!("{:X}", v));
                            arg_idx += 1;
                        }
                    }
                    'o' => {
                        if arg_idx < args.len() {
                            let v = args[arg_idx].as_int().unwrap_or(0);
                            result.push_str(&format!("{:o}", v));
                            arg_idx += 1;
                        }
                    }
                    'c' => {
                        if arg_idx < args.len() {
                            let v = args[arg_idx].as_int().unwrap_or(0) as u32;
                            if let Some(c) = char::from_u32(v) {
                                result.push(c);
                            }
                            arg_idx += 1;
                        }
                    }
                    _ => {
                        result.push('%');
                        result.push(spec);
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }
    Ok(Value::from_str(&result))
}

// ---------- glob (file pattern matching) ----------

#[cfg(feature = "std")]
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

#[cfg(not(feature = "std"))]
pub fn cmd_glob(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "glob: not available in no-std mode",
        crate::error::ErrorCode::InvalidOp,
    ))
}
