//! Control flow commands: if, switch, break, continue, return, exit,
//! catch, error, try, tailcall.

use crate::error::{Error, Result};
use crate::interp::Interp;
use crate::value::Value;

use super::list::set_error_code;

/// Evaluate an `if` arm body script.  tclsh compiles these constructs
/// inline: a body error propagates framelessly (F2/N8 — no frame names the
/// `if` command itself), so mark the next harness append suppressed.
///
/// The body also gets cmd_while's line rebasing: tclsh compiles against one
/// absolute line table, so a body command's `(procedure ... line N)` reports
/// its line in the enclosing script — the arm sits on the `if` command's
/// line — not within the body text (`if {0} {…} else { nosuch }` on the
/// proc body's second line reports line 2, not 1).  A body from a variable
/// keeps the unshifted numbering (origin unknowable).
///
/// `same_unit`: the compiler inlined this `if` (all arms verbatim; see
/// [`if_inline_body_indices`]) — the arm body is part of the proc's unit,
/// so its script keeps the lexical (compiled-context) evaluation, and a
/// foreach dispatched from inside it takes the compiled semantics exactly
/// as it would inside the unit the compiler built.
fn eval_arm_body(
    interp: &mut Interp,
    word_idx: usize,
    script: &str,
    same_unit: bool,
) -> Result<Value> {
    let saved_offset = interp.line_offset;
    let offset = match interp.body_is_verbatim_script(word_idx, script) {
        Some(extra) => Some(saved_offset + interp.cur_cmd_line.max(1) - 1 + extra),
        None => None,
    };
    if let Some(off) = offset {
        interp.line_offset = off;
    }
    if same_unit {
        interp.next_eval_lexical = true;
    }
    let r = interp.eval(script);
    interp.line_offset = saved_offset;
    if let Err(e) = &r {
        // Only a true error participates in harness-frame suppression —
        // a control-flow completion (break/continue/return) never reaches
        // an append, and arming `fresh` for it would leak into the NEXT
        // error's framing, swallowing one of its harness frames
        // (`if {...; continue}` followed by an unrelated error).
        if interp.err_is_error(e) {
            interp.err_fresh = true;
        }
    }
    r
}

/// Value-level mirror of `Compiler::compile_if`'s structural walk: the
/// word indices of every body this `if` could run, `Some` only when the
/// word sequence matches the grammar the compiler inlines — any
/// deviation (dangling keyword, non-keyword word after a body, trailing
/// words after `else`, non-literal keyword words) means the compiler
/// dispatches the real `if`, so the tree side must not preserve the unit
/// context through the bodies either (a foreach inside them stays
/// dispatched in BOTH engines).
fn if_inline_body_indices(interp: &Interp, args: &[Value]) -> Option<Vec<usize>> {
    let is_kw = |i: usize, kw: &str| {
        args.get(i).is_some_and(|v| v.as_str() == kw) && interp.word_is_plain_literal(i, kw)
    };
    let n = args.len();
    let mut idxs = Vec::new();
    let mut i = 2usize;
    loop {
        if i >= n {
            return None; // dangling keyword / no body
        }
        i += 1; // past the condition
        if is_kw(i, "then") {
            i += 1;
        }
        if i >= n {
            return None; // condition without body
        }
        idxs.push(i);
        i += 1;
        if i >= n {
            break;
        }
        if is_kw(i, "elseif") {
            i += 1;
        } else if is_kw(i, "else") {
            i += 1;
            if i + 1 != n {
                return None; // missing body / trailing words
            }
            idxs.push(i);
            break;
        } else {
            return None; // implicit-arm shape: the compiler DynCalls
        }
    }
    Some(idxs)
}

pub fn cmd_if(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_msg(
            "wrong # args: no expression after \"if\" argument",
        ));
    }

    // The compiler's all-or-nothing inline decision, evaluated before any
    // arm runs: every body of a structurally inlinable `if` is part of
    // the proc's unit, so all of them keep the lexical context.
    let same_unit = interp.lexical_body
        && if_inline_body_indices(interp, args).is_some_and(|idxs| {
            idxs.iter().all(|&bi| interp.body_would_compile(bi, args[bi].as_str()))
        });

    let expr = args[1].as_str();
    let cond = interp.eval_expr(expr)?;

    // Skip optional "then" keyword after the condition
    let mut i = 2;
    if i < args.len() && args[i].as_str() == "then" {
        i += 1;
    }

    if i >= args.len() {
        return Err(Error::wrong_args_msg(format!(
            "wrong # args: no script following \"{}\" argument",
            args[i - 1].as_str()
        )));
    }

    if crate::types::expr_funcs::strict_bool(&cond)? {
        return eval_arm_body(interp, i, args[i].as_str(), same_unit);
    }
    i += 1;

    while i < args.len() {
        let word = args[i].as_str();
        match word {
            "elseif" => {
                i += 1;
                if i >= args.len() {
                    return Err(Error::wrong_args_msg(
                        "wrong # args: no expression after \"elseif\" argument",
                    ));
                }
                let expr = args[i].as_str();
                let cond = interp.eval_expr(expr)?;
                i += 1;
                // Skip optional "then" keyword
                if i < args.len() && args[i].as_str() == "then" {
                    i += 1;
                }
                if i >= args.len() {
                    return Err(Error::wrong_args_msg(format!(
                        "wrong # args: no script following \"{}\" argument",
                        args[i - 1].as_str()
                    )));
                }
                if crate::types::expr_funcs::strict_bool(&cond)? {
                    return eval_arm_body(interp, i, args[i].as_str(), same_unit);
                }
                i += 1;
            }
            "else" => {
                if i + 1 >= args.len() {
                    return Err(Error::wrong_args_msg(
                        "wrong # args: no script following \"else\" argument",
                    ));
                }
                if i + 1 != args.len() - 1 {
                    return Err(Error::wrong_args_msg(
                        "wrong # args: extra words after \"else\" clause in \"if\" command",
                    ));
                }
                return eval_arm_body(interp, i + 1, args[i + 1].as_str(), same_unit);
            }
            _ => {
                return eval_arm_body(interp, i, word, same_unit);
            }
        }
    }

    Ok(Value::empty())
}

