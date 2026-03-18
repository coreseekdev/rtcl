//! Miscellaneous commands: set, expr, incr, unset, info, subst, append, disassemble.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;
use rtcl_parser::Compiler;

/// Get the hostname (cross-platform via environment variables).
#[cfg(feature = "std")]
fn hostname_get() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string())
}

// ---------- Arithmetic operator commands: +, -, *, / ----------

/// `+ ?number ...?` — Sum all arguments (0 if none).
pub fn cmd_add(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut int_sum: i64 = 0;
    let mut use_float = false;
    let mut float_sum: f64 = 0.0;

    for arg in &args[1..] {
        if use_float {
            float_sum += arg.as_float().ok_or_else(|| {
                Error::runtime(
                    format!("expected number but got \"{}\"", arg.as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
        } else if let Some(i) = arg.as_int() {
            int_sum += i;
        } else if let Some(f) = arg.as_float() {
            use_float = true;
            float_sum = int_sum as f64 + f;
        } else {
            return Err(Error::runtime(
                format!("expected number but got \"{}\"", arg.as_str()),
                crate::error::ErrorCode::Generic,
            ));
        }
    }
    if use_float {
        Ok(Value::from_float(float_sum))
    } else {
        Ok(Value::from_int(int_sum))
    }
}

/// `* ?number ...?` — Multiply all arguments (1 if none).
pub fn cmd_mul(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let mut int_prod: i64 = 1;
    let mut use_float = false;
    let mut float_prod: f64 = 1.0;

    for arg in &args[1..] {
        if use_float {
            float_prod *= arg.as_float().ok_or_else(|| {
                Error::runtime(
                    format!("expected number but got \"{}\"", arg.as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
        } else if let Some(i) = arg.as_int() {
            int_prod *= i;
        } else if let Some(f) = arg.as_float() {
            use_float = true;
            float_prod = int_prod as f64 * f;
        } else {
            return Err(Error::runtime(
                format!("expected number but got \"{}\"", arg.as_str()),
                crate::error::ErrorCode::Generic,
            ));
        }
    }
    if use_float {
        Ok(Value::from_float(float_prod))
    } else {
        Ok(Value::from_int(int_prod))
    }
}

/// `- number ?number ...?` — Unary negation (1 arg) or subtract remaining from first.
pub fn cmd_sub(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("-", 2, args.len(), "number ?number ...?"));
    }

    if args.len() == 2 {
        // Unary negation
        if let Some(i) = args[1].as_int() {
            return Ok(Value::from_int(-i));
        }
        if let Some(f) = args[1].as_float() {
            return Ok(Value::from_float(-f));
        }
        return Err(Error::runtime(
            format!("expected number but got \"{}\"", args[1].as_str()),
            crate::error::ErrorCode::Generic,
        ));
    }

    // Multi-arg: subtract from first
    let mut use_float = false;
    let mut int_val: i64 = 0;
    let mut float_val: f64 = 0.0;

    if let Some(i) = args[1].as_int() {
        int_val = i;
    } else if let Some(f) = args[1].as_float() {
        use_float = true;
        float_val = f;
    } else {
        return Err(Error::runtime(
            format!("expected number but got \"{}\"", args[1].as_str()),
            crate::error::ErrorCode::Generic,
        ));
    }

    for arg in &args[2..] {
        if use_float {
            float_val -= arg.as_float().ok_or_else(|| {
                Error::runtime(
                    format!("expected number but got \"{}\"", arg.as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
        } else if let Some(i) = arg.as_int() {
            int_val -= i;
        } else if let Some(f) = arg.as_float() {
            use_float = true;
            float_val = int_val as f64 - f;
        } else {
            return Err(Error::runtime(
                format!("expected number but got \"{}\"", arg.as_str()),
                crate::error::ErrorCode::Generic,
            ));
        }
    }
    if use_float {
        Ok(Value::from_float(float_val))
    } else {
        Ok(Value::from_int(int_val))
    }
}

/// `/ number ?number ...?` — Reciprocal (1 arg) or divide first by remaining.
pub fn cmd_div(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("/", 2, args.len(), "number ?number ...?"));
    }

    if args.len() == 2 {
        // Reciprocal: 1/x
        let f = args[1].as_float().ok_or_else(|| {
            Error::runtime(
                format!("expected number but got \"{}\"", args[1].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })?;
        if f == 0.0 {
            return Err(Error::runtime("Division by zero", crate::error::ErrorCode::Generic));
        }
        return Ok(Value::from_float(1.0 / f));
    }

    // Multi-arg: divide first by remaining
    let mut use_float = false;
    let mut int_val: i64 = 0;
    let mut float_val: f64 = 0.0;

    if let Some(i) = args[1].as_int() {
        int_val = i;
    } else if let Some(f) = args[1].as_float() {
        use_float = true;
        float_val = f;
    } else {
        return Err(Error::runtime(
            format!("expected number but got \"{}\"", args[1].as_str()),
            crate::error::ErrorCode::Generic,
        ));
    }

    for arg in &args[2..] {
        if use_float {
            let d = arg.as_float().ok_or_else(|| {
                Error::runtime(
                    format!("expected number but got \"{}\"", arg.as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            if d == 0.0 {
                return Err(Error::runtime("Division by zero", crate::error::ErrorCode::Generic));
            }
            float_val /= d;
        } else if let Some(d) = arg.as_int() {
            if d == 0 {
                return Err(Error::runtime("Division by zero", crate::error::ErrorCode::Generic));
            }
            int_val /= d;
        } else if let Some(d) = arg.as_float() {
            if d == 0.0 {
                return Err(Error::runtime("Division by zero", crate::error::ErrorCode::Generic));
            }
            use_float = true;
            float_val = int_val as f64 / d;
        } else {
            return Err(Error::runtime(
                format!("expected number but got \"{}\"", arg.as_str()),
                crate::error::ErrorCode::Generic,
            ));
        }
    }
    if use_float {
        Ok(Value::from_float(float_val))
    } else {
        Ok(Value::from_int(int_val))
    }
}

// ---------- env ----------

/// `env ?varName? ?default?` — Read environment variables.
#[cfg(feature = "env")]
pub fn cmd_env(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    match args.len() {
        1 => {
            // Return flat list of all env vars: key val key val ...
            let mut list = Vec::new();
            for (k, v) in std::env::vars() {
                list.push(Value::from_str(&k));
                list.push(Value::from_str(&v));
            }
            Ok(Value::from_list(&list))
        }
        2 => {
            let key = args[1].as_str();
            match std::env::var(key) {
                Ok(val) => Ok(Value::from_str(&val)),
                Err(_) => Err(Error::runtime(
                    format!("environment variable \"{}\" does not exist", key),
                    crate::error::ErrorCode::NotFound,
                )),
            }
        }
        3 => {
            let key = args[1].as_str();
            match std::env::var(key) {
                Ok(val) => Ok(Value::from_str(&val)),
                Err(_) => Ok(args[2].clone()), // default
            }
        }
        _ => Err(Error::wrong_args_with_usage("env", 1, args.len(), "?varName? ?default?")),
    }
}

// ---------- rand ----------

/// `rand ?min? ?max?` — Generate random integer.
pub fn cmd_rand(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let (min, max) = match args.len() {
        1 => (0i64, i64::MAX),
        2 => {
            let m = args[1].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[1].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            (0, m)
        }
        3 => {
            let lo = args[1].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[1].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            let hi = args[2].as_int().ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", args[2].as_str()),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            (lo, hi)
        }
        _ => return Err(Error::wrong_args_with_usage("rand", 1, args.len(), "?min? ?max?")),
    };
    if max < min {
        return Err(Error::runtime(
            "Invalid arguments (max < min)",
            crate::error::ErrorCode::Generic,
        ));
    }
    let len = (max - min) as u64;
    if len == 0 {
        return Ok(Value::from_int(min));
    }
    // Simple PRNG using system time as seed (no external dep)
    let r = simple_random(len);
    Ok(Value::from_int(min + r as i64))
}

/// Simple pseudo-random number in [0, range) using time-based entropy.
fn simple_random(range: u64) -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);

    // Seed from time on first call
    let mut s = STATE.load(Ordering::Relaxed);
    if s == 0 {
        #[cfg(feature = "std")]
        {
            s = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(12345678);
        }
        #[cfg(not(feature = "std"))]
        {
            s = 6364136223846793005; // fixed seed for no_std
        }
    }
    // xorshift64
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    STATE.store(s, Ordering::Relaxed);
    s % range
}

// ---------- debug ----------

/// `debug subcommand ?arg ...?` — Interpreter debug introspection.
pub fn cmd_debug(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("debug", 2, args.len(), "subcommand ?arg ...?"));
    }
    let sub = args[1].as_str();
    match sub {
        "refcount" => {
            // Always 1 for rtcl (Rust ownership model)
            Ok(Value::from_int(1))
        }
        "objcount" => {
            // Return 0 — Rust manages memory, no free-list
            Ok(Value::from_int(0))
        }
        "invstr" => {
            // No-op — Rust strings are immutable
            Ok(Value::empty())
        }
        "scriptlen" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage("debug scriptlen", 3, args.len(), "script"));
            }
            let compiled = rtcl_parser::Compiler::compile_script(args[2].as_str())
                .map_err(|e| Error::runtime(e.to_string(), crate::error::ErrorCode::Generic))?;
            Ok(Value::from_int(compiled.ops().len() as i64))
        }
        "exprlen" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage("debug exprlen", 3, args.len(), "expression"));
            }
            let expr_script = format!("expr {{{}}}", args[2].as_str());
            let compiled = rtcl_parser::Compiler::compile_script(&expr_script)
                .map_err(|e| Error::runtime(e.to_string(), crate::error::ErrorCode::Generic))?;
            Ok(Value::from_int(compiled.ops().len() as i64))
        }
        "show" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage("debug show", 3, args.len(), "object"));
            }
            let v = &args[2];
            let detail = format!("type=string, len={}, value={}", v.as_str().len(), v.as_str());
            Ok(Value::from_str(&detail))
        }
        "tainted" => {
            // List tainted variable names
            let mut names: Vec<Value> = interp.tainted_vars.keys()
                .map(|k| Value::from_str(k))
                .collect();
            names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&names))
        }
        _ => Err(Error::runtime(
            format!("unknown debug subcommand \"{}\": must be refcount, objcount, invstr, scriptlen, exprlen, show, or tainted", sub),
            crate::error::ErrorCode::Generic,
        )),
    }
}

