//! Procedure-related commands: proc, eval, uplevel, upvar, global, rename.

use crate::error::{Error, Result};
use crate::interp::{Interp, ProcDef, UpvarLink};
use crate::value::Value;

use super::super::Rc;

use super::list::{set_error_code, strict_list, tcl_get_int};

#[cfg(not(feature = "embedded"))]
use std::collections::HashMap;

#[cfg(feature = "embedded")]
use alloc::collections::BTreeMap as HashMap;

/// Parse a proc/lambda parameter list into (name, default) pairs.
/// Tcl validates the specifiers at definition time, per parameter, in
/// this order (probed on 8.6.17): too many fields → contains `::` →
/// contains `(` → empty (`a::b(1)` reports "not a simple name", not
/// "array element").
fn parse_param_specs(params: &[Value]) -> Result<Vec<(String, Option<String>)>> {
    let mut specs: Vec<(String, Option<String>)> = Vec::new();
    for param in params {
        let parts = param.as_list().unwrap_or_else(|| vec![param.clone()]);
        if parts.len() > 2 {
            return Err(Error::Msg(format!(
                "too many fields in argument specifier \"{}\"",
                param.as_str()
            )));
        }
        if !parts.is_empty() {
            let name = parts[0].as_str();
            if name.contains("::") {
                return Err(Error::Msg(format!(
                    "formal parameter \"{}\" is not a simple name",
                    name
                )));
            }
            if name.contains('(') {
                return Err(Error::Msg(format!(
                    "formal parameter \"{}\" is an array element",
                    name
                )));
            }
        }
        if parts.is_empty() {
            return Err(Error::Msg("argument with no name".to_string()));
        }
        if parts.len() == 2 {
            specs.push((
                parts[0].as_str().to_string(),
                Some(parts[1].as_str().to_string()),
            ));
        } else {
            specs.push((parts[0].as_str().to_string(), None));
        }
    }
    Ok(specs)
}

pub fn cmd_proc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // 3-arg form: proc name argList body
    // 4-arg form: proc name argList statics body  (jimtcl-compatible
    // extension kept for compatibility; the arity error still quotes
    // tclsh 8.6's standard usage — proc-old-5.1..5.3).
    if args.len() < 4 || args.len() > 5 {
        return Err(Error::wrong_args_with_usage(
            "proc", 4, args.len(), "name args body",
        ));
    }

    let raw_name = args[1].as_str();
    // Qualify the proc name if it names a namespace path or we're inside a
    // namespace context — `proc e1::cmd` at global and `namespace eval e1
    // {proc cmd}` must land on the SAME registered key (`::e1::cmd`,
    // tclsh), otherwise redefinition splits into two commands.
    let name = if raw_name.contains("::") || interp.current_namespace.as_ref() != "::" {
        super::namespace::qualify(&interp.current_namespace, raw_name)
    } else {
        raw_name.to_string()
    };

    let (param_arg, statics_arg, body_arg) = if args.len() == 5 {
        // 4-arg form: proc name argList statics body
        (&args[2], Some(&args[3]), &args[4])
    } else {
        // 3-arg form: proc name argList body
        (&args[2], None, &args[3])
    };

    // The argument list is split with Tcl_SplitList: a malformed list
    // (`proc t {a` …) errors at definition time with the list-parse
    // message and `TCL VALUE LIST …` errorCode (proc-old-5.4).
    let params = strict_list(interp, param_arg)?;
    let body = body_arg.as_str().to_string();

    // Definition-time parameter errors carry a `(creating proc "name")`
    // frame: pre-seed errorInfo so the harness logs the `proc` command
    // as `invoked from within` (tclsh ERR_ALREADY_LOGGED). Malformed
    // per-argument specifiers also raise
    // `TCL OPERATION PROC FORMALARGUMENTFORMAT` (proc-old-5.5..5.7).
    let defaults = match parse_param_specs(&params) {
        Ok(d) => d,
        Err(e) => {
            if let Error::Msg(_) = &e {
                set_error_code(interp, "TCL OPERATION PROC FORMALARGUMENTFORMAT");
            }
            interp.err_info = Some(format!(
                "{}\n    (creating proc \"{}\")",
                e,
                args[1].as_str()
            ));
            return Err(e);
        }
    };

    // Parse statics list: each element is {varName ?initialValue?}
    let mut statics = HashMap::new();
    if let Some(statics_val) = statics_arg {
        let static_list = statics_val.as_list().unwrap_or_default();
        for item in &static_list {
            let parts = item.as_list().unwrap_or_else(|| vec![item.clone()]);
            let var_name = parts[0].as_str().to_string();
            let init_val = if parts.len() >= 2 {
                Value::from_str(parts[1].as_str())
            } else {
                Value::empty()
            };
            statics.insert(var_name, init_val);
        }
    }

    // M3：先扫描当前遮蔽的 Tier1 折叠名再编译——epoch 戳防「编译后」
    // 遮蔽（bump 退役），既有遮蔽在此点名（分派活绑定）。
    let shadows = interp.shadowed_tier1_folds();
    let compiled =
        super::super::vm_exec::compile_proc_body(&defaults, &body, interp.tier1_epoch, &shadows);
    if let Some(code) = &compiled {
        interp.const_pool_insert(code);
        // M4「定义即编译」：JIT 引擎（若注册）就地发射 wasm 模块并缓存
        // （拒绝落负缓存）——首调用零编译延迟。门在钩子运行侧
        // （fallback/epoch/bytecode_applicable 与 Site A 同构）；无钩子
        // 注册时此处零开销（解释器构建零变化）。
        super::super::vm_exec::run_proc_precompile_hook(interp, code);
    }
    let proc_def = ProcDef {
        params: Rc::new(defaults),
        body: Rc::from(body),
        statics: Rc::new(statics),
        compiled,
    };

    // 自删体帧窗口的复活（basic-15.3 钉形；oracle n7/n8 实测）：窗口
    // 内重建的命令名若在 ns 自删时的命令名单上，整个 ns 以**新空表**
    // 归位（exists→1，旧命令不复返），本次定义落进复活后的 ns。名单
    // 外的新名字无此效果（tclsh 只活到体帧退出，ns_eval 出口清扫）。
    let resurrect_ns = interp
        .ns_resurrect_cmds
        .iter()
        .find(|(ns, names)| {
            (name.starts_with(&format!("{}::", ns.as_str())) || name.as_str() == ns.as_str())
                && names.contains(&name)
        })
        .map(|(ns, _)| ns.clone());
    if let Some(dead_ns) = resurrect_ns {
        interp.ns_resurrect_cmds.remove(&dead_ns);
        super::namespace::ensure_namespace(interp, &dead_ns);
    }

    // Redefining a command is a silent delete+create (tclsh 8.6.17): no
    // rename/delete trace fires and the old command's stored traces are
    // dropped (trace-19.5, trace-20.3.1).
    if interp.procs.contains_key(&name)
        || interp.commands.contains_key(&name)
        || interp.ensembles.contains_key(&name)
        || interp.import_aliases.contains_key(&name)
    {
        interp.wipe_cmd_exec_traces(&name);
        if let Some(old) = interp.procs.get(&name).cloned() {
            interp.purge_proc_artifacts(&old);
        }
    }
    // A proc named like an inline-folded Tier1 command (`proc set {...}`)
    // invalidates every compiled body that folded that command.
    super::super::vm_exec::note_tier1_mutation(interp, &name);
    interp.note_cmd_mutation();
    if interp.procs.insert(name.clone(), super::super::Rc::new(proc_def)).is_none() {
        interp.note_new_command(&name);
    }
    Ok(Value::empty())
}

pub fn cmd_eval(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args("eval", 2, args.len()));
    }

    if args.len() == 2 {
        eval_tagged(interp, args[1].as_str(), "eval")
    } else {
        let script: String = args[1..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ");
        eval_tagged(interp, &script, "eval")
    }
}

/// Evaluate a nested script for `eval`/`uplevel`: a body error gains the
/// construct's exit frame — `("eval" body line N)` (F3) / `("uplevel"
/// body line N)` (N1) — before the enclosing harness names the command.
fn eval_tagged(interp: &mut Interp, script: &str, tag: &str) -> Result<Value> {
    // Tcl_EvalEx parses one command at a time; an early command's output
    // survives a later syntax error.
    let r = interp.eval_incremental(script);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            if interp.err_info.is_none() {
                if let Error::Syntax { .. } = e {
                    // A parse failure inside eval/uplevel has no parsed
                    // command to name: tclsh logs the SCRIPT text itself
                    // as the failing "command", with TclMaxLogLength
                    // truncation (parseOld-10.14).
                    interp.err_info = Some(format!(
                        "{}\n    while executing\n\"{}\"",
                        e,
                        crate::interp::eval::tcl_log_excerpt(script)
                    ));
                }
            }
            interp.err_exit_frame(&format!("\"{}\" body", tag));
        }
    }
    r
}