pub fn cmd_switch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    const SWITCH_USAGE: &str = "?-option ...? string ?pattern body ...? ?default body?";
    let objc = args.len();

    #[derive(PartialEq, Clone, Copy)]
    enum MatchMode { Exact, Glob, Regexp }

    // tclsh parses options only while two arguments (string + at least one
    // pattern) remain — `switch -- x` never consumes the "--", so the string
    // is "--" and the lone "x" is an odd list element.
    const OPTIONS: [&str; 7] = [
        "-exact", "-glob", "-indexvar", "-matchvar", "-nocase", "-regexp", "--",
    ];
    let mut mode = MatchMode::Exact;
    let mut found_mode = false;
    let mut nocase = false;
    let mut matchvar: Option<String> = None;
    let mut indexvar: Option<String> = None;

    let bad_option = |opt: &str, msg: &str| {
        Error::runtime(
            format!("bad option \"{}\": {}", opt, msg),
            crate::error::ErrorCode::Generic,
        )
    };

    let mut i = 1usize;
    while i + 2 < objc {
        if !args[i].as_str().starts_with('-') {
            break;
        }
        // Tcl_GetIndexFromObj: unique case-sensitive prefix match.
        let word = args[i].as_str();
        let hits: Vec<&str> = OPTIONS
            .iter()
            .copied()
            .filter(|o| o.starts_with(word))
            .collect();
        let opt = match hits.as_slice() {
            [one] => *one,
            [] => {
                return Err(bad_option(
                    word,
                    "must be -exact, -glob, -indexvar, -matchvar, -nocase, -regexp, or --",
                ));
            }
            _ => {
                return Err(bad_option(
                    word,
                    "must be -exact, -glob, -indexvar, -matchvar, -nocase, -regexp, or --",
                ));
            }
        };
        match opt {
            "-exact" | "-glob" | "-regexp" => {
                if found_mode {
                    return Err(bad_option(
                        word,
                        &format!(
                            "{} option already found",
                            match mode {
                                MatchMode::Exact => "-exact",
                                MatchMode::Glob => "-glob",
                                MatchMode::Regexp => "-regexp",
                            }
                        ),
                    ));
                }
                found_mode = true;
                mode = match opt {
                    "-glob" => MatchMode::Glob,
                    "-regexp" => MatchMode::Regexp,
                    _ => MatchMode::Exact,
                };
            }
            "-nocase" => nocase = true,
            "-indexvar" | "-matchvar" => {
                i += 1;
                if i + 2 >= objc {
                    return Err(Error::runtime(
                        format!(
                            "missing variable name argument to {} option",
                            opt
                        ),
                        crate::error::ErrorCode::Generic,
                    ));
                }
                if opt == "-indexvar" {
                    indexvar = Some(args[i].as_str().to_string());
                } else {
                    matchvar = Some(args[i].as_str().to_string());
                }
            }
            _ => {
                // "--": skip and stop option parsing.
                i += 1;
                break;
            }
        }
        i += 1;
    }

    if objc < i + 2 {
        return Err(Error::wrong_args_with_usage(
            "switch", 2, objc, SWITCH_USAGE,
        ));
    }
    if (matchvar.is_some() || indexvar.is_some()) && mode != MatchMode::Regexp {
        let which = if matchvar.is_some() { "-matchvar" } else { "-indexvar" };
        return Err(Error::runtime(
            format!("{} option requires -regexp option", which),
            crate::error::ErrorCode::Generic,
        ));
    }

    let string = args[i].as_str();
    i += 1;

    let patterns: Vec<(String, String)> = if objc - i == 1 {
        let list = args[i].as_list().unwrap_or_default();
        if list.is_empty() {
            return Err(Error::wrong_args_msg(
                "wrong # args: should be \"switch ?-option ...? string {?pattern body ...? ?default body?}\"",
            ));
        }
        if list.len() % 2 != 0 {
            let pats: Vec<String> = list.iter().map(|v| v.as_str().to_string()).collect();
            return Err(extra_pattern_error(true, &pats));
        }
        list.chunks(2)
            .map(|c| (c[0].as_str().to_string(), c[1].as_str().to_string()))
            .collect()
    } else {
        if (objc - i) % 2 != 0 {
            let pats: Vec<String> = args[i..].iter().map(|v| v.as_str().to_string()).collect();
            return Err(extra_pattern_error(false, &pats));
        }
        args[i..]
            .chunks(2)
            .map(|c| (c[0].as_str().to_string(), c[1].as_str().to_string()))
            .collect()
    };

    // Pre-validation: a trailing "-" body means its pattern has no body at
    // all — tclsh reports this BEFORE any matching happens
    // (switch-7.3: {a - b -foo c -} names "c", never reaching "-foo").
    if patterns.last().map(|p| p.1.as_str()) == Some("-") {
        let name = patterns[patterns.len() - 1].0.clone();
        return Err(Error::runtime(
            format!("no body specified for pattern \"{}\"", name),
            crate::error::ErrorCode::Generic,
        ));
    }

    let fold = |s: &str| if nocase { s.to_ascii_lowercase() } else { s.to_string() };
    let haystack = fold(string);
    let n = patterns.len();

    for pi in 0..n {
        let (pattern, body) = &patterns[pi];

        // Only the LAST pair, spelled exactly "default" (case-sensitive,
        // even under -nocase), is the fallback arm. Earlier "default"
        // words are ordinary patterns — in exact mode the string
        // "default" matches one literally (switch-1.6).
        if pi == n - 1 && pattern == "default" {
            // TIP #75: reaching the default arm with -matchvar/-indexvar
            // set stores empty lists (switch-11.4: x becomes {}).
            if let Some(name) = &indexvar {
                interp.set_var(name, Value::empty())?;
            }
            if let Some(name) = &matchvar {
                interp.set_var(name, Value::empty())?;
            }
            return eval_switch_body(interp, pattern, body);
        }

        let matched = match mode {
            MatchMode::Exact => haystack == fold(pattern),
            MatchMode::Glob => super::super::glob_match(&fold(pattern), &haystack),
            MatchMode::Regexp => {
                #[cfg(feature = "regexp")]
                {
                    let pat = if nocase {
                        format!("(?i){}", pattern)
                    } else {
                        pattern.clone()
                    };
                    regex::Regex::new(&pat)
                        .map(|re| re.is_match(&haystack))
                        .unwrap_or(false)
                }
                #[cfg(not(feature = "regexp"))]
                {
                    let _ = nocase;
                    return Err(Error::runtime(
                        "switch -regexp requires 'regexp' feature",
                        crate::error::ErrorCode::Generic,
                    ));
                }
            }
        };

        if matched {
            if mode == MatchMode::Regexp {
                set_regexp_match_vars(interp, &matchvar, &indexvar, pattern, &haystack, nocase)?;
            }
            // A "-" body falls through: take the next body position that is
            // not exactly "-", skipping over intervening patterns without
            // examining them (switch-7.1).
            let mut j = pi;
            while patterns[j].1 == "-" {
                j += 1;
            }
            return eval_switch_body(interp, &patterns[j].0, &patterns[j].1);
        }
    }

    Ok(Value::empty())
}

