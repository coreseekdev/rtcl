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
    let mut format: Option<String> = None;
    let mut base: Option<i64> = None;
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
                    "-format" => format = Some(val.to_string()),
                    "-base" => {
                        // tclsh 的 scan -base 坏值不带错误码（ec=NONE）。
                        match Value::from_str(val).as_int() {
                            Some(b) => base = Some(b),
                            None => {
                                return Err(Error::Msg(format!(
                                    "expected integer but got \"{}\"",
                                    val
                                )))
                            }
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

    // 显式 -format：按格式解析（tclsh 的 strptime 面）。
    if let Some(fmt) = format {
        let base_epoch = base.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
        let mut fail = ScanFail::Mismatch;
        let parsed = scan_with_format(
            &input,
            &fmt,
            base_epoch,
            use_gmt,
            zone.as_deref(),
            &mut fail,
        );
        return match parsed {
            Some(epoch) => Ok(Value::from_int(epoch)),
            None => match fail {
                ScanFail::Other(msg) => Err(Error::Msg(msg)),
                ScanFail::BadZone(name) => {
                    set_error_code(interp, &format!("CLOCK badTZName {}", name));
                    Err(Error::Msg(format!("time zone \"{}\" not found", name)))
                }
                ScanFail::Mismatch => {
                    set_error_code(interp, "CLOCK badInputString");
                    Err(Error::Msg(
                        "input string does not match supplied format".to_string(),
                    ))
                }
            },
        };
    }

    // 自由格式（tclsh ClockFreeScan 常用子集）：裸数字按 HHMM 时刻，
    // 不按 epoch 秒（tclsh `clock scan 123` = 今日 01:23）。
    let base_epoch = base.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });
    with_zone(zone.as_deref(), || scan_free(&input, base_epoch, use_gmt))
}

/// 按 `-format` 解析（strptime 面的 Tcl 语义，2026-11-06 实测）：
/// * 数字字段跳过前导空白、1-2 位（%Y 恰 4 位、%j 1-3 位）；
/// * 格式里的空白要求输入至少一个空白并吞掉整段（格式化串尾随空格
///   须有对应输入空白）；
/// * 格式中缺席的日期字段从 `-base` 取，缺席的时间字段默认 0；
/// * %y 窗口：00-37→20xx、38-99→19xx（Tcl 的 32 位 time_t 安全窗）；
/// * %B/%b 收全称与缩写、大小写不敏感；%a/%A 解析但忽略（不做一致性
///   校验）；%p 参与 12 小时换算；%z 支持 +HHMM/+HH:MM 并回推；
///   %Z 消费字母并忽略；%s 直接取 epoch；%% 字面。
/// * 日期归一化交给 mktime / days_from_civil（Feb 30 → Mar 1、13 月
///   → 次年 1 月，tclsh 同）。
/// `scan_with_format` 的失败种类（None 返回时的原因）。
enum ScanFail {
    Mismatch,
    BadZone(String),
    Other(String),
}