// ---------- xtrace ----------

/// `xtrace callback` — Set/clear execution trace callback. Empty string disables.
pub fn cmd_xtrace(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage("xtrace", 2, args.len(), "callback"));
    }
    interp.xtrace_callback = args[1].as_str().to_string();
    Ok(Value::empty())
}

pub fn cmd_set(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    match args.len() {
        2 => interp.get_var(args[1].as_str()).cloned(),
        3 => interp.set_var(args[1].as_str(), args[2].clone()),
        _ => Err(Error::wrong_args("set", 2, args.len())),
    }
}

pub fn cmd_expr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("expr", 2, args.len()));
    }
    let expr_str = if args.len() == 2 {
        args[1].as_str().to_string()
    } else {
        args[1..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };
    interp.eval_expr(&expr_str)
}

pub fn cmd_incr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args("incr", 2, args.len()));
    }
    let var_name = args[1].as_str();
    let amount = if args.len() == 3 {
        args[2].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", args[2].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })?
    } else {
        1
    };
    let current = match interp.get_var(var_name) {
        Ok(v) => v.as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", v.as_str()),
                crate::error::ErrorCode::Generic,
            )
        })?,
        Err(_) => 0, // Tcl: incr on non-existent var starts at 0
    };
    let new_val = Value::from_int(current + amount);
    interp.set_var(var_name, new_val)
}

