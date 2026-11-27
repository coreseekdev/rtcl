//! Miscellaneous commands: set, expr, incr, unset, info, subst, append, disassemble.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;
use rtcl_parser::Compiler;

use super::list::{set_error_code, tcl_err, tcl_get_int};

/// Get the hostname (cross-platform via environment variables).
#[cfg(feature = "std")]
fn hostname_get() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string())
}

/// Resolve a user-typed proc name to its registry key, mirroring the
/// dispatch lookup (as-typed, current namespace, `::`-qualify, global
/// fallback, `::`-bare).  `info body`/`args`/`statics` accept relative
/// names like `test_ns_simple::test` (namespace-old-2.9).
pub(crate) fn resolve_proc_key(interp: &Interp, name: &str) -> Option<String> {
    let has = |i: &Interp, k: &str| i.procs.contains_key(k);
    if has(interp, name) {
        return Some(name.to_string());
    }
    if interp.current_namespace.as_ref() != "::" && !name.starts_with("::") {
        let qualified = super::namespace::qualify(&interp.current_namespace, name);
        if has(interp, &qualified) {
            return Some(qualified);
        }
    }
    if !name.starts_with("::") && name.contains("::") {
        let qualified = format!("::{}", name);
        if has(interp, &qualified) {
            return Some(qualified);
        }
    }
    if name.contains("::") {
        let norm = super::namespace::normalise(name);
        if norm != name && has(interp, &norm) {
            return Some(norm);
        }
    }
    if !name.starts_with("::") {
        let qualified = format!("::{}", name);
        if has(interp, &qualified) {
            return Some(qualified);
        }
    }
    if name.starts_with("::") && !name[2..].contains("::") && has(interp, &name[2..]) {
        return Some(name[2..].to_string());
    }
    None
}

// ---------- Arithmetic operator commands: +, -, *, / ----------

/// A fold in progress: the i64 fast path, the exact (bignum) accumulator
/// an overflowing step switches to, or the float path.
enum Acc {
    Int(i64),
    Wide(Value),
    Float(f64),
}

impl Acc {
    fn value(self) -> Value {
        match self {
            Acc::Int(i) => Value::from_int(i),
            Acc::Wide(v) => v,
            Acc::Float(f) => Value::from_float(f),
        }
    }
}

fn not_a_number(arg: &Value) -> Error {
    Error::runtime(
        format!("expected number but got \"{}\"", arg.as_str()),
        crate::error::ErrorCode::Generic,
    )
}

fn zero_divisor() -> Error {
    Error::runtime("Division by zero", crate::error::ErrorCode::Generic)
}

fn is_zero_number(arg: &Value) -> bool {
    arg.as_float() == Some(0.0)
}

/// One fold step `acc op arg`. The i64 step is checked: an overflow moves
/// to the exact accumulator instead of wrapping (`+ 9223372036854775807 1`
/// used to give INT64_MIN) or panicking (`/ INT64_MIN -1` is a divide
/// overflow in Rust). Division keeps jimtcl's truncating integer form.
fn fold_step(acc: Acc, arg: &Value, op: char) -> Result<Acc> {
    Ok(match acc {
        Acc::Float(a) => {
            let b = arg.as_float().ok_or_else(|| not_a_number(arg))?;
            Acc::Float(match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                _ => {
                    if b == 0.0 {
                        return Err(zero_divisor());
                    }
                    a / b
                }
            })
        }
        Acc::Int(a) => {
            if let Some(b) = arg.as_int() {
                if op == '/' && b == 0 {
                    return Err(zero_divisor());
                }
                let fast = match op {
                    '+' => a.checked_add(b),
                    '-' => a.checked_sub(b),
                    '*' => a.checked_mul(b),
                    _ => a.checked_div(b),
                };
                match fast {
                    Some(v) => Acc::Int(v),
                    None => Acc::Wide(crate::types::expr_ops::numeric_binop(
                        &Value::from_int(a),
                        &Value::from_int(b),
                        op,
                    )?),
                }
            } else if let Some(b) = arg.as_float() {
                Acc::Float(match op {
                    '+' => a as f64 + b,
                    '-' => a as f64 - b,
                    '*' => a as f64 * b,
                    _ => {
                        if b == 0.0 {
                            return Err(zero_divisor());
                        }
                        a as f64 / b
                    }
                })
            } else {
                return Err(not_a_number(arg));
            }
        }
        Acc::Wide(w) => {
            if op == '/' && is_zero_number(arg) {
                return Err(zero_divisor());
            }
            let next = crate::types::expr_ops::numeric_binop(&w, arg, op).map_err(|_| not_a_number(arg))?;
            // An integer result (i64 or bignum) stays exact; a float one
            // moves the fold to the float path.
            if crate::types::bignum::int_rep(&next).is_some() {
                Acc::Wide(next)
            } else {
                Acc::Float(next.as_float().unwrap_or(0.0))
            }
        }
    })
}

fn fold(init: Acc, rest: &[Value], op: char) -> Result<Value> {
    let mut acc = init;
    for arg in rest {
        acc = fold_step(acc, arg, op)?;
    }
    Ok(acc.value())
}

/// `+ ?number ...?` — Sum all arguments (0 if none).
pub fn cmd_add(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    fold(Acc::Int(0), &args[1..], '+')
}

/// `* ?number ...?` — Multiply all arguments (1 if none).
pub fn cmd_mul(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    fold(Acc::Int(1), &args[1..], '*')
}

/// `- number ?number ...?` — Unary negation (1 arg) or subtract remaining from first.
pub fn cmd_sub(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("-", 2, args.len(), "number ?number ...?"));
    }
    if args.len() == 2 {
        // Unary negation is 0 - x, exact for INT64_MIN too.
        return fold(Acc::Int(0), &args[1..], '-');
    }
    // a - b - c folds from the first operand (not from 0 - a).
    let first = match (args[1].as_int(), args[1].as_float()) {
        (Some(i), _) => Acc::Int(i),
        (None, Some(f)) => Acc::Float(f),
        (None, None) => return Err(not_a_number(&args[1])),
    };
    fold(first, &args[2..], '-')
}

/// `/ number ?number ...?` — Reciprocal (1 arg) or divide first by remaining.
pub fn cmd_div(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage("/", 2, args.len(), "number ?number ...?"));
    }

    if args.len() == 2 {
        // Reciprocal: 1/x is real division, also for integers.
        let f = args[1].as_float().ok_or_else(|| not_a_number(&args[1]))?;
        if f == 0.0 {
            return Err(zero_divisor());
        }
        return Ok(Value::from_float(1.0 / f));
    }

    let first = match (args[1].as_int(), args[1].as_float()) {
        (Some(i), _) => Acc::Int(i),
        (None, Some(f)) => Acc::Float(f),
        (None, None) => return Err(not_a_number(&args[1])),
    };
    fold(first, &args[2..], '/')
}

// ---------- env ----------

/// `env ?varName? ?default?` — Read environment variables.
#[cfg(feature = "env")]
pub fn cmd_env(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    match args.len() {
        1 => {
            // Return flat list of all env vars: key val key val ...
            let mut list = Vec::new();
            // `vars_os` / `var_os` + lossy: `std::env::vars` and
            // `std::env::var` panic on a value that is not valid Unicode.
            for (k, v) in std::env::vars_os() {
                list.push(Value::from_str(&k.to_string_lossy()));
                list.push(Value::from_str(&v.to_string_lossy()));
            }
            Ok(Value::from_list(&list))
        }
        2 => {
            let key = args[1].as_str();
            match std::env::var_os(key).map(|v| v.to_string_lossy().into_owned()).ok_or(()) {
                Ok(val) => Ok(Value::from_str(&val)),
                Err(_) => Err(Error::runtime(
                    format!("environment variable \"{}\" does not exist", key),
                    crate::error::ErrorCode::NotFound,
                )),
            }
        }
        3 => {
            let key = args[1].as_str();
            match std::env::var_os(key).map(|v| v.to_string_lossy().into_owned()).ok_or(()) {
                Ok(val) => Ok(Value::from_str(&val)),
                Err(_) => Ok(args[2].clone()), // default
            }
        }
        _ => Err(Error::wrong_args_with_usage("env", 1, args.len(), "?varName? ?default?")),
    }
}

// ---------- rand ----------

/// `rand ?min? ?max?` — Generate random integer.
/// `::tcl::mathfunc::<fn>` — expr 数学函数的真命令形态（9.1 的
/// BuiltinFuncTable 双注册面；`info commands tcl::mathfunc::*` 列出的
/// 就是这些名字）。函数体与 expr 共用
/// [`crate::types::expr_funcs::call_math_func`]，rand 的种子语义也和
/// expr 求值路径一致（call_depth）。
pub fn cmd_mathfunc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let fname = args[0].as_str().rsplit("::").next().unwrap_or("");
    let seed = interp.call_depth;
    crate::types::expr_funcs::call_math_func(fname, args[1..].to_vec(), seed)
}

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

/// const varName value — tclsh 9.1 (tclVar.c Tcl_ConstObjCmd): create
/// the variable and flag it constant; writes, unsets and incr error.
/// Re-const on an existing constant is a silent no-op; on an existing
/// non-constant it errors (TCL LOOKUP CONST).
pub fn cmd_const(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(Error::wrong_args_with_usage("const", 3, args.len(), "varName value"));
    }
    let name = args[1].as_str();
    if name.contains('(') {
        set_error_code(interp, "TCL LOOKUP CONST");
        return Err(tcl_err(format!(
            "can't make constant \"{}\": name refers to an element in an array",
            name
        )));
    }
    // An existing array cannot become a constant.
    if interp.is_array_semantic(name) {
        set_error_code(interp, "TCL LOOKUP CONST");
        return Err(tcl_err(format!(
            "can't make constant \"{}\": variable is array",
            name
        )));
    }
    if interp.var_exists_quiet(name) {
        if interp.is_const_marked(name) {
            return Ok(Value::empty());
        }
        set_error_code(interp, "TCL LOOKUP CONST");
        return Err(tcl_err(format!(
            "can't make constant \"{}\": variable already exists",
            name
        )));
    }
    // Create through the ordinary set (fires write traces, tclsh pins
    // the creation-set as a write), then flag constant in the owning
    // scope: frame-scoped for proc locals, the global set otherwise.
    // A ::-qualified name in a missing parent namespace errors with the
    // const-command path text (var-25.6: can't const "::nosuchns::x":
    // parent namespace doesn't exist) — set_var's own guard produces the
    // same message with a can't set head, so reword it.
    if let Err(e) = interp.set_var(name, args[2].clone()) {
        let msg = e.to_string();
        if msg.contains("parent namespace doesn't exist") {
            set_error_code(interp, "TCL LOOKUP VARNAME");
            return Err(tcl_err(format!(
                "can't const \"{}\": parent namespace doesn't exist",
                name
            )));
        }
        return Err(e);
    }
    if interp.frames.is_empty() {
        interp.const_globals.push(name.to_string());
    } else {
        if let Some(frame) = interp.frames.last_mut() {
            frame.consts.get_or_insert_with(Vec::new).push(name.to_string());
        }
    }
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
        "can't use non-numeric string as operand of \"+\"".to_string(),
        crate::error::ErrorCode::InvalidOp,
    ))
}

/// `incr` 的整数解析（D5，9.1 口径）：任意精度整数（含 bignum 字符串与
/// `Big` rep）都收；浮点形值（double 能解析、整数不能）与全非数字都报
/// `expected integer but got "<值>"`，errorCode 按 9.1 分工——前者
/// `TCL VALUE INTEGER`，后者 `TCL VALUE NUMBER`（incr_widen 钉形）。
fn incr_int_of(interp: &mut Interp, v: &Value) -> std::result::Result<num_bigint::BigInt, Error> {
    match crate::types::bignum::int_rep(v) {
        Some(crate::types::bignum::IntRep::I64(i)) => Ok(num_bigint::BigInt::from(i)),
        Some(crate::types::bignum::IntRep::Big(b)) => Ok(b),
        None => {
            if v.as_float().is_some() {
                set_error_code(interp, "TCL VALUE INTEGER");
            } else {
                set_error_code(interp, "TCL VALUE NUMBER");
            }
            // 9.1 报错尾注（tclStrToD.c formaterr）：值是列表样串
            // （≥2 词）→ but got a list（incr-2.32/2.33、incr-old-2.10）。
            Err(tcl_err(crate::value::value_report("integer", v.as_str())))
        }
    }
}

