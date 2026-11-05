//! Clock commands: clock seconds/clicks/microseconds/milliseconds/format/scan/add.
//!
//! D-25（2026-11-06）：`clock add` 落地 + `-timezone` 解析（format/scan/add
//! 共用；unix 下经 libc TZ/localtime_r/mktime，zoneinfo 文件与 tclsh 同源
//! ——实测 tclsh 的 -timezone 输出与 `TZ=<zone> date` 逐字一致）。
//! 自由格式 `clock scan`（"next Friday"/相对短语）仍是残差。

use crate::error::{Error, Result};
use crate::interp::commands::list::set_error_code;
use crate::interp::Interp;
use crate::value::Value;

const CLOCK_SUBCMDS: &str = "add, clicks, format, microseconds, milliseconds, scan, or seconds";

pub fn cmd_clock(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "clock", 2, args.len(),
            "subcommand ?arg ...?",
        ));
    }

    let subcmd = args[1].as_str();
    match resolve_subcommand(subcmd) {
        Some("seconds") => {
            if args.len() != 2 {
                return Err(usage_err(interp, "clock seconds".to_string(), None));
            }
            clock_seconds()
        }
        Some("clicks") => clock_clicks(interp, args),
        Some("microseconds") => {
            if args.len() != 2 {
                return Err(usage_err(interp, "clock microseconds".to_string(), None));
            }
            clock_microseconds()
        }
        Some("milliseconds") => {
            if args.len() != 2 {
                return Err(usage_err(interp, "clock milliseconds".to_string(), None));
            }
            clock_milliseconds()
        }
        Some("format") => clock_format(interp, args),
        Some("scan") => clock_scan(interp, args),
        Some("add") => clock_add(interp, args),
        _ => {
            // tclsh：Tcl_GetIndexFromObj 的未知/歧义消息 + 子命令码。
            set_error_code(interp, &format!("TCL LOOKUP SUBCOMMAND {}", subcmd));
            Err(Error::Msg(format!(
                "unknown or ambiguous subcommand \"{}\": must be {}",
                subcmd, CLOCK_SUBCMDS
            )))
        }
    }
}

/// 子命令词：精确名优先，否则唯一前缀（tclsh Tcl_GetIndexFromObj 语义）。
fn resolve_subcommand(word: &str) -> Option<&'static str> {
    const LIST: [&str; 7] = [
        "add", "clicks", "format", "microseconds", "milliseconds", "scan", "seconds",
    ];
    if let Some(exact) = LIST.iter().find(|s| **s == word) {
        return Some(exact);
    }
    let mut hits = LIST.iter().filter(|s| s.starts_with(word));
    let first = hits.next()?;
    if hits.next().is_some() {
        return None; // 歧义（"s"）——tclsh 报 unknown or ambiguous
    }
    Some(first)
}

fn clock_seconds() -> Result<Value> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(Value::from_int(secs as i64))
}