/// Evaluate a matched `switch` arm body.  A body error gains the
/// `("<pattern>" arm line N)` construct-exit frame, then the switch
/// command's own harness frame appends normally (probed on tclsh
/// 8.6.17, switch-4.1/4.5).
fn eval_switch_body(interp: &mut Interp, pattern: &str, body: &str) -> Result<Value> {
    let r = interp.eval(body);
    if let Err(e) = &r {
        if interp.err_is_error(e) {
            interp.err_exit_frame(&format!("\"{}\" arm", pattern));
        }
    }
    r
}

/// Odd arm count error; when the arms were split from a single list
/// argument and a pattern position starts with '#', tclsh appends the
/// comment heuristic.
fn extra_pattern_error(_split: bool, _patterns: &[String]) -> Error {
    let _ = _split;
    let mut msg = "extra switch pattern with no body".to_string();
    if _split {
        for k in (0.._patterns.len()).step_by(2) {
            if _patterns[k].starts_with('#') {
                msg.push_str(
                    ", this may be due to a comment incorrectly placed outside of a switch body - see the \"switch\" documentation",
                );
                break;
            }
        }
    }
    Error::runtime(msg, crate::error::ErrorCode::InvalidOp)
}

/// Store -matchvar/-indexvar results for a -regexp arm: the match variable
/// gets the list of the full match plus each capture group; the index
/// variable gets one {start end} pair (inclusive character indices) per
/// group.
#[cfg(feature = "regexp")]
fn set_regexp_match_vars(
    interp: &mut Interp,
    matchvar: &Option<String>,
    indexvar: &Option<String>,
    pattern: &str,
    haystack: &str,
    nocase: bool,
) -> Result<()> {
    if matchvar.is_none() && indexvar.is_none() {
        return Ok(());
    }
    let pat = if nocase {
        format!("(?i){}", pattern)
    } else {
        pattern.to_string()
    };
    let re = match regex::Regex::new(&pat) {
        Ok(re) => re,
        Err(_) => return Ok(()),
    };
    if let Some(caps) = re.captures(haystack) {
        // tclsh writes the indices list first, then the matches list — a
        // failing second write leaves the first one in place (switch-13.6).
        if let Some(name) = indexvar {
            let items: Vec<Value> = (0..caps.len())
                .map(|g| match caps.get(g) {
                    Some(m) => Value::from_str(&format!("{} {}", m.start(), m.end() - 1)),
                    None => Value::from_str("-1 -1"),
                })
                .collect();
            interp.set_var(name, Value::from_list(&items))?;
        }
        if let Some(name) = matchvar {
            let items: Vec<Value> = (0..caps.len())
                .map(|g| Value::from_str(caps.get(g).map(|m| m.as_str()).unwrap_or("")))
                .collect();
            interp.set_var(name, Value::from_list(&items))?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "regexp"))]
