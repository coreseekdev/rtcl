use crate::interp::Interp;
use crate::Value;

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

// -- info usage / info help tests ----------------------------------------

#[test]
fn test_info_usage_builtin() {
    let mut interp = Interp::new();
    let r = interp.eval("info usage set").unwrap();
    assert_eq!(r.as_str(), "set varName ?newValue?");
}

#[test]
fn test_info_usage_builtin_lsort() {
    let mut interp = Interp::new();
    let r = interp.eval("info usage lsort").unwrap();
    assert_eq!(r.as_str(), "lsort ?-option value ...? list");
}

#[test]
fn test_info_usage_proc_simple() {
    let mut interp = Interp::new();
    interp.eval("proc myfunc {a b} { return ok }").unwrap();
    let r = interp.eval("info usage myfunc").unwrap();
    assert_eq!(r.as_str(), "myfunc a b");
}

#[test]
fn test_info_usage_proc_defaults() {
    let mut interp = Interp::new();
    interp.eval("proc greet {name {greeting hello}} { return ok }").unwrap();
    let r = interp.eval("info usage greet").unwrap();
    assert_eq!(r.as_str(), "greet name ?greeting?");
}

#[test]
fn test_info_usage_proc_args() {
    let mut interp = Interp::new();
    interp.eval("proc varfn {x args} { return ok }").unwrap();
    let r = interp.eval("info usage varfn").unwrap();
    assert_eq!(r.as_str(), "varfn x ?arg ...?");
}

#[test]
fn test_info_usage_not_found() {
    let mut interp = Interp::new();
    assert!(interp.eval("info usage nosuchcommand").is_err());
}

#[test]
fn test_info_usage_wrong_args() {
    let mut interp = Interp::new();
    assert!(interp.eval("info usage").is_err());
    assert!(interp.eval("info usage a b").is_err());
}

#[test]
fn test_info_help_builtin() {
    let mut interp = Interp::new();
    let r = interp.eval("info help set").unwrap();
    assert_eq!(r.as_str(), "Read or write a variable");
}

#[test]
fn test_info_help_builtin_lsort() {
    let mut interp = Interp::new();
    let r = interp.eval("info help lsort").unwrap();
    assert_eq!(r.as_str(), "Sort a list");
}

#[test]
fn test_info_help_proc_no_meta() {
    let mut interp = Interp::new();
    interp.eval("proc myfn {} { return ok }").unwrap();
    let r = interp.eval("info help myfn").unwrap();
    assert_eq!(r.as_str(), "No help available for command \"myfn\"");
}

#[test]
fn test_info_help_not_found() {
    let mut interp = Interp::new();
    assert!(interp.eval("info help nosuchcommand").is_err());
}

#[test]
fn test_info_help_wrong_args() {
    let mut interp = Interp::new();
    assert!(interp.eval("info help").is_err());
    assert!(interp.eval("info help a b").is_err());
}

// ---- info frame / stacktrace / references / tainted / statics / source ----

#[test]
fn test_info_frame_global() {
    let mut interp = Interp::new();
    let r = interp.eval("info frame").unwrap();
    assert!(r.as_str().contains("type source"));
    assert!(r.as_str().contains("level 0"));
}

#[test]
fn test_info_frame_in_proc() {
    let mut interp = Interp::new();
    interp.eval("proc myp {} { info frame }").unwrap();
    let r = interp.eval("myp").unwrap();
    assert!(r.as_str().contains("type proc"));
    assert!(r.as_str().contains("level"));
}

#[test]
fn test_info_frame_bad_level() {
    let mut interp = Interp::new();
    let r = interp.eval("info frame 999");
    assert!(r.is_err());
}

#[test]
fn test_info_stacktrace_global() {
    let mut interp = Interp::new();
    let r = interp.eval("info stacktrace").unwrap();
    // At global level with no proc frames, should be empty
    assert_eq!(r.as_str(), "");
}

#[test]
fn test_info_stacktrace_in_proc() {
    let mut interp = Interp::new();
    interp.eval("proc sp {} { info stacktrace }").unwrap();
    let r = interp.eval("sp").unwrap();
    assert!(r.as_str().contains("frame"));
}

