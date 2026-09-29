//! The `binary` command — format/scan/encode/decode (tclsh 8.6.17).
//!
//! Byte ↔ string mapping follows tclsh: a string contributes one byte per
//! character (the character's low byte), and a byte string renders as one
//! character per byte (latin-1 style).  This keeps `binary format`/`scan`
//! results round-trippable through ordinary string values.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

// ── string ↔ bytes ─────────────────────────────────────────────────────

/// String → bytes: one byte per character, low 8 bits (tclsh: `Ā` → 0x00,
/// `€` → 0xac).
fn string_to_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

/// Bytes → string: one character per byte (latin-1), so the mapping is
/// invertible.
fn bytes_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from_u32(b as u32).unwrap()).collect()
}

// ── field spec parsing ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct Field {
    ch: u8,
    count: Option<usize>,
    /// `*` count — "all" (all digits / all list elements / all remaining).
    star: bool,
    unsigned: bool,
}

/// Parse a format string into fields.  Grammar: letter, optional `u`
/// modifier, optional decimal count (`@` requires a count).
fn parse_fields(fmt: &str) -> Result<Vec<Field>> {
    let mut fields = Vec::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if !(ch == '@' || ch.is_ascii_alphabetic()) {
            return Err(Error::runtime(
                format!("bad field specifier \"{}\"", ch),
                ErrorCode::Generic,
            ));
        }
        if !matches!(ch, 'a'|'A'|'b'|'B'|'h'|'H'|'c'|'s'|'S'|'t'|'i'|'I'|'n'|'w'|'W'|'m'|'q'|'Q'|'d'|'f'|'r'|'R'|'x'|'X'|'@') {
            return Err(Error::runtime(
                format!("bad field specifier \"{}\"", ch),
                ErrorCode::Generic,
            ));
        }
        let mut field = Field { ch: ch as u8, count: None, star: false, unsigned: false };
        i += 1;
        if i < chars.len() && chars[i] == 'u' && field.ch != b'@' {
            field.unsigned = true;
            i += 1;
        }
        if i < chars.len() && chars[i] == '*' {
            field.star = true;
            i += 1;
        }
        let mut count: usize = 0;
        let mut have_count = false;
        while i < chars.len() && chars[i].is_ascii_digit() {
            count = count * 10 + (chars[i] as u8 - b'0') as usize;
            have_count = true;
            i += 1;
        }
        if have_count {
            field.count = Some(count);
        }
        if field.ch == b'@' && field.count.is_none() {
            return Err(Error::runtime(
                "missing count for \"@\" field specifier",
                ErrorCode::Generic,
            ));
        }
        fields.push(field);
    }
    Ok(fields)
}

// ── command dispatch ───────────────────────────────────────────────────

pub fn cmd_binary(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "binary", 2, args.len(),
            "subcommand ?arg ...?",
        ));
    }
    match resolve_prefix(args[1].as_str(), &["decode", "encode", "format", "scan"]) {
        Some("format") => binary_format(&args[2..]),
        Some("scan") => binary_scan(interp, &args[2..]),
        Some("encode") => binary_encode(&args[2..]),
        Some("decode") => binary_decode(&args[2..]),
        _ => Err(Error::runtime(
            format!(
                "unknown or ambiguous subcommand \"{}\": must be decode, encode, format, or scan",
                args[1].as_str()
            ),
            ErrorCode::Generic,
        )),
    }
}

/// Tcl-style unique-prefix subcommand resolution (exact match wins).
fn resolve_prefix<'a>(name: &str, options: &[&'a str]) -> Option<&'a str> {
    if let Some(exact) = options.iter().find(|o| **o == name) {
        return Some(exact);
    }
    let matches: Vec<&'a str> =
        options.iter().filter(|o| o.starts_with(name)).map(|o| *o).collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

// ── format ─────────────────────────────────────────────────────────────

fn not_enough_args() -> Error {
    Error::runtime(
        "not enough arguments for all format specifiers",
        ErrorCode::Generic,
    )
}