fn set_regexp_match_vars(
    _interp: &mut Interp,
    _matchvar: &Option<String>,
    _indexvar: &Option<String>,
    _pattern: &str,
    _haystack: &str,
    _nocase: bool,
) -> Result<()> {
    Ok(())
}

pub fn cmd_break(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Tcl 8.6 `break` takes no arguments — any argument is an arity
    // error (`break foo` / `break 2` → wrong # args, for-3.1).
    if args.len() != 1 {
        return Err(Error::wrong_args_with_usage("break", 1, args.len(), ""));
    }
    Err(Error::brk())
}

pub fn cmd_continue(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Tcl 8.6 `continue` takes no arguments (for-2.1).
    if args.len() != 1 {
        return Err(Error::wrong_args_with_usage("continue", 1, args.len(), ""));
    }
    Err(Error::cont())
}

pub fn cmd_return(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Tcl 8.6 Tcl_ReturnObjCmd: "An even number of words means an explicit
    // result argument is present" — objc counts the `return` word itself,
    // and the leading option words are consumed as key/value pairs.
    // UNKNOWN KEYS ARE IGNORED (tclsh stores them in the return-options
    // dict where nothing reads them back), so `return -badOption foo
    // message` completes with a plain TCL_RETURN carrying "message"
    // (proc-old-7.15).  Only the values of recognised options are
    // validated (TclMergeReturnOptions).
    let objc = args.len();
    let explicit_result = objc % 2 == 0;
    let num_option_words = objc - 1 - usize::from(explicit_result);
    let result = if explicit_result {
        Some(args[objc - 1].clone())
    } else {
        None
    };

    // Options are merged in word order — a later pair overwrites an
    // earlier one, and `-options {dict}` expands in place (recursively).
    let mut opts: Vec<(String, Value)> = Vec::new();
    {
        fn expand(words: &[Value], opts: &mut Vec<(String, Value)>) -> Result<()> {
            let mut i = 0;
            // Callers pass even-length word lists (numOptionWords, or a
            // validated dictionary), so each iteration consumes a pair.
            while i < words.len() {
                let key = words[i].as_str().to_string();
                if key == "-options" {
                    // The value must be a dictionary (even-length list).
                    let dict = match words[i + 1].as_list_strict() {
                        Ok(d) if d.len() % 2 == 0 => d,
                        _ => {
                            return Err(Error::runtime(
                                format!(
                                    "expected dict but got \"{}\"",
                                    words[i + 1].as_str()
                                ),
                                crate::error::ErrorCode::Generic,
                            ));
                        }
                    };
                    expand(&dict, opts)?;
                } else {
                    let val = words[i + 1].clone();
                    if let Some(slot) = opts.iter_mut().find(|(k, _)| *k == key) {
                        slot.1 = val;
                    } else {
                        opts.push((key, val));
                    }
                }
                i += 2;
            }
            Ok(())
        }
        expand(&args[1..1 + num_option_words], &mut opts).map_err(|e| {
            // A malformed -options value is a RESULT-family error
            // (tclsh: `return -options {-code} x` → TCL RESULT
            // ILLEGAL_OPTIONS).
            set_error_code(_interp, "TCL RESULT ILLEGAL_OPTIONS");
            e
        })?;
    }

    let get = |name: &str| -> Option<&Value> {
        opts.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    };

    let mut code: Option<i32> = None;
    if let Some(v) = get("-code") {
        let c = v.as_str();
        code = Some(match c {
            "ok" => 0,
            "error" => 1,
            "return" => 2,
            "break" => 3,
            "continue" => 4,
            // Integers go through Tcl's integer parser (hex `0x2` and
            // 64-bit words accepted) and truncate to the C int the
            // completion code travels in (`3000000000` → -1294967296);
            // anything else is a TCL RESULT ILLEGAL_CODE error.
            _ => match super::list::tcl_get_int(c) {
                Some(v) => v as i32,
                None => {
                    set_error_code(_interp, "TCL RESULT ILLEGAL_CODE");
                    return Err(Error::runtime(
                        format!("bad completion code \"{}\": must be ok, error, return, break, continue, or an integer", c),
                        crate::error::ErrorCode::Generic,
                    ));
                }
            },
        });
    }

    let mut level: i32 = 1; // default level
    let mut level_given = false;
    if let Some(v) = get("-level") {
        level = v.as_str().parse::<i32>().map_err(|_| {
            set_error_code(
                _interp,
                "TCL RESULT ILLEGAL_LEVEL",
            );
            Error::runtime(
                format!(
                    "bad -level value: expected non-negative integer but got \"{}\"",
                    v.as_str()
                ),
                crate::error::ErrorCode::Generic,
            )
        })?;
        if level < 0 {
            set_error_code(_interp, "TCL RESULT ILLEGAL_LEVEL");
            return Err(Error::runtime(
                format!(
                    "bad -level value: expected non-negative integer but got \"{}\"",
                    v.as_str()
                ),
                crate::error::ErrorCode::Generic,
            ));
        }
        level_given = true;
    }

    let mut error_info: Option<String> = None;
    if let Some(v) = get("-errorinfo") {
        error_info = Some(v.as_str().to_string());
    }

    let mut error_code: Option<String> = None;
    if let Some(v) = get("-errorcode") {
        // tclsh validates the value as a list before installing it
        // as errorCode (result-6.3).
        if v.as_list_strict().is_err() {
            set_error_code(_interp, "TCL RESULT ILLEGAL_ERRORCODE");
            return Err(Error::runtime(
                format!(
                    "bad -errorcode value: expected a list but got \"{}\"",
                    v.as_str()
                ),
                crate::error::ErrorCode::Generic,
            ));
        }
        error_code = Some(v.as_str().to_string());
    }

    // `return -level 0 ?value?` with completion code ok is an ordinary
    // command completion (tclsh: TclProcessReturn at level 0 / code TCL_OK
    // just stores the value as the *command's* result — the current script
    // keeps running; probed: trailing commands still execute, `catch` sees
    // code 0).  -errorinfo/-errorcode are inert at code ok.  Error /
    // break / continue codes with `-level 0` still fire below.
    if level == 0 && code.unwrap_or(0) == 0 {
        return Ok(result.unwrap_or_default());
    }

    match code {
        Some(c) => {
            // `-level 0` fires the completion right here (tclsh:
            // TclProcessReturn at level 0 materializes the code
            // immediately — probed: `catch {return -level 0 -code
            // error x}` reports 1, a `-code break` in a loop body ends
            // the loop; the SAME codes without -level 0 stay deferred
            // Return completions that only convert at proc boundaries).
            // `-code return` and integer user codes pass through
            // unchanged (catch reports 2 / the code).
            if level == 0 {
                return Err(match c {
                    1 => Error::ControlFlow {
                        kind: crate::error::ControlFlow::Error,
                        value: result,
                        level: 1,
                        error_info,
                        error_code,
                    },
                    3 => Error::brk(),
                    4 => Error::cont(),
                    _ => Error::return_with_options(c, result, error_info, error_code),
                });
            }
            Err(Error::return_with_options(c, result, error_info, error_code))
        }
        None => {
            if error_info.is_some() || error_code.is_some() {
                Err(Error::return_with_options(0, result, error_info, error_code))
            } else if level_given {
                // Explicit -level (>= 1 here): encoded as -(N+1).
                Err(Error::ret_level(level, result))
            } else {
                Err(Error::ret(result))
            }
        }
    }
}