/// apply lambdaExpr ?arg ...?
/// lambdaExpr is a two-element list: {params body}
pub fn cmd_apply(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "apply",
            2,
            args.len(),
            "lambdaExpr ?arg ...?",
        ));
    }

    let lambda = args[1].as_list().ok_or_else(|| {
        Error::runtime(
            format!(
                "can't interpret \"{}\" as a lambda expression",
                args[1].as_str()
            ),
            crate::error::ErrorCode::Generic,
        )
    })?;

    if lambda.len() < 2 || lambda.len() > 3 {
        return Err(Error::runtime(
            format!(
                "can't interpret \"{}\" as a lambda expression",
                args[1].as_str()
            ),
            crate::error::ErrorCode::Generic,
        ));
    }

    let param_list = lambda[0].as_list().unwrap_or_default();
    let body = lambda[1].as_str().to_string();

    // Optional third element: namespace in which the lambda body runs
    // (tclsh: `apply {{args} {body} ::ns}` — must already exist).
    let ns_override = if lambda.len() == 3 {
        let qualified =
            super::namespace::qualify(&interp.current_namespace, lambda[2].as_str());
        if !interp.namespaces.contains_key(&qualified) {
            super::list::set_error_code(
                interp,
                &format!("TCL LOOKUP NAMESPACE {}", crate::value::tcl_quote(&qualified)),
            );
            return Err(Error::runtime(
                format!("namespace \"{}\" not found", qualified),
                crate::error::ErrorCode::NotFound,
            ));
        }
        Some(Rc::from(qualified))
    } else {
        None
    };

    // Build param defaults (same logic as cmd_proc).  Parse errors carry
    // a `(parsing lambda expression "<term>")` frame between the message
    // and the caller's harness frame: pre-seed errorInfo so the harness
    // logs the command as `invoked from within` (tclsh ERR_ALREADY_LOGGED).
    let defaults = match parse_param_specs(&param_list) {
        Ok(d) => d,
        Err(e) => {
            interp.err_info = Some(format!(
                "{}\n    (parsing lambda expression \"{}\")",
                e,
                args[1].as_str()
            ));
            return Err(e);
        }
    };

    // tclsh compiles lambda bodies like proc bodies (a proc-context unit
    // with compiledLocals): inline foreach/lmap, frameless construct-body
    // errors, plain loop-variable writes.  Memoised by the term string —
    // the term IS the lambda's identity — and gated per call by the same
    // epoch/trace checks as proc bodies in `call_proc`.
    let compiled = match interp.lambda_code_cache.get(args[1].as_str()) {
        Some(c) => Some(Rc::clone(c)),
        None => {
            // M3 遮蔽感知折叠与命名 proc 同源（lambda 体同洞：定义后
            // epoch 戳只防新遮蔽，既有遮蔽在此点名）。
            let shadows = interp.shadowed_tier1_folds();
            let c = super::super::vm_exec::compile_proc_body(
                &defaults,
                &body,
                interp.tier1_epoch,
                &shadows,
            );
            if let Some(code) = &c {
                let cost = code.estimated_heap_size() + args[1].as_str().len() * 2 + 64;
                if interp.lambda_code_cache.len() >= 1024
                    || interp.lambda_code_cache_bytes + cost
                        > crate::interp::LAMBDA_CACHE_BYTE_BUDGET
                {
                    interp.lambda_code_cache.clear();
                    interp.lambda_code_cache_bytes = 0;
                }
                interp.const_pool_insert(code);
                interp.lambda_code_cache
                    .insert(args[1].as_str().to_string(), Rc::clone(code));
                interp.lambda_code_cache_bytes += cost;
            }
            c
        }
    };

    let proc_def = ProcDef {
        params: Rc::new(defaults),
        body: Rc::from(body),
        statics: Rc::new(HashMap::new()),
        compiled,
    };

    // Create args for call_proc: [name, arg1, arg2, ...]
    // The proc name renders in arity errors as "apply lambdaExpr" (Tcl-compatible).
    let mut call_args = vec![Value::from_str("apply")];
    for arg in &args[2..] {
        call_args.push(arg.clone());
    }

    // `info level 0` inside the lambda shows `apply {<term>} <args...>`:
    // the term as ONE list element (brace-wrapped) plus the arguments.
    // CALL 条目也取这一渲染（err_log_call 用帧 level0）。
    let mut level0 = vec![Value::from_str("apply"), args[1].clone()];
    level0.extend(args[2..].iter().cloned());
    interp.frame_level0_args = Some(level0);

    let r = interp.call_proc(&proc_def, &call_args, "apply lambdaExpr", ns_override);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            // F5: `(lambda term "{} {boomap}" line N)` — the lambda TERM's
            // value renders in the tag.  tclsh caps THIS frame at 60
            // chars (empirical: for-8.2/for-8.8 both truncate there —
            // a different budget than the 150-char log excerpt).
            let term = args[1].as_str();
            let term: std::borrow::Cow<str> = if term.chars().count() > 60 {
                let mut s: String = term.chars().take(60).collect();
                s.push_str("...");
                s.into()
            } else {
                term.into()
            };
            let tag = format!("lambda term \"{}\"", term);
            interp.err_exit_frame(&tag);
        }
    }
    r
}

/// A parsed `upvar`/`uplevel` level word.  `#N` is absolute (chain index
/// above the global scope), plain integers are relative to the innermost
/// scope.  The original word is kept for `bad level` messages.
enum LevelSpec {
    Rel(i64, String),
    Abs(usize, String),
}

/// A word is a level candidate iff its first char is `#`, `+`, `-` or a
/// digit (tclsh's TclGetLevelFromArg inspects the first byte only:
/// `uplevel .2 {}` runs `.2 {}` as the script).
fn level_shaped(w: &str) -> bool {
    matches!(
        w.as_bytes().first(),
        Some(b'#') | Some(b'+') | Some(b'-') | Some(b'0'..=b'9')
    )
}

/// The stricter shape `upvar` requires at the true global level: `#`
/// (with anything after it), a digit, or a sign followed by a digit.
/// A word that fails this is NOT a level there — `upvar a x y` and
/// `upvar - x y` fall back to pairs at the default level (bad level
/// "1"), while `upvar 0x x y` and `upvar 1.5 x y` error as-typed.
fn level_shaped_global(w: &str) -> bool {
    let b = w.as_bytes();
    match b.first() {
        Some(b'#') => true,
        Some(b'0'..=b'9') => true,
        Some(b'+') | Some(b'-') => matches!(b.get(1), Some(b'0'..=b'9')),
        _ => false,
    }
}

/// Parse a level word with Tcl's integer grammar (`#-1` and `#0.2` fail,
/// `#010` is octal 8, `+2` is relative 2).  Failure yields the word
/// verbatim for the error message.
fn parse_level(spec: &str) -> std::result::Result<LevelSpec, String> {
    if let Some(rest) = spec.strip_prefix('#') {
        return match tcl_get_int(rest) {
            Some(n) if n >= 0 => Ok(LevelSpec::Abs(n as usize, spec.to_string())),
            _ => Err(spec.to_string()),
        };
    }
    match tcl_get_int(spec) {
        Some(n) => Ok(LevelSpec::Rel(n, spec.to_string())),
        None => Err(spec.to_string()),
    }
}

/// Tcl's `bad level` error with its `TCL LOOKUP LEVEL <word>` errorCode.
fn bad_level(interp: &mut Interp, spec: &str) -> Error {
    set_error_code(interp, &format!("TCL LOOKUP LEVEL {}", spec));
    Error::Msg(format!("bad level \"{}\"", spec))
}