pub fn cmd_incr(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // const 守卫（tclVar.c 2310：incr 路径的 ISCONST 错形）。
    if args.len() >= 2 {
        if let Some(e) = interp.const_incr_err(args[1].as_str()) {
            return Err(e);
        }
    }
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::wrong_args("incr", 2, args.len()));
    }
    crate::interp::dbgtrace::event("cmd_incr", args[1].as_str());
    let var_name = args[1].as_str();
    // tclsh reads the variable first: a bad value there wins over a bad
    // increment (`incr x 1a` with x="1a" carries no increment frame).
    // 读取走 read_var(Tcl_GetVarFromObj 的 read trace 触发面:
    // incr 读现值 fire read trace——incr_window_p11_trace 钉形)。
    let current = match interp.read_var(var_name) {
        Ok(v) => incr_int_of(interp, &v)?,
        Err(e) => {
            // Tcl: incr on a non-existent variable starts at 0。类型冲突
            // 的方向不同：标量目标实际是数组 → 写路径错误
            // `can't set ... variable is array`（TCL WRITE VARNAME）；
            // 数组元素目标而基名是标量 → 读错误（TCL LOOKUP VARNAME）。
            if interp.var_exists(var_name) {
                if e.message_text().contains("variable is array") {
                    set_error_code(interp, "TCL WRITE VARNAME");
                    return Err(Error::Msg(format!(
                        "can't set \"{}\": variable is array",
                        var_name
                    )));
                }
                return Err(e);
            }
            // Element-of-scalar also conflicts even though `info exists`
            // reports 0 for the element itself.
            if crate::interp::vars::is_type_conflict(&e) {
                return Err(e);
            }
            num_bigint::BigInt::from(0)
        }
    };
    let amount = if args.len() == 3 {
        match incr_int_of(interp, &args[2]) {
            Ok(n) => n,
            Err(e) => {
                // tclsh frames the failed increment parse between the
                // message and the invoking command's harness frame —
                // no line number (`incr-old-2.5`).  帧只进 errorInfo，
                // catch 面的消息不带（incr_window_p07 钉形）。
                let msg = e.message_text().to_string();
                interp.err_info = Some(msg.clone());
                if let Some(info) = &mut interp.err_info {
                    info.push_str("\n    (reading increment)");
                }
                return Err(tcl_err(msg));
            }
        }
    } else {
        num_bigint::BigInt::from(1)
    };
    // tclsh widens an overflowing i64 sum to an exact bignum and accepts
    // bignum current/increment at any width (incr on 2**63 grows, it does
    // not wrap; 9.1 重钉).  expr's `+` already goes through numeric_binop's
    // BigInt path — same convention here.
    interp.set_var(var_name, Value::from_bigint(current + amount))
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
        if nocomplain {
            // A swallowed "no such variable" must not leave the lookup
            // errorCode behind (tclsh: `unset -nocomplain missing` does not
            // touch ::errorCode).
            // D1 步骤 1：读经 arena、删经 global_remove（双删）。
            let saved_code = interp.global_get("errorCode").cloned();
            let saved_raised = interp.err_code_raised;
            if interp.unset_var(name).is_err() {
                match saved_code {
                    Some(v) => {
                        interp.global_insert("errorCode".to_string(), v);
                    }
                    None => {
                        interp.global_remove("errorCode");
                    }
                }
                interp.err_code_raised = saved_raised;
            }
        } else {
            interp.unset_var(name)?;
        }
    }
    Ok(Value::empty())
}

/// A command key's namespace and simple name: keys split at their last
/// `::` (`::e1::c1` → `::e1`, `c1`; `tcl::mathop::+` → `::tcl::mathop`,
/// `+`), unqualified keys live in `::`.
fn split_key_ns(key: &str) -> (String, String) {
    match key.strip_prefix("::") {
        Some(rest) => match rest.rfind("::") {
            None => ("::".to_string(), rest.to_string()),
            Some(i) => (format!("::{}", &rest[..i]), rest[i + 2..].to_string()),
        },
        None => match key.rfind("::") {
            Some(i) => (format!("::{}", &key[..i]), key[i + 2..].to_string()),
            None => ("::".to_string(), key.to_string()),
        },
    }
}

/// The full (leading-`::`) form of a command key, as tclsh's
/// `Tcl_GetCommandFullName` renders it when the pattern names a namespace.
fn full_key(key: &str) -> String {
    if key.starts_with("::") {
        key.to_string()
    } else {
        format!("::{}", key)
    }
}

/// Resolve an `info commands` pattern the way tclsh's
/// `TclGetNamespaceForQualName` does: walk each `::`-terminated qualifier as
/// a child of the current namespace (absolute patterns start at `::`); the
/// remainder after the last separator is the simple pattern. Returns `None`
/// when a qualifier namespace doesn't exist — tclsh then lists nothing.
/// `(effective_ns, simple_pattern, specific_ns_in_pattern)`.
fn resolve_info_pattern(interp: &Interp, pat: &str) -> Option<(String, Option<String>, bool)> {
    let mut ns = if pat.starts_with("::") {
        "::".to_string()
    } else {
        interp.current_namespace.to_string()
    };
    let mut rest = pat.trim_start_matches(':');
    loop {
        if rest.is_empty() {
            // Pattern ended with a separator (or was empty): the simple
            // name is the empty string.
            return Some((ns, Some(String::new()), pat.contains("::")));
        }
        match rest.find("::") {
            Some(i) => {
                let comp = &rest[..i];
                let child = if ns == "::" {
                    format!("::{}", comp)
                } else {
                    format!("{}::{}", ns, comp)
                };
                let child = super::namespace::normalise(&child);
                if interp.namespaces.contains_key(&child) {
                    ns = child;
                } else {
                    return None;
                }
                rest = rest[i + 2..].trim_start_matches(':');
            }
            None => {
                return Some((
                    ns,
                    Some(rest.to_string()),
                    pat.contains("::"),
                ));
            }
        }
    }
}

/// `info commands ?pattern?` — a faithful port of tclsh's
/// `InfoCommandsCmd`: list the effective namespace's own commands (matching
/// the simple pattern), rendered fully-qualified when the pattern contains
/// `::` and unqualified otherwise; non-global effective namespaces also
/// merge in non-hidden global commands.
fn list_commands(interp: &Interp, pattern: Option<&str>) -> Result<Value> {
    list_commands_in(interp, pattern, true, true)
}

/// Core of `info commands`/`info procs` listing; `builtins` controls whether
/// the builtin command table participates, `merge_globals` whether a
/// non-global effective namespace also lists unhidden global commands.
fn list_commands_in(
    interp: &Interp,
    pattern: Option<&str>,
    builtins: bool,
    merge_globals: bool,
) -> Result<Value> {
    let (eff, simple, specific) = match pattern {
        None => (interp.current_namespace.to_string(), None, false),
        Some(p) => match resolve_info_pattern(interp, p) {
            None => return Ok(Value::from_list(&[])),
            Some(pi) => pi,
        },
    };

    // Walk the Tcl-order log: keys currently live in any command table,
    // visible (not hidden).  Dedup to the LAST log occurrence (recreates),
    // replay the per-namespace subsequence through the Tcl hash-table
    // mechanics (tclsh's iteration order), then filter by glob.
    let live = |key: &str| -> bool {
        let hit = |k: &str| {
            interp.procs.contains_key(k)
                || interp.import_aliases.contains_key(k)
                || interp.ensembles.contains_key(k)
                || (builtins && interp.commands.contains_key(k))
        };
        hit(key)
            || (key.starts_with("::") && hit(&key[2..]))
            || hit(&format!("::{}", key))
    };
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, key) in interp.command_order.iter().enumerate() {
        if live(key) && !interp.is_command_hidden(key) {
            seen.insert(key.as_str(), i);
        }
    }
    // Collect (full key, table tail) pairs per target table — tclsh's
    // namespace tables hash the command TAIL, so the replay runs on tails.
    let mut eff_keys: Vec<(String, String)> = Vec::new();
    let mut glob_keys: Vec<(String, String)> = Vec::new();
    for (i, key) in interp.command_order.iter().enumerate() {
        if seen.get(key.as_str()) != Some(&i) {
            continue;
        }
        let (ns, tail) = split_key_ns(key);
        if ns == eff {
            eff_keys.push((key.clone(), tail));
        } else if merge_globals && ns == "::" {
            glob_keys.push((key.clone(), tail));
        }
    }
    let mut out: Vec<Value> = Vec::new();
    let mut listed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push_matched = |keys: &[(String, String)], out: &mut Vec<Value>,
                            listed: &mut std::collections::HashSet<String>,
                            simple: Option<&String>, specific: bool| {
        // The fully-qualified key each tail came from — the first one, as
        // a search of `keys` found it, but found once.
        let mut first_of: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::with_capacity(keys.len());
        for (full, tail) in keys {
            first_of.entry(tail.as_str()).or_insert_with(|| full.as_str());
        }
        for tail in crate::interp::tclhash::tcl_order(keys.iter().map(|(_, t)| t.as_str())) {
            if let Some(sp) = simple {
                if !super::super::glob_match(sp, tail) {
                    continue;
                }
            }
            if listed.insert(tail.to_string()) {
                // `::`-bearing patterns render the fully-qualified key
                // (tclsh's InfoCommandsCmd); bare patterns the tail.
                let rendered = if specific {
                    match first_of.get(tail).copied() {
                        Some(full) if full.starts_with("::") => full.to_string(),
                        Some(full) => format!("::{}", full),
                        None => tail.to_string(),
                    }
                } else {
                    tail.to_string()
                };
                out.push(Value::from_str(&rendered));
            }
        }
    };
    push_matched(&eff_keys, &mut out, &mut listed, simple.as_ref(), specific);
    // Non-global effective namespace, unqualified pattern: merge in global
    // commands that aren't hidden by a local one (global table's own
    // replay order).
    if merge_globals && eff != "::" && !specific {
        push_matched(&glob_keys, &mut out, &mut listed, simple.as_ref(), false);
    }
    Ok(Value::from_list(&out))
}

/// tclsh's `info` subcommand list, in the order it prints on an
/// unknown-or-ambiguous error.
/// The expr math functions, in tclsh 8.6.17's `Tcl_ListMathFuncs` table
/// order (deterministic per build) — `info functions` lists them.
/// C99 全集（TIP 745/521）入列，顺序对 tclsh 9.1.0 的表序实测钉形
/// （info4 的 `info functions` 逐字节一致）。
const EXPR_MATH_FUNCTIONS: &[&str] = &[
    "signbit", "wide", "isinf", "sin", "dim", "log10", "log1p", "bool", "atan", "hypot",
    "isnormal", "tanh", "tan", "int", "atanh", "ldexp", "pow", "round", "copysign",
    "rand", "atan2", "abs", "trunc", "min", "erf", "exp", "max", "nextafter", "cbrt",
    "acos", "entier", "issubnormal", "srand", "erfc", "isnan", "floor", "logb", "cos",
    "sqrt", "double", "expm1", "gamma", "isfinite", "exp2", "sinh", "log", "asinh",
    "asin", "isqrt", "fma", "isunordered", "log2", "ceil", "cosh", "acosh", "fmod",
    "lgamma", "remainder",
];

/// tclsh 9.1.0's `info` subcommand list (unknown-or-ambiguous error
/// order; `cmdtype`/`constant`/`consts` are 9.x additions).
const INFO_CANONICAL_SUBCMDS: &[&str] = &[
    "args", "body", "class", "cmdcount", "cmdtype", "commands", "complete", "constant",
    "consts", "coroutine", "default", "errorstack", "exists", "frame", "functions",
    "globals", "hostname", "level", "library", "loaded", "locals", "nameofexecutable",
    "object", "patchlevel", "procs", "script", "sharedlibextension", "tclversion", "vars",
];

