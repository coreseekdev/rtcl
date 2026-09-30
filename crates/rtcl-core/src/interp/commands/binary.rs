//! The `binary` command — format/scan/encode/decode (tclsh 8.6.17).
//!
//! Byte ↔ string mapping follows tclsh: a string contributes one byte per
//! character (the character's low byte), and a byte string renders as one
//! character per byte (latin-1 style).  This keeps `binary format`/`scan`
//! results round-trippable through ordinary string values.
//!
//! Field parsing and the per-field semantics port tclBinary.c (GetFormatSpec,
//! BinaryFormatCmd, BinaryScanCmd); encode/decode port BinaryEncode64/Uu and
//! BinaryDecodeHex/64/Uu, including option-pair parsing, strict mode, and
//! error message/position details verified against tclsh 8.6.17.

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

/// Maximum Tcl value size; larger format results are refused (8.6.17:
/// "max size for a Tcl value (2147483647 bytes) exceeded").
const MAXOBJ: usize = 2147483647;

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

/// Tcl's isspace set (TclIsSpaceProc).
fn is_tcl_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn max_size_err() -> Error {
    Error::runtime(
        "max size for a Tcl value (2147483647 bytes) exceeded",
        ErrorCode::Generic,
    )
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

/// Valid field specifier characters (tclsh 8.6 — no `C`).
const FIELD_CHARS: &[u8] = b"aAbBhHcsStiInWwmqQdfrRxX@";

/// Parse a format string into fields — port of GetFormatSpec: fields are
/// separated by spaces; each is a letter, optional `u` modifier, then
/// either `*` or a decimal count.  `@` requires a count (`@*` ok).
fn parse_fields(fmt: &str) -> Result<Vec<Field>> {
    let bytes = fmt.as_bytes();
    let mut fields = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b' ' {
            i += 1;
            continue;
        }
        if !FIELD_CHARS.contains(&ch) {
            return Err(Error::runtime(
                format!("bad field specifier \"{}\"", ch as char),
                ErrorCode::Generic,
            ));
        }
        i += 1;
        let mut field = Field { ch, count: None, star: false, unsigned: false };
        if i < bytes.len() && bytes[i] == b'u' && ch != b'@' {
            field.unsigned = true;
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'*' {
            field.star = true;
            i += 1;
        } else {
            let mut count: usize = 0;
            let mut have_count = false;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                count = count
                    .saturating_mul(10)
                    .saturating_add((bytes[i] - b'0') as usize);
                have_count = true;
                i += 1;
            }
            if have_count {
                field.count = Some(count);
            }
        }
        if ch == b'@' && !field.star && field.count.is_none() {
            return Err(Error::runtime(
                "missing count for \"@\" field specifier",
                ErrorCode::Generic,
            ));
        }
        fields.push(field);
    }
    Ok(fields)
}

