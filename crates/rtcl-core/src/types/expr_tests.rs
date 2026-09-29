use super::*;

#[test]
fn test_arithmetic() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "1 + 2").unwrap().as_int(), Some(3));
    assert_eq!(eval_expr(&mut interp, "10 - 3").unwrap().as_int(), Some(7));
    assert_eq!(eval_expr(&mut interp, "4 * 5").unwrap().as_int(), Some(20));
    assert_eq!(eval_expr(&mut interp, "15 / 3").unwrap().as_int(), Some(5));
}

#[test]
fn test_comparison() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "1 < 2").unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, "2 > 1").unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, "1 == 1").unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, "1 != 2").unwrap().as_bool(), Some(true));
}

#[test]
fn test_logical() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "1 && 1").unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, "1 && 0").unwrap().as_bool(), Some(false));
    assert_eq!(eval_expr(&mut interp, "0 || 1").unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, "!0").unwrap().as_bool(), Some(true));
}

#[test]
fn test_functions() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "abs(-5)").unwrap().as_int(), Some(5));
    assert_eq!(eval_expr(&mut interp, "sqrt(16)").unwrap().as_float(), Some(4.0));
    // Tcl's pow() always returns a double: pow(2,3) => 8.0
    assert_eq!(eval_expr(&mut interp, "pow(2, 3)").unwrap().as_str(), "8.0");
}

#[test]
fn test_variables() {
    let mut interp = Interp::new();
    interp.set_var("x", Value::from_int(10)).unwrap();
    assert_eq!(eval_expr(&mut interp, "$x + 5").unwrap().as_int(), Some(15));
}

// -- String comparison operators: lt, gt, le, ge --

