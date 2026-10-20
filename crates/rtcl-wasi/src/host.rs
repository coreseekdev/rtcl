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

/// 解析宿主应答帧（与请求同格式）：`[u32 n][u32 len + bytes]...`。
fn parse_frame(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let rd = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap()) as usize;
    if bytes.len() < 4 {
        return Err("malformed frame".into());
    }
    let n = rd(&bytes[..4]);
    let mut off = 4usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if off + 4 > bytes.len() {
            return Err("truncated frame length".into());
        }
        let l = rd(&bytes[off..off + 4]);
        off += 4;
        if off + l > bytes.len() {
            return Err("truncated frame field".into());
        }
        out.push(bytes[off..off + l].to_vec());
        off += l;
    }
    Ok(out)
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

/// `host::env::get <name>` —— 宿主环境变量（空 = 未设置）。
pub fn cmd_env_get(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let name = arg(args, 1);
    call_text("host::env::get <name>", &[b"env.get", name.as_bytes()])
}

/// `host::clock::now-ms` —— Unix 毫秒。
pub fn cmd_clock_now_ms(_i: &mut Interp, _args: &[Value]) -> TclResult<Value> {
    call_text("host::clock::now-ms", &[b"clock.now-ms"])
}

/// `host::fs::sha256 <path>` —— 小写十六进制。
pub fn cmd_fs_sha256(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_text("host::fs::sha256 <path>", &[b"fs.sha256", path.as_bytes()])
}

/// `host::yaml::load <path>` —— YAML 文件 → Tcl dict/list 字符串。
pub fn cmd_yaml_load(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let path = arg(args, 1);
    call_text("host::yaml::load <path>", &[b"yaml.load", path.as_bytes()])
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

/// `host::exec::pipeline <stage-list> <stdin>` —— 宿主按 stage 连接有界管道；
/// 返回会话 id。stage-list = 每 stage 一个 Tcl 列表（首词 = 命令名）。
pub fn cmd_exec_pipeline(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    pipeline_call(b"exec.pipeline", args)
}

/// 后台形态：宿主把最后 stdout / 全部 stderr 直接 drain 到自己的终端；
/// guest 只拿 job id，用 job wait/kill 管理。
pub fn cmd_exec_pipeline_bg(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    pipeline_call(b"exec.pipeline.bg", args)
}

fn pipeline_call(method: &[u8], args: &[Value]) -> TclResult<Value> {
    let stages = args
        .get(1)
        .and_then(|v| v.as_list())
        .ok_or_else(|| terr("host::exec::pipeline <stage-list> <stdin>"))?;
    if stages.is_empty() {
        return Err(terr("host::exec::pipeline: empty pipeline"));
    }
    let stdin = arg(args, 2);
    let stdin_file = arg(args, 3);
    let stderr_mode = arg(args, 4);
    let stderr_path = arg(args, 5);
    let merges = args
        .get(6)
        .and_then(|v| v.as_list())
        .unwrap_or_default();
    let mut owned: Vec<Vec<u8>> = vec![
        method.to_vec(),
        stdin.into_bytes(),
        stdin_file.into_bytes(),
        stderr_mode.into_bytes(),
        stderr_path.into_bytes(),
        stages.len().to_string().into_bytes(),
    ];
    for (idx, st) in stages.iter().enumerate() {
        let words = st
            .as_list()
            .ok_or_else(|| terr("host::exec::pipeline: stage must be a list"))?;
        if words.is_empty() {
            return Err(terr("host::exec::pipeline: empty stage"));
        }
        let merge = merges
            .get(idx)
            .map(|v| v.as_str() == "1")
            .unwrap_or(false);
        owned.push(if merge { b"1".to_vec() } else { b"0".to_vec() });
        owned.push(words.len().to_string().into_bytes());
        for w in words {
            owned.push(w.as_str().as_bytes().to_vec());
        }
    }
    let refs: Vec<&[u8]> = owned.iter().map(|b| b.as_slice()).collect();
    host_call(&refs)
        .map(text_of)
        .map_err(|e| terr(format!("host::exec::pipeline: {e}")))
}

/// `host::exec::pipeline.read <id> ?max?` —— 读最后 stage stdout（空 = EOF）。
pub fn cmd_exec_pipeline_read(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    let max = arg(args, 2);
    if id.is_empty() {
        return Err(terr("host::exec::pipeline.read <id> ?max?"));
    }
    let reply = host_call(&[
        b"exec.pipeline.read",
        id.as_bytes(),
        max.as_bytes(),
    ])
    .map_err(|e| terr(format!("host::exec::pipeline.read: {e}")))?;
    Ok(text_of(reply))
}

/// `host::exec::pipeline.wait <id>` —— 等所有 stage；设置 code/stderr。
pub fn cmd_exec_pipeline_wait(interp: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::pipeline.wait <id>"));
    }
    let reply = host_call(&[b"exec.pipeline.wait", id.as_bytes()])
        .map_err(|e| terr(format!("host::exec::pipeline.wait: {e}")))?;
    let parts = parse_frame(&reply)
        .map_err(|e| terr(format!("host::exec::pipeline.wait: {e}")))?;
    let code: i32 = parts
        .first()
        .and_then(|b| String::from_utf8_lossy(b).parse().ok())
        .unwrap_or(1);
    let err = parts.get(1).cloned().unwrap_or_default();
    let _ = interp.eval("namespace eval ::host::exec {}");
    interp.set_var("::host::exec::code", Value::from_str(&code.to_string()))?;
    interp.set_var(
        "::host::exec::stderr",
        Value::from_str(&String::from_utf8_lossy(&err)),
    )?;
    Ok(Value::from_str(&code.to_string()))
}

