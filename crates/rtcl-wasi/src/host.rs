//! guest 侧 `host::*` 绑定：把 ABI import（`host.call` / `host.read_result`）
//! 包装成 Tcl 命令面（fs/http 原语），并装配 `__rtcl_http` 桥
//! （事件面与 native 桥同形：`_delta` / `_done_vars`）。
//!
//! 请求帧：[u32 nfields][每字段: u32 len + bytes...]，字段 0 = 方法名。
//! 响应：`host.call` 返回 n >= 0 成功（结果 n 字节）；n < 0 错误（错误文本
//! -n 字节）；两者都经 `host.read_result` 取回（宿主在 PENDING 暂存）。
//! 宿主在 import 内完成 p-root 围栏（fs）与网络（http）。

use rtcl_core::error::{Error, ErrorCode};
use rtcl_core::error::Result as TclResult;
use rtcl_core::{Interp, Value};

#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "host")]
    extern "C" {
        /// 执行一次宿主调用，返回结果长度（>=0）或错误文本长度（<0）。
        pub fn host_call(ptr: *const u8, len: usize) -> i32;
        /// 取回 PENDING 文本到 guest 缓冲，返回实际拷贝字节数。
        pub fn host_read_result(out: *mut u8, cap: usize) -> usize;
    }
}

fn frame(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for f in fields {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(f);
    }
    out
}

/// 调用宿主原语；成功返回结果字节，失败返回错误文本。
pub fn host_call(fields: &[&[u8]]) -> Result<Vec<u8>, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let req = frame(fields);
        let n = unsafe { imports::host_call(req.as_ptr(), req.len()) };
        let (is_err, len) = if n >= 0 {
            (false, n as u32)
        } else {
            (true, (n as i64).unsigned_abs() as u32)
        };
        let mut buf = vec![0u8; len as usize];
        let copied = if len == 0 {
            0
        } else {
            unsafe { imports::host_read_result(buf.as_mut_ptr(), buf.len()) }
        };
        buf.truncate(copied);
        if is_err {
            Err(String::from_utf8_lossy(&buf).to_string())
        } else {
            Ok(buf)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = fields;
        Err("host imports unavailable in native build".to_string())
    }
}

fn terr(msg: impl Into<String>) -> Error {
    Error::runtime(msg.into(), ErrorCode::Generic)
}

fn arg(args: &[Value], i: usize) -> String {
    args.get(i).map(|v| v.as_str().to_string()).unwrap_or_default()
}

fn text_of(bytes: Vec<u8>) -> Value {
    Value::from_str(&String::from_utf8_lossy(&bytes))
}

fn call_text(usage: &str, fields: &[&[u8]]) -> TclResult<Value> {
    host_call(fields)
        .map(text_of)
        .map_err(|e| terr(format!("{usage}: {e}")))
}

fn call_unit(usage: &str, fields: &[&[u8]]) -> TclResult<Value> {
    host_call(fields)
        .map(|_| Value::empty())
        .map_err(|e| terr(format!("{usage}: {e}")))
}

/// `host::fs::read <path>` —— 读 p-root 文件（UTF-8 lossy，与 native 同）。
pub fn cmd_fs_read(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_text("host::fs::read <path>", &[b"fs.read", path.as_bytes()])
}

/// `host::fs::write <path> <text>` —— 原子写（native `rogo::fs::write` 同语义）。
pub fn cmd_fs_write(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    let data = arg(args, 2);
    call_unit(
        "host::fs::write <path> <text>",
        &[b"fs.write", path.as_bytes(), data.as_bytes()],
    )
}

/// `host::fs::append <path> <text>` —— 追加写。
pub fn cmd_fs_append(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    let data = arg(args, 2);
    call_unit(
        "host::fs::append <path> <text>",
        &[b"fs.append", path.as_bytes(), data.as_bytes()],
    )
}

/// `host::fs::list <dir>` —— 目录项名（换行分隔，排序）。
pub fn cmd_fs_list(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_text("host::fs::list <dir>", &[b"fs.list", path.as_bytes()])
}

/// `host::fs::stat <path> ?field?` —— `type X size N` dict 或单字段。
pub fn cmd_fs_stat(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    let field = arg(args, 2);
    call_text(
        "host::fs::stat <path> ?field?",
        &[b"fs.stat", path.as_bytes(), field.as_bytes()],
    )
}

/// `host::fs::exists <path>` —— "1"/"0"。
pub fn cmd_fs_exists(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_text("host::fs::exists <path>", &[b"fs.exists", path.as_bytes()])
}

/// `host::fs::mkdir <dir>`。
pub fn cmd_fs_mkdir(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_unit("host::fs::mkdir <dir>", &[b"fs.mkdir", path.as_bytes()])
}

/// `host::fs::remove <path>`。
pub fn cmd_fs_remove(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_unit("host::fs::remove <path>", &[b"fs.remove", path.as_bytes()])
}

/// `host::http::start <method> <url> <body> <opts> <headers_encoded>`
/// （headers 编码：每行 `k\tv`；见 wasi 桥）。
pub fn cmd_http_start(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let method = arg(args, 1);
    let url = arg(args, 2);
    let body = arg(args, 3);
    let opts = arg(args, 4);
    let headers = arg(args, 5);
    call_unit(
        "host::http::start <method> <url> <body> <opts> <headers>",
        &[
            b"http.start",
            method.as_bytes(),
            url.as_bytes(),
            body.as_bytes(),
            opts.as_bytes(),
            headers.as_bytes(),
        ],
    )
}

/// `host::http::next` —— 拉取下一个事件：`chunk <text>` / `done <status>` /
/// `error <msg>`。
pub fn cmd_http_next(_i: &mut Interp, _args: &[Value]) -> TclResult<Value> {
    call_text("host::http::next", &[b"http.next"])
}

/// `host::http::body` —— 已累积响应体（字节精确、lossy 转文本）。
pub fn cmd_http_body(_i: &mut Interp, _args: &[Value]) -> TclResult<Value> {
    call_text("host::http::body", &[b"http.body"])
}

/// `host::http::cancel <id>` —— 协作取消（下一事件以 `error cancelled` 收尾）。
pub fn cmd_http_cancel(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    call_unit("host::http::cancel <id>", &[b"http.cancel", id.as_bytes()])
}

pub fn register(interp: &mut Interp) {
    interp.register_command("host::fs::read", cmd_fs_read);
    interp.register_command("host::fs::write", cmd_fs_write);
    interp.register_command("host::fs::append", cmd_fs_append);
    interp.register_command("host::fs::list", cmd_fs_list);
    interp.register_command("host::fs::stat", cmd_fs_stat);
    interp.register_command("host::fs::exists", cmd_fs_exists);
    interp.register_command("host::fs::mkdir", cmd_fs_mkdir);
    interp.register_command("host::fs::remove", cmd_fs_remove);
    interp.register_command("host::http::start", cmd_http_start);
    interp.register_command("host::http::next", cmd_http_next);
    interp.register_command("host::http::body", cmd_http_body);
    interp.register_command("host::http::cancel", cmd_http_cancel);
}
