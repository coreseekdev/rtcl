//! spencermod — rtcl dynmod 侧模块：Spencer ARE 引擎全集
//! （wasm 构建的 regexp/regsub 从 JS 语义子集升级为 native 同源引擎）。
//!
//! 与 dynmod-side 同款形态：独立编译的 wasm32-unknown-unknown cdylib，
//! 数值 ABI；宿主 JS 胶水（rtcl-worker.js）负责字节 ↔ 本模块线性内存
//! 的搬运与 envelope 编解码。
//!
//! 导出面：
//!   spencer_abi() -> i32            协议版本（=1）
//!   exec(in_ptr, in_len) -> i32     单次调用：解析输入 envelope、
//!                                   编译执行、写结果 envelope；
//!                                   返回 0（含编译错误——tag 区分）
//!   about(in_ptr, in_len) -> i32    `regexp -about` 分析面（native
//!                                   cmd_regexp -about 分支同构）：
//!                                   编译 + ast 分析，写结果 envelope
//!   res_ptr() -> *const u8          结果 envelope 指针
//!   res_len() -> u32                结果 envelope 长度
//!
//! 输入 envelope（LE u32 前缀长度 + UTF-8 字节记录）：
//!   exec:  u32 nargs(=5) | per arg: u32 len + bytes
//!     args = [pattern, subject, flags, start, all]
//!   about: u32 nargs(=2) | per arg: u32 len + bytes
//!     args = [pattern, flags]（pattern 在首）
//!   flags 位（spencer_options 同映射；newline 不经命令面）：
//!     1=-nocase  2=-linestop  4=-lineanchor  8=-expanded  16=include_end
//!     （-line = 2|4；expanded 由引擎原生支持——JS 子集缺失项；
//!     include_end = regsub 的 -all 循环形：允许 p == len 的尾空匹配）
//!   start = -start 字符偏移（可越过串尾——虚拟起点语义）
//!   all   = 0/1
//!
//! 输出 envelope：
//!   exec：
//!   tag 0（成功）：u32 0 | u32 ngroups | u32 nmatches
//!     | per match: ngroups × 2 × i32（**字符起止**，端点独占；
//!     未参与 = -1,-1）。引擎内部 span 即字符索引——native 的字节
//!     span 仅是命令层中间量，用户面（-indices / string range）皆
//!     字符单位，侧模块直出字符避免宿主二次换算。
//!   about：
//!   tag 0（成功）：u32 0 | u32 ngroups | u32 nflags
//!     | per flag: u32 len + bytes（REG_* 名，tclsh re_info 位序）
//!     ngroups = prog.nsubs（捕获组数，不含组 0）；flag 集/位序与
//!     native `regexp -about` 输出逐字段对齐（对齐 oracle =
//!     specs/regex/about-probes.tcl 的 rtcl 侧基线，快照测试见
//!     tests/about_align.rs）。
//!   tag 1（编译错误，exec/about 共用形）：
//!     u32 1 | u32 codeLen + code | u32 msgLen + msg
//!     （code = tclsh REG_* 名；msg 与 native Error 同文）
//!
//! -all 迭代语义与 native collect_matches 精确对齐：
//!   from = search_start = 每次尝试的字符起点（\A 绑定扫描起点）；
//!   空匹配推进一字符、非空匹配推到端点；regexp -all 首位允许
//!   p == len、此后 p >= len 即止；-start 越尾 = 虚拟起点（在串尾
//!   尝试、索引按原始 -start 计）——虚拟判定与索引 delta 由宿主
//!   wrapper 处理，本模块只收 effective start 与 raw search_start。
#![allow(static_mut_refs)]

use rtcl_regexp_spencer::ast;
use rtcl_regexp_spencer::captures;
use rtcl_regexp_spencer::matcher::Matcher;
use rtcl_regexp_spencer::parser::Options;
use rtcl_regexp_spencer::{compile, compile_with_options, Error as RegError};

static mut RES: Option<Vec<u8>> = None;