fn binary_format(args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(Error::wrong_args_with_usage(
            "binary", 1, args.len(),
            "format formatString ?arg ...?",
        ));
    }
    let fields = parse_fields(args[0].as_str())?;
    let mut out: Vec<u8> = Vec::new();
    let mut pos: usize = 0;
    let mut argi: usize = 1;

    for f in &fields {
        match f.ch {
            b'a' | b'A' => {
                let arg = args.get(argi).ok_or_else(not_enough_args)?;
                argi += 1;
                let chars: Vec<char> = arg.as_str().chars().collect();
                let count = f.count.unwrap_or(chars.len());
                let pad = if f.ch == b'a' { 0u8 } else { b' ' };
                let bytes: Vec<u8> = (0..count)
                    .map(|k| chars.get(k).map(|&c| c as u32 as u8).unwrap_or(pad))
                    .collect();
                write_at(&mut out, pos, &bytes);
                pos += bytes.len();
            }
            b'b' | b'B' => {
                let arg = args.get(argi).ok_or_else(not_enough_args)?;
                argi += 1;
                let src = arg.as_str();
                let chars: Vec<char> = src.chars().collect();
                let count = if f.star { chars.len() } else { f.count.unwrap_or(8) };
                let need = count.div_ceil(8);
                let mut bytes = vec![0u8; need];
                for k in 0..count {
                    let bit_val = match chars.get(k) {
                        None => 0u8, // beyond the digit string: pad with 0
                        Some('1') => 1u8,
                        Some('0') => 0u8,
                        Some(_) => {
                            return Err(Error::runtime(
                                format!(
                                    "expected binary string but got \"{}\" instead",
                                    src
                                ),
                                ErrorCode::Generic,
                            ));
                        }
                    };
                    if bit_val == 1 {
                        let bit = if f.ch == b'b' { k % 8 } else { 7 - (k % 8) };
                        bytes[k / 8] |= 1 << bit;
                    }
                }
                write_at(&mut out, pos, &bytes);
                pos += need;
            }
            b'h' | b'H' => {
                let arg = args.get(argi).ok_or_else(not_enough_args)?;
                argi += 1;
                let src = arg.as_str();
                let chars: Vec<char> = src.chars().collect();
                let count = if f.star { chars.len() } else { f.count.unwrap_or(2) };
                let mut nibbles = Vec::with_capacity(count);
                for k in 0..count {
                    let v = match chars.get(k) {
                        None => 0, // beyond the digit string: pad with 0
                        Some(&c) => match c.to_digit(16) {
                            Some(d) => d as u8,
                            None => {
                                return Err(Error::runtime(
                                    format!(
                                        "expected hexadecimal string but got \"{}\" instead",
                                        src
                                    ),
                                    ErrorCode::Generic,
                                ));
                            }
                        },
                    };
                    nibbles.push(v);
                }
                let full = count / 2;
                let mut bytes = vec![0u8; full + count % 2];
                for k in 0..full {
                    let (lo, hi) = if f.ch == b'h' {
                        (nibbles[2 * k], nibbles[2 * k + 1])
                    } else {
                        (nibbles[2 * k + 1], nibbles[2 * k])
                    };
                    bytes[k] = lo | (hi << 4);
                }
                if count % 2 == 1 {
                    let last = nibbles[count - 1];
                    bytes[full] = if f.ch == b'h' { last } else { last << 4 };
                }
                write_at(&mut out, pos, &bytes);
                pos += bytes.len();
            }
            b'x' => {
                let n = f.count.unwrap_or(1);
                write_at(&mut out, pos, &vec![0u8; n]);
                pos += n;
            }
            b'X' => pos = pos.saturating_sub(f.count.unwrap_or(1)),
            b'@' => pos = f.count.unwrap_or(0),
            b'c' | b's' | b'S' | b't' | b'i' | b'I' | b'n' | b'w' | b'W' | b'm' => {
                let size: usize = match f.ch {
                    b'c' => 1,
                    b's' | b'S' | b't' => 2,
                    b'i' | b'I' | b'n' => 4,
                    _ => 8,
                };
                let big_endian = matches!(f.ch, b'S' | b'I' | b'W');
                let count = f.count.unwrap_or(1);
                if count == 0 {
                    args.get(argi).ok_or_else(not_enough_args)?;
                    argi += 1;
                    continue;
                }
                let values = if f.star {
                    format_star_ints(args, &mut argi)?
                } else {
                    format_int_args(args, &mut argi, count)?
                };
                let mut bytes = Vec::with_capacity(size * values.len());
                for v in values {
                    let v = v as u64;
                    for k in 0..size {
                        let shift = if big_endian { 8 * (size - 1 - k) } else { 8 * k };
                        bytes.push(((v >> shift) & 0xff) as u8);
                    }
                }
                write_at(&mut out, pos, &bytes);
                pos += bytes.len();
            }
            b'q' | b'Q' | b'd' | b'f' | b'r' | b'R' => {
                let size: usize = if matches!(f.ch, b'd' | b'q' | b'Q') { 8 } else { 4 };
                let big_endian = matches!(f.ch, b'Q' | b'R');
                let count = f.count.unwrap_or(1);
                if count == 0 {
                    args.get(argi).ok_or_else(not_enough_args)?;
                    argi += 1;
                    continue;
                }
                let values = if f.star {
                    format_star_floats(args, &mut argi)?
                } else {
                    format_float_args(args, &mut argi, count)?
                };
                let mut bytes = Vec::with_capacity(size * values.len());
                for v in values {
                    let bits = if size == 8 { v.to_bits() } else { (v as f32).to_bits() as u64 };
                    for k in 0..size {
                        let shift = if big_endian { 8 * (size - 1 - k) } else { 8 * k };
                        bytes.push(((bits >> shift) & 0xff) as u8);
                    }
                }
                write_at(&mut out, pos, &bytes);
                pos += bytes.len();
            }
            _ => unreachable!(),
        }
    }
    Ok(Value::from_str(&bytes_to_string(&out)))
}