pub fn cmd_uplevel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "uplevel",
            2,
            args.len(),
            "?level? command ?arg ...?",
        ));
    }

    // The level word is present iff args[1] merely LOOKS like a level —
    // shape only, no arity check (`uplevel .2 {}` executes ".2 {}") —
    // AND parses as one: `0.2`/`1a` fail Tcl's integer grammar →
    // TclObjGetFrame result=0 → the word is a SCRIPT word (apply/proc
    // 内 `uplevel 0.2 {}` → `invalid command name "0.2"`，corpus
    // 4.29/4.31/4.35；global 的隐式层级 1 落到下方 m>scopes_total 的
    // bad level "1"，oracle u2 g-direct 同形)。`#0.2` 等 # 形失败仍
    // 是 as-typed 的 bad level（4.30/4.32/4.34）。
    let mut has_level = level_shaped(args[1].as_str());
    let spec = if has_level {
        match parse_level(args[1].as_str()) {
            Ok(s) => s,
            Err(w) if w.starts_with('#') => return Err(bad_level(interp, &w)),
            Err(_) => {
                has_level = false;
                LevelSpec::Rel(1, "1".to_string())
            }
        }
    } else {
        LevelSpec::Rel(1, "1".to_string())
    };
    // fast-tier 跨帧写哨兵：任何 upvar 链接（相对层级≥1 即跨帧；upvar 0
    // 是本帧别名，无跨帧写风险）。
    if !matches!(&spec, LevelSpec::Rel(0, _)) {
        crate::interp::jit_note_cross_frame();
    }
    let script_start = if has_level { 2usize } else { 1usize };

    // tclsh counts every varFrame — proc frames AND open `namespace
    // eval`s — as one level.  Rebuild that chain (bottom→top) from the
    // proc frames' `ns_depth` markers and the live ns-eval stack:
    // [ns evals below frame 0] frame 0 [ns evals between 0 and 1] …
    let scopes_total = interp.frames.len() + interp.ns_stack.len();
    let (num_to_pop_frames, ns_trim, target_ns, target_is_ns) = match &spec {
        LevelSpec::Abs(n, word) => {
            // `#N` names the absolute Nth scope above the global level
            // (#0 = the global scope itself); past the top: bad level.
            if *n == 0 {
                (interp.frames.len(), 0usize, "::".to_string(), false)
            } else if *n > scopes_total {
                return Err(bad_level(interp, word));
            } else {
                uplevel_resolve(interp, n - 1)
            }
        }
        LevelSpec::Rel(n, word) => {
            // Relative levels count OUT from the innermost scope; past
            // the top: bad level（引原词）。负层级：tclsh
            // TclObjGetFrame 整数形 w<0 → 直接 bad level（引**原词**
            // ——`-0xffffffff` 原样、`[expr -0xffffffff]` 的规范十进制
            // "-4294967295"；corpus 4.17/4.19/4.21/4.23）。
            if *n < 0 {
                return Err(bad_level(interp, word));
            }
            let m = *n as usize;
            if m > scopes_total {
                return Err(bad_level(interp, word));
            }
            let target = scopes_total as isize - m as isize - 1;
            if target < 0 {
                (interp.frames.len(), 0usize, "::".to_string(), false)
            } else {
                uplevel_resolve(interp, target as usize)
            }
        }
    };

    // Range resolution precedes the empty-command arity check
    // (`uplevel 9` at global → bad level, not wrong # args).
    if script_start >= args.len() {
        return Err(Error::wrong_args_with_usage(
            "uplevel",
            2,
            args.len(),
            "?level? command ?arg ...?",
        ));
    }

    let script = if args.len() - script_start == 1 {
        args[script_start].as_str().to_string()
    } else {
        args[script_start..]
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    };

    // Pop proc frames and trim ns evals down to the target scope, eval,
    // then restore everything.
    let split_point = interp.frames.len() - num_to_pop_frames;
    let saved_frames: Vec<_> = interp.frames.split_off(split_point);
    // 目标是 ns eval 帧：split_off 保留到目标本体（含）——目标条目
    // 存活（info level / info level 0 可见）；仅其上的 ns eval 随帧弹出。
    let ns_keep = ns_trim + usize::from(target_is_ns);
    let saved_ns_stack: Vec<_> = interp.ns_stack.split_off(ns_keep);
    let saved_l0: Vec<_> = interp.ns_level0.split_off(ns_keep);
    let saved_ns = interp.current_namespace.clone();
    interp.current_namespace = Rc::from(target_ns);
    // D-3 -errorstack：uplevel 求值期间的错误日志事件记
    // `UP <framePtr.level - varFramePtr.level>`（tclsh Tcl_LogCommandInfo
    // 的 varFramePtr 分支），delta = 弹出的作用域数（proc 帧 + ns eval
    // ——split_off 之前算，saved_ns_stack.len() 即被裁掉的数量）。
    let up_delta = num_to_pop_frames + saved_ns_stack.len();
    if up_delta > 0 {
        // (delta, 帧基)：UP 条目只在 frames.len() == 帧基（当前帧仍是
        // uplevel 帧）时记——uplevel 脚本里调用的 proc 帧记 CALL。
        interp.err_uplevel.push((up_delta, interp.frames.len()));
    }
    let result = eval_tagged(interp, &script, "uplevel");
    if up_delta > 0 {
        interp.err_uplevel.pop();
    }
    interp.current_namespace = saved_ns;
    interp.ns_level0.extend(saved_l0);
    interp.ns_stack.extend(saved_ns_stack);
    interp.frames.extend(saved_frames);
    result
}

