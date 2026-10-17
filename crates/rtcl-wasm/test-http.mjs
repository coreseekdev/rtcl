// http 桥的 node 端到端冒烟：真 fetch + 本地 http 服务往返。
// 校验：GET/JSON Lines、POST 回显、HTTP >= 400 语义、网络错误、
// 超时中止、并发请求分派。用法：node test-http.mjs
// （浏览器侧另有 www/ui-http-acc.html）。

import fs from 'node:fs';
import http from 'node:http';
import initPkg, { RtclHandle } from './pkg/rtcl_wasm.js';
import { installHttpBridge } from './www/http-bridge.js';

await initPkg(new Uint8Array(fs.readFileSync(new URL('./pkg/rtcl_wasm_bg.wasm', import.meta.url))));

let fails = 0;
const ok = (cond, label, d) => {
  if (cond) console.log(`  ok  ${label}`);
  else { fails++; console.error(`FAIL  ${label}${d !== undefined ? ' :: ' + JSON.stringify(d) : ''}`); }
};

// ── 本地服务：JSON Lines / 回显 / 500 / 慢响应 ──
const server = http.createServer((req, res) => {
  if (req.url === '/lines') {
    res.writeHead(200, { 'content-type': 'application/x-ndjson' });
    res.end('{"level":"info","text":"a"}\n{"level":"warn","text":"b"}\n');
  } else if (req.url === '/boom') {
    res.writeHead(500, { 'content-type': 'text/plain' });
    res.end('server error');
  } else if (req.url === '/echo' && req.method === 'POST') {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ len: body.length, got: body }));
    });
  } else if (req.url === '/slow') {
    setTimeout(() => { res.writeHead(200); res.end('late'); }, 900);
  } else {
    res.writeHead(404, { 'content-type': 'text/plain' });
    res.end('nope');
  }
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const P = server.address().port;

// ── 装载 wasm + 桥 ──
const rtcl = new RtclHandle();
rtcl.set_output_handler(() => {});
const bridgeErrors = [];
installHttpBridge(rtcl, { onError: (m) => bridgeErrors.push(m) });
rtcl.register_command('_ui', () => '');

const wait = (fn, ms = 4000) => new Promise((res, rej) => {
  const t0 = Date.now();
  const t = setInterval(() => {
    let r = false;
    try { r = fn(); } catch { r = false; }
    if (r) { clearInterval(t); res(); }
    else if (Date.now() - t0 > ms) { clearInterval(t); rej(new Error('wait timeout')); }
  }, 10);
});
const exists = (v) => () => rtcl.exec(`info exists ${v}`) === '1';

// 1) GET JSON Lines：异步回灌、状态/错误/体三变量
try {
  rtcl.exec(`http::get http://127.0.0.1:${P}/lines -command {set ::r1 [list $http_status $http_error $http_body]}`);
  ok(rtcl.exec('info exists ::r1') === '0', 'Tcl script returned immediately (no block)');
  await wait(exists('::r1'));
  ok(rtcl.exec('set ::r1').startsWith('200 {} '), 'GET: status 200, no error', rtcl.exec('set ::r1'));
  // 变量在回调后复位：jsonl 检查用 r1 捕下的体（lindex 2）
  ok(rtcl.exec('llength [http::jsonl [lindex $::r1 2]]') === '2', 'jsonl: two lines');
  ok(rtcl.exec('info exists ::http_body') === '0', 'http vars reset after callback');
} catch (e) { fails++; console.error('FAIL  GET round trip ::', String(e)); }

// 2) POST 回显（body + content-type 直达）
try {
  rtcl.exec(`http::post http://127.0.0.1:${P}/echo {{"k":1}} -command {set ::r2 $http_body}`);
  await wait(exists('::r2'));
  const got = rtcl.exec('set ::r2');
  ok(got.includes('"len":7'), 'POST echo: server saw 7 bytes', got);
} catch (e) { fails++; console.error('FAIL  POST round trip ::', String(e)); }

// 3) HTTP >= 400 是"完成"：status 如实、error 空
try {
  rtcl.exec(`http::get http://127.0.0.1:${P}/boom -command {set ::r3 [list $http_status $http_error $http_body]}`);
  await wait(exists('::r3'));
  ok(rtcl.exec('set ::r3') === '500 {} {server error}', '500 delivered as completion', rtcl.exec('set ::r3'));
  rtcl.exec(`http::get http://127.0.0.1:${P}/nope -command {set ::r4 $http_status}`);
  await wait(exists('::r4'));
  ok(rtcl.exec('set ::r4') === '404', '404 status');
} catch (e) { fails++; console.error('FAIL  4xx/5xx semantics ::', String(e)); }

// 4) 网络错误：status 0 + error 非空
try {
  rtcl.exec('http::get http://127.0.0.1:1/nope -command {set ::r5 [list $http_status $http_error]}');
  await wait(exists('::r5'));
  ok(rtcl.exec('set ::r5').startsWith('0 {') , 'network error: status 0 + message', rtcl.exec('set ::r5'));
} catch (e) { fails++; console.error('FAIL  network error ::', String(e)); }

// 5) 超时：-timeout 150 对 /slow（900ms）→ 中止
try {
  rtcl.exec(`http::get http://127.0.0.1:${P}/slow -timeout 150 -command {set ::r6 [list $http_status $http_error]}`);
  await wait(exists('::r6'));
  ok(rtcl.exec('set ::r6').includes('timeout after 150ms'), 'timeout aborts and reports', rtcl.exec('set ::r6'));
} catch (e) { fails++; console.error('FAIL  timeout ::', String(e)); }

// 6) 并发：两个请求各自回灌到自己的回调
try {
  rtcl.exec(`http::get http://127.0.0.1:${P}/lines -command {set ::c1 $http_status}`);
  rtcl.exec(`http::get http://127.0.0.1:${P}/boom -command {set ::c2 $http_status}`);
  await wait(() => exists('::c1')() && exists('::c2')());
  ok(rtcl.exec('set ::c1') === '200' && rtcl.exec('set ::c2') === '500', 'concurrent requests route to their own callbacks');
} catch (e) { fails++; console.error('FAIL  concurrency ::', String(e)); }

ok(bridgeErrors.length === 0, 'no bridge/callback errors', bridgeErrors);

server.close();
const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