/// Write `bytes` at `pos`, zero-extending the buffer if needed.
fn write_at(out: &mut Vec<u8>, pos: usize, bytes: &[u8]) {
    if pos + bytes.len() > out.len() {
        out.resize(pos + bytes.len(), 0);
    }
    out[pos..pos + bytes.len()].copy_from_slice(bytes);
}

/// Collect `count` integer values starting at `*argi` (scalar for count 1,
/// exact-length list for count > 1).
fn format_int_args(args: &[Value], argi: &mut usize, count: usize) -> Result<Vec<i64>> {
    let arg = args.get(*argi).ok_or_else(not_enough_args)?;
    *argi += 1;
    if count == 1 {
        return Ok(vec![arg.as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", arg.as_str()),
                ErrorCode::Generic,
            )
        })?]);
    }
    let list = arg.as_list().ok_or_else(list_count_mismatch)?;
    if list.len() != count {
        return Err(list_count_mismatch());
    }
    list.iter()
        .map(|v| {
            v.as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", v.as_str()),
                    ErrorCode::Generic,
                )
            })
        })
        .collect()
}

fn format_float_args(args: &[Value], argi: &mut usize, count: usize) -> Result<Vec<f64>> {
    let arg = args.get(*argi).ok_or_else(not_enough_args)?;
    *argi += 1;
    if count == 1 {
        return Ok(vec![arg.as_float().ok_or_else(|| {
            Error::runtime(
                format!("expected floating-point number but got \"{}\"", arg.as_str()),
                ErrorCode::Generic,
            )
        })?]);
    }
    let list = arg.as_list().ok_or_else(list_count_mismatch)?;
    if list.len() != count {
        return Err(list_count_mismatch());
    }
    list.iter()
        .map(|v| {
            v.as_float().ok_or_else(|| {
                Error::runtime(
                    format!("expected floating-point number but got \"{}\"", v.as_str()),
                    ErrorCode::Generic,
                )
            })
        })
        .collect()
}