/// `host::exec::pipeline.kill <id>` —— 删除会话并关闭宿主侧句柄。
pub fn cmd_exec_pipeline_kill(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::pipeline.kill <id>"));
    }
    exec_call(
        b"exec.pipeline.kill",
        &[id.as_bytes()],
        "host::exec::pipeline.kill",
    )
}

/// `host::exec::pipeline.stop <id>` —— 请求协作式挂起。
pub fn cmd_exec_pipeline_stop(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::pipeline.stop <id>"));
    }
    exec_call(
        b"exec.pipeline.stop",
        &[id.as_bytes()],
        "host::exec::pipeline.stop",
    )
}

/// `host::exec::pipeline.resume <id>`。
pub fn cmd_exec_pipeline_resume(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::pipeline.resume <id>"));
    }
    exec_call(
        b"exec.pipeline.resume",
        &[id.as_bytes()],
        "host::exec::pipeline.resume",
    )
}

/// `host::exec::pipeline.state <id>` → running/stopping/stopped/killed。
pub fn cmd_exec_pipeline_state(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::pipeline.state <id>"));
    }
    exec_call(
        b"exec.pipeline.state",
        &[id.as_bytes()],
        "host::exec::pipeline.state",
    )
}

/// `host::exec::run <prog> ?arg ...?` —— 经宿主 PATH 执行 wasm 命令。
/// v0 聚合形态：stdin 为空；stdout 作为结果返回；`::host::exec::code` /
/// `::host::exec::stderr` 存退出码与 stderr（文本，lossy）。
pub fn cmd_exec_run(interp: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let prog = arg(args, 1);
    if prog.is_empty() {
        return Err(terr("host::exec::run <prog> ?arg ...?"));
    }
    let argbuf: Vec<String> = (2..args.len()).map(|i| arg(args, i)).collect();
    let mut fields: Vec<&[u8]> = vec![b"exec.run", prog.as_bytes(), b""];
    for a in &argbuf {
        fields.push(a.as_bytes());
    }
    let reply = host_call(&fields).map_err(|e| terr(format!("host::exec::run: {e}")))?;
    let parts = parse_frame(&reply).map_err(|e| terr(format!("host::exec::run: {e}")))?;
    let out = parts.first().cloned().unwrap_or_default();
    let err = parts.get(1).cloned().unwrap_or_default();
    let code: i32 = parts
        .get(2)
        .and_then(|b| String::from_utf8_lossy(b).parse().ok())
        .unwrap_or(0);
    let _ = interp.eval("namespace eval ::host::exec {}");
    interp.set_var("::host::exec::code", Value::from_str(&code.to_string()))?;
    interp.set_var(
        "::host::exec::stderr",
        Value::from_str(&String::from_utf8_lossy(&err)),
    )?;
    Ok(Value::from_str(&String::from_utf8_lossy(&out)))
}

fn exec_call(method: &[u8], fields: &[&[u8]], usage: &str) -> TclResult<Value> {
    let mut v: Vec<&[u8]> = vec![method];
    v.extend_from_slice(fields);
    host_call(&v)
        .map(|b| text_of(b))
        .map_err(|e| terr(format!("{usage}: {e}")))
}