pub fn cmd_unset(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("unset", 2, args.len()));
    }
    let mut nocomplain = false;
    let start = if args[1].as_str() == "-nocomplain" {
        nocomplain = true;
        2
    } else {
        1
    };
    for arg in &args[start..] {
        let name = arg.as_str();
        if !interp.var_exists(name) && !nocomplain {
            return Err(Error::runtime(
                format!("can't unset \"{}\": no such variable", name),
                crate::error::ErrorCode::NotFound,
            ));
        }
        let _ = interp.unset_var(name);
    }
    Ok(Value::empty())
}

pub fn cmd_info(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("info", 2, args.len()));
    }

    let subcmd = args[1].as_str();
    match subcmd {
        "commands" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut cmds: Vec<Value> = interp
                .commands
                .keys()
                .chain(interp.procs.keys())
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            cmds.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&cmds))
        }
        "procs" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut names: Vec<Value> = interp
                .procs
                .keys()
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&names))
        }
        "exists" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info exists", 3, args.len()));
            }
            let name = args[2].as_str();
            Ok(Value::from_bool(interp.var_exists(name)))
        }
        "vars" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut vars: Vec<Value> = interp
                .scope_vars()
                .keys()
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&vars))
        }
        "globals" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut vars: Vec<Value> = interp
                .globals
                .keys()
                .filter(|name| {
                    pattern
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&vars))
        }
        "body" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info body", 3, args.len()));
            }
            let name = args[2].as_str();
            if let Some(proc_def) = interp.procs.get(name) {
                Ok(Value::from_str(&proc_def.body))
            } else {
                Err(Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                ))
            }
        }
        "args" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info args", 3, args.len()));
            }
            let name = args[2].as_str();
            if let Some(proc_def) = interp.procs.get(name) {
                let arg_names: Vec<Value> = proc_def
                    .params
                    .iter()
                    .map(|(n, _)| Value::from_str(n))
                    .collect();
                Ok(Value::from_list(&arg_names))
            } else {
                Err(Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                ))
            }
        }
        "level" => Ok(Value::from_int(interp.frames.len() as i64)),
        "complete" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info complete", 3, args.len()));
            }
            Ok(Value::from_bool(rtcl_parser::is_complete(args[2].as_str())))
        }
        #[cfg(feature = "std")]
        "script" => Ok(Value::from_str(interp.script_name())),
        "locals" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            if let Some(frame) = interp.frames.last() {
                let mut vars: Vec<Value> = frame.locals.keys()
                    .filter(|name| {
                        pattern.map(|p| super::super::glob_match(p, name)).unwrap_or(true)
                    })
                    .map(|name| Value::from_str(name))
                    .collect();
                vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                Ok(Value::from_list(&vars))
            } else {
                Ok(Value::from_str(""))
            }
        }
        #[cfg(feature = "std")]
        "channels" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut chans: Vec<Value> = interp.channels.channel_names()
                .into_iter()
                .filter(|name| {
                    pattern.map(|p| super::super::glob_match(p, name)).unwrap_or(true)
                })
                .map(Value::from_str)
                .collect();
            chans.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&chans))
        }
        "version" => Ok(Value::from_str("8.6")),
        "patchlevel" => Ok(Value::from_str("8.6.0-rtcl")),
        "hostname" => {
            #[cfg(feature = "std")]
            {
                let name = hostname_get();
                Ok(Value::from_str(&name))
            }
            #[cfg(not(feature = "std"))]
            Ok(Value::from_str("localhost"))
        }
        "nameofexecutable" => {
            #[cfg(feature = "std")]
            {
                let exe = std::env::current_exe()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(Value::from_str(&exe))
            }
            #[cfg(not(feature = "std"))]
            Ok(Value::from_str(""))
        }
        "returncodes" => {
            if args.len() == 3 {
                // info returncodes code → name
                let code = args[2].as_int().unwrap_or(-1);
                let name = match code {
                    0 => "ok",
                    1 => "error",
                    2 => "return",
                    3 => "break",
                    4 => "continue",
                    _ => "unknown",
                };
                Ok(Value::from_str(name))
            } else {
                // info returncodes → list
                Ok(Value::from_str("ok error return break continue"))
            }
        }
        "alias" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info alias", 3, args.len()));
            }
            let name = args[2].as_str();
            if let Some(info) = interp.aliases.get(name) {
                let mut parts = vec![Value::from_str(&info.target)];
                for a in &info.prefix_args {
                    parts.push(Value::from_str(a));
                }
                Ok(Value::from_list(&parts))
            } else {
                Err(Error::runtime(
                    format!("\"{}\" is not an alias", name),
                    crate::error::ErrorCode::NotFound,
                ))
            }
        }
        "aliases" => {
            let mut names: Vec<Value> = interp.aliases.keys()
                .map(|name| Value::from_str(name))
                .collect();
            names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&names))
        }
        _ => Err(Error::runtime(
            format!("unknown info subcommand: {}", subcmd),
            crate::error::ErrorCode::InvalidOp,
        )),
    }
}