#[test]
fn test_lt_gt_le_ge() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, r#""abc" lt "abd""#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""abd" gt "abc""#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""abc" le "abc""#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""abc" ge "abd""#).unwrap().as_bool(), Some(false));
    assert_eq!(eval_expr(&mut interp, r#""z" gt "a""#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""abc" le "abd""#).unwrap().as_bool(), Some(true));
}

// -- Glob match operator: =* --

#[test]
fn test_glob_match_op() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, r#""hello" =* "hel*""#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""hello" =* "world""#).unwrap().as_bool(), Some(false));
    assert_eq!(eval_expr(&mut interp, r#""foo.bar" =* "*.bar""#).unwrap().as_bool(), Some(true));
}

// -- Regexp match operator: =~ (requires regexp feature) --

#[cfg(feature = "regexp")]
#[test]
fn test_regexp_match_op() {
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, r#""abc" =~ {^[a-z]+$}"#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""123" =~ {^[a-z]+$}"#).unwrap().as_bool(), Some(false));
    assert_eq!(eval_expr(&mut interp, r#""hello123" =~ {\d+}"#).unwrap().as_bool(), Some(true));
}

// -- Rotate operators: <<<, >>> --

#[test]
fn test_rotate_left() {
    let mut interp = Interp::new();
    // 1 <<< 1 = 2
    assert_eq!(eval_expr(&mut interp, "1 <<< 1").unwrap().as_int(), Some(2));
    // 1 <<< 63 should rotate to top bit position
    let r = eval_expr(&mut interp, "1 <<< 63").unwrap().as_int().unwrap();
    assert_eq!(r, i64::MIN); // bit 63 set = negative for signed
}

#[test]
fn test_rotate_right() {
    let mut interp = Interp::new();
    // 2 >>> 1 = 1
    assert_eq!(eval_expr(&mut interp, "2 >>> 1").unwrap().as_int(), Some(1));
    // 1 >>> 1 should wrap high bit
    let r = eval_expr(&mut interp, "1 >>> 1").unwrap().as_int().unwrap();
    assert_eq!(r, i64::MIN); // wraps to MSB
}

#[test]
fn test_shift_still_works() {
    // Ensure << and >> still work correctly after adding <<< and >>>
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "1 << 4").unwrap().as_int(), Some(16));
    assert_eq!(eval_expr(&mut interp, "16 >> 2").unwrap().as_int(), Some(4));
}

// -- braced literals: backslash-escaped braces do not count for balancing
//    (tclsh 8.6.17: `expr {$s eq {a \{}}` compares the 4-char literal) --

#[test]
fn test_expr_braced_literal_escaped_brace() {
    let mut interp = Interp::new();
    let r = interp
        .eval(r#"set s "a b c d d300 d35 e \\{"; expr {$s eq {a b c d d300 d35 e \{}}"#)
        .unwrap();
    assert_eq!(r.as_str(), "1");
}

#[test]
fn test_expr_braced_literal_keeps_backslashes_literal() {
    let mut interp = Interp::new();
    // Content keeps backslashes verbatim; \{ does not open a nested brace.
    let r = interp
        .eval(r#"string length [expr {{a \{}}]"#)
        .unwrap();
    assert_eq!(r.as_str(), "4");
}

#[test]
fn test_expr_braced_literal_escaped_brace_in_if() {
    let mut interp = Interp::new();
    let r = interp
        .eval(r#"set r [lsort {d e c b a \{ d35 d300}]; if {$r eq {a b c d d300 d35 e \{}} {set o YES} {set o NO}; set o"#)
        .unwrap();
    assert_eq!(r.as_str(), "YES");
}

// -- math domain errors (tclsh 8.6.17 oracle) --

#[test]
fn test_sqrt_negative_domain_error() {
    let mut interp = Interp::new();
    let e = eval_expr(&mut interp, "sqrt(-1)").unwrap_err().to_string();
    assert_eq!(e, "domain error: argument not in valid range");
}

#[test]
fn test_log_negative_domain_error() {
    let mut interp = Interp::new();
    assert_eq!(
        eval_expr(&mut interp, "log(-1)").unwrap_err().to_string(),
        "domain error: argument not in valid range"
    );
    assert_eq!(
        eval_expr(&mut interp, "log10(-1)").unwrap_err().to_string(),
        "domain error: argument not in valid range"
    );
}

#[test]
fn test_log_zero_is_neg_inf() {
    // tclsh: log(0) → -Inf, no error
    let mut interp = Interp::new();
    let r = eval_expr(&mut interp, "log(0)").unwrap();
    assert!(r.as_str().contains("Inf"), "log(0) = {}", r.as_str());
}

#[test]
fn test_asin_acos_domain_error() {
    let mut interp = Interp::new();
    assert_eq!(
        eval_expr(&mut interp, "asin(2)").unwrap_err().to_string(),
        "domain error: argument not in valid range"
    );
    assert_eq!(
        eval_expr(&mut interp, "acos(-2)").unwrap_err().to_string(),
        "domain error: argument not in valid range"
    );
}

#[test]
fn test_pow_negative_base_fractional_exponent() {
    let mut interp = Interp::new();
    assert_eq!(
        eval_expr(&mut interp, "pow(-2, 0.5)").unwrap_err().to_string(),
        "domain error: argument not in valid range"
    );
    // integer exponent on negative base is fine
    assert_eq!(
        eval_expr(&mut interp, "pow(-2, 3)").unwrap().as_str(),
        "-8.0"
    );
}

#[test]
fn test_fmod_by_zero_domain_error() {
    let mut interp = Interp::new();
    assert_eq!(
        eval_expr(&mut interp, "fmod(1, 0)").unwrap_err().to_string(),
        "domain error: argument not in valid range"
    );
}

#[test]
fn test_logical_returns_boolean_not_operand() {
    // tclsh: && and || ALWAYS yield 0/1 (expr-3.1: `3||0` -> 1).
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "3 || 0").unwrap().as_str(), "1");
    assert_eq!(eval_expr(&mut interp, "1.3 || 0").unwrap().as_str(), "1");
    assert_eq!(eval_expr(&mut interp, "0 || 0").unwrap().as_str(), "0");
    assert_eq!(eval_expr(&mut interp, "2.5 && 3").unwrap().as_str(), "1");
    assert_eq!(eval_expr(&mut interp, "3 && 0").unwrap().as_str(), "0");
}

#[test]
fn test_glued_eq_ne_in_operators() {
    // tclsh: only eq/ne/in glue to a preceding number (3eq2 -> 0).
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "3eq2").unwrap().as_str(), "0");
    assert_eq!(eval_expr(&mut interp, "3ne2").unwrap().as_str(), "1");
    assert_eq!(eval_expr(&mut interp, "3in2").unwrap().as_str(), "0");
    assert_eq!(eval_expr(&mut interp, "3eq2.0").unwrap().as_str(), "0");
}

#[test]
fn test_invalid_bareword_after_digits() {
    // tclsh: digit-led alnum runs that aren't numbers are errors.
    let mut interp = Interp::new();
    for e in ["3x2", "3e", "5mod3", "3and2", "3lt2", "3gt2"] {
        let err = eval_expr(&mut interp, e).unwrap_err().to_string();
        assert!(
            err.contains(&format!("invalid bareword \"{}\"", e)),
            "expr {}: got {}",
            e,
            err
        );
    }
}

#[test]
fn test_chained_unary_signs() {
    // tclsh: --5 -> 5, +--++36 -> 36.
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "--5").unwrap().as_str(), "5");
    assert_eq!(eval_expr(&mut interp, "+--++36").unwrap().as_str(), "36");
}