/// `host::exec::open <prog> ?arg ...?` → 会话 id（分块流式）。
pub fn cmd_exec_open(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let prog = arg(args, 1);
    if prog.is_empty() {
        return Err(terr("host::exec::open <prog> ?arg ...?"));
    }
    let argbuf: Vec<String> = (2..args.len()).map(|i| arg(args, i)).collect();
    let mut fields: Vec<&[u8]> = vec![prog.as_bytes()];
    for a in &argbuf {
        fields.push(a.as_bytes());
    }
    exec_call(b"exec.open", &fields, "host::exec::open")
}

/// `host::exec::write <id> <data>` → 写入字节数（0 = 写端已关/EPIPE）。
pub fn cmd_exec_write(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    let data = arg(args, 2);
    if id.is_empty() {
        return Err(terr("host::exec::write <id> <data>"));
    }
    exec_call(b"exec.write", &[id.as_bytes(), data.as_bytes()], "host::exec::write")
}

/// `host::exec::read <id> ?max?` → stdout 分块（空 = EOF）。
pub fn cmd_exec_read(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    let max = arg(args, 2);
    if id.is_empty() {
        return Err(terr("host::exec::read <id> ?max?"));
    }
    exec_call(b"exec.read", &[id.as_bytes(), max.as_bytes()], "host::exec::read")
}

/// `host::exec::close-stdin <id>` → 子进程见 EOF。
pub fn cmd_exec_close_stdin(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::close-stdin <id>"));
    }
    exec_call(b"exec.close-stdin", &[id.as_bytes()], "host::exec::close-stdin")
}

/// `host::exec::wait <id>` → 退出码；`::host::exec::stderr` 存放 stderr。
pub fn cmd_exec_wait(interp: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::wait <id>"));
    }
    let reply = host_call(&[b"exec.wait", id.as_bytes()])
        .map_err(|e| terr(format!("host::exec::wait: {e}")))?;
    let parts = parse_frame(&reply).map_err(|e| terr(format!("host::exec::wait: {e}")))?;
    let code: i32 = parts
        .first()
        .and_then(|b| String::from_utf8_lossy(b).parse().ok())
        .unwrap_or(1);
    let err = parts.get(1).cloned().unwrap_or_default();
    let _ = interp.eval("namespace eval ::host::exec {}");
    interp.set_var("::host::exec::code", Value::from_str(&code.to_string()))?;
    interp.set_var(
        "::host::exec::stderr",
        Value::from_str(&String::from_utf8_lossy(&err)),
    )?;
    Ok(Value::from_str(&code.to_string()))
}

/// `host::exec::kill <id>` → 关闭宿主侧句柄（v0 无法强制终止运行中的 wasm）。
pub fn cmd_exec_kill(_i: &mut Interp, args: &[Value]) -> TclResult<Value> {
    let id = arg(args, 1);
    if id.is_empty() {
        return Err(terr("host::exec::kill <id>"));
    }
    exec_call(b"exec.kill", &[id.as_bytes()], "host::exec::kill")
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
    interp.register_command("host::fs::sha256", cmd_fs_sha256);
    interp.register_command("host::env::get", cmd_env_get);
    interp.register_command("host::clock::now-ms", cmd_clock_now_ms);
    interp.register_command("host::yaml::load", cmd_yaml_load);
    interp.register_command("host::http::start", cmd_http_start);
    interp.register_command("host::http::next", cmd_http_next);
    interp.register_command("host::http::body", cmd_http_body);
    interp.register_command("host::http::cancel", cmd_http_cancel);
    interp.register_command("host::exec::pipeline", cmd_exec_pipeline);
    interp.register_command("host::exec::pipeline.bg", cmd_exec_pipeline_bg);
    interp.register_command("host::exec::pipeline.read", cmd_exec_pipeline_read);
    interp.register_command("host::exec::pipeline.wait", cmd_exec_pipeline_wait);
    interp.register_command("host::exec::pipeline.kill", cmd_exec_pipeline_kill);
    interp.register_command("host::exec::pipeline.stop", cmd_exec_pipeline_stop);
    interp.register_command("host::exec::pipeline.resume", cmd_exec_pipeline_resume);
    interp.register_command("host::exec::pipeline.state", cmd_exec_pipeline_state);
    interp.register_command("host::exec::run", cmd_exec_run);
    interp.register_command("host::exec::open", cmd_exec_open);
    interp.register_command("host::exec::write", cmd_exec_write);
    interp.register_command("host::exec::read", cmd_exec_read);
    interp.register_command("host::exec::close-stdin", cmd_exec_close_stdin);
    interp.register_command("host::exec::wait", cmd_exec_wait);
    interp.register_command("host::exec::kill", cmd_exec_kill);
}