pub fn cmd_subst(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("subst", 2, args.len()));
    }

    let mut nobackslashes = false;
    let mut nocommands = false;
    let mut novariables = false;
    let mut i = 1;

    while i < args.len() - 1 {
        match args[i].as_str() {
            "-nobackslashes" => nobackslashes = true,
            "-nocommands" => nocommands = true,
            "-novariables" => novariables = true,
            _ => break,
        }
        i += 1;
    }

    let template = args[i].as_str();
    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut ci = 0;

    while ci < chars.len() {
        let ch = chars[ci];
        match ch {
            '\\' if !nobackslashes && ci + 1 < chars.len() => {
                ci += 1;
                match chars[ci] {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    other => { result.push('\\'); result.push(other); }
                }
                ci += 1;
            }
            '$' if !novariables && ci + 1 < chars.len() => {
                ci += 1;
                let mut var_name = String::new();
                if ci < chars.len() && chars[ci] == '{' {
                    ci += 1;
                    while ci < chars.len() && chars[ci] != '}' {
                        var_name.push(chars[ci]);
                        ci += 1;
                    }
                    if ci < chars.len() { ci += 1; } // skip '}'
                } else {
                    while ci < chars.len() && (chars[ci].is_alphanumeric() || chars[ci] == '_') {
                        var_name.push(chars[ci]);
                        ci += 1;
                    }
                    // Check for array ref
                    if ci < chars.len() && chars[ci] == '(' {
                        var_name.push('(');
                        ci += 1;
                        while ci < chars.len() && chars[ci] != ')' {
                            var_name.push(chars[ci]);
                            ci += 1;
                        }
                        if ci < chars.len() {
                            var_name.push(')');
                            ci += 1;
                        }
                    }
                }
                if let Ok(val) = interp.get_var(&var_name) {
                    result.push_str(val.as_str());
                } else {
                    result.push('$');
                    result.push_str(&var_name);
                }
            }
            '[' if !nocommands => {
                ci += 1;
                let mut depth = 1;
                let mut cmd = String::new();
                while ci < chars.len() && depth > 0 {
                    if chars[ci] == '[' { depth += 1; }
                    else if chars[ci] == ']' {
                        depth -= 1;
                        if depth == 0 { ci += 1; break; }
                    }
                    cmd.push(chars[ci]);
                    ci += 1;
                }
                let val = interp.eval(&cmd)?;
                result.push_str(val.as_str());
            }
            _ => {
                result.push(ch);
                ci += 1;
            }
        }
    }

    Ok(Value::from_str(&result))
}