/// Locate the scope at chain index `target` (bottom→top over interleaved
/// proc frames and open `namespace eval`s): returns the proc-frame split
/// point or the ns-eval stack position plus the scope's namespace.
fn uplevel_resolve(interp: &Interp, target: usize) -> (usize, usize, String, bool) {
    let mut idx = target;
    let mut split: Option<usize> = None;
    let mut ns_at: Option<usize> = None;
    for (i, f) in interp.frames.iter().enumerate() {
        let below = f.ns_depth.min(interp.ns_stack.len());
        if idx < below {
            // An ns eval below this frame
            ns_at = Some(idx);
            break;
        }
        idx -= below;
        if idx == 0 {
            split = Some(i);
            break;
        }
        idx -= 1;
    }
    if split.is_none() && ns_at.is_none() && idx < interp.ns_stack.len() {
        // The ns evals above the outermost frame (or all of them at
        // global level)
        ns_at = Some(idx);
    }
    match (split, ns_at) {
        (Some(fi), _) => {
            let depth = interp.frames[fi].ns_depth.min(interp.ns_stack.len());
            let ns = interp.frames[fi]
                .ns
                .as_deref()
                .unwrap_or("::")
                .to_string();
            // Frames strictly above the target pop; `uplevel 0` stays in
            // the running frame.
            (interp.frames.len() - fi - 1, depth, ns, false)
        }
        // 目标是 namespace eval 帧：该帧本身在 uplevel 求值期间保持
        // 存活（tclsh 的 uplevel 把目标 varFrame 置为当前帧——目标
        // 帧不销毁），ns_level0/ns_stack 的目标条目必须保留——`info
        // level` 计它一层、`info level 0` 报 namespace eval 调用词
        // （namespace_uplevel2 E、ei2 H 钉形）。
        (None, Some(k)) => (interp.frames.len(), k, interp.ns_stack[k].clone(), true),
        (None, None) => (interp.frames.len(), 0, "::".to_string(), false),
    }
}
pub fn cmd_upvar(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "?level? otherVar localVar ?otherVar localVar ...?"));
    }

    // Level-word detection: PARITY decides everywhere — an even argument
    // count nominates args[1] as the level, an odd count makes it the
    // first otherVar (`upvar 0 x` at global is the pair ("0", "x")).  A
    // nominated word parses STRICTLY inside any enclosing scope (`upvar
    // a b c` → bad level "a"); at the TRUE global level (no frames, no
    // ns evals) a word that doesn't look like a level falls back to
    // pairs at the default level instead (`upvar a x y` → bad level
    // "1", but `upvar 0x x y` → bad level "0x").
    let has_level = if args.len().is_multiple_of(2) {
        !interp.frames.is_empty()
            || !interp.ns_stack.is_empty()
            || level_shaped_global(args[1].as_str())
    } else {
        false
    };
    let spec = if has_level {
        match parse_level(args[1].as_str()) {
            Ok(s) => s,
            // `#0.2` 等 # 形失败 → as-typed bad level；非整数词
            // （1a/2.5/0.2/0x）→ TclObjGetFrame result=0：真全局（隐式
            // 层级 1 无效）→ bad level "1"（oracle upvar_levels
            // u12/u13/u15/u30）；帧内 → as-typed（corpus upvar-8.3
            // `proc p1 {upvar a b c}` → bad level "a"）。
            Err(w) if w.starts_with('#') => return Err(bad_level(interp, &w)),
            Err(_) if interp.frames.is_empty() && interp.ns_stack.is_empty() => {
                return Err(bad_level(interp, "1"));
            }
            Err(w) => return Err(bad_level(interp, &w)),
        }
    } else {
        LevelSpec::Rel(1, "1".to_string())
    };
    let start = if has_level { 2usize } else { 1usize };
    if start >= args.len() {
        return Err(Error::wrong_args_with_usage("upvar", 3, args.len(), "?level? otherVar localVar ?otherVar localVar ...?"));
    }

    let scopes_total = interp.frames.len() + interp.ns_stack.len();
    let scope: TargetScope = match &spec {
        LevelSpec::Abs(n, word) => {
            if *n == 0 {
                TargetScope::Global
            } else if *n > scopes_total {
                return Err(bad_level(interp, word));
            } else {
                TargetScope::Idx(n - 1)
            }
        }
        LevelSpec::Rel(n, word) => {
            // 负层级直接 bad level（引原词——tclsh TclObjGetFrame 整数
            // 形 w<0 即 -1；upvar_levels u16）。
            if *n < 0 {
                return Err(bad_level(interp, word));
            }
            let m = *n as usize;
            if m > scopes_total {
                return Err(bad_level(interp, word));
            }
            let t = scopes_total as isize - m as isize - 1;
            if t < 0 {
                TargetScope::Bottom
            } else {
                TargetScope::Idx(t as usize)
            }
        }
    };

    // At the global level (no call frames) upvar still links two
    // variables — the link is an eval-level flat alias (var-3.10:
    // `namespace eval {} { variable bar 0; namespace eval foo { upvar
    // bar bar } }` links foo::bar → bar).  The scope is the innermost one
    // without a proc variable frame — a body called from a proc too
    // (`var_frame_idx`).
    if interp.var_frame_idx().is_none() {
        // The scope the level names: an open body's namespace, the global
        // namespace — or a proc frame below the body.
        let mut proc_target = false;
        let target_ns = match scope {
            TargetScope::Global | TargetScope::Bottom => "::".to_string(),
            TargetScope::Idx(k) => match upvar_target(interp, k) {
                UpvarTarget::Frame(_) => {
                    proc_target = true;
                    "::".to_string()
                }
                UpvarTarget::GlobalNs => "::".to_string(),
                UpvarTarget::Ns(ns) => ns.to_string(),
            },
        };
        let mut i = start;
        while i + 1 < args.len() {
            let other_var = args[i].as_str();
            let local_var = args[i + 1].as_str();
            // A link made from a namespace table cannot point at a
            // procedure's variable: the variable dies with the proc's
            // frame and the link would dangle.  A `::`-qualified name
            // is a namespace's variable and links fine.
            if proc_target && !other_var.starts_with("::") {
                set_error_code(interp, "TCL UPVAR INVERTED");
                return Err(Error::runtime(
                    format!(
                        "bad variable name \"{}\": can't create namespace variable that refers to procedure variable",
                        local_var
                    ),
                    crate::error::ErrorCode::Generic,
                ));
            }
            let target_key = if target_ns == "::" {
                // 根 ns 目标：同帧路径的规范化 + 死父 ns 检查（宿主 ns
                // 按 tclsh 分段——尾 `::` 串 = 空名变量，宿主是 quals
                // 路径本身，旧 rsplit 对 `::t4:::` 算出 `::t4:`）。
                let key = other_var.strip_prefix("::").unwrap_or(other_var);
                let (quals, _) = crate::interp::commands::namespace::split_ns_tail(key);
                if !quals.is_empty() {
                    let parent = crate::interp::commands::namespace::normalise(quals);
                    if parent != "::" && !interp.namespaces.contains_key(&parent) {
                        set_error_code(
                            interp,
                            &format!("TCL LOOKUP VARNAME {}", other_var),
                        );
                        return Err(Error::runtime(
                            format!(
                                "can't access \"{}\": parent namespace doesn't exist",
                                other_var
                            ),
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                }
                key.to_string()
            } else {
                interp.canonical_global_in(&target_ns, other_var)
            };
            let alias_key = interp.canonical_global(local_var);
            // Same container creation as the frame path: an element link
            // makes the ARRAY exist (`upvar 0 a(e) x` → array exists a).
            if let Some(paren) = target_key.find('(') {
                let base = &target_key[..paren];
                if !interp.is_array_semantic(base) && !interp.var_exists(base) {
                    let _ = interp.mark_array(base);
                }
            }
            // Level 0 at the global level names the current scope
            // itself: `upvar 0 zz zz` is a self-reference (tclsh
            // 8.6.17).
            if target_key == alias_key {
                set_error_code(interp, "TCL UPVAR SELF");
                return Err(Error::Msg(
                    "can't upvar from variable to itself".to_string(),
                ));
            }
            // The link puts its target in the table, value or not: a
            // write to the bare name inside the body then finds it.
            if target_ns == "::" {
                interp.note_global_key(&target_key);
            }
            match interp
                .flat_aliases
                .iter_mut()
                .find(|(k, _)| *k == alias_key)
            {
                Some((_, t)) => *t = target_key,
                None => interp.flat_aliases.push((alias_key, target_key)),
            }
            // The top-level slot cache holds detached copies of the
            // unit's globals; an eval-level alias redirects reads and
            // writes of its name, which the slot ops bypass (`set g 0;
            // upvar 0 g q; set q 5` left g at 0). Flush what the slots
            // hold and drop them: nothing later in the unit uses a slot.
            interp.top_flush();
            interp.top_slots = None;
            i += 2;
        }
        return Ok(Value::empty());
    }

    let current_idx = interp.frames.len() - 1;

    // Map the scope to the upvar target over the tclsh varFrame chain
    // (proc frames + open namespace evals, one level each).
    let target: UpvarTarget = match scope {
        TargetScope::Global => UpvarTarget::GlobalNs,
        TargetScope::Bottom => {
            // Fell off every scope: the CALLER's namespace context
            // (var-15.1: `namespace eval test A ...` + `upvar $name`
            // lands in ::test, not the proc's own ns).
            let ns = interp.frames[current_idx]
                .call_ns
                .clone()
                .unwrap_or_else(|| Rc::clone(&interp.ns_root));
            UpvarTarget::Ns(ns)
        }
        TargetScope::Idx(k) => upvar_target(interp, k),
    };

    // Create upvar links
    let mut i = start;
    while i + 1 < args.len() {
        let other_var = args[i].as_str().to_string();
        let local_var = args[i + 1].as_str().to_string();

        // Resolve the named variable through any existing upvar links in
        // the target scope, so aliasing an alias lands on the ULTIMATE
        // target (upvar-4.2: p3 aliases p2's alias of p1's local;
        // upvar-8.5: the reverse link closes a cycle onto itself).
        let mut link = match &target {
            UpvarTarget::GlobalNs => {
                // 根 ns 目标键规范化：剥前导 ::——元素形式 a(e) 的下游
                // 写路径按无 :: 的全局键路由（深水探针 DIFF-6：
                // `upvar 0 ::a(e) x` 原落到 "::a(e)" 键，元素写丢失）。
                let key = other_var.strip_prefix("::").unwrap_or(&other_var);
                // 建链目标的宿主 ns 必须存在（tclsh：can't access
                // "::d::v": parent namespace doesn't exist——深水探针
                // DIFF-7：原对已删 ns 静默建链；宿主按 tclsh 分段，
                // 尾 `::` 串 = 空名变量、宿主是 quals 路径）。
                let (quals, _) = crate::interp::commands::namespace::split_ns_tail(key);
                if !quals.is_empty() {
                    let parent = crate::interp::commands::namespace::normalise(quals);
                    if parent != "::" && !interp.namespaces.contains_key(&parent) {
                        set_error_code(
                            interp,
                            &format!("TCL LOOKUP VARNAME {}", other_var),
                        );
                        return Err(Error::runtime(
                            format!(
                                "can't access \"{}\": parent namespace doesn't exist",
                                other_var
                            ),
                            crate::error::ErrorCode::Generic,
                        ));
                    }
                }
                UpvarLink::Global(Rc::from(key))
            }
            UpvarTarget::Ns(ns) => {
                UpvarLink::Global(Rc::from(interp.canonical_global_in(ns, &other_var)))
            }
            UpvarTarget::Frame(fi) => {
                // 帧目标的**根限定**名穿 ns 规则解析（tclsh level-N 帧
                // 的 ::x = 全局变量，不是帧局部字面名——深水探针
                // DIFF-6/7 公共根因：`upvar 0 ::d::v l` 原 Rel(0) 落
                // Frame{var_name: "::d::v"} 静默建帧局部，ns 检查与
                // 元素路由全被绕过）。
                match other_var.strip_prefix("::") {
                    Some(key) => {
                        let (quals, _) =
                            crate::interp::commands::namespace::split_ns_tail(key);
                        if !quals.is_empty() {
                            let parent =
                                crate::interp::commands::namespace::normalise(quals);
                            if parent != "::" && !interp.namespaces.contains_key(&parent) {
                                set_error_code(
                                    interp,
                                    &format!("TCL LOOKUP VARNAME {}", other_var),
                                );
                                return Err(Error::runtime(
                                    format!(
                                        "can't access \"{}\": parent namespace doesn't exist",
                                        other_var
                                    ),
                                    crate::error::ErrorCode::Generic,
                                ));
                            }
                        }
                        UpvarLink::Global(Rc::from(key))
                    }
                    None => UpvarLink::Frame {
                        frame_index: *fi,
                        var_name: other_var.clone(),
                    },
                }
            }
        };
        for _ in 0..=interp.frames.len() {
            let next = match &link {
                UpvarLink::Frame { frame_index, var_name } => interp
                    .frames
                    .get(*frame_index)
                    .and_then(|f| f.upvars.get(var_name.as_str()))
                    .cloned(),
                UpvarLink::Global(_) | UpvarLink::Dead { .. } => None,
            };
            match next {
                Some(l) => link = l,
                None => break,
            }
        }

        // A frame-target link must land in the target's name-keyed store
        // (slot cells and upvar links do not alias): degrade the target
        // frame when the linked name sits in its slot table.
        if let UpvarLink::Frame { frame_index, var_name } = &link {
            interp.degrade_frame_local(*frame_index, var_name);
            // tclsh ObjMakeUpvar 的 otherVar 帧内解析（createPart1/2=1）：
            // 元素形的基缺**帧局部**容器时建占位数组——容器存在、元素
            // 未定义（S2 钉形：`upvar 0 nosuch(1) foo` → array exists
            // nosuch=1、read foo=no such variable）；基是帧局部**标量**
            // 则建链即报 `can't access "x(1)": variable isn't array`
            // （S1 钉形）；全局同名不顶替（S3：链落帧局部占位）。宿主
            // unset 后别名转 dangling（deaden_element_links → Dead），
            // 写报 element in deleted array（var-1.17/trv_p11 ③④）。
                if let Some(paren) = var_name.find('(') {
                    let base = var_name[..paren].to_string();
                    if let Some(f) = interp.frames.get_mut(*frame_index) {
                        // D1 步骤 2：在席/数组基位走帧 vt 咽喉（debug 双
                        // 读对拍）。占位（flag-only 数组基）入日志——
                        // 日志 ⊇ vt 条目不变式恢复（array_elements 的
                        // 快径守卫 log.len == 活条目数在占位在场时不再
                        // 误判；重放的 in_table 过滤按有值域把占位滤
                        // 出——列举输出不变）。
                        if super::super::vars::frame_local_contains(f, &base)
                            && !super::super::vars::frame_local_is_array(f, &base)
                        {
                            set_error_code(interp, &format!("TCL LOOKUP VARNAME {}", base));
                            return Err(Error::runtime(
                                format!("can't access \"{}\": variable isn't array", other_var),
                                crate::error::ErrorCode::Generic,
                            ));
                        }
                        if !super::super::vars::frame_local_is_array(f, &base) {
                            super::super::vars::frame_local_mark_array(f, &base);
                            super::super::vars::frame_log_link(f, &base);
                        }
                    }
                }
        }

        // Self-reference, direct or through a link chain.
        if let UpvarLink::Frame { frame_index, var_name } = &link {
            if *frame_index == current_idx && *var_name == local_var {
                set_error_code(interp, "TCL UPVAR SELF");
                return Err(Error::Msg(
                    "can't upvar from variable to itself".to_string(),
                ));
            }
        }

        // A traced variable can't be turned into an alias (upvar-8.7).
        // Whole-var traces only: an element trace (`trace add variable
        // a(1) ...`) marks `a` as an existing array instead, so the
        // exists check below reports it (probed on 8.6.17).
        // D1 步骤 4：被追踪的帧局部 = 帧 vt 条目上的 TraceSet（注册面
        // 建 host——含摘净后的 lingering 空集，键形 contains_key 逐字同
        // ——「摘净后仍拦」是基线钉形）。
        if interp.frames[current_idx]
            .vt
            .get(local_var.as_str())
            .is_some_and(|v| v.trace_set().is_some_and(|s| s.var_linger))
        {
            set_error_code(interp, "TCL UPVAR TRACED");
            return Err(Error::Msg(format!(
                "variable \"{}\" has traces: can't use for upvar",
                local_var
            )));
        }

        // An existing local that is not itself an alias blocks the link;
        // replacing an existing alias is silent (upvar-6.1 re-links `x`
        // once per loop iteration).  Existence spans every store: the
        // name-keyed map, an array container, and a defined slot (a
        // `set x` before `global x` errors, tclsh TclPtrObjMakeUpvarIdx
        // "variable already exists" — an undefined slot links fine).
        // Checked BEFORE the link aliases the slot, so the error leaves
        // the variable as it was.  The alias name leaves the slot model
        // after the check (links and slot cells don't alias) —
        // link-only aliasing: the frame's other slots keep their
        // compiled fast path, and this frame's own reads/writes of the
        // alias re-resolve through the upvar guards.
        let has_link = interp.frames[current_idx].upvars.contains_key(local_var.as_str());
        if !has_link
            && (super::super::vars::frame_local_contains(&interp.frames[current_idx], &local_var)
                || super::super::vars::frame_local_is_array(&interp.frames[current_idx], &local_var)
                || interp.frames[current_idx].slot_value(&local_var).is_some())
        {
            set_error_code(interp, "TCL UPVAR EXISTS");
            return Err(Error::Msg(format!(
                "variable \"{}\" already exists",
                local_var
            )));
        }
        if let UpvarLink::Global(gname) = &link {
            // The global has an entry from now on, value or not.
            interp.note_global_key(gname);
        }
        interp.degrade_frame_link(current_idx, &local_var, &link);

        interp.frames[current_idx]
            .upvars
            .insert(Rc::from(local_var.as_str()), link.clone());
        // Linking to an ELEMENT of a missing array still creates the
        // array container (not the element) in tclsh (probed:
        // `upvar 0 a(e) x` → `array exists a` = 1, `info exists x` = 0).
        if let UpvarLink::Global(gname) = &link {
            if let Some(paren) = super::super::util::find_paren(gname) {
                let base = &gname[..paren];
                if !interp.is_array_semantic(base) && !interp.var_exists(base) {
                    let _ = interp.mark_array(base);
                }
            }
        }
        // The linked name is in the proc's variable table (tclsh `info
        // vars` lists it): mirror the current value of the ULTIMATE
        // target when one exists; reads/writes still go through the
        // link.
        let mirror: Option<Value> = match &link {
            // D1 步骤 1：读经 arena。
            UpvarLink::Global(gname) => interp.global_get(&**gname).cloned(),
            // D1 步骤 2：帧目标值读走 vt（frame_local_get）。
            UpvarLink::Frame { frame_index, var_name } => interp
                .frames
                .get(*frame_index)
                .and_then(|f| super::super::vars::frame_local_get(f, var_name.as_str()))
                .cloned(),
            UpvarLink::Dead { .. } => None,
        };
        if let Some(v) = mirror {
            super::super::vars::frame_local_insert(
                &mut interp.frames[current_idx],
                local_var.clone(),
                v,
            );
        } else {
            // D1 步骤 2：种子摘除走 vt（全量）。
            super::super::vars::frame_local_remove(&mut interp.frames[current_idx], &local_var);
            // Nothing to mirror, but the link is a frame-table entry all
            // the same (tclsh: `upvar #0 alsomissing y; info vars` lists
            // y): log its creation for the hash-order replay.
            super::super::vars::frame_log_link(&mut interp.frames[current_idx], &local_var);
        }
        i += 2;
    }

    Ok(Value::empty())
}

/// Which scope an `upvar` level names at the top of the chain.
enum TargetScope {
    /// Fell off every scope: the caller's namespace context inside a
    /// proc, the true global namespace at eval level.
    Bottom,
    /// The true global namespace (`upvar #0`).
    Global,
    /// A scope-chain index (bottom→top over frames + ns evals).
    Idx(usize),
}

/// Which scope an `upvar` level names.
enum UpvarTarget {
    /// The true global namespace (`upvar #0`, fell off everything).
    GlobalNs,
    /// An open `namespace eval`'s (or fallen-off caller's) variable table.
    Ns(Rc<str>),
    /// A proc frame by absolute index.
    Frame(usize),
}

/// Map a scope-chain index (bottom→top) to its upvar target.
fn upvar_target(interp: &Interp, target: usize) -> UpvarTarget {
    let mut idx = target;
    for (i, f) in interp.frames.iter().enumerate() {
        let below = f.ns_depth.min(interp.ns_stack.len());
        if idx < below {
            return UpvarTarget::Ns(Rc::from(interp.ns_stack[idx].as_str()));
        }
        idx -= below;
        if idx == 0 {
            return UpvarTarget::Frame(i);
        }
        idx -= 1;
    }
    if idx < interp.ns_stack.len() {
        UpvarTarget::Ns(Rc::from(interp.ns_stack[idx].as_str()))
    } else {
        UpvarTarget::GlobalNs
    }
}

pub fn cmd_global(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        // Bare `global` is a no-op (var-6.5, var-6.6).
        return Ok(Value::empty());
    }

    // At global level, global is a no-op — inside a `namespace eval`
    // body as well, called from a proc or not.
    if interp.var_frame_idx().is_none() {
        return Ok(Value::empty());
    }

    let current_idx = interp.frames.len() - 1;
    // fast-tier 哨兵：global 链接到全局帧（proc 深度>0 时必跨帧）。
    if interp.frames.len() > 1 {
        crate::interp::jit_note_cross_frame();
    }

    for arg in &args[1..] {
        let name = arg.as_str();
        // Every `global` name resolves against the TRUE global namespace:
        // `::`-qualified targets normalise exactly like `set` does (colon
        // runs collapse, a trailing run survives — var-6.3's
        // `global ::test_ns_var::test_ns_nested::` links the empty-named
        // variable), plain names are verbatim.
        let target =
            crate::interp::Interp::global_key(name).unwrap_or_else(|| name.to_string());
        // The local alias is the tail after the last `::` run
        // (`global a::b` binds `b` — tclsh), `ns::` binds the empty name.
        let local = super::namespace::split_var_tail(name).to_string();
        // The alias name leaves the slot model (links and slot cells
        // don't alias); link-only aliasing keeps the frame's other slots
        // on their compiled fast path — the frame's own ops on the name
        // re-resolve through the upvar guards, the way tclsh's linked
        // compiledLocal does.
        let glink = UpvarLink::Global(Rc::from(target.as_str()));
        // A defined local (map entry, array container, or a defined
        // slot — the compiler registers `global` names as locals, so a
        // preceding `set x` lands in the slot) blocks the link, exactly
        // like `upvar` (tclsh TclPtrObjMakeUpvarIdx "variable already
        // exists").  Checked before the alias touches the slot.
        let has_link = interp.frames[current_idx].upvars.contains_key(local.as_str());
        if !has_link
            && (super::super::vars::frame_local_contains(&interp.frames[current_idx], &local)
                || super::super::vars::frame_local_is_array(&interp.frames[current_idx], &local)
                || interp.frames[current_idx].slot_value(&local).is_some())
        {
            set_error_code(interp, "TCL UPVAR EXISTS");
            return Err(Error::Msg(format!(
                "variable \"{}\" already exists",
                local
            )));
        }
        // The global has an entry from now on, value or not.
        interp.note_global_key(&target);
        interp.degrade_frame_link(current_idx, &local, &glink);
        interp.frames[current_idx].upvars.insert(Rc::from(local.as_str()), glink);
        // The linked name lives in the proc's variable table (tclsh's
        // `info vars` lists it) — mirror the current value when one
        // exists; reads/writes still go through the link.  With no
        // value to mirror the link is still a table entry: log its
        // creation so the hash-order replay lists it (tclsh:
        // `global nosuch; info vars` lists nosuch).
        // D1 步骤 1：读经 arena。
        match interp.global_get(&target) {
            Some(v) => {
                let v = v.clone();
                super::super::vars::frame_local_insert(
                    &mut interp.frames[current_idx],
                    local.clone(),
                    v,
                );
            }
            None => {
                super::super::vars::frame_log_link(&mut interp.frames[current_idx], &local);
            }
        }
    }

    Ok(Value::empty())
}

pub fn cmd_rename(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args_with_usage("rename", 3, args.len(), "oldName newName"));
    }

    let old_name = args[1].as_str().to_string();
    let new_name = args[2].as_str().to_string();

    // Command names resolve through the same fallback chain dispatch uses:
    // exact, namespace-qualified, then `::`-prefixed (tclsh: `rename
    // nb::pp nb::qq` finds the proc registered as `::nb::pp`).
    let old_key = resolve_command_key(interp, &old_name).ok_or_else(|| {
        // tclsh: can't rename "name": command doesn't exist
        super::list::set_error_code(
            interp,
            &format!("TCL LOOKUP COMMAND {}", old_name),
        );
        Error::runtime(
            format!("can't rename \"{}\": command doesn't exist", old_name),
            crate::error::ErrorCode::NotFound,
        )
    })?;
    let new_key = if new_name.is_empty() {
        String::new()
    } else if new_name.contains("::") || interp.current_namespace.as_ref() != "::" {
        // Namespace-qualified targets store fully-qualified, exactly like
        // `proc` definition does (basic-18.6: `rename q test_ns_basic::p`
        // must land on the key the ns-first dispatch — and the namespace
        // deletion sweep — looks up: ::test_ns_basic::p).
        super::namespace::qualify(&interp.current_namespace, &new_name)
    } else {
        new_name.clone()
    };

    // Renaming into a namespace creates it, including intermediates
    // (tclsh probed: `rename q deep::nest::p` → `namespace exists
    // deep::nest` = 1).
    if !new_key.is_empty() && new_key.contains("::") {
        let cut = new_key.rfind("::").unwrap();
        let ns = if cut == 0 { "::" } else { &new_key[..cut] };
        interp.ensure_ns_path(ns);
    }

    // A rename touching a Tier1-named command — either end — invalidates
    // compiled bodies (the old binding's fold may now be shadowed, or a
    // shadow just went away; the bump only costs those bodies the VM).
    super::super::vm_exec::note_tier1_mutation(interp, &old_key);
    if !new_key.is_empty() {
        super::super::vm_exec::note_tier1_mutation(interp, &new_key);
    }
    // The rename moves entries across every resolution table (commands,
    // procs, ensembles, import aliases) — age the resolution cache out.
    interp.note_cmd_mutation();

    // OO objects/classes（D-42①②）：命令注册在 builtins 表，但 oo 侧
    // 状态（objects/classes/creation_order/引用边）必须随键迁移——空
    // 新名 = destroy（析构器照跑）。必须在 builtins 通用路径之前拦截。
    let oo_old = if old_key.starts_with("::") {
        old_key.clone()
    } else {
        format!("::{}", old_key)
    };
    if interp.oo.objects.contains_key(&oo_old) || interp.oo.classes.contains_key(&oo_old) {
        super::oo::oo_rename(interp, &old_key, &new_key)?;
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Coroutine records（内存审计 2026-11-20 A7 面）：resume 命令与
    // 挂起 record 同生命周期——tclsh 语义：rename away = 协程删除
    // （record/帧/冻结链释放，深度单元归还）；rename 新名 = record
    // 随键平移（新名可续，tclsh 实测钉形）。此前两条路径都不触碰
    // `interp.coroutines`——挂起 record 成孤儿（可脚本化无界增长，
    // 帧链+冻结字节码滞留）且新名 resume 失效。必须在 builtins 通用
    // 路径之前拦截（resume 命令注册在 commands 表）。
    let is_coro_resume = interp
        .commands
        .get(&old_key)
        .map(|f| *f == super::coro::cmd_coro_resume)
        .unwrap_or(false);
    if is_coro_resume {
        let record_key = [
            old_key.clone(),
            format!("::{}", old_key.trim_start_matches("::")),
            super::namespace::qualify(&interp.current_namespace, &old_name),
        ]
        .into_iter()
        .find(|k| interp.coroutines.contains_key(k));
        if let Some(record_key) = record_key {
            if new_key.is_empty() {
                super::coro::destroy_record(interp, &record_key);
            } else {
                let new_record_key =
                    super::namespace::qualify(&interp.current_namespace, &new_name);
                if let Some(rec) = interp.coroutines.remove(&record_key) {
                    interp.coroutines.insert(new_record_key.clone(), rec);
                }
                if let Some(func) = interp.commands.remove(&old_key) {
                    if interp.commands.insert(new_key.clone(), func).is_none() {
                        interp.note_new_command(&new_key);
                    }
                }
            }
            finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
            return Ok(Value::empty());
        }
    }

    // Rename in builtins
    if let Some(func) = interp.commands.remove(&old_key) {
        // An interpreter's command and an alias's record belong to the
        // command, not to its name (tclsh): they move with it — or, for an
        // alias, the rename is refused when it would close a loop.
        if let Err(e) = super::interp_cmd::rename_bound(interp, &old_key, &new_key, func) {
            interp.commands.insert(old_key.clone(), func);
            return Err(e);
        }
        let cat = interp.command_categories.remove(&old_key);
        let meta = interp.command_meta.remove(&old_key);
        if !new_key.is_empty() {
            if interp.commands.insert(new_key.clone(), func).is_none() {
                interp.note_new_command(&new_key);
            }
            if let Some(c) = cat {
                interp.command_categories.insert(new_key.clone(), c);
            }
            if let Some(m) = meta {
                interp.command_meta.insert(new_key.clone(), m);
            }
        }
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Rename in procs
    if let Some(proc_def) = interp.procs.remove(&old_key) {
        if !new_key.is_empty() {
            if interp.procs.insert(new_key.clone(), proc_def).is_none() {
                interp.note_new_command(&new_key);
            }
        } else {
            interp.purge_proc_artifacts(&proc_def);
        }
        move_alias_origins(interp, &old_key, &new_key);
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Rename a namespace ensemble command
    if let Some(def) = interp.ensembles.remove(&old_key) {
        if !new_key.is_empty() {
            if interp.ensembles.insert(new_key.clone(), def).is_none() {
                interp.note_new_command(&new_key);
            }
        }
        move_alias_origins(interp, &old_key, &new_key);
        finish_cmd_rename(interp, &old_key, &new_key, &old_name, &new_name);
        return Ok(Value::empty());
    }

    // Rename an import alias: the alias entry itself moves (the origin
    // keeps its key; alias-table keys are always fully qualified).
    if let Some(origin) = interp.import_aliases.remove(&old_key) {
        if !new_key.is_empty() {
            let new_alias_key = if new_key.starts_with("::") {
                new_key.clone()
            } else {
                format!("::{}", new_key)
            };
            if interp.import_aliases.insert(new_alias_key.clone(), origin).is_none() {
                interp.note_new_command(&new_alias_key);
            }
            // Aliases imported *from* this alias track the origin command,
            // not its name, so they follow the rename.
            let moved: Vec<String> = interp
                .import_aliases
                .iter()
                .filter(|(_, o)| o.as_str() == old_key)
                .map(|(a, _)| a.clone())
                .collect();
            for dep in moved {
                if interp.import_aliases.insert(dep.clone(), new_alias_key.clone()).is_none() {
                    interp.note_new_command(&dep);
                }
            }
        } else {
            // Deleting the alias deletes its dependent re-imports too
            // (namespace-old-9.18).
            move_alias_origins(interp, &old_key, "");
        }
        return Ok(Value::empty());
    }

    // tclsh: can't rename "name": command doesn't exist
    super::list::set_error_code(
        interp,
        &format!("TCL LOOKUP COMMAND {}", crate::value::tcl_quote(&old_name)),
    );
    Err(Error::runtime(
        format!("can't rename \"{}\": command doesn't exist", old_name),
        crate::error::ErrorCode::NotFound,
    ))
}

/// Post-rename bookkeeping shared by the builtin and proc paths: move
/// command traces to the new key, then fire rename/delete traces
/// (tclsh fires them after the operation; callback errors are
/// background errors and never fail the rename).
fn finish_cmd_rename(
    interp: &mut Interp,
    old_key: &str,
    new_key: &str,
    old_name: &str,
    new_name: &str,
) {
    if new_key.is_empty() {
        interp.fire_cmd_traces(old_key, old_name, "", "delete");
    } else {
        interp.rekey_cmd_traces(old_key, new_key);
        interp.fire_cmd_traces(new_key, old_name, new_name, "rename");
    }
}

/// Import aliases follow renames of their origin (tclsh 48.2: renaming
/// `foo::bar` keeps the imported `bar` dispatching).
pub(crate) fn move_alias_origins(interp: &mut Interp, old_key: &str, new_key: &str) {
    if old_key == new_key {
        return;
    }
    if new_key.is_empty() {
        // Deleting the origin deletes the imported alias with it (48.2:
        // `rename foo::bar2 {}` → exist spong = 0) — and transitively,
        // aliases imported *from* that alias (namespace-old-9.18:
        // renaming `test_ns_import::cmd1` {} removes the global `::cmd1`
        // and `test_ns_import_use::cmd1` re-imports of it).
        let mut removed_keys: Vec<String> = vec![old_key.to_string()];
        loop {
            let doomed: Vec<String> = interp
                .import_aliases
                .iter()
                .filter(|(_, origin)| removed_keys.iter().any(|k| origin == &k))
                .map(|(a, _)| a.clone())
                .collect();
            if doomed.is_empty() {
                break;
            }
            for alias in &doomed {
                interp.import_aliases.remove(alias);
                removed_keys.push(alias.clone());
            }
        }
    } else {
        let moved: Vec<String> = interp
            .import_aliases
            .iter()
            .filter(|(_, origin)| origin.as_str() == old_key)
            .map(|(a, _)| a.clone())
            .collect();
        for alias in moved {
            if interp.import_aliases.insert(alias.clone(), new_key.to_string()).is_none() {
                interp.note_new_command(&alias);
            }
        }
    }
    // An ensemble renaming onto/away from an alias-resolved key is handled
    // by find_ensemble_key's alias chain at dispatch time.
}

/// Resolve a command name to its registered key, mirroring dispatch's
/// fallback chain (namespace-qualified → exact → `::`-prefixed).
pub(crate) fn resolve_command_key(interp: &Interp, name: &str) -> Option<String> {
    // A simple name inside a namespace resolves there first — the order
    // of `dispatch_values_inner`. Probing the exact key first found the
    // bare-keyed global command: inside `namespace eval c` with both `h`
    // and `c::h`, `rename h {}` removed the global one where tclsh
    // removes `c::h` (and a `tailcall h` reached the global body).
    let simple_in_ns = interp.current_namespace.as_ref() != "::"
        && !name.starts_with("::")
        && !name.contains("::");
    if simple_in_ns {
        let qualified = super::namespace::qualify(&interp.current_namespace, name);
        if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
            return Some(qualified);
        }
    }
    if interp.procs.contains_key(name) || interp.commands.contains_key(name) {
        return Some(name.to_string());
    }
    if interp.current_namespace.as_ref() != "::" && !name.starts_with("::") {
        let qualified =
            super::namespace::qualify(&interp.current_namespace, name);
        if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
            return Some(qualified);
        }
    }
    if !name.starts_with("::") && name.contains("::") {
        let qualified = format!("::{}", name);
        if interp.procs.contains_key(&qualified) || interp.commands.contains_key(&qualified) {
            return Some(qualified);
        }
    }
    // Colon runs collapse in command names: `p1:::g` names `::p1::g`.
    if name.contains("::") {
        let norm = super::namespace::normalise(name);
        if norm != name
            && (interp.procs.contains_key(&norm) || interp.commands.contains_key(&norm))
        {
            return Some(norm);
        }
    }
    // A `::`-qualified simple name in the global namespace: builtins and
    // global procs are keyed unqualified (`rename ::unknown ...`, 52.6).
    if name.starts_with("::") && !name[2..].contains("::") {
        let bare = &name[2..];
        if interp.procs.contains_key(bare)
            || interp.commands.contains_key(bare)
            || interp.ensembles.contains_key(bare)
        {
            return Some(bare.to_string());
        }
    }
    // Namespace ensembles resolve like commands for rename — same order.
    let mut keys: Vec<String> = if simple_in_ns {
        vec![
            super::namespace::qualify(&interp.current_namespace, name),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };
    if !name.starts_with("::") {
        // A relative name from the global level names `::name`
        // (ensembles created with `-command ::foo`, renamed as `foo`).
        keys.push(format!("::{}", name));
    }
    if let Some(k) = keys.into_iter().find(|k| interp.ensembles.contains_key(k)) {
        return Some(k);
    }
    // Import aliases are real commands: renaming one moves the alias
    // itself, not the origin (48.2).
    let alias_key = if name.starts_with("::") {
        name.to_string()
    } else if interp.current_namespace.as_ref() != "::" {
        super::namespace::qualify(&interp.current_namespace, name)
    } else {
        format!("::{}", name)
    };
    if interp.import_aliases.contains_key(&alias_key) {
        return Some(alias_key);
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_proc_statics_counter() {
        let mut interp = Interp::new();
        interp.eval("proc counter {} {{count 0}} { incr count; return $count }").unwrap();
        assert_eq!(interp.eval("counter").unwrap().as_str(), "1");
        assert_eq!(interp.eval("counter").unwrap().as_str(), "2");
        assert_eq!(interp.eval("counter").unwrap().as_str(), "3");
    }

    #[test]
    fn test_proc_statics_multiple() {
        let mut interp = Interp::new();
        interp.eval("proc accum {val} {{sum 0} {n 0}} { incr n; set sum [expr {$sum + $val}]; return \"$sum $n\" }").unwrap();
        assert_eq!(interp.eval("accum 10").unwrap().as_str(), "10 1");
        assert_eq!(interp.eval("accum 20").unwrap().as_str(), "30 2");
        assert_eq!(interp.eval("accum 5").unwrap().as_str(), "35 3");
    }

    #[test]
    fn test_proc_statics_default_empty() {
        let mut interp = Interp::new();
        // Static with no initial value defaults to empty string
        interp.eval("proc setter {} {x} { if {$x eq {}} { set x hello }; return $x }").unwrap();
        assert_eq!(interp.eval("setter").unwrap().as_str(), "hello");
        assert_eq!(interp.eval("setter").unwrap().as_str(), "hello");
    }

    #[test]
    fn test_proc_statics_persists_across_calls() {
        let mut interp = Interp::new();
        interp.eval("proc tracker {} {{items {}}} { append items x; return $items }").unwrap();
        assert_eq!(interp.eval("tracker").unwrap().as_str(), "x");
        assert_eq!(interp.eval("tracker").unwrap().as_str(), "xx");
        assert_eq!(interp.eval("tracker").unwrap().as_str(), "xxx");
    }

    #[test]
    fn test_proc_statics_with_args() {
        let mut interp = Interp::new();
        interp.eval("proc add_to {val} {{total 0}} { set total [expr {$total + $val}]; return $total }").unwrap();
        assert_eq!(interp.eval("add_to 5").unwrap().as_str(), "5");
        assert_eq!(interp.eval("add_to 3").unwrap().as_str(), "8");
        assert_eq!(interp.eval("add_to 2").unwrap().as_str(), "10");
    }

    #[test]
    fn test_proc_no_statics_unchanged() {
        // Standard 3-arg proc still works
        let mut interp = Interp::new();
        interp.eval("proc double {x} { expr {$x * 2} }").unwrap();
        assert_eq!(interp.eval("double 5").unwrap().as_str(), "10");
    }

    #[test]
    fn test_info_statics_shows_values() {
        let mut interp = Interp::new();
        interp.eval("proc counter {} {{count 0}} { incr count; return $count }").unwrap();
        interp.eval("counter").unwrap();
        interp.eval("counter").unwrap();
        let r = interp.eval("info statics counter").unwrap();
        assert_eq!(r.as_str(), "count 2");
    }

    #[test]
    fn test_args_variable_roundtrips_quote_and_brace_words() {
        // tclsh：`$args` 持有规范 Tcl list，元素按 TclScanElement 规则
        // 加引号后可被 lassign/lindex 无损重解析。此前手写 join 只有
        // 空白词加 {}，行级 JSON 一类词即碎。
        let mut interp = Interp::new();
        interp
            .eval(r#"proc peek {sub args} { lassign $args path value; list [llength $args] $sub $path $value }"#)
            .unwrap();
        assert_eq!(
            interp
                .eval(r#"peek set /pods/42 {"name":"pod-42","status":"Running"}"#)
                .unwrap()
                .as_str(),
            r#"2 set /pods/42 {"name":"pod-42","status":"Running"}"#
        );
        // display 形式与 tclsh 一致：[list a {…}] 规范化保留引号包装，
        // `$args` 同一形式（可再往返）。注意 args 不含已绑定的 sub。
        let d = interp
            .eval(r#"proc disp {sub args} { return $args }; disp set /pods/42 {"name":"x","y"}"#)
            .unwrap();
        assert_eq!(d.as_str(), r#"/pods/42 {"name":"x","y"}"#);
        assert_eq!(
            interp
                .eval(r#"set d [list a {"name":"x","y"}]; set d"#)
                .unwrap()
                .as_str(),
            r#"a {"name":"x","y"}"#
        );
    }
}

#[cfg(test)]
mod apply_rename_tests {
    use crate::interp::Interp;

    // -- apply lambdaExpr shapes (tclsh 8.6.17) --

    #[test]
    fn test_apply_non_lambda_message() {
        let mut interp = Interp::new();
        let e = interp.eval("apply xx").unwrap_err().to_string();
        assert_eq!(e, "can't interpret \"xx\" as a lambda expression");
    }

    #[test]
    fn test_apply_one_element_lambda_message() {
        let mut interp = Interp::new();
        let e = interp.eval("apply {x}").unwrap_err().to_string();
        assert_eq!(e, "can't interpret \"x\" as a lambda expression");
    }

    #[test]
    fn test_apply_four_element_lambda_message() {
        let mut interp = Interp::new();
        let e = interp
            .eval(r#"apply {{a} {set a} ::NN extra} 1"#)
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "can't interpret \"{a} {set a} ::NN extra\" as a lambda expression"
        );
    }

    // -- apply namespace element --

    #[test]
    fn test_apply_runs_in_named_namespace() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("namespace eval NN10 {}; apply {{} {namespace current} ::NN10}")
                .unwrap()
                .as_str(),
            "::NN10"
        );
        // relative name qualifies against the current namespace
        assert_eq!(
            interp
                .eval("namespace eval NN13 {}; apply {{} {namespace current} NN13}")
                .unwrap()
                .as_str(),
            "::NN13"
        );
    }

    #[test]
    fn test_apply_ns_resolves_procs_of_that_ns() {
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("namespace eval NN11 {proc helper {} {return H}}; apply {{} {helper} ::NN11}")
                .unwrap()
                .as_str(),
            "H"
        );
    }

    #[test]
    fn test_apply_ns_must_exist() {
        let mut interp = Interp::new();
        let e = interp
            .eval(r#"apply [list x {set x 1} ::NONEXIST::FOR::SURE] x"#)
            .unwrap_err()
            .to_string();
        assert_eq!(e, "namespace \"::NONEXIST::FOR::SURE\" not found");
        let e2 = interp
            .eval("apply {{a} {set a} NOPE} 1")
            .unwrap_err()
            .to_string();
        assert_eq!(e2, "namespace \"::NOPE\" not found");
    }

    // -- rename of namespace-qualified procs --

    #[test]
    fn test_rename_qualified_proc() {
        // tclsh: rename test_ns_basic::p test_ns_basic::q works
        let mut interp = Interp::new();
        assert_eq!(
            interp
                .eval("namespace eval nb {proc pp {} {return 1}}; rename nb::pp nb::qq; nb::qq")
                .unwrap()
                .as_str(),
            "1"
        );
    }

    #[test]
    fn test_rename_qualified_delete() {
        let mut interp = Interp::new();
        let r = interp
            .eval("namespace eval nb2 {proc pp {} {return 1}}; rename nb2::pp {}; info procs nb2::*")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(r, "");
    }

    #[test]
    fn test_rename_missing_message() {
        // tclsh: can't rename "name": command doesn't exist
        let mut interp = Interp::new();
        let e = interp
            .eval("catch {rename nosuch::cmd other} m; set m")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(e, "can't rename \"nosuch::cmd\": command doesn't exist");
    }
}