fn scan_with_format(
    input: &str,
    fmt: &str,
    base: i64,
    gmt: bool,
    zone_opt: Option<&str>,
    fail: &mut ScanFail,
) -> Option<i64> {
    #[derive(Default)]
    struct F {
        y: Option<i64>,
        y2: Option<i64>,
        mo: Option<i64>,
        d: Option<i64>,
        yd: Option<i64>,
        h: Option<i64>,
        mi: Option<i64>,
        se: Option<i64>,
        pm: Option<bool>,
        epoch: Option<i64>,
        tz: Option<i64>,
        zone_name: Option<String>,
    }
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0usize;
    let mut f = F::default();
    // tclsh：输入两端的多余空白忽略（格式内空白仍须有输入空白对应）。
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let fmt_chars: Vec<char> = fmt.chars().collect();
    let mut fi = 0usize;

    let skip_ws = |pos: &mut usize| {
        while *pos < chars.len() && chars[*pos].is_whitespace() {
            *pos += 1;
        }
    };
    let digits = |pos: &mut usize, min: usize, max: usize| -> Option<i64> {
        let start = *pos;
        while *pos < chars.len() && *pos - start < max && chars[*pos].is_ascii_digit() {
            *pos += 1;
        }
        if *pos - start < min {
            return None;
        }
        chars[start..*pos].iter().collect::<String>().parse::<i64>().ok()
    };
    let word = |pos: &mut usize| -> Option<String> {
        let start = *pos;
        while *pos < chars.len() && chars[*pos].is_ascii_alphabetic() {
            *pos += 1;
        }
        if *pos == start {
            return None;
        }
        Some(chars[start..*pos].iter().collect::<String>().to_ascii_lowercase())
    };

    while fi < fmt_chars.len() {
        let c = fmt_chars[fi];
        fi += 1;
        if c == '%' {
            if fi >= fmt_chars.len() {
                return None;
            }
            let spec = fmt_chars[fi];
            fi += 1;
            match spec {
                'Y' => {
                    skip_ws(&mut pos);
                    f.y = Some(digits(&mut pos, 4, 4)?);
                }
                'y' => {
                    skip_ws(&mut pos);
                    f.y2 = Some(digits(&mut pos, 1, 2)?);
                }
                'm' => {
                    skip_ws(&mut pos);
                    f.mo = Some(digits(&mut pos, 1, 2)?);
                }
                'd' | 'e' => {
                    skip_ws(&mut pos);
                    f.d = Some(digits(&mut pos, 1, 2)?);
                }
                'H' => {
                    skip_ws(&mut pos);
                    f.h = Some(digits(&mut pos, 1, 2)?);
                }
                'I' => {
                    skip_ws(&mut pos);
                    f.h = Some(digits(&mut pos, 1, 2)?);
                }
                'M' => {
                    skip_ws(&mut pos);
                    f.mi = Some(digits(&mut pos, 1, 2)?);
                }
                'S' => {
                    skip_ws(&mut pos);
                    f.se = Some(digits(&mut pos, 1, 2)?);
                }
                'j' => {
                    skip_ws(&mut pos);
                    f.yd = Some(digits(&mut pos, 1, 3)?);
                }
                'p' => {
                    skip_ws(&mut pos);
                    let w = word(&mut pos)?;
                    f.pm = Some(match w.as_str() {
                        "am" => false,
                        "pm" => true,
                        _ => return None,
                    });
                }
                'b' | 'h' | 'B' => {
                    skip_ws(&mut pos);
                    let w = word(&mut pos)?;
                    f.mo = Some(match_month_name(&w)?);
                }
                'a' | 'A' => {
                    skip_ws(&mut pos);
                    let w = word(&mut pos)?;
                    if !WEEKDAY_NAMES.iter().any(|n| n.starts_with(&w)) {
                        return None;
                    }
                }
                's' => {
                    skip_ws(&mut pos);
                    let neg = pos < chars.len() && chars[pos] == '-';
                    if neg {
                        pos += 1;
                    }
                    let v = digits(&mut pos, 1, 18)?;
                    f.epoch = Some(if neg { -v } else { v });
                }
                'z' => {
                    skip_ws(&mut pos);
                    let sign = match chars.get(pos) {
                        Some('+') => 1i64,
                        Some('-') => -1,
                        _ => return None,
                    };
                    pos += 1;
                    let hh = digits(&mut pos, 2, 2)?;
                    if chars.get(pos) == Some(&':') {
                        pos += 1;
                    }
                    let mm = digits(&mut pos, 2, 2)?;
                    f.tz = Some(sign * (hh * 3600 + mm * 60));
                }
                'Z' => {
                    skip_ws(&mut pos);
                    let mut w = word(&mut pos)?;
                    // tclsh 的 %Z token 收字母后**可选一串数字**（zp4/
                    // zp5/zp6；"est5edt" 仍 mismatch——数字后还有字母）。
                    let dstart = pos;
                    while pos < chars.len() && chars[pos].is_ascii_digit() {
                        pos += 1;
                    }
                    if pos > dstart {
                        w.push_str(&chars[dstart..pos].iter().collect::<String>());
                    }
                    f.zone_name = Some(w);
                }
                '%' => {
                    if chars.get(pos) != Some(&'%') {
                        return None;
                    }
                    pos += 1;
                }
                'n' => {
                    if chars.get(pos) != Some(&'\n') {
                        return None;
                    }
                    pos += 1;
                }
                't' => {
                    if chars.get(pos) != Some(&'\t') {
                        return None;
                    }
                    pos += 1;
                }
                _ => return None,
            }
        } else if c.is_whitespace() {
            // 格式空白：输入须有至少一个空白，吞掉整段。
            if pos >= chars.len() || !chars[pos].is_whitespace() {
                return None;
            }
            skip_ws(&mut pos);
        } else {
            if chars.get(pos) != Some(&c) {
                return None;
            }
            pos += 1;
        }
    }
    // 尾随空白忽略；其余尾随文本 → mismatch（tclsh 同）。
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos != chars.len() {
        return None;
    }
    if let Some(e) = f.epoch {
        return Some(e);
    }

    // -base 的历法分解（年/月/日兜底）+ 组合；`gmt_eff` 决定 UTC/本地
    // （%Z / -timezone 时在对应时区上下文里按本地历法求值）。
    let fields_of = |gmt_eff: bool| -> Option<(i64, i64, i64, i64, i64, i64)> {
    let Some((by, bmo, bd, _, _, _)) = clock_civil(base, gmt_eff) else {
        return None;
    };
    let has_y = f.y.or_else(|| f.y2.map(map_two_digit_year));

    // tclsh 8.6.17 实测的字段合成规则：
    // * 年只有与**月+日**（或 %j）同时出现才生效；单给年/月都忽略；
    // * 日可单独生效（年/月取 base）；
    // * %j 单独生效（年取 base）；
    // * 时间字段只有 %H/%I 出现才整体生效（单给 %M/%S 忽略）。
    let (yy, mo, dd) = if let (Some(y), Some(yd)) = (has_y, f.yd) {
        let (a, b, c) = civil_from_jdn(jdn_from_civil(y, 1, 1) + yd - 1);
        (a, b, c)
    } else if f.mo.is_some() && f.d.is_some() {
        (has_y.unwrap_or(by), f.mo.unwrap(), f.d.unwrap())
    } else if let Some(d) = f.d {
        (by, bmo, d)
    } else if let Some(yd) = f.yd {
        let (a, b, c) = civil_from_jdn(jdn_from_civil(by, 1, 1) + yd - 1);
        let _ = a;
        (by, b, c)
    } else {
        (by, bmo, bd)
    };

    let (mut hour, mut minute, mut second) = (0i64, 0i64, 0i64);
    if f.h.is_some() {
        hour = f.h.unwrap();
        if let Some(pm) = f.pm {
            // 12 小时制换算（%I 后接 %p）：12 AM → 0，12 PM → 12。
            let h12 = hour % 12;
            hour = if pm { h12 + 12 } else { h12 };
        }
        minute = f.mi.unwrap_or(0);
        second = f.se.unwrap_or(0);
    }

    Some((yy, mo, dd, hour, minute, second))
    };
    let compute = |gmt_eff: bool| -> Option<i64> {
        let (yy, mo, dd, hour, minute, second) = fields_of(gmt_eff)?;
        Some(if let Some(off) = f.tz {
            // %z：字段按 UTC 解释后减去偏移。
            epoch_from_jdn(jdn_from_civil(yy, mo, dd), hour, minute, second) - off
        } else {
            clock_epoch(yy, mo, dd, hour, minute, second, gmt_eff)
        })
    };

    // %Z：tclsh 8.6.17 实测的**时区缩写表**（名字不分大小写、消息用
    // 小写；%Z 优先于 -gmt 与 -timezone；未知缩写 = badTZName）。
    if let Some(name) = &f.zone_name {
        match zone_abbrev(name) {
            Some(off) => {
                let (yy, mo, dd, hour, minute, second) = fields_of(true)?;
                Some(epoch_from_jdn(jdn_from_civil(yy, mo, dd), hour, minute, second) + off)
            }
            None => {
                *fail = ScanFail::BadZone(name.clone());
                None
            }
        }
    } else if let Some(z) = zone_opt {
        with_zone(Some(z), || compute(gmt))
    } else {
        compute(gmt)
    }
}

/// 该缩写是否 DST 变体（tclDate TimezoneTable 的 tDAYZONE 项）。
fn zone_is_dst(name: &str) -> bool {
    matches!(
        name,
        "bst" | "edt" | "cdt" | "mdt" | "pdt" | "ydt" | "akdt" | "hdt" | "adt" | "ndt"
            | "cest" | "mest" | "sst" | "fst" | "wadt" | "cadt" | "eadt" | "jdt" | "kdt"
            | "nzdt"
    )
}