#[test]
fn test_info_references_empty() {
    let mut interp = Interp::new();
    let r = interp.eval("info references").unwrap();
    assert_eq!(r.as_str(), "");
}

#[test]
fn test_info_references_after_ref() {
    let mut interp = Interp::new();
    let handle = interp.eval("ref hello mytag").unwrap();
    let r = interp.eval("info references").unwrap();
    assert!(r.as_str().contains(handle.as_str()));
}

#[test]
fn test_info_tainted_empty() {
    let mut interp = Interp::new();
    let r = interp.eval("info tainted").unwrap();
    assert_eq!(r.as_str(), "");
}

#[test]
fn test_info_tainted_with_var() {
    let mut interp = Interp::new();
    interp.eval("set x 42").unwrap();
    interp.eval("taint x").unwrap();
    let r = interp.eval("info tainted").unwrap();
    assert_eq!(r.as_str(), "x");
}

#[test]
fn test_info_tainted_pattern() {
    let mut interp = Interp::new();
    interp.eval("set abc 1").unwrap();
    interp.eval("set xyz 2").unwrap();
    interp.eval("taint abc").unwrap();
    interp.eval("taint xyz").unwrap();
    let r = interp.eval("info tainted a*").unwrap();
    assert_eq!(r.as_str(), "abc");
}

#[test]
fn test_info_statics_stub() {
    let mut interp = Interp::new();
    interp.eval("proc myp {} { return ok }").unwrap();
    let r = interp.eval("info statics myp").unwrap();
    assert_eq!(r.as_str(), "");
}

#[test]
fn test_info_statics_not_proc() {
    let mut interp = Interp::new();
    let r = interp.eval("info statics set");
    assert!(r.is_err());
}

#[test]
fn test_info_source_stub() {
    let mut interp = Interp::new();
    let r = interp.eval("info source set").unwrap();
    assert_eq!(r.as_str(), "");
}

#[test]
fn test_info_source_not_found() {
    let mut interp = Interp::new();
    let r = interp.eval("info source nosuchcmd");
    assert!(r.is_err());
}

#[test]
fn test_register_command_with_meta() {
    use crate::command::CommandMeta;

    fn cmd_custom(_interp: &mut Interp, _args: &[Value]) -> crate::Result<Value> {
        Ok(Value::from_str("custom"))
    }

    let mut interp = Interp::new();
    interp.register_command_with_meta(
        "mycmd",
        cmd_custom,
        CommandMeta { usage: "arg1 ?arg2?", help: "A custom command" },
    );

    let r = interp.eval("info usage mycmd").unwrap();
    assert_eq!(r.as_str(), "mycmd arg1 ?arg2?");

    let r = interp.eval("info help mycmd").unwrap();
    assert_eq!(r.as_str(), "A custom command");

    // Verify the command itself works
    let r = interp.eval("mycmd").unwrap();
    assert_eq!(r.as_str(), "custom");
}

#[test]
fn test_delete_command_clears_meta() {
    use crate::command::CommandMeta;

    fn cmd_tmp(_interp: &mut Interp, _args: &[Value]) -> crate::Result<Value> {
        Ok(Value::from_str("tmp"))
    }

    let mut interp = Interp::new();
    interp.register_command_with_meta(
        "tmpcmd",
        cmd_tmp,
        CommandMeta { usage: "x", help: "Temporary" },
    );

    // Metadata accessible before delete
    assert!(interp.eval("info usage tmpcmd").is_ok());
    // Delete the command
    interp.eval("rename tmpcmd {}").unwrap();
    // Metadata gone after delete
    assert!(interp.eval("info usage tmpcmd").is_err());
}

#[test]
fn test_command_usage_api() {
    let interp = Interp::new();
    assert_eq!(interp.command_usage("set"), Some("varName ?newValue?".to_string()));
    assert_eq!(interp.command_usage("nosuch"), None);
}

#[test]
fn test_command_help_api() {
    let interp = Interp::new();
    assert_eq!(interp.command_help("set"), Some("Read or write a variable".to_string()));
    assert_eq!(interp.command_help("nosuch"), None);
}