pub fn cmd_exit(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 2 {
        return Err(Error::wrong_args_with_usage("exit", 1, args.len(), "?returnCode?"));
    }
    let code = if args.len() > 1 {
        args[1].as_int().ok_or_else(|| {
            Error::runtime(
                format!("expected integer but got \"{}\"", args[1].as_str()),
                crate::error::ErrorCode::Generic,
            )
        })? as i32
    } else {
        0
    };
    Err(Error::exit(Some(code)))
}

pub fn cmd_catch(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "catch",
            2,
            args.len(),
            "script ?resultVarName? ?optionVarName?",
        ));
    }

    let script = args[1].as_str();
    let result_var = if args.len() > 2 { Some(args[2].as_str()) } else { None };
    let opts_var = if args.len() > 3 { Some(args[3].as_str()) } else { None };

    // tclsh compiles a braced catch body INLINE into the enclosing unit
    // when that unit is a proc context (compiledLocals): the body then
    // runs with compiled/lexical semantics — construct bodies frameless,
    // loop-variable writes plain.  Everywhere else (top level, eval'd
    // strings, uplevel) the body is its own dispatched unit with the
    // decorated shapes.  Mirror the split: `lexical_body` is the
    // tree-walk's compiled-context flag, `in_locals_unit` the compiled
    // executor's — either way a braces-only verbatim body runs lexically.
    let same_unit = (interp.lexical_body || interp.in_locals_unit)
        && interp.body_would_compile(1, script);
    let caught = if same_unit {
        interp.eval_lexical_script(script)
    } else {
        interp.eval(script)
    };
    match caught {
        Ok(v) => {
            if let Some(var) = result_var {
                interp.set_var(var, v)?;
            }
            if let Some(ov) = opts_var {
                interp.set_var(ov, return_options_dict(0, 0))?;
            }
            Ok(Value::from_int(0))
        }
        Err(e) => {
            // A `tailcall` completion caught here: the deferred command
            // stays armed on the frame marker and fires at frame exit;
            // the completion itself reports as a `return` (code 2) with
            // an empty value (tailcall-12.3b1).
            let is_tc = e.is_tail_call();
            if let Some(var) = result_var {
                // Tcl: the result variable receives the result payload itself,
                // not a rendering of the error (e.g. `return hi` → "hi",
                // `break` → "").
                let payload = if is_tc {
                    Value::empty()
                } else {
                    Value::from_str(&e.message_text())
                };
                interp.set_var(var, payload)?;
            }
            let code = if is_tc { 2 }
            else if e.is_return() {
                // A user integer completion code (`return -code 7`) passes
                // through as the catch code (tclsh); the standard codes
                // stay deferred completions reporting 2 until a proc
                // boundary materializes them.
                if let Error::ControlFlow { level, .. } = &e {
                    if *level > 4 { *level as i64 } else { 2 }
                } else { 2 }
            }
            else if e.is_break() { 3 }
            else if e.is_continue() { 4 }
            else { 1 };
            // Tcl writes ::errorInfo (the frames accumulated while the
            // error propagated from its raise site to this catch) and
            // ::errorCode as soon as the error is caught.
            let accumulated = if interp.err_is_error(&e) {
                interp.err_info.take()
            } else {
                None
            };
            interp.err_fresh = false;
            // The catch consumed the propagation — no deferred
            // enclosing-command frame can reach the top-level report.
            interp.err_pending_top = None;
            // Only an ERROR completion touches ::errorInfo — a caught
            // break/continue/plain-return leaves whatever is in there
            // alone (tclsh: catching `return -code break x` does not
            // clobber a previously accumulated errorInfo).
            let error_info = accumulated.unwrap_or_else(|| e.message_text());
            if interp.err_is_error(&e) {
                let _ = interp.set_var("::errorInfo", Value::from_str(&error_info));
            }
            // ::errorCode: a raise site that installed it (scan formats,
            // exec CHILDSTATUS, `error`, `return -errorcode`) wins;
            // otherwise the error variant derives it (TCL WRONGARGS,
            // ARITH DIVZERO) and plain errors reset the var to NONE —
            // tclsh resets the code for every new error (rename-3.1) and
            // the var always exists after an error (fresh interp:
            // `info exists errorCode` → 0 until then).
            if code == 1 {
                if interp.err_code_raised {
                    interp.err_code_raised = false;
                } else {
                    // Derived codes: tclsh stamps a code on every error —
                    // wrong-args (TCL WRONGARGS), arithmetic (ARITH ...),
                    // var lookups (TCL LOOKUP/READ VARNAME), unknown
                    // commands (TCL LOOKUP COMMAND); plain errors reset
                    // the var to NONE.
                    let tec = e.tcl_error_code();
                    let _ = interp.set_var("::errorCode", Value::from_str(&tec));
                }
            }
            if let Some(ov) = opts_var {
                let level = if e.is_return() { 0 } else { 1 };
                let opts = if code == 1 {
                    // Tcl error completions carry -errorcode and -errorinfo.
                    error_options_dict(1, 0, &e, script, &error_info)
                } else {
                    return_options_dict(code, level)
                };
                interp.set_var(ov, opts)?;
            }
            Ok(Value::from_int(code))
        }
    }
}

