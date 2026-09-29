//! Tests for the `binary` command — expectations verified against
//! tclsh 8.6.17 (probe scripts in specs/DIVERGENCES.md workflow).

use crate::interp::Interp;

fn eval(script: &str) -> String {
    let mut interp = Interp::new();
    interp.eval(script).unwrap().as_str().to_string()
}

fn eval_err(script: &str) -> String {
    let mut interp = Interp::new();
    interp.eval(script).unwrap_err().to_string()
}

// ── format: strings ────────────────────────────────────────────────────

#[test]
fn test_format_a_pads_with_nulls() {
    // tclsh: bytes 61 62 00
    assert_eq!(eval("binary scan [binary format a3 ab] H* h; set h"), "616200");
}

#[test]
fn test_format_a_truncates() {
    assert_eq!(eval("binary scan [binary format a1 abc] H* h; set h"), "61");
}

#[test]
fn test_format_A_pads_with_spaces() {
    assert_eq!(eval("binary scan [binary format A3 ab] H* h; set h"), "616220");
}

#[test]
fn test_format_a_default_count_is_whole_string() {
    assert_eq!(eval("binary scan [binary format a ab] H* h; set h"), "6162");
}

#[test]
fn test_format_b_bits_lsb_first() {
    // "1010" b4 → bits 0,2 set → 0x05
    assert_eq!(eval("binary scan [binary format b4 1010] H* h; set h"), "05");
}

#[test]
fn test_format_B_bits_msb_first() {
    assert_eq!(eval("binary scan [binary format B4 1010] H* h; set h"), "a0");
}

#[test]
fn test_format_b_default_count_8() {
    assert_eq!(eval("binary scan [binary format b 1] H* h; set h"), "01");
}

#[test]
fn test_format_b_rejects_non_digits() {
    assert_eq!(
        eval_err("binary format b2 1x"),
        "expected binary string but got \"1x\" instead"
    );
}

#[test]
fn test_format_h_low_nibble_first() {
    assert_eq!(eval("binary scan [binary format h4 abcd] H* h; set h"), "badc");
}

#[test]
fn test_format_H_high_nibble_first() {
    assert_eq!(eval("binary scan [binary format H4 abcd] H* h; set h"), "abcd");
}

#[test]
fn test_format_h_odd_count_pads_low() {
    assert_eq!(eval("binary scan [binary format h3 abc] H* h; set h"), "ba0c");
}

#[test]
fn test_format_h_rejects_non_hex() {
    assert_eq!(
        eval_err("binary format h2 ax"),
        "expected hexadecimal string but got \"ax\" instead"
    );
}

// ── format: integers ───────────────────────────────────────────────────

#[test]
fn test_format_c_truncates_mod_256() {
    // tclsh: c 300 → 0x2c
    assert_eq!(eval("binary scan [binary format c 300] H* h; set h"), "2c");
}

#[test]
fn test_format_c_negative() {
    assert_eq!(eval("binary scan [binary format c -1] H* h; set h"), "ff");
}

#[test]
fn test_format_s_little_endian() {
    assert_eq!(eval("binary scan [binary format s -2] H* h; set h"), "feff");
}

#[test]
fn test_format_S_big_endian() {
    assert_eq!(eval("binary scan [binary format S 1] H* h; set h"), "0001");
}

#[test]
fn test_format_i_le_w() {
    assert_eq!(
        eval("binary scan [binary format i 1] H* a; binary scan [binary format w 1] H* b; list $a $b"),
        "01000000 0100000000000000"
    );
}

#[test]
fn test_format_I_W_big_endian() {
    assert_eq!(
        eval("binary scan [binary format I 1] H* a; binary scan [binary format W 1] H* b; list $a $b"),
        "00000001 0000000000000001"
    );
}

#[test]
fn test_format_c2_takes_list() {
    assert_eq!(
        eval("binary scan [binary format c2 {1 2}] H* h; set h"),
        "0102"
    );
}

#[test]
fn test_format_c2_list_mismatch() {
    assert_eq!(
        eval_err("binary format c2 {1}"),
        "number of elements in list does not match count"
    );
}

#[test]
fn test_format_c_bad_integer() {
    assert_eq!(
        eval_err("binary format c xx"),
        "expected integer but got \"xx\""
    );
}

#[test]
fn test_format_c0_consumes_arg_but_emits_nothing() {
    assert_eq!(eval("binary scan [binary format c0 1] H* h; set h"), "");
    assert!(eval_err("binary format c0").contains("not enough arguments"));
}