/// Resolve an `info` subcommand word: exact matches win (including the
/// jimtflavored extras rtcl supports), otherwise a unique prefix of
/// tclsh's canonical set resolves; anything else is tclsh's
/// unknown-or-ambiguous error.
fn resolve_info_subcmd(sub_raw: &str) -> Result<String> {
    const EXACT_KNOWN: &[&str] = &[
        "args", "body", "commands", "complete", "exists", "globals", "hostname", "level",
        "locals", "nameofexecutable", "patchlevel", "library", "procs", "script", "vars", "alias",
        "aliases", "channels", "version", "help", "returncodes", "usage", "frame",
        "stacktrace", "references", "tainted", "statics", "source",
        "constant", "consts", "cmdtype",
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
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info commands",
                    1,
                    args.len(),
                    "?pattern?",
                ));
            }
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            list_commands(interp, pattern)
        }
        "procs" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info procs",
                    1,
                    args.len(),
                    "?pattern?",
                ));
            }
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            // Import aliases count as procs (tclsh's TclGetOriginalCommand
            // check); unlike `info commands`, `info procs` never merges in
            // the global namespace's procs.
            list_commands_in(interp, pattern, false, false)
        }
        "exists" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "info exists",
                    2,
                    args.len(),
                    "varName",
                ));
            }
            let name = args[2].as_str().to_string();
            Ok(Value::from_bool(interp.exists_firing(&name)))
        }
        "vars" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info vars",
                    1,
                    args.len(),
                    "?pattern?",
                ));
            }
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            // A leading "::" in the pattern is scope qualification, not part
            // of the stored (canonical) key — but it does ask for qualified
            // results (tclsh: `info vars ::err*` → `::errorCode ...`).
            let (match_pat, qualified) = match pattern {
                Some(p) => (Some(p.strip_prefix("::").unwrap_or(p)), p.starts_with("::")),
                None => (None, false),
            };
            // Pick the TABLE the query addresses, collect its live
            // entries as (stored key, table tail), and replay the tails
            // through the Tcl hash mechanics — tclsh's per-namespace var
            // tables hash the short name, so the tail is the hash input.
            //   - `::`-qualified pattern → the named namespace's table
            //     (frame locals never carry a qualified name);
            //   - inside a proc → the frame's table;
            //   - inside `namespace eval nn` → ::nn's table;
            //   - bare at global → ::'s table.
            // Inside a `namespace eval` body there is no proc variable
            // frame, even when a proc called it: the body's tables are
            // listed (see `var_frame_idx`).
            let in_ns = interp.var_frame_idx().is_none()
                && interp.current_namespace.as_ref() != "::";
            let ns_prefix = if in_ns {
                Some(format!("{}::", &interp.current_namespace[2..]))
            } else {
                None
            };
            let mut entries: Vec<(String, String)> = Vec::new();
            let mut table_ns: Option<String> = None;
            // 非全局 ns 语境的全局 fallback 分界（tclsh InfoVarsCmd：
            // 可见集 = 当前 ns 表 + :: 表，IVP/IVQ 钉形——前段当前表
            // hash 序、后段全局表 hash 序分段拼接）。
            let mut global_split: Option<usize> = None;
            // A relative pattern carrying `::` (n::* at global) names a
            // namespace's table just like a qualified one; results
            // render qualified (tclsh: `info vars n::*` → `::n::v`).
            let qualified = qualified
                || match_pat.map(|p| p.contains("::")).unwrap_or(false);
            // The :: table's keys in hash order (live, each once at its
            // last occurrence — `global_var_order`).
            if qualified {
                // Walk the pattern's `::` qualifiers to the target table.
                let stripped = pattern.unwrap().trim_start_matches(':').to_string();
                let target_ns = match stripped.rfind("::") {
                    Some(i) => format!("::{}", &stripped[..i]),
                    None => "::".to_string(),
                };
                // No `namespaces.contains_key` gate: unset traces can
                // run while the namespace itself is already deleted
                // (trace-18.4) — the var table still answers.
                {
                    let prefix = if target_ns == "::" {
                        String::new()
                    } else {
                        format!("{}::", &target_ns[2..])
                    };
                    for k in interp.global_var_order() {
                        let tail = match k.strip_prefix(prefix.as_str()) {
                            Some(t) if !t.contains("::") => t.to_string(),
                            _ => continue,
                        };
                        entries.push((k.to_string(), tail));
                    }
                    table_ns = Some(target_ns);
                }
            } else if interp.var_frame().is_some() {
                // Compiled locals first (slot-table order, parameters
                // first), then the frame's dynamic table in Tcl hash
                // order; `global`/`upvar`/`variable` links are frame
                // hash entries and list among the dynamic names (tclsh
                // AppendLocals includeLinks=true).  Read straight off
                // the slots — no degrade.
                for name in interp.list_frame_locals(true) {
                    entries.push((name.clone(), name));
                }
            } else if let Some(pfx) = &ns_prefix {
                // Namespace scope: that namespace's own table, then the
                // global :: table（tclsh InfoVarsCmd 的可见集合并——
                // IVP 钉形 `argx a g1` 仅 proc 局部、IVQ 钉形 ns 体 =
                // ns 表 + 全部全局裸名）。plain globals 列裸 tail。
                for k in interp.global_var_order() {
                    let tail = match k.strip_prefix(pfx.as_str()) {
                        Some(t) if !t.contains("::") => t.to_string(),
                        _ => continue,
                    };
                    entries.push((k.to_string(), tail));
                }
                global_split = Some(entries.len());
                for k in interp.global_var_order() {
                    if !k.contains("::") {
                        entries.push((k.to_string(), k.to_string()));
                    }
                }
            } else {
                for k in interp.global_var_order() {
                    if !k.contains("::") {
                        entries.push((k.to_string(), k.to_string()));
                    }
                }
            }
            // Frame tables iterate in insertion order (compiled locals
            // array); global/ns tables replay the hash mechanics.
            // ns 语境的全局 fallback 分段重放：当前表 hash 序在前、
            // :: 表 hash 序在后（tclsh InfoVarsCmd 的双表遍历序——
            // IVQ 钉形 A: `bb cc aa` + 全局段）。
            let ordered_tails: Vec<&str> = if interp.var_frame().is_some() && !qualified {
                entries.iter().map(|(_, t)| t.as_str()).collect()
            } else if let Some(split) = global_split {
                let mut o = super::super::tclhash::tcl_order(
                    entries[..split].iter().map(|(_, t)| t.as_str()),
                );
                o.extend(super::super::tclhash::tcl_order(
                    entries[split..].iter().map(|(_, t)| t.as_str()),
                ));
                o
            } else {
                super::super::tclhash::tcl_order(entries.iter().map(|(_, t)| t.as_str()))
            };
            // The stored name each listed tail came from (the first entry
            // with that tail, as a search of `entries` found it — for
            // every tail, which made `info vars` quadratic).
            let mut stored_of: std::collections::HashMap<&str, &str> =
                std::collections::HashMap::with_capacity(entries.len());
            for (stored, tail) in &entries {
                stored_of.entry(tail.as_str()).or_insert_with(|| stored.as_str());
            }
            let mut vars: Vec<Value> = Vec::new();
            let mut listed: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for tail in ordered_tails {
                let name = stored_of.get(tail).copied().unwrap_or(tail);
                let rel: &str = match &ns_prefix {
                    Some(pfx) => match name.strip_prefix(pfx.as_str()) {
                        Some(rest) if !rest.contains("::") => rest,
                        None if !name.contains("::") => name,
                        _ => continue,
                    },
                    None => tail,
                };
                if let Some(p) = match_pat {
                    let hit = super::super::glob_match(p, rel)
                        || (p.contains("::")
                            && (super::super::glob_match(p, name)
                                || super::super::glob_match(p, &format!("::{}", name))));
                    if !hit {
                        continue;
                    }
                }
                // Bare `info vars` at the global level lists only true
                // globals (namespace-old-7.4/7.7); a pattern re-enables
                // qualified matching below.
                if table_ns.is_none()
                    && ns_prefix.is_none()
                    && !qualified
                    && match_pat.is_none()
                    && name.contains("::")
                {
                    continue;
                }
                // Report qualified when the pattern asked for it, or
                // when the variable belongs to another namespace.
                let rendered = if qualified || (name.contains("::") && ns_prefix.is_none()) {
                    if ns_prefix.is_some() && !name.contains("::") {
                        rel.to_string()
                    } else {
                        format!("::{}", name)
                    }
                } else {
                    rel.to_string()
                };
                if listed.insert(rendered.clone()) {
                    vars.push(Value::from_str(&rendered));
                }
            }
            Ok(Value::from_list(&vars))
        }
        "globals" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info globals",
                    1,
                    args.len(),
                    "?pattern?",
                ));
            }
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let match_pat = pattern.map(|p| p.strip_prefix("::").unwrap_or(p));
            // :: var table in Tcl hash order: replay the insertion log
            // (bare names only — namespace variables are not globals,
            // tclsh: `info globals q::*` on a namespace var returns
            // empty).
            let names: Vec<&str> = interp
                .global_var_order()
                .into_iter()
                .filter(|name| !name.contains("::"))
                .collect();
            let vars: Vec<Value> = super::super::tclhash::tcl_order(names.iter().copied())
                .into_iter()
                .filter(|name| {
                    match_pat
                        .map(|p| super::super::glob_match(p, name))
                        .unwrap_or(true)
                })
                .map(Value::from_str)
                .collect();
            Ok(Value::from_list(&vars))
        }
        "body" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "info body", 3, args.len(), "procname",
                ));
            }
            let name = args[2].as_str();
            if let Some(key) = resolve_proc_key(interp, name) {
                Ok(Value::from_str(&interp.procs[&key].body))
            } else {
                Err(Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                ))
            }
        }
        "args" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "info args",
                    2,
                    args.len(),
                    "procname",
                ));
            }
            let name = args[2].as_str();
            if let Some(key) = resolve_proc_key(interp, name) {
                let proc_def = &interp.procs[&key];
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
        "default" => {
            if args.len() != 5 {
                return Err(Error::wrong_args_with_usage(
                    "info default",
                    3,
                    args.len(),
                    "procname arg varname",
                ));
            }
            let name = args[2].as_str();
            let Some(key) = resolve_proc_key(interp, name) else {
                set_error_code(interp, &format!("TCL LOOKUP PROCEDURE {}", name));
                return Err(Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                ));
            };
            let pname = args[3].as_str().to_string();
            let param = interp.procs[&key]
                .params
                .iter()
                .find(|(n, _)| n.as_str() == pname)
                .map(|(_, d)| d.clone());
            let Some(param) = param else {
                set_error_code(interp, &format!("TCL LOOKUP ARGUMENT {}", pname));
                return Err(Error::runtime(
                    format!(
                        "procedure \"{}\" doesn't have an argument \"{}\"",
                        name, pname
                    ),
                    crate::error::ErrorCode::Generic,
                ));
            };
            // The varname is written in the caller's (current) scope; a
            // parameter without a default leaves it set to "".
            let value = match &param {
                Some(d) => Value::from_str(d),
                None => Value::from_str(""),
            };
            interp.set_var(args[4].as_str(), value)?;
            Ok(Value::from_bool(param.is_some()))
        }
        "functions" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info functions",
                    1,
                    args.len(),
                    "?pattern?",
                ));
            }
            let names: Vec<Value> = EXPR_MATH_FUNCTIONS
                .iter()
                .filter(|f| {
                    args.len() < 3 || super::super::glob_match(args[2].as_str(), f)
                })
                .map(|f| Value::from_str(f))
                .collect();
            Ok(Value::from_list(&names))
        }
        "tclversion" => {
            if args.len() != 2 {
                return Err(Error::wrong_args_msg(
                    "wrong # args: should be \"info tclversion\"",
                ));
            }
            // Reads the variable like tclsh (after `unset tcl_version`:
            // `can't read "tcl_version"`), not a map entry whose absence
            // panicked.
            interp.get_var("::tcl_version").cloned()
        }
        "level" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info level",
                    1,
                    args.len(),
                    "?number?",
                ));
            }
            if args.len() == 2 {
                // tclsh's level count includes live `namespace eval`
                // varFrames as well as proc calls (`namespace eval x
                // {info level}` → 1).
                let depth = interp.frames.len() + interp.ns_level0.len();
                return Ok(Value::from_int(depth as i64));
            }
            let raw = args[2].as_str();
            let n = tcl_get_int(raw).ok_or_else(|| {
                Error::runtime(
                    format!("expected integer but got \"{}\"", raw),
                    crate::error::ErrorCode::Generic,
                )
            })?;
            let depth = (interp.frames.len() + interp.ns_level0.len()) as i64;
            // Resolve n to a 1-based position: positive is absolute;
            // negative counts back from the current level; 0 is the
            // current level itself.  At the bare global level (depth 0)
            // every explicit number is out of range.
            let pos = if n > 0 {
                n
            } else if n == 0 {
                depth
            } else {
                depth + n
            };
            if pos < 1 || pos > depth {
                return Err(Error::runtime(
                    format!("bad level \"{}\"", raw),
                    crate::error::ErrorCode::Generic,
                ));
            }
            // Rebuild the combined chronological stack: proc frames
            // interleave with the namespace evals that enclose them —
            // each frame's ns_depth marks how many were live at its push
            // (`info level N` yields the proc's invocation words, 47.1,
            // or the ns-eval command's source, 25.9).  Frame entries
            // render lazily from the stored invocation words.
            let mut entries: Vec<String> = Vec::with_capacity(depth as usize);
            let mut consumed = 0usize;
            for f in &interp.frames {
                while consumed < f.ns_depth && consumed < interp.ns_level0.len() {
                    entries.push(interp.ns_level0[consumed].clone());
                    consumed += 1;
                }
                entries.push(crate::interp::frame_level0(f));
            }
            while consumed < interp.ns_level0.len() {
                entries.push(interp.ns_level0[consumed].clone());
                consumed += 1;
            }
            Ok(Value::from_str(&entries[(pos - 1) as usize]))
        }
        "complete" => {
            if args.len() != 3 {
                return Err(Error::wrong_args_with_usage(
                    "info complete",
                    2,
                    args.len(),
                    "command",
                ));
            }
            Ok(Value::from_bool(rtcl_parser::is_complete(args[2].as_str())))
        }
        #[cfg(feature = "std")]
        "script" => Ok(Value::from_str(interp.script_name())),
        "locals" => {
            if args.len() > 3 {
                return Err(Error::wrong_args_with_usage(
                    "info locals",
                    1,
                    args.len(),
                    "?pattern?",
                ));
            }
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            // tclsh's order (AppendLocals, includeLinks=false): the
            // compiled locals first — in the slot table's order,
            // parameters first — then the frame's dynamic table in Tcl
            // hash order.  A link (`global`/`upvar`/`variable`) is not a
            // local.  Read straight off the slots: the listing no longer
            // degrades the frame to the name-keyed store.
            let vars: Vec<Value> = interp
                .list_frame_locals(false)
                .into_iter()
                .filter(|name| {
                    pattern.map(|p| super::super::glob_match(p, name)).unwrap_or(true)
                })
                .map(|name| Value::from_str(&name))
                .collect();
            Ok(Value::from_list(&vars))
        }
        #[cfg(feature = "std")]
        "channels" => {
            let pattern = if args.len() > 2 { Some(args[2].as_str()) } else { None };
            let mut chans: Vec<Value> = interp.channels.channel_names()
                .into_iter()
                .filter(|name| {
                    pattern.map(|p| super::super::glob_match(p, name.as_str())).unwrap_or(true)
                })
                .map(|name| Value::from_str(name.as_str()))
                .collect();
            chans.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            Ok(Value::from_list(&chans))
        }
        "version" => Ok(Value::from_str("8.6")),
        "patchlevel" => Ok(Value::from_str("8.6.17")),
        // info constant varName — 9.1 (tclInfo.c InfoConstantCmd): 1 when
        // the name is const-marked in the reading scope.  A missing
        // variable reads 0 (pinned: `info constant foo` -> 0).
        "constant" => {
            if args.len() != 3 {
                set_error_code(interp, "TCL WRONGARGS");
                return Err(Error::wrong_args_with_usage(
                    "info",
                    2,
                    args.len(),
                    "constant varName",
                ));
            }
            Ok(Value::from_bool(interp.is_const_marked(args[2].as_str())))
        }
        // info consts ?pattern? — the const-marked names visible in the
        // current scope (frame marks first, then the global set), in
        // hash-table replay order.
        "consts" => {
            if args.len() > 3 {
                set_error_code(interp, "TCL WRONGARGS");
                return Err(Error::wrong_args_with_usage(
                    "info",
                    2,
                    args.len(),
                    "consts ?pattern?",
                ));
            }
            let pattern = args.get(2).map(|v| v.as_str());
            // Proc scope lists only the frame's constants (pinned:
            // `info consts` inside apply does not leak the global ones).
            let mut names: Vec<Value> = Vec::new();
            if let Some(frame) = interp.frames.last() {
                if let Some(consts) = &frame.consts {
                    for c in consts {
                        names.push(Value::from_str(c));
                    }
                }
            } else {
                for c in &interp.const_globals {
                    names.push(Value::from_str(c));
                }
            }
            if let Some(p) = pattern {
                names.retain(|n| crate::interp::glob_match(p, n.as_str()));
            }
            Ok(Value::from_list(&names))
        }
        // info cmdtype commandName — 9.0 (tclInfo.c InfoCmdTypeCmd):
        // proc | native | alias | ensemble (the corpus pins proc/native).
        "cmdtype" => {
            if args.len() != 3 {
                set_error_code(interp, "TCL WRONGARGS");
                return Err(Error::wrong_args_with_usage(
                    "info",
                    2,
                    args.len(),
                    "cmdtype commandName",
                ));
            }
            let name = args[2].as_str();
            // 全局表命令以**裸名**注册（if/puts/…）：`::if` 形输入要归一
            // 才查得到（9.1 语料 info-40.4/40.5/40.6 钉 `info cmdtype
            // ::if/::puts/::yield` → native）。
            let bare = name.trim_start_matches(':');
            let prefixed = format!("::{}", bare);
            let hit = |k: &str| {
                interp.procs.contains_key(k)
                    || interp.commands.contains_key(k)
                    || interp.ensembles.contains_key(k)
                    || interp.import_aliases.contains_key(k)
            };
            if !(hit(name) || hit(bare) || hit(prefixed.as_str())) {
                set_error_code(interp, "TCL LOOKUP COMMAND");
                return Err(tcl_err(format!("unknown command \"{}\"", name)));
            }
            let kind = if interp.procs.contains_key(bare)
                || interp.procs.contains_key(&prefixed)
            {
                "proc"
            } else if interp.import_aliases.contains_key(bare)
                || interp.import_aliases.contains_key(&prefixed)
            {
                "alias"
            } else {
                "native"
            };
            Ok(Value::from_str(kind))
        }
        "library" => Ok(interp
            .get_var("::tcl_library")
            .map(|v| v.clone())
            .unwrap_or_else(|_| Value::from_str("/usr/share/tcltk/tcl8.6"))),
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
            // Frame locals in tclsh order (compiled table first, then
            // the hash-order dynamic names; links included), read
            // straight off the slots — the listing no longer degrades
            // the target frame.
            let var_list = interp.list_frame_locals_at(level, true).join(" ");
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
            let key = resolve_proc_key(interp, name).ok_or_else(|| {
                Error::runtime(
                    format!("\"{}\" isn't a procedure", name),
                    crate::error::ErrorCode::NotFound,
                )
            })?;
            let proc_def = &interp.procs[&key];
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
        // 9.1 usage spells the positive options first (pinned
        // subst-1.1).
        return Err(Error::wrong_args_with_usage(
            "subst",
            2,
            args.len(),
            "?-backslashes? ?-commands? ?-variables? ?-nobackslashes? ?-nocommands? ?-novariables? string",
        ));
    }

    // 9.1 option face (tclCmdMZ.c TclSubstOptions): EVERY argument but
    // the last is an option — GetIndexFromObj's exact/prefix rules with
    // `ambiguous option` vs `bad option` wordings; the positive options
    // (-backslashes/-commands/-variables) turn on ONLY those
    // substitutions, the negative ones mask the default all-on set, and
    // mixing the two families errors.
    const SUBST_OPTIONS: [&str; 6] = [
        "-backslashes",
        "-commands",
        "-variables",
        "-nobackslashes",
        "-nocommands",
        "-novariables",
    ];
    let mut pos_back = false;
    let mut pos_cmds = false;
    let mut pos_vars = false;
    let mut nobackslashes = false;
    let mut nocommands = false;
    let mut novariables = false;
    for a in &args[1..args.len() - 1] {
        let word = a.as_str();
        let resolved = if SUBST_OPTIONS.contains(&word) {
            Some(word)
        } else {
            let hits: Vec<&str> = SUBST_OPTIONS
                .iter()
                .copied()
                .filter(|o| o.starts_with(word))
                .collect();
            match hits.as_slice() {
                [one] => Some(*one),
                _ => {
                    set_error_code(
                        interp,
                        &format!("TCL LOOKUP INDEX option {}", word),
                    );
                    let kind = if hits.is_empty() { "bad" } else { "ambiguous" };
                    return Err(tcl_err(format!(
                        "{} option \"{}\": must be -backslashes, -commands, -variables, -nobackslashes, -nocommands, or -novariables",
                        kind, word
                    )));
                }
            }
        };
        match resolved {
            Some("-backslashes") => pos_back = true,
            Some("-commands") => pos_cmds = true,
            Some("-variables") => pos_vars = true,
            Some("-nobackslashes") => nobackslashes = true,
            Some("-nocommands") => nocommands = true,
            Some("-novariables") => novariables = true,
            _ => {}
        }
    }
    if pos_back || pos_cmds || pos_vars {
        if nobackslashes || nocommands || novariables {
            return Err(tcl_err("cannot combine positive and negative options"));
        }
        nobackslashes = !pos_back;
        nocommands = !pos_cmds;
        novariables = !pos_vars;
    }

    let template = args[args.len() - 1].as_str();
    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut ci = 0;

    while ci < chars.len() {
        let ch = chars[ci];
        match ch {
            '\\' if !nobackslashes && ci + 1 < chars.len() => {
                ci += 1;
                result.push_str(&subst_escape(&chars, &mut ci));
            }
            '$' if !novariables && ci + 1 < chars.len() => {
                ci += 1;
                // tclsh: a `$` not followed by a name stays literal; an
                // unset variable is an ERROR (subst-6.1), never a
                // literal passthrough.
                let var_name = scan_var_name(&chars, &mut ci);
                if var_name.is_empty() {
                    result.push('$');
                } else if ci < chars.len() && chars[ci] == '(' {
                    // Array reference: the index takes word-level
                    // substitutions, and a `return`/`break`/`continue`
                    // inside it aborts the read (subst-8.9/10.6/11.6).
                    ci += 1;
                    let mut index = String::new();
                    let mut skip_read = false;
                    while ci < chars.len() && chars[ci] != ')' {
                        match chars[ci] {
                            '[' => {
                                ci += 1;
                                match run_bracket_script(
                                    interp,
                                    &chars,
                                    &mut ci,
                                    nobackslashes,
                                )? {
                                    SubstOutcome::Done(s) => index.push_str(&s),
                                    SubstOutcome::Returned(s) => {
                                        result.push_str(&s);
                                        skip_read = true;
                                        break;
                                    }
                                    SubstOutcome::Break => {
                                        return Ok(Value::from_str(&result))
                                    }
                                    SubstOutcome::Continue => {
                                        skip_read = true;
                                        break;
                                    }
                                }
                            }
                            '\\' if !nobackslashes && ci + 1 < chars.len() => {
                                ci += 1;
                                index.push_str(&subst_escape(&chars, &mut ci));
                            }
                            c => {
                                index.push(c);
                                ci += 1;
                            }
                        }
                    }
                    if ci < chars.len() {
                        ci += 1; // past ')'
                    }
                    if !skip_read {
                        let full = format!("{}({})", var_name, index);
                        let val = interp.get_var(&full)?;
                        result.push_str(val.as_str());
                    }
                } else {
                    let val = interp.get_var(&var_name)?;
                    result.push_str(val.as_str());
                }
            }
            '[' if !nocommands => {
                ci += 1;
                match run_bracket_script(interp, &chars, &mut ci, nobackslashes)? {
                    SubstOutcome::Done(s) | SubstOutcome::Returned(s) => {
                        result.push_str(&s)
                    }
                    // `break` stops subst with what it has so far
                    // (subst-10.1: "foo ").
                    SubstOutcome::Break => return Ok(Value::from_str(&result)),
                    // `continue` skips the rest of the bracketed script
                    // (subst-11.1: "foo  bar").
                    SubstOutcome::Continue => {}
                }
            }
            _ => {
                result.push(ch);
                ci += 1;
            }
        }
    }

    Ok(Value::from_str(&result))
}