/// Minimal Tcl return-options dict: `-code` and `-level` are the keys
/// real scripts consult (e.g. `dict get $opts -code`).
fn return_options_dict(code: i64, level: i64) -> Value {
    Value::from_str(&format!("-code {} -level {}", code, level))
}

/// Tcl return-options dict for an error completion: adds `-errorcode`
/// and `-errorinfo` next to `-code`/`-level`.
fn error_options_dict(code: i64, level: i64, err: &Error, script: &str, accumulated: &str) -> Value {
    let error_code = err.tcl_error_code();
    Value::from_str(&format!(
        "-code {} -level {} -errorcode {} -errorinfo {}",
        code,
        level,
        crate::value::tcl_quote(&error_code),
        crate::value::tcl_quote(accumulated),
    ))
}

pub fn cmd_error(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 || args.len() > 4 {
        return Err(Error::wrong_args_with_usage(
            "error",
            2,
            args.len(),
            "message ?errorInfo? ?errorCode?",
        ));
    }
    let msg = args[1].as_str().to_string();
    let error_info = args.get(2).map(|v| v.as_str().to_string());
    let error_code = args.get(3).map(|v| v.as_str().to_string());
    // Tcl writes the globals at raise time: errorCode is NONE unless the
    // caller supplies one, errorInfo only when supplied.
    let code = error_code.clone().unwrap_or_else(|| "NONE".to_string());
    let _ = interp.set_var("::errorCode", Value::from_str(&code));
    interp.err_code_raised = true;
    // A non-empty info argument REPLACES the accumulated errorInfo and
    // suppresses the next harness frame (25.7: the `while executing
    // "error ..."` frame is absent).  Without info the error behaves like
    // any failing command (N5).
    if let Some(info) = &error_info {
        if !info.is_empty() {
            interp.err_info = Some(info.clone());
            interp.err_fresh = true;
            interp.err_pending_top = None;
        }
    }
    if error_info.is_none() && error_code.is_none() {
        return Err(Error::Msg(msg));
    }
    Err(Error::ControlFlow {
        kind: crate::error::ControlFlow::Error,
        value: Some(Value::from_str(&msg)),
        level: 1,
        error_info,
        error_code,
    })
}

