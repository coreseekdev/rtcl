// http-bridge.js —— 把 fetch 接到 Tcl 的 http::* 词表（docs/ROADMAP.md §3）。
//
//   installHttpBridge(rtcl, { onError })
//
// 注册 `__rtcl_http <id> <method> <url> <body> <optsJson>` 桥命令：
// 立即返回（fetch 在 JS 里跑，Tcl 不阻塞），完成/失败经
// rtcl.dispatch_event 回灌 "::http::_done <id> <status> <error> <body>"。
// body/error 做 Tcl 大括号转义（任意文本安全过线，多行保留）。
// HTTP >= 400 也是"完成"（status 如实、error 空）——语义归消费方；
// 网络失败/超时/中止 → status 0 + error 消息。
//
// onError 收两种错误：回调脚本自身抛错（dispatch_event 返回的错误，
// 不静默）与桥内部异常。

// Tcl 双引号词转义（无损）：大括号词内反斜杠原样保留，花括号
// 转义天生有损；引号词走标准反斜杠替换，\ " $ [ 四个足矣。
const TCL_QUOTE = (s) => '"' + String(s)
  .replace(/\\/g, '\\\\').replace(/"/g, '\\"')
  .replace(/\$/g, '\\$').replace(/\[/g, '\\[') + '"';

export function installHttpBridge(rtcl, opts = {}) {
  const onError = opts.onError ?? ((m) => console.error('[http]', m));
  const inflight = new Map();   // id -> AbortController（预留取消/诊断）

  rtcl.register_command('__rtcl_http', (id, method, url, body, optsJson) => {
    let o = {};
    try { o = JSON.parse(optsJson || '{}'); } catch { o = {}; }
    const ctrl = new AbortController();
    const timeout = Number(o.timeout) || 0;
    const timer = timeout > 0 ? setTimeout(() => ctrl.abort(), timeout) : null;
    inflight.set(id, ctrl);

    const finish = (status, error, text) => {
      if (timer !== null) clearTimeout(timer);
      inflight.delete(id);
      try {
        rtcl.dispatch_event(
          `::http::_done ${id} ${status} ${TCL_QUOTE(error)} ${TCL_QUOTE(text)}`);
      } catch (e) {
        onError(String((e && e.message) || e));   // 回调脚本错误：宿主呈现（bgerror 回路）
      }
    };

    const init = { method, signal: ctrl.signal };
    if (o.ctype) init.headers = { 'content-type': o.ctype };
    if (method !== 'GET' && method !== 'HEAD' && body !== '') init.body = body;

    fetch(url, init).then(
      (resp) => resp.text().then((text) => finish(resp.status, '', text),
        (e) => finish(0, `body read failed: ${(e && e.message) || e}`, '')),
      (e) => {
        const aborted = e && e.name === 'AbortError';
        finish(0, aborted ? `timeout after ${timeout}ms` : String((e && e.message) || e), '');
      },
    );
    return '';   // 立即返回：完成走事件回灌
  });
}