/// Decode one backslash escape in a subst template; `ci` points at the
/// character following the backslash and advances past the escape.
/// Mirrors the parser's rules: `\n`-style shorthands, `\xHH` (all hex
/// digits, low 8 bits), `\uHHHH`, 1-3 octal digits, and
/// backslash-newline collapsing to a single space.
fn subst_escape(chars: &[char], ci: &mut usize) -> String {
    let c = chars[*ci];
    let simple = match c {
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        'f' => Some('\u{000C}'),
        'v' => Some('\u{000B}'),
        'b' => Some('\u{0008}'),
        'a' => Some('\u{0007}'),
        '\\' => Some('\\'),
        '"' => Some('"'),
        _ => None,
    };
    if let Some(ch) = simple {
        *ci += 1;
        return ch.to_string();
    }
    match c {
        'x' => {
            *ci += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            while *ci < chars.len() && n < 8 {
                match chars[*ci].to_digit(16) {
                    Some(d) => { v = v * 16 + d; *ci += 1; n += 1; }
                    None => break,
                }
            }
            if n == 0 {
                "x".to_string()
            } else {
                char::from_u32(v & 0xff).unwrap_or('\u{FFFD}').to_string()
            }
        }
        'u' => {
            *ci += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            while *ci < chars.len() && n < 4 {
                match chars[*ci].to_digit(16) {
                    Some(d) => { v = v * 16 + d; *ci += 1; n += 1; }
                    None => break,
                }
            }
            if n == 0 {
                "u".to_string()
            } else {
                char::from_u32(v).unwrap_or('\u{FFFD}').to_string()
            }
        }
        '0'..='7' => {
            let mut v: u32 = 0;
            let mut n = 0;
            while *ci < chars.len() && n < 3 {
                match chars[*ci].to_digit(8) {
                    Some(d) => { v = v * 8 + d; *ci += 1; n += 1; }
                    None => break,
                }
            }
            char::from_u32(v & 0xff).unwrap_or('\u{FFFD}').to_string()
        }
        '\n' => {
            *ci += 1;
            while *ci < chars.len() && (chars[*ci] == ' ' || chars[*ci] == '\t') {
                *ci += 1;
            }
            " ".to_string()
        }
        other => {
            *ci += 1;
            other.to_string()
        }
    }
}