fn clock_clicks(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // `clock clicks ?-milliseconds|-microseconds?`（默认微秒）。
    // tclsh 先查 arity（多词 → wrong # args），再看选项。
    if args.len() > 3 {
        return Err(usage_err(interp, "clock clicks ?-switch?".to_string(), None));
    }
    let millis = match args.get(2) {
        None => false,
        Some(a) => {
            let w = a.as_str();
            let by_prefix = |full: &str| full.starts_with(w);
            if w == "-milliseconds" || (by_prefix("-milliseconds") && !by_prefix("-microseconds")) {
                true
            } else if w == "-microseconds" || by_prefix("-microseconds") {
                false
            } else {
                set_error_code(interp, &format!("TCL LOOKUP INDEX option {}", w));
                return Err(Error::Msg(format!(
                    "bad option \"{}\": must be -milliseconds or -microseconds",
                    w
                )));
            }
        }
    };
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

// ── clock format ───────────────────────────────────────────────────────

const FORMAT_USAGE: &str =
    "clock format clockval ?-format string? ?-gmt boolean? ?-locale LOCALE? ?-timezone ZONE?";

/// `clock format clockval ?-format fmt? ?-gmt bool? ?-locale L? ?-timezone Z?`
fn clock_format(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(usage_err(interp, FORMAT_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
    }

    let seconds = args[2].as_int().ok_or_else(|| {
        set_error_code(interp, "TCL VALUE NUMBER");
        Error::Msg(format!("expected integer but got \"{}\"", args[2].as_str()))
    })?;

    let mut format_str = "%a %b %d %H:%M:%S %Z %Y".to_string();
    let mut use_gmt = false;
    let mut zone: Option<String> = None;
    let mut i = 3;

    while i < args.len() {
        let w = args[i].as_str();
        match w {
            "-format" | "-gmt" | "-locale" | "-timezone" => {
                if i + 1 >= args.len() {
                    return Err(usage_err(interp, FORMAT_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
                }
                let val = args[i + 1].as_str();
                match w {
                    "-format" => format_str = val.to_string(),
                    "-gmt" => use_gmt = expr_bool(interp, val)?,
                    "-locale" => {}
                    _ => zone = Some(parse_zone(interp, val)?),
                }
                i += 2;
            }
            _ => {
                // format 的 bad option 短语带冒号+牛津逗号（tclsh Tcl 实现差异）。
                set_error_code(interp, &format!("CLOCK badOption {}", w));
                return Err(Error::Msg(format!(
                    "bad option \"{}\": must be -format, -gmt, -locale, or -timezone",
                    w
                )));
            }
        }
    }

    if use_gmt && zone.is_some() {
        set_error_code(interp, "CLOCK gmtWithTimezone");
        return Err(Error::Msg("cannot use -gmt and -timezone in same call".to_string()));
    }
    let out = with_zone(zone.as_deref(), || format_timestamp(seconds, &format_str, use_gmt));
    Ok(Value::from_str(&out))
}

/// `-gmt` 的 format 变体：expr 布尔（整数 0/非零，或 Tcl 布尔词）。
fn expr_bool(interp: &mut Interp, text: &str) -> Result<bool> {
    if let Some(n) = Value::from_str(text).as_int() {
        return Ok(n != 0);
    }
    strict_bool(text).ok_or_else(|| {
        // format 走 Tcl 实现的 expr 布尔上下文：错误码是 TCL VALUE
        // NUMBER（与 add/scan 的 Tcl_GetBoolean 路径 ec=NONE 不同）。
        set_error_code(interp, "TCL VALUE NUMBER");
        Error::Msg(format!("expected boolean value but got \"{}\"", text))
    })
}

// ── clock scan ─────────────────────────────────────────────────────────

const SCAN_USAGE: &str = "clock scan string ?-base seconds? ?-format string? ?-gmt boolean? ?-locale LOCALE? ?-timezone ZONE?";

/// `clock scan string ?-format fmt? ?-gmt bool? ?-base secs? ...`
fn clock_scan(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(usage_err(interp, SCAN_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
    }

    let input = args[2].as_str().to_string();
    let mut use_gmt = false;
    let mut zone: Option<String> = None;
    let mut i = 3;
    while i < args.len() {
        let w = args[i].as_str();
        match w {
            "-base" | "-format" | "-gmt" | "-locale" | "-timezone" => {
                if i + 1 >= args.len() {
                    return Err(usage_err(interp, SCAN_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
                }
                let val = args[i + 1].as_str();
                match w {
                    "-gmt" => {
                        use_gmt = strict_bool(val).ok_or_else(|| {
                            Error::Msg(format!("expected boolean value but got \"{}\"", val))
                        })?;
                    }
                    "-timezone" => zone = Some(parse_zone(interp, val)?),
                    "-base" => {
                        // tclsh 的 scan -base 坏值不带错误码（ec=NONE）。
                        if Value::from_str(val).as_int().is_none() {
                            return Err(Error::Msg(format!(
                                "expected integer but got \"{}\"",
                                val
                            )));
                        }
                    }
                    _ => {}
                }
                i += 2;
            }
            _ => {
                // scan 的短语（逗号分隔、无冒号）与 format 不同源。
                set_error_code(interp, &format!("CLOCK badOption {}", w));
                return Err(Error::Msg(format!(
                    "bad option \"{}\", must be -base, -format, -gmt, -locale or -timezone",
                    w
                )));
            }
        }
    }
    if use_gmt && zone.is_some() {
        set_error_code(interp, "CLOCK gmtWithTimezone");
        return Err(Error::Msg("cannot use -gmt and -timezone in same call".to_string()));
    }

    // 纯整数：按 epoch 秒直通（tclsh 同）。
    if let Some(n) = args[2].as_int() {
        return Ok(Value::from_int(n));
    }
    with_zone(zone.as_deref(), || scan_iso(&input, use_gmt))
}

/// ISO 形（YYYY-MM-DD[ T]HH:MM:SS）——本地时区（-gmt 时 UTC），两遍法
/// 解偏移（先猜 UTC 求该时刻的本地偏移，再回推）。调用方已设好 zone。
fn scan_iso(input: &str, use_gmt: bool) -> Result<Value> {
    let parse_iso = |s: &str| -> Option<(i64, i64, i64, i64, i64, i64)> {
        let b: Vec<char> = s.chars().collect();
        if b.len() < 10 {
            return None;
        }
        let num = |a: usize, n: usize| -> Option<i64> { s.get(a..a + n)?.parse::<i64>().ok() };
        if b[4] != '-' || b[7] != '-' {
            return None;
        }
        let y = num(0, 4)?;
        let mo = num(5, 2)?;
        let d = num(8, 2)?;
        let (h, mi, se) = if b.len() >= 19
            && (b[10] == ' ' || b[10] == 'T')
            && b[13] == ':'
            && b[16] == ':'
        {
            (num(11, 2)?, num(14, 2)?, num(17, 2)?)
        } else if b.len() == 10 {
            (0, 0, 0)
        } else {
            return None;
        };
        Some((y, mo, d, h, mi, se))
    };
    if let Some((y, mo, d, h, mi, se)) = parse_iso(input) {
        let days = days_from_civil(y, mo, d);
        let utc_guess = days * 86400 + h * 3600 + mi * 60 + se;
        let epoch = if use_gmt {
            utc_guess
        } else {
            match local_offset_name(utc_guess) {
                Some((off, _)) => utc_guess - off as i64,
                None => utc_guess,
            }
        };
        return Ok(Value::from_int(epoch));
    }

    // tclsh 尾巴："syntax error (characters 0-7)"（0..len-1 的消费跨度）。
    let last = input.chars().count().saturating_sub(1);
    Err(Error::runtime(
        format!(
            "unable to convert date-time string \"{}\": syntax error (characters 0-{})",
            input, last
        ),
        crate::error::ErrorCode::InvalidOp,
    ))
}

// ── clock add ──────────────────────────────────────────────────────────

const ADD_USAGE: &str =
    "clock add clockval ?number units?... ?-gmt boolean? ?-locale LOCALE? ?-timezone ZONE?";

/// `clock add clockval ?number unit?... ?-gmt bool? ?-locale L? ?-timezone Z?`
///
/// 单位按给定顺序**顺序**作用（实测：Jan 30 +1 day +1 month = Feb 29，
/// +1 month +1 day = Mar 1）。年/月/日/周按目标时区的**日历**运算
/// （月份越界夹到月末：Jan 31 +1m = Feb 29；DST 下 +1 day 保持墙钟
/// 12:00），时/分/秒是固定秒数。
fn clock_add(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(usage_err(interp, "clock add clockval ?arg ...?".to_string(), None));
    }
    let clockval = match args[2].as_int() {
        Some(v) => v,
        None => {
            return Err(Error::Msg(format!(
                "expected number but got \"{}\"",
                args[2].as_str()
            )))
        }
    };

    let mut pairs: Vec<(i64, String)> = Vec::new();
    let mut gmt = false;
    let mut zone: Option<String> = None;
    let mut i = 3;
    while i < args.len() {
        let w = args[i].as_str();
        match w {
            "-gmt" => {
                if i + 1 >= args.len() {
                    return Err(usage_err(interp, ADD_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
                }
                let val = args[i + 1].as_str();
                gmt = strict_bool(val).ok_or_else(|| {
                    Error::Msg(format!("expected boolean value but got \"{}\"", val))
                })?;
                i += 2;
            }
            "-timezone" => {
                if i + 1 >= args.len() {
                    return Err(usage_err(interp, ADD_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
                }
                zone = Some(parse_zone(interp, args[i + 1].as_str())?);
                i += 2;
            }
            "-locale" => {
                if i + 1 >= args.len() {
                    return Err(usage_err(interp, ADD_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
                }
                i += 2;
            }
            _ => {
                // 先试 Tcl 整数（0x 前缀同 tclsh）；失败即 bad option 短语。
                let count = match Value::from_str(w).as_int() {
                    Some(n) => n,
                    None => {
                        set_error_code(interp, &format!("CLOCK badOption {}", w));
                        return Err(Error::Msg(format!(
                            "bad option \"{}\", must be -gmt, -locale or -timezone",
                            w
                        )));
                    }
                };
                if i + 1 >= args.len() {
                    return Err(usage_err(interp, ADD_USAGE.to_string(), Some("CLOCK wrongNumArgs")));
                }
                let unit = args[i + 1].as_str().to_string();
                if !is_time_unit(&unit) {
                    set_error_code(interp, &format!("CLOCK badUnit {}", unit));
                    return Err(Error::Msg(format!(
                        "unknown unit \"{}\", must be  years, months, weeks, days, hours, minutes or seconds",
                        unit
                    )));
                }
                pairs.push((count, unit));
                i += 2;
            }
        }
    }

    if gmt && zone.is_some() {
        set_error_code(interp, "CLOCK gmtWithTimezone");
        return Err(Error::Msg("cannot use -gmt and -timezone in same call".to_string()));
    }

    let out = with_zone(zone.as_deref(), || apply_add(clockval, &pairs, gmt));
    Ok(Value::from_int(out))
}

fn is_time_unit(u: &str) -> bool {
    matches!(
        u,
        "year" | "years"
            | "month"
            | "months"
            | "week"
            | "weeks"
            | "day"
            | "days"
            | "hour"
            | "hours"
            | "minute"
            | "minutes"
            | "second"
            | "seconds"
    )
}

/// 按序应用各单位的调整（调用方已设好时区上下文）。
fn apply_add(mut epoch: i64, pairs: &[(i64, String)], gmt: bool) -> i64 {
    for (n, unit) in pairs {
        match unit.as_str() {
            "second" | "seconds" => epoch = epoch.saturating_add(*n),
            "minute" | "minutes" => epoch = epoch.saturating_add(n.saturating_mul(60)),
            "hour" | "hours" => epoch = epoch.saturating_add(n.saturating_mul(3600)),
            "week" | "weeks" | "day" | "days" => {
                let days = if unit.starts_with("week") { n.saturating_mul(7) } else { *n };
                if gmt {
                    // UTC 无 DST：固定秒数（tclsh 对 -gmt 同）。
                    epoch = epoch.saturating_add(days.saturating_mul(86400));
                } else {
                    let Some((y, mo, d, h, mi, s)) = local_civil(epoch) else { continue };
                    let (ny, nm, nd) = civil_from_days(days_from_civil(y, mo, d) + days);
                    epoch = local_from_civil(ny, nm, nd, h, mi, s);
                }
            }
            _ => {
                // 年 / 月：日历月运算 + 月末夹取（tclsh：Jan 31 +1m =
                // Feb 29、Feb 29 +1y = Feb 28）。
                let months = if unit.starts_with("year") { n.saturating_mul(12) } else { *n };
                let (y, mo, d, h, mi, s) = if gmt {
                    let (y, mo, d) = civil_from_days(epoch.div_euclid(86400));
                    let secs = epoch.rem_euclid(86400);
                    (y, mo, d, secs / 3600, (secs % 3600) / 60, secs % 60)
                } else {
                    match local_civil(epoch) {
                        Some(t) => t,
                        None => continue,
                    }
                };
                let total = y.saturating_mul(12).saturating_add(mo - 1).saturating_add(months);
                let ny = total.div_euclid(12);
                let nm = total.rem_euclid(12) + 1;
                let nd = d.min(days_in_month(ny, nm));
                epoch = if gmt {
                    days_from_civil(ny, nm, nd) * 86400 + h * 3600 + mi * 60 + s
                } else {
                    local_from_civil(ny, nm, nd, h, mi, s)
                };
            }
        }
    }
    epoch
}

// ── 时区 / 日历辅助 ────────────────────────────────────────────────────

fn usage_err(interp: &mut Interp, usage: String, code: Option<&str>) -> Error {
    if let Some(c) = code {
        set_error_code(interp, c);
    }
    Error::Msg(format!("wrong # args: should be \"{}\"", usage))
}

/// 校验并规范化 -timezone 值：剥前导 ':'，须为 zoneinfo 文件（或
/// localtime）。未知时 tclsh 报 `time zone ":<name>" not found` +
/// `CLOCK badTimeZone :<name>`。
fn parse_zone(interp: &mut Interp, raw: &str) -> Result<String> {
    let name = raw.strip_prefix(':').unwrap_or(raw);
    let ok = !name.is_empty()
        && !name.starts_with('/')
        && !name.split('/').any(|c| c == "." || c == "..")
        && (name == "localtime"
            || std::path::Path::new("/usr/share/zoneinfo").join(name).is_file());
    if !ok {
        let disp = format!(":{}", name);
        set_error_code(interp, &format!("CLOCK badTimeZone {}", disp));
        return Err(Error::Msg(format!("time zone \"{}\" not found", disp)));
    }
    Ok(name.to_string())
}

/// 严格 Tcl 布尔（clock add/scan 的 -gmt）：0/1 或六个词的不含歧义前缀
/// （tclsh：t/f/y/n 收，o 歧义拒，2 拒）。
fn strict_bool(s: &str) -> Option<bool> {
    if s.is_empty() {
        return None;
    }
    if s == "0" {
        return Some(false);
    }
    if s == "1" {
        return Some(true);
    }
    let low = s.to_ascii_lowercase();
    let mut hit: Option<bool> = None;
    for (word, val) in [
        ("false", false),
        ("true", true),
        ("no", false),
        ("yes", true),
        ("off", false),
        ("on", true),
    ] {
        if word.starts_with(&low) {
            if hit.is_some() {
                return None; // 歧义（"o"）
            }
            hit = Some(val);
        }
    }
    hit
}

/// 在指定时区上下文里执行（unix：临时 TZ + tzset，互斥保护；zone=None
/// 时不改环境）。tclsh 的 -timezone 输出与 libc zoneinfo 同源（实测
/// America/New_York、Europe/Berlin、Asia/Shanghai、Australia/Sydney 逐字
/// 一致），因此直接借 libc 的转换表。
#[cfg(unix)]
fn with_zone<T>(zone: Option<&str>, f: impl FnOnce() -> T) -> T {
    use std::sync::Mutex;
    extern "C" {
        fn tzset();
    }
    static TZ_LOCK: Mutex<()> = Mutex::new(());
    match zone {
        None => f(),
        Some(z) => {
            let _guard = TZ_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let saved = std::env::var_os("TZ");
            std::env::set_var("TZ", z);
            unsafe { tzset() };
            let out = f();
            match saved {
                Some(v) => std::env::set_var("TZ", v),
                None => std::env::remove_var("TZ"),
            }
            unsafe { tzset() };
            out
        }
    }
}

#[cfg(not(unix))]
fn with_zone<T>(_zone: Option<&str>, f: impl FnOnce() -> T) -> T {
    f()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 民用历反向：日号 → (年, 月, 日)。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

/// 当前（调用方已设时区）的历法分解。
#[cfg(unix)]
fn local_civil(epoch: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let t = epoch as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some((
        tm.tm_year as i64 + 1900,
        tm.tm_mon as i64 + 1,
        tm.tm_mday as i64,
        tm.tm_hour as i64,
        tm.tm_min as i64,
        tm.tm_sec as i64,
    ))
}

#[cfg(not(unix))]
fn local_civil(_epoch: i64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    None
}

/// 历法分解 → epoch（调用方已设时区；tm_isdst=-1 让 libc 自行消解
/// DST 间隙——tclsh 实测 02:30 春季跳变 +1d → 03:30）。
#[cfg(unix)]
fn local_from_civil(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = (y - 1900) as libc::c_int;
    tm.tm_mon = (mo - 1) as libc::c_int;
    tm.tm_mday = d as libc::c_int;
    tm.tm_hour = h as libc::c_int;
    tm.tm_min = mi as libc::c_int;
    tm.tm_sec = s as libc::c_int;
    tm.tm_isdst = -1;
    unsafe { libc::mktime(&mut tm) as i64 }
}

#[cfg(not(unix))]
fn local_from_civil(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s
}

/// 纯-Rust strftime-like 格式化（见 `format_timestamp`）。
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

/// D-24：本地时区（unix = libc localtime_r：/etc/localtime + TZ 环境变
/// 量 + DST/历史转换全部由 C 库承担）。返回 (偏移秒, 缩写)。
#[cfg(unix)]
fn local_offset_name(epoch: i64) -> Option<(i32, String)> {
    let t = epoch as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::localtime_r(&t, &mut tm) };
    if ok.is_null() {
        return None;
    }
    let off = tm.tm_gmtoff as i32;
    let name = if tm.tm_zone.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }
            .to_string_lossy()
            .to_string()
    };
    Some((off, name))
}

#[cfg(not(unix))]
fn local_offset_name(_epoch: i64) -> Option<(i32, String)> {
    None
}

/// gmtoff 秒 → "+HHMM"（%z 形状）。
fn render_z(off: i32) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.unsigned_abs();
    format!("{}{:02}{:02}", sign, a / 3600, (a % 3600) / 60)
}

fn epoch_to_datetime(epoch: i64, _gmt: bool) -> DateTime {
    let secs = epoch;
    let days = secs.div_euclid(86400);
    let day_secs = secs.rem_euclid(86400) as u32;

    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    // Days since 0000-03-01 (March epoch trick)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u32; // day of era
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
    let month_days: [u32; 12] = [
        31,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut yday = d;
    for &md in &month_days[..(m as usize - 1)] {
        yday += md;
    }

    DateTime {
        year,
        month: m,
        day: d,
        hour,
        minute,
        second,
        wday,
        yday: yday - 1,
    }
}

fn format_timestamp(epoch: i64, fmt: &str, gmt: bool) -> String {
    // D-24：非 -gmt 时按本地时区偏移折算（历史 DST 由 localtime_r 的
    // 转换表承担）；%Z 出缩写、%z 出 +HHMM（原 %z 未实现原样输出）。
    let (eff_epoch, tz_name, tz_off) = if gmt {
        (epoch, "GMT".to_string(), 0)
    } else {
        match local_offset_name(epoch) {
            Some((off, name)) => (epoch + off as i64, name, off),
            None => (epoch, "UTC".to_string(), 0),
        }
    };
    let dt = epoch_to_datetime(eff_epoch, gmt);
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;

    let short_days = [
        "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat",
    ];
    let full_days = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let short_months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let full_months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

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
                    let h = if dt.hour == 0 {
                        12
                    } else if dt.hour > 12 {
                        dt.hour - 12
                    } else {
                        dt.hour
                    };
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
                'Z' => result.push_str(&tz_name),
                'z' => result.push_str(&render_z(tz_off)),
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
                        dt.day,
                        dt.hour,
                        dt.minute,
                        dt.second,
                        dt.year
                    ));
                }
                'D' => {
                    // %m/%d/%y
                    result.push_str(&format!(
                        "{:02}/{:02}/{:02}",
                        dt.month,
                        dt.day,
                        dt.year % 100
                    ));
                }
                'T' => {
                    // %H:%M:%S
                    result.push_str(&format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second));
                }
                other => {
                    result.push('%');
                    result.push(other);
                }
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
}