/// Collect ALL elements of the list argument (count `*`).
fn format_star_ints(args: &[Value], argi: &mut usize) -> Result<Vec<i64>> {
    let arg = args.get(*argi).ok_or_else(not_enough_args)?;
    *argi += 1;
    arg.as_list()
        .ok_or_else(list_count_mismatch)?
        .iter()
        .map(|v| {
            v.as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", v.as_str()),
                    ErrorCode::Generic,
                )
            })
        })
        .collect()
}

/// Collect ALL elements of the list argument as floats (count `*`).
fn format_star_floats(args: &[Value], argi: &mut usize) -> Result<Vec<f64>> {
    let arg = args.get(*argi).ok_or_else(not_enough_args)?;
    *argi += 1;
    arg.as_list()
        .ok_or_else(list_count_mismatch)?
        .iter()
        .map(|v| {
            v.as_float().ok_or_else(|| {
                Error::runtime(
                    format!("expected floating-point number but got \"{}\"", v.as_str()),
                    ErrorCode::Generic,
                )
            })
        })
        .collect()
}

fn list_count_mismatch() -> Error {
    Error::runtime(
        "number of elements in list does not match count",
        ErrorCode::Generic,
    )
}

// ── scan ───────────────────────────────────────────────────────────────

fn binary_scan(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "binary", 2, args.len(),
            "scan value formatString ?varName ...?",
        ));
    }
    let fields = parse_fields(args[1].as_str())?;
    let mut data = string_to_bytes(args[0].as_str());
    let mut pos: usize = 0;
    let mut var_i: usize = 2;
    let mut sets: i64 = 0;

    for f in &fields {
        match f.ch {
            b'x' => {
                // tclsh: `x*` skips all remaining bytes; a bare `x` skips 1.
                pos += if f.star {
                    data.len() - pos
                } else {
                    f.count.unwrap_or(1)
                };
            }
            b'X' => pos = pos.saturating_sub(f.count.unwrap_or(1)),
            b'@' => {
                pos = f.count.unwrap_or(0);
                if pos > data.len() {
                    data.resize(pos, 0);
                }
            }
            b'a' | b'A' => {
                let count = f.count.unwrap_or_else(|| data.len().saturating_sub(pos));
                if pos + count > data.len() {
                    continue; // incomplete: variable untouched
                }
                let mut s = bytes_to_string(&data[pos..pos + count]);
                if f.ch == b'A' && f.count.is_none() {
                    s.truncate(s.trim_end().len());
                }
                let name = args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                interp.set_var(&name, Value::from_str(&s))?;
                sets += 1;
                var_i += 1;
                pos += count;
            }
            b'b' | b'B' => {
                // tclsh scan: an absent count reads ONE bit (unlike format,
                // which writes 8 by default).
                let count = if f.star { (data.len() - pos) * 8 } else { f.count.unwrap_or(1) };
                let need = count.div_ceil(8);
                if pos + need > data.len() {
                    continue;
                }
                let mut s = String::with_capacity(count);
                for k in 0..count {
                    let byte = data[pos + k / 8];
                    let bit = if f.ch == b'b' {
                        (byte >> (k % 8)) & 1
                    } else {
                        (byte >> (7 - (k % 8))) & 1
                    };
                    s.push(if bit == 1 { '1' } else { '0' });
                }
                let name = args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                interp.set_var(&name, Value::from_str(&s))?;
                sets += 1;
                var_i += 1;
                pos += need;
            }
            b'h' | b'H' => {
                // tclsh scan: an absent count reads ONE nibble.
                let count = if f.star { (data.len() - pos) * 2 } else { f.count.unwrap_or(1) };
                let need = count.div_ceil(2);
                if pos + need > data.len() {
                    continue;
                }
                let mut s = String::with_capacity(count);
                for k in 0..count {
                    let byte = data[pos + k / 2];
                    let nib = if f.ch == b'h' {
                        if k % 2 == 0 { byte & 0xf } else { byte >> 4 }
                    } else if k % 2 == 0 {
                        byte >> 4
                    } else {
                        byte & 0xf
                    };
                    s.push(char::from_digit(nib as u32, 16).unwrap());
                }
                let name = args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                interp.set_var(&name, Value::from_str(&s))?;
                sets += 1;
                var_i += 1;
                pos += need;
            }
            b'c' | b's' | b'S' | b't' | b'i' | b'I' | b'n' | b'w' | b'W' | b'm' => {
                let size: usize = match f.ch {
                    b'c' => 1,
                    b's' | b'S' | b't' => 2,
                    b'i' | b'I' | b'n' => 4,
                    _ => 8,
                };
                let big_endian = matches!(f.ch, b'S' | b'I' | b'W');
                let count = if f.star {
                    (data.len() - pos) / size
                } else {
                    f.count.unwrap_or(1)
                };
                if pos + size * count > data.len() {
                    continue;
                }
                let mut values: Vec<Value> = Vec::with_capacity(count);
                for g in 0..count {
                    let base = pos + g * size;
                    let mut v: u64 = 0;
                    for k in 0..size {
                        let b = data[base + k] as u64;
                        if big_endian {
                            v = (v << 8) | b;
                        } else {
                            v |= b << (8 * k);
                        }
                    }
                    let out = if f.unsigned {
                        match size {
                            1 => v as u8 as i64,
                            2 => v as u16 as i64,
                            4 => v as u32 as i64,
                            _ => v as i64,
                        }
                    } else {
                        match size {
                            1 => v as u8 as i8 as i64,
                            2 => v as u16 as i16 as i64,
                            4 => v as u32 as i32 as i64,
                            _ => v as i64,
                        }
                    };
                    values.push(Value::from_int(out));
                }
                let name = args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let val = if count == 1 { values.remove(0) } else { Value::from_list(&values) };
                interp.set_var(&name, val)?;
                sets += 1;
                var_i += 1;
                pos += size * count;
            }
            b'q' | b'Q' | b'd' | b'f' | b'r' | b'R' => {
                let size: usize = if matches!(f.ch, b'd' | b'q' | b'Q') { 8 } else { 4 };
                let big_endian = matches!(f.ch, b'Q' | b'R');
                let count = if f.star {
                    (data.len() - pos) / size
                } else {
                    f.count.unwrap_or(1)
                };
                if pos + size * count > data.len() {
                    continue;
                }
                let mut values: Vec<Value> = Vec::with_capacity(count);
                for g in 0..count {
                    let base = pos + g * size;
                    let mut bits: u64 = 0;
                    for k in 0..size {
                        let b = data[base + k] as u64;
                        if big_endian {
                            bits = (bits << 8) | b;
                        } else {
                            bits |= b << (8 * k);
                        }
                    }
                    let v = if size == 8 {
                        f64::from_bits(bits)
                    } else {
                        f32::from_bits(bits as u32) as f64
                    };
                    values.push(Value::from_float(v));
                }
                let name = args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let val = if count == 1 { values.remove(0) } else { Value::from_list(&values) };
                interp.set_var(&name, val)?;
                sets += 1;
                var_i += 1;
                pos += size * count;
            }
            _ => unreachable!(),
        }
    }
    Ok(Value::from_int(sets))
}

