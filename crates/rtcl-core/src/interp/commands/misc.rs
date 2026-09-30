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
        2 => interp.read_var(args[1].as_str()),
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

/// `tcl::mathop::+ ?value ...?`: fold operands with expr's numeric `+`
/// (ints stay ints, overflow promotes to double, any float forces float).
pub fn cmd_mathop_plus(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let _ = interp;
    let mut acc = Value::from_int(0);
    for arg in &args[1..] {
        acc = mathop_add(&acc, arg)?;
    }
    Ok(acc)
}

fn mathop_add(a: &Value, b: &Value) -> Result<Value> {
    if let (Some(x), Some(y)) = (a.as_int(), b.as_int()) {
        return Ok(match x.checked_add(y) {
            Some(r) => Value::from_int(r),
            None => crate::types::expr_funcs::float_value(x as f64 + y as f64),
        });
    }
    if let (Some(x), Some(y)) = (a.as_float().or(a.as_int().map(|i| i as f64)), b.as_float().or(b.as_int().map(|i| i as f64))) {
        return Ok(crate::types::expr_funcs::float_value(x + y));
    }
    Err(Error::runtime(
        format!("can't use non-numeric string as operand of \"+\""),
        crate::error::ErrorCode::InvalidOp,
    ))
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
        Err(e) => {
            // Tcl: incr on a non-existent variable starts at 0, but a
            // scalar/array type conflict propagates as a read error.
            if interp.var_exists(var_name) {
                return Err(e);
            }
            // Element-of-scalar also conflicts even though `info exists`
            // reports 0 for the element itself.
            if crate::interp::vars::is_type_conflict(&e) {
                return Err(e);
            }
            0
        }
    };
    let new_val = Value::from_int(current + amount);
    interp.set_var(var_name, new_val)
}

pub fn cmd_unset(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // tclsh: zero names is a no-op (set-old-7.2), not an error.
    if args.len() < 2 {
        return Ok(Value::empty());
    }
    let mut nocomplain = false;
    let mut start = if args[1].as_str() == "-nocomplain" {
        nocomplain = true;
        2
    } else {
        1
    };
    // Optional end-of-options marker (set-old-7.14/7.15): `unset --`
    // alone unsets nothing; `unset -- --` unsets the variable "--".
    if start < args.len() && args[start].as_str() == "--" {
        start += 1;
    }
    for arg in &args[start..] {
        let name = arg.as_str();
        if let Err(e) = interp.unset_var(name) {
            if !nocomplain {
                return Err(e);
            }
        }
    }
    Ok(Value::empty())
}

/// Glob-match a name against a pattern, tolerant of a leading `::` on
/// either side: tclsh's `info commands test_ns_basic::*` still finds
/// `::test_ns_basic::p`. Output keeps the stored name unchanged.
fn matches_qualified_glob(pattern: &str, name: &str) -> bool {
    let p = pattern.strip_prefix("::").unwrap_or(pattern);
    let n = name.strip_prefix("::").unwrap_or(name);
    super::super::glob_match(p, n)
}

/// tclsh's `info` subcommand list, in the order it prints on an
/// unknown-or-ambiguous error.
const INFO_CANONICAL_SUBCMDS: &[&str] = &[
    "args", "body", "class", "cmdcount", "commands", "complete", "coroutine", "default",
    "errorstack", "exists", "frame", "functions", "globals", "hostname", "level", "library",
    "loaded", "locals", "nameofexecutable", "object", "patchlevel", "procs", "script",
    "sharedlibextension", "tclversion", "vars",
];

/// Resolve an `info` subcommand word: exact matches win (including the
/// jimtflavored extras rtcl supports), otherwise a unique prefix of
/// tclsh's canonical set resolves; anything else is tclsh's
/// unknown-or-ambiguous error.
fn resolve_info_subcmd(sub_raw: &str) -> Result<String> {
    const EXACT_KNOWN: &[&str] = &[
        "args", "body", "commands", "complete", "exists", "globals", "hostname", "level",
        "locals", "nameofexecutable", "patchlevel", "procs", "script", "vars", "alias",
        "aliases", "channels", "version", "help", "returncodes", "usage", "frame",
        "stacktrace", "references", "tainted", "statics", "source",
    ];
    if EXACT_KNOWN.contains(&sub_raw) {
        return Ok(sub_raw.to_string());
    }
    let matches: Vec<&str> = INFO_CANONICAL_SUBCMDS
        .iter()
        .copied()
        .filter(|s| s.starts_with(sub_raw))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).to_string()),
        _ => {
            let list = INFO_CANONICAL_SUBCMDS.join(", ").replacen(
                ", vars",
                ", or vars",
                1,
            );
            Err(Error::runtime(
                format!(
                    "unknown or ambiguous subcommand \"{}\": must be {}",
                    sub_raw, list
                ),
                crate::error::ErrorCode::NotFound,
            ))
        }
    }
}