// ── format: floats ─────────────────────────────────────────────────────

#[test]
fn test_format_d_1_0() {
    assert_eq!(
        eval("binary scan [binary format d 1.0] H* h; set h"),
        "000000000000f03f"
    );
}

#[test]
fn test_format_q_le_Q_be() {
    assert_eq!(
        eval("binary scan [binary format q 1.0] H* a; binary scan [binary format Q 1.0] H* b; list $a $b"),
        "000000000000f03f 3ff0000000000000"
    );
}

#[test]
fn test_format_f_le_r_le() {
    // f/r are 32-bit floats LE: 1.0 → 3f800000 reversed = 0000803f
    assert_eq!(
        eval("binary scan [binary format f 1.0] H* a; binary scan [binary format r 1.0] H* b; list $a $b"),
        "0000803f 0000803f"
    );
}

#[test]
fn test_format_R_big_endian_float() {
    assert_eq!(eval("binary scan [binary format R 1.0] H* h; set h"), "3f800000");
}

#[test]
fn test_format_f_bad_float() {
    assert_eq!(
        eval_err("binary format f zz"),
        "expected floating-point number but got \"zz\""
    );
}

// ── format: cursor ─────────────────────────────────────────────────────

#[test]
fn test_format_x_nuls() {
    assert_eq!(eval("binary scan [binary format x3] H* h; set h"), "000000");
}

#[test]
fn test_format_X_moves_back() {
    assert_eq!(eval("binary scan [binary format a1X1 ab] H* h; set h"), "61");
}

#[test]
fn test_format_X_clamps_at_zero() {
    assert_eq!(eval("binary scan [binary format a3X5 z] H* h; set h"), "7a0000");
}

#[test]
fn test_format_at_seeks_and_pads() {
    assert_eq!(eval("binary scan [binary format @3a1 z] H* h; set h"), "0000007a");
}

#[test]
fn test_format_at_requires_count() {
    assert_eq!(
        eval_err("binary format @"),
        "missing count for \"@\" field specifier"
    );
}

#[test]
fn test_format_bad_field_specifier() {
    assert_eq!(eval_err("binary format u 1"), "bad field specifier \"u\"");
    assert_eq!(eval_err("binary format F 1"), "bad field specifier \"F\"");
}

#[test]
fn test_format_not_enough_arguments() {
    assert_eq!(
        eval_err("binary format B"),
        "not enough arguments for all format specifiers"
    );
}

#[test]
fn test_format_no_args_usage() {
    assert_eq!(
        eval_err("binary format"),
        "wrong # args: should be \"binary format formatString ?arg ...?\""
    );
}

#[test]
fn test_format_multiple_fields_reuse_cursor() {
    // a2a2: two string fields concatenate
    assert_eq!(
        eval("binary scan [binary format a2a2 ab cd] H* h; set h"),
        "61626364"
    );
}

// ── scan ───────────────────────────────────────────────────────────────

#[test]
fn test_scan_c_signed() {
    assert_eq!(eval("binary scan \\xcf c v; set v"), "-49");
}

#[test]
fn test_scan_cu_unsigned() {
    assert_eq!(eval("binary scan \\xcf cu v; set v"), "207");
}

#[test]
fn test_scan_s_signed_su_unsigned() {
    assert_eq!(
        eval("binary scan \\xff\\xff s v; binary scan \\xff\\xff su w; list $v $w"),
        "-1 65535"
    );
}

#[test]
fn test_scan_S_big_endian() {
    assert_eq!(eval("binary scan \\x00\\x01 S v; set v"), "1");
}

#[test]
fn test_scan_i_overflow_positive() {
    assert_eq!(eval("binary scan \\xff\\xff\\xff\\x7f i v; set v"), "2147483647");
}

#[test]
fn test_scan_w_max() {
    assert_eq!(
        eval("binary scan \\xff\\xff\\xff\\xff\\xff\\xff\\xff\\x7f w v; set v"),
        "9223372036854775807"
    );
}

#[test]
fn test_scan_c2_produces_list() {
    assert_eq!(eval("binary scan \\x01\\x02 c2 v; list $v [llength $v]"), "{1 2} 2");
}

#[test]
fn test_scan_returns_var_count() {
    assert_eq!(
        eval("set n [binary scan \\x01\\x02 cc v w]; set n"),
        "2"
    );
}