pub fn cmd_append(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("append", 2, args.len()));
    }
    let var_name = args[1].as_str();
    let mut current = interp.get_var(var_name).ok().map(|v| v.as_str().to_string()).unwrap_or_default();
    for arg in &args[2..] {
        current.push_str(arg.as_str());
    }
    let result = Value::from_str(&current);
    interp.set_var(var_name, result.clone())
}

/// `disassemble script` — compile and display the bytecode for a script.
pub fn cmd_disassemble(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("disassemble", 2, args.len()));
    }
    let script = args[1].as_str();
    let code = Compiler::compile_script(script)
        .map_err(|e| Error::syntax(e.to_string(), 0, 0))?;
    Ok(Value::from_str(&code.to_string()))
}

/// `scan string format ?varName ...?`
///
/// Parses `string` according to `format` (subset of C sscanf).
/// If varNames are given, stores results and returns count of conversions.
/// If no varNames, returns a list of converted values.
pub fn cmd_scan(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "scan",
            3,
            args.len(),
            "string format ?varName ...?",
        ));
    }

    let input = args[1].as_str();
    let format = args[2].as_str();
    let has_vars = args.len() > 3;

    let mut results: Vec<String> = Vec::new();
    let mut input_pos = 0;
    let fmt_bytes = format.as_bytes();
    let mut fmt_pos = 0;

    while fmt_pos < fmt_bytes.len() {
        if fmt_bytes[fmt_pos] == b'%' {
            fmt_pos += 1;
            if fmt_pos >= fmt_bytes.len() {
                break;
            }

            // Handle %%
            if fmt_bytes[fmt_pos] == b'%' {
                if input_pos < input.len() && input.as_bytes()[input_pos] == b'%' {
                    input_pos += 1;
                }
                fmt_pos += 1;
                continue;
            }

            // Check for suppress flag *
            let suppress = fmt_bytes[fmt_pos] == b'*';
            if suppress {
                fmt_pos += 1;
            }

            // Optional width
            let mut width: Option<usize> = None;
            let width_start = fmt_pos;
            while fmt_pos < fmt_bytes.len() && fmt_bytes[fmt_pos].is_ascii_digit() {
                fmt_pos += 1;
            }
            if fmt_pos > width_start {
                width = format[width_start..fmt_pos].parse().ok();
            }

            if fmt_pos >= fmt_bytes.len() {
                break;
            }

            let spec = fmt_bytes[fmt_pos];
            fmt_pos += 1;

            let inp = &input[input_pos..];

            match spec {
                b'd' | b'i' => {
                    // Skip whitespace
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let s = &trimmed[..max.min(trimmed.len())];
                    let end = s.find(|c: char| !c.is_ascii_digit() && c != '-' && c != '+')
                        .unwrap_or(s.len());
                    if end == 0 {
                        break;
                    }
                    let num_str = &s[..end];
                    if !suppress {
                        results.push(num_str.to_string());
                    }
                    input_pos += end;
                }
                b'x' | b'X' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let s = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X"))
                        .unwrap_or(trimmed);
                    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                        input_pos += 2;
                    }
                    let max = width.unwrap_or(s.len());
                    let end = s[..max.min(s.len())].find(|c: char| !c.is_ascii_hexdigit())
                        .unwrap_or(max.min(s.len()));
                    if end == 0 {
                        break;
                    }
                    let val = i64::from_str_radix(&s[..end], 16).unwrap_or(0);
                    if !suppress {
                        results.push(val.to_string());
                    }
                    input_pos += end;
                }
                b'o' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let end = trimmed[..max.min(trimmed.len())]
                        .find(|c: char| !('0'..='7').contains(&c))
                        .unwrap_or(max.min(trimmed.len()));
                    if end == 0 {
                        break;
                    }
                    let val = i64::from_str_radix(&trimmed[..end], 8).unwrap_or(0);
                    if !suppress {
                        results.push(val.to_string());
                    }
                    input_pos += end;
                }
                b'f' | b'e' | b'g' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let s = &trimmed[..max.min(trimmed.len())];
                    let end = s.find(|c: char| {
                        !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e' && c != 'E'
                    }).unwrap_or(s.len());
                    if end == 0 {
                        break;
                    }
                    if !suppress {
                        results.push(s[..end].to_string());
                    }
                    input_pos += end;
                }
                b's' => {
                    let trimmed = inp.trim_start();
                    input_pos += inp.len() - trimmed.len();
                    let max = width.unwrap_or(trimmed.len());
                    let s = &trimmed[..max.min(trimmed.len())];
                    let end = s.find(|c: char| c.is_whitespace()).unwrap_or(s.len());
                    if end == 0 {
                        break;
                    }
                    if !suppress {
                        results.push(s[..end].to_string());
                    }
                    input_pos += end;
                }
                b'c' => {
                    if inp.is_empty() {
                        break;
                    }
                    let ch = inp.chars().next().unwrap();
                    if !suppress {
                        results.push((ch as u32).to_string());
                    }
                    input_pos += ch.len_utf8();
                }
                b'n' => {
                    if !suppress {
                        results.push(input_pos.to_string());
                    }
                }
                _ => {
                    return Err(Error::runtime(
                        format!("bad scan conversion character '{}'", spec as char),
                        crate::error::ErrorCode::Generic,
                    ));
                }
            }
        } else if fmt_bytes[fmt_pos].is_ascii_whitespace() {
            // Whitespace in format matches any whitespace in input
            fmt_pos += 1;
            while input_pos < input.len() && input.as_bytes()[input_pos].is_ascii_whitespace() {
                input_pos += 1;
            }
        } else {
            // Literal match
            if input_pos < input.len() && input.as_bytes()[input_pos] == fmt_bytes[fmt_pos] {
                input_pos += 1;
                fmt_pos += 1;
            } else {
                break;
            }
        }
    }

    if has_vars {
        // Store in variables, return count
        let mut count = 0;
        for (idx, var_arg) in args[3..].iter().enumerate() {
            if let Some(val) = results.get(idx) {
                interp.set_var(var_arg.as_str(), Value::from_str(val))?;
                count += 1;
            }
        }
        Ok(Value::from_int(count))
    } else {
        // Return as list
        let list_str = results
            .iter()
            .map(|s| {
                if s.is_empty() || s.contains(' ') {
                    format!("{{{}}}", s)
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Value::from_str(&list_str))
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    // ── + ────────────────────────────────────────────────────────
    #[test]
    fn test_add_no_args() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("+").unwrap().as_str(), "0");
    }

    #[test]
    fn test_add_integers() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("+ 1 2 3").unwrap().as_str(), "6");
    }

    #[test]
    fn test_add_floats() {
        let mut interp = Interp::new();
        let r = interp.eval("+ 1 2.5 3").unwrap();
        assert_eq!(r.as_float(), Some(6.5));
    }

    #[test]
    fn test_add_single() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("+ 42").unwrap().as_str(), "42");
    }

    #[test]
    fn test_add_non_number() {
        let mut interp = Interp::new();
        assert!(interp.eval("+ 1 abc").is_err());
    }

    // ── - ────────────────────────────────────────────────────────
    #[test]
    fn test_sub_negate() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("- 5").unwrap().as_str(), "-5");
    }

    #[test]
    fn test_sub_negate_float() {
        let mut interp = Interp::new();
        let r = interp.eval("- 3.14").unwrap();
        assert_eq!(r.as_float(), Some(-3.14));
    }

    #[test]
    fn test_sub_multi() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("- 10 3").unwrap().as_str(), "7");
    }

    #[test]
    fn test_sub_multi_chain() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("- 100 20 30").unwrap().as_str(), "50");
    }

    #[test]
    fn test_sub_no_args_error() {
        let mut interp = Interp::new();
        assert!(interp.eval("-").is_err());
    }

    // ── * ────────────────────────────────────────────────────────
    #[test]
    fn test_mul_no_args() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("*").unwrap().as_str(), "1");
    }

    #[test]
    fn test_mul_integers() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("* 2 3 4").unwrap().as_str(), "24");
    }

    #[test]
    fn test_mul_float_promotion() {
        let mut interp = Interp::new();
        let r = interp.eval("* 2 1.5").unwrap();
        assert_eq!(r.as_float(), Some(3.0));
    }

    // ── / ────────────────────────────────────────────────────────
    #[test]
    fn test_div_reciprocal() {
        let mut interp = Interp::new();
        let r = interp.eval("/ 4.0").unwrap();
        assert_eq!(r.as_float(), Some(0.25));
    }

    #[test]
    fn test_div_integer() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("/ 12 3").unwrap().as_str(), "4");
    }

    #[test]
    fn test_div_chain() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("/ 120 2 3").unwrap().as_str(), "20");
    }

    #[test]
    fn test_div_by_zero() {
        let mut interp = Interp::new();
        assert!(interp.eval("/ 10 0").is_err());
    }

    #[test]
    fn test_div_by_zero_float() {
        let mut interp = Interp::new();
        assert!(interp.eval("/ 10.0 0.0").is_err());
    }

    #[test]
    fn test_div_reciprocal_zero() {
        let mut interp = Interp::new();
        assert!(interp.eval("/ 0.0").is_err());
    }

    #[test]
    fn test_div_no_args_error() {
        let mut interp = Interp::new();
        assert!(interp.eval("/").is_err());
    }

    // ── env ──────────────────────────────────────────────────────
    #[cfg(feature = "env")]
    #[test]
    fn test_env_list_all() {
        let mut interp = Interp::new();
        let r = interp.eval("env").unwrap();
        // Should return a non-empty flat list
        assert!(!r.as_str().is_empty());
    }

    #[cfg(feature = "env")]
    #[test]
    fn test_env_get_with_default() {
        let mut interp = Interp::new();
        let r = interp.eval("env RTCL_NONEXISTENT_VAR fallback").unwrap();
        assert_eq!(r.as_str(), "fallback");
    }

    #[cfg(feature = "env")]
    #[test]
    fn test_env_missing_error() {
        let mut interp = Interp::new();
        assert!(interp.eval("env RTCL_NONEXISTENT_VAR").is_err());
    }

    // ── rand ─────────────────────────────────────────────────────
    #[test]
    fn test_rand_no_args() {
        let mut interp = Interp::new();
        let r = interp.eval("rand").unwrap();
        assert!(r.as_int().is_some());
    }

    #[test]
    fn test_rand_max() {
        let mut interp = Interp::new();
        let r = interp.eval("rand 10").unwrap();
        let v = r.as_int().unwrap();
        assert!((0..10).contains(&v));
    }

    #[test]
    fn test_rand_min_max() {
        let mut interp = Interp::new();
        let r = interp.eval("rand 5 10").unwrap();
        let v = r.as_int().unwrap();
        assert!((5..10).contains(&v));
    }

    #[test]
    fn test_rand_invalid_range() {
        let mut interp = Interp::new();
        assert!(interp.eval("rand 10 5").is_err());
    }

    // ── debug ────────────────────────────────────────────────────
    #[test]
    fn test_debug_refcount() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("debug refcount x").unwrap().as_str(), "1");
    }

    #[test]
    fn test_debug_objcount() {
        let mut interp = Interp::new();
        assert_eq!(interp.eval("debug objcount").unwrap().as_str(), "0");
    }

    #[test]
    fn test_debug_scriptlen() {
        let mut interp = Interp::new();
        let r = interp.eval("debug scriptlen {set x 1}").unwrap();
        assert!(r.as_int().unwrap() > 0);
    }

    #[test]
    fn test_debug_unknown_sub() {
        let mut interp = Interp::new();
        assert!(interp.eval("debug bogus").is_err());
    }

    #[test]
    fn test_debug_no_args() {
        let mut interp = Interp::new();
        assert!(interp.eval("debug").is_err());
    }

    // ── xtrace ───────────────────────────────────────────────────
    #[test]
    fn test_xtrace_set_clear() {
        let mut interp = Interp::new();
        interp.eval("xtrace mycallback").unwrap();
        assert_eq!(interp.xtrace_callback, "mycallback");
        interp.eval("xtrace {}").unwrap();
        assert_eq!(interp.xtrace_callback, "");
    }

    #[test]
    fn test_xtrace_wrong_args() {
        let mut interp = Interp::new();
        assert!(interp.eval("xtrace").is_err());
        assert!(interp.eval("xtrace a b").is_err());
    }
}
