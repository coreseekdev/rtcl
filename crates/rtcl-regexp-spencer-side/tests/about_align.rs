//! `regexp -about` wasm 侧模块对齐快照（native rtcl @ 99f30fc 实测）。
//!
//! oracle = specs/regex/about-probes.tcl 的 rtcl 侧基线（101 例：A 区
//! 56 + UIMPOSSIBLE 边界 21 + -expanded E 区 14 + flag 组合 X 区 10），
//! 由 native rtcl `-about` 逐例录得。侧模块 about_run 的输出按 native
//! 输出形（`numGroups flags...`，flags 0/1/multi 列表形）逐字对齐——
//! 本测试在两侧（regexp_cmds.rs 词法扫描器 / 侧模块复制件）漂移时
//! 即刻红。残余 4 例 about UIMPOSSIBLE 为 native 内部已知分歧（匹配
//! 无差异），快照按 native 原样录制，不在对齐范围内另行修正。

use spencermod::about_run;

fn put_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// 组装 about 输入 envelope：[pattern(bytes), flags(十进制文本)]。
fn about_input(pattern_hex: &str, flags: u32) -> Vec<u8> {
    let pat: Vec<u8> = (0..pattern_hex.len() / 2)
        .map(|i| u8::from_str_radix(&pattern_hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    let mut v = Vec::new();
    put_u32(&mut v, 2);
    put_u32(&mut v, pat.len() as u32);
    v.extend_from_slice(&pat);
    let fl = flags.to_string();
    put_u32(&mut v, fl.len() as u32);
    v.extend_from_slice(fl.as_bytes());
    v
}

/// 解析成功 envelope 并按 native 输出形渲染
/// `numGroups flagStr`（flagStr: 0 个 = `{}`、1 个 = 裸词、
/// 多个 = `{a b}` 列表形）。
fn render(out: &[u8]) -> String {
    let u32le = |o: usize| u32::from_le_bytes(out[o..o + 4].try_into().unwrap());
    assert_eq!(u32le(0), 0, "expected success envelope");
    let ngroups = u32le(4);
    let nflags = u32le(8) as usize;
    let mut flags = Vec::new();
    let mut o = 12usize;
    for _ in 0..nflags {
        let l = u32le(o) as usize;
        o += 4;
        flags.push(String::from_utf8(out[o..o + l].to_vec()).unwrap());
        o += l;
    }
    let flag_str = match flags.len() {
        0 => "{}".to_string(),
        1 => flags[0].clone(),
        _ => format!("{{{}}}", flags.join(" ")),
    };
    format!("{} {}", ngroups, flag_str)
}

const CASES: &[(&str, u32, &str)] = &[
    ("",                                            0, "0 {REG_UUNSPEC REG_UEMPTYMATCH}",                        ), // A
    ("616263",                                      0, "0 {}",                                                   ), // A
    ("612a",                                        0, "0 REG_UEMPTYMATCH",                                      ), // A
    ("612b",                                        0, "0 {}",                                                   ), // A
    ("613f",                                        0, "0 REG_UEMPTYMATCH",                                      ), // A
    ("617b307d",                                    0, "0 {REG_UBOUNDS REG_UEMPTYMATCH}",                        ), // A
    ("617b302c327d",                                0, "0 {REG_UBOUNDS REG_UEMPTYMATCH}",                        ), // A
    ("617b312c327d",                                0, "0 REG_UBOUNDS",                                          ), // A
    ("617b7c7d",                                    0, "0 {REG_UBRACES REG_UUNSPEC}",                            ), // A
    ("2829",                                        0, "1 {REG_UUNSPEC REG_UEMPTYMATCH}",                        ), // A
    ("283f3a29",                                    0, "0 {REG_UNONPOSIX REG_UUNSPEC REG_UEMPTYMATCH}",          ), // A
    ("283f3d6129",                                  0, "0 {REG_ULOOKAHEAD REG_UNONPOSIX}",                       ), // A
    ("28612a295c31",                                0, "1 {REG_UBACKREF REG_UNONPOSIX REG_UEMPTYMATCH}",         ), // A
    ("2861295c31",                                  0, "1 {REG_UBACKREF REG_UNONPOSIX}",                         ), // A
    ("617c",                                        0, "0 {REG_UUNSPEC REG_UEMPTYMATCH}",                        ), // A
    ("7c61",                                        0, "0 {REG_UUNSPEC REG_UEMPTYMATCH}",                        ), // A
    ("617c7c62",                                    0, "0 {REG_UUNSPEC REG_UEMPTYMATCH}",                        ), // A
    ("617b2c337d",                                  0, "0 {REG_UBRACES REG_UUNSPEC}",                            ), // A
    ("617b7d62",                                    0, "0 {REG_UBRACES REG_UUNSPEC}",                            ), // A
    ("617b627d",                                    0, "0 {REG_UBRACES REG_UUNSPEC}",                            ), // A
    ("617b",                                        0, "0 {REG_UBRACES REG_UUNSPEC}",                            ), // A
    ("617d",                                        0, "0 {}",                                                   ), // A
    ("283f6229",                                    0, "0 {REG_UNONPOSIX REG_UUNSPEC REG_UEMPTYMATCH}",          ), // A
    ("283f6229617b327d",                            0, "0 REG_UNONPOSIX",                                        ), // A
    ("283f6229615c7b325c7d",                        0, "0 {REG_UBOUNDS REG_UNONPOSIX}",                          ), // A
    ("283f62292829",                                0, "0 REG_UNONPOSIX",                                        ), // A
    ("283f65292829",                                0, "1 {REG_UNONPOSIX REG_UUNSPEC REG_UEMPTYMATCH}",          ), // A
    ("283f652928612929",                            0, "1 {REG_UPBOTCH REG_UNONPOSIX}",                          ), // A
    ("283f6529617c",                                0, "0 {REG_UNONPOSIX REG_UUNSPEC REG_UEMPTYMATCH}",          ), // A
    ("5c64",                                        0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // A
    ("5b5b3a616c7068613a5d5d",                      0, "0 REG_ULOCALE",                                          ), // A
    ("5c79",                                        0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UEMPTYMATCH}",          ), // A
    ("615c7962",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // A
    ("615c792062",                                  0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // A
    ("615c592062",                                  0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // A
    ("615c6d62",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // A
    ("615c4d63",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // A
    ("615c4d2063",                                  0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // A
    ("5c79",                                        0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UEMPTYMATCH}",          ), // A
    ("2461",                                        0, "0 REG_UIMPOSSIBLE",                                      ), // A
    ("612462",                                      0, "0 REG_UIMPOSSIBLE",                                      ), // A
    ("6124",                                        0, "0 {}",                                                   ), // A
    ("5e61",                                        0, "0 {}",                                                   ), // A
    ("615e",                                        0, "0 REG_UIMPOSSIBLE",                                      ), // A
    ("615c41",                                      0, "0 {REG_UNONPOSIX REG_UIMPOSSIBLE}",                      ), // A
    ("5c5a61",                                      0, "0 {REG_UNONPOSIX REG_UIMPOSSIBLE}",                      ), // A
    ("615c5a",                                      0, "0 REG_UNONPOSIX",                                        ), // A
    ("612a3f",                                      0, "0 {REG_UNONPOSIX REG_UEMPTYMATCH REG_USHORTEST}",        ), // A
    ("612b3f",                                      0, "0 {REG_UNONPOSIX REG_USHORTEST}",                        ), // A
    ("28612b2928612a3f29",                          0, "2 REG_UNONPOSIX",                                        ), // A
    ("28612b3f2928612a29",                          0, "2 {REG_UNONPOSIX REG_USHORTEST}",                        ), // A
    ("617b312c327d3f",                              0, "0 {REG_UBOUNDS REG_UNONPOSIX REG_USHORTEST}",            ), // A
    ("2a2a2a3d",                                    0, "0 {REG_UNONPOSIX REG_UUNSPEC REG_UEMPTYMATCH}",          ), // A
    ("2a2a2a3d286129",                              0, "0 REG_UNONPOSIX",                                        ), // A
    ("2a2a2a3a",                                    0, "0 {REG_UNONPOSIX REG_UUNSPEC REG_UEMPTYMATCH}",          ), // A
    ("2a2a2a3a286129",                              0, "1 REG_UNONPOSIX",                                        ), // A
    ("5e24612a5c79",                                0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e24612a5c59",                                0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UEMPTYMATCH}",          ), // I
    ("5e245c79",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e245c59",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UEMPTYMATCH}",          ), // I
    ("5c415c795c5a",                                0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5c415c595c5a",                                0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UEMPTYMATCH}",          ), // I
    ("5e612a5c7924",                                0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // I
    ("5e612a5c6d24",                                0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e612a5c4d24",                                0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // I
    ("612a5c795e24",                                0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5c795e24",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e5c4d",                                      0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("617b627d5c5a5c79",                            0, "0 {REG_UBRACES REG_UNONPOSIX REG_UUNSPEC REG_ULOCALE REG_UIMPOSSIBLE}",), // I
    ("615c5a5c79",                                  0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // I
    ("615c5a5c59",                                  0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("61245c79",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // I
    ("61245c59",                                    0, "0 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e24285c7929",                                0, "1 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e24285c797c6129",                            0, "1 {REG_UNONPOSIX REG_ULOCALE REG_UIMPOSSIBLE}",          ), // I
    ("5e24285c797c5c5929",                          0, "1 {REG_UNONPOSIX REG_ULOCALE REG_UEMPTYMATCH}",          ), // I
    ("5e24612a5c797c62",                            0, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // I
    ("6162",                                        8, "0 {}",                                                   ), // E
    ("612062",                                      8, "0 REG_UNONPOSIX",                                        ), // E
    ("61202062",                                    8, "0 REG_UNONPOSIX",                                        ), // E
    ("610962",                                      8, "0 REG_UNONPOSIX",                                        ), // E
    ("610a62",                                      8, "0 REG_UNONPOSIX",                                        ), // E
    ("6123630a62",                                  8, "0 REG_UNONPOSIX",                                        ), // E
    ("612023630a2062",                              8, "0 REG_UNONPOSIX",                                        ), // E
    ("23630a6162",                                  8, "0 REG_UNONPOSIX",                                        ), // E
    ("615c2062",                                    8, "0 {}",                                                   ), // E
    ("615b205d62",                                  8, "0 {}",                                                   ), // E
    ("612023205c645c6e2062",                        8, "0 REG_UNONPOSIX",                                        ), // E
    ("61202320283f3d78295c6e2062",                  8, "0 REG_UNONPOSIX",                                        ), // E
    ("612023207b322c337d0a2062",                    8, "0 REG_UNONPOSIX",                                        ), // E
    ("612023205b5b3a616c7068613a5d5d0a2062",        8, "0 REG_UNONPOSIX",                                        ), // E
    ("61205c64",                                    1, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // X
    ("61205c64",                                    6, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // X
    ("61205c64",                                    4, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // X
    ("61205c64",                                    2, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // X
    ("6128622961",                                  1, "1 {}",                                                   ), // X
    ("5e6124",                                      6, "0 {}",                                                   ), // X
    ("6162205c79",                                  1, "0 {REG_UNONPOSIX REG_ULOCALE}",                          ), // X
    ("61202c62",                                    8, "0 REG_UNONPOSIX",                                        ), // X
    ("2a2a2a3d612a",                                8, "0 REG_UNONPOSIX",                                        ), // X
    ("5c5c644b",                                    1, "0 {}",                                                   ), // X
];

#[test]
fn about_matches_native_rtcl_baseline() {
    let mut n_bad = 0;
    for (hex, flags, want) in CASES {
        let out = about_run(&about_input(hex, *flags));
        let got = render(&out);
        if got != *want {
            n_bad += 1;
            eprintln!("MISMATCH hex={} flags={} want=<{}> got=<{}>", hex, flags, want, got);
        }
    }
    assert_eq!(n_bad, 0, "{} / {} about cases diverge from native rtcl baseline", n_bad, CASES.len());
}