// -- info vars / info globals with namespace-qualified variables
//    (tclsh 8.6.17: patterns match without the leading "::", results are
//    reported qualified when the pattern was qualified or the variable
//    lives in a namespace; `info globals` never lists namespace vars) --

#[test]
fn test_info_vars_qualified_pattern() {
    let mut interp = Interp::new();
    assert_eq!(
        interp
            .eval("namespace eval n { variable v 1 }; info vars ::n::*")
            .unwrap()
            .as_str(),
        "::n::v"
    );
}

#[test]
fn test_info_vars_unqualified_pattern_namespaced_result() {
    let mut interp = Interp::new();
    assert_eq!(
        interp
            .eval("namespace eval n { variable v 1 }; info vars n::*")
            .unwrap()
            .as_str(),
        "::n::v"
    );
}

#[test]
fn test_info_vars_global_via_qualified_pattern() {
    let mut interp = Interp::new();
    assert_eq!(
        interp
            .eval("set errorCode NONE; info vars ::err*")
            .unwrap()
            .as_str(),
        "::errorCode ::errorInfo"
    );
}

#[test]
fn test_info_globals_excludes_namespace_vars() {
    let mut interp = Interp::new();
    assert_eq!(
        interp
            .eval("namespace eval q { variable loc 1 }; info globals q::*")
            .unwrap()
            .as_str(),
        ""
    );
    assert_eq!(
        interp
            .eval("set errorCode NONE; info globals ::err*")
            .unwrap()
            .as_str(),
        "errorCode errorInfo"
    );
}

#[test]
fn test_variable_without_value_creates_nothing() {
    let mut interp = Interp::new();
    assert_eq!(
        interp
            .eval("set two 2; namespace eval nsv { variable two }; info exists nsv::two")
            .unwrap()
            .as_str(),
        "0"
    );
}

// -- wrong-args messages carry the registry usage text (tclsh 8.6.17
//    prints `wrong # args: should be "cmd usage..."` for every builtin;
//    rtcl used to print the generic `cmd N args`) --

