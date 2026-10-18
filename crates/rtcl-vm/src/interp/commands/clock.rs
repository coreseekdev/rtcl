//! Clock commands: clock seconds/clicks/microseconds/milliseconds/format/scan.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

pub fn cmd_clock(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "clock", 2, args.len(),
            "clock subcommand ?arg ...?",
        ));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        "seconds" => clock_seconds(),
        "clicks" => clock_clicks(args),
        "microseconds" => clock_microseconds(),
        "milliseconds" => clock_milliseconds(),
        "format" => clock_format(args),
        "scan" => clock_scan(args),
        _ => Err(Error::runtime(
            format!("unknown clock subcommand \"{}\": must be clicks, format, microseconds, milliseconds, scan, or seconds", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

fn clock_seconds() -> Result<Value> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(Value::from_int(secs as i64))
}

fn clock_clicks(args: &[Value]) -> Result<Value> {
    // `clock clicks` returns high-resolution timer in microseconds by default
    // `clock clicks -milliseconds` returns milliseconds
    let millis = args.get(2).map(|a| a.as_str() == "-milliseconds").unwrap_or(false);
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    if millis {
        Ok(Value::from_int(dur.as_millis() as i64))
    } else {
        Ok(Value::from_int(dur.as_micros() as i64))
    }
}

fn clock_microseconds() -> Result<Value> {
    let us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    Ok(Value::from_int(us as i64))
}

fn clock_milliseconds() -> Result<Value> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(Value::from_int(ms as i64))
}

/// `clock format seconds ?-format fmt? ?-gmt bool?`
fn clock_format(args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "clock format", 3, args.len(),
            "clock format seconds ?-format fmt? ?-gmt bool?",
        ));
    }

    let seconds = args[2].as_int().ok_or_else(|| {
        Error::runtime(
            format!("expected integer but got \"{}\"", args[2].as_str()),
            crate::error::ErrorCode::Generic,
        )
    })?;

    let mut format_str = "%a %b %d %H:%M:%S %Z %Y".to_string();
    let mut use_gmt = false;
    let mut i = 3;

    while i < args.len() {
        match args[i].as_str() {
            "-format" => {
                i += 1;
                if i < args.len() {
                    format_str = args[i].as_str().to_string();
                }
                i += 1;
            }
            "-gmt" => {
                i += 1;
                if i < args.len() {
                    use_gmt = matches!(args[i].as_str(), "1" | "true" | "yes");
                }
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    // Simple strftime-like formatting using pure Rust
    let result = format_timestamp(seconds, &format_str, use_gmt);
    Ok(Value::from_str(&result))
}

/// `clock scan string -format fmt ?-gmt bool?`
fn clock_scan(args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "clock scan", 3, args.len(),
            "clock scan string -format fmt ?-gmt bool?",
        ));
    }

    let _input = args[2].as_str();
    let mut _format_str = String::new();
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "-format" => {
                i += 1;
                if i < args.len() {
                    _format_str = args[i].as_str().to_string();
                }
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    // Basic scan: if the input looks like an integer, treat it as epoch seconds
    if let Some(n) = args[2].as_int() {
        return Ok(Value::from_int(n));
    }

    Err(Error::runtime(
        "clock scan: format parsing not yet implemented",
        crate::error::ErrorCode::InvalidOp,
    ))
}

// ── Pure-Rust strftime-like formatter ──────────────────────────────────

/// Break an epoch timestamp into calendar components.
struct DateTime {
    year: i64,
    month: u32,   // 1..=12
    day: u32,     // 1..=31
    hour: u32,    // 0..=23
    minute: u32,  // 0..=59
    second: u32,  // 0..=59
    wday: u32,    // 0=Sunday
    yday: u32,    // 0..=365
}

fn epoch_to_datetime(epoch: i64, _gmt: bool) -> DateTime {
    // Civil-calendar algorithm (UTC only for now)
    let secs = epoch;
    let days = secs.div_euclid(86400);
    let day_secs = secs.rem_euclid(86400) as u32;

    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    // Days since 0000-03-01 (March epoch trick)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u32;           // day of era
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    // Day of week: 0=Sunday
    let wday = ((days + 4).rem_euclid(7)) as u32; // 1970-01-01 is Thursday (4)

    // Day of year
    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days: [u32; 12] = [31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut yday = d;
    for &md in &month_days[..(m as usize - 1)] {
        yday += md;
    }

    DateTime { year, month: m, day: d, hour, minute, second, wday, yday: yday - 1 }
}

fn format_timestamp(epoch: i64, fmt: &str, gmt: bool) -> String {
    let dt = epoch_to_datetime(epoch, gmt);
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;

    let short_days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let full_days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let short_months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let full_months = ["January", "February", "March", "April", "May", "June",
                       "July", "August", "September", "October", "November", "December"];

    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'Y' => result.push_str(&format!("{:04}", dt.year)),
                'y' => result.push_str(&format!("{:02}", dt.year % 100)),
                'm' => result.push_str(&format!("{:02}", dt.month)),
                'd' => result.push_str(&format!("{:02}", dt.day)),
                'e' => result.push_str(&format!("{:>2}", dt.day)),
                'H' => result.push_str(&format!("{:02}", dt.hour)),
                'I' => {
                    let h = if dt.hour == 0 { 12 } else if dt.hour > 12 { dt.hour - 12 } else { dt.hour };
                    result.push_str(&format!("{:02}", h));
                }
                'M' => result.push_str(&format!("{:02}", dt.minute)),
                'S' => result.push_str(&format!("{:02}", dt.second)),
                'p' => result.push_str(if dt.hour < 12 { "AM" } else { "PM" }),
                'P' => result.push_str(if dt.hour < 12 { "am" } else { "pm" }),
                'a' => result.push_str(short_days[dt.wday as usize]),
                'A' => result.push_str(full_days[dt.wday as usize]),
                'b' | 'h' => result.push_str(short_months[(dt.month - 1) as usize]),
                'B' => result.push_str(full_months[(dt.month - 1) as usize]),
                'j' => result.push_str(&format!("{:03}", dt.yday + 1)),
                'w' => result.push_str(&format!("{}", dt.wday)),
                'Z' => result.push_str(if gmt { "GMT" } else { "UTC" }),
                'n' => result.push('\n'),
                't' => result.push('\t'),
                '%' => result.push('%'),
                's' => result.push_str(&format!("{}", epoch)),
                'c' => {
                    // Locale date/time: %a %b %e %H:%M:%S %Y
                    result.push_str(&format!(
                        "{} {} {:>2} {:02}:{:02}:{:02} {:04}",
                        short_days[dt.wday as usize],
                        short_months[(dt.month - 1) as usize],
                        dt.day, dt.hour, dt.minute, dt.second, dt.year
                    ));
                }
                'D' => {
                    // %m/%d/%y
                    result.push_str(&format!("{:02}/{:02}/{:02}", dt.month, dt.day, dt.year % 100));
                }
                'T' => {
                    // %H:%M:%S
                    result.push_str(&format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second));
                }
                other => { result.push('%'); result.push(other); }
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
}
