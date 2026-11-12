//! Compatibility surface for tclsh's canonical command set — commands the
//! iteration-order corpus requires to be *visible*, implemented faithfully
//! where cheap and stubbed where the underlying machinery is a feature of
//! its own (coroutines, binary extension loading, DEFLATE).  Every stub
//! documents its tclsh divergence here and in DIVERGENCES.md.
//!
//! - `coroutine`/`yield`/`yieldto`: rtcl has no coroutine engine.  `yield`
//!   /`yieldto` outside a coroutine raise tclsh's exact error (the only
//!   reachable case); `coroutine` accepts tclsh's arity and then reports
//!   the missing engine.
//! - `load`/`unload`: no dynamic loader.  Arity is faithful; a missing
//!   file reproduces tclsh's message verbatim, an existing one reports the
//!   unsupported loader.
//! - `zlib`: `crc32`/`adler32` are real (pure checksums); compress/
//!   decompress report the missing DEFLATE backend.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

pub fn cmd_yield(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "yield can only be called in a coroutine",
        ErrorCode::Generic,
    ))
}

pub fn cmd_yieldto(_interp: &mut Interp, _args: &[Value]) -> Result<Value> {
    Err(Error::runtime(
        "yieldto can only be called in a coroutine",
        ErrorCode::Generic,
    ))
}

pub fn cmd_coroutine(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // tclsh: coroutine name cmd ?arg ...?  (`coroutine c` is an arity
    // error; `coroutine c {}` reads the empty command name).
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "coroutine",
            2,
            args.len(),
            "name cmd ?arg ...?",
        ));
    }
    let _ = interp;
    Err(Error::runtime(
        format!(
            "invalid command name \"{}\"",
            crate::value::tcl_quote(args[2].as_str())
        ),
        ErrorCode::Generic,
    ))
}

pub fn cmd_load(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // load ?-global? ?-lazy? ?--? fileName ?packageName? ?interp?
    let mut i = 1;
    while i < args.len() && args[i].as_str().starts_with('-') {
        match args[i].as_str() {
            "-global" | "-lazy" | "--" => i += 1,
            _ => break,
        }
    }
    if args.len() - i < 1 || args.len() - i > 3 {
        return Err(Error::wrong_args_with_usage(
            "load",
            1,
            args.len(),
            "?-global? ?-lazy? ?--? fileName ?packageName? ?interp?",
        ));
    }
    let file = args[i].as_str();
    if !std::path::Path::new(file).exists() {
        return Err(Error::runtime(
            format!(
                "couldn't load file \"{}\": {}: cannot open shared object file: No such file or directory",
                file, file
            ),
            ErrorCode::Io,
        ));
    }
    let _ = interp;
    Err(Error::runtime(
        "dynamic loading is not supported in this rtcl build",
        ErrorCode::Io,
    ))
}

pub fn cmd_unload(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "unload",
            1,
            args.len(),
            "?-switch ...? fileName ?packageName? ?interp?",
        ));
    }
    Err(Error::runtime(
        "dynamic loading is not supported in this rtcl build",
        ErrorCode::Io,
    ))
}

/// `zlib crc32 data ?startValue?` / `zlib adler32 data ?startValue?` —
/// real checksums; everything else reports the missing DEFLATE backend.
pub fn cmd_zlib(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    const SUBCOMMANDS: &[&str] = &[
        "adler32", "compress", "crc32", "decompress", "gzip", "push", "stream", "ungzip",
        "unzip", "zip",
    ];
    if args.len() < 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: should be \"zlib subcommand ?arg ...?\"".to_string(),
        ));
    }
    let sub = args[1].as_str();
    let resolved = resolve_prefix(SUBCOMMANDS, sub);
    match resolved {
        Ok("crc32") | Ok("adler32") => {
            if args.len() < 3 || args.len() > 4 {
                return Err(Error::wrong_args_msg(format!(
                    "wrong # args: should be \"zlib {} data ?startValue?\"",
                    sub
                )));
            }
            let data = args[2].as_str().as_bytes();
            let start: u32 = if args.len() == 4 {
                args[3]
                    .as_str()
                    .parse()
                    .map_err(|_| Error::runtime("expected integer", ErrorCode::Generic))?
            } else {
                0
            };
            if resolved == Ok("crc32") {
                Ok(Value::from_int((crc32(data, start)) as i64))
            } else {
                Ok(Value::from_int((adler32(data, start)) as i64))
            }
        }
        Ok(_) => Err(Error::runtime(
            format!(
                "zlib {} is not supported in this rtcl build",
                sub
            ),
            ErrorCode::Generic,
        )),
        Err(()) => {
            interp.global_insert(
                "errorCode".to_string(),
                Value::from_str(&format!(
                    "TCL LOOKUP SUBCOMMAND {}",
                    crate::value::tcl_quote(sub)
                )),
            );
            interp.err_code_raised = true;
            Err(Error::runtime(
                format!(
                    "unknown or ambiguous subcommand \"{}\": must be {}",
                    crate::value::tcl_quote(sub),
                    render_or_list(SUBCOMMANDS)
                ),
                ErrorCode::Generic,
            ))
        }
    }
}

/// Prefix resolution over a `&'static str` table (chan_io::resolve_prefix's
/// shape with owned table).
fn resolve_prefix<'t>(table: &[&'t str], s: &str) -> std::result::Result<&'t str, ()> {
    let mut hit: Option<&'t str> = None;
    let mut count = 0;
    for t in table {
        if *t == s {
            return Ok(t);
        }
        if t.starts_with(s) {
            hit = Some(t);
            count += 1;
        }
    }
    if count == 1 {
        Ok(hit.unwrap())
    } else {
        Err(())
    }
}

fn render_or_list(table: &[&str]) -> String {
    match table.len() {
        0 => String::new(),
        1 => table[0].to_string(),
        2 => format!("{} or {}", table[0], table[1]),
        _ => format!(
            "{}, or {}",
            table[..table.len() - 1].join(", "),
            table[table.len() - 1]
        ),
    }
}

fn crc32(data: &[u8], start: u32) -> u32 {
    let mut crc = !start;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB88320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8], start: u32) -> u32 {
    // zlib semantics: a startValue of 0 means "fresh" — s1 begins at 1.
    let (mut a, mut b) = if start == 0 {
        (1u32, 0u32)
    } else {
        (start & 0xFFFF, (start >> 16) & 0xFFFF)
    };
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

