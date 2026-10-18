//! native HTTP 宿主服务（`net` feature）：[`HostHttp`] 的 reqwest（blocking）
//! 实现 + `__rtcl_http` / `__rtcl_http_cancel` 桥命令。
//! 由 rtcl-cli 与 rogo 等宿主消费（`rtcl_native::net::install`）。
//!
//! 事件面与 wasm 的 `www/http-bridge.js` 完全同形（Tcl 脚本无感）：
//! - 逐块：宿主 set_var `::http_chunk` + `::http::ev_id`，再 re-entrant
//!   `eval ::http::_delta`；
//! - 收尾：宿主 set_var `::http_status` / `::http_error` / `::http_body` 与
//!   `::http::ev_id`，再 `eval ::http::_done_vars`（全走 set_var，无 Tcl 转义问题）。
//!
//! 运行模式 A（阻塞内联）：桥命令在读取循环里驱动回调；`http::get` 返回时
//! 回调已跑完。模式 B（工作线程 + 事件队列 + `net::pump` 真异步）是后续项。
//!
//! 取消：`__rtcl_http_cancel <id>` 置协作标记，读取循环在下一块边界中止，
//! 以 `(status 0, error "cancelled")` 收尾。
//!
//! opts 线格式是 http.tcl 手搓的固定 JSON 形状
//! `{"timeout":N,"ctype":"…","stream":true?}`——R0 用最小字段扫描，
//! 正式化时再换成结构化边界（不在此处引入完整 JSON 解析器）。

use rtcl_core::host::{HostHttp, HttpRequest, HttpResponse};
use rtcl_core::{Interp, Value};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 装载 native HTTP 宿主：注入 `NativeHttp` + 注册桥命令 + 加载 `http.tcl` 词表。
pub fn install(interp: &mut Interp) {
    interp.set_http(Box::new(NativeHttp));
    interp.register_command("__rtcl_http", bridge);
    interp.register_command("__rtcl_http_cancel", bridge_cancel);
    if let Err(e) = interp.load_http() {
        eprintln!("rtcl: warning: http::* word layer failed to load: {e}");
    }
}

/// reqwest blocking 实现。
pub struct NativeHttp;

impl HostHttp for NativeHttp {
    fn request(&self, req: &HttpRequest) -> Result<Box<dyn HttpResponse>, String> {
        let client = reqwest::blocking::Client::builder()
            .redirect(if req.follow_redirects {
                reqwest::redirect::Policy::limited(10)
            } else {
                reqwest::redirect::Policy::none()
            })
            .build()
            .map_err(|e| format!("http client build failed: {e}"))?;
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| format!("bad method {:?}: {e}", req.method))?;
        let mut rb = client.request(method, &req.url);
        for (k, v) in &req.headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        if let Some(b) = &req.body {
            rb = rb.body(b.clone());
        }
        if req.timeout_ms > 0 {
            rb = rb.timeout(Duration::from_millis(req.timeout_ms));
        }
        let resp = rb.send().map_err(reqwest_err)?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        Ok(Box::new(NativeResponse {
            inner: resp,
            status,
            final_url,
            headers,
        }))
    }
}

struct NativeResponse {
    inner: reqwest::blocking::Response,
    status: u16,
    final_url: String,
    headers: Vec<(String, String)>,
}

impl HttpResponse for NativeResponse {
    fn status(&self) -> u16 {
        self.status
    }

    fn final_url(&self) -> String {
        self.final_url.clone()
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    fn headers(&self) -> Vec<(String, String)> {
        self.headers.clone()
    }

    fn read_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        use std::io::Read;
        let mut buf = vec![0u8; 8192];
        match self.inner.read(&mut buf) {
            Ok(0) => Ok(None),
            Ok(n) => {
                buf.truncate(n);
                Ok(Some(buf))
            }
            Err(e) => Err(err_chain(&e)),
        }
    }
}