// ── encode ─────────────────────────────────────────────────────────────

fn binary_encode(args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(Error::wrong_args_with_usage(
            "binary", 1, args.len(),
            "encode subcommand ?arg ...?",
        ));
    }
    let sub = resolve_prefix(args[0].as_str(), &["base64", "hex", "uuencode"])
        .ok_or_else(|| {
            Error::runtime(
                format!(
                    "unknown subcommand \"{}\": must be base64, hex, or uuencode",
                    args[0].as_str()
                ),
                ErrorCode::Generic,
            )
        })?;
    match sub {
        "hex" => {
            if args.len() != 2 {
                return Err(Error::wrong_args_with_usage(
                    "binary", 2, args.len(),
                    "encode hex data",
                ));
            }
            let data = string_to_bytes(args[1].as_str());
            let mut s = String::with_capacity(data.len() * 2);
            for b in data {
                s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
                s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
            }
            Ok(Value::from_str(&s))
        }
        "base64" => {
            const USAGE: &str =
                "encode base64 ?-maxlen len? ?-wrapchar char? data";
            let mut maxlen: usize = 0;
            let mut wrapchar = "\n".to_string();
            let mut i = 1;
            while i < args.len() {
                let a = args[i].as_str();
                if let Some(opt) = a.strip_prefix('-') {
                    match resolve_prefix(opt, &["maxlen", "wrapchar"]) {
                        Some("maxlen") => {
                            i += 1;
                            maxlen = args
                                .get(i)
                                .and_then(|v| v.as_int())
                                .ok_or_else(|| {
                                    Error::runtime(
                                        format!(
                                            "expected integer but got \"{}\"",
                                            args.get(i)
                                                .map(|v| v.as_str())
                                                .unwrap_or("")
                                        ),
                                        ErrorCode::Generic,
                                    )
                                })? as usize;
                        }
                        Some("wrapchar") => {
                            i += 1;
                            wrapchar = args
                                .get(i)
                                .map(|v| v.as_str().to_string())
                                .ok_or_else(|| {
                                    Error::runtime(
                                        "missing wrapchar argument",
                                        ErrorCode::Generic,
                                    )
                                })?;
                        }
                        _ => {
                            return Err(Error::runtime(
                                format!(
                                    "bad option \"{}\": must be -maxlen or -wrapchar",
                                    a
                                ),
                                ErrorCode::Generic,
                            ));
                        }
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            if args.len() - i != 1 {
                return Err(Error::wrong_args_with_usage(
                    "binary", i + 1, args.len(), USAGE,
                ));
            }
            let data = string_to_bytes(args[i].as_str());
            let encoded = base64_encode(&data);
            Ok(Value::from_str(&wrap_encoded(&encoded, maxlen, &wrapchar)))
        }
        _ => {
            // uuencode
            if args.len() != 2 {
                return Err(Error::wrong_args_with_usage(
                    "binary", 2, args.len(),
                    "encode uuencode data",
                ));
            }
            let data = string_to_bytes(args[1].as_str());
            let mut out = String::new();
            for chunk in data.chunks(45) {
                let n = chunk.len();
                out.push(uu_char(n as u64));
                let groups = (n * 8).div_ceil(6);
                for g in 0..groups {
                    let mut v: u64 = 0;
                    for b in 0..6 {
                        let bit_idx = g * 6 + b;
                        v <<= 1;
                        if bit_idx < n * 8 {
                            let byte = chunk[bit_idx / 8];
                            v |= ((byte >> (7 - (bit_idx % 8))) & 1) as u64;
                        }
                    }
                    out.push(uu_char(v));
                }
                out.push('\n');
            }
            Ok(Value::from_str(&out))
        }
    }
}

/// uuencode alphabet: value + 0x20, with 0 rendered as '`' (0x60).
fn uu_char(v: u64) -> char {
    let c = v as u8 + 0x20;
    char::from_u32(if c == 0x20 { 0x60 } else { c as u32 }).unwrap()
}

fn base64_encode(data: &[u8]) -> String {
    const TBL: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Insert `wrapchar` between every `maxlen` encoded characters (no trailing
/// separator; maxlen 0 → no wrapping).
fn wrap_encoded(encoded: &str, maxlen: usize, wrapchar: &str) -> String {
    if maxlen == 0 || wrapchar.is_empty() {
        return encoded.to_string();
    }
    let chars: Vec<char> = encoded.chars().collect();
    let mut out = String::with_capacity(encoded.len() + (chars.len() / maxlen) * wrapchar.len());
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && i % maxlen == 0 {
            out.push_str(wrapchar);
        }
        out.push(*c);
    }
    out
}

// ── decode ─────────────────────────────────────────────────────────────

fn binary_decode(args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(Error::wrong_args_with_usage(
            "binary", 1, args.len(),
            "decode subcommand ?arg ...?",
        ));
    }
    let sub = resolve_prefix(args[0].as_str(), &["base64", "hex", "uuencode"])
        .ok_or_else(|| {
            Error::runtime(
                format!(
                    "unknown subcommand \"{}\": must be base64, hex, or uuencode",
                    args[0].as_str()
                ),
                ErrorCode::Generic,
            )
        })?;
    let cmd = format!("binary decode {}", sub);
    let usage = match sub {
        "hex" => "decode hex ?options? data",
        "base64" => "decode base64 ?options? data",
        _ => "decode uuencode ?options? data",
    };
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage(
            &cmd, 2, args.len(), usage,
        ));
    }
    let input = args[1].as_str();
    let bytes = match sub {
        "hex" => decode_hex(input)?,
        "base64" => decode_base64(input),
        _ => decode_uuencode(input)?,
    };
    Ok(Value::from_str(&bytes_to_string(&bytes)))
}

