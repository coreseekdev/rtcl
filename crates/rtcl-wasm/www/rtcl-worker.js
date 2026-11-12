// 浏览器 worker 原生 postMessage/onmessage；node worker_threads 测试链
// 路由 parentPort 提供（self/全局兼容垫片）。
const BOOT_QUEUE = [];
let booted = false;
let __nodePort = null;
if (typeof postMessage !== 'function') {
  // node worker_threads（本仓测试链路）——ESM 顶层 await
  const { parentPort } = await import('node:worker_threads');
  globalThis.postMessage = (m) => parentPort.postMessage(m);
  parentPort.on('message', (m) => globalThis.__recv({ data: m }));
}
// rtcl-worker.js — rtcl 的 worker 部署形态（网页 dispatcher 的真实运行
// 结构）：解释器 + JS 桥全部在本 worker，主线程经消息协议投递脚本。
//
import init, { RtclHandle } from '../pkg/rtcl_wasm.js';
import { makeRegexpMod } from './regexpmod.js';

// regexp 外置的语义 wrapper proc（::regexp/::regsub 解包结构化编码并
// upvar 写变量；native 同面）。
const REGEXP_SETUP_TCL = `
proc ::regexp {args} {
  set r [::regexp::regexp [join $args "\u0001"]]
  if {[lindex $r 0] eq "__RXCAP"} {
    set k [lindex $r 1]
    set vals [lrange $r 2 end]
    set i 0
    foreach name [lrange $args [expr {[llength $args] - $k}] end] {
      upvar 1 $name v
      set v [lindex $vals $i]
      incr i
    }
    return 1
  }
  if {[lindex $r 0] eq "__RXIDX"} {
    set k [lindex $r 1]
    set pairs [lrange $r 2 end]
    set i 0
    foreach name [lrange $args [expr {[llength $args] - $k}] end] {
      upvar 1 $name v
      set v [lindex $pairs $i]
      incr i
    }
    return 1
  }
  return $r
}
proc ::regsub {args} {
  set r [::regsub::regsub [join $args "\u0001"]]
  if {[lindex $r 0] eq "__RS"} {
    upvar 1 [lindex $args end] v
    set v [lindex $r 1]
    return [lindex $r 2]
  }
  return $r
}
`;