/// What a bracketed script did for the surrounding subst.
enum SubstOutcome {
    /// Ran through the closing `]` — the accumulated output.
    Done(String),
    /// An inner `return` — its value is the substitution result.
    Returned(String),
    Break,
    Continue,
}

/// Run one `[`-bracketed command script for subst: commands are parsed
/// one at a time and evaluated as each terminator is seen, so
/// `subst "\[incr x;"` still increments x before the missing
/// close-bracket error (subst-12.3) while `subst {[set a 1}` runs
/// nothing (subst-5.5).  `return` aborts the remaining script, its
/// value is the substitution result (subst-8.1) — and the remainder
/// must still PARSE (`subst {foo [return {x} ; set a {}"" ; stuff]
/// bar}` is a parse error, subst-8.7).
fn run_bracket_script(
    interp: &mut Interp,
    chars: &[char],
    ci: &mut usize,
    nobackslashes: bool,
) -> Result<SubstOutcome> {
    let mut result = String::new();
    loop {
        let mut cmd = String::new();
        let mut brace = 0i32;
        let mut bdepth = 0i32;
        let mut quote = false;
        let mut close = false;
        let mut terminated = false;
        // tclParse.c word rules: `"` opens a quoted word only at word
        // start — a mid-word quote is an ordinary character (pidx 钉形:
        // `$foo([x"y])` 的下标命令名是 `x"y`，引号在词中不开引文；
        // word_start 在命令起点、`[` 之后与空白之后置真).
        let mut word_start = true;
        while *ci < chars.len() {
            let c = chars[*ci];
            if quote {
                if c == '\\' && !nobackslashes && *ci + 1 < chars.len() {
                    cmd.push(c);
                    cmd.push(chars[*ci + 1]);
                    *ci += 2;
                    continue;
                }
                if c == '"' {
                    quote = false;
                    word_start = false;
                }
                cmd.push(c);
                *ci += 1;
                continue;
            }
            if brace > 0 {
                // Inside a braced word everything is literal except the
                // brace nesting itself.
                match c {
                    '{' => brace += 1,
                    '}' => brace -= 1,
                    _ => {}
                }
                cmd.push(c);
                *ci += 1;
                continue;
            }
            match c {
                '\\' if !nobackslashes && *ci + 1 < chars.len() => {
                    cmd.push(c);
                    cmd.push(chars[*ci + 1]);
                    *ci += 2;
                    word_start = false;
                }
                '"' if word_start => { quote = true; cmd.push(c); *ci += 1; }
                '"' => { cmd.push(c); *ci += 1; word_start = false; }
                '{' => { brace = 1; cmd.push(c); *ci += 1; }
                '[' => { bdepth += 1; cmd.push(c); *ci += 1; word_start = true; }
                ']' if bdepth > 0 => { bdepth -= 1; cmd.push(c); *ci += 1; }
                ']' => {
                    *ci += 1;
                    close = true;
                    terminated = true;
                    break;
                }
                ';' | '\n' => {
                    *ci += 1;
                    terminated = true;
                    break;
                }
                ' ' | '\t' | '\r' => { cmd.push(c); *ci += 1; word_start = true; }
                _ => { cmd.push(c); *ci += 1; word_start = false; }
            }
        }
        if terminated && !cmd.trim().is_empty() {
            match interp.eval(&cmd) {
                Ok(v) => result.push_str(v.as_str()),
                Err(Error::ControlFlow {
                    kind: crate::error::ControlFlow::Return,
                    ref value,
                    ..
                }) => {
                    let vtext = value
                        .as_ref()
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default();
                    if !close {
                        check_bracket_tail(chars, ci, nobackslashes)?;
                    }
                    return Ok(SubstOutcome::Returned(vtext));
                }
                Err(Error::ControlFlow {
                    kind: crate::error::ControlFlow::Break,
                    ..
                }) => return Ok(SubstOutcome::Break),
                Err(Error::ControlFlow {
                    kind: crate::error::ControlFlow::Continue,
                    ..
                }) => {
                    if !close {
                        check_bracket_tail(chars, ci, nobackslashes)?;
                    }
                    return Ok(SubstOutcome::Continue);
                }
                Err(e) => return Err(e),
            }
        }
        if close {
            return Ok(SubstOutcome::Done(result));
        }
        if !terminated {
            // An open quoted word at EOF is the quote's error (tclsh
            // `subst {[foo "bar baz]}` → missing ").
            if quote {
                return Err(Error::Msg("missing \"".to_string()));
            }
            return Err(Error::Msg("missing close-bracket".to_string()));
        }
        // Terminated by `;`/newline — the next command of the same
        // bracket follows.
    }
}

/// Consume the remainder of a bracketed script after `return` or
/// `continue` aborts it — not evaluated, but it must still parse
/// (`set a {}""` surfaces "extra characters after close-brace",
/// subst-8.7).
fn check_bracket_tail(chars: &[char], ci: &mut usize, nobackslashes: bool) -> Result<()> {
    let mut rest = String::new();
    let mut brace = 0i32;
    let mut bdepth = 0i32;
    let mut quote = false;
    // Word-start discipline for `"` — same rule as run_bracket_script
    // (mid-word quotes are literal characters).
    let mut word_start = true;
    while *ci < chars.len() {
        let c = chars[*ci];
        if quote {
            if c == '\\' && !nobackslashes && *ci + 1 < chars.len() {
                rest.push(c);
                rest.push(chars[*ci + 1]);
                *ci += 2;
                continue;
            }
            if c == '"' {
                quote = false;
                word_start = false;
            }
            rest.push(c);
            *ci += 1;
            continue;
        }
        if brace > 0 {
            match c {
                '{' => brace += 1,
                '}' => brace -= 1,
                _ => {}
            }
            rest.push(c);
            *ci += 1;
            continue;
        }
        match c {
            '\\' if !nobackslashes && *ci + 1 < chars.len() => {
                rest.push(c);
                rest.push(chars[*ci + 1]);
                *ci += 2;
                word_start = false;
            }
            '"' if word_start => { quote = true; rest.push(c); *ci += 1; }
            '"' => { rest.push(c); *ci += 1; word_start = false; }
            '{' => { brace = 1; rest.push(c); *ci += 1; }
            '[' => { bdepth += 1; rest.push(c); *ci += 1; word_start = true; }
            ']' if bdepth > 0 => { bdepth -= 1; rest.push(c); *ci += 1; }
            ']' => {
                *ci += 1;
                if let Err(pe) = rtcl_parser::parse(rest.trim_end()) {
                    return Err(Error::Msg(pe.message));
                }
                return Ok(());
            }
            ' ' | '\t' | '\r' => { rest.push(c); *ci += 1; word_start = true; }
            _ => { rest.push(c); *ci += 1; word_start = false; }
        }
    }
    if quote {
        return Err(Error::Msg("missing \"".to_string()));
    }
    Err(Error::Msg("missing close-bracket".to_string()))
}