fn put_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put_i32(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}

#[no_mangle]
pub extern "C" fn spencer_abi() -> i32 {
    1
}

#[no_mangle]
pub extern "C" fn exec(in_ptr: *const u8, in_len: u32) -> i32 {
    let input = if in_ptr.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(in_ptr, in_len as usize) }
    };
    let out = run(input);
    unsafe { RES = Some(out) };
    0
}

#[no_mangle]
pub extern "C" fn res_ptr() -> *const u8 {
    unsafe {
        match &RES {
            Some(v) => v.as_ptr(),
            None => std::ptr::null(),
        }
    }
}

#[no_mangle]
pub extern "C" fn res_len() -> u32 {
    unsafe { RES.as_ref().map(|v| v.len() as u32).unwrap_or(0) }
}

/// 输入缓冲（JS 胶水 alloc 后写入；单线程 JS，静态复用）。
static mut INPUT: Option<Vec<u8>> = None;

#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    unsafe {
        let v = INPUT.get_or_insert_with(Vec::new);
        v.clear();
        v.resize(len as usize, 0);
        v.as_mut_ptr()
    }
}

/// 输入解析失败/致命错误：tag 1 envelope，code = REG_BADPAT 概括。
fn fatal(msg: &str) -> Vec<u8> {
    err_envelope("REG_BADPAT", msg)
}

fn err_envelope(code: &str, msg: &str) -> Vec<u8> {
    let mut v = Vec::new();
    put_u32(&mut v, 1);
    put_u32(&mut v, code.len() as u32);
    v.extend_from_slice(code.as_bytes());
    put_u32(&mut v, msg.len() as u32);
    v.extend_from_slice(msg.as_bytes());
    v
}

fn err_of(e: &RegError) -> Vec<u8> {
    err_envelope(e.code, &e.message)
}

/// 记录解析：u32 len + bytes。
fn read_records(input: &[u8], nargs: u32) -> Result<Vec<&[u8]>, ()> {
    let mut pos = 4usize;
    let mut out = Vec::with_capacity(nargs as usize);
    for _ in 0..nargs {
        if pos + 4 > input.len() {
            return Err(());
        }
        let n = u32::from_le_bytes(input[pos..pos + 4].try_into().ok().ok_or(())?) as usize;
        pos += 4;
        if pos + n > input.len() {
            return Err(());
        }
        out.push(&input[pos..pos + n]);
        pos += n;
    }
    Ok(out)
}