function makeOpfsFs(root) {
  const files = new Map();   // path -> {bytes: Uint8Array, mtime, atime}
  const dirs = new Set(['/']);
  let cwd = '/';
  const queue = [];          // 持久化队列：{op:'put'|'rm'|'mkdir', path, bytes?}
  let flushing = false;

  const norm = (p) => {
    if (!p.startsWith('/')) p = (cwd === '/' ? '' : cwd) + '/' + p;
    const parts = [];
    for (const seg of p.split('/')) {
      if (seg === '' || seg === '.') continue;
      if (seg === '..') parts.pop(); else parts.push(seg);
    }
    return '/' + parts.join('/');
  };
  const parent = (p) => { const i = p.lastIndexOf('/'); return i <= 0 ? '/' : p.slice(0, i); };
  const base = (p) => p.slice(p.lastIndexOf('/') + 1);
  const touchDir = (p) => { let d = norm(p); while (d && d !== '/') { dirs.add(d); d = parent(d); } };
  const now = () => Math.floor(Date.now() / 1000);
  const enc = (s) => { const u = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i) & 0xff; return u; };
  const dec = (u) => { let s = ''; for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]); return s; };

  function enqueue(op) { queue.push(op); scheduleFlush(); }
  function scheduleFlush() {
    if (flushing) return;
    flushing = true;
    queueMicrotask(() => { drain().finally(() => { flushing = false; if (queue.length) scheduleFlush(); }); });
  }
  async function drain() {
    while (queue.length) {
      const op = queue.shift();
      try {
        if (op.op === 'put') {
          const dh = await dirHandle(parent(op.path), true);
          const fh = await dh.getFileHandle(base(op.path), { create: true });
          const w = await fh.createWritable();
          await w.write(op.bytes);
          await w.close();
        } else if (op.op === 'rm') {
          const dh = await dirHandle(parent(op.path), false);
          await dh.removeEntry(base(op.path), { recursive: true });
        } else if (op.op === 'mkdir') {
          await dirHandle(op.path, true);
        }
      } catch (e) {
        // 回写失败不阻断影子语义（影子是权威）；留痕供诊断。
        console.warn('[opfs] flush failed for', op.op, op.path, e && e.message);
      }
    }
  }
  async function dirHandle(path, create) {
    let cur = root;
    for (const seg of norm(path).split('/').filter(Boolean)) {
      cur = await cur.getDirectoryHandle(seg, { create });
    }
    return cur;
  }
  async function loadTree() {
    async function walk(dh, prefix, depth) {
      if (depth > 8) return;
      for await (const [name, h] of dh.entries()) {
        const p = prefix + '/' + name;
        if (h.kind === 'directory') {
          dirs.add(p);
          await walk(h, p, depth + 1);
        } else {
          const f = await h.getFile();
          const bytes = new Uint8Array(await f.arrayBuffer());
          files.set(p, { bytes, mtime: Math.floor(f.lastModified / 1000), atime: Math.floor(f.lastModified / 1000) });
        }
      }
    }
    await walk(root, '', 0);
  }

  const statFn = (p) => {
    p = norm(p);
    if (files.has(p)) {
      const f = files.get(p);
      return { size: f.bytes.length, mtime: f.mtime, atime: f.atime, dir: false, file: true };
    }
    if (dirs.has(p)) return { size: 0, mtime: 0, atime: 0, dir: true, file: false };
    return null;
  };

  const contract = {
    open(path, mode) {
      path = norm(path);
      const m = mode.toLowerCase();
      const exists = files.has(path);
      if (m.startsWith('r') && !exists) {
        throw new Error("couldn't open \"" + path + "\": no such file or directory");
      }
      if (!exists) {
        files.set(path, { bytes: new Uint8Array(0), mtime: now(), atime: now() });
        enqueue({ op: 'put', path, bytes: new Uint8Array(0) });
        touchDir(parent(path));
        enqueue({ op: 'mkdir', path: parent(path) });
      }
      const entry = files.get(path);
      let pos = 0;
      if (m.startsWith('a')) pos = entry.bytes.length;
      if (m.startsWith('w')) {
        entry.bytes = new Uint8Array(0);
        entry.mtime = now();
      }
      const persist = () => {
        entry.mtime = now();
        enqueue({ op: 'put', path, bytes: entry.bytes.slice() });
      };
      return {
        read(n) {
          if (pos >= entry.bytes.length) return '';
          const take = Math.min(n || 1, entry.bytes.length - pos);
          let s = '';
          for (let i = 0; i < take; i++) s += String.fromCharCode(entry.bytes[pos + i]);
          pos += take;
          return s;
        },
        readline() {
          if (pos >= entry.bytes.length) return null;
          let s = '';
          while (pos < entry.bytes.length) {
            const c = String.fromCharCode(entry.bytes[pos++]);
            if (c === '\n') return s.replace(/\r$/, '');
            s += c;
          }
          return s;
        },
        readall() { return dec(entry.bytes); },
        write(s) {
          const add = enc(s);
          if (pos + add.length > entry.bytes.length) {
            const bigger = new Uint8Array(pos + add.length);
            bigger.set(entry.bytes, 0);
            entry.bytes = bigger;
          }
          entry.bytes.set(add, pos);
          pos += add.length;
          persist();
        },
        flush() { persist(); },
        seek(p, whence) {
          const len = entry.bytes.length;
          if (whence === 'set') pos = p;
          else if (whence === 'cur') pos += p;
          else pos = len + p;
          pos = Math.max(0, Math.min(pos, len));
          return pos;
        },
        tell() { return pos; },
        eof() { return pos >= entry.bytes.length; },
        close() { persist(); },
      };
    },
    stat(path) { return statFn(path); },
    access(path) {
      path = norm(path);
      const ok = files.has(path) || dirs.has(path);
      return { read: ok, write: true, exec: false };
    },
    mkdir(path, all) {
      path = norm(path);
      if (!all && !dirs.has(parent(path))) {
        throw new Error('couldn\'t create directory "' + path + '": no such file or directory');
      }
      touchDir(path);
      enqueue({ op: 'mkdir', path });
    },
    delete(path) {
      path = norm(path);
      if (dirs.has(path)) {
        for (const k of [...files.keys()]) if (k.startsWith(path + '/')) { files.delete(k); enqueue({ op: 'rm', path: k }); }
        for (const k of [...dirs]) if (k.startsWith(path + '/')) dirs.delete(k);
        dirs.delete(path);
        enqueue({ op: 'rm', path });
      } else if (files.has(path)) {
        files.delete(path);
        enqueue({ op: 'rm', path });
      }
    },
    rename(src, dst) {
      src = norm(src); dst = norm(dst);
      if (!files.has(src)) throw new Error('no such file');
      files.set(dst, files.get(src));
      files.delete(src);
      enqueue({ op: 'put', path: dst, bytes: files.get(dst).bytes.slice() });
      enqueue({ op: 'rm', path: src });
    },
    copy(src, dst) {
      src = norm(src); dst = norm(dst);
      if (!files.has(src)) throw new Error('no such file');
      files.set(dst, { bytes: files.get(src).bytes.slice(), mtime: now(), atime: now() });
      enqueue({ op: 'put', path: dst, bytes: files.get(dst).bytes.slice() });
    },
    readdir(path) {
      path = norm(path);
      const out = [];
      for (const k of dirs) if (parent(k) === path && k !== '/') out.push(base(k));
      for (const k of files.keys()) if (parent(k) === path) out.push(base(k));
      return out;
    },
    pwd() { return cwd; },
    cd(path) { cwd = norm(path); },
    readfile(path) {
      path = norm(path);
      if (!files.has(path)) throw new Error("couldn't read file \"" + path + "\": no such file or directory");
      return dec(files.get(path).bytes);
    },
    tempfile(prefix) {
      const p = '/tmp/' + prefix + (Date.now() % 100000);
      touchDir('/tmp');
      files.set(p, { bytes: new Uint8Array(0), mtime: now(), atime: now() });
      enqueue({ op: 'put', path: p, bytes: new Uint8Array(0) });
      return p;
    },
    settimes(path, atime, mtime) {
      path = norm(path);
      if (!files.has(path)) throw new Error("couldn't set time for \"" + path + "\": no such file");
      const f = files.get(path);
      if (atime !== null && atime !== undefined) f.atime = Number(atime);
      if (mtime !== null && mtime !== undefined) f.mtime = Number(mtime);
    },
    readlink() { throw new Error('invalid argument'); },
    statfull(path, follow) {
      const s = statFn(path);
      if (s === null) return null;
      let ino = 0;
      for (let i = 0; i < path.length; i++) ino = (ino * 31 + path.charCodeAt(i)) >>> 0;
      const mode = s.dir ? 0o40755 : 0o100644;
      return Object.assign({}, s, {
        ctime: s.mtime, dev: 0, ino, nlink: 1, uid: 1000, gid: 1000,
        mode, blksize: 4096, blocks: Math.ceil(s.size / 512), link: false,
      });
    },
  };

  return { contract, loadTree };
}