fn decode_hex(input: &str) -> Result<Vec<u8>> {
    let mut nibbles: Vec<u8> = Vec::with_capacity(input.len());
    for (i, c) in input.chars().enumerate() {
        match c.to_digit(16) {
            Some(d) => nibbles.push(d as u8),
            None => {
                return Err(Error::runtime(
                    format!("invalid hexadecimal digit \"{}\" at position {}", c, i),
                    ErrorCode::Generic,
                ));
            }
        }
    }
    let mut out = Vec::with_capacity(nibbles.len().div_ceil(2));
    let mut k = 0;
    while k + 1 < nibbles.len() {
        out.push((nibbles[k] << 4) | nibbles[k + 1]);
        k += 2;
    }
    if k < nibbles.len() {
        out.push(nibbles[k]);
    }
    Ok(out)
}

fn decode_base64(input: &str) -> Vec<u8> {
    // tclsh silently skips whitespace, '=', and any other non-alphabet char.
    let mut vals: Vec<u8> = Vec::new();
    for c in input.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => continue,
        } as u8;
        vals.push(v);
    }
    let mut out = Vec::with_capacity(vals.len() * 3 / 4);
    let mut k = 0;
    while k + 1 < vals.len() {
        let have = (vals.len() - k).min(4);
        let mut n: u32 = vals[k] as u32;
        for j in 1..4 {
            n = (n << 6) | *vals.get(k + j).unwrap_or(&0) as u32;
        }
        out.push((n >> 16) as u8);
        if have > 2 {
            out.push((n >> 8) as u8);
        }
        if have > 3 {
            out.push(n as u8);
        }
        k += 4;
    }
    out
}

fn decode_uuencode(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for line in input.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let len = ((chars[0] as u32).wrapping_sub(0x20) & 0x3f) as usize;
        let mut vals: Vec<u8> = Vec::with_capacity(chars.len().saturating_sub(1));
        for (i, &c) in chars.iter().enumerate().skip(1) {
            let u = (c as u32).wrapping_sub(0x20);
            if u > 0x3f {
                return Err(Error::runtime(
                    format!("invalid uuencode character \"{}\" at position {}", c, i - 1),
                    ErrorCode::Generic,
                ));
            }
            vals.push(u as u8);
        }
        let need = len.div_ceil(3) * 4;
        if vals.len() < need {
            return Err(Error::runtime(
                "uuencode line is truncated",
                ErrorCode::Generic,
            ));
        }
        for g in 0..len.div_ceil(3) {
            let n: u32 = ((vals[g * 4] as u32) << 18)
                | ((vals[g * 4 + 1] as u32) << 12)
                | ((vals[g * 4 + 2] as u32) << 6)
                | (vals[g * 4 + 3] as u32);
            for b in 0..3 {
                if g * 3 + b < len {
                    out.push((n >> (16 - 8 * b)) as u8);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod tests;