fn run(input: &[u8]) -> Vec<u8> {
    let recs = match read_records(input, 5) {
        Ok(r) => r,
        Err(_) => return fatal("malformed request envelope"),
    };
    let pattern = match std::str::from_utf8(recs[0]) {
        Ok(s) => s,
        Err(_) => return fatal("pattern is not valid UTF-8"),
    };
    let subject = match std::str::from_utf8(recs[1]) {
        Ok(s) => s.to_string(),
        Err(_) => return fatal("subject is not valid UTF-8"),
    };
    let num = |b: &[u8]| -> u32 {
        std::str::from_utf8(b)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let flags = num(recs[2]);
    let start = num(recs[3]) as usize;
    let all = num(recs[4]) != 0;
    let include_end = flags & 16 != 0;

    let opts = Options {
        icase: flags & 1 != 0,
        nlstop: flags & 2 != 0,
        nlanch: flags & 4 != 0,
        expanded: flags & 8 != 0,
        ..Default::default()
    };
    let prog = match compile(pattern, opts) {
        Ok(p) => p,
        Err(e) => return err_of(&e),
    };

    let chars: Vec<char> = subject.chars().collect();
    let char_len = chars.len();
    let char_start = start.min(char_len);
    let virtual_start = start > char_len;

    let mut matcher = Matcher::new(&prog);
    let ngroups = prog.nsubs + 1;

    // collect_matches 同构（regexp 形：include_end=false）。
    let mut out = Vec::new();
    put_u32(&mut out, 0);
    put_u32(&mut out, ngroups as u32);
    let nmatches_at = out.len();
    put_u32(&mut out, 0); // nmatches 占位

    let mut nmatches: u32 = 0;
    if !all {
        // 非 -all：单次尝试（虚拟起点：p=串尾、search_start=原始值）。
        let ss = if virtual_start { start } else { char_start };
        if let Some(c) = captures::exec_with(&prog, &chars, char_start, ss, &mut matcher) {
            push_match(&mut out, &c);
            nmatches = 1;
        }
    } else if start > char_len {
        // regexp -all 且 -start 越尾：虚拟起点——首个位置允许 p==len，
        // 尝试一次后按 p>=len 止。
        if let Some(c) = captures::exec_with(&prog, &chars, char_len, start, &mut matcher) {
            push_match(&mut out, &c);
            nmatches = 1;
        }
    } else {
        let mut p = char_start;
        let mut first = true;
        loop {
            if !first && (if include_end { p > char_len } else { p >= char_len }) {
                break;
            }
            first = false;
            match captures::exec_with(&prog, &chars, p, p, &mut matcher) {
                Some(c) => {
                    let (s, e) = c.spans[0].unwrap_or((p, p));
                    push_match(&mut out, &c);
                    nmatches += 1;
                    // 空匹配推进一字符、非空匹配推到端点（spans 为
                    // 字符索引；字节换算只发生在输出侧）。
                    p = e.max(s + 1);
                }
                None => break,
            }
        }
    }
    out[nmatches_at..nmatches_at + 4].copy_from_slice(&nmatches.to_le_bytes());
    out
}

fn push_match(out: &mut Vec<u8>, c: &captures::Captures) {
    for span in &c.spans {
        match span {
            Some((s, e)) => {
                put_i32(out, *s as i32);
                put_i32(out, *e as i32);
            }
            None => {
                put_i32(out, -1);
                put_i32(out, -1);
            }
        }
    }
}

// ── about：`regexp -about` 分析面（native 同形）──────────────────────
//
// 与 rtcl-vm interp/commands/regexp_cmds.rs 的 cmd_regexp -about 分支
// 逐字段同构：compile_with_options + ast 分析（nullable_matchable /
// impossible / has_bracket_range / flags.shorter）+ 词法扫描器（本文件
// 下方 `-about 词法扫描器` 节，自 regexp_cmds.rs 平移的纯字节扫描，源
// 头注释见彼处）→ REG_* 标志集按 tclsh re_info 位序输出。
// 对齐 oracle = specs/regex/about-probes.tcl 的 rtcl 侧基线；
// tests/about_align.rs 快照全量护栏（101 例）。
//
// 输出 envelope：tag 0 | u32 ngroups | u32 nflags | per-flag (len+bytes)。
// 宿主 wrapper 据此组装 native 形 `numGroups flags...`（flags 0/1/multi
// 的 Tcl 列表形由 wrapper 层负责）。
pub fn about_run(input: &[u8]) -> Vec<u8> {
    let recs = match read_records(input, 2) {
        Ok(r) => r,
        Err(_) => return fatal("malformed request envelope"),
    };
    let pattern = match std::str::from_utf8(recs[0]) {
        Ok(s) => s,
        Err(_) => return fatal("pattern is not valid UTF-8"),
    };
    let num = |b: &[u8]| -> u32 {
        std::str::from_utf8(b)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let flags = num(recs[1]);
    let opts = Options {
        icase: flags & 1 != 0,
        nlstop: flags & 2 != 0,
        nlanch: flags & 4 != 0,
        expanded: flags & 8 != 0,
        ..Default::default()
    };
    let (prog, effective) = match compile_with_options(pattern, opts) {
        Ok(p) => p,
        Err(e) => return err_of(&e),
    };

    // 引擎分析位（native engine_about_bits 同：可空 / 不可命中 /
    // 根偏好 SHORTER / 类内范围）。
    let empty_ok = ast::nullable_matchable(&prog.tree);
    let impossible = ast::impossible(&prog.tree);
    let shortest = prog.flags.shorter;
    let bracket_range = ast::has_bracket_range(&prog.tree);

    // 生效选项（AboutEff 同：expanded/quote/bre/ere）。
    let eff_expanded = effective.expanded;
    let quote = effective.quote;
    let escape_in_class = !(effective.bre || effective.ere);
    let (scan_owned, expanded_ignored) = if eff_expanded && !quote {
        strip_expanded(pattern, escape_in_class)
    } else {
        (pattern.to_string(), false)
    };
    let scan = scan_owned.as_str();
    let nongreedy = !quote && pattern_is_nongreedy(scan);
    let lex = about_lex(scan);
    let ubbs = pattern_has_class_backslash(scan);

    // 位序与 tclsh 的 re_info 一致（native 同序）：UBACKREF →
    // ULOOKAHEAD → UBOUNDS → UBRACES → UBSALNUM → UPBOTCH → UBBS →
    // UNONPOSIX → UUNSPEC → UUNPORT → ULOCALE → UEMPTYMATCH →
    // UIMPOSSIBLE → USHORTEST。
    let mut out_flags: Vec<&str> = Vec::new();
    if lex.backref {
        out_flags.push("REG_UBACKREF");
    }
    if pattern_has_lookahead(scan) {
        out_flags.push("REG_ULOOKAHEAD");
    }
    if lex.bounds {
        out_flags.push("REG_UBOUNDS");
    }
    if lex.braces {
        out_flags.push("REG_UBRACES");
    }
    if lex.ubsalnum {
        out_flags.push("REG_UBSALNUM");
    }
    if pattern_has_upbotch(scan) {
        out_flags.push("REG_UPBOTCH");
    }
    if ubbs {
        out_flags.push("REG_UBBS");
    }
    if ubbs || expanded_ignored || pattern_is_nonposix(scan, nongreedy) {
        out_flags.push("REG_UNONPOSIX");
    }
    if lex.unspec {
        out_flags.push("REG_UUNSPEC");
    }
    if bracket_range {
        out_flags.push("REG_UUNPORT");
    }
    // -nocase 本身不置 ULOCALE；只有 Unicode 依赖的类才置。
    if pattern_uses_ulocale(scan) {
        out_flags.push("REG_ULOCALE");
    }
    // 不可命中的模式 tclsh 不再置 UEMPTYMATCH。
    if empty_ok && !impossible {
        out_flags.push("REG_UEMPTYMATCH");
    }
    if impossible {
        out_flags.push("REG_UIMPOSSIBLE");
    }
    // Spencer 根偏好 SHORTER 才置位（native：引擎 flags.shorter）。
    if shortest {
        out_flags.push("REG_USHORTEST");
    }

    let mut out = Vec::new();
    put_u32(&mut out, 0);
    put_u32(&mut out, prog.nsubs as u32);
    put_u32(&mut out, out_flags.len() as u32);
    for f in &out_flags {
        put_u32(&mut out, f.len() as u32);
        out.extend_from_slice(f.as_bytes());
    }
    out
}

/// about 入口（exec 同款：结果写静态 RES，返回 0）。
#[no_mangle]
pub extern "C" fn about(in_ptr: *const u8, in_len: u32) -> i32 {
    let input = if in_ptr.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(in_ptr, in_len as usize) }
    };
    let out = about_run(input);
    unsafe { RES = Some(out) };
    0
}