// ── worker 主体 ─────────────────────────────────────────────────────

// ── SAB 同步桥（bridge 模式的 fs 契约）：同步阻塞等待 fs-worker 的
// OPFS 回执——每个 Tcl 文件操作在返回前完成 OPFS 落盘（严格 durable）。
// ── SAB 严格 durable 形态的契约构造（fs-worker 为 OPFS 执行端）────
function makeBridgeFs(sab, fsPort) {
  const i32 = new Int32Array(sab);
  const u8 = new Uint8Array(sab);
  let seq = 0;
  function callFs(op, params) {
    seq += 1;
    // BigInt（i64 实参）不可 JSON.stringify——统一 Number 归一
    const safe = JSON.stringify({ op, params }, (_k, v) =>
      typeof v === 'bigint' ? Number(v) : v);
    const req = new TextEncoder().encode(safe);
    u8.set(req.subarray(0, 256 * 1024 - 64), 64);
    Atomics.store(i32, 2, req.length);          // PAYLOAD_LEN
    Atomics.store(i32, 0, seq);                 // REQ_SEQ
    Atomics.store(i32, 1, 0);                   // REPLY_SEQ 复位
    fsPort.postMessage({ seq });
    const r = Atomics.wait(i32, 1, 0);          // 阻塞至 fs worker 回执
    if (r !== 'ok') throw new Error('fs bridge wait failed: ' + r);
    const len = Atomics.load(i32, 2);
    // SAB 视图不可直接喂 TextDecoder——拷贝
    const copy = new Uint8Array(len);
    copy.set(u8.subarray(64, 64 + len));
    const reply = JSON.parse(new TextDecoder().decode(copy));
    if (!reply.ok) throw new Error(reply.error);
    return reply.result;
  }
  // 契约面（方法名 = fs_bridge.rs 的 JsFs 契约；open 返回句柄对象，
  // JsChannel 经 FILE_OBJECTS 间接调用其 read/write/... 方法）。
  return {
    open(path, mode) {
      const fid = String(callFs('open', { path, mode }));
      return {
        read(n) { return callFs('read', { fid, n: Number(n) }); },
        readline() { return callFs('readline', { fid }); },
        readall() { return callFs('readall', { fid }); },
        write(s) { return callFs('write', { fid, data: s }); },
        flush() { return callFs('flush', { fid }); },
        seek(pos, whence) { return callFs('seek', { fid, pos: Number(pos), whence }); },
        tell() { return callFs('tell', { fid }); },
        eof() { return callFs('eof', { fid }); },
        close() { return callFs('close', { fid }); },
        _fid: fid,
      };
    },
    stat(path) { return callFs('stat', { path }); },
    statfull(path, follow) { return callFs('statfull', { path, follow }); },
    access(path) { return callFs('access', { path }); },
    mkdir(path, all) { return callFs('mkdir', { path, all }); },
    delete(path) { return callFs('delete', { path }); },
    rename(src, dst) { return callFs('rename', { src, dst }); },
    copy(src, dst) { return callFs('copy', { src, dst }); },
    readdir(path) { return callFs('readdir', { path }); },
    pwd() { return callFs('pwd', {}); },
    cd(path) { return callFs('cd', { path }); },
    readfile(path) { return callFs('readfile', { path }); },
    tempfile(prefix) { return callFs('tempfile', { prefix }); },
    settimes(path, atime, mtime) { return callFs('settimes', { path, atime, mtime }); },
    readlink(path) { return callFs('readlink', { path }); },
  };
}