/// try body ?on code varList script? ... ?finally script?
pub fn cmd_try(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "try",
            2,
            args.len(),
            "body ?handler ...? ?finally script?",
        ));
    }

    // Execute the body
    let body = args[1].as_str();
    let body_result = interp.eval(body);
    // try swallows body errors: consume the accumulated errorInfo the
    // same way catch does (writes ::errorInfo, resets the accumulator).
    if let Err(e) = &body_result {
        if interp.err_is_error(e) {
            let info = interp.err_info.take().unwrap_or_else(|| e.message_text());
            let _ = interp.set_var("::errorInfo", Value::from_str(&info));
        }
        interp.err_fresh = false;
    }

    // Determine the exit code and result value
    let (exit_code, result_value) = match &body_result {
        Ok(v) => (0i32, v.as_str().to_string()),
        Err(e) => (e.return_code(), e.message_text()),
    };

    // Parse on/finally handlers
    let mut i = 2;
    let mut handler_result: Option<Result<Value>> = None;
    let mut finally_script: Option<&str> = None;

    while i < args.len() {
        let keyword = args[i].as_str();
        match keyword {
            "on" => {
                // on code varList script
                if i + 3 >= args.len() {
                    return Err(Error::wrong_args_msg(
                        "wrong # args to on clause: must be \"... on code variableList script\"",
                    ));
                }
                let code_spec = args[i + 1].as_str();
                let var_list = args[i + 2].as_str();
                let handler_body = args[i + 3].as_str();

                // Parse the code spec
                let match_code = match code_spec {
                    "ok" => 0,
                    "error" => 1,
                    "return" => 2,
                    "break" => 3,
                    "continue" => 4,
                    "*" => -1, // match any
                    s => s.parse::<i32>().unwrap_or(-2),
                };

                // Check if this handler matches (only use first match)
                if handler_result.is_none() && (match_code == -1 || match_code == exit_code) {
                    // Set variables from varList
                    let vars: Vec<&str> = var_list.split_whitespace().collect();
                    if let Some(msg_var) = vars.first() {
                        if !msg_var.is_empty() {
                            interp.set_var(msg_var, Value::from_str(&result_value))?;
                        }
                    }
                    if let Some(opts_var) = vars.get(1) {
                        if !opts_var.is_empty() {
                            let opts = match &body_result {
                                Err(e) if exit_code == 1 => {
                                    let accumulated = interp
                                        .get_var("::errorInfo")
                                        .map(|v| v.as_str().to_string())
                                        .unwrap_or_default();
                                    error_options_dict(exit_code as i64, 0, e, body, &accumulated)
                                }
                                _ => return_options_dict(exit_code as i64, 0),
                            };
                            interp.set_var(opts_var, opts)?;
                        }
                    }
                    handler_result = Some(interp.eval(handler_body));
                }
                i += 4;
            }
            "finally" => {
                if i + 1 >= args.len() {
                    return Err(Error::wrong_args_msg(
                        "wrong # args to finally clause: must be \"... finally script\"",
                    ));
                }
                finally_script = Some(args[i + 1].as_str());
                i += 2;
            }
            _ => {
                return Err(Error::runtime(
                    format!("bad handler type \"{}\": must be finally, on, or trap", keyword),
                    crate::error::ErrorCode::Generic,
                ));
            }
        }
    }

    // Execute finally script if present
    if let Some(script) = finally_script {
        interp.eval(script)?;
    }

    // Determine the final result
    if let Some(hr) = handler_result {
        // Handler was executed — its result is the return value
        hr
    } else {
        // No handler matched — re-raise original error/result
        body_result
    }
}

/// `tailcall command ?arg ...?`
/// Replaces the current proc invocation with a call to the given command.
/// Simplified implementation: evaluates and returns via return control flow.
pub fn cmd_tailcall(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // tclsh: tailcall outside any proc/lambda/method frame is an error.
    if interp.frames.is_empty() {
        return Err(Error::runtime(
            "tailcall can only be called from a proc, lambda or method",
            crate::error::ErrorCode::Generic,
        ));
    }

    // Signal a tail-call: collect arg strings for TCO re-dispatch in
    // call_proc.  `tailcall` with no command defers an empty invocation —
    // the frame yields the empty completion value.
    let tc_args: Vec<String> = args[1..]
        .iter()
        .map(|a| a.as_str().to_string())
        .collect();
    // Arm the frame marker: a catch consuming the completion still lets
    // the deferred command fire at frame exit, and a later tailcall in
    // the same frame overwrites it (tailcall-12.3).
    interp.frames.last_mut().unwrap().tailcall = Some(tc_args.clone());
    Err(Error::tail_call(tc_args))
}

#[cfg(test)]
mod switch_tests {
    use crate::interp::Interp;

    fn ev(script: &str) -> String {
        let mut interp = Interp::new();
        interp.eval(script).unwrap().as_str().to_string()
    }

    #[test]
    fn test_switch_nocase() {
        // tclsh: -nocase folds pattern and string (switch-1.8..1.10).
        assert_eq!(
            ev("switch -nocase b a {subst 1} b {subst 2} c {subst 3} default {subst 4}"),
            "2"
        );
        assert_eq!(
            ev("switch -nocase B a {subst 1} b {subst 2} c {subst 3} default {subst 4}"),
            "2"
        );
        assert_eq!(
            ev("switch -nocase b a {subst 1} B {subst 2} c {subst 3} default {subst 4}"),
            "2"
        );
    }

    #[test]
    fn test_switch_default_last_wins_after_all_patterns() {
        // tclsh (switch-1.7): default bodies never shadow earlier patterns;
        // with several defaults the LAST one fires only after all fail.
        assert_eq!(
            ev("switch x a {subst 1} default {subst 2} c {subst 3} default {subst 4}"),
            "4"
        );
        assert_eq!(
            ev("switch c a {subst 1} default {subst 2} c {subst 3} default {subst 4}"),
            "3"
        );
        assert_eq!(ev("switch z a {subst 1} default {subst 2}"), "2");
    }

    #[test]
    fn test_switch_nocase_glob() {
        assert_eq!(
            ev("switch -nocase -glob hello HE* {set r one} Hell* {set r two} default {set r three}"),
            "one"
        );
    }

    #[test]
    fn test_switch_default_mode_is_exact() {
        // tclsh (switch-3.4): with no mode flag, patterns match exactly —
        // "a*" does not glob-match "abc".
        assert_eq!(
            ev("switch abc a* {subst glob} abc {subst exact}"),
            "exact"
        );
        assert_eq!(
            ev("switch aaaab {^a*b$} {subst regexp} *b {subst glob} \
                aaaab {subst exact} default {subst none}"),
            "exact"
        );
    }