// ── `-about` 词法扫描器（自 regexp_cmds.rs 平移的纯字节扫描）────────
//
// 单一事实源 = crates/rtcl-vm/src/interp/commands/regexp_cmds.rs
// （pattern_is_nongreedy / pattern_is_nonposix / pattern_has_lookahead /
// pattern_uses_ulocale / pattern_has_upbotch / about_lex 及其辅助 /
// strip_expanded / pattern_has_class_backslash）。native 侧零改动约束
// 下在侧模块复制本实现；tests/about_align.rs 快照护栏在两侧漂移时
// 即刻红（对齐 oracle = specs/regex/about-probes.tcl rtcl 侧基线）。

/// `-about` 的语法扫描结果（与 tclsh 的 re_info 位一一对应）。
#[derive(Default)]
struct AboutLex {
    backref: bool,
    bounds: bool,
    braces: bool,
    ubsalnum: bool,
    unspec: bool,
}

/// 前缀里声明的方言：`(bre, ere)`；默认 ARE。
fn about_dialect(pat: &str) -> (bool, bool) {
    let p = pat.strip_prefix("***:").unwrap_or(pat);
    if let Some(rest) = p.strip_prefix("(?") {
        if let Some(end) = rest.find(')') {
            let letters = &rest[..end];
            if letters.chars().all(|c| c.is_ascii_alphabetic()) {
                let bre = letters.contains('b');
                let ere = !bre && letters.contains('e');
                return (bre, ere);
            }
        }
    }
    (false, false)
}

