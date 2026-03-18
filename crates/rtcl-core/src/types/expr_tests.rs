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
    assert_eq!(eval_expr(&mut interp, "pow(2, 3)").unwrap().as_int(), Some(8));
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
    assert_eq!(eval_expr(&mut interp, r#""abc" =~ "^[a-z]+$""#).unwrap().as_bool(), Some(true));
    assert_eq!(eval_expr(&mut interp, r#""123" =~ "^[a-z]+$""#).unwrap().as_bool(), Some(false));
    assert_eq!(eval_expr(&mut interp, r#""hello123" =~ "\\d+""#).unwrap().as_bool(), Some(true));
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