#[test]
fn test_bool_literals_keep_string_form() {
    // tclsh: `expr false` renders "false" (expr-21.1); operators coerce.
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "false").unwrap().as_str(), "false");
    assert_eq!(eval_expr(&mut interp, "true").unwrap().as_str(), "true");
    assert_eq!(eval_expr(&mut interp, "!false").unwrap().as_str(), "1");
    assert_eq!(
        eval_expr(&mut interp, "false && false").unwrap().as_str(),
        "0"
    );
    assert_eq!(
        eval_expr(&mut interp, "true && 1").unwrap().as_str(),
        "1"
    );
}

#[test]
fn test_isqrt_error_messages() {
    // tclsh: function-specific messages, not generic domain error.
    let mut interp = Interp::new();
    let e1 = eval_expr(&mut interp, "isqrt(-1)").unwrap_err().to_string();
    assert_eq!(e1, "square root of negative argument");
    let e2 = eval_expr(&mut interp, "isqrt(1,2)").unwrap_err().to_string();
    assert_eq!(e2, "too many arguments for math function \"isqrt\"");
}

#[test]
fn test_min_int_literal() {
    // tclsh: -9223372036854775808 stays an integer.
    let mut interp = Interp::new();
    assert_eq!(
        eval_expr(&mut interp, "-9223372036854775808")
            .unwrap()
            .as_str(),
        "-9223372036854775808"
    );
    // Note: `0 - 9223372036854775808` needs bignum widening (tclsh gives
    // the exact integer); rtcl promotes to double — deferred.
}

#[test]
fn test_exponent_floats_keep_exponent() {
    // Regression: `3.0e98` must not lose its `e` (gen_util-6.6).
    let mut interp = Interp::new();
    assert_eq!(eval_expr(&mut interp, "3.0e98").unwrap().as_str(), "3e+98");
    assert_eq!(eval_expr(&mut interp, "1e-3").unwrap().as_str(), "0.001");
    assert_eq!(eval_expr(&mut interp, "2E+3").unwrap().as_str(), "2000.0");
    assert_eq!(eval_expr(&mut interp, "3eq2").unwrap().as_str(), "0");
}

#[test]
fn test_mathfunc_namespace_commands() {
    // expr-38.5/38.11: expr functions are commands in ::tcl::mathfunc.
    let mut interp = Interp::new();
    assert_eq!(
        interp.eval("::tcl::mathfunc::abs -0").unwrap().as_str(),
        "0"
    );
    assert_eq!(
        interp.eval("tcl::mathfunc::abs -3").unwrap().as_str(),
        "3"
    );
    assert_eq!(
        interp.eval("::tcl::mathfunc::int 3.7").unwrap().as_str(),
        "3"
    );
}

#[test]
fn test_isqrt_non_numeric_arg_message() {
    // expr-47.2: non-numeric operand -> expected number but got "rubbish".
    let mut interp = Interp::new();
    let e = eval_expr(&mut interp, "isqrt({rubbish})")
        .unwrap_err()
        .to_string();
    assert_eq!(e, "expected number but got \"rubbish\"");
}

#[test]
fn test_braced_numeric_string_normalized() {
    // expr-1.8: a lone braced literal converts to a number, incl. hex.
    // (Outer braces are stripped by the script parser, so expr sees
    // `{-0x1234}` — a braced string literal — and normalizes it.)
    let mut interp = Interp::new();
    assert_eq!(
        interp.eval("expr {{-0x1234}}").unwrap().as_str(),
        "-4660"
    );
    assert_eq!(interp.eval("expr {{0b101}}").unwrap().as_str(), "5");
}

#[test]
fn test_nan_compares_unequal() {
    // expr-22.9: NaN != NaN.
    let mut interp = Interp::new();
    interp.set_var("x", Value::from_str("NaN")).unwrap();
    assert_eq!(eval_expr(&mut interp, "$x == $x").unwrap().as_str(), "0");
}