    #[test]
    fn test_switch_default_pattern_literal_match() {
        // tclsh (switch-1.6): when the string IS "default", a "default"
        // pattern matches it literally instead of acting as the fallback.
        assert_eq!(
            ev("switch default a {subst 1} default {subst 2} c {subst 3} default {subst 4}"),
            "2"
        );
        // -nocase folds the literal match too.
        assert_eq!(
            ev("switch -nocase default DEFAULT {subst lit} fallback {subst 2}"),
            "lit"
        );
        // In every mode the word "default" still acts as the fallback arm.
        assert_eq!(ev("switch -glob x default {subst 1}"), "1");
    }

    #[test]
    fn test_switch_dash_body_fallthrough() {
        // tclsh (switch-7.1): a "-" body defers to the next real body,
        // which is used whether or not its own pattern matches.
        assert_eq!(ev("switch a {a - b {subst 2}}"), "2");
        assert_eq!(ev("switch a {a - b - c {subst 9}}"), "9");
        assert_eq!(ev("switch a {a - default {subst 7}}"), "7");
        assert_eq!(ev("switch a {a - z {subst 1}}"), "1");
        // A matched pattern with a real body is unaffected.
        assert_eq!(ev("switch b {a {subst 1} a - b {subst 2}}"), "2");
        // A trailing "-" with no body to inherit is an error naming the
        // last pattern that held "-".
        assert_eq!(
            ev("catch {switch a {a - b -}} m; set m"),
            "no body specified for pattern \"b\""
        );
    }

    #[test]
    fn test_switch_arg_count_errors() {
        // tclsh (switch-9.2): one leftover word after the options is a
        // pattern with no body.
        assert_eq!(
            ev("catch {switch -- x} m; set m"),
            "extra switch pattern with no body"
        );
        assert_eq!(
            ev("catch {switch x a} m; set m"),
            "extra switch pattern with no body"
        );
        // Bare `switch x` (a string but no arms) is a usage error.
        assert_eq!(
            ev("catch {switch x} m; set m"),
            "wrong # args: should be \"switch ?-option ...? string ?pattern body ...? ?default body?\""
        );
        // `switch -exact x` leaves no room for arms either — options only
        // parse while two arguments remain, so the arms go odd.
        assert_eq!(
            ev("catch {switch -exact x} m; set m"),
            "extra switch pattern with no body"
        );
    }

    #[test]
    fn test_switch_option_prefixes_and_restrictions() {
        // tclsh (switch-3.12): unique option prefixes are accepted.
        assert_eq!(ev("switch -exa Foo Foo {set result OK}"), "OK");
        // Two mode flags are rejected, naming the winner.
        assert_eq!(
            ev("catch {switch -exact -glob x a {b}} m; set m"),
            "bad option \"-glob\": -exact option already found"
        );
        // -matchvar/-indexvar demand regexp mode.
        assert_eq!(
            ev("catch {switch -matchvar v x a {b}} m; set m"),
            "-matchvar option requires -regexp option"
        );
    }

    #[test]
    fn test_switch_comment_heuristic_in_list_form() {
        // tclsh (switch-9.8/9.10): an odd list arm count where a pattern
        // position starts with '#' gets the comment hint.
        assert_eq!(
            ev("catch {switch x {a {} # comment b}} m; set m"),
            "extra switch pattern with no body, this may be due to a comment incorrectly placed outside of a switch body - see the \"switch\" documentation"
        );
        // Inline odd arms never get the hint; neither does a '#' in a body
        // position.
        assert_eq!(
            ev("catch {switch x {a {} b # comment}} m; set m"),
            "extra switch pattern with no body"
        );
    }

    #[cfg(feature = "regexp")]
    #[test]
    fn test_switch_default_zeroes_match_vars() {
        // tclsh (switch-11.4/13.4): reaching the default arm with
        // -matchvar/-indexvar set overwrites them with empty values.
        assert_eq!(
            ev(r#"set x BAD
                switch -regexp -matchvar x -- "a b c" {
                    bc {list $x YES}
                    default {set x}
                }"#),
            ""
        );
        assert_eq!(
            ev(r#"set x -; set y -
                switch -regexp -indexvar x -matchvar y abc {
                    (.)(.)(.). -
                    default {list $x $y}
                }"#),
            "{} {}"
        );
        // Fall-through can land on the default pair's body.
        assert_eq!(ev("switch a {a - default {subst 7} z {subst 8}}"), "7");
    }

    #[cfg(feature = "regexp")]
    #[test]
    fn test_switch_matchvar_indexvar() {
        // tclsh (switch-11.1/12.1/13.1): -matchvar collects the full match
        // plus capture groups; -indexvar collects {start end} pairs with
        // inclusive end indices.
        assert_eq!(
            ev("switch -regexp -matchvar x -- abc {.(.). {set x}}"),
            "abc b"
        );
        assert_eq!(
            ev("switch -regexp -indexvar x -- abc {.(.). {set x}}"),
            "{0 2} {1 1}"
        );
        assert_eq!(
            ev("switch -regexp -indexvar x -matchvar y abc {. {list $x $y}}"),
            "{{0 0}} a"
        );
    }
}