/// `{m}` / `{m,}` / `{m,n}`（ARE/ERE），返回 `}` 之后的位置。
fn about_valid_bound(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let start = j;
    let mut m = 0u32;
    while j < b.len() && b[j].is_ascii_digit() {
        m = m.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
        j += 1;
    }
    if j == start {
        return None;
    }
    let mut n = m;
    let mut unbounded = false;
    if j < b.len() && b[j] == b',' {
        j += 1;
        let ds = j;
        let mut v = 0u32;
        while j < b.len() && b[j].is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
            j += 1;
        }
        if j == ds {
            unbounded = true;
        } else {
            n = v;
        }
    }
    if j >= b.len() || b[j] != b'}' {
        return None;
    }
    if m > 255 || (!unbounded && n > 255) || m > n {
        return None;
    }
    Some(j + 1)
}

/// BRE `\{m,n\}`，返回 `\}` 之后的位置（m 可省略）。
fn about_valid_bre_bound(b: &[u8], i: usize) -> Option<usize> {
    if i + 1 >= b.len() || b[i] != b'\\' || b[i + 1] != b'{' {
        return None;
    }
    let mut j = i + 2;
    let mut m = 0u32;
    let ds = j;
    while j < b.len() && b[j].is_ascii_digit() {
        m = m.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
        j += 1;
    }
    let had_m = j > ds;
    let mut n = m;
    let mut unbounded = false;
    if j < b.len() && b[j] == b',' {
        j += 1;
        let ds2 = j;
        let mut v = 0u32;
        while j < b.len() && b[j].is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add((b[j] - b'0') as u32);
            j += 1;
        }
        if j == ds2 {
            unbounded = true;
        } else {
            n = v;
        }
    } else if !had_m {
        return None;
    }
    if j + 1 >= b.len() || b[j] != b'\\' || b[j + 1] != b'}' {
        return None;
    }
    if m > 255 || (!unbounded && n > 255) || m > n {
        return None;
    }
    Some(j + 2)
}

