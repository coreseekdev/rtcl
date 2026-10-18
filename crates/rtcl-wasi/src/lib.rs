//! rtcl-wasi —— rtcl 解释器的 WASI（wasm32-wasip1）C ABI v0。
//!
//! 目标（见 rogo `docs/superpowers/specs/2026-10-18-rtcl-wasi-abi.md`）：
//! 解释器作为**可替换的 wasm 组件**由宿主（rogo / wasmtime）加载，实现
//! 解释器独立于宿主二进制的动态升级；能力（fs/net/console/clock）后续以
//! 宿主 imports 白名单形式注入，本 crate 只给 ABI 骨架。
//!
//! ABI v0（UTF-8 字节跨界，句柄式调用）：
//! - `rtcl_abi_version() -> u32`
//! - `rtcl_alloc(len) -> *mut u8` / `rtcl_free(ptr, len)`
//! - `rtcl_init(abi) -> u32`（handle；0 = 失败）
//! - `rtcl_eval(h, ptr, len) -> i32`（0=ok / 1=error / 2=参数或状态错）
//! - `rtcl_result_len(h) -> usize` / `rtcl_result_copy(h, out, cap) -> usize`
//! - `rtcl_error_len(h) -> usize` / `rtcl_error_copy(h, out, cap) -> usize`
//!
//! 宿主加载流程：校验 `rtcl_abi_version` → `rtcl_alloc` 写入脚本 →
//! `rtcl_init` → `rtcl_eval` → 两次调用 `*_len`/`*_copy` 取回文本。

use rtcl_core::Interp;
use std::cell::RefCell;

mod host;

/// examine bridge 词表（guest 内 Tcl；http.tcl 由 `load_http()` 提供）。
const WASI_BRIDGE: &str = include_str!("../tcl/wasi-bridge.tcl");

/// ABI 版本（宿主加载时必须相等）。
pub const ABI_VERSION: u32 = 1;

const STATUS_OK: i32 = 0;
const STATUS_ERROR: i32 = 1;
const STATUS_INVALID: i32 = 2;

struct State {
    interp: Interp,
    result: String,
    error: String,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// 解释器 ABI 版本。
#[no_mangle]
pub extern "C" fn rtcl_abi_version() -> u32 {
    ABI_VERSION
}

/// 分配 `len` 字节缓冲（宿主把脚本/数据写进去后交回）。
#[no_mangle]
pub extern "C" fn rtcl_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// 释放 [`rtcl_alloc`] 的缓冲（`len` 必须与分配时一致）。
#[no_mangle]
pub extern "C" fn rtcl_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

/// 初始化解释器：`abi` 必须等于 [`ABI_VERSION`]；返回 handle（v0 恒为 1），0 = 失败。
/// 同时装配 guest 词层：http.tcl（native 契约）+ `__rtcl_http` 桥 + `host::*` 原语。
#[no_mangle]
pub extern "C" fn rtcl_init(abi: u32) -> u32 {
    if abi != ABI_VERSION {
        return 0;
    }
    STATE.with(|s| {
        let mut interp = Interp::new();
        host::register(&mut interp);
        // 词层失败仅警告（对齐 rtcl-native::net::install 的纪律：不静默但也
        // 不因可选词层失败阻断解释器本体）。
        if let Err(e) = interp.load_http() {
            eprintln!("rtcl-wasi: warning: http word layer failed to load: {e}");
        }
        if let Err(e) = interp.eval(WASI_BRIDGE) {
            eprintln!("rtcl-wasi: warning: http guest bridge failed to load: {e}");
        }
        *s.borrow_mut() = Some(State {
            interp,
            result: String::new(),
            error: String::new(),
        });
    });
    1
}

/// 求值一段脚本（UTF-8 字节）。见模块头状态码。
#[no_mangle]
pub extern "C" fn rtcl_eval(_handle: u32, ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() {
        return STATUS_INVALID;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let script = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return STATUS_INVALID,
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(st) = s.as_mut() else {
            return STATUS_INVALID;
        };
        match st.interp.eval(script) {
            Ok(v) => {
                st.result = v.as_str().to_string();
                st.error.clear();
                STATUS_OK
            }
            Err(e) => {
                st.error = e.to_string();
                st.result.clear();
                STATUS_ERROR
            }
        }
    })
}

/// 结果文本长度（字节）。
#[no_mangle]
pub extern "C" fn rtcl_result_len(_handle: u32) -> usize {
    STATE.with(|s| s.borrow().as_ref().map_or(0, |st| st.result.len()))
}

/// 拷贝结果文本到 `out`（最多 `cap` 字节）；返回实际写入长度。
#[no_mangle]
pub extern "C" fn rtcl_result_copy(_handle: u32, out: *mut u8, cap: usize) -> usize {
    copy_out(|st| &st.result, out, cap)
}

/// 错误文本长度（字节）。
#[no_mangle]
pub extern "C" fn rtcl_error_len(_handle: u32) -> usize {
    STATE.with(|s| s.borrow().as_ref().map_or(0, |st| st.error.len()))
}

/// 拷贝错误文本到 `out`（最多 `cap` 字节）；返回实际写入长度。
#[no_mangle]
pub extern "C" fn rtcl_error_copy(_handle: u32, out: *mut u8, cap: usize) -> usize {
    copy_out(|st| &st.error, out, cap)
}

fn copy_out(pick: impl Fn(&State) -> &String, out: *mut u8, cap: usize) -> usize {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(st) = s.as_ref() else {
            return 0;
        };
        let bytes = pick(st).as_bytes();
        let n = bytes.len().min(cap);
        if out.is_null() || n == 0 {
            return n;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, n);
        }
        n
    })
}