// ── 模块级消息路由 ────────────────────────────────────────────────
// module worker 规范：eval 完成前父线程投递的消息由浏览器排队，eval
// 结束后才派发——所以 onmessage 必须在 eval 期间就位，消息入
// BOOT_QUEUE，IIFE 在 fs 分支判断**前**重放（否则 bridge SAB/port 到
// 达时 fs 模式已定为直连——stFs 静默降级的根因）。
let handle = null;
let bridgeSab = null;
let bridgePort = null;
let outputs = [];

function handleControl(msg) {
  if (msg.sab && msg.fsPort) {
    bridgeSab = msg.sab;
    bridgePort = msg.fsPort;
    return true;
  }
  return false;
}

function dispatchExec({ id, script }) {
  outputs = [];
  try {
    const result = handle.exec(script);
    globalThis.postMessage({ id, ok: true, result, output: outputs.join('') });
  } catch (e) {
    globalThis.postMessage({ id, ok: false, error: String(e), output: outputs.join('') });
  }
}

function dispatchAndControl(ev) {
  const msg = ev.data;
  if (msg.id !== undefined && msg.script !== undefined) dispatchExec(msg);
  else handleControl(msg);
}

globalThis.onmessage = (ev) => {
  if (booted) { dispatchAndControl(ev); return; }
  BOOT_QUEUE.push(ev.data);
};
if (typeof globalThis.__recv !== 'function') {
  globalThis.__recv = (ev) => {
    if (booted) { dispatchAndControl(ev); return; }
    BOOT_QUEUE.push(ev.data);
  };
}

// ── 启动序列 ────────────────────────────────────────────────────────

