if (typeof postMessage !== 'function') {
  const { parentPort } = require('node:worker_threads');
  globalThis.postMessage = (m) => parentPort.postMessage(m);
  globalThis.onmessage = null;
  parentPort.on('message', (m) => { if (globalThis.onmessage) globalThis.onmessage({ data: m }); });
}
// fs-worker.js — OPFS 执行 worker（SAB 同步桥的另一端）。
//
// 协议（与 rtcl-worker.js 的 callFs 配对）：
//   主线程把 SAB postMessage 给本 worker 与 interp worker 两端。
//   interp worker 的同步 HostFs 调用 → 写请求 JSON 到 SAB payload +
//   压 REQ_SEQ + postMessage({seq}) → 本 worker 执行真实 async OPFS →
//   把回执 JSON 写入 SAB + 压 REPLY_SEQ=seq + Atomics.notify →
//   interp worker 从 Atomics.wait 醒来读取回执返回 Rust。
//   **回执在 OPFS 写入完成之后才发**——Tcl 文件操作返回即落盘
//   （严格 durable）。
//
// SAB 布局（Int32 视图 + payload 字节区）：
//   [0]=REQ_SEQ  [1]=REPLY_SEQ  [2]=PAYLOAD_LEN
//   payload 从字节偏移 64 起（256 KiB 容量）。
//
// 元数据（mtime/atime）：OPFS 无 utimes API——sidecar 文件
//   /__rtcl_meta.json 持久化 {path: {mtime, atime}}，settimes/write
//   更新并回写（跨刷新保留）。
//
// OPFS 说明：sync access handle 仅 worker 可用——本 worker 内 open 后
// 的 read/write/seek/tell/eof 全同步执行（快路径）。

let root = null;
const meta = new Map();       // path -> {mtime, atime}
const handles = new Map();    // fid -> {access, pos, path, mode}
let nextFid = 1;
let initialized = false;
const queued = [];            // init 完成前排队的消息

const META_PATH = '/__rtcl_meta.json';
const PAYLOAD_OFFSET = 64;
const PAYLOAD_CAP = 256 * 1024;

let i32 = null, u8 = null;

const parent = (p) => { const i = p.lastIndexOf('/'); return i <= 0 ? '/' : p.slice(0, i); };
const base = (p) => p.slice(p.lastIndexOf('/') + 1);
const now = () => Math.floor(Date.now() / 1000);
let cwd = '/';
const cwdDirs = [];
const norm = (p, cwd = '/') => {
  if (!p.startsWith('/')) p = (cwd === '/' ? '' : cwd) + '/' + p;
  const parts = [];
  for (const seg of p.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') parts.pop(); else parts.push(seg);
  }
  return '/' + parts.join('/');
};

async function dirHandle(path, create) {
  let cur = root;
  for (const seg of norm(path, '/').split('/').filter(Boolean)) {
    cur = await cur.getDirectoryHandle(seg, { create });
  }
  return cur;
}

async function loadMeta() {
  try {
    const dh = await root.getDirectoryHandle('/', { create: false });
    const fh = await dh.getFileHandle(base(META_PATH), { create: false });
    const f = await fh.getFile();
    const obj = JSON.parse(await f.text());
    for (const [k, v] of Object.entries(obj)) meta.set(k, v);
  } catch { /* 无 sidecar：全新存储 */ }
}

async function persistMeta() {
  const obj = {};
  for (const [k, v] of meta.entries()) obj[k] = v;
  const dh = await dirHandle('/', true);
  const fh = await dh.getFileHandle(base(META_PATH), { create: true });
  const w = await fh.createWritable();
  await w.write(JSON.stringify(obj));
  await w.close();
}

function touchMeta(path, patch) {
  const p = norm(path, '/');
  const cur = meta.get(p) || { mtime: 0, atime: 0 };
  const next = { ...cur, ...patch };
  meta.set(p, next);
  persistMeta();   // fire-and-forget（sidecar 自身允许 eventual）
  return next;
}

// ── 18 方法（async 真实 OPFS）───────────────────────────────────────