/// 错误 + 完整 source 链（超时/连接拒绝等细节在 source 里，只打 Display
/// 会丢失 "operation timed out" 这类关键信息）。
fn err_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(c) = src {
        s.push_str(": ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

fn reqwest_err(e: reqwest::Error) -> String {
    err_chain(&e)
}

/// 协作取消标记表（Interp 非 Send，桥命令与读取循环同线程，无跨线程需求）。
fn cancels() -> &'static Mutex<HashSet<String>> {
    static CANCELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CANCELS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// `__rtcl_http id method url body optsJson`（wasm 桥同签名）。
pub fn bridge(interp: &mut Interp, args: &[Value]) -> rtcl_core::error::Result<Value> {
    let arg = |i: usize| -> String {
        args.get(i)
            .map(|v| v.as_str().to_string())
            .unwrap_or_default()
    };
    let id = arg(1);
    let method = arg(2);
    let url = arg(3);
    let body = arg(4);
    let opts = arg(5);

    let timeout = opt_u64(&opts, "timeout").unwrap_or(0);
    let stream = opt_bool(&opts, "stream").unwrap_or(false);
    let ctype = opt_str(&opts, "ctype").unwrap_or_else(|| "application/json".to_string());

    let req = HttpRequest {
        method,
        url,
        headers: if body.is_empty() {
            Vec::new()
        } else {
            vec![("content-type".to_string(), ctype)]
        },
        body: if body.is_empty() {
            None
        } else {
            Some(body.clone().into_bytes())
        },
        timeout_ms: timeout,
        follow_redirects: true,
    };

    cancels()
        .lock()
        .expect("cancel table lock")
        .remove(&id);

    // 取服务借用立即结束（返回拥有的响应体）；随后读取循环可 re-entrant eval。
    let mut resp = match interp.host_http().request(&req) {
        Ok(r) => r,
        Err(e) => {
            finish(interp, &id, 0, &e, "")?;
            return Ok(Value::empty());
        }
    };

    let status = resp.status();
    let mut acc: Vec<u8> = Vec::new();
    let mut error: Option<String> = None;

    loop {
        if cancels().lock().expect("cancel table lock").remove(&id) {
            error = Some("cancelled".to_string());
            break;
        }
        match resp.read_chunk() {
            Ok(Some(chunk)) => {
                acc.extend_from_slice(&chunk);
                if stream {
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    interp.set_var("::http_chunk", Value::from_str(&text))?;
                    interp.set_var("::http::ev_id", Value::from_str(&id))?;
                    interp.eval("::http::_delta")?;
                }
            }
            Ok(None) => break,
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }

    let body = String::from_utf8_lossy(&acc).to_string();
    match error {
        Some(e) => {
            finish(interp, &id, 0, &e, "")?;
        }
        None => {
            finish(interp, &id, status, "", &body)?;
        }
    }
    Ok(Value::empty())
}

/// `__rtcl_http_cancel id`：置协作取消标记（读取循环下一块边界检查）。
pub fn bridge_cancel(_interp: &mut Interp, args: &[Value]) -> rtcl_core::error::Result<Value> {
    if let Some(id) = args.get(1) {
        cancels()
            .lock()
            .expect("cancel table lock")
            .insert(id.as_str().to_string());
    }
    Ok(Value::empty())
}

/// 收尾回灌：set_var（免转义）+ `::http::_done_vars`。
fn finish(
    interp: &mut Interp,
    id: &str,
    status: u16,
    error: &str,
    body: &str,
) -> rtcl_core::error::Result<()> {
    interp.set_var("::http_status", Value::from_str(&status.to_string()))?;
    interp.set_var("::http_error", Value::from_str(error))?;
    interp.set_var("::http_body", Value::from_str(body))?;
    interp.set_var("::http::ev_id", Value::from_str(id))?;
    interp.eval("::http::_done_vars")?;
    Ok(())
}

// -- opts 最小字段扫描（固定线格式，见模块注释） ------------------------------

fn opt_u64(opts: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = opts.find(&pat)? + pat.len();
    let rest = opts[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn opt_bool(opts: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{key}\":");
    let start = opts.find(&pat)? + pat.len();
    let rest = opts[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn opt_str(opts: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = opts.find(&pat)? + pat.len();
    let mut out = String::new();
    let mut chars = opts[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ => out.push(c),
        }
    }
    None
}

// -- 测试：LLM stream API 形态的独立 mock（本地、无外网、无 key） ----------------
//
// 覆盖：OpenAI 风格 `data:` SSE、Anthropic 风格 `event:`+`data:`、UTF-8、
// 非 2xx 如实上报、超时、协作取消。传输用 chunked + 逐块 flush，模拟真实
// LLM 流式端点；每次 sleep 保证"块边界"可观察（增量性断言）。

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    struct Mock {
        url: String,
        _join: thread::JoinHandle<()>,
    }

    /// 一次性 mock：`status` 状态行；`head_delay_ms` 在响应头前等待；
    /// `chunks` 为 (距上一块的延迟 ms, 文本)，chunked 传输、逐块 flush。
    fn spawn_mock(status: u16, head_delay_ms: u64, chunks: Vec<(u64, String)>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}/sse", addr.port());
        let join = thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            // 读请求头（到空行），请求体不解析（测试不用）。
            let mut buf = [0u8; 2048];
            let mut seen: Vec<u8> = Vec::new();
            loop {
                match sock.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        seen.extend_from_slice(&buf[..n]);
                        if seen.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => return,
                }
            }
            thread::sleep(Duration::from_millis(head_delay_ms));
            let head = format!(
                "HTTP/1.1 {status} MOCK\r\n\
                 Content-Type: text/event-stream\r\n\
                 Transfer-Encoding: chunked\r\n\
                 Connection: close\r\n\r\n"
            );
            if sock.write_all(head.as_bytes()).is_err() {
                return;
            }
            for (delay, text) in &chunks {
                thread::sleep(Duration::from_millis(*delay));
                let frame = format!("{:x}\r\n{}\r\n", text.len(), text);
                if sock.write_all(frame.as_bytes()).is_err() {
                    return; // 客户端取消/超时后关闭，属预期
                }
                let _ = sock.flush();
            }
            let _ = sock.write_all(b"0\r\n\r\n");
            let _ = sock.flush();
        });
        Mock { url, _join: join }
    }

    fn test_interp() -> Interp {
        let mut interp = Interp::new();
        install(&mut interp);
        interp
    }

    fn set_url(interp: &mut Interp, mock: &Mock) {
        interp
            .set_var("::u", Value::from_str(&mock.url))
            .expect("set ::u");
    }

    fn get(interp: &mut Interp, script: &str) -> String {
        interp.eval(script).unwrap().as_str().to_string()
    }

    #[test]
    fn opts_field_extraction() {
        let s = r#"{"timeout":120,"ctype":"text\/plain","stream":true}"#;
        assert_eq!(opt_u64(s, "timeout"), Some(120));
        assert_eq!(opt_bool(s, "stream"), Some(true));
        assert_eq!(opt_str(s, "ctype").as_deref(), Some("text/plain"));
        assert_eq!(opt_bool(s, "missing"), None);
    }

    #[test]
    fn openai_style_sse_deltas_reassemble_and_stream_incrementally() {
        let chunks = vec![
            (
                30,
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n".to_string(),
            ),
            (
                30,
                "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n".to_string(),
            ),
            (30, "data: [DONE]\n\n".to_string()),
        ];
        let expected: String = chunks.iter().map(|(_, c)| c.as_str()).collect();
        let mock = spawn_mock(200, 0, chunks);

        let mut interp = test_interp();
        set_url(&mut interp, &mock);
        interp
            .eval(
                r#"set ::parts {}
http::get $::u -stream {lappend ::parts $::http_chunk} -command {set ::final [list $http_status $http_error $http_body]}"#,
            )
            .unwrap();

        let joined = interp
            .eval("join $::parts {}")
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(joined, expected, "streamed chunks must reassemble losslessly");

        let n_parts: usize = get(&mut interp, "llength $::parts").parse().unwrap();
        let first_len: usize = get(&mut interp, "string length [lindex $::parts 0]")
            .parse()
            .unwrap();
        assert!(n_parts >= 1, "at least one delta must fire");
        assert!(
            first_len < expected.len(),
            "first delta must arrive before the stream ends (incremental delivery), got first={first_len} total={}",
            expected.len()
        );

        assert_eq!(get(&mut interp, "lindex $::final 0"), "200");
        assert_eq!(get(&mut interp, "lindex $::final 1"), "");
        assert_eq!(get(&mut interp, "lindex $::final 2"), expected);
        // 读取变量复位
        assert_eq!(get(&mut interp, "info exists ::http_body"), "0");
    }

    #[test]
    fn anthropic_style_events_and_utf8_pass_through_byte_exact() {
        let chunks = vec![
            (
                10,
                "event: message_start\ndata: {\"type\":\"message_start\"}\n\n".to_string(),
            ),
            (
                10,
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好，世界\"}}\n\n".to_string(),
            ),
            (
                10,
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
            ),
        ];
        let expected: String = chunks.iter().map(|(_, c)| c.as_str()).collect();
        let mock = spawn_mock(200, 0, chunks);

        let mut interp = test_interp();
        set_url(&mut interp, &mock);
        interp
            .eval(
                r#"set ::acc {}
http::get $::u -stream {append ::acc $::http_chunk} -command {set ::final [list $http_status $http_error $http_body]}"#,
            )
            .unwrap();

        assert_eq!(get(&mut interp, "set ::acc"), expected);
        assert_eq!(get(&mut interp, "lindex $::final 2"), expected);
        assert_eq!(get(&mut interp, "lindex $::final 0"), "200");
    }

    #[test]
    fn non_2xx_status_is_reported_as_is() {
        let mock = spawn_mock(500, 0, vec![(0, "{\"error\":\"boom\"}".to_string())]);
        let mut interp = test_interp();
        set_url(&mut interp, &mock);
        interp
            .eval(r#"http::get $::u -command {set ::final [list $http_status $http_error $http_body]}"#)
            .unwrap();
        assert_eq!(get(&mut interp, "lindex $::final 0"), "500");
        assert_eq!(get(&mut interp, "lindex $::final 1"), "");
        assert!(get(&mut interp, "lindex $::final 2").contains("boom"));
    }

    #[test]
    fn timeout_is_reported_as_network_error() {
        let mock = spawn_mock(200, 400, vec![]);
        let mut interp = test_interp();
        set_url(&mut interp, &mock);
        interp
            .eval(r#"http::get $::u -timeout 80 -command {set ::final [list $http_status $http_error]}"#)
            .unwrap();
        assert_eq!(get(&mut interp, "lindex $::final 0"), "0");
        let err = get(&mut interp, "lindex $::final 1");
        assert!(!err.is_empty(), "timeout must surface an error");
        assert!(
            err.to_lowercase().contains("tim"),
            "error should read like a timeout: {err}"
        );
    }

    #[test]
    fn cancel_stops_stream_at_chunk_boundary() {
        let chunks: Vec<(u64, String)> = (0..10)
            .map(|i| (40, format!("data: {{\"n\":{i}}}\n\n")))
            .collect();
        let mock = spawn_mock(200, 0, chunks);
        let mut interp = test_interp();
        set_url(&mut interp, &mock);
        interp
            .eval(
                r#"set ::n 0
http::get $::u -stream {incr ::n; if {$::n == 1} {http::cancel $::http_id}} -command {set ::final [list $http_status $http_error]}"#,
            )
            .unwrap();
        assert_eq!(get(&mut interp, "set ::n"), "1", "only the first delta should be seen");
        assert_eq!(get(&mut interp, "lindex $::final 0"), "0");
        assert!(
            get(&mut interp, "lindex $::final 1").contains("cancel"),
            "cancel must surface as error"
        );
    }
}
