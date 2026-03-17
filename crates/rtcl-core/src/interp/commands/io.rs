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