const ops = {
  async open({ path, mode }) {
    path = norm(path, '/');
    const m = mode.toLowerCase();
    const writeMode = !m.startsWith('r');
    const dh = await dirHandle(parent(path), writeMode);
    let fh;
    try {
      fh = await dh.getFileHandle(base(path), { create: writeMode });
    } catch {
      throw new Error("couldn't open \"" + path + '": no such file or directory');
    }
    const access = await fh.createSyncAccessHandle();
    const fid = String(nextFid++);
    if (m.startsWith('w')) { access.truncate(0); }
    handles.set(fid, {
      access,
      pos: m.startsWith('a') ? access.getSize() : (m.startsWith('w') ? 0 : 0),
      path, mode: m,
    });
    if (writeMode) touchMeta(path, { mtime: now(), atime: now() });
    return fid;
  },

  read({ fid, n }) {
    const h = handles.get(fid);
    const size = h.access.getSize();
    if (h.pos >= size) return '';
    const take = Math.min(n || 1, size - h.pos);
    const buf = new Uint8Array(take);
    const got = h.access.read(buf, { at: h.pos });
    h.pos += got;
    let s = '';
    for (let i = 0; i < got; i++) s += String.fromCharCode(buf[i]);
    return s;
  },

  readline({ fid }) {
    const h = handles.get(fid);
    const size = h.access.getSize();
    if (h.pos >= size) return null;
    let s = '';
    while (h.pos < size) {
      const buf = new Uint8Array(1);
      const got = h.access.read(buf, { at: h.pos });
      if (got === 0) break;
      h.pos += 1;
      const c = String.fromCharCode(buf[0]);
      if (c === '\n') return s.replace(/\r$/, '');
      s += c;
    }
    return s;
  },

  readall({ fid }) {
    const h = handles.get(fid);
    const size = h.access.getSize();
    const buf = new Uint8Array(size);
    h.access.read(buf, { at: 0 });
    let s = '';
    for (let i = 0; i < size; i++) s += String.fromCharCode(buf[i]);
    return s;
  },

  write({ fid, data }) {
    const h = handles.get(fid);
    const bytes = new Uint8Array(data.length);
    for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
    const written = h.access.write(bytes, { at: h.pos });
    h.pos += written;
    touchMeta(h.path, { mtime: now() });
    return written;
  },

  flush({ fid }) {
    handles.get(fid).access.flush();
    return null;
  },

  seek({ fid, pos, whence }) {
    const h = handles.get(fid);
    const size = h.access.getSize();
    let p;
    if (whence === 'set') p = pos;
    else if (whence === 'cur') p = h.pos + pos;
    else p = size + pos;
    p = Math.max(0, Math.min(p, size));
    h.pos = p;
    return p;
  },

  tell({ fid }) { return handles.get(fid).pos; },

  eof({ fid }) { return handles.get(fid).pos >= handles.get(fid).access.getSize(); },

  close({ fid }) {
    handles.get(fid).access.close();
    handles.delete(fid);
    return null;
  },

  async stat({ path }) {
    path = norm(path, '/');
    // 文件？
    try {
      const dh = await dirHandle(parent(path), false);
      const fh = await dh.getFileHandle(base(path), { create: false });
      const access = await fh.createSyncAccessHandle();
      const size = access.getSize();
      access.close();
      const m = meta.get(path) || {};
      return { size, mtime: m.mtime ?? 0, atime: m.atime ?? 0, dir: false, file: true };
    } catch { /* 不是文件，试目录 */ }
    try {
      await dirHandle(path, false);
      return { size: 0, mtime: 0, atime: 0, dir: true, file: false };
    } catch {
      return null;
    }
  },

  async access({ path }) {
    const s = await ops.stat({ path });
    return { read: s !== null, write: true, exec: false };
  },

  async mkdir({ path, all }) {
    path = norm(path, '/');
    if (!all) {
      if (!(await existsDir(parent(path)))) {
        throw new Error('couldn\'t create directory "' + path + '": no such file or directory');
      }
    }
    await dirHandle(path, true);
    return null;
  },

  async delete({ path }) {
    path = norm(path, '/');
    const dh = await dirHandle(parent(path), false);
    await dh.removeEntry(base(path), { recursive: true });
    meta.delete(path);
    persistMeta();
    return null;
  },

  async rename({ src, dst }) {
    src = norm(src, '/'); dst = norm(dst, '/');
    const dh = await dirHandle(parent(src), false);
    const fh = await dh.getFileHandle(base(src), { create: false });
    if (typeof fh.move === 'function') {
      // Chrome OPFS 扩展：跨目录 move
      const dstDir = await dirHandle(parent(dst), true);
      await fh.move(dstDir, base(dst));
    } else {
      // 回退：读 + 写 + 删
      const access = await fh.createSyncAccessHandle();
      const size = access.getSize();
      const buf = new Uint8Array(size);
      access.read(buf, { at: 0 });
      access.close();
      const dstDh = await dirHandle(parent(dst), true);
      const dstFh = await dstDh.getFileHandle(base(dst), { create: true });
      const w = await dstFh.createWritable();
      await w.write(buf);
      await w.close();
      await dh.removeEntry(base(src));
    }
    if (meta.has(src)) { meta.set(dst, meta.get(src)); meta.delete(src); persistMeta(); }
    return null;
  },

  async copy({ src, dst }) {
    src = norm(src, '/'); dst = norm(dst, '/');
    const dh = await dirHandle(parent(src), false);
    const fh = await dh.getFileHandle(base(src), { create: false });
    const access = await fh.createSyncAccessHandle();
    const size = access.getSize();
    const buf = new Uint8Array(size);
    access.read(buf, { at: 0 });
    access.close();
    const dstDh = await dirHandle(parent(dst), true);
    const dstFh = await dstDh.getFileHandle(base(dst), { create: true });
    const w = await dstFh.createWritable();
    await w.write(buf);
    await w.close();
    return null;
  },

  async readdir({ path }) {
    const dh = await dirHandle(norm(path, '/'), false);
    const out = [];
    for await (const [name, h] of dh.entries()) out.push(name);
    return out;
  },

  pwd() { return cwd; },
  cd({ path }) { cwd = norm(path, '/'); return null; },

  async readfile({ path }) {
    path = norm(path, '/');
    const dh = await dirHandle(parent(path), false);
    const fh = await dh.getFileHandle(base(path), { create: false });
    const f = await fh.getFile();
    const bytes = new Uint8Array(await f.arrayBuffer());
    let s = '';
    for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
    return s;
  },

  tempfile({ prefix }) {
    const p = '/tmp/' + prefix + (Date.now() % 100000);
    cwdDirs.push(p);
    return p;
  },

  async settimes({ path, atime, mtime }) {
    path = norm(path, '/');
    if (!(await existsEntry(path))) {
      throw new Error("couldn't set time for \"" + path + '": no such file');
    }
    touchMeta(path, {
      mtime: mtime === null || mtime === undefined ? undefined : Number(mtime),
      atime: atime === null || atime === undefined ? undefined : Number(atime),
    });
    return null;
  },

  readlink() { throw new Error('invalid argument'); },

  async statfull({ path, follow }) {
    const s = await ops.stat({ path: norm(path, '/') });
    if (s === null) return null;
    let ino = 0;
    for (let i = 0; i < path.length; i++) ino = (ino * 31 + path.charCodeAt(i)) >>> 0;
    const mode = s.dir ? 0o40755 : 0o100644;
    return Object.assign({}, s, {
      ctime: s.mtime, dev: 0, ino, nlink: 1, uid: 1000, gid: 1000,
      mode, blksize: 4096, blocks: Math.ceil(s.size / 512), link: false,
      readable: true, writable: true, executable: false, owned: true,
    });
  },
};