#[test]
fn test_scan_partial_leaves_var_unset() {
    // c3 needs 3 bytes; only 2 → var unset, return 0
    assert_eq!(
        eval("unset -nocomplain v; set n [binary scan \\x01\\x02 c3 v]; list $n [info exists v]"),
        "0 0"
    );
}

#[test]
fn test_scan_a5_short_unset() {
    assert_eq!(
        eval("unset -nocomplain v; set n [binary scan abc a5 v]; list $n [info exists v]"),
        "0 0"
    );
}

#[test]
fn test_scan_a_stops_at_count() {
    assert_eq!(eval("binary scan abc\\x00 a3 v; set v"), "abc");
}

#[test]
fn test_scan_A_strips_trailing_ws_only_with_star() {
    assert_eq!(eval(r#"binary scan "ab\t  " A* v; set v"#), "ab");
    assert_eq!(eval(r#"binary scan "ab\t  " A3 v; set v"#), "ab\t");
}

#[test]
fn test_scan_b_bits_lsb_first() {
    // \x80: bit7 set → b8 → "00000001"
    assert_eq!(eval("binary scan \\x80 b8 v; set v"), "00000001");
}

#[test]
fn test_scan_B_bits_msb_first() {
    assert_eq!(eval("binary scan \\x80 B4 v; set v"), "1000");
}

#[test]
fn test_scan_h_lsb_nibble_first() {
    assert_eq!(eval("binary scan \\xab h2 v; set v"), "ba");
}

#[test]
fn test_scan_H_msb_nibble_first() {
    assert_eq!(eval("binary scan \\xab H2 v; set v"), "ab");
}

#[test]
fn test_scan_h_odd_count() {
    assert_eq!(eval("binary scan abc h5 v; set v"), "16263");
}

#[test]
fn test_scan_x_moves_cursor_no_var() {
    // x does not consume a varName
    assert_eq!(
        eval("binary scan \\x01\\x02 xc v w; list [set v] [info exists w]"),
        "2 0"
    );
}

#[test]
fn test_scan_at_seeks() {
    assert_eq!(eval("binary scan \\x01\\x02 @1c v; set v"), "2");
}

#[test]
fn test_scan_floats() {
    assert_eq!(
        eval(r#"binary scan \x00\x00\x00\x00\x00\x00\xf0\x3f d v; binary scan \x00\x00\x80\x3f f w; list $v $w"#),
        "1.0 1.0"
    );
}

#[test]
fn test_scan_too_few_varnames() {
    assert_eq!(
        eval_err("binary scan \\x01\\x02 cc v"),
        "not enough arguments for all format specifiers"
    );
}

#[test]
fn test_scan_extra_varnames_ok() {
    assert_eq!(
        eval("unset -nocomplain w; set n [binary scan \\x01 c v w]; list $n [info exists w]"),
        "1 0"
    );
}

#[test]
fn test_scan_no_varnames_usage() {
    assert_eq!(
        eval_err("binary scan abc"),
        "wrong # args: should be \"binary scan value formatString ?varName ...?\""
    );
}

#[test]
fn test_scan_u_standalone_is_bad_field() {
    assert_eq!(eval_err("binary scan \\x01 u v"), "bad field specifier \"u\"");
    assert_eq!(eval_err("binary scan \\x01\\x01 c2u v"), "bad field specifier \"u\"");
}

#[test]
fn test_scan_count_zero_sets_empty() {
    assert_eq!(
        eval("unset -nocomplain v; set n [binary scan abc a0 v]; list $n [set v]!"),
        "1 !"
    );
}

// ── string ↔ bytes mapping (non-ASCII) ────────────────────────────────

#[test]
fn test_format_a_takes_low_byte_per_char() {
    // tclsh 8.6.17: binary format a* €₽ → bytes ac bd
    assert_eq!(eval("binary scan [binary format a* €₽] H* h; set h"), "acbd");
}

#[test]
fn test_format_a_char_u0100_low_byte_zero() {
    assert_eq!(eval("binary scan [binary format a* Ā] H* h; set h"), "00");
}

#[test]
fn test_scan_h_of_euro() {
    // corpus binary-46.2: bytes ac bd scanned as s → -16980
    assert_eq!(
        eval("list [binary scan [binary format a* €₽] s x] $x"),
        "1 -16980"
    );
}

// ── encode/decode ──────────────────────────────────────────────────────

#[test]
fn test_encode_hex() {
    assert_eq!(eval("binary encode hex hello"), "68656c6c6f");
    assert_eq!(eval("binary encode hex {}"), "");
}

#[test]
fn test_decode_hex() {
    assert_eq!(eval("binary decode hex 68656c6c6f"), "hello");
    assert_eq!(eval("binary decode hex AbCd"), "\u{ab}\u{cd}");
}

#[test]
fn test_decode_hex_odd_pads_low_nibble() {
    // tclsh: abc → ab 0c
    assert_eq!(eval("binary scan [binary decode hex abc] H* h; set h"), "ab0c");
}

#[test]
fn test_decode_hex_invalid_digit() {
    assert_eq!(
        eval_err("binary decode hex zz"),
        "invalid hexadecimal digit \"z\" at position 0"
    );
}

#[test]
fn test_encode_base64() {
    assert_eq!(eval("binary encode base64 abc"), "YWJj");
    assert_eq!(eval("binary encode base64 ab"), "YWI=");
    assert_eq!(eval("binary encode base64 a"), "YQ==");
}

#[test]
fn test_decode_base64() {
    assert_eq!(eval("binary decode base64 YWJj"), "abc");
    assert_eq!(eval(r#"binary decode base64 "YW\nJj""#), "abc");
}

#[test]
fn test_decode_base64_skips_invalid_chars() {
    // tclsh: YW*J → ab (no error)
    assert_eq!(eval("binary decode base64 YW*J"), "ab");
}

#[test]
fn test_encode_base64_maxlen_wrapchar() {
    // tclsh: -maxlen 4 -wrapchar X abcdef → groups joined by X, no trailing
    assert_eq!(
        eval("binary encode base64 -maxlen 4 -wrapchar X abcdef"),
        "YWJjXZGVm"
    );
}

#[test]
fn test_encode_base64_maxlen_zero_no_wrap() {
    assert_eq!(
        eval("binary encode base64 -maxlen 0 abcdef"),
        "YWJjZGVm"
    );
}

#[test]
fn test_encode_base64_bad_option() {
    assert_eq!(
        eval_err("binary encode base64 -pad 0 ab"),
        "bad option \"-pad\": must be -maxlen or -wrapchar"
    );
}

#[test]
fn test_encode_uuencode() {
    // tclsh: binary encode uuencode abc → "#86)C\n"
    assert_eq!(eval("binary encode uuencode abc"), "#86)C\n");
}

#[test]
fn test_decode_uuencode() {
    assert_eq!(eval("binary decode uuencode #86)C\n"), "abc");
}

// ── subcommand dispatch ────────────────────────────────────────────────

#[test]
fn test_binary_bare_usage() {
    assert_eq!(
        eval_err("binary"),
        "wrong # args: should be \"binary subcommand ?arg ...?\""
    );
}

#[test]
fn test_binary_prefix_abbreviation() {
    // tclsh: `binary f` resolves to format, then arity error
    assert_eq!(
        eval_err("binary f"),
        "wrong # args: should be \"binary format formatString ?arg ...?\""
    );
}

#[test]
fn test_binary_unknown_subcommand() {
    assert_eq!(
        eval_err("binary foo"),
        "unknown or ambiguous subcommand \"foo\": must be decode, encode, format, or scan"
    );
}

#[test]
fn test_binary_encode_unknown() {
    assert_eq!(
        eval_err("binary encode zzz x"),
        "unknown subcommand \"zzz\": must be base64, hex, or uuencode"
    );
}

#[test]
fn test_binary_encode_hex_missing_data() {
    assert_eq!(
        eval_err("binary encode hex"),
        "wrong # args: should be \"binary encode hex data\""
    );
}

// ── scan default counts (tclsh 8.6.17: b/B→1 bit, h/H→1 nibble) ───────

#[test]
fn test_scan_b_default_count_is_one_bit() {
    // tclsh: binary scan \x82\x53 b v → v="0"
    assert_eq!(eval("unset -nocomplain v; list [binary scan \\x82\\x53 b v] $v"), "1 0");
    assert_eq!(eval("unset -nocomplain v; list [binary scan \\x82\\x53 B v] $v"), "1 1");
}

#[test]
fn test_scan_h_default_count_is_one_nibble() {
    // tclsh: \x82 → h="2", H="8"
    assert_eq!(eval("unset -nocomplain v; list [binary scan \\x82\\x53 h v] $v"), "1 2");
    assert_eq!(eval("unset -nocomplain v; list [binary scan \\x82\\x53 H v] $v"), "1 8");
}

#[test]
fn test_scan_b_star_all_bits() {
    assert_eq!(eval("unset -nocomplain v; list [binary scan \\x41 b* v] $v"), "1 10000010");
}