/// 去掉 expanded 模式忽略的空白与 `#` 注释（保留转义字符与字符类
/// 内容），返回 (扫描串, 是否真的忽略过内容)。
fn strip_expanded(pat: &str, escape_in_class: bool) -> (String, bool) {
    let b = pat.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let (mut i, mut in_class, mut ignored) = (0usize, false, false);
    while i < b.len() {
        let c = b[i];
        if in_class {
            if c == b'\\' && escape_in_class && i + 1 < b.len() {
                out.push(c);
                out.push(b[i + 1]);
                i += 2;
                continue;
            }
            out.push(c);
            if c == b']' {
                in_class = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'[' => {
                in_class = true;
                out.push(c);
                i += 1;
            }
            b'\\' if i + 1 < b.len() => {
                out.push(c);
                out.push(b[i + 1]);
                i += 2;
            }
            b'#' => {
                ignored = true;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                if i < b.len() {
                    i += 1;
                }
            }
            c if c.is_ascii_whitespace() => {
                ignored = true;
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    (
        String::from_utf8(out).unwrap_or_else(|_| pat.to_string()),
        ignored,
    )
}

/// 字符类里出现反斜杠（任意方言）→ REG_UBBS。`***=` 字面模式除外。
fn pattern_has_class_backslash(pat: &str) -> bool {
    if pat.starts_with("***=") {
        return false;
    }
    let b = pat.as_bytes();
    let mut in_class = false;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'[' if !in_class => {
                in_class = true;
                i += 1;
            }
            b']' if in_class => {
                in_class = false;
                i += 1;
            }
            b'\\' if in_class => return true,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    false
}

/// 扫描 pattern（跳过前缀/字符类/转义）收集 -about 语法位。
fn about_lex(pat: &str) -> AboutLex {
    let mut out = AboutLex::default();
    let b = pat.as_bytes();
    let mut i = 0usize;
    let mut quoted = false;
    if pat.starts_with("***=") {
        quoted = true;
        i = 4;
    } else if pat.starts_with("***:") {
        i = 4;
    }
    let mut bre = false;
    let mut ere = false;
    // 开头的内嵌选项组（(?b) / (?e) / (?i) ...）。
    if !quoted {
        let (bflag, eflag) = about_dialect(pat);
        bre = bflag;
        ere = eflag;
        if b.get(i) == Some(&b'(') && b.get(i + 1) == Some(&b'?') {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            if b.get(j) == Some(&b')') {
                i = j + 1;
            }
        }
    }
    if quoted {
        out.unspec = i >= b.len();
        return out;
    }
    let are = !bre && !ere;
    if i >= b.len() {
        out.unspec = true;
        return out;
    }
    let mut in_class = false;
    // 上一个 token 是否可作分支端点（用于空分支 UUNSPEC）。
    let mut prev_atom = false;
    while i < b.len() {
        let c = b[i];
        if in_class {
            if c == b']' {
                in_class = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'[' => {
                in_class = true;
                i += 1;
                prev_atom = true;
            }
            b'\\' if i + 1 < b.len() => {
                let e = b[i + 1];
                if bre {
                    if e.is_ascii_digit() && e != b'0' {
                        out.backref = true;
                    } else if e.is_ascii_alphanumeric() {
                        // BRE 没有 `\d` 等转义：`\`+字母数字是
                        // 未指定语法（tclsh: UBSALNUM + UUNSPEC）。
                        out.ubsalnum = true;
                        out.unspec = true;
                    }
                    if e == b'{' {
                        if let Some(end) = about_valid_bre_bound(b, i) {
                            out.bounds = true;
                            i = end;
                            prev_atom = true;
                            continue;
                        }
                    }
                    if e == b'(' && b.get(i + 2) == Some(&b'\\')
                        && b.get(i + 3) == Some(&b')')
                    {
                        out.unspec = true;
                    }
                    i += 2;
                    prev_atom = true;
                } else {
                    if are && e.is_ascii_digit() && e != b'0' {
                        out.backref = true;
                    } else if ere && e.is_ascii_alphanumeric() {
                        out.ubsalnum = true;
                        out.unspec = true;
                    }
                    i += 2;
                    prev_atom = true;
                }
            }
            b'{' if !bre => {
                if let Some(end) = about_valid_bound(b, i) {
                    out.bounds = true;
                    i = end;
                } else {
                    out.braces = true;
                    out.unspec = true;
                    i += 1;
                }
                prev_atom = true;
            }
            b'(' if !bre => {
                // 空组 `()` / `(?:)` 是 POSIX 未指定行为。
                if b.get(i + 1) == Some(&b')')
                    || (b.get(i + 1) == Some(&b'?')
                        && b.get(i + 2) == Some(&b':')
                        && b.get(i + 3) == Some(&b')'))
                {
                    out.unspec = true;
                }
                i += 1;
                prev_atom = false;
            }
            b')' if !bre => {
                i += 1;
                prev_atom = true;
            }
            b'|' if !bre => {
                let next = b.get(i + 1).copied();
                if !prev_atom
                    || next.is_none()
                    || next == Some(b'|')
                    || next == Some(b')')
                {
                    out.unspec = true;
                }
                i += 1;
                prev_atom = false;
            }
            _ => {
                i += 1;
                prev_atom = true;
            }
        }
    }
    out
}

/// Non-greedy quantifier present (`+?` `*?` `??` `}?`)?
fn pattern_is_nongreedy(pat: &str) -> bool {
    let b = pat.as_bytes();
    for i in 1..b.len() {
        if b[i] == b'?' && matches!(b[i - 1], b'+' | b'*' | b'?' | b'}') {
            return true;
        }
    }
    false
}

/// Constructs POSIX ERE cannot express (Perl classes like `\d`,
/// lookaround, `(?` groups)?  Non-greedy quantifiers count too.
fn pattern_is_nonposix(pat: &str, nongreedy: bool) -> bool {
    if nongreedy {
        return true;
    }
    // `***=` / `***:` 元语法本身就是非 POSIX 扩展。
    if pat.starts_with("***=") || pat.starts_with("***:") {
        return true;
    }
    let b = pat.as_bytes();
    let (mut in_class, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => {
                if i + 1 < b.len() && b[i + 1].is_ascii_alphanumeric() {
                    return true;
                }
                i += 2;
            }
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class && b.get(i + 1) == Some(&b'?') => return true,
            _ => i += 1,
        }
    }
    false
}

/// `(?=` / `(?!` outside classes and escapes（-about 的
/// REG_ULOOKAHEAD；语法上是 non-POSIX，与 REG_UNONPOSIX 并列输出）。
fn pattern_has_lookahead(pat: &str) -> bool {
    if pat.starts_with("***=") {
        return false;
    }
    let (bre, ere) = about_dialect(pat);
    if bre || ere {
        return false;
    }
    let b = pat.as_bytes();
    let (mut in_class, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class => {
                if b.get(i + 1) == Some(&b'?')
                    && matches!(b.get(i + 2), Some(b'=') | Some(b'!'))
                {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// Tcl 里 REG_ULOCALE 表示用到了 Unicode 依赖的类：Perl 类
/// `\d\s\w` 及其反集、词断言 `\y\m\M\Y`、POSIX `[[:class:]]`。
/// `-nocase` 本身不产生该标志（tclsh 8.6.17 实测）。
fn pattern_uses_ulocale(pat: &str) -> bool {
    if pat.starts_with("***=") {
        return false;
    }
    let (bre, ere) = about_dialect(pat);
    let escapes_locale = !bre && !ere;
    let b = pat.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() => {
                if escapes_locale
                    && matches!(
                        b[i + 1],
                        b'd' | b'D' | b's' | b'S' | b'w' | b'W' | b'y' | b'm' | b'M' | b'Y'
                    )
                {
                    return true;
                }
                i += 2;
            }
            b'[' => {
                // `[[:alpha:]]`：POSIX 类名（Unicode 表）。
                if b.get(i + 1) == Some(&b'[') && b.get(i + 2) == Some(&b':') {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// ERE 方言（`(?e)` / `***:(?e)`）下存在多余的 `)`——tclsh 把它当
/// 字面字符并在 -about 里报 REG_UPBOTCH。
fn pattern_has_upbotch(pat: &str) -> bool {
    let p = pat.strip_prefix("***:").unwrap_or(pat);
    let Some(rest) = p.strip_prefix("(?") else {
        return false;
    };
    let Some(end) = rest.find(')') else {
        return false;
    };
    let letters = &rest[..end];
    if !letters.contains('e') || letters.contains('b') {
        return false;
    }
    let b = rest[end + 1..].as_bytes();
    let (mut depth, mut in_class, mut i) = (0usize, false, 0usize);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' => { in_class = true; i += 1; }
            b']' => { in_class = false; i += 1; }
            b'(' if !in_class => { depth += 1; i += 1; }
            b')' if !in_class => {
                if depth == 0 {
                    return true;
                }
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}
