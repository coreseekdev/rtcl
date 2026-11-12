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
//!   res_ptr() -> *const u8          结果 envelope 指针
//!   res_len() -> u32                结果 envelope 长度
//!
//! 输入 envelope（LE u32 前缀长度 + UTF-8 字节记录）：
//!   u32 nargs(=5) | per arg: u32 len + bytes
//!   args = [pattern, subject, flags, start, all]
//!   flags 位（spencer_options 同映射；newline 不经命令面）：
//!     1=-nocase  2=-linestop  4=-lineanchor  8=-expanded  16=include_end
//!     （-line = 2|4；expanded 由引擎原生支持——JS 子集缺失项；
//!     include_end = regsub 的 -all 循环形：允许 p == len 的尾空匹配）
//!   start = -start 字符偏移（可越过串尾——虚拟起点语义）
//!   all   = 0/1
//!
//! 输出 envelope：
//!   tag 0（成功）：u32 0 | u32 ngroups | u32 nmatches
//!     | per match: ngroups × 2 × i32（**字符起止**，端点独占；
//!     未参与 = -1,-1）。引擎内部 span 即字符索引——native 的字节
//!     span 仅是命令层中间量，用户面（-indices / string range）皆
//!     字符单位，侧模块直出字符避免宿主二次换算。
//!   tag 1（编译错误）：u32 1 | u32 codeLen + code | u32 msgLen + msg
//!     （code = tclsh REG_* 名；msg 与 native Error 同文）
//!
//! -all 迭代语义与 native collect_matches 精确对齐：
//!   from = search_start = 每次尝试的字符起点（\A 绑定扫描起点）；
//!   空匹配推进一字符、非空匹配推到端点；regexp -all 首位允许
//!   p == len、此后 p >= len 即止；-start 越尾 = 虚拟起点（在串尾
//!   尝试、索引按原始 -start 计）——虚拟判定与索引 delta 由宿主
//!   wrapper 处理，本模块只收 effective start 与 raw search_start。
#![allow(static_mut_refs)]

use rtcl_regexp_spencer::captures;
use rtcl_regexp_spencer::matcher::Matcher;
use rtcl_regexp_spencer::parser::Options;
use rtcl_regexp_spencer::{compile, Error as RegError};

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