(async () => {
  try {
    await init();
    handle = new RtclHandle();
    handle.set_output_handler((s) => outputs.push(s));

    // ① 重放排队的消息：控制消息（bridge SAB/port）先于 exec
    const execQ = [];
    while (BOOT_QUEUE.length) {
      const msg = BOOT_QUEUE.shift();
      if (msg.id !== undefined && msg.script !== undefined) execQ.push(msg);
      else handleControl(msg);
    }

    // ② fs 模式判定（bridge → 直连 OPFS → 内存回落）
    let fsNote = 'memory';
    if (bridgeSab !== null) {
      // SAB 严格 durable 形态：Atomics.wait 阻塞等待 fs-worker 的
      // OPFS 回执（回执在写入完成之后发出）。契约对象经
      // RtclHandle::set_fs → JsFs 包装（方法名 = fs_bridge.rs 契约）。
      const i32 = new Int32Array(bridgeSab);
      const u8 = new Uint8Array(bridgeSab);
      let seq = 0;
      function callFs(op, params) {
        seq += 1;
        const safe = JSON.stringify({ op, params }, (_k, v) =>
          typeof v === 'bigint' ? Number(v) : v);
        const req = new TextEncoder().encode(safe);
        u8.set(req.subarray(0, 256 * 1024 - 64), 64);
        Atomics.store(i32, 2, req.length);
        Atomics.store(i32, 0, seq);
        Atomics.store(i32, 1, 0);
        bridgePort.postMessage({ seq });
        const r = Atomics.wait(i32, 1, 0);
        if (r !== 'ok') throw new Error('fs bridge wait failed: ' + r);
        const len = Atomics.load(i32, 2);
        // SAB 视图不可直接喂 TextDecoder——拷贝
        const copy = new Uint8Array(len);
        copy.set(u8.subarray(64, 64 + len));
        const reply = JSON.parse(new TextDecoder().decode(copy));
        if (!reply.ok) throw new Error(reply.error);
        return reply.result;
      }
      const contract = {
        open(path, mode) {
          const h = callFs('open', { path, mode });
          const fid = String(h);
          return {
            read(n) { return callFs('read', { fid, n: Number(n) }); },
            readline() { return callFs('readline', { fid }); },
            readall() { return callFs('readall', { fid }); },
            write(s) { return callFs('write', { fid, data: s }); },
            flush() { return callFs('flush', { fid }); },
            seek(pos, whence) { return callFs('seek', { fid, pos: Number(pos), whence }); },
            tell() { return callFs('tell', { fid }); },
            eof() { return callFs('eof', { fid }); },
            close() { return callFs('close', { fid }); },
            _fid: fid,
          };
        },
        stat(path) { return callFs('stat', { path }); },
        statfull(path, follow) { return callFs('statfull', { path, follow }); },
        access(path) { return callFs('access', { path }); },
        mkdir(path, all) { return callFs('mkdir', { path, all }); },
        delete(path) { return callFs('delete', { path }); },
        rename(src, dst) { return callFs('rename', { src, dst }); },
        copy(src, dst) { return callFs('copy', { src, dst }); },
        readdir(path) { return callFs('readdir', { path }); },
        pwd() { return callFs('pwd', {}); },
        cd(path) { return callFs('cd', { path }); },
        readfile(path) { return callFs('readfile', { path }); },
        tempfile(prefix) { return callFs('tempfile', { prefix }); },
        settimes(path, atime, mtime) { return callFs('settimes', { path, atime, mtime }); },
        readlink(path) { return callFs('readlink', { path }); },
      };
      handle.set_fs(contract);
      fsNote = 'opfs-sab';
    } else {
      try {
        if (typeof navigator !== 'undefined' && navigator.storage && navigator.storage.getDirectory) {
          const root = await navigator.storage.getDirectory();
          const fs = await makeOpfsFs(root);
          await fs.loadTree();
          handle.set_fs(fs.contract);
          fsNote = 'opfs';
        }
      } catch (e) {
        console.warn('[worker] OPFS unavailable, memory fs fallback:', e && e.message);
      }
    }

    handle.set_output_handler((s) => outputs.push(s));

    // regexp 外置：注册 JS 面并 load + 装语义 wrapper（wasm 子集不含
    // regexp-lite，native 命令面不存在——dynmod 注入同面实现）。
    if (!handle.exec('namespace which -command ::regexp')) {
      handle.register_module('regexp', makeRegexpMod());
      handle.exec('load regexp');
      handle.exec(REGEXP_SETUP_TCL);
    }

    globalThis.postMessage({ type: 'ready', fs: fsNote });

    // ③ exec 队列补跑（bridge/direct 均就绪后）
    booted = true;
    for (const msg of execQ) dispatchExec(msg);
  } catch (e) {
    globalThis.postMessage({ type: 'init-error', error: String(e && e.stack || e) });
  }
})();