async function existsDir(path) {
  try { await dirHandle(path, false); return true; } catch { return false; }
}
async function existsEntry(path) {
  try {
    const dh = await dirHandle(parent(path), false);
    await dh.getFileHandle(base(path), { create: false });
    return true;
  } catch {
    return existsDir(path);
  }
}

// ── SAB 回执协议 ────────────────────────────────────────────────────

let callPort = null;

function handleCall(msg) {
  const seq = msg.seq;
  try {
    const len = Atomics.load(i32, 2);
    // SAB 视图不可直接喂 TextDecoder——拷贝到普通缓冲
    const copy = new Uint8Array(len);
    copy.set(u8.subarray(64, 64 + len));
    const req = JSON.parse(new TextDecoder().decode(copy));
    const fn = ops[req.op];
    if (!fn) throw new Error('unknown fs op: ' + req.op);
    Promise.resolve(fn(req.params)).then((res) => {
      const r = { ok: true, result: res === undefined ? null : res };
      replyWrite(seq, r);
    }).catch((e) => replyWrite(seq, { ok: false, error: String(e && e.message || e) }));
  } catch (e) {
    replyWrite(seq, { ok: false, error: String(e && e.message || e) });
  }
}

function replyWrite(seq, r) {
  const enc = new TextEncoder().encode(JSON.stringify(r));
  u8.set(enc.subarray(0, Math.min(enc.length, PAYLOAD_CAP)), PAYLOAD_OFFSET);
  Atomics.store(i32, 2, enc.length);
  Atomics.store(i32, 1, seq);
  Atomics.notify(i32, 1);
}

globalThis.onmessage = async (ev) => {
  const msg = ev.data;
  if (!initialized) {
    if (msg.sab) {
      i32 = new Int32Array(msg.sab);
      u8 = new Uint8Array(msg.sab);
      root = await navigator.storage.getDirectory();
      await loadMeta();
      initialized = true;
      if (msg.port) {
        callPort = msg.port;
        callPort.onmessage = (e2) => handleCall(e2.data);
      }
      for (const q of queued) handleCall(q);
    } else {
      queued.push(msg);
    }
    return;
  }
  if (msg.port) {
    callPort = msg.port;
    callPort.onmessage = (e2) => handleCall(e2.data);
    return;
  }
  handleCall(msg);
};
