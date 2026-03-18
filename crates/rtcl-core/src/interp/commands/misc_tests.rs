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
    assert_eq!(r.as_str(), "set varName ?value?");
}

#[test]
fn test_info_usage_builtin_lsort() {
    let mut interp = Interp::new();
    let r = interp.eval("info usage lsort").unwrap();
    assert_eq!(r.as_str(), "lsort ?options? list");
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
    assert_eq!(interp.command_usage("set"), Some("varName ?value?".to_string()));
    assert_eq!(interp.command_usage("nosuch"), None);
}

#[test]
fn test_command_help_api() {
    let interp = Interp::new();
    assert_eq!(interp.command_help("set"), Some("Read or write a variable".to_string()));
    assert_eq!(interp.command_help("nosuch"), None);
}