pub fn cmd_info(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("info", 2, args.len()));
    }

    let subcmd = resolve_info_subcmd(args[1].as_str())?;
    match subcmd.as_str() {
        "commands" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut cmds: Vec<Value> = interp
                .commands
                .keys()
                .chain(interp.procs.keys())
                .filter(|name| {
                    pattern
                        .map(|p| matches_qualified_glob(p, name))
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
                        .map(|p| matches_qualified_glob(p, name))
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
            let name = args[2].as_str().to_string();
            Ok(Value::from_bool(interp.exists_firing(&name)))
        }
        "vars" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            // A leading "::" in the pattern is scope qualification, not part
            // of the stored (canonical) key — but it does ask for qualified
            // results (tclsh: `info vars ::err*` → `::errorCode ...`).
            let (match_pat, qualified) = match pattern {
                Some(p) => (Some(p.strip_prefix("::").unwrap_or(p)), p.starts_with("::")),
                None => (None, false),
            };
            let mut vars: Vec<Value> = interp
                .scope_vars()
                .keys()
                .filter(|name| {
                    match_pat
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(|name| {
                    // Namespace variables are always reported qualified.
                    if qualified || name.contains("::") {
                        Value::from_str(&format!("::{}", name))
                    } else {
                        Value::from_str(name)
                    }
                })
                .collect();
            vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&vars))
        }
        "globals" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let match_pat = pattern.map(|p| p.strip_prefix("::").unwrap_or(p));
            let mut vars: Vec<Value> = interp
                .globals
                .keys()
                // Namespace variables are not globals (tclsh: `info globals
                // q::*` on a namespace var returns empty).
                .filter(|name| !name.contains("::"))
                .filter(|name| {
                    match_pat
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
        "usage" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info usage", 3, args.len()));
            }
            let name = args[2].as_str();
            match interp.command_usage(name) {
                Some(usage) => {
                    let full = if usage.is_empty() {
                        name.to_string()
                    } else {
                        format!("{} {}", name, usage)
                    };
                    Ok(Value::from_str(&full))
                }
                None => Err(Error::runtime(
                    format!("invalid command name \"{}\"", name),
                    crate::error::ErrorCode::NotFound,
                )),
            }
        }
        "help" => {
            if args.len() != 3 {
                return Err(Error::wrong_args("info help", 3, args.len()));
            }
            let name = args[2].as_str();
            match interp.command_help(name) {
                Some(help) if !help.is_empty() => Ok(Value::from_str(&help)),
                Some(_) => Ok(Value::from_str(
                    &format!("No help available for command \"{}\"", name)
                )),
                None => Err(Error::runtime(
                    format!("invalid command name \"{}\"", name),
                    crate::error::ErrorCode::NotFound,
                )),
            }
        }
        "frame" => {
            // info frame ?level? — return call frame details as a dict-style list
            let depth = interp.frames.len();
            let level = if args.len() > 2 {
                let n = args[2].as_int().ok_or_else(|| {
                    Error::runtime(
                        format!("expected integer but got \"{}\"", args[2].as_str()),
                        crate::error::ErrorCode::Generic,
                    )
                })?;
                if n < 0 {
                    // Negative: relative to current frame
                    let abs = (depth as i64 + n) as usize;
                    if abs >= depth {
                        return Err(Error::runtime(
                            "bad level",
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                    abs
                } else {
                    let abs = n as usize;
                    if abs >= depth {
                        return Err(Error::runtime(
                            "bad level",
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                    abs
                }
            } else {
                if depth == 0 {
                    // At global level
                    return Ok(Value::from_str("type source level 0 cmd {}"));
                }
                depth - 1
            };
            let vars: Vec<String> = if let Some(frame) = interp.frames.get(level) {
                frame.locals.keys().cloned().collect()
            } else {
                Vec::new()
            };
            let var_list = vars.join(" ");
            let result = format!("type proc level {} cmd {{}} locals {{{}}}", level, var_list);
            Ok(Value::from_str(&result))
        }
        "stacktrace" => {
            // info stacktrace — reuse the stacktrace command logic
            let depth = interp.frames.len();
            let mut entries = Vec::new();
            for i in (0..depth).rev() {
                let frame_info = format!("frame{}", i);
                entries.push(Value::from_str(&frame_info));
                entries.push(Value::from_str(""));
                entries.push(Value::from_int(0));
            }
            Ok(Value::from_list(&entries))
        }
        "references" => {
            // info references — list active reference IDs
            let mut refs: Vec<Value> = interp.references.keys()
                .map(|k| Value::from_str(k))
                .collect();
            refs.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&refs))
        }
        "tainted" => {
            // info tainted ?pattern? — list tainted variable names
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut vars: Vec<Value> = interp.tainted_vars.keys()
                .filter(|name| {
                    pattern.map(|p| super::super::glob_match(p, name)).unwrap_or(true)
                })
                .map(|name| Value::from_str(name))
                .collect();
            vars.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&vars))
        }
        "statics" => {
            // info statics procName — list static variables as {name value ...}
            if args.len() != 3 {
                return Err(Error::wrong_args("info statics", 3, args.len()));
            }
            let name = args[2].as_str();
            let proc_def = interp.procs.get(name).ok_or_else(|| {
                Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                )
            })?;
            let mut entries: Vec<Value> = Vec::new();
            let mut keys: Vec<&String> = proc_def.statics.keys().collect();
            keys.sort();
            for k in keys {
                entries.push(Value::from_str(k));
                entries.push(proc_def.statics[k].clone());
            }
            Ok(Value::from_list(&entries))
        }
        "source" => {
            // info source cmdName — return definition source location
            if args.len() != 3 {
                return Err(Error::wrong_args("info source", 3, args.len()));
            }
            let name = args[2].as_str();
            if !interp.procs.contains_key(name) && !interp.commands.contains_key(name) {
                return Err(Error::runtime(
                    format!("invalid command name \"{}\"", name),
                    crate::error::ErrorCode::NotFound,
                ));
            }
            // Source tracking will be enhanced when proc records file/line
            Ok(Value::from_str(""))
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
    // tclsh: `append var` with no values is a pure read — a missing
    // variable errors instead of being created.
    if args.len() == 2 {
        return interp.get_var(var_name).cloned();
    }
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
// ── scan ────────────────────────────────────────────────────────────
//
// A port of tcl 8.6's tclScan.c (ValidateFormat + Tcl_ScanObjCmd) plus
// the relevant parts of TclParseNumber (tclStrToD.c), pinned against
// tclsh 8.6.17 probes:
//   - integers accumulate in u64; values fitting u64 wrap through the
//     i64 reinterpretation (0xffffffffffffffff → -1) and the sign is
//     applied afterwards (-0xffffffffffffffff → 1); anything bigger
//     saturates to i64::MIN/MAX.
//   - %i: decimal, 0x hex, leading-0 octal; a bare or junk 0x prefix
//     accepts just the "0" (consumed length 1). No 0b/0o for %i.
//   - %x: bare hex or 0x prefix; %b: bare binary or 0b prefix; %o: no
//     prefixes at all.
//   - floats accept "inf" (case-insensitive, always consumes exactly
//     the 3 chars) but never NaN: "nan"/"NaN" parses OK in
//     TclParseNumber yet scan refuses the value, consuming nothing and
//     counting no conversion.
//   - underflow (-1 / empty list) happens only when a FAILED conversion
//     stopped at end-of-input (unlimited width) or exactly at the width
//     boundary (limited width); a plain mismatch stops quietly.

/// One parsed conversion specifier.
struct ScanSpec {
    op: u8,
    suppress: bool,
    width: Option<usize>,
    /// 1-based XPG `%n$` slot, when given.
    xpg: Option<usize>,
}

fn scan_err(msg: &str, code: &str, interp: &mut Interp) -> Error {
    super::list::set_error_code(interp, code);
    super::list::tcl_err(msg.to_string())
}

/// tclScan.c ValidateFormat: check the format and compute how many result
/// slots (variables) it needs.
fn validate_scan_format(
    interp: &mut Interp,
    format: &str,
    num_vars: usize,
) -> Result<(Vec<ScanSpec>, usize)> {
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0usize;
    let mut specs = Vec::new();
    let mut got_xpg = false;
    let mut got_sequential = false;
    let mut obj_index = 0usize; // sequential slot counter
    let mut xpg_size = 0usize;
    let mut nassign: Vec<u32> = vec![0; num_vars];

    macro_rules! nassign_slot {
        ($slot:expr) => {{
            let slot = $slot;
            if slot >= nassign.len() {
                let newsize = if xpg_size > 0 { xpg_size } else { nassign.len() + 16 };
                nassign.resize(newsize.max(slot + 1), 0);
            }
            slot
        }};
    }

    while i < chars.len() {
        let ch = chars[i];
        i += 1;
        if ch != '%' {
            continue;
        }
        // Reading past the end yields '\0', like TclUtfToUniChar at the end
        // of the format: a trailing "%", "%5" or "%1$" dies with
        // `bad scan conversion character "\0"`.
        macro_rules! next_ch {
            () => {
                if i < chars.len() {
                    let c = chars[i];
                    i += 1;
                    c
                } else {
                    '\0'
                }
            };
        }
        let mut ch = next_ch!();
        if ch == '%' {
            continue;
        }
        let suppress = ch == '*';
        if suppress {
            // Suppressed specs skip the XPG check entirely: `%*1$d`
            // reaches the width parser and then errors on '$'.
            ch = next_ch!();
        }
        let mut spec = ScanSpec { op: 0, suppress, width: None, xpg: None };
        let mut xpg_slot: Option<usize> = None;
        let mut size_mod = false; // SCAN_LONGER|SCAN_BIG seen (not 'h')

        if !suppress && ch.is_ascii_digit() {
            // Possible %n$ XPG specifier.
            let mut j = i - 1;
            let mut val: u64 = 0;
            while j < chars.len() && chars[j].is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add(chars[j] as u64 - '0' as u64);
                j += 1;
            }
            if j < chars.len() && chars[j] == '$' {
                i = j + 1;
                got_xpg = true;
                if got_sequential {
                    return Err(scan_err(
                        "cannot mix \"%\" and \"%n$\" conversion specifiers",
                        "TCL FORMAT MIXEDSPECTYPES",
                        interp,
                    ));
                }
                if val == 0 || val >= i32::MAX as u64 {
                    return Err(scan_err(
                        "\"%n$\" argument index out of range",
                        "TCL FORMAT INDEXRANGE",
                        interp,
                    ));
                }
                let slot = val as usize - 1;
                if num_vars > 0 && slot >= num_vars {
                    return Err(scan_err(
                        "\"%n$\" argument index out of range",
                        "TCL FORMAT INDEXRANGE",
                        interp,
                    ));
                } else if num_vars == 0 {
                    xpg_size = xpg_size.max(val as usize);
                }
                xpg_slot = Some(slot);
                ch = next_ch!();
            } else {
                // The digits are a width after all.
                got_sequential = true;
                if got_xpg {
                    return Err(scan_err(
                        "cannot mix \"%\" and \"%n$\" conversion specifiers",
                        "TCL FORMAT MIXEDSPECTYPES",
                        interp,
                    ));
                }
            }
        } else if !suppress {
            got_sequential = true;
            if got_xpg {
                return Err(scan_err(
                    "cannot mix \"%\" and \"%n$\" conversion specifiers",
                    "TCL FORMAT MIXEDSPECTYPES",
                    interp,
                ));
            }
        }

        // Width specifier.
        if ch.is_ascii_digit() {
            let mut j = i - 1;
            let mut val: usize = 0;
            while j < chars.len() && chars[j].is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add(chars[j] as usize - '0' as usize);
                j += 1;
            }
            i = j;
            spec.width = Some(val);
            ch = next_ch!();
        }

        // Size modifiers: 8.6 accepts L, l, ll, j, q (each rejecting
        // %c/%n/%s/%[ below) and h (allowed everywhere). z and t are 9.0
        // additions — 8.6.17 rejects them as bad conversion characters.
        match ch {
            'L' | 'j' | 'q' => {
                size_mod = true;
                ch = next_ch!();
            }
            'l' => {
                size_mod = true;
                if i < chars.len() && chars[i] == 'l' {
                    i += 1;
                }
                ch = next_ch!();
            }
            'h' => {
                ch = next_ch!();
            }
            _ => {}
        }

        if !suppress && num_vars > 0 && obj_index >= num_vars {
            // Sequential specifiers ran past the variable list.
            return Err(scan_err(
                "different numbers of variable names and field specifiers",
                "TCL FORMAT FIELDVARMISMATCH",
                interp,
            ));
        }

        match ch {
            'c' => {
                if spec.width.is_some() {
                    return Err(scan_err(
                        "field width may not be specified in %c conversion",
                        "TCL FORMAT BADWIDTH",
                        interp,
                    ));
                }
                if size_mod {
                    return Err(scan_err(
                        "field size modifier may not be specified in %c conversion",
                        "TCL FORMAT BADSIZE",
                        interp,
                    ));
                }
            }
            'n' | 's' | '[' => {
                if size_mod {
                    return Err(scan_err(
                        &format!(
                            "field size modifier may not be specified in %{} conversion",
                            ch
                        ),
                        "TCL FORMAT BADSIZE",
                        interp,
                    ));
                }
                if ch == '[' {
                    // The set body must contain a closing bracket; a leading
                    // ']' or '-' is a literal member.
                    if i >= chars.len() {
                        return Err(scan_err(
                            "unmatched [ in format string",
                            "TCL FORMAT BRACKET",
                            interp,
                        ));
                    }
                    if chars[i] == '^' {
                        i += 1;
                        if i >= chars.len() {
                            return Err(scan_err(
                                "unmatched [ in format string",
                                "TCL FORMAT BRACKET",
                                interp,
                            ));
                        }
                    }
                    if chars[i] == ']' {
                        i += 1;
                        if i >= chars.len() {
                            return Err(scan_err(
                                "unmatched [ in format string",
                                "TCL FORMAT BRACKET",
                                interp,
                            ));
                        }
                    }
                    while i < chars.len() && chars[i] != ']' {
                        i += 1;
                    }
                    if i >= chars.len() {
                        return Err(scan_err(
                            "unmatched [ in format string",
                            "TCL FORMAT BRACKET",
                            interp,
                        ));
                    }
                    i += 1; // the ']'
                }
            }
            'd' | 'e' | 'E' | 'f' | 'g' | 'G' | 'i' | 'o' | 'x' | 'X' | 'b' | 'u' => {}
            _ => {
                return Err(scan_err(
                    &format!("bad scan conversion character \"{}\"", ch),
                    "TCL FORMAT BADTYPE",
                    interp,
                ));
            }
        }

        spec.op = ch as u8;
        spec.xpg = xpg_slot;
        if !suppress {
            let slot = if let Some(x) = xpg_slot { x } else { obj_index };
            let s = nassign_slot!(slot);
            nassign[s] += 1;
            if xpg_slot.is_none() {
                obj_index += 1;
            }
        }
        specs.push(spec);
    }

    let total = if num_vars > 0 {
        num_vars
    } else if xpg_size > 0 {
        xpg_size
    } else {
        obj_index
    };
    for slot in 0..total {
        if nassign.get(slot).copied().unwrap_or(0) > 1 {
            return Err(scan_err(
                "variable is assigned by multiple \"%n$\" conversion specifiers",
                "TCL FORMAT POLYASSIGNED",
                interp,
            ));
        } else if xpg_size == 0 && nassign.get(slot).copied().unwrap_or(0) == 0 {
            return Err(scan_err(
                "variable is not assigned by any conversion specifiers",
                "TCL FORMAT UNASSIGNED",
                interp,
            ));
        }
    }
    Ok((specs, total))
}

/// Build a `[...]` character set: (members, ranges, exclude). Mirrors
/// tclScan.c BuildCharSet including reversed ranges (c-a matches abc).
fn build_char_set(fmt: &[char], start: usize) -> (Vec<char>, Vec<(char, char)>, bool, usize) {
    let mut i = start;
    let exclude = fmt[i] == '^';
    if exclude {
        i += 1;
    }
    let mut members: Vec<char> = Vec::new();
    let mut ranges: Vec<(char, char)> = Vec::new();
    // A leading ']' or '-' right after (optional) '^' is literal.
    if fmt[i] == ']' || fmt[i] == '-' {
        members.push(fmt[i]);
        i += 1;
    }
    while i < fmt.len() && fmt[i] != ']' {
        let ch = fmt[i];
        if ch == '-' {
            // '-' as last char before ']' is literal.
            if i + 1 >= fmt.len() || fmt[i + 1] == ']' {
                members.push(ch);
                i += 1;
                continue;
            }
            // Range: previous member becomes the range start.
            let start_ch = members.pop().unwrap_or(ch);
            let end_ch = fmt[i + 1];
            let (lo, hi) = if start_ch < end_ch {
                (start_ch, end_ch)
            } else {
                (end_ch, start_ch)
            };
            ranges.push((lo, hi));
            i += 2;
        } else {
            members.push(ch);
            i += 1;
        }
    }
    // i points at ']' (or end).
    (members, ranges, exclude, i + 1)
}

fn char_in_set(members: &[char], ranges: &[(char, char)], exclude: bool, ch: char) -> bool {
    let m = members.contains(&ch) || ranges.iter().any(|&(lo, hi)| lo <= ch && ch <= hi);
    m != exclude
}

fn is_tcl_space_ch(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
}

/// Result of a numeric scan attempt. `Fail` carries the char position the
/// parse stopped at (the offending char is NOT consumed).
enum NumScan {
    Ok(i64, usize),
    Fail(usize),
}

/// Turn an accumulated u64 significand into the scan result value.
/// Values fitting u64 wrap through the i64 reinterpretation with the sign
/// applied afterwards (tclsh: 0xffffffffffffffff → -1 but
/// -0xffffffffffffffff → 1); bigger magnitudes saturate.
fn finish_u64(mag: u64, overflow: bool, neg: bool) -> i64 {
    if overflow {
        if neg {
            i64::MIN
        } else {
            i64::MAX
        }
    } else {
        let v = mag as i64;
        if neg {
            v.wrapping_neg()
        } else {
            v
        }
    }
}

/// Accumulate digits in `base` from `*i` up to `end`; advances `*i` past
/// every digit consumed. Returns (significand, overflowed).
fn acc_radix(s: &[char], i: &mut usize, end: usize, base: u32) -> (u64, bool) {
    let mut mag: u64 = 0;
    let mut ovf = false;
    while *i < end {
        match s[*i].to_digit(base) {
            Some(d) => {
                mag = match mag.checked_mul(base as u64).and_then(|m| m.checked_add(d as u64)) {
                    Some(m) => m,
                    None => {
                        ovf = true;
                        mag
                    }
                };
                *i += 1;
            }
            None => break,
        }
    }
    (mag, ovf)
}

/// Skip an optional sign, returning the negation flag.
fn scan_sign(s: &[char], i: &mut usize, end: usize) -> bool {
    if *i < end && (s[*i] == '+' || s[*i] == '-') {
        let neg = s[*i] == '-';
        *i += 1;
        neg
    } else {
        false
    }
}

/// %d/%u: TclParseNumber DECIMAL_ONLY — decimal digits with optional
/// sign, underscore stops the scan (NO_UNDERSCORE), overflow saturates.
fn scan_decimal(s: &[char], start: usize, limit: usize) -> NumScan {
    let end = start.saturating_add(limit).min(s.len());
    let mut i = start;
    let neg = scan_sign(s, &mut i, end);
    let ds = i;
    let (mag, ovf) = acc_radix(s, &mut i, end, 10);
    if i == ds {
        return NumScan::Fail(i);
    }
    NumScan::Ok(finish_u64(mag, ovf, neg), i - start)
}

/// %x/%X: hex digits with optional sign; a 0x/0X prefix is skipped when
/// hex digits follow, otherwise just the "0" is accepted.
fn scan_x(s: &[char], start: usize, limit: usize) -> NumScan {
    let end = start.saturating_add(limit).min(s.len());
    let mut i = start;
    let neg = scan_sign(s, &mut i, end);
    if i < end && s[i] == '0' && i + 1 < end && (s[i + 1] == 'x' || s[i + 1] == 'X') {
        let mut j = i + 2;
        let ds = j;
        let (mag, ovf) = acc_radix(s, &mut j, end, 16);
        if j > ds {
            return NumScan::Ok(finish_u64(mag, ovf, neg), j - start);
        }
        // Bare or junk prefix: only the "0" was accepted.
        return NumScan::Ok(0, i + 1 - start);
    }
    let ds = i;
    let (mag, ovf) = acc_radix(s, &mut i, end, 16);
    if i == ds {
        return NumScan::Fail(i);
    }
    NumScan::Ok(finish_u64(mag, ovf, neg), i - start)
}

/// %b: binary digits with optional sign; a 0b/0B prefix is skipped when
/// binary digits follow, otherwise just the "0" is accepted.
fn scan_b(s: &[char], start: usize, limit: usize) -> NumScan {
    let end = start.saturating_add(limit).min(s.len());
    let mut i = start;
    let neg = scan_sign(s, &mut i, end);
    if i < end && s[i] == '0' && i + 1 < end && (s[i + 1] == 'b' || s[i + 1] == 'B') {
        let mut j = i + 2;
        let ds = j;
        let (mag, ovf) = acc_radix(s, &mut j, end, 2);
        if j > ds {
            return NumScan::Ok(finish_u64(mag, ovf, neg), j - start);
        }
        return NumScan::Ok(0, i + 1 - start);
    }
    let ds = i;
    let (mag, ovf) = acc_radix(s, &mut i, end, 2);
    if i == ds {
        return NumScan::Fail(i);
    }
    NumScan::Ok(finish_u64(mag, ovf, neg), i - start)
}

/// %i: TclParseNumber SCAN_PREFIXES — decimal, 0x hex, leading-0 octal.
/// A "0" is always accepted on its own (so "0b101"/"0o17"/"0x" scan as
/// 0 with consumed length 1).
fn scan_i(s: &[char], start: usize, limit: usize) -> NumScan {
    let end = start.saturating_add(limit).min(s.len());
    let mut i = start;
    let neg = scan_sign(s, &mut i, end);
    if i >= end {
        return NumScan::Fail(i);
    }
    if s[i] == '0' {
        // ZERO_X: 0x/0X hex prefix.
        if i + 1 < end && (s[i + 1] == 'x' || s[i + 1] == 'X') {
            let mut j = i + 2;
            let ds = j;
            let (mag, ovf) = acc_radix(s, &mut j, end, 16);
            if j > ds {
                return NumScan::Ok(finish_u64(mag, ovf, neg), j - start);
            }
            return NumScan::Ok(0, i + 1 - start);
        }
        // ZERO → zeroo: leading-0 octal ('0' always accepted).
        let ds = i;
        let (mag, ovf) = acc_radix(s, &mut i, end, 8);
        let _ = ds;
        return NumScan::Ok(finish_u64(mag, ovf, neg), i - start);
    }
    if s[i].is_ascii_digit() {
        let ds = i;
        let (mag, ovf) = acc_radix(s, &mut i, end, 10);
        if i == ds {
            return NumScan::Fail(i);
        }
        return NumScan::Ok(finish_u64(mag, ovf, neg), i - start);
    }
    NumScan::Fail(i)
}

/// Float scan result. Mirrors TclParseNumber's DECIMAL path plus the
/// inf/nan state machine.
enum FloatScan {
    /// Decimal number matched; value is the consumed char count.
    Digits(usize),
    /// "inf" matched (case-insensitive); consumed count includes the sign.
    Inf(usize),
    /// "nan" form matched — TclParseNumber accepts it but scan refuses the
    /// value: nothing is consumed and no conversion is counted.
    NanMatched,
    /// Parse failed; char position the parse stopped at.
    Fail(usize),
}

/// %e/%E/%f/%g/%G: Tcl decimal float with optional sign, plus the inf/nan
/// prefixes. Exponent digits are required for the exponent to count
/// ("12e" scans as 12). A lone "." fails having consumed it.
fn scan_float(s: &[char], start: usize, limit: usize) -> FloatScan {
    let end = start.saturating_add(limit).min(s.len());
    let mut i = start;
    if i < end && (s[i] == '+' || s[i] == '-') {
        i += 1;
    }
    if i < end && (s[i] == 'i' || s[i] == 'I') {
        if i + 3 <= end && (s[i + 1].to_ascii_lowercase() == 'n') && (s[i + 2].to_ascii_lowercase() == 'f')
        {
            return FloatScan::Inf(i + 3 - start);
        }
        // Partial "i"/"in": consumed prefix only.
        let matched = if i + 1 < end && s[i + 1].to_ascii_lowercase() == 'n' {
            2
        } else {
            1
        };
        return FloatScan::Fail(i + matched);
    }
    if i < end && (s[i] == 'n' || s[i] == 'N') {
        if i + 3 <= end
            && s[i + 1].to_ascii_lowercase() == 'a'
            && s[i + 2].to_ascii_lowercase() == 'n'
        {
            return FloatScan::NanMatched;
        }
        let matched = if i + 1 < end && s[i + 1].to_ascii_lowercase() == 'a' {
            2
        } else {
            1
        };
        return FloatScan::Fail(i + matched);
    }
    let mut int_digits = 0;
    while i < end && s[i].is_ascii_digit() {
        i += 1;
        int_digits += 1;
    }
    let mut frac_digits = 0;
    if i < end && s[i] == '.' {
        let mut j = i + 1;
        while j < end && s[j].is_ascii_digit() {
            j += 1;
            frac_digits += 1;
        }
        // "3." accepts just "3"; the bare '.' is only consumed when a
        // fraction follows.
        if frac_digits > 0 {
            i = j;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        if i < end && s[i] == '.' {
            return FloatScan::Fail(i + 1);
        }
        return FloatScan::Fail(i);
    }
    // Optional exponent; requires at least one digit.
    if i < end && (s[i] == 'e' || s[i] == 'E') {
        let mut j = i + 1;
        if j < end && (s[j] == '+' || s[j] == '-') {
            j += 1;
        }
        let mut exp_digits = 0;
        while j < end && s[j].is_ascii_digit() {
            j += 1;
            exp_digits += 1;
        }
        if exp_digits > 0 {
            i = j;
        }
    }
    FloatScan::Digits(i - start)
}

pub fn cmd_scan(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "scan",
            3,
            args.len(),
            "string format ?varName ...?",
        ));
    }
    let input = args[1].as_str().to_string();
    let format = args[2].as_str().to_string();
    let num_vars = args.len() - 3;
    let (_, total) = validate_scan_format(interp, &format, num_vars)?;

    let s: Vec<char> = input.chars().collect();
    let fmt: Vec<char> = format.chars().collect();
    let mut pos = 0usize; // char position in the input
    let mut objs: Vec<Option<Value>> = vec![None; total];
    let mut nconversions = 0usize;
    let mut underflow = false;
    let mut fi = 0usize; // format char index
    let mut next_seq = 0usize; // sequential result slot

    macro_rules! store {
        ($spec:expr, $val:expr) => {{
            let v: Value = $val;
            let sp: &ScanSpec = &$spec;
            let slot = sp.xpg.unwrap_or(next_seq);
            if slot < objs.len() {
                objs[slot] = Some(v);
            }
            if sp.xpg.is_none() {
                next_seq += 1;
            }
        }};
    }

    while fi < fmt.len() {
        let ch = fmt[fi];
        fi += 1;

        if is_tcl_space_ch(ch) {
            // Whitespace in the format skips whitespace in the string;
            // running out of input here is a plain stop, not underflow.
            while pos < s.len() && is_tcl_space_ch(s[pos]) {
                pos += 1;
            }
            continue;
        }
        if ch != '%' {
            // Literal: mismatch or input exhaustion stops the scan (only
            // exhaustion sets underflow).
            if pos >= s.len() {
                underflow = true;
                break;
            }
            let sch = s[pos];
            pos += 1;
            if sch != ch {
                break;
            }
            continue;
        }

        // Parse one specifier (validation already succeeded, so this
        // mirrors the main loop of Tcl_ScanObjCmd).
        let mut ch = fmt[fi];
        fi += 1;
        let mut spec = ScanSpec { op: 0, suppress: false, width: None, xpg: None };
        if ch == '%' {
            // "%%" literal
            if pos >= s.len() {
                underflow = true;
                break;
            }
            let sch = s[pos];
            pos += 1;
            if sch != '%' {
                break;
            }
            continue;
        }
        if ch == '*' {
            spec.suppress = true;
            if fi < fmt.len() {
                ch = fmt[fi];
                fi += 1;
            }
        } else if ch.is_ascii_digit() {
            let mut j = fi - 1;
            let mut val = 0usize;
            while j < fmt.len() && fmt[j].is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add(fmt[j] as usize - '0' as usize);
                j += 1;
            }
            if j < fmt.len() && fmt[j] == '$' {
                spec.xpg = Some(val - 1);
                fi = j + 1;
            } else {
                // The digits were a width after all.
                spec.width = Some(val);
                fi = j;
            }
            if fi < fmt.len() {
                ch = fmt[fi];
                fi += 1;
            }
        }
        if spec.width.is_none() && ch.is_ascii_digit() {
            let mut j = fi - 1;
            let mut val = 0usize;
            while j < fmt.len() && fmt[j].is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add(fmt[j] as usize - '0' as usize);
                j += 1;
            }
            spec.width = Some(val);
            fi = j;
            if fi < fmt.len() {
                ch = fmt[fi];
                fi += 1;
            }
        }
        // Skip size modifiers (validated already).
        if matches!(ch, 'L' | 'l' | 'j' | 'q' | 'h') {
            if ch == 'l' && fi < fmt.len() && fmt[fi] == 'l' {
                fi += 1;
            }
            if fi < fmt.len() {
                ch = fmt[fi];
                fi += 1;
            }
        }
        let op = ch as u8;

        if op == b'n' {
            if !spec.suppress {
                store!(spec, Value::from_int(pos as i64));
            }
            nconversions += 1;
            continue;
        }

        // All other conversions need input.
        if pos >= s.len() {
            underflow = true;
            break;
        }
        if !matches!(op, b'c' | b'[') {
            while pos < s.len() && is_tcl_space_ch(s[pos]) {
                pos += 1;
            }
            if pos >= s.len() {
                underflow = true;
                break;
            }
        }
        // An explicit width of 0 means unlimited.
        let width = match spec.width {
            Some(0) | None => usize::MAX,
            Some(w) => w,
        };

        match op {
            b's' => {
                let start = pos;
                let mut w = width;
                while pos < s.len() && w > 0 {
                    if is_tcl_space_ch(s[pos]) {
                        break;
                    }
                    pos += 1;
                    w -= 1;
                }
                if !spec.suppress {
                    let text: String = s[start..pos].iter().collect();
                    store!(spec, Value::from_str(&text));
                }
            }
            b'[' => {
                let (members, ranges, exclude, next_fi) = build_char_set(&fmt, fi);
                fi = next_fi;
                let start = pos;
                let mut w = width;
                while pos < s.len() && w > 0 {
                    if !char_in_set(&members, &ranges, exclude, s[pos]) {
                        break;
                    }
                    pos += 1;
                    w -= 1;
                }
                if start == pos {
                    // Nothing matched the set: stop processing.
                    break;
                }
                if !spec.suppress {
                    let text: String = s[start..pos].iter().collect();
                    store!(spec, Value::from_str(&text));
                }
            }
            b'c' => {
                if !spec.suppress {
                    store!(spec, Value::from_int(s[pos] as u32 as i64));
                }
                pos += 1;
            }
            b'd' | b'u' | b'i' | b'o' | b'x' | b'X' | b'b' => {
                let limit = width.min(s.len().saturating_sub(pos));
                let r = match op {
                    b'd' | b'u' => scan_decimal(&s, pos, limit),
                    b'i' => scan_i(&s, pos, limit),
                    b'o' => acc_signed(&s, pos, limit, 8),
                    b'x' | b'X' => scan_x(&s, pos, limit),
                    b'b' => scan_b(&s, pos, limit),
                    _ => unreachable!(),
                };
                match r {
                    NumScan::Ok(v, n) => {
                        pos += n;
                        if !spec.suppress {
                            if op == b'u' {
                                // %u reports the u64 reinterpretation
                                // (tclsh stores a bignum; the decimal
                                // string matches byte-for-byte).
                                let u = v as u64;
                                store!(spec, Value::from_str(&u.to_string()));
                            } else {
                                store!(spec, Value::from_int(v));
                            }
                        }
                    }
                    NumScan::Fail(stop) => {
                        // TclParseNumber failed: underflow only when the
                        // parse stopped at end-of-input (unlimited width)
                        // or exactly at the width boundary.
                        if width == usize::MAX {
                            if stop == s.len() {
                                underflow = true;
                            }
                        } else if stop == pos + width {
                            underflow = true;
                        }
                        break;
                    }
                }
            }
            b'f' | b'e' | b'E' | b'g' | b'G' => {
                match scan_float(&s, pos, width) {
                    FloatScan::Inf(n) => {
                        let neg = s[pos] == '-';
                        pos += n;
                        if !spec.suppress {
                            let d = if neg { f64::NEG_INFINITY } else { f64::INFINITY };
                            store!(spec, crate::types::expr_funcs::float_value(d));
                        }
                    }
                    FloatScan::NanMatched => {
                        // Accepted by the number parser, refused by scan:
                        // nothing consumed, nothing counted.
                        break;
                    }
                    FloatScan::Digits(n) => {
                        let text: String = s[pos..pos + n].iter().collect();
                        pos += n;
                        if !spec.suppress {
                            let d: f64 = text.parse().unwrap_or(0.0);
                            store!(spec, crate::types::expr_funcs::float_value(d));
                        }
                    }
                    FloatScan::Fail(stop) => {
                        if width == usize::MAX {
                            if stop == s.len() {
                                underflow = true;
                            }
                        } else if stop == pos + width {
                            underflow = true;
                        }
                        break;
                    }
                }
            }
            _ => unreachable!("validated"),
        }
        nconversions += 1;
    }

    if num_vars > 0 {
        let mut result = 0i64;
        for (slot, var) in args[3..].iter().enumerate() {
            if let Some(v) = objs[slot].take() {
                result += 1;
                interp.set_var(var.as_str(), v)?;
            }
        }
        if underflow && nconversions == 0 {
            Ok(Value::from_int(-1))
        } else {
            Ok(Value::from_int(result))
        }
    } else {
        if underflow && nconversions == 0 {
            return Ok(Value::from_list(&[]));
        }
        let list: Vec<Value> = objs
            .into_iter()
            .map(|o| o.unwrap_or_default())
            .collect();
        Ok(Value::from_list(&list))
    }
}

/// %o helper: plain base-8 digits with optional sign (no prefixes at all).
fn acc_signed(s: &[char], start: usize, limit: usize, base: u32) -> NumScan {
    let end = start.saturating_add(limit).min(s.len());
    let mut i = start;
    let neg = scan_sign(s, &mut i, end);
    let ds = i;
    let (mag, ovf) = acc_radix(s, &mut i, end, base);
    if i == ds {
        return NumScan::Fail(i);
    }
    NumScan::Ok(finish_u64(mag, ovf, neg), i - start)
}

#[cfg(test)]
#[path = "misc_tests.rs"]
mod tests;