/// `$name`, `${any text}`, `$::ns::name` — returns the variable name;
/// an empty result leaves the `$` literal.  Name characters follow the
/// parser's rule: ASCII alphanumerics, `_`, and non-ASCII letters
/// (symbols like `→` never join).  The array index is handled by the
/// caller since it needs command substitution.
fn scan_var_name(chars: &[char], ci: &mut usize) -> String {
    let mut name = String::new();
    if chars[*ci] == '{' {
        *ci += 1;
        while *ci < chars.len() && chars[*ci] != '}' {
            name.push(chars[*ci]);
            *ci += 1;
        }
        if *ci < chars.len() {
            *ci += 1; // skip '}'
        }
        return name;
    }
    while *ci < chars.len() {
        let c = chars[*ci];
        if c == ':' && *ci + 1 < chars.len() && chars[*ci + 1] == ':' {
            name.push_str("::");
            *ci += 2;
        } else if c.is_ascii_alphanumeric() || c == '_' || c.is_alphabetic() {
            name.push(c);
            *ci += 1;
        } else {
            break;
        }
    }
    name
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
    // Fast path: an ordinary local or global with no trace on it, already
    // holding a plain string — append to its own buffer (tclsh appends
    // to the variable's object).  The buffer grows, so a loop of appends
    // is linear; rebuilding the string took two copies of the whole
    // thing on every call.  Everything else takes the path below: the
    // variable's creation and its errors, other representations (an
    // integer, a list), links, arrays, and traced variables, whose write
    // traces `set_var` fires.
    // D1 步骤 1：with_var_slot（全局臂双写 arena + 镜像重放——闭包
    // 逐 arg 追加、对 args 纯读，重放确定性成立）。
    if let Some(v) = interp.with_var_slot(var_name, |slot| {
        let mut in_place = true;
        for arg in &args[2..] {
            if !slot.append_str_in_place(arg.as_str()) {
                in_place = false;
                break;
            }
        }
        in_place.then(|| slot.clone())
    }) {
        return Ok(v);
    }
    // A value that is not a plain string: what was appended so far
    // is nothing (the first non-string value stopped the loop before
    // any change), and the rebuild below reads it as it is.
    // T7 vars: element fast path — an existing element whose flat key is
    // the name as spelled (frame array-local, or a global array at "::"),
    // no traces, holding a plain string: append to the cell's own buffer
    // (the same rationale as the scalar path above; var_slot_fast skips
    // element names outright).  Creation, non-string cells, links and
    // namespaces take the generic rebuild below — same reads, same
    // errors, same trace firing (none when no traces are registered).
    if let Some((array_name, elem_index)) = crate::interp::util::split_array_ref(var_name) {
        if !crate::interp::util::has_ns_sep(var_name)
            && interp.n_trace_hosts == 0
            && interp.dead_flat.iter().all(|k| k != array_name)
            && interp.flat_aliases.iter().all(|(k, _)| k != array_name)
            && interp.ns_transient.iter().all(|(k, _)| k != array_name)
        {
            let slot = if let Some(frame) = interp.var_frame_mut() {
                // D1 步骤 3：宿主单探测（vt 位 + 子表格——T7 快径两跳；
                // 改格 ⟺ Some——rep 前置拒绝都在改格前；镜像重放 debug
                // 并跑）。
                if frame.upvars.is_empty() || !frame.upvars.contains_key(array_name) {
                    if let Some(host) = frame.vt.get_mut(array_name) {
                        if host.is_array() {
                            let r = host
                                .elements
                                .as_deref_mut()
                                .and_then(|t| t.get_mut(elem_index))
                                .filter(|v| v.has_value())
                                .map(|c| {
                                    let mut in_place = true;
                                    for arg in &args[2..] {
                                        if !c.value.append_str_in_place(arg.as_str()) {
                                            in_place = false;
                                            break;
                                        }
                                    }
                                    in_place.then(|| c.value.clone())
                                });
                            #[cfg(debug_assertions)]
                            if r.is_some() {
                                if let Some(c) = frame.locals.get_mut(var_name) {
                                    let mut in_place = true;
                                    for arg in &args[2..] {
                                        if !c.append_str_in_place(arg.as_str()) {
                                            in_place = false;
                                            break;
                                        }
                                    }
                                    let _ = in_place;
                                }
                            }
                            r
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else if interp.current_namespace.as_ref() == "::" {
                // D1 步骤 3：arena 权威子表格（宿主单探测）+ 镜像重放。
                let r = {
                    match interp.var_arena.root_mut().get_mut(array_name) {
                        Some(host) if host.is_array() => host
                            .elements
                            .as_deref_mut()
                            .and_then(|t| t.get_mut(elem_index))
                            .filter(|v| v.has_value())
                            .map(|c| {
                                let mut in_place = true;
                                for arg in &args[2..] {
                                    if !c.value.append_str_in_place(arg.as_str()) {
                                        in_place = false;
                                        break;
                                    }
                                }
                                in_place.then(|| c.value.clone())
                            }),
                        _ => None,
                    }
                };
                #[cfg(debug_assertions)]
                if r.is_some() {
                    if let Some(c) = interp.globals.get_mut(var_name) {
                        let mut in_place = true;
                        for arg in &args[2..] {
                            if !c.append_str_in_place(arg.as_str()) {
                                in_place = false;
                                break;
                            }
                        }
                        let _ = in_place;
                    }
                }
                r
            } else {
                None
            };
            let slot = slot.flatten();
            if let Some(v) = slot {
                return Ok(v);
            }
        }
    }
    let mut current = String::new();
    if let Ok(old) = interp.get_var(var_name) {
        current.push_str(old.as_str());
    }
    for arg in &args[2..] {
        current.push_str(arg.as_str());
    }
    interp.set_var(var_name, Value::from_string(current))
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

// ── ::tcl::unsupported::assemble ────────────────────────────────────
//
// A stack-machine interpreter for tclsh 8.6's assembler.  The corpus
// pins the exception-range pattern of assemble-52.1 (six nested
// beginCatch/push/invokeStk/jump/label/endCatch sequences) plus the
// assembler's own error strings, all probed against tclsh 8.6.17
// (judge/probes/unsupported_assemble.tcl).  Opcodes outside the
// implemented subset error with tclsh's exact candidate list.

/// tclsh's exact `bad instruction` candidate list (sorted, as ensemble
/// lists are), captured verbatim from 8.6.17.
const ASSEMBLE_OPCODES: &str = "push, add, append, appendArray, appendArrayStk, appendStk, arrayExistsImm, arrayExistsStk, arrayMakeImm, arrayMakeStk, beginCatch, bitand, bitnot, bitor, bitxor, clockRead, concat, concatStk, coroName, currentNamespace, dictAppend, dictExists, dictExpand, dictGet, dictIncrImm, dictLappend, dictRecombineStk, dictRecombineImm, dictSet, dictUnset, div, dup, endCatch, eq, eval, evalStk, exist, existArray, existArrayStk, existStk, expon, expr, exprStk, ge, gt, incr, incrArray, incrArrayImm, incrArrayStk, incrArrayStkImm, incrImm, incrStk, incrStkImm, infoLevelArgs, infoLevelNumber, invokeStk, jump, jump4, jumpFalse, jumpFalse4, jumpTable, jumpTrue, jumpTrue4, label, land, lappend, lappendArray, lappendArrayStk, lappendList, lappendListArray, lappendListArrayStk, lappendListStk, lappendStk, le, lindexMulti, list, listConcat, listIn, listIndex, listIndexImm, listLength, listNotIn, load, loadArray, loadArrayStk, loadStk, lor, lsetFlat, lsetList, lshift, lt, mod, mult, neq, nop, not, nsupvar, numericType, originCmd, over, pop, pushReturnCode, pushReturnOpts, pushResult, regexp, resolveCmd, reverse, rshift, store, storeArray, storeArrayStk, storeStk, strcaseLower, strcaseTitle, strcaseUpper, strcmp, strcat, streq, strfind, strindex, strlen, strmap, strmatch, strneq, strrange, strreplace, strrfind, strtrim, strtrimLeft, strtrimRight, sub, tclooClass, tclooIsObject, tclooNamespace, tclooSelf, tryCvtToBoolean, tryCvtToNumeric, uminus, unset, unsetArray, unsetArrayStk, unsetStk, uplus, upvar, variable, verifyDict, or yield";

enum AsmIns {
    Push(Value),
    Pop,
    Nop,
    Over(i64),
    Invoke(usize),
    Jump(usize),
    BeginCatch(usize),
    EndCatch,
    Label,
}

/// One instruction in first-pass form: label references still by name.
enum RawAsmIns {
    Push(Value),
    Pop,
    Nop,
    Over(i64),
    Invoke(usize),
    Jump(String),
    BeginCatch(String),
    EndCatch,
    Label(String),
}

fn assemble_operand(interp: &mut Interp, word: &crate::parser::Word) -> Result<String> {
    match word {
        crate::parser::Word::Literal(s) => Ok(s.clone()),
        _ => {
            // Non-literal operands substitute at assemble time.
            Ok(interp.eval_word(word)?.as_str().to_string())
        }
    }
}

pub fn cmd_assemble(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage(
            "::tcl::unsupported::assemble",
            2,
            args.len(),
            "bytecodeList",
        ));
    }
    let script = args[1].as_str().to_string();
    let commands = crate::parser::parse(&script)?;

    let mut raw: Vec<RawAsmIns> = Vec::new();
    for cmd in &commands {
        let arity = cmd.words.len();
        let op = match &cmd.words[0] {
            crate::parser::Word::Literal(s) => s.clone(),
            _ => interp.eval_word(&cmd.words[0])?.as_str().to_string(),
        };
        match op.as_str() {
            "push" => {
                if arity != 2 {
                    return Err(Error::wrong_args_with_usage(
                        "push", 2, arity, "value",
                    ));
                }
                // push stores the word's VALUE (literal text for literal
                // words).
                let v = interp.eval_word(&cmd.words[1])?;
                raw.push(RawAsmIns::Push(v));
            }
            "pop" => raw.push(RawAsmIns::Pop),
            "over" => {
                if arity != 2 {
                    return Err(Error::wrong_args_with_usage("over", 2, arity, "count"));
                }
                let text = assemble_operand(interp, &cmd.words[1])?;
                let n = tcl_get_int(&text).ok_or_else(|| {
                    Error::runtime(
                        format!("expected integer but got \"{}\"", text),
                        crate::error::ErrorCode::Generic,
                    )
                })?;
                raw.push(RawAsmIns::Over(n));
            }
            "nop" => raw.push(RawAsmIns::Nop),
            "invokeStk" => {
                if arity != 2 {
                    return Err(Error::wrong_args_with_usage(
                        "invokeStk", 2, arity, "invokeStk count",
                    ));
                }
                let text = assemble_operand(interp, &cmd.words[1])?;
                let n = tcl_get_int(&text).ok_or_else(|| {
                    Error::runtime(
                        format!("expected integer but got \"{}\"", text),
                        crate::error::ErrorCode::Generic,
                    )
                })?;
                raw.push(RawAsmIns::Invoke(n as usize));
            }
            "jump" | "jump4" => {
                if arity != 2 {
                    return Err(Error::wrong_args_with_usage(
                        "jump", 2, arity, "jump label",
                    ));
                }
                raw.push(RawAsmIns::Jump(assemble_operand(interp, &cmd.words[1])?));
            }
            "beginCatch" => {
                if arity != 2 {
                    return Err(Error::wrong_args_with_usage(
                        "beginCatch", 2, arity, "beginCatch label",
                    ));
                }
                raw.push(RawAsmIns::BeginCatch(assemble_operand(
                    interp,
                    &cmd.words[1],
                )?));
            }
            "endCatch" => raw.push(RawAsmIns::EndCatch),
            "label" => {
                if arity != 2 {
                    return Err(Error::wrong_args_with_usage(
                        "label", 2, arity, "label name",
                    ));
                }
                raw.push(RawAsmIns::Label(assemble_operand(interp, &cmd.words[1])?));
            }
            other => {
                // The instruction word through tclsh's index lookup.
                set_error_code(
                    interp,
                    &format!("TCL LOOKUP INDEX instruction {}", crate::value::tcl_quote(other)),
                );
                return Err(Error::runtime(
                    format!(
                        "bad instruction \"{}\": must be {}",
                        other, ASSEMBLE_OPCODES
                    ),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
    }

    // Second pass: resolve labels to instruction indices.
    let mut labels: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, ins) in raw.iter().enumerate() {
        if let RawAsmIns::Label(name) = ins {
            labels.insert(name.clone(), i);
        }
    }
    // 重复 label 定义 → tclsh 同文错误（cmp_assemble a_dup_label）。
    {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for ins in raw.iter() {
            if let RawAsmIns::Label(name) = ins {
                if !seen.insert(name.as_str()) {
                    // tclsh names the label in the code (TclAssembleObjCmd).
                    set_error_code(interp, &format!("TCL ASSEM DUPLABEL {}", name));
                    return Err(Error::runtime(
                        format!("duplicate definition of label \"{}\"", name),
                        crate::error::ErrorCode::Generic,
                    ));
                }
            }
        }
    }
    let mut resolve = |name: &str| -> Result<usize> {
        labels.get(name).copied().ok_or_else(|| {
            set_error_code(interp, &format!("TCL ASSEM NOLABEL {}", name));
            Error::runtime(
                format!("undefined label \"{}\"", name),
                crate::error::ErrorCode::NotFound,
            )
        })
    };
    let mut code: Vec<AsmIns> = Vec::with_capacity(raw.len());
    for ins in raw {
        code.push(match ins {
            RawAsmIns::Push(v) => AsmIns::Push(v),
            RawAsmIns::Pop => AsmIns::Pop,
            RawAsmIns::Nop => AsmIns::Nop,
            RawAsmIns::Invoke(n) => AsmIns::Invoke(n),
            RawAsmIns::Over(n) => AsmIns::Over(n),
            RawAsmIns::Jump(name) => AsmIns::Jump(resolve(&name)?),
            RawAsmIns::BeginCatch(name) => AsmIns::BeginCatch(resolve(&name)?),
            RawAsmIns::EndCatch => AsmIns::EndCatch,
            RawAsmIns::Label(_) => AsmIns::Label,
        });
    }

    // 异常语境一致性检查（tclAssembly.c ProcessCatchesInBasicBlock 的
    // 指令级镜像）：每条指令的 (enclosing catch, state) 逐路径一致，
    // 不一致 → "execution reaches an instruction in inconsistent
    // exception contexts"（tclsh TCL ASSEM BADCATCH）。rtcl 子集 =
    // jump/label/beginCatch/endCatch，逐指令传播即可覆盖。
    {
        const NONE: u8 = 0;
        const INCATCH: u8 = 1;
        const CAUGHT: u8 = 2;
        let n = code.len();
        let mut ctx: Vec<(Option<usize>, u8)> = vec![(None, NONE); n.max(1)];
        // 三态：0=未访问。enclosing = 该路径上的 beginCatch 指令下标。
        let mut visited: Vec<Option<(Option<usize>, u8)>> = vec![None; n];
        if n > 0 {
            let mut work = vec![(0usize, None::<usize>, NONE)];
            while let Some((pc, enclosing, state)) = work.pop() {
                if pc >= n {
                    continue;
                }
                match visited[pc] {
                    Some((old_enc, old_state)) => {
                        if old_enc != enclosing {
                            set_error_code(interp, "TCL ASSEM BADCATCH");
                            return Err(Error::runtime(
                                "execution reaches an instruction in inconsistent exception contexts".to_string(),
                                crate::error::ErrorCode::Generic,
                            ));
                        }
                        if state <= old_state {
                            continue;
                        }
                    }
                    None => {
                        visited[pc] = Some((enclosing, state));
                    }
                }
                ctx[pc] = (enclosing, state);
                match &code[pc] {
                    AsmIns::BeginCatch(target) => {
                        // 直落：进入 catch 体；异常落点：CAUGHT。
                        work.push((pc + 1, Some(pc), INCATCH));
                        work.push((*target, Some(pc), CAUGHT));
                    }
                    AsmIns::EndCatch => {
                        match enclosing {
                            None => {
                                set_error_code(interp, "TCL ASSEM BADENDCATCH");
                                return Err(Error::runtime(
                                    "endCatch without a corresponding beginCatch".to_string(),
                                    crate::error::ErrorCode::Generic,
                                ));
                            }
                            Some(b) => {
                                // 退出后回到 beginCatch 自身的语境。
                                let (outer_enc, outer_state) = ctx[b];
                                work.push((pc + 1, outer_enc, outer_state));
                            }
                        }
                    }
                    AsmIns::Jump(target) => work.push((*target, enclosing, state)),
                    _ => work.push((pc + 1, enclosing, state)),
                }
            }
            // 代码收尾仍在 catch 内 → "catch still active on exit"。
            let last = n - 1;
            let (_, state) = ctx[last];
            if state >= INCATCH
                && !matches!(code[last], AsmIns::EndCatch | AsmIns::Jump(_))
            {
                set_error_code(interp, "TCL ASSEM UNCLOSEDCATCH");
                return Err(Error::runtime(
                    "catch still active on exit from assembly code".to_string(),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
    }

    // Execute.
    let mut stack: Vec<Value> = Vec::new();
    // Catch frames: (handler pc, operand-stack depth at beginCatch).
    let mut catches: Vec<(usize, usize)> = Vec::new();
    let mut pc = 0usize;
    while pc < code.len() {
        // A `nop`/`jump` loop invokes no command: without this it outran
        // an armed step budget.
        interp.charge_step()?;
        let cur = pc;
        pc += 1;
        match &code[cur] {
            AsmIns::Push(v) => stack.push(v.clone()),
            AsmIns::Pop => {
                if stack.pop().is_none() {
                    return Err(Error::runtime(
                        "stack underflow".to_string(),
                        crate::error::ErrorCode::Generic,
                    ));
                }
            }
            AsmIns::Nop | AsmIns::Label => {}
            AsmIns::Over(depth) => {
                // INST_OVER：操作数为立即数，压入栈顶下方 depth 处的
                // 副本（0 = 复制栈顶；cmp_assemble a_over）。
                let d = *depth;
                if d < 0 || (d as usize) >= stack.len() {
                    return Err(Error::runtime(
                        "stack underflow".to_string(),
                        crate::error::ErrorCode::Generic,
                    ));
                }
                let v = stack[stack.len() - 1 - d as usize].clone();
                stack.push(v);
            }
            AsmIns::Jump(target) => pc = *target,
            AsmIns::BeginCatch(handler) => catches.push((*handler, stack.len())),
            AsmIns::EndCatch => {
                catches.pop();
            }
            AsmIns::Invoke(n) => {
                if stack.len() < *n {
                    return Err(Error::runtime(
                        "stack underflow".to_string(),
                        crate::error::ErrorCode::Generic,
                    ));
                }
                let split = stack.len() - n;
                let call: Vec<Value> = stack.drain(split..).collect();
                match interp.dispatch_values(&call) {
                    Ok(v) => stack.push(v),
                    Err(e) => {
                        if !interp.err_is_error(&e) {
                            return Err(e);
                        }
                        // An exception unwinds to the innermost active
                        // range: resume at its label with the stack back
                        // at the depth where the range began.  The catch
                        // consumes the error (nothing leaks into
                        // errorInfo from inside the assembled program).
                        match catches.pop() {
                            Some((handler, depth)) => {
                                stack.truncate(depth);
                                pc = handler;
                            }
                            None => return Err(e),
                        }
                    }
                }
            }
        }
    }
    match stack.len() {
        0 => Ok(Value::empty()),
        1 => Ok(stack.remove(0)),
        n => {
            set_error_code(interp, "TCL ASSEM BADSTACK");
            Err(Error::runtime(
                format!("stack is unbalanced on exit from the code (depth={})", n),
                crate::error::ErrorCode::Generic,
            ))
        }
    }
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
    tcl_err(msg.to_string())
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
        if i + 3 <= end && (s[i + 1].eq_ignore_ascii_case(&'n')) && (s[i + 2].eq_ignore_ascii_case(&'f'))
        {
            return FloatScan::Inf(i + 3 - start);
        }
        // Partial "i"/"in": consumed prefix only.
        let matched = if i + 1 < end && s[i + 1].eq_ignore_ascii_case(&'n') {
            2
        } else {
            1
        };
        return FloatScan::Fail(i + matched);
    }
    if i < end && (s[i] == 'n' || s[i] == 'N') {
        if i + 3 <= end
            && s[i + 1].eq_ignore_ascii_case(&'a')
            && s[i + 2].eq_ignore_ascii_case(&'n')
        {
            return FloatScan::NanMatched;
        }
        let matched = if i + 1 < end && s[i + 1].eq_ignore_ascii_case(&'a') {
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

// ── ::tcl::pkgconfig ────────────────────────────────────────────────────

/// Build-configuration table for `::tcl::pkgconfig` (tclPkgConfig.c).
/// Key order and values mirror the reference oracle build (tclsh 8.6.17,
/// Debian) so `pkgconfig list` / `get` render byte-identically; the corpus
/// pins determinism only, byte-parity is the target. Keys are looked up
/// case-sensitively (tclsh: `get LIST` → key not known).
const PKG_CONFIG: &[(&str, &str)] = &[
    ("debug", "0"),
    ("threaded", "1"),
    ("profiled", "0"),
    ("64bit", "0"),
    ("optimized", "1"),
    ("mem_debug", "0"),
    ("compile_debug", "0"),
    ("compile_stats", "0"),
    ("libdir,runtime", "/usr/lib/x86_64-linux-gnu"),
    ("bindir,runtime", "/usr/bin"),
    ("scriptdir,runtime", "/usr/share/tcltk/tcl8.6"),
    ("includedir,runtime", "/usr/include/tcl8.6"),
    ("docdir,runtime", "/usr/share/man"),
    ("libdir,install", "/usr/lib/x86_64-linux-gnu"),
    ("bindir,install", "/usr/bin"),
    ("scriptdir,install", "/usr/share/tcltk/tcl8.6"),
    ("includedir,install", "/usr/include/tcl8.6"),
    ("docdir,install", "/usr/share/man"),
];

/// `::tcl::pkgconfig subcommand ?arg ...?` — build-configuration query.
/// Decoded from tclsh 8.6.17 (probes in judge/probes/pkgconfig_probe.tcl):
/// - usage messages embed the *invoked* spelling (`objv[0]`): unqualified
///   `tcl::pkgconfig` reports `should be "tcl::pkgconfig ..."`.
/// - subcommand resolution is Tcl_GetIndexFromObj-style: exact, else unique
///   prefix (`ge`→get, `l`→list); empty string is ambiguous ("g" vs "l"
///   both match → `ambiguous subcommand`); case-sensitive (`GET`/`LIST`
///   fail as `bad subcommand`).
/// - arity table (both subcommands fold overflow onto the generic usage):
///   no subcommand → `subcommand ?arg?`; `list` +1 extra → `cmd list`,
///   +≥2 → generic; `get` +0 → `cmd get key`, +≥2 → generic.
/// - unknown key → `key not known` + `TCL LOOKUP CONFIG <key>`
///   (case-sensitive); lookup failures carry `TCL LOOKUP INDEX`/`CONFIG`
///   built as proper lists so empty/space elements brace (tclsh renders
///   `subcommand {}` / `CONFIG {a b}`).
pub fn cmd_pkgconfig(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let invoked = args[0].as_str();
    if args.len() < 2 {
        return Err(pkgconfig_wrong(
            interp,
            format!("wrong # args: should be \"{invoked} subcommand ?arg?\""),
        ));
    }
    let sub = args[1].as_str();
    match pkgconfig_resolve_sub(sub) {
        Ok("list") => match args.len() {
            2 => {
                let keys: Vec<Value> = PKG_CONFIG
                    .iter()
                    .map(|(k, _)| Value::from_str(k))
                    .collect();
                Ok(Value::from_list(&keys))
            }
            3 => Err(pkgconfig_wrong(
                interp,
                format!("wrong # args: should be \"{invoked} list\""),
            )),
            // Overflow folds back onto the ensemble-level usage.
            _ => Err(pkgconfig_wrong(
                interp,
                format!("wrong # args: should be \"{invoked} subcommand ?arg?\""),
            )),
        },
        // Only "get" remains — resolve_sub yields "get"/"list" exclusively.
        Ok(_) => match args.len() {
            2 => Err(pkgconfig_wrong(
                interp,
                format!("wrong # args: should be \"{invoked} get key\""),
            )),
            3 => {
                let key = args[2].as_str();
                match PKG_CONFIG.iter().find(|(k, _)| *k == key) {
                    Some((_, val)) => Ok(Value::from_str(val)),
                    None => {
                        set_error_code(interp, &pkgconfig_code(&["TCL", "LOOKUP", "CONFIG", key]));
                        Err(tcl_err("key not known"))
                    }
                }
            }
            _ => Err(pkgconfig_wrong(
                interp,
                format!("wrong # args: should be \"{invoked} subcommand ?arg?\""),
            )),
        },
        Err(ambiguous) => {
            let verb = if ambiguous { "ambiguous" } else { "bad" };
            set_error_code(
                interp,
                &pkgconfig_code(&["TCL", "LOOKUP", "INDEX", "subcommand", sub]),
            );
            Err(tcl_err(format!(
                "{verb} subcommand \"{sub}\": must be get or list"
            )))
        }
    }
}

/// Resolve the pkgconfig subcommand Tcl_GetIndexFromObj-style: exact match
/// first, else unique non-empty prefix. `None` = no match; `Some(true)` =
/// ambiguous prefix (only the empty string, "g"/"l" leave nothing shared).
fn pkgconfig_resolve_sub(sub: &str) -> core::result::Result<&'static str, bool> {
    const SUBS: [&str; 2] = ["get", "list"];
    if let Some(found) = SUBS.iter().find(|s| **s == sub) {
        return Ok(found);
    }
    // Empty string prefix-matches every entry (tclsh: `pkgconfig ""` →
    // ambiguous subcommand).
    let matches: Vec<&str> = SUBS
        .iter()
        .copied()
        .filter(|s| s.starts_with(sub))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(false),
        _ => Err(true),
    }
}

/// Render a `TCL LOOKUP ...` errorCode as a list string so
/// empty/brace-needing elements render like tclsh (`subcommand {}`,
/// `CONFIG {a b}`).
fn pkgconfig_code(parts: &[&str]) -> String {
    Value::from_list(&parts.iter().map(|p| Value::from_str(p)).collect::<Vec<_>>())
        .as_str()
        .to_string()
}

/// Wrong-args failure for pkgconfig: message as decoded + `TCL WRONGARGS`.
fn pkgconfig_wrong(interp: &mut Interp, msg: String) -> Error {
    set_error_code(interp, "TCL WRONGARGS");
    tcl_err(msg)
}

// ── ::tcl::prefix（tclsh 8.6 ensemble，tclIndexObj.c）────────────────

fn prefix_usage_error(interp: &mut Interp, invoked: &str, usage: &str) -> Error {
    set_error_code(interp, "TCL WRONGARGS");
    tcl_err(format!("wrong # args: should be \"{} {}\"", invoked, usage))
}

fn prefix_sub_not_found(interp: &mut Interp, sub: &str) -> Error {
    set_error_code(interp, &format!("TCL LOOKUP SUBCOMMAND {}", sub));
    tcl_err(format!(
        "unknown or ambiguous subcommand \"{}\": must be all, longest, or match",
        sub
    ))
}

/// `::tcl::prefix all table string` — 以 string 为前缀的全部表元素。
fn prefix_all(interp: &mut Interp, invoked: &str, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(prefix_usage_error(interp, invoked, "table string"));
    }
    let table = args[0]
        .as_list()
        .ok_or_else(|| {
            set_error_code(interp, "TCL VALUE LIST");
            tcl_err(format!("invalid list \"{}\"", args[0].as_str()))
        })?;
    let s = args[1].as_str();
    let mut out = Vec::new();
    for t in &table {
        if t.as_str().starts_with(s) {
            out.push(t.clone());
        }
    }
    Ok(Value::from_list(&out))
}

/// `::tcl::prefix longest table string` — 匹配元素的最长公共前缀
/// （Tcl_UtfPrev 语义：按字符截断到首个分歧行之前）。
fn prefix_longest(interp: &mut Interp, invoked: &str, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(prefix_usage_error(interp, invoked, "table string"));
    }
    let table = args[0]
        .as_list()
        .ok_or_else(|| {
            set_error_code(interp, "TCL VALUE LIST");
            tcl_err(format!("invalid list \"{}\"", args[0].as_str()))
        })?;
    let s: Vec<char> = args[1].as_str().chars().collect();
    let mut best: Option<Vec<char>> = None;
    for t in &table {
        let tc: Vec<char> = t.as_str().chars().collect();
        if tc.len() < s.len() || tc[..s.len()] != s[..] {
            continue;
        }
        best = Some(match best {
            None => tc,
            Some(b) => {
                let n = b.len().min(tc.len());
                let mut k = 0;
                while k < n && b[k] == tc[k] {
                    k += 1;
                }
                b[..k].to_vec()
            }
        });
    }
    Ok(Value::from_str(
        &best.map(|b: Vec<char>| b.into_iter().collect::<String>())
            .unwrap_or_default(),
    ))
}

/// `::tcl::prefix match ?options? table string` — 唯一前缀匹配
/// （TclGetIndexFromObjList 面：-error/-exact/-message）。
/// Tcl's index lookup (Tcl_GetIndexFromObjStruct), as a function of its
/// inputs: an exact entry wins; a unique prefix is accepted unless
/// `exact`.  A miss comes back as the error tclsh raises — `bad`/`ambiguous
/// <message> "<word>": must be a, b, or c` (`no valid options` for an
/// empty table) — with the errorCode that goes with it
/// (`TCL LOOKUP INDEX <message> <word>`); the caller decides whether to
/// install that code.
pub(crate) fn index_in_table(
    word: &str,
    table: &[String],
    message: &str,
    exact: bool,
) -> std::result::Result<usize, (Error, String)> {
    if let Some(i) = table.iter().position(|t| t == word) {
        return Ok(i);
    }
    let hits: Vec<usize> = table
        .iter()
        .enumerate()
        .filter(|(_, t)| !exact && t.starts_with(word))
        .map(|(i, _)| i)
        .collect();
    if hits.len() == 1 {
        return Ok(hits[0]);
    }
    let kind = if hits.is_empty() { "bad" } else { "ambiguous" };
    let e = if table.is_empty() {
        tcl_err(format!(
            "{} {} \"{}\": no valid options",
            kind, message, word
        ))
    } else {
        let must = match table.len() {
            1 => table[0].clone(),
            2 => format!("{} or {}", table[0], table[1]),
            n => format!("{}, or {}", table[..n - 1].join(", "), table[n - 1]),
        };
        tcl_err(format!(
            "{} {} \"{}\": must be {}",
            kind, message, word, must
        ))
    };
    Err((e, format!("TCL LOOKUP INDEX {} {}", message, crate::value::tcl_quote(word))))
}

fn prefix_match(interp: &mut Interp, invoked: &str, args: &[Value]) -> Result<Value> {
    // tclsh PrefixMatchObjCmd: everything but the last two words is an
    // option, the last two are the table and the word.
    if args.len() < 2 {
        return Err(prefix_usage_error(interp, invoked, "?options? table string"));
    }
    const OPTIONS: [&str; 3] = ["-error", "-exact", "-message"];
    let option_table: Vec<String> = OPTIONS.iter().map(|o| o.to_string()).collect();
    let mut exact = false;
    let mut message = "option";
    let mut err_opts: Option<Vec<Value>> = None;
    let mut i = 0usize;
    while i + 2 < args.len() {
        // The option words go through the same lookup: `-ex` is -exact,
        // `-e` is ambiguous.
        let which = index_in_table(args[i].as_str(), &option_table, "option", false)
            .map_err(|(e, code)| {
                set_error_code(interp, &code);
                e
            })?;
        match OPTIONS[which] {
            "-exact" => exact = true,
            "-message" => {
                if i + 3 >= args.len() {
                    set_error_code(interp, "TCL OPERATION NOARG");
                    return Err(tcl_err("missing value for -message".to_string()));
                }
                i += 1;
                message = args[i].as_str();
            }
            _ => {
                if i + 3 >= args.len() {
                    set_error_code(interp, "TCL OPERATION NOARG");
                    return Err(tcl_err("missing value for -error".to_string()));
                }
                i += 1;
                let items = super::list::strict_list(interp, &args[i])?;
                if items.len() % 2 != 0 {
                    set_error_code(interp, "TCL VALUE DICTIONARY");
                    return Err(tcl_err(
                        "error options must have an even number of elements".to_string(),
                    ));
                }
                err_opts = Some(items);
            }
        }
        i += 1;
    }
    let table: Vec<String> = super::list::strict_list(interp, &args[i])?
        .iter()
        .map(|v| v.as_str().to_string())
        .collect();
    let word = args[i + 1].as_str();
    // A miss with `-error` to hand is not this command's error to raise:
    // its code stays as it was.
    let miss = match index_in_table(word, &table, message, exact) {
        Ok(idx) => return Ok(Value::from_str(&table[idx])),
        Err(m) => m,
    };
    match err_opts {
        None => {
            set_error_code(interp, &miss.1);
            Err(miss.0)
        }
        // `-error {}`: an empty result and ok.
        Some(items) if items.is_empty() => Ok(Value::empty()),
        // Otherwise the options are the completion's own, `-code` included:
        // the lookup failed, so the code is the error it reported.
        Some(mut items) => {
            items.push(Value::from_str("-code"));
            items.push(Value::from_int(1));
            let ret_args = [
                Value::from_str("return"),
                Value::from_str("-options"),
                Value::from_list(&items),
                Value::from_str(&miss.0.message_text()),
            ];
            super::control::cmd_return(interp, &ret_args).map(|_| Value::empty())
        }
    }
}

/// `::tcl::prefix` — tclsh 8.6 的 ::tcl 命名空间 ensemble（probe1 的
/// `::tcl::*` 面 + prefix 匹配面）。
pub fn cmd_tcl_prefix(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let invoked = args[0].as_str();
    if args.len() < 2 {
        return Err(prefix_usage_error(interp, invoked, "subcommand ?arg ...?"));
    }
    let sub = args[1].as_str().to_string();
    match sub.as_str() {
        "all" => prefix_all(interp, &format!("{} all", invoked), &args[2..]),
        "longest" => prefix_longest(interp, &format!("{} longest", invoked), &args[2..]),
        "match" => prefix_match(interp, &format!("{} match", invoked), &args[2..]),
        other => Err(prefix_sub_not_found(interp, other)),
    }
}

// ── ::tcl::Bgerror（tclEvent.c TclDefaultBgErrorHandlerObjCmd）────────

/// 后台错误默认处理器。rtcl 无事件泵后台错误队列——可调用面对齐：
/// 元数/options 校验、非 error 码的消息换形、-errorcode/-errorinfo
/// 副作用，bgerror 命令存在即调用；handler 缺失或失败时向 stderr
/// 报告并返回 ok。
pub fn cmd_tcl_bgerror(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let invoked = args[0].as_str();
    if args.len() != 3 {
        set_error_code(interp, "TCL WRONGARGS");
        return Err(tcl_err(format!(
            "wrong # args: should be \"{} msg options\"",
            invoked
        )));
    }
    let mut get_opt = |interp: &mut Interp, key: &str| -> Result<(Value, i64)> {
        let v = Value::from_str(args[2].as_str())
            .as_dict()
            .and_then(|d| d.get(key).cloned())
            .ok_or_else(|| {
                set_error_code(interp, "TCL ARGUMENT MISSING");
                tcl_err(format!("missing return option \"{}\"", key))
            })?;
        let n = v.as_int().ok_or_else(|| {
            set_error_code(interp, "TCL VALUE INTEGER");
            tcl_err(format!("expected integer but got \"{}\"", v.as_str()))
        })?;
        Ok((v, n))
    };
    let (_, level) = get_opt(interp, "-level")?;
    let (_, mut code) = get_opt(interp, "-code")?;
    if level != 0 {
        code = 2; // TCL_RETURN
    }
    if code == 0 {
        return Ok(Value::empty());
    }
    let msg = match code {
        1 => args[1].clone(),
        3 => Value::from_str("invoked \"break\" outside of a loop"),
        4 => Value::from_str("invoked \"continue\" outside of a loop"),
        n => Value::from_str(&format!("command returned bad code: {}", n)),
    };
    if let Some(ec) = Value::from_str(args[2].as_str())
        .as_dict()
        .and_then(|d| d.get("-errorcode").cloned())
    {
        let _ = interp.set_var("::errorCode", ec);
    }
    if let Some(ei) = Value::from_str(args[2].as_str())
        .as_dict()
        .and_then(|d| d.get("-errorinfo").cloned())
    {
        if !ei.as_str().is_empty() {
            let cur = interp
                .get_var("::errorInfo")
                .map(|v| v.as_str().to_string())
                .unwrap_or_default();
            let _ = interp.set_var(
                "::errorInfo",
                Value::from_str(&format!("{}\n{}", cur, ei.as_str())),
            );
        }
    }
    // The handler runs at the global level and may complete with any
    // code.  When it fails, tclsh reports on stderr and returns ok: with
    // no `bgerror` command at all, the restored ::errorInfo of the
    // original error; with one that failed, the three-line report below
    // (the handler's own error is what the interpreter is left holding).
    // rtcl has no init.tcl, so the missing-handler case is the common one
    // — it used to return silently, ::errorCode left naming the failure
    // of the lookup itself.
    // Read off the tables: a `get_var` of an unset variable raises, and
    // that would install its own errorCode.
    let saved_code = interp.global_get("errorCode").cloned();
    let saved_info = interp.global_get("errorInfo").cloned();
    let handler_args = [Value::from_str("bgerror"), msg.clone()];
    let r = interp.dispatch_values(&handler_args);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            if matches!(e, crate::error::Error::InvalidCommand { ref name } if name == "bgerror") {
                let info = saved_info
                    .as_ref()
                    .map(|v| v.as_str().to_string())
                    .unwrap_or_default();
                for (name, key, saved) in [
                    ("::errorCode", "errorCode", saved_code),
                    ("::errorInfo", "errorInfo", saved_info),
                ] {
                    match saved {
                        Some(v) => {
                            let _ = interp.set_var(name, v);
                        }
                        // Only what the failed lookup left behind: unsetting
                        // a variable that never existed is itself an error,
                        // and raises that errorCode.
                        None => {
                            if interp.global_has(key) {
                                let _ = interp.unset_var(name);
                            }
                        }
                    }
                }
                let _ = interp.console.write_stderr(&format!("{}\n", info));
            } else {
                let _ = interp.console.write_stderr(&format!(
                    "bgerror failed to handle background error.\n    Original error: {}\n    Error in bgerror: {}\n",
                    msg.as_str(),
                    e.message_text()
                ));
            }
        }
    }
    Ok(Value::empty())
}

#[cfg(test)]
#[path = "misc_tests.rs"]
mod tests;
