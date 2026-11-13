//! dynmod 侧模块示例 —— 一个最小的 rtcl 扩展模块（wasm32-unknown-unknown）。
//!
//! 导出面：
//!   add(a, b) -> i64                数值 ABI（rtcl 侧包装为 ::myext::add，
//!                                   参数字符串进；裸 wasm i64 是 BigInt）
//!   version() -> i32
//!   argc(in_ptr, in_len) -> i32     envelope 记录数（"到达词数" oracle）
//!   echo_words(in_ptr, in_len) -> i32  envelope 逐词回显 → res envelope
//!   alloc(len) -> *mut u8           输入缓冲（JS 胶水写入；静态定长区）
//!   res_ptr() -> *const u8          echo 结果 envelope 指针
//!   res_len() -> u32                echo 结果 envelope 长度
//!
//! envelope 记录表（与 spencermod 同形：LE u32 前缀长度 + 字节记录）：
//!   u32 nrecords | per record: u32 len + bytes
//! v2 dynmod 参数协议（DynAbi::V2List，宿主 wrapper 体 `{*}$args` 逐词
//! 直传）下每个调用实参 = 一条记录——argc/echo_words 是参数逐词到达与
//! 数据保真（\u0001 字面 / 空白词 / 全 Unicode / 空串）的 wasm 面
//! oracle，dynmod-test.html V 区据此展示。
//!
//! 侧模块无法自注册（与宿主无共享地址空间）——命令面由宿主 JS 胶水
//! 经 RtclHandle::register_module 声明，`load myext` 挂线。
//!
//! 内存形态：no_std 极简——无堆（无 global_allocator），输入/输出走
//! 16 KiB 静态定长区（单线程 JS 安全；spencermod 的静态 RES 同思路）。
#![no_std]
#![allow(static_mut_refs)]

use core::panic::PanicInfo;

const BUF_CAP: usize = 16 * 1024;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

/// 输入缓冲（JS 胶水 alloc 后写入；单线程 JS，静态复用）。
static mut INPUT_BUF: [u8; BUF_CAP] = [0; BUF_CAP];

/// echo 结果 envelope（静态定长区 + 长度，spencermod 静态 RES 同思路）。
static mut RES_BUF: [u8; BUF_CAP] = [0; BUF_CAP];
static mut RES_LEN: usize = 0;

#[no_mangle]
pub extern "C" fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[no_mangle]
pub extern "C" fn version() -> i32 {
    1
}

/// 输入缓冲申请：返回静态区指针（超出容量 → null）。
#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    if len as usize > BUF_CAP {
        return core::ptr::null_mut();
    }
    unsafe { INPUT_BUF.as_mut_ptr() }
}

#[no_mangle]
pub extern "C" fn res_ptr() -> *const u8 {
    unsafe { RES_BUF.as_ptr() }
}

#[no_mangle]
pub extern "C" fn res_len() -> u32 {
    unsafe { RES_LEN as u32 }
}

/// envelope 记录表一遍校验走表（无分配）：返回 Ok((记录数, 总字节数
/// 含头))；形态不合法（越界/截断）→ None。
fn walk_records(input: &[u8]) -> Option<(usize, usize)> {
    if input.len() < 4 {
        return None;
    }
    let n = u32::from_le_bytes(input[0..4].try_into().ok()?) as usize;
    let mut pos = 4usize;
    for _ in 0..n {
        if pos + 4 > input.len() {
            return None;
        }
        let l = u32::from_le_bytes(input[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;
        if pos + l > input.len() {
            return None;
        }
        pos += l;
    }
    Some((n, pos))
}

/// argc：到达词数（记录数）。逐条走表——形态损坏 → -1。
#[no_mangle]
pub extern "C" fn argc(in_ptr: *const u8, in_len: u32) -> i32 {
    let input = if in_ptr.is_null() {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(in_ptr, in_len as usize) }
    };
    match walk_records(input) {
        Some((n, _)) => n as i32,
        None => -1,
    }
}

/// echo_words：逐词回显——校验走表后自解析态重建记录表（非 memcpy）
/// 写入 RES_BUF（与输入同形：u32 n + 记录）。JS 胶水逐词比对原词；
/// 形态损坏或超出静态区容量 → -1。
#[no_mangle]
pub extern "C" fn echo_words(in_ptr: *const u8, in_len: u32) -> i32 {
    let input = if in_ptr.is_null() {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(in_ptr, in_len as usize) }
    };
    let (n, total) = match walk_records(input) {
        Some(x) => x,
        None => return -1,
    };
    if total > BUF_CAP {
        return -1;
    }
    unsafe {
        let mut w = 0usize;
        RES_BUF[w..w + 4].copy_from_slice(&(n as u32).to_le_bytes());
        w += 4;
        let mut pos = 4usize;
        for _ in 0..n {
            let l = u32::from_le_bytes(input[pos..pos + 4].try_into().unwrap()) as usize;
            RES_BUF[w..w + 4].copy_from_slice(&(l as u32).to_le_bytes());
            w += 4;
            pos += 4;
            RES_BUF[w..w + l].copy_from_slice(&input[pos..pos + l]);
            w += l;
            pos += l;
        }
        RES_LEN = w;
    }
    0
}