/// 时区缩写 → 偏移（epoch = 该墙钟按 UTC 解释 + off；off = -本地东向偏移）。
/// 表按 tclsh 8.6.17 逐个实测（与 tclDate.c 的 TimezoneTable 同源，
/// 但本机 oracle 的个别条目与 vendored 新版不同，以 oracle 为准）。
fn zone_abbrev(lower: &str) -> Option<i64> {
    let h = 3600i64;
    Some(match lower {
        "gmt" | "ut" | "utc" | "wet" => 0,
        "bst" => -h,
        "wat" => h,
        "at" => 2 * h,
        "nft" | "nst" => 7 * h / 2,
        "ndt" => 5 * h / 2,
        "ast" => 4 * h,
        "adt" => 3 * h,
        "est" => 5 * h,
        "edt" => 4 * h,
        "cst" => 6 * h,
        "cdt" => 5 * h,
        "mst" => 7 * h,
        "mdt" => 6 * h,
        "pst" => 8 * h,
        "pdt" => 7 * h,
        "yst" => 9 * h,
        "ydt" => 8 * h,
        "akst" => 9 * h,
        "akdt" => 8 * h,
        "hst" => 10 * h,
        "hdt" => 9 * h,
        "cat" | "ahst" => 10 * h,
        "nt" => 11 * h,
        "idlw" => 12 * h,
        "cet" | "met" | "mewt" | "swt" | "fwt" => -h,
        "cest" | "mest" | "sst" | "fst" | "eet" => -2 * h,
        "bt" => -3 * h,
        "it" => -7 * h / 2,
        "zp4" => -4 * h,
        "zp5" => -5 * h,
        "ist" => -11 * h / 2,
        "zp6" => -6 * h,
        "wast" => -7 * h,
        "wadt" => -8 * h,
        "jt" => -15 * h / 2,
        "cct" => -8 * h,
        "jst" | "kst" => -9 * h,
        "jdt" | "kdt" => -10 * h,
        "cast" => -19 * h / 2,
        "cadt" => -21 * h / 2,
        "east" | "gst" => -10 * h,
        "eadt" => -21 * h / 2,
        "nzt" | "nzst" | "idle" => -12 * h,
        "nzdt" => -13 * h,
        _ => return None,
    })
}

/// %y → 世纪：Tcl 的 32 位 time_t 安全窗（00-37 → 2000+，38-99 → 1900+）。
fn map_two_digit_year(y: i64) -> i64 {
    if y <= 37 {
        2000 + y
    } else {
        1900 + y
    }
}

const MONTH_NAMES: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const WEEKDAY_NAMES: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

/// 月份名匹配：全称或缩写前缀（大小写不敏感，已 lower）。
fn match_month_name(w: &str) -> Option<i64> {
    for (i, name) in MONTH_NAMES.iter().enumerate() {
        if name.starts_with(w) {
            return Some(i as i64 + 1);
        }
    }
    None
}