#[test]
fn test_wrong_args_uses_registry_usage() {
    let cases: Vec<(&str, String)> = vec![
        ("catch {set} m; set m", r#"wrong # args: should be "set varName ?newValue?""#.to_string()),
        ("catch {append} m; set m", r#"wrong # args: should be "append varName ?value ...?""#.to_string()),
        ("catch {dict} m; set m", r#"wrong # args: should be "dict subcommand ?arg ...?""#.to_string()),
        ("catch {catch} m; set m", r#"wrong # args: should be "catch script ?resultVarName? ?optionVarName?""#.to_string()),
    ];
    for (script, expected) in cases {
        let mut interp = Interp::new();
        let got = interp.eval(script).unwrap().as_str().to_string();
        assert_eq!(got, expected, "script: {}", script);
    }
}

// ── append with no values (tclsh 8.6.17) ─────────────────────

#[test]
fn test_append_no_values_on_missing_errors() {
    // tclsh: `append x` with no values reads x; missing → error
    let mut interp = Interp::new();
    let e = interp
        .eval("unset -nocomplain x; append x")
        .unwrap_err()
        .to_string();
    assert_eq!(e, "can't read \"x\": no such variable");
}

#[test]
fn test_append_no_values_returns_current() {
    let mut interp = Interp::new();
    assert_eq!(interp.eval("set x 5; append x").unwrap().as_str(), "5");
}

// ── foreach varlist validation (tclsh 8.6.17) ────────────────

#[test]
fn test_foreach_empty_varlist_errors() {
    let mut interp = Interp::new();
    let e = interp
        .eval("proc foo {} { foreach {} x { error \"reached body\" } }; catch {foo} m; set m")
        .unwrap()
        .as_str()
        .to_string();
    assert_eq!(e, "foreach varlist is empty");
}

// ── syntax error messages are bare (tclsh 8.6.17) ────────────

#[test]
fn test_missing_close_bracket_message_is_bare() {
    let mut interp = Interp::new();
    let e = interp.eval("set x [foo").unwrap_err().to_string();
    assert_eq!(e, "missing close-bracket");
}

#[test]
fn test_missing_close_brace_message_is_bare() {
    let mut interp = Interp::new();
    let e = interp.eval("set x {foo").unwrap_err().to_string();
    assert_eq!(e, "missing close-brace");
}

// ── info subcommand unique-prefix resolution (tclsh 8.6.17) ──

#[test]
fn test_info_subcommand_prefix() {
    let mut interp = Interp::new();
    // `info a` resolves to args
    assert_eq!(
        interp
            .eval("proc t1 {{a default1} {bbb default2}} {return x}; info a t1")
            .unwrap()
            .as_str(),
        "a bbb"
    );
}

#[test]
fn test_info_subcommand_ambiguous_message() {
    let mut interp = Interp::new();
    let e = interp.eval("info com").unwrap_err().to_string();
    assert_eq!(
        e,
        "unknown or ambiguous subcommand \"com\": must be args, body, class, cmdcount, commands, complete, coroutine, default, errorstack, exists, frame, functions, globals, hostname, level, library, loaded, locals, nameofexecutable, object, patchlevel, procs, script, sharedlibextension, tclversion, or vars"
    );
}

#[test]
fn test_info_commands_ns_pattern() {
    // tclsh: pattern "test_ns_basic::*" finds ::test_ns_basic::p
    let mut interp = Interp::new();
    let r = interp
        .eval("namespace eval test_ns_basic {proc p {} {return 1}}; info commands test_ns_basic::*")
        .unwrap()
        .as_str()
        .to_string();
    assert_eq!(r, "::test_ns_basic::p");
}

// ── expr nan literal (tclsh 8.6.17) ──────────────────────────

#[test]
fn test_expr_bare_nan_is_domain_error() {
    let mut interp = Interp::new();
    let e = interp.eval("catch {expr nan} m; list $m [set ::errorCode]").unwrap();
    assert_eq!(
        e.as_str(),
        "{domain error: argument not in valid range} {ARITH DOMAIN {domain error: argument not in valid range}}"
    );
}

#[test]
fn test_expr_inf_literal_still_works() {
    let mut interp = Interp::new();
    assert_eq!(interp.eval("expr inf").unwrap().as_str(), "Inf");
    assert_eq!(interp.eval("expr -inf").unwrap().as_str(), "-Inf");
}

// ── binary scan x* consumes all remaining bytes ──────────────

#[test]
fn test_scan_x_star_skips_to_end() {
    // tclsh: a3x*a3 on "abcdefg" → second a3 has no bytes left, arg2 unset
    let mut interp = Interp::new();
    let r = interp
        .eval("unset -nocomplain a1 a2; set a2 zz; list [binary scan abcdefg a3x*a3 a1 a2] $a1 $a2")
        .unwrap();
    assert_eq!(r.as_str(), "1 abc zz");
}

// ── scan: tclsh 8.6.17 parity (probed in judge/probes/scan_*.tcl) ──

#[test]
fn test_scan_validator_errors_and_codes() {
    let mut interp = Interp::new();
    // trailing '%' → bad conversion character NUL
    let r = interp
        .eval(r#"catch {scan abc {%}} m; list $m"#)
        .unwrap();
    assert_eq!(
        r.as_str(),
        format!("{{bad scan conversion character \"\u{0}\"}}")
    );
    // %c with width
    let r = interp
        .eval("catch {scan abc {%2c} v} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{field width may not be specified in %c conversion} {TCL FORMAT BADWIDTH}"
    );
    // size modifier forbidden on %c/%n/%s/%[
    let r = interp
        .eval("catch {scan 5 %zn v} m; list $m [set ::errorCode]")
        .unwrap()
    ;
    // %z is a bad conversion character in 8.6 (z/t arrived in 9.0)
    assert_eq!(
        r.as_str(),
        "{bad scan conversion character \"z\"} {TCL FORMAT BADTYPE}"
    );
    let r = interp
        .eval("catch {scan 5 %ln v} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{field size modifier may not be specified in %n conversion} {TCL FORMAT BADSIZE}"
    );
    // %h is allowed everywhere
    assert_eq!(interp.eval("scan abc %hc v; set v").unwrap().as_str(), "97");
    // too many vars
    let r = interp
        .eval("catch {scan 5 %d a b} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{variable is not assigned by any conversion specifiers} {TCL FORMAT UNASSIGNED}"
    );
    // %*1$d: suppressed specs skip the XPG parse, '$' becomes a bad type
    let r = interp
        .eval(r#"catch {scan 5 {%*1$d} a} m; set m"#)
        .unwrap();
    assert_eq!(r.as_str(), "bad scan conversion character \"$\"");
    // valid pure-XPG form succeeds
    let r = interp
        .eval(r#"catch {scan 55 {%1$d %2$d} a b} m; list $m $a [info exists b]"#)
        .unwrap();
    assert_eq!(r.as_str(), "1 55 0");
    // digits-without-$ after an XPG spec -> mixing error
    let r = interp
        .eval("catch {scan 5 {%1$d %2} a b} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{cannot mix \"%\" and \"%n$\" conversion specifiers} {TCL FORMAT MIXEDSPECTYPES}"
    );
}

#[test]
fn test_scan_integer_semantics() {
    let mut interp = Interp::new();
    // u64 wrap then sign-apply
    assert_eq!(
        interp.eval("scan ffffffffffffffff %x").unwrap().as_str(),
        "-1"
    );
    assert_eq!(
        interp.eval("scan -ffffffffffffffff %x").unwrap().as_str(),
        "1"
    );
    assert_eq!(
        interp
            .eval("scan fffffffffffffffff %x")
            .unwrap()
            .as_str(),
        "9223372036854775807"
    );
    // decimal saturation
    assert_eq!(
        interp
            .eval("scan 99999999999999999999 %d")
            .unwrap()
            .as_str(),
        "9223372036854775807"
    );
    // %d stops at underscore, no 32-bit clamp
    assert_eq!(interp.eval("scan 1_000 %d").unwrap().as_str(), "1");
    assert_eq!(interp.eval("scan 2147483648 %d").unwrap().as_str(), "2147483648");
    // %i: 0x hex, leading-0 octal, no 0b/0o; junk prefix accepts the "0"
    assert_eq!(interp.eval("scan 0x1f %i").unwrap().as_str(), "31");
    assert_eq!(interp.eval("scan 010 %i").unwrap().as_str(), "8");
    assert_eq!(interp.eval("scan 018 %i").unwrap().as_str(), "1");
    assert_eq!(interp.eval("scan 0b101 %i").unwrap().as_str(), "0");
    // consumed length via %n
    assert_eq!(
        interp
            .eval("scan 0b101 %i%n v n; list $v $n")
            .unwrap()
            .as_str(),
        "0 1"
    );
    // %o: no prefixes; %b: 0b prefix
    assert_eq!(interp.eval("scan 0x12 %o").unwrap().as_str(), "0");
    assert_eq!(interp.eval("scan 0b101 %b").unwrap().as_str(), "5");
    assert_eq!(interp.eval("scan 1012 %b").unwrap().as_str(), "5");
    // %u reports the u64 reinterpretation
    assert_eq!(
        interp.eval("scan -1 %u").unwrap().as_str(),
        "18446744073709551615"
    );
    // width 0 means unlimited
    assert_eq!(
        interp
            .eval("scan abcdef %0s%n v n; list $v $n")
            .unwrap()
            .as_str(),
        "abcdef 6"
    );
}

#[test]
fn test_scan_float_semantics() {
    let mut interp = Interp::new();
    assert_eq!(interp.eval("scan 12e %f").unwrap().as_str(), "12.0");
    assert_eq!(interp.eval("scan .5 %f").unwrap().as_str(), "0.5");
    assert_eq!(interp.eval("scan 3. %f").unwrap().as_str(), "3.0");
    // inf accepted, always 3 chars; nan accepted by the parser but
    // refused by scan (no conversion counted, nothing consumed)
    assert_eq!(interp.eval("scan inf %f").unwrap().as_str(), "Inf");
    assert_eq!(interp.eval("scan -Infinity %f").unwrap().as_str(), "-Inf");
    assert_eq!(interp.eval("scan infX %f%n v n; set n").unwrap().as_str(), "3");
    interp.eval("unset -nocomplain nv nn").unwrap();
    assert_eq!(
        interp
            .eval("catch {scan NaN %f%n nv nn} r; list $r [info exists nv]")
            .unwrap()
            .as_str(),
        "0 0"
    );
    // underflow only when the failed parse reached end of input
    assert_eq!(interp.eval("scan na %f").unwrap().as_str(), "");
    assert_eq!(
        interp
            .eval("catch {scan n %f v} r")
            .unwrap()
            .as_str(),
        "0"
    );
    // partial inf/nan fails quietly
    assert_eq!(interp.eval("scan iNx %f").unwrap().as_str(), "{}");
}

#[test]
fn test_scan_result_shapes() {
    let mut interp = Interp::new();
    // underflow with no conversions: -1 / empty list
    assert_eq!(interp.eval("scan {} %c v").unwrap().as_str(), "-1");
    assert_eq!(interp.eval("scan abc %d").unwrap().as_str(), "{}");
    // list mode pads with {}
    assert_eq!(
        interp.eval("scan 12 %d%d").unwrap().as_str(),
        "12 {}"
    );
    // set-no-match stops quietly
    assert_eq!(
        interp
            .eval("catch {scan ab {%[%]} x} r; list $r")
            .unwrap()
            .as_str(),
        "0"
    );
    // %n counts even after a suppressed conversion
    assert_eq!(
        interp
            .eval("scan abc {%*s%n} b; list $b")
            .unwrap()
            .as_str(),
        "3"
    );
}

#[test]
fn test_format_strict_errors() {
    let mut interp = Interp::new();
    // strict integer argument
    let r = interp
        .eval("catch {format %d 2a} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{expected integer but got \"2a\"} {TCL VALUE NUMBER}"
    );
    // * width from args must be an integer (TCL VALUE INTEGER)
    let r = interp
        .eval("catch {format %*d x 3} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{expected integer but got \"x\"} {TCL VALUE INTEGER}"
    );
    // *.*: precision arg also strict
    let r = interp
        .eval("catch {format %*.*f 2 xyz 3} m; list $m")
        .unwrap();
    assert_eq!(r.as_str(), "{expected integer but got \"xyz\"}");
    // bad conversion char
    let r = interp
        .eval("catch {format %q x} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{bad field specifier \"q\"} {TCL FORMAT BADTYPE}"
    );
    // '-' after '.' is a bad field specifier
    let r = interp
        .eval(r#"catch {format "a%.-2sa" foobarbaz} m; set m"#)
        .unwrap();
    assert_eq!(r.as_str(), "bad field specifier \"-\"");
    // flags after width are a bad field specifier
    let r = interp
        .eval("catch {format {%5-d} 42} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "bad field specifier \"-\"");
    // trailing % with no args left -> not enough arguments
    let r = interp
        .eval("catch {format ab%} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{not enough arguments for all format specifiers} {TCL FORMAT FIELDVARMISMATCH}"
    );
    // trailing % with args available -> ended in middle
    let r = interp
        .eval("catch {format ab% 12} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{format string ended in middle of field specifier} {TCL FORMAT INCOMPLETE}"
    );
    // %l with an arg -> ended in middle
    let r = interp
        .eval("catch {format %l 5} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "format string ended in middle of field specifier");
    // %5% mid-string -> bad field specifier "%"
    let r = interp
        .eval("catch {format {%5%} x} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "bad field specifier \"%\"");
    // mixing sequential and XPG
    let r = interp
        .eval("catch {format {%d %1$d} 1 2} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{cannot mix \"%\" and \"%n$\" conversion specifiers} {TCL FORMAT MIXEDSPECTYPES}"
    );
    // XPG index out of range (message has a literal n)
    let r = interp
        .eval("catch {format {%5$d} x} m; list $m [set ::errorCode]")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "{\"%n$\" argument index out of range} {TCL FORMAT INDEXRANGE}"
    );
    // strict integer grammar: 08 is invalid octal
    let r = interp
        .eval("catch {format %d 08} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "expected integer but got \"08\"");
}

#[test]
fn test_format_h_modifier_and_xpg() {
    let mut interp = Interp::new();
    // h truncates to 16 bits
    assert_eq!(interp.eval("format %hd 0xffff").unwrap().as_str(), "-1");
    assert_eq!(interp.eval("format %hd 0x10000").unwrap().as_str(), "0");
    assert_eq!(interp.eval("format %hx -2").unwrap().as_str(), "fffe");
    assert_eq!(interp.eval("format %hu 70000").unwrap().as_str(), "4464");
    assert_eq!(interp.eval("format %hd -65536").unwrap().as_str(), "0");
    // l/ll are no-ops
    assert_eq!(interp.eval("format %ld 5").unwrap().as_str(), "5");
    assert_eq!(interp.eval("format %lld 5").unwrap().as_str(), "5");
    // %hs is allowed in format
    assert_eq!(interp.eval("format {%hs} ab").unwrap().as_str(), "ab");
    // XPG positional args
    assert_eq!(
        interp.eval("format {%2$s-%1$s} a b").unwrap().as_str(),
        "b-a"
    );
    assert_eq!(
        interp.eval("format {%2$d %1$x} 10 20").unwrap().as_str(),
        "20 a"
    );
    // same arg twice is fine
    assert_eq!(
        interp.eval("format {%1$d %1$d} 5").unwrap().as_str(),
        "5 5"
    );
    // flags and width combine with XPG
    assert_eq!(
        interp.eval("format {%1$+d} 5").unwrap().as_str(),
        "+5"
    );
    assert_eq!(
        interp.eval("format {%2$5d} 1 2").unwrap().as_str(),
        "    2"
    );
    // star after XPG -> bad field specifier "$"
    let r = interp
        .eval("catch {format {%1$*2$d} 42 6} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "bad field specifier \"$\"");
}

#[test]
fn test_format_wrap_and_padding() {
    let mut interp = Interp::new();
    // bignum wraps mod 2^64 then reinterprets
    assert_eq!(
        interp
            .eval("format %d 99999999999999999999")
            .unwrap()
            .as_str(),
        "7766279631452241919"
    );
    assert_eq!(
        interp
            .eval("format %d -9223372036854775809")
            .unwrap()
            .as_str(),
        "9223372036854775807"
    );
    assert_eq!(
        interp
            .eval("format %d 9223372036854775808")
            .unwrap()
            .as_str(),
        "-9223372036854775808"
    );
    // Tcl integer grammar
    assert_eq!(interp.eval("format %d 017").unwrap().as_str(), "15");
    assert_eq!(interp.eval("format %d 0b101").unwrap().as_str(), "5");
    assert_eq!(interp.eval("format %d 0o17").unwrap().as_str(), "15");
    assert_eq!(interp.eval("format %d { 12}").unwrap().as_str(), "12");
    // negative star width = left justify
    assert_eq!(
        interp.eval("format {%*d} -5 42").unwrap().as_str(),
        "42   "
    );
    // negative star precision = 0
    assert_eq!(
        interp.eval("format {%.*f} -2 3.14159").unwrap().as_str(),
        "3"
    );
    // 0 flag pads s and c with zeros, even with '-'
    assert_eq!(interp.eval("format %05s a").unwrap().as_str(), "0000a");
    assert_eq!(interp.eval("format {%-05s} ab").unwrap().as_str(), "ab000");
    assert_eq!(interp.eval("format %05c 61").unwrap().as_str(), "0000=");
    // %#b prefix
    assert_eq!(interp.eval("format {%#b} 5").unwrap().as_str(), "0b101");
    // hash keeps the point at precision 0
    assert_eq!(interp.eval("format {%#.0f} 9.5").unwrap().as_str(), "10.");
    assert_eq!(interp.eval("format {%#.0e} 9.5").unwrap().as_str(), "1.e+01");
    // width pads by characters, not bytes
    assert_eq!(
        interp.eval("format {%4s} éé").unwrap().as_str(),
        "  éé"
    );
    // precision truncates strings by characters
    assert_eq!(
        interp.eval("format {%.2s} ééé").unwrap().as_str(),
        "éé"
    );
}

#[test]
fn test_format_xpg_cursor_and_star() {
    let mut interp = Interp::new();
    // XPG n$ is an offset from the sequential cursor; star widths feed
    // the sequential stream. "%1$*d 6 42" -> width 6 from star, value
    // 42 from cursor+1-1.
    assert_eq!(
        interp.eval("format {%1$*d} 6 42").unwrap().as_str(),
        "    42"
    );
    // star then xpg is mixing, xpg-then-star-in-spec is not
    let r = interp
        .eval("catch {format {%*d %1$d} 6 42} m; set m")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "cannot mix \"%\" and \"%n$\" conversion specifiers"
    );
    let r = interp
        .eval("catch {format {%1$d %*d} 42 6 99} m; set m")
        .unwrap();
    assert_eq!(
        r.as_str(),
        "cannot mix \"%\" and \"%n$\" conversion specifiers"
    );
    // xpg index out of range is checked against cursor+n
    let r = interp
        .eval("catch {format {%5$*d} 6 42} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "\"%n$\" argument index out of range");
    // %5$ with 5 args is valid and lands on the last one
    let r = interp
        .eval("catch {format {%5$d} a b c d e} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "expected integer but got \"e\"");
    // digits after '*' are consumed and discarded
    assert_eq!(
        interp.eval("format {%*5d} 6 42").unwrap().as_str(),
        "    42"
    );
    assert_eq!(
        interp.eval("format {%*05d} 6 42").unwrap().as_str(),
        "    42"
    );
    // zero flag before star
    assert_eq!(
        interp.eval("format {%0*d} 6 42").unwrap().as_str(),
        "000042"
    );
    // '.' with no digits is precision 0
    assert_eq!(interp.eval("format {%.d} 5").unwrap().as_str(), "5");
    assert_eq!(interp.eval("format {%.f} 3.14159").unwrap().as_str(), "3");
    // invalid conversion char: not-enough wins when no args remain
    let r = interp
        .eval("catch {format %q} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "not enough arguments for all format specifiers");
    // ...with args available the bad specifier fires
    let r = interp
        .eval("catch {format {%5%} a b c d e} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "bad field specifier \"%\"");
}

#[test]
fn test_format_xpg_zero_index_and_caps() {
    let mut interp = Interp::new();
    // %0$ parses as an XPG index (not the zero flag) and is out of range
    let r = interp
        .eval("catch {format {%0$d} 4} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "\"%n$\" argument index out of range");
    // ...including after a valid XPG spec (no spurious mixing error)
    let r = interp
        .eval("catch {format {%2$d %0$d} 4 5 6} m; set m")
        .unwrap();
    assert_eq!(r.as_str(), "\"%n$\" argument index out of range");
    // %0$ + zero flag still works ("%02$d" -> zero flag + xpg 2)
    assert_eq!(interp.eval("format {%2$03d} 4 5").unwrap().as_str(), "005");
    // float + 0 + no minus zero-fills right; with minus left-justifies
    assert_eq!(
        interp
            .eval("format %020f -9.99996")
            .unwrap()
            .as_str(),
        "-000000000009.999960"
    );
    assert_eq!(
        interp
            .eval(r#"format "%-020f" -9.99996"#)
            .unwrap()
            .as_str(),
        "-9.999960           "
    );
    // ints + 0 ignore '-': right zero-fill, sign first
    assert_eq!(
        interp.eval("format %-020d -9").unwrap().as_str(),
        "-0000000000000000009"
    );
    // XPG argvIndex model: star width consumes argv[n-1], value argv[n]
    assert_eq!(
        interp.eval("format {%2$*d %3$d} 1 10 4").unwrap().as_str(),
        "         4 4"
    );
    assert_eq!(
        interp
            .eval("format {%2$.*s %4$d} 1 5 abcdefghijklmnop 44")
            .unwrap()
            .as_str(),
        "abcde 44"
    );
    let r = interp
        .eval(r#"set c [catch {format {%2$*d} 4 5 6} m]; list $c $m"#)
        .unwrap();
    assert_eq!(r.as_str(), "0 {    6}");
}