fn int_size(ch: u8) -> usize {
    match ch {
        b'c' => 1,
        b's' | b'S' | b't' => 2,
        b'i' | b'I' | b'n' => 4,
        _ => 8,
    }
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

fn expected_int(v: &Value) -> Error {
    Error::runtime(
        format!("expected integer but got \"{}\"", v.as_str()),
        ErrorCode::Generic,
    )
}

fn expected_float(v: &Value) -> Error {
    Error::runtime(
        format!("expected floating-point number but got \"{}\"", v.as_str()),
        ErrorCode::Generic,
    )
}

fn list_count_mismatch() -> Error {
    Error::runtime(
        "number of elements in list does not match count",
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
    // High-water mark of `pos`, consulted by `X`/`@` (maxPos in tclBinary.c;
    // it is only refreshed on entry to those two cases).
    let mut max_pos: usize = 0;
    let mut argi: usize = 1;

    for f in &fields {
        match f.ch {
            b'a' | b'A' => {
                let arg = args.get(argi).ok_or_else(not_enough_args)?;
                argi += 1;
                let chars: Vec<char> = arg.as_str().chars().collect();
                let count = if f.star { chars.len() } else { f.count.unwrap_or(1) };
                if pos.saturating_add(count) > MAXOBJ {
                    return Err(max_size_err());
                }
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
                if pos.saturating_add(need) > MAXOBJ {
                    return Err(max_size_err());
                }
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
                if pos.saturating_add(count.div_ceil(2)) > MAXOBJ {
                    return Err(max_size_err());
                }
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
                if f.star {
                    return Err(Error::runtime(
                        "cannot use \"*\" in format string with \"x\"",
                        ErrorCode::Generic,
                    ));
                }
                let n = f.count.unwrap_or(1);
                if pos.saturating_add(n) > MAXOBJ {
                    return Err(max_size_err());
                }
                if pos + n > out.len() {
                    out.resize(pos + n, 0);
                }
                pos += n;
            }
            b'X' => {
                max_pos = max_pos.max(pos);
                let n = f.count.unwrap_or(1);
                if f.star || n > pos {
                    pos = 0;
                } else {
                    pos -= n;
                }
            }
            b'@' => {
                max_pos = max_pos.max(pos);
                pos = if f.star { max_pos } else { f.count.unwrap_or(0) };
            }
            b'c' | b's' | b'S' | b't' | b'i' | b'I' | b'n' | b'w' | b'W' | b'm' => {
                let size = int_size(f.ch);
                let big_endian = matches!(f.ch, b'S' | b'I' | b'W');
                let values = format_int_values(args, &mut argi, f)?;
                let total = size.saturating_mul(values.len());
                if pos.saturating_add(total) > MAXOBJ {
                    return Err(max_size_err());
                }
                let mut bytes = Vec::with_capacity(total);
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
                let values = format_float_values(args, &mut argi, f)?;
                let total = size.saturating_mul(values.len());
                if pos.saturating_add(total) > MAXOBJ {
                    return Err(max_size_err());
                }
                let mut bytes = Vec::with_capacity(total);
                for v in values {
                    // tclsh clamps 4-byte fields to ±FLT_MAX (even ±Inf);
                    // a plain cast would overflow to Inf.
                    let bits = if size == 8 {
                        v.to_bits()
                    } else {
                        let fmax = f32::MAX as f64;
                        let f = if v.is_nan() {
                            f32::NAN
                        } else if v > fmax {
                            f32::MAX
                        } else if v < -fmax {
                            -f32::MAX
                        } else {
                            v as f32
                        };
                        f.to_bits() as u64
                    };
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

/// Select the value(s) a numeric format field consumes.  Without a count the
/// argument itself is the single scalar; with a count (or `*`) the argument
/// is a list — a short list is an error, extra elements are ignored.
fn select_numeric_args(
    args: &[Value],
    argi: &mut usize,
    f: &Field,
) -> Result<Vec<Value>> {
    let arg = args.get(*argi).ok_or_else(not_enough_args)?;
    *argi += 1;
    if f.count.is_none() && !f.star {
        return Ok(vec![arg.clone()]);
    }
    let list = arg.as_list().ok_or_else(list_count_mismatch)?;
    let count = if f.star { list.len() } else { f.count.unwrap_or(0) };
    if list.len() < count {
        return Err(list_count_mismatch());
    }
    Ok(list.into_iter().take(count).collect())
}

fn format_int_values(args: &[Value], argi: &mut usize, f: &Field) -> Result<Vec<i64>> {
    select_numeric_args(args, argi, f)?
        .iter()
        .map(|v| v.as_int().ok_or_else(|| expected_int(v)))
        .collect()
}

fn format_float_values(args: &[Value], argi: &mut usize, f: &Field) -> Result<Vec<f64>> {
    select_numeric_args(args, argi, f)?
        .iter()
        .map(|v| v.as_float().ok_or_else(|| expected_float(v)))
        .collect()
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
    let data = string_to_bytes(args[0].as_str());
    let mut pos: usize = 0;
    let mut var_i: usize = 2;
    let mut sets: i64 = 0;

    for f in &fields {
        match f.ch {
            b'x' => {
                let n = if f.star { usize::MAX } else { f.count.unwrap_or(1) };
                if f.star || n > data.len() - pos {
                    pos = data.len();
                } else {
                    pos += n;
                }
            }
            b'X' => {
                let n = if f.star { usize::MAX } else { f.count.unwrap_or(1) };
                if f.star || n > pos {
                    pos = 0;
                } else {
                    pos -= n;
                }
            }
            b'@' => {
                pos = if f.star {
                    data.len()
                } else {
                    f.count.unwrap_or(0).min(data.len())
                };
            }
            b'a' | b'A' => {
                let name =
                    args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let count = if f.star { data.len() - pos } else { f.count.unwrap_or(1) };
                if count > data.len() - pos {
                    break; // short data: scanning stops, variable untouched
                }
                let mut size = count;
                if f.ch == b'A' {
                    while size > 0 && matches!(data[pos + size - 1], 0 | b' ') {
                        size -= 1;
                    }
                }
                let s = bytes_to_string(&data[pos..pos + size]);
                interp.set_var(&name, Value::from_str(&s))?;
                sets += 1;
                var_i += 1;
                pos += count;
            }
            b'b' | b'B' => {
                // tclsh scan: an absent count reads ONE bit (unlike format,
                // which writes 8 by default).
                let name =
                    args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let count = if f.star {
                    (data.len() - pos) * 8
                } else {
                    f.count.unwrap_or(1)
                };
                if count > (data.len() - pos) * 8 {
                    break;
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
                interp.set_var(&name, Value::from_str(&s))?;
                sets += 1;
                var_i += 1;
                pos += count.div_ceil(8);
            }
            b'h' | b'H' => {
                // tclsh scan: an absent count reads ONE nibble.
                let name =
                    args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let count = if f.star {
                    (data.len() - pos) * 2
                } else {
                    f.count.unwrap_or(1)
                };
                if count > (data.len() - pos) * 2 {
                    break;
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
                interp.set_var(&name, Value::from_str(&s))?;
                sets += 1;
                var_i += 1;
                pos += count.div_ceil(2);
            }
            b'c' | b's' | b'S' | b't' | b'i' | b'I' | b'n' | b'w' | b'W' | b'm' => {
                let size = int_size(f.ch);
                let big_endian = matches!(f.ch, b'S' | b'I' | b'W');
                let name =
                    args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let count = if f.star {
                    (data.len() - pos) / size
                } else {
                    f.count.unwrap_or(1)
                };
                if data.len() - pos < count.saturating_mul(size) {
                    break;
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
                let val =
                    if count == 1 { values.remove(0) } else { Value::from_list(&values) };
                interp.set_var(&name, val)?;
                sets += 1;
                var_i += 1;
                pos += size * count;
            }
            b'q' | b'Q' | b'd' | b'f' | b'r' | b'R' => {
                let size: usize = if matches!(f.ch, b'd' | b'q' | b'Q') { 8 } else { 4 };
                let big_endian = matches!(f.ch, b'Q' | b'R');
                let name =
                    args.get(var_i).ok_or_else(not_enough_args)?.as_str().to_string();
                let count = if f.star {
                    (data.len() - pos) / size
                } else {
                    f.count.unwrap_or(1)
                };
                if data.len() - pos < count.saturating_mul(size) {
                    break;
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
                let val =
                    if count == 1 { values.remove(0) } else { Value::from_list(&values) };
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
            if args.len() < 2 || args.len() % 2 != 0 {
                return Err(Error::wrong_args_with_usage(
                    "binary", 2, args.len(),
                    "encode base64 ?-maxlen len? ?-wrapchar char? data",
                ));
            }
            let (maxlen, wrapchar) = parse_encode_opts(args, false)?;
            let data = string_to_bytes(args[args.len() - 1].as_str());
            let encoded = base64_encode(&data);
            Ok(Value::from_str(&wrap_encoded(&encoded, maxlen, &wrapchar)))
        }
        _ => {
            // uuencode
            if args.len() < 2 || args.len() % 2 != 0 {
                return Err(Error::wrong_args_with_usage(
                    "binary", 2, args.len(),
                    "encode uuencode ?-maxlen len? ?-wrapchar char? data",
                ));
            }
            let (line_length, wrapchar) = parse_encode_opts(args, true)?;
            // Bytes of raw data encoded per line (line_length includes the
            // leading count character).
            let raw_len = (line_length - 1) * 3 / 4;
            let data = string_to_bytes(args[args.len() - 1].as_str());
            let mut out = String::new();
            let mut offset = 0;
            while offset < data.len() {
                let line_bytes = raw_len.min(data.len() - offset);
                out.push(uu_char(line_bytes as u64));
                let mut n: u64 = 0;
                let mut bits = 0u32;
                for _ in 0..line_bytes {
                    n = (n << 8) | data[offset] as u64;
                    offset += 1;
                    bits += 8;
                    while bits > 6 {
                        bits -= 6;
                        out.push(uu_char((n >> bits) & 0x3F));
                    }
                }
                if bits > 0 {
                    out.push(uu_char(((n << 8) >> (bits + 2)) & 0x3F));
                }
                out.push_str(&wrapchar);
            }
            Ok(Value::from_str(&out))
        }
    }
}

/// Parse `-maxlen`/`-wrapchar` option pairs (exact option names, per
/// tclBinary.c BinaryEncode64/Uu).  For base64 the length is a count of
/// encoded characters per line (negative → error); for uuencode it is the
/// total line length including the count character, restricted to 5..85 and
/// snapped to 5, 9, 13 …, and the wrapchar may only contain \t \v \f \r \n.
fn parse_encode_opts(args: &[Value], uu: bool) -> Result<(usize, String)> {
    let mut maxlen: i64 = if uu { 61 } else { 0 };
    let mut wrapchar = "\n".to_string();
    let mut i = 1;
    while i < args.len() - 1 {
        match args[i].as_str() {
            "-maxlen" => {
                let v = args
                    .get(i + 1)
                    .and_then(|v| v.as_int())
                    .ok_or_else(|| {
                        Error::runtime(
                            format!(
                                "expected integer but got \"{}\"",
                                args.get(i + 1).map(|v| v.as_str()).unwrap_or("")
                            ),
                            ErrorCode::Generic,
                        )
                    })?;
                if uu {
                    if !(5..=85).contains(&v) {
                        return Err(Error::runtime(
                            "line length out of range",
                            ErrorCode::Generic,
                        ));
                    }
                    maxlen = ((v - 1) & !3) + 1;
                } else {
                    if v < 0 {
                        return Err(Error::runtime(
                            "line length out of range",
                            ErrorCode::Generic,
                        ));
                    }
                    maxlen = v;
                }
            }
            "-wrapchar" => {
                wrapchar = args
                    .get(i + 1)
                    .map(|v| v.as_str().to_string())
                    .ok_or_else(|| {
                        Error::runtime("missing wrapchar argument", ErrorCode::Generic)
                    })?;
                if uu
                    && !wrapchar
                        .chars()
                        .all(|c| matches!(c, '\t' | '\u{b}' | '\u{c}' | '\r' | '\n'))
                {
                    return Err(Error::runtime(
                        "invalid wrapchar; will defeat decoding",
                        ErrorCode::Generic,
                    ));
                }
            }
            other => {
                return Err(Error::runtime(
                    format!(
                        "bad option \"{}\": must be -maxlen or -wrapchar",
                        other
                    ),
                    ErrorCode::Generic,
                ));
            }
        }
        i += 2;
    }
    if !uu && wrapchar.is_empty() {
        maxlen = 0;
    }
    Ok((maxlen as usize, wrapchar))
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
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "binary", 2, args.len(),
            &format!("decode {} ?options? data", sub),
        ));
    }
    let mut strict = false;
    for a in &args[1..args.len() - 1] {
        match a.as_str() {
            "-strict" => strict = true,
            other => {
                return Err(Error::runtime(
                    format!("bad option \"{}\": must be -strict", other),
                    ErrorCode::Generic,
                ));
            }
        }
    }
    let input = args[args.len() - 1].as_str();
    let bytes = match sub {
        "hex" => decode_hex(input, strict)?,
        "base64" => decode_base64(input, strict)?,
        _ => decode_uuencode(input, strict)?,
    };
    Ok(Value::from_str(&bytes_to_string(&bytes)))
}

fn invalid_hex_digit(c: u8, pos: usize) -> Error {
    Error::runtime(
        format!(
            "invalid hexadecimal digit \"{}\" at position {}",
            c as char, pos
        ),
        ErrorCode::Generic,
    )
}

fn invalid_char(kind: &str, c: u8, pos: usize) -> Error {
    Error::runtime(
        format!("invalid {} character \"{}\" at position {}", kind, c as char, pos),
        ErrorCode::Generic,
    )
}

/// Hex decode: whitespace is skipped, hex digit pairs become bytes, and a
/// trailing lone nibble is dropped (even with -strict; tclsh 8.6.17).
fn decode_hex(input: &str, strict: bool) -> Result<Vec<u8>> {
    let data = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(data.len() / 2 + 1);
    let mut i = 0;
    let mut cut = 0usize;
    while i < data.len() {
        let mut value: u8 = 0;
        let mut nib = 0;
        while nib < 2 {
            if i >= data.len() {
                value <<= 4;
                break;
            }
            let c = data[i];
            i += 1;
            if !(c as char).is_ascii_hexdigit() {
                if strict || !is_tcl_space(c) {
                    return Err(invalid_hex_digit(c, i - 1));
                }
                continue; // whitespace does not count toward the pair
            }
            value = value.wrapping_shl(4) | (c as char).to_digit(16).unwrap() as u8;
            nib += 1;
        }
        if nib < 2 {
            cut += 1;
        }
        out.push(value);
    }
    let keep = out.len().saturating_sub(cut);
    out.truncate(keep);
    Ok(out)
}

/// Base64 decode — port of BinaryDecode64: groups of four alphabet chars;
/// `=` only cuts within the final group; in non-strict mode any other
/// character (and padding oddities) is skipped.
fn decode_base64(input: &str, strict: bool) -> Result<Vec<u8>> {
    let data = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 3 / 4 + 3);
    let mut pos = 0;
    let mut cut = 0usize;
    while pos < data.len() {
        let mut value: u32 = 0;
        let mut i = 0;
        while i < 4 {
            let c: u8;
            if pos < data.len() {
                c = data[pos];
                pos += 1;
            } else if i > 1 {
                c = b'=';
            } else {
                // Input ends 1–2 chars into the group.
                if strict {
                    return Err(invalid_char("base64", data[data.len() - 1], data.len() - 1));
                }
                cut += 3;
                break;
            }
            if cut > 0 {
                if c == b'=' && i > 1 {
                    value <<= 6;
                    cut += 1;
                } else if !strict {
                    i -= 1;
                } else {
                    return Err(invalid_char("base64", c, pos - 1));
                }
            } else if c.is_ascii_uppercase() {
                value = (value << 6) | (c - b'A') as u32;
            } else if c.is_ascii_lowercase() {
                value = (value << 6) | (c - b'a') as u32 + 26;
            } else if c.is_ascii_digit() {
                value = (value << 6) | (c - b'0') as u32 + 52;
            } else if c == b'+' {
                value = (value << 6) | 0x3E;
            } else if c == b'/' {
                value = (value << 6) | 0x3F;
            } else if c == b'=' && (!strict || i > 1) {
                value <<= 6;
                if i > 0 {
                    cut += 1;
                }
            } else if strict {
                return Err(invalid_char("base64", c, pos - 1));
            } else {
                i -= 1;
            }
            i += 1;
        }
        out.push((value >> 16) as u8);
        out.push((value >> 8) as u8);
        out.push(value as u8);
        if cut > 0 && pos < data.len() && strict {
            return Err(invalid_char("base64", data[pos], pos));
        }
    }
    let keep = out.len().saturating_sub(cut);
    out.truncate(keep);
    Ok(out)
}

/// Uuencode decode — port of BinaryDecodeUu.  Each line starts with a
/// length character (value (c-32)&0x3F, chars 0x20..=0x60 valid); group
/// characters beyond the input act as zero bytes, so short trailing groups
/// decode the bytes they still can.  Without -strict invalid characters are
/// skipped; with -strict they are errors, and a newline inside a group (or
/// a line that ends short) is "short uuencode data".
fn decode_uuencode(input: &str, strict: bool) -> Result<Vec<u8>> {
    let data = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 3 / 4 + 3);
    let mut pos = 0;
    let mut line_len: i64 = -1;
    let group_val = |b: u8| -> u32 { (((b as i32) - 32) & 0x3F) as u32 };

    'outer: while pos < data.len() {
        let mut d = [0u8; 4];
        if line_len < 0 {
            // Read the line length character.
            let c = loop {
                if pos >= data.len() {
                    break 'outer;
                }
                let c = data[pos];
                pos += 1;
                if !(32..=96).contains(&c) {
                    if strict || !is_tcl_space(c) {
                        return Err(invalid_char("uuencode", c, pos - 1));
                    }
                    continue;
                }
                break c;
            };
            line_len = i64::from(((c as i32) - 32) & 0x3F);
        }
        // Read one four-character grouping.
        let mut i = 0;
        while i < 4 {
            if pos >= data.len() {
                break;
            }
            let c = data[pos];
            pos += 1;
            d[i] = c;
            if (32..=96).contains(&c) {
                i += 1;
                continue;
            }
            if strict {
                if !is_tcl_space(c) {
                    return Err(invalid_char("uuencode", c, pos - 1));
                }
                if c == b'\n' {
                    return Err(Error::runtime(
                        "short uuencode data",
                        ErrorCode::Generic,
                    ));
                }
            }
            // invalid char skipped: d[i] is rewritten by the next char
        }
        // Translate the grouping into up to three bytes.
        if line_len > 0 {
            out.push((group_val(d[0]) << 2 | group_val(d[1]) >> 4) as u8);
            line_len -= 1;
            if line_len > 0 {
                out.push((group_val(d[1]) << 4 | group_val(d[2]) >> 2) as u8);
                line_len -= 1;
                if line_len > 0 {
                    out.push((group_val(d[2]) << 6 | group_val(d[3])) as u8);
                    line_len -= 1;
                }
            }
        }
        // At the end of a line, skip until a newline (a valid character
        // rewinds and starts the next line — newline-less concatenation).
        if line_len == 0 && pos < data.len() {
            line_len = -1;
            while pos < data.len() {
                let c = data[pos];
                pos += 1;
                if c == b'\n' {
                    break;
                }
                if (32..=96).contains(&c) {
                    pos -= 1;
                    break;
                }
                if strict || !is_tcl_space(c) {
                    return Err(invalid_char("uuencode", c, pos - 1));
                }
            }
        }
    }
    if line_len > 0 && strict {
        return Err(Error::runtime("short uuencode data", ErrorCode::Generic));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod tests;