/// 自由格式 `clock scan`（tclsh ClockFreeScan 的常用子集，2026-11-06
/// 实测钉形）：
/// * 空串/`now`/`today` → base 原值；`tomorrow`/`yesterday` → ±1 天 00:00；
/// * 绝对日期：ISO（YYYY-MM-DD[ T]HH:MM[:SS][Z|±HHMM]）、YYYYMMDD、
///   M/D[/Y[Y]]、Mon D[, YYYY] / D Mon [YYYY] / D-Mon-YYYY；
/// * 时刻：H:MM[:SS][am|pm] / 3pm / 3 pm / 裸数字（≤4 位按 HHMM）；
/// * 星期名（唯一前缀，可带 next/last；单独出现取 base 之后/之前）；
/// * 相对短语：`<n> <unit>[s] [ago]`、`next|last <unit>`；单位
///   second/minute/hour/day/week/fortnight/month/year（sec/secs 亦收）；
/// * 数字时区 ±HHMM 与 UTC/GMT；`Z` 单独出现视为 UTC。
/// 组合规则：缺席日期取 base；有日期或**非零日级相对量**时时刻归零
/// （显式时刻优先）；相对量按序施加（月/年夹到月末）；weekday 单独
/// 出现时 weekday 的 Plain 语义 = base 当天或之后（含当天），next =
/// 下一周同名日，last = 严格之前最近一天。
fn scan_free(input: &str, base: i64, use_gmt: bool) -> Result<Value> {
    let chars: Vec<char> = input.chars().collect();
    // 空白分词（记字符区间）。
    let mut toks: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let st = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        toks.push((st, i));
    }
    let joined = |t: (usize, usize)| -> String { chars[t.0..t.1].iter().collect() };
    if toks.is_empty() {
        return Ok(Value::from_int(base));
    }

    fn digits_of(cs: &[char]) -> Option<i64> {
        if cs.is_empty() || !cs.iter().all(|c| c.is_ascii_digit()) {
            return None;
        }
        cs.iter().collect::<String>().parse::<i64>().ok()
    }
    fn take_num(cs: &[char], p: &mut usize, min: usize, max: usize) -> Option<i64> {
        let st = *p;
        while *p < cs.len() && *p - st < max && cs[*p].is_ascii_digit() {
            *p += 1;
        }
        if *p - st < min {
            return None;
        }
        digits_of(&cs[st..*p])
    }
    /// ISO token 解析：Ok(None) = 不是 ISO 形；Ok(Some(..)) 成功；
    /// Err(pos) = 前缀像 ISO 但尾部有无法消费的字符（字符位相对 token）。
    fn parse_iso_tok(t: &[char]) -> std::result::Result<
        Option<(i64, i64, i64, i64, i64, i64, Option<i64>)>,
        usize,
    > {
        let mut p = 0usize;
        let Some(y) = take_num(t, &mut p, 4, 4) else {
            return Ok(None);
        };
        if t.get(p) != Some(&'-') {
            return Ok(None);
        }
        p += 1;
        let Some(mo) = take_num(t, &mut p, 1, 2) else {
            return Ok(None);
        };
        if t.get(p) != Some(&'-') {
            return Ok(None);
        }
        p += 1;
        let Some(d) = take_num(t, &mut p, 1, 2) else {
            return Ok(None);
        };
        let (mut h, mut mi, mut se, mut zone) = (0i64, 0i64, 0i64, None);
        if p < t.len() {
            let sep = t[p];
            if sep == 'T' || sep == 't' || sep == ' ' {
                p += 1;
                let Some(hh) = take_num(t, &mut p, 1, 2) else {
                    return Err(p);
                };
                if t.get(p) != Some(&':') {
                    return Err(p);
                }
                p += 1;
                let Some(mm) = take_num(t, &mut p, 1, 2) else {
                    return Err(p);
                };
                h = hh;
                mi = mm;
                if t.get(p) == Some(&':') {
                    p += 1;
                    let Some(ss) = take_num(t, &mut p, 1, 2) else {
                        return Err(p);
                    };
                    se = ss;
                }
                if p < t.len() {
                    match t[p] {
                        'Z' | 'z' => {
                            zone = Some(0);
                            p += 1;
                        }
                        '+' | '-' => {
                            let sign = if t[p] == '+' { 1i64 } else { -1 };
                            p += 1;
                            let Some(zh) = take_num(t, &mut p, 2, 2) else {
                                return Err(p);
                            };
                            let Some(zm) = take_num(t, &mut p, 2, 2) else {
                                return Err(p);
                            };
                            zone = Some(sign * (zh * 3600 + zm * 60));
                        }
                        _ => return Err(p),
                    }
                }
            } else {
                return Err(p);
            }
        }
        if p != t.len() {
            return Err(p);
        }
        Ok(Some((y, mo, d, h, mi, se, zone)))
    }
    /// 时刻 token：H:MM[:SS] 可带紧贴的 am/pm。Err(pos) 为尾部垃圾位。
    fn parse_time_tok(t: &[char]) -> std::result::Result<Option<(i64, i64, i64)>, usize> {
        let mut p = 0usize;
        let Some(h) = take_num(t, &mut p, 1, 2) else {
            return Ok(None);
        };
        if t.get(p) != Some(&':') {
            // 无冒号：只有紧贴 am/pm 才是时刻（"3pm"）。
            if p < t.len() {
                let rest: String = t[p..].iter().collect::<String>().to_ascii_lowercase();
                if rest == "am" || rest == "pm" {
                    return Ok(Some((meridian(h, rest == "pm"), 0, 0)));
                }
            }
            return Ok(None);
        }
        p += 1;
        let Some(mi) = take_num(t, &mut p, 1, 2) else {
            return Err(p);
        };
        let mut se = 0i64;
        if t.get(p) == Some(&':') {
            p += 1;
            let Some(x) = take_num(t, &mut p, 1, 2) else {
                return Err(p);
            };
            se = x;
        }
        let mut h = h;
        if p < t.len() {
            let rest: String = t[p..].iter().collect::<String>().to_ascii_lowercase();
            if let Some(tail) = rest.strip_prefix("am") {
                if !tail.is_empty() {
                    return Err(p);
                }
                h = meridian(h, false);
                p = t.len();
            } else if let Some(tail) = rest.strip_prefix("pm") {
                if !tail.is_empty() {
                    return Err(p);
                }
                h = meridian(h, true);
                p = t.len();
            } else {
                return Err(p);
            }
        }
        Ok(Some((h, mi, se)))
    }
    fn meridian(h: i64, pm: bool) -> i64 {
        if pm {
            if h < 12 {
                h + 12
            } else {
                h
            }
        } else if h == 12 {
            0
        } else {
            h
        }
    }

    // ── 状态 ──
    let mut adate: Option<(Option<i64>, i64, i64)> = None; // (year?, month, day)
    let mut tset: Option<(i64, i64, i64)> = None;
    let mut zone: Option<i64> = None;
    let mut weekday: Option<(usize, WdMode)> = None;
    // next/last <月名> 的序号月请求：(月, 是否 next) 与显式年份。
    let mut ordinal: Option<(i64, bool)> = None;
    let mut ordinal_year: Option<i64> = None;
    let mut rel: Vec<(i64, FreeUnit)> = Vec::new();
    let mut ago_watermark: usize = 0;
    let syntax_err = |input: &str, a: usize, b: usize| -> Error {
        Error::runtime(
            format!(
                "unable to convert date-time string {:?}: syntax error (characters {}-{})",
                input, a, b
            ),
            crate::error::ErrorCode::InvalidOp,
        )
    };
    let span_of = |ti: usize| -> (usize, usize) {
        let (st, en) = toks[ti];
        if st > 0 {
            (st - 1, en - 1)
        } else {
            (0, en - 1)
        }
    };
    let more_than_one_time = || {
        Error::Msg(format!(
            "unable to convert date-time string {:?}: more than one time of day in string",
            input
        ))
    };

    let mut ti = 0usize;
    while ti < toks.len() {
        let raw = joined(toks[ti]);
        let lower = raw.to_ascii_lowercase();
        let core = lower.trim_end_matches(',').to_string();
        let had_comma = core.len() != lower.len();

        let tok_chars: Vec<char> = raw.chars().collect();

        // 0.5) 数值短横日期 = **年-月-日**（tclsh 实测：年 <100 走 %y
        // 窗口；月/日可越界归一化——"1-2-2000" = 2001-02-01 + 1999 天，
        // "0-0-0" = 1999-11-30）。两段数值短横 → token 尾处 syntax error。
        if core.contains('-') {
            let parts: Vec<&str> = core.split('-').collect();
            if parts.len() == 3
                && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            {
                let yv: i64 = parts[0].parse().unwrap();
                let y = if yv < 100 { map_two_digit_year(yv) } else { yv };
                let mo: i64 = parts[1].parse().unwrap();
                let d: i64 = parts[2].parse().unwrap();
                if matches!(ordinal, Some((_, false))) {
                    return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
                }
                adate = Some((Some(y), mo, d));
                ti += 1;
                continue;
            }
            if parts.len() == 2
                && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            {
                let e = toks[ti].1;
                return Err(syntax_err(input, e, e));
            }
        }

        // 1) ISO 日期（带 T 的时刻形；纯日期已由 0.5 覆盖）
        match parse_iso_tok(&tok_chars) {
            Ok(Some((y, mo, d, h, mi, se, z))) => {
                if matches!(ordinal, Some((_, false))) {
                    return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
                }
                adate = Some((Some(y), mo, d));
                if h != 0 || mi != 0 || se != 0 || raw.len() > 10 {
                    if tset.is_some() {
                        return Err(more_than_one_time());
                    }
                    tset = Some((h, mi, se));
                }
                if let Some(zz) = z {
                    zone = Some(zz);
                }
                ti += 1;
                continue;
            }
            Err(pos) => {
                let g = toks[ti].0 + pos;
                return Err(syntax_err(input, g, g));
            }
            Ok(None) => {}
        }
        // 2) 8 位数字日期 YYYYMMDD
        if core.len() == 8 && core.chars().all(|c| c.is_ascii_digit()) {
            let y: i64 = core[0..4].parse().unwrap();
            let mo: i64 = core[4..6].parse().unwrap();
            let d: i64 = core[6..8].parse().unwrap();
            if matches!(ordinal, Some((_, false))) {
                return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
            }
            adate = Some((Some(y), mo, d));
            ti += 1;
            continue;
        }
        // 3) 斜杠日期 M/D[/Y[Y]]
        if core.contains('/') {
            let parts: Vec<&str> = core.split('/').collect();
            if parts.len() == 2
                && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            {
                let mo: i64 = parts[0].parse().unwrap();
                let d: i64 = parts[1].parse().unwrap();
                if matches!(ordinal, Some((_, false))) {
                    return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
                }
                adate = Some((None, mo, d));
                ti += 1;
                continue;
            }
            if parts.len() == 3
                && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            {
                let mo: i64 = parts[0].parse().unwrap();
                let d: i64 = parts[1].parse().unwrap();
                let ys = parts[2];
                let y = if ys.len() <= 2 {
                    map_two_digit_year(ys.parse().unwrap())
                } else {
                    ys.parse().unwrap()
                };
                if matches!(ordinal, Some((_, false))) {
                    return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
                }
                adate = Some((Some(y), mo, d));
                ti += 1;
                continue;
            }
        }
        // 4) D-Mon-YYYY 形（token 内含 '-' 与月份名）
        if core.contains('-') {
            let parts: Vec<&str> = core.split('-').collect();
            if parts.len() == 3
                && parts[0].chars().all(|c| c.is_ascii_digit())
                && !parts[0].is_empty()
            {
                if let Some(mo) = free_month(parts[1]) {
                    let d: i64 = parts[0].parse().unwrap();
                    let y: i64 = if parts[2].chars().all(|c| c.is_ascii_digit()) {
                        let p2: i64 = parts[2].parse().unwrap();
                        if parts[2].len() <= 2 {
                            map_two_digit_year(p2)
                        } else {
                            p2
                        }
                    } else {
                        return Err(syntax_err(input, toks[ti].0, toks[ti].1 - 1));
                    };
                    if matches!(ordinal, Some((_, false))) {
                        return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
                    }
                    adate = Some((Some(y), mo, d));
                    ti += 1;
                    continue;
                }
            }
        }
        // 5) 月名优先（Mon D[, YYYY]）
        if let Some(mo) = free_month(&core) {
            let Some(nt) = toks.get(ti + 1).copied() else {
                let e = toks[ti].1;
                return Err(syntax_err(input, e, e));
            };
            let nraw = joined(nt);
            let ncore = nraw.trim_end_matches(',').to_string();
            let ncomma = ncore.len() != nraw.len();
            let Some(d) = digits_of(&ncore.chars().collect::<Vec<char>>()) else {
                return Err(syntax_err(input, toks[ti].0, toks[ti].1 - 1));
            };
            let mut adv = 2usize;
            let mut year: Option<i64> = None;
            if ncomma {
                if let Some(yt) = toks.get(ti + 2).copied() {
                    let ys = joined(yt);
                    if ys.chars().all(|c| c.is_ascii_digit()) && !ys.is_empty() {
                        let p2: i64 = ys.parse().unwrap();
                        year = Some(if ys.len() <= 2 { map_two_digit_year(p2) } else { p2 });
                        adv = 3;
                    }
                }
            } else if let Some(yt) = toks.get(ti + 2).copied() {
                let ys = joined(yt);
                // tclsh 怪形：无逗号的 4 位数字——尚未出现时刻时记作
                // HHMM 时刻（"Jan 2 2000" = Jan 2 20:00），已有时刻则
                // 当成年份（"3:04 PM Jan 2 2000"）。
                if ys.len() == 4 && ys.chars().all(|c| c.is_ascii_digit()) {
                    let v: i64 = ys.parse().unwrap();
                    if tset.is_some() {
                        year = Some(v);
                    } else {
                        tset = Some((v / 100, v % 100, 0));
                    }
                    adv = 3;
                }
            }
            if matches!(ordinal, Some((_, false))) {
                return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
            }
            adate = Some((year, mo, d));
            ti += adv;
            continue;
        }
        // 6) 日优先：D Mon [YYYY]
        if let Some(d) = digits_of(&core.chars().collect::<Vec<char>>()) {
            if let Some(nt) = toks.get(ti + 1).copied() {
                let nlow = joined(nt).to_ascii_lowercase();
                if let Some(mo) = free_month(&nlow) {
                    let mut adv = 2usize;
                    let mut year: Option<i64> = None;
                    if let Some(yt) = toks.get(ti + 2).copied() {
                        let ys = joined(yt);
                        if ys.chars().all(|c| c.is_ascii_digit()) && !ys.is_empty() {
                            let p2: i64 = ys.parse().unwrap();
                            year =
                                Some(if ys.len() <= 2 { map_two_digit_year(p2) } else { p2 });
                            adv = 3;
                        }
                    }
                    if matches!(ordinal, Some((_, false))) {
                        return Err(Error::Msg(format!(
                        "unable to convert date-time string {:?}: more than one time zone in string",
                        input
                    )));
                    }
                    adate = Some((year, mo, d));
                    ti += adv;
                    continue;
                }
            }
        }
        // 6.5) last <月名> 的后续数字 = 年份（%y 窗口）；同一 token 数字
        // 后有剩余字符（"3pm"/"3:00"/"12:00"）→ syntax error 指向该字符
        // （tclsh 8.6.17 实测）。
        if let Some((_, false)) = ordinal {
            let mut cs: &[char] = &tok_chars;
            if cs.first() == Some(&'-') {
                cs = &cs[1..];
            }
            if !cs.is_empty() && cs.iter().all(|c| c.is_ascii_digit()) {
                let ys: String = cs.iter().collect();
                let v: i64 = ys.parse().ok().unwrap_or(0);
                ordinal_year = Some(if ys.len() <= 2 { map_two_digit_year(v) } else { v });
                ti += 1;
                continue;
            }
            if !cs.is_empty() && cs[0].is_ascii_digit() {
                let ndig = cs.iter().take_while(|c| c.is_ascii_digit()).count();
                let ys: String = cs[..ndig].iter().collect();
                let v: i64 = ys.parse().ok().unwrap_or(0);
                ordinal_year = Some(if ys.len() <= 2 { map_two_digit_year(v) } else { v });
                let g = toks[ti].0 + ndig;
                let end = if chars.get(g) == Some(&':') {
                    g
                } else {
                    toks[ti].1.saturating_sub(1)
                };
                return Err(syntax_err(input, g, end));
            }
        }
        // 7) next / last
        if core == "next" || core == "last" {
            let mode = if core == "next" { WdMode::Next } else { WdMode::Last };
            let Some(nt) = toks.get(ti + 1).copied() else {
                return Err(syntax_err(input, toks[ti].0, toks[ti].1 - 1));
            };
            let nlow = joined(nt).to_ascii_lowercase();
            if let Some(dow) = free_weekday(&nlow) {
                weekday = Some((dow, mode));
                ti += 2;
                continue;
            }
            if let Some(u) = free_unit(&nlow) {
                let n = if core == "next" { 1 } else { -1 };
                rel.push((n, u));
                ti += 2;
                continue;
            }
            if let Some(mo) = free_month(&nlow) {
                // next/last <月名>：见组合段（last 的公式很怪，实测钉形）。
                ordinal = Some((mo, core == "next"));
                ti += 2;
                continue;
            }
            return Err(syntax_err(input, toks[ti].0, toks[ti].1 - 1));
        }
        // 7.5) 时区缩写（tclDate TimezoneTable，含军事单字母
        // a=-1h…m=-12h、n=+1h…y=+12h、z=0；j 未定义）。tclsh 实测：
        // * 无日期时可任意位置出现（时刻/相对量前后均可）；
        // * 在**全日期之后**且尚无时刻时，只有 DST 缩写（tDAYZONE：
        //   edt/cdt/…/bst）被接受，其余报 syntax error（指向 token 尾）；
        // * 有时刻后（无论先后）全部接受；
        // * epoch = 墙钟按 UTC 解释 + off。
        if !core.is_empty() && core.chars().all(|c| c.is_ascii_alphabetic()) {
            let mil: Option<i64> = if core.len() == 1 {
                match core.as_bytes()[0] {
                    b'a'..=b'i' => Some(-((core.as_bytes()[0] - b'a' + 1) as i64) * 3600),
                    b'k'..=b'm' => Some(-((core.as_bytes()[0] - b'k' + 10) as i64) * 3600),
                    b'n'..=b'y' => Some(((core.as_bytes()[0] - b'n' + 1) as i64) * 3600),
                    b'z' => Some(0),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(off) = mil.or_else(|| zone_abbrev(&core)) {
                if adate.is_some() && tset.is_none() && !zone_is_dst(&core) {
                    let e = toks[ti].1;
                    return Err(syntax_err(input, e, e));
                }
                // 日期后的半时区 DST 缩写：tclsh 8.6.17 的整数小时路径
                // 与独立出现不同（实测 ndt=+2h、cadt=-10h、eadt=-11h）。
                let off = if adate.is_some() && zone_is_dst(&core) {
                    match core.as_str() {
                        "ndt" => 7200,
                        "cadt" => -36000,
                        "eadt" => -39600,
                        _ => off,
                    }
                } else {
                    off
                };
                zone = Some(-off);
                ti += 1;
                continue;
            }
        }
        // 8) 星期名
        if let Some(dow) = free_weekday(&core) {
            weekday = Some((dow, WdMode::Plain));
            ti += 1;
            continue;
        }
        // 9) now / today / tomorrow / yesterday
        match core.as_str() {
            "now" | "today" => {
                ti += 1;
                continue;
            }
            "tomorrow" => {
                rel.push((1, FreeUnit::Day));
                ti += 1;
                continue;
            }
            "yesterday" => {
                rel.push((-1, FreeUnit::Day));
                ti += 1;
                continue;
            }
            _ => {}
        }


        {
            let cs: Vec<char> = core.chars().collect();
            if cs.len() == 5
                && (cs[0] == '+' || cs[0] == '-')
                && cs[1..].iter().all(|c| c.is_ascii_digit())
            {
                let sign = if cs[0] == '+' { 1i64 } else { -1 };
                let zh = (cs[1] as i64 - '0' as i64) * 10 + (cs[2] as i64 - '0' as i64);
                let zm = (cs[3] as i64 - '0' as i64) * 10 + (cs[4] as i64 - '0' as i64);
                zone = Some(sign * (zh * 3600 + zm * 60));
                ti += 1;
                continue;
            }
        }
        // 10.5) 裸单位 = +1 个单位（tclsh 实测："days" → +1 天，
        // "hours" → +1 小时，与数字短语同规则）。
        if let Some(u) = free_unit(&core) {
            rel.push((1, u));
            ti += 1;
            continue;
        }
        // 11) 时刻 token（其后可跟独立的 am/pm）
        match parse_time_tok(&tok_chars) {
            Ok(Some((mut h, mi, se))) => {
                let mut adv = 1usize;
                if let Some(nt) = toks.get(ti + 1).copied() {
                    let nlow = joined(nt).to_ascii_lowercase();
                    if nlow == "am" || nlow == "pm" {
                        h = meridian(h, nlow == "pm");
                        adv = 2;
                    }
                }
                if tset.is_some() {
                    return Err(more_than_one_time());
                }
                tset = Some((h, mi, se));
                ti += adv;
                continue;
            }
            Err(pos) => {
                let g = toks[ti].0 + pos;
                return Err(syntax_err(input, g, g));
            }
            Ok(None) => {}
        }
        // 12) 数字 + 单位（相对） / 数字 + am|pm / 裸数字（HHMM）
        let signed: Option<i64> = {
            let mut cs: &[char] = &tok_chars;
            let mut neg = false;
            if cs.first() == Some(&'-') {
                neg = true;
                cs = &cs[1..];
            }
            digits_of(cs).map(|v| if neg { -v } else { v })
        };
        if let Some(n) = signed {
            // 数字 + am/pm（"3 pm"）
            if let Some(nt) = toks.get(ti + 1).copied() {
                let nlow = joined(nt).to_ascii_lowercase();
                if nlow == "am" || nlow == "pm" {
                    if tset.is_some() {
                        return Err(more_than_one_time());
                    }
                    tset = Some((meridian(n, nlow == "pm"), 0, 0));
                    ti += 2;
                    continue;
                }
                if let Some(u) = free_unit(&nlow) {
                    let mut adv = 2usize;
                    rel.push((n, u));
                    // "ago" 否定**此前**（尚未被否定过的）全部相对项：
                    // "2 days 3 hours ago" = -(2d+3h)，"2 days ago 3 hours"
                    // 只否定 2 days（tclsh 实测）。
                    if let Some(agt) = toks.get(ti + 2).copied() {
                        if joined(agt).to_ascii_lowercase() == "ago" {
                            for t in &mut rel[ago_watermark..] {
                                t.0 = -t.0;
                            }
                            ago_watermark = rel.len();
                            adv = 3;
                        }
                    }
                    ti += adv;
                    continue;
                }
            }
            // 裸数字：1-2 位 = 小时（"2" → 02:00），3-4 位按 HHMM
            // （"2000" → 20:00，"123" → 01:23）。
            if !had_comma && n >= 0 && core.len() <= 4 {
                if tset.is_some() {
                    return Err(more_than_one_time());
                }
                tset = if core.len() <= 2 {
                    Some((n, 0, 0))
                } else {
                    Some((n / 100, n % 100, 0))
                };
                ti += 1;
                continue;
            }
        }
        // 拼接缩写形（"est5edt"）：字母段都是时区缩写 → tclsh 的
        // "more than one time zone/time of day in string"。
        {
            let segs: Vec<String> = core
                .split(|c: char| c.is_ascii_digit())
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect();
            if segs.len() >= 2
                && segs.iter().all(|p| zone_abbrev(p).is_some())
            {
                let msg = if tset.is_some() {
                    "more than one time of day in string"
                } else {
                    "more than one time zone in string"
                };
                return Err(Error::Msg(format!(
                    "unable to convert date-time string {:?}: {}",
                    input, msg
                )));
            }
        }
        // 无法识别（若 token 以数字开头，错误位从首个非数字字符起：
        // 字母段 → 覆盖到 token 尾（"1st" → 1-2），标点 → 单字符
        // （"2000.01.02" → 4-4））。
        let (a, b) = span_of(ti);
        let mut a = a;
        let tb = &tok_chars;
        let mut q = 0usize;
        while q < tb.len() && tb[q].is_ascii_digit() {
            q += 1;
        }
        let mut b = b;
        if q > 0 && q < tb.len() {
            a = toks[ti].0 + q;
            if !tb[q].is_ascii_alphabetic() {
                b = a;
            }
        }
        let _ = had_comma;
        return Err(syntax_err(input, a, b));
    }

    // ── 组合 ──
    let Some((by, bmo, bd, bh, bmi, bs)) = clock_civil(base, use_gmt) else {
        return Ok(Value::from_int(base));
    };
    let mut y = by;
    let mut mo = bmo;
    let mut d = bd;
    let mut h = bh;
    let mut mi = bmi;
    let mut s = bs;

    let (base_jdn, base_wd) = {
        let jdn = jdn_from_civil(by, bmo, bd);
        (jdn, ((jdn + 1).rem_euclid(7)) as usize)
    };
    if let Some((ay, amo, ad)) = adate {
        y = ay.unwrap_or(by);
        mo = amo;
        d = ad;
    } else if let Some((dow, mode)) = weekday {
        let delta: i64 = match mode {
            WdMode::Plain => ((dow + 7 - base_wd) % 7) as i64,
            WdMode::Next => (((dow + 7 - base_wd) % 7) + 7) as i64,
            WdMode::Last => {
                let mut delta = (base_wd + 7 - dow) % 7;
                if delta == 0 {
                    delta = 7;
                }
                -(delta as i64)
            }
        };
        let (ny, nm, nd) = civil_from_jdn(base_jdn + delta);
        y = ny;
        mo = nm;
        d = nd;
    } else if let Some((mon, next)) = ordinal {
        // next <月名>：目标月 > base 月则同年，否则次年；日 = base 日
        // 夹到目标月长（tclsh 实测：next june 从 6 月 → 次年 6 月同日）。
        // last <月名>：(年, 月, 1) - 2 天；显式年份时 - 1 天（怪但实测
        // 钉形：last june → (Y,6,1)-2d；last june 2005 → 2005-05-31? 否
        // ——实测 last june 5 → 2005-05-30 = -2d）。
        if next {
            let year = by + if mon <= bmo { 1 } else { 0 };
            let j0 = jdn_from_civil(year, mon, 1);
            let j1 = if mon == 12 {
                jdn_from_civil(year + 1, 1, 1)
            } else {
                jdn_from_civil(year, mon + 1, 1)
            };
            let dim = j1 - j0;
            let (ny, nm, nd) = civil_from_jdn(j0 + (bd - 1).min(dim - 1));
            y = ny;
            mo = nm;
            d = nd;
        } else {
            let year = ordinal_year.unwrap_or(by);
            let (ny, nm, nd) = civil_from_jdn(jdn_from_civil(year, mon, 1) - 2);
            y = ny;
            mo = nm;
            d = nd;
        }
    }

    let day_scale_nonzero = rel.iter().any(|(n, u)| {
        *n != 0
            && !matches!(u, FreeUnit::Sec | FreeUnit::Min | FreeUnit::Hour)
    });
    if adate.is_some() || weekday.is_some() || ordinal.is_some() || day_scale_nonzero {
        h = 0;
        mi = 0;
        s = 0;
    }
    if let Some((th, tm, tsx)) = tset {
        h = th;
        mi = tm;
        s = tsx;
    }

    // 相对量：日历级（日/周/双周/月/年）改字段，时间级（时/分/秒）改秒数。
    let mut small_secs = 0i64;
    for (n, u) in &rel {
        match u {
            FreeUnit::Sec => small_secs = small_secs.saturating_add(*n),
            FreeUnit::Min => small_secs = small_secs.saturating_add(n.saturating_mul(60)),
            FreeUnit::Hour => small_secs = small_secs.saturating_add(n.saturating_mul(3600)),
            FreeUnit::Day | FreeUnit::Week | FreeUnit::Fortnight => {
                let days = match u {
                    FreeUnit::Day => *n,
                    FreeUnit::Week => n.saturating_mul(7),
                    _ => n.saturating_mul(14),
                };
                let (ny, nm, nd) = civil_from_jdn(jdn_from_civil(y, mo, d) + days);
                y = ny;
                mo = nm;
                d = nd;
            }
            FreeUnit::Month | FreeUnit::Year => {
                let months = if matches!(u, FreeUnit::Year) {
                    n.saturating_mul(12)
                } else {
                    *n
                };
                let total = y.saturating_mul(12).saturating_add(mo - 1).saturating_add(months);
                let ny = total.div_euclid(12);
                let nm = total.rem_euclid(12) + 1;
                let dim = if nm == 2 {
                    if jdn_from_civil(ny, nm, 1) < GREGORIAN_CHANGE_JDN {
                        days_in_month_julian(ny, 2)
                    } else {
                        days_in_month(ny, 2)
                    }
                } else {
                    days_in_month(ny, nm)
                };
                let nd = d.min(dim);
                y = ny;
                mo = nm;
                d = nd;
            }
        }
    }

    let epoch = if let Some(off) = zone {
        epoch_from_jdn(jdn_from_civil(y, mo, d), h, mi, s) - off
    } else {
        clock_epoch(y, mo, d, h, mi, s, use_gmt)
    };
    Ok(Value::from_int(epoch.saturating_add(small_secs)))
}

#[derive(Clone, Copy, PartialEq)]
enum FreeUnit {
    Sec,
    Min,
    Hour,
    Day,
    Week,
    Fortnight,
    Month,
    Year,
}

#[derive(Clone, Copy, PartialEq)]
enum WdMode {
    Plain,
    Next,
    Last,
}

fn free_unit(w: &str) -> Option<FreeUnit> {
    match w {
        "second" | "seconds" | "sec" | "secs" => Some(FreeUnit::Sec),
        "minute" | "minutes" => Some(FreeUnit::Min),
        "hour" | "hours" => Some(FreeUnit::Hour),
        "day" | "days" => Some(FreeUnit::Day),
        "week" | "weeks" => Some(FreeUnit::Week),
        "fortnight" | "fortnights" => Some(FreeUnit::Fortnight),
        "month" | "months" => Some(FreeUnit::Month),
        "year" | "years" => Some(FreeUnit::Year),
        _ => None,
    }
}

fn free_weekday(w: &str) -> Option<usize> {
    let mut hit: Option<usize> = None;
    for (i, name) in WEEKDAY_NAMES.iter().enumerate() {
        if name.starts_with(w) {
            if hit.is_some() {
                return None;
            }
            hit = Some(i);
        }
    }
    hit
}

fn free_month(w: &str) -> Option<i64> {
    // tclsh 只收全称、三字母缩写与 "sept"（"Ja"/"Janu" 均拒，实测）。
    const ABBREV: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    for (i, (name, ab)) in MONTH_NAMES.iter().zip(ABBREV.iter()).enumerate() {
        if w == *name || w == *ab || (i == 8 && w == "sept") {
            return Some(i as i64 + 1);
        }
    }
    None
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
                    let Some((y, mo, d, h, mi, s)) = clock_civil(epoch, false) else { continue };
                    let (ny, nm, nd) = civil_from_jdn(jdn_from_civil(y, mo, d) + days);
                    epoch = clock_epoch(ny, nm, nd, h, mi, s, false);
                }
            }
            _ => {
                // 年 / 月：日历月运算 + 月末夹取（tclsh：Jan 31 +1m =
                // Feb 29、Feb 29 +1y = Feb 28）。
                let months = if unit.starts_with("year") { n.saturating_mul(12) } else { *n };
                let (y, mo, d, h, mi, s) = match clock_civil(epoch, gmt) {
                    Some(t) => t,
                    None => continue,
                };
                let total = y.saturating_mul(12).saturating_add(mo - 1).saturating_add(months);
                let ny = total.div_euclid(12);
                let nm = total.rem_euclid(12) + 1;
                // 月末夹取：儒略历区（<1582-10-15）用儒略闰年规则。
                let calendar_jdn = jdn_from_civil(ny, nm, 1);
                let dim = if nm == 2 {
                    if calendar_jdn < GREGORIAN_CHANGE_JDN {
                        days_in_month_julian(ny, 2)
                    } else {
                        days_in_month(ny, 2)
                    }
                } else {
                    days_in_month(ny, nm)
                };
                let nd = d.min(dim);
                epoch = clock_epoch(ny, nm, nd, h, mi, s, gmt);
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

// ── 儒略历/格里历切换（tclsh 8.6.17 实测：1582-10-15 切换）──────────────

/// JDN of the Posix epoch day (1970-01-01).
const JDN_POSIX_EPOCH: i64 = 2440588;
/// 1582-10-15（格里历）的 JDN——此日之前用儒略历，之后用格里历
/// （tclsh 8.6.17 实测：1582-10-04 Julian = JDN 2299160，1582-10-15
/// Gregorian = 2299161；1752 年前后无跳变）。
const GREGORIAN_CHANGE_JDN: i64 = 2299161;

/// 格里历 JDN（不处理切换）。
fn gregorian_jdn(y: i64, m: i64, d: i64) -> i64 {
    days_from_civil(y, m, d) + JDN_POSIX_EPOCH
}

/// 儒略历 JDN（Tcl `GetJulianDayFromEraYearMonthDay` 的 Julian 臂）。
fn julian_jdn(y: i64, m: i64, d: i64) -> i64 {
    let ym1 = y - 1;
    let mut prior = 0i64;
    for mm in 1..m {
        prior += days_in_month_julian(y, mm);
    }
    1721424 - 1 + d + prior + 365 * ym1 + ym1.div_euclid(4)
}

fn days_in_month_julian(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y.rem_euclid(4) == 0 {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

/// 民用历 → JDN：先按格里历算，落在切换点之前改用儒略历（Tcl 语义）；
/// 月份先按 Tcl 的方式归约（13 月 → 次年 1 月）。
fn jdn_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let q = (m - 1).div_euclid(12);
    let y = y + q;
    let m = (m - 1).rem_euclid(12) + 1;
    let greg = gregorian_jdn(y, m, d);
    if greg < GREGORIAN_CHANGE_JDN {
        julian_jdn(y, m, d)
    } else {
        greg
    }
}

/// JDN → 民用历（切换点之前用儒略历逆式，之后格里历）。
fn civil_from_jdn(jdn: i64) -> (i64, i64, i64) {
    if jdn < GREGORIAN_CHANGE_JDN {
        // Julian inverse（Richards）。
        let c = jdn + 32082;
        let d1 = (4 * c + 3) / 1461;
        let e = c - 1461 * d1 / 4;
        let m = (5 * e + 2) / 153;
        let day = e - (153 * m + 2) / 5 + 1;
        let month = m + 3 - 12 * (m / 10);
        (d1 - 4800 + m / 10, month, day)
    } else {
        civil_from_days(jdn - JDN_POSIX_EPOCH)
    }
}

fn epoch_from_jdn(jdn: i64, h: i64, mi: i64, s: i64) -> i64 {
    (jdn - JDN_POSIX_EPOCH) * 86400 + h * 3600 + mi * 60 + s
}

/// epoch 的日历分解（tclsh 的儒略/格里历切换；gmt=纯 UTC）。
fn clock_civil(epoch: i64, gmt: bool) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let local = if gmt {
        epoch
    } else {
        epoch + local_offset_name(epoch)?.0 as i64
    };
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    let (y, mo, d) = civil_from_jdn(days + JDN_POSIX_EPOCH);
    Some((y, mo, d, secs / 3600, (secs % 3600) / 60, secs % 60))
}

/// 日历字段 → epoch（现代日期走 mktime 以保留 DST 消解；古日期用
/// JDN + 该时刻的本地偏移）。
fn clock_epoch(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, gmt: bool) -> i64 {
    if gmt {
        epoch_from_jdn(jdn_from_civil(y, mo, d), h, mi, s)
    } else if y >= 1583 {
        local_from_civil(y, mo, d, h, mi, s)
    } else {
        let utc = epoch_from_jdn(jdn_from_civil(y, mo, d), h, mi, s);
        utc - local_offset_name(utc).map(|o| o.0 as i64).unwrap_or(0)
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

    // D-25：儒略历/格里历切换（tclsh 8.6.17：<1582-10-15 用儒略历）。
    let jdn = days + JDN_POSIX_EPOCH;
    let (year, m, d) = civil_from_jdn(jdn);
    let julian = jdn < GREGORIAN_CHANGE_JDN;

    // Day of week: 0=Sunday（JDN 连续，切换无关）。
    let wday = ((jdn + 1).rem_euclid(7)) as u32;

    // Day of year（闰年规则随历法）。
    let leap = if julian {
        year.rem_euclid(4) == 0
    } else {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
    };
    let month_days: [u32; 12] = [
        31,
        if leap { 29 } else { 28 },
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
    let mut yday = d as i64;
    for &md in &month_days[..(m as usize - 1)] {
        yday += md as i64;
    }

    DateTime {
        year,
        month: m as u32,
        day: d as u32,
        hour,
        minute,
        second,
        wday,
        yday: yday as u32 - 1,
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
